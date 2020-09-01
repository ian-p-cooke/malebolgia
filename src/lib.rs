use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use strum_macros::Display;
use thiserror::Error;

#[cfg(test)]
#[macro_use]
extern crate rusty_fork;

#[cfg(feature = "tokio-compat")]
use tokio::{self, task::JoinHandle as TokioJoinHandle};

#[cfg(any(feature = "tokio-compat", feature = "futures-compat"))]
use futures::future::{AbortHandle, Abortable};

#[cfg(feature = "smol-compat")]
use smol::Task as SmolTask;

#[cfg(feature = "async-std-compat")]
use async_std::task::JoinHandle as AsyncStdJoinHandle;

use once_cell::sync::OnceCell;

#[cfg(feature = "futures-compat")]
use futures::{executor::ThreadPool, future::RemoteHandle};

#[cfg(any(feature = "tokio-compat", feature = "futures-compat"))]
use futures::FutureExt;

#[derive(Error, Debug)]
pub enum SpawnError {
    #[error("a task ended with a panic")]
    JoinHandleError(String),
    #[error("a global executor is already set")]
    SingletonError,
}

#[derive(Display, Debug)]
pub enum Executor {
    None,
    #[cfg(feature = "tokio-compat")]
    Tokio,
    #[cfg(feature = "smol-compat")]
    Smol,
    #[cfg(feature = "async-std-compat")]
    AsyncStd,
    #[cfg(feature = "futures-compat")]
    Futures(ThreadPool),
}

static EX: OnceCell<Executor> = OnceCell::new();

impl Executor {
    pub fn set(self) -> Result<(), SpawnError> {
        match EX.set(self) {
            Ok(_) => Ok(()),
            Err(_) => Err(SpawnError::SingletonError),
        }
    }
    pub fn get() -> Option<&'static Executor> {
        EX.get()
    }
}

pub enum JoinHandle<T> {
    None(T),
    #[cfg(feature = "tokio-compat")]
    Tokio((TokioJoinHandle<T>, Option<AbortHandle>)),
    #[cfg(feature = "smol-compat")]
    Smol(SmolTask<T>),
    #[cfg(feature = "async-std-compat")]
    AsyncStd(AsyncStdJoinHandle<T>),
    #[cfg(feature = "futures-compat")]
    Futures((RemoteHandle<T>, Option<AbortHandle>)),
}

impl<T: Unpin + 'static> Future for JoinHandle<T> {
    type Output = Result<T, SpawnError>;

    #[allow(unused_variables)]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.get_mut() {
            JoinHandle::None(_) => panic!("JoinHandle::None cannot be polled!"),
            #[cfg(feature = "tokio-compat")]
            JoinHandle::Tokio((handle, _)) => match Pin::new(handle).poll(cx) {
                Poll::Ready(result) => match result {
                    Ok(output) => Poll::Ready(Ok(output)),
                    Err(e) => Poll::Ready(Err(SpawnError::JoinHandleError(e.to_string()))),
                },
                Poll::Pending => Poll::Pending,
            },
            #[cfg(feature = "smol-compat")]
            JoinHandle::Smol(t) => match Pin::new(t).poll(cx) {
                Poll::Ready(output) => Poll::Ready(Ok(output)),
                Poll::Pending => Poll::Pending,
            },
            #[cfg(feature = "async-std-compat")]
            JoinHandle::AsyncStd(handle) => match Pin::new(handle).poll(cx) {
                Poll::Ready(output) => Poll::Ready(Ok(output)),
                Poll::Pending => Poll::Pending,
            },
            #[cfg(feature = "futures-compat")]
            JoinHandle::Futures((handle, _)) => match Pin::new(handle).poll(cx) {
                Poll::Ready(output) => Poll::Ready(Ok(output)),
                Poll::Pending => Poll::Pending,
            },
        }
    }
}

unsafe impl<T: Send> Send for JoinHandle<T> {}
unsafe impl<T: Sync> Sync for JoinHandle<T> {}
impl<T> Unpin for JoinHandle<T> {}

impl<T: 'static> JoinHandle<T> {
    pub fn detach(self) {
        match self {
            JoinHandle::None(_) => panic!("you can not detatch JoinHandle::None!"),
            #[cfg(feature = "tokio-compat")]
            JoinHandle::Tokio((handle, _)) => drop(handle),
            #[cfg(feature = "smol-compat")]
            JoinHandle::Smol(task) => task.detach(),
            #[cfg(feature = "async-std-compat")]
            JoinHandle::AsyncStd(handle) => drop(handle),
            #[cfg(feature = "futures-compat")]
            JoinHandle::Futures((handle, _)) => handle.forget(),
        }
    }
    pub async fn cancel(self) -> Option<T> {
        match self {
            JoinHandle::None(_) => panic!("you can not detatch JoinHandle::None!"),
            #[cfg(feature = "tokio-compat")]
            JoinHandle::Tokio((handle, abort_handle)) => {
                match abort_handle {
                    Some(abort_handle) => {
                        let maybe_output = handle.now_or_never();
                        let output = if let Some(Ok(output)) = maybe_output {
                            Some(output)
                        } else {
                            None
                        };
                        abort_handle.abort();
                        output
                    }
                    None => {
                        // cancel shouldn't return until the task is canceled or finished so we have to wait
                        match handle.await {
                            Ok(output) => Some(output),
                            Err(_) => None,
                        }
                    }
                }
            }
            #[cfg(feature = "smol-compat")]
            JoinHandle::Smol(task) => task.cancel().await,
            #[cfg(feature = "async-std-compat")]
            JoinHandle::AsyncStd(handle) => handle.cancel().await,
            #[cfg(feature = "futures-compat")]
            JoinHandle::Futures((handle, abort_handle)) => {
                match abort_handle {
                    Some(abort_handle) => {
                        let output = handle.now_or_never();
                        abort_handle.abort();
                        output
                    }
                    None => {
                        // cancel shouldn't return until the task is canceled or finished so we have to wait
                        Some(handle.await)
                    }
                }
            }
        }
    }
}

pub struct Spawner;

impl Spawner {
    #[allow(unused_variables)]
    pub fn spawn<T>(future: impl Future<Output = T> + Send + 'static) -> JoinHandle<T>
    where
        T: Send + 'static,
    {
        match EX.get() {
            Some(Executor::None) => {
                panic!("Executor::None can not spawn anything");
            }
            #[cfg(feature = "tokio-compat")]
            Some(Executor::Tokio) => {
                let (abort_handle, abort_registration) = AbortHandle::new_pair();
                let future = Abortable::new(future, abort_registration);
                let future = async move { future.await.unwrap() };
                let handle = tokio::spawn(future);
                JoinHandle::Tokio((handle, Some(abort_handle)))
            }
            #[cfg(feature = "smol-compat")]
            Some(Executor::Smol) => {
                let task = smol::spawn(future);
                JoinHandle::Smol(task)
            }
            #[cfg(feature = "async-std-compat")]
            Some(Executor::AsyncStd) => {
                let handle = async_std::task::spawn(future);
                JoinHandle::AsyncStd(handle)
            }
            #[cfg(feature = "futures-compat")]
            Some(Executor::Futures(thread_pool)) => {
                let (abort_handle, abort_registration) = AbortHandle::new_pair();
                let future = Abortable::new(future, abort_registration);
                let future = async move { future.await.unwrap() };
                let (future, handle) = future.remote_handle();
                thread_pool.spawn_ok(future);
                JoinHandle::Futures((handle, Some(abort_handle)))
            }
            None => panic!("Spawner::spawn must be called after setting an Executor"),
        }
    }

    #[allow(unused_variables)]
    pub fn spawn_blocking<F, T>(f: F) -> JoinHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        match EX.get() {
            Some(Executor::None) => {
                panic!("Executor::None can not spawn anything");
            }
            #[cfg(feature = "tokio-compat")]
            Some(Executor::Tokio) => {
                let handle = tokio::task::spawn_blocking(f);
                JoinHandle::Tokio((handle, None))
            }
            #[cfg(feature = "smol-compat")]
            Some(Executor::Smol) => Spawner::spawn(smol::unblock(f)),
            #[cfg(feature = "async-std-compat")]
            Some(Executor::AsyncStd) => {
                let handle = async_std::task::spawn_blocking(f);
                JoinHandle::AsyncStd(handle)
            }
            #[cfg(feature = "futures-compat")]
            Some(Executor::Futures(_)) => Spawner::spawn(blocking::unblock(f)),
            None => panic!("Spawner::spawn_blocking must be called after setting an Executor"),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Executor, JoinHandle, SpawnError, Spawner};
    use futures::executor::ThreadPool;
    use std::time::Duration;

    async fn detach_test() -> Result<(), Box<dyn std::error::Error>> {
        let (tx, rx) = async_channel::unbounded();
        let handle = Spawner::spawn(async move {
            async_io::Timer::new(Duration::from_micros(500)).await;
            tx.send(()).await.unwrap();
        });

        handle.detach();

        match rx.recv().await {
            Ok(_) => {} // pass
            Err(_) => panic!("you should receive something because the task should run to completion!"),
        };

        Ok::<(), Box<dyn std::error::Error>>(())
    }

    async fn cancel_test_none() -> Result<(), Box<dyn std::error::Error>> {
        let (tx, rx) = async_channel::unbounded();
        let handle = Spawner::spawn(async move {
            async_io::Timer::new(Duration::from_secs(5)).await;
            tx.send(()).await.unwrap();
        });

        let output = handle.cancel().await;
        assert_eq!(output, None);

        match rx.recv().await {
            Ok(_) => panic!("you should receive nothing because the task was canceled!"),
            Err(_) => {} //pass
        };

        Ok::<(), Box<dyn std::error::Error>>(())
    }

    async fn cancel_test_some() -> Result<(), Box<dyn std::error::Error>> {
        let handle = Spawner::spawn(async {
            2u64 + 2u64
        });

        async_io::Timer::new(Duration::from_micros(100)).await; //give the spawned future time to run

        let output = handle.cancel().await;
        assert_eq!(output, Some(4));

        Ok::<(), Box<dyn std::error::Error>>(())
    }

    rusty_fork_test! {
    #[test]
    #[should_panic]
    fn spawn_no_executor_set_fails() {
        let _ = Spawner::spawn(async { 2 + 2 });
    }

    #[test]
    fn tokio_spawn() {
        Executor::Tokio.set().unwrap();
        let mut ex = tokio::runtime::Runtime::new().unwrap();
        ex.block_on(async {
            let handle = Spawner::spawn(async { 2 + 2 });
            if let JoinHandle::Tokio(_) = handle {
                //pass
            } else {
                panic!("wrong join handle!");
            }
            let output = handle.await?;
            assert_eq!(output, 4);
            Ok::<(), SpawnError>(())
        })
        .unwrap();
    }

    #[test]
    fn smol_spawn() {
        Executor::Smol.set().unwrap();
        smol::block_on(async {
            let handle = Spawner::spawn(async { 2 + 2 });
            if let JoinHandle::Smol(_) = handle {
                //pass
            } else {
                panic!("wrong join handle!");
            }
            let output = handle.await?;
            assert_eq!(output, 4);
            Ok::<(), SpawnError>(())
        })
        .unwrap();
    }

    #[test]
    fn async_std_spawn() {
        Executor::AsyncStd.set().unwrap();
        async_std::task::block_on(async {
            let handle = Spawner::spawn(async { 2 + 2 });
            if let JoinHandle::AsyncStd(_) = handle {
                //pass
            } else {
                panic!("wrong join handle!");
            }
            let output = handle.await?;
            assert_eq!(output, 4);
            Ok::<(), SpawnError>(())
        })
        .unwrap();
    }

    #[test]
    fn futures_spawn() {
        let ex = ThreadPool::new().unwrap();
        Executor::Futures(ex).set().unwrap();

        futures::executor::block_on(async move {
            let handle = Spawner::spawn(async { 2 + 2 });
            if let JoinHandle::Futures(_) = handle {
                //pass
            } else {
                panic!("wrong join handle!");
            }
            let output = handle.await?;
            assert_eq!(output, 4);
            Ok::<(), SpawnError>(())
        })
        .unwrap();
    }

    #[test]
    #[should_panic]
    fn tokio_cannot_spawn_smol_executor() {
        Executor::Tokio.set().unwrap();
        smol::block_on(async {
            let _ = Spawner::spawn(async { 2 + 2 });
        });
    }

    #[test]
    fn cancel_tokio_none() {
        Executor::Tokio.set().unwrap();
        let mut ex = tokio::runtime::Runtime::new().unwrap();
        ex.block_on(cancel_test_none()).unwrap();
    }

    #[test]
    fn cancel_tokio_executor_some() {
        Executor::Tokio.set().unwrap();
        let mut ex = tokio::runtime::Runtime::new().unwrap();
        ex.block_on(cancel_test_some()).unwrap();
    }

    #[test]
    fn cancel_smol_none() {
        Executor::Smol.set().unwrap();
        smol::block_on(cancel_test_none()).unwrap();
    }

    #[test]
    fn cancel_smol_executor_some() {
        Executor::Smol.set().unwrap();
        smol::block_on(cancel_test_some()).unwrap();
    }

    #[test]
    fn cancel_futures_none() {
        let ex = ThreadPool::new().unwrap();
        Executor::Futures(ex).set().unwrap();
        futures::executor::block_on(cancel_test_none()).unwrap();
    }

    #[test]
    fn cancel_futures_some() {
        let ex = ThreadPool::new().unwrap();
        Executor::Futures(ex).set().unwrap();
        futures::executor::block_on(cancel_test_some()).unwrap();
    }

    #[test]
    fn detach_tokio() {
        Executor::Tokio.set().unwrap();
        let mut ex = tokio::runtime::Runtime::new().unwrap();
        ex.block_on(detach_test()).unwrap();
    }

    #[test]
    fn detach_smol() {
        Executor::Smol.set().unwrap();
        smol::block_on(detach_test()).unwrap();
    }

    #[test]
    fn detach_futures() {
        let ex = ThreadPool::new().unwrap();
        Executor::Futures(ex).set().unwrap();
        futures::executor::block_on(detach_test()).unwrap();
    }

    }
}

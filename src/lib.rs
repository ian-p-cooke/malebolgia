use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use strum_macros::Display;
use thiserror::Error;

#[cfg(test)]
#[macro_use]
extern crate rusty_fork;

#[cfg(feature = "async-executor-compat")]
use async_executor::Task as AsyncExecutorTask;

#[cfg(feature = "tokio-compat")]
use tokio::{self, task::JoinHandle as TokioJoinHandle};

#[cfg(any(feature = "tokio-compat", feature="futures-compat"))]
use futures::future::{AbortHandle, Abortable};

#[cfg(feature = "smol-compat")]
use smol::Task as SmolTask;

#[cfg(feature = "async-std-compat")]
use async_std::task::JoinHandle as AsyncStdJoinHandle;

use once_cell::sync::OnceCell;
k
#[cfg(feature = "futures-compat")]
use futures::{executor::ThreadPool, future::RemoteHandle};

#[cfg(any(feature="tokio-compat", feature="futures-compat"))]
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
    #[cfg(feature = "async-executor-compat")]
    AsyncExecutor,
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
    #[cfg(feature = "async-executor-compat")]
    AsyncExecutor(AsyncExecutorTask<T>),
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
            #[cfg(feature = "async-executor-compat")]
            JoinHandle::AsyncExecutor(task) => match Pin::new(task).poll(cx) {
                Poll::Ready(output) => Poll::Ready(Ok(output)),
                Poll::Pending => Poll::Pending,
            },
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
            #[cfg(feature = "async-executor-compat")]
            JoinHandle::AsyncExecutor(task) => task.detach(),
            #[cfg(feature = "futures-compat")]
            JoinHandle::Futures((handle, _)) => handle.forget(),
        }
    }
    pub async fn cancel(self) -> Option<T> {
        match self {
            JoinHandle::None(_) => panic!("you can not detatch JoinHandle::None!"),
            #[cfg(feature = "tokio-compat")]
            JoinHandle::Tokio((handle, abort_handle)) => {
                let maybe_output = handle.now_or_never();
                let output = if let Some(Ok(output)) = maybe_output {
                    Some(output)
                } else {
                    None
                };
                if let Some(abort_handle) = abort_handle {
                    abort_handle.abort();
                }
                output
            }
            #[cfg(feature = "smol-compat")]
            JoinHandle::Smol(task) => task.cancel().await,
            #[cfg(feature = "async-std-compat")]
            JoinHandle::AsyncStd(handle) => handle.cancel().await,
            #[cfg(feature = "async-executor-compat")]
            JoinHandle::AsyncExecutor(task) => task.cancel().await,
            #[cfg(feature = "futures-compat")]
            JoinHandle::Futures((handle, abort_handle)) => {
                let output = handle.now_or_never();
                if let Some(abort_handle) = abort_handle {
                    abort_handle.abort();
                }
                output
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
            #[cfg(feature = "async-executor-compat")]
            Some(Executor::AsyncExecutor) => {
                let task = AsyncExecutorTask::spawn(future);
                JoinHandle::AsyncExecutor(task)
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
                let task = SmolTask::spawn(future);
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
            #[cfg(feature = "async-executor-compat")]
            Some(Executor::AsyncExecutor) => {
                //async-executor has no native spawn_blocking; using blocking directly
                Spawner::spawn(async move { blocking::unblock!(f()) })
            }
            #[cfg(feature = "tokio-compat")]
            Some(Executor::Tokio) => {
                let handle = tokio::task::spawn_blocking(f);
                JoinHandle::Tokio((handle, None))
            }
            #[cfg(feature = "smol-compat")]
            Some(Executor::Smol) => Spawner::spawn(async move { smol::unblock!(f()) }),
            #[cfg(feature = "async-std-compat")]
            Some(Executor::AsyncStd) => {
                let handle = async_std::task::spawn_blocking(f);
                JoinHandle::AsyncStd(handle)
            }
            #[cfg(feature = "futures-compat")]
            Some(Executor::Futures(_)) => Spawner::spawn(async move { blocking::unblock!(f()) }),
            None => panic!("Spawner::spawn_blocking must be called after setting an Executor"),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Executor, JoinHandle, SpawnError, Spawner};
    use futures::executor::ThreadPool;

    rusty_fork_test! {
        #[test]
        #[should_panic]
        fn spawn_no_executor_set_fails() {
            let _ = Spawner::spawn(async { 2 + 2 });
        }

        #[test]
        fn async_executor_spawn()  {
            Executor::AsyncExecutor.set().unwrap();
            let ex = async_executor::Executor::new();
            ex.run(async {
                let handle = Spawner::spawn(async { 2 + 2 });
                if let JoinHandle::AsyncExecutor(_) = handle {
                    //pass
                } else {
                    panic!("wrong join handle!");
                }
                let output = handle.await?;
                assert_eq!(output, 4);
                Ok::<(), SpawnError>(())
            }).unwrap();
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
            }).unwrap();
        }

        #[test]
        fn smol_spawn() {
            Executor::Smol.set().unwrap();
            smol::run(async {
                let handle = Spawner::spawn(async { 2 + 2 });
                if let JoinHandle::Smol(_) = handle {
                    //pass
                } else {
                    panic!("wrong join handle!");
                }
                let output = handle.await?;
                assert_eq!(output, 4);
                Ok::<(), SpawnError>(())
            }).unwrap();
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
            }).unwrap();
        }

        #[test]
        fn futures_spawn() {
            let ex = ThreadPool::new().unwrap();
            Executor::Futures(ex).set().unwrap();

            futures::executor::block_on( async move {
                let handle = Spawner::spawn(async { 2 + 2 });
                if let JoinHandle::Futures(_) = handle {
                    //pass
                } else {
                    panic!("wrong join handle!");
                }
                let output = handle.await?;
                assert_eq!(output, 4);
                Ok::<(), SpawnError>(())
            }).unwrap();
        }

        #[test]
        #[should_panic]
        fn tokio_cannot_spawn_async_executor() {
            Executor::Tokio.set().unwrap();
            let ex = async_executor::Executor::new();
            ex.run(async {
                let _ = Spawner::spawn(async { 2 + 2 });
            });
        }

        #[test]
        #[should_panic]
        fn async_executor_cannot_spawn_tokio() {
            Executor::AsyncExecutor.set().unwrap();
            let mut ex = tokio::runtime::Runtime::new().unwrap();
            ex.block_on(async {
                let _ = Spawner::spawn(async { 2 + 2 });
            });
        }
    }
}

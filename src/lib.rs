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

#[cfg(feature = "smol-compat")]
use smol::Task as SmolTask;

#[cfg(feature = "async-std-compat")]
use async_std::task::JoinHandle as AsyncStdJoinHandle;
use once_cell::sync::OnceCell;

#[derive(Error, Debug)]
pub enum SpawnError {
    #[error("a task ended with a panic")]
    JoinHandleError(String),
    #[error("a global executor is already set: {0}")]
    SingletonError(Executor),
}

#[derive(Display, Debug, Copy, Clone)]
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
}

static EX: OnceCell<Executor> = OnceCell::new();

impl Executor {
    pub fn set(self) -> Result<(), SpawnError> {
        match EX.set(self) {
            Ok(_) => Ok(()),
            Err(_) => Err(SpawnError::SingletonError(*EX.get().unwrap())),
        }
    }
    pub fn get() -> Option<Executor> {
        return EX.get().map(|ex| *ex);
    }
}

pub enum JoinHandle<T> {
    None(T),
    #[cfg(feature = "async-executor-compat")]
    AsyncExecutor(AsyncExecutorTask<T>),
    #[cfg(feature = "tokio-compat")]
    Tokio(TokioJoinHandle<T>),
    #[cfg(feature = "smol-compat")]
    Smol(SmolTask<T>),
    #[cfg(feature = "async-std-compat")]
    AsyncStd(AsyncStdJoinHandle<T>),
}

impl<T: Unpin> Future for JoinHandle<T> {
    type Output = Result<T, SpawnError>;

    #[allow(unused_variables)]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.get_mut() {
            JoinHandle::None(_) => panic!("JoinHandle::None cannot be polled!"),
            #[cfg(feature = "async-executor-compat")]
            JoinHandle::AsyncExecutor(t) => match Pin::new(t).poll(cx) {
                Poll::Ready(output) => Poll::Ready(Ok(output)),
                Poll::Pending => Poll::Pending,
            },
            #[cfg(feature = "tokio-compat")]
            JoinHandle::Tokio(t) => match Pin::new(t).poll(cx) {
                Poll::Ready(r) => match r {
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
                let handle = tokio::spawn(future);
                JoinHandle::Tokio(handle)
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
                JoinHandle::Tokio(handle)
            }
            #[cfg(feature = "smol-compat")]
            Some(Executor::Smol) => Spawner::spawn(async move { smol::unblock!(f()) }),
            #[cfg(feature = "async-std-compat")]
            Some(Executor::AsyncStd) => {
                let handle = async_std::task::spawn_blocking(f);
                JoinHandle::AsyncStd(handle)
            }
            None => panic!("Spawner::spawn_blocking must be called after setting an Executor"),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Executor, JoinHandle, SpawnError, Spawner};
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

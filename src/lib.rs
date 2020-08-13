use scoped_tls::scoped_thread_local;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use thiserror::Error;

#[cfg(feature = "async-executor-compat")]
use async_executor::Task as AsyncExecutorTask;

#[cfg(feature = "tokio-compat")]
use tokio::{self, task::JoinHandle as TokioJoinHandle};

#[cfg(feature = "smol-compat")]
use smol::Task as SmolTask;

#[cfg(feature = "async-std-compat")]
use async_std::task::JoinHandle as AsyncStdJoinHandle;

pub enum Executor {
    #[cfg(feature = "async-executor-compat")]
    AsyncExecutor,
    #[cfg(feature = "tokio-compat")]
    Tokio,
    #[cfg(feature = "smol-compat")]
    Smol,
    #[cfg(feature = "async-std-compat")]
    AsyncStd,
}

impl Executor {
    pub fn run<T>(&self, f: impl FnOnce() -> T) -> T {
        EX.set(self, f)
    }
}

scoped_thread_local!(static EX: Executor);

pub enum JoinHandle<T> {
    #[cfg(feature = "async-executor-compat")]
    AsyncExecutor(AsyncExecutorTask<T>),
    #[cfg(feature = "tokio-compat")]
    Tokio(TokioJoinHandle<T>),
    #[cfg(feature = "smol-compat")]
    Smol(SmolTask<T>),
    #[cfg(feature = "async-std-compat")]
    AsyncStd(AsyncStdJoinHandle<T>),
}

#[derive(Error, Debug)]
pub enum SpawnError {
    #[error("a task ended with a panic")]
    JoinHandleError(String),
}

impl<T> Future for JoinHandle<T> {
    type Output = Result<T, SpawnError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.get_mut() {
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
    pub fn spawn<T>(future: impl Future<Output = T> + Send + 'static) -> JoinHandle<T>
    where
        T: Send + 'static,
    {
        if EX.is_set() {
            EX.with(|ex| match &ex {
                #[cfg(feature = "async-executor-compat")]
                Executor::AsyncExecutor => {
                    let task = AsyncExecutorTask::spawn(future);
                    JoinHandle::AsyncExecutor(task)
                }
                #[cfg(feature = "tokio-compat")]
                Executor::Tokio => {
                    let handle = tokio::spawn(future);
                    JoinHandle::Tokio(handle)
                }
                #[cfg(feature = "smol-compat")]
                Executor::Smol => {
                    let task = SmolTask::spawn(future);
                    JoinHandle::Smol(task)
                }
                #[cfg(feature = "async-std-compat")]
                Executor::AsyncStd => {
                    let handle = async_std::task::spawn(future);
                    JoinHandle::AsyncStd(handle)
                }
            })
        } else {
            panic!("Spawner::spawn must be called within the context of Executor::run");
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Executor, JoinHandle, SpawnError, Spawner};

    #[test]
    #[should_panic]
    fn spawn_no_executor_set_fails() {
        let _ = Spawner::spawn(async { 2 + 2 });
    }

    #[test]
    fn async_executor_spawn() -> Result<(), SpawnError> {
        Executor::AsyncExecutor.run(|| {
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
            })
        })
    }

    #[test]
    fn tokio_spawn() -> Result<(), SpawnError> {
        Executor::Tokio.run(|| {
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
        })
    }

    #[test]
    fn smol_spawn() -> Result<(), SpawnError> {
        Executor::Smol.run(|| {
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
            })
        })
    }

    #[test]
    fn async_std_spawn() -> Result<(), SpawnError> {
        Executor::AsyncStd.run(|| {
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
        })
    }

    #[test]
    #[should_panic]
    fn tokio_cannot_spawn_async_executor() {
        Executor::Tokio.run(|| {
            let ex = async_executor::Executor::new();
            ex.run(async {
                let _ = Spawner::spawn(async { 2 + 2 });
            });
        });
    }

    #[test]
    #[should_panic]
    fn async_executor_cannot_spawn_tokio() {
        Executor::AsyncExecutor.run(|| {
            let mut ex = tokio::runtime::Runtime::new().unwrap();
            ex.block_on(async {
                let _ = Spawner::spawn(async { 2 + 2 });
            });
        });
    }
}

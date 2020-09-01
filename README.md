# Malebolgia

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](
https://github.com/ian-p-cooke/malebolgia)
[![Cargo](https://img.shields.io/crates/v/malebolgia.svg)](
https://crates.io/crates/malebolgia)

Async Executor access for spawning futures and blocking functions from libraries.

This is the implementation of an idea for how a library can spawn tasks without caring about what executor is going to take care of it.
It provides a JoinHandle that can be awaited, detached, or canceled.  The detach and cancel methods are experimental.  If the desired executor does not support canceling then it uses Abortable from the futures crate to provide that.
A method for spawning a function that blocks is also provided.  spawn_blocking is also experimental and will use the blocking crate to provide the feature if it is not provided by the desired executor.

For library authors you use `Spawner` to spawn tasks instead of a specific runtime.

For application authors you use `Executor` to set an executor before any library calls `spawn`.

[async-executors](https://docs.rs/async_executors) is the most similar crate.  `malebolgia` came about because I wanted something more simple that I could use for testing the performance of several Executors.

[Malebolgia](https://en.wikipedia.org/wiki/Malebolgia) is a fictional super-villain that created [Spawn](https://en.wikipedia.org/wiki/Spawn_%28comics%29).

## Examples

In a library:

```rust,no_run

use malebolgia::Spawner;

async fn process(keys: Vec<&str>) -> Result<(), Error> {
    let mut fetch_tasks = Vec::with_capacity(keys.len());
    for key in keys {
        let task = async move {
            // process key
            Ok::<(), Error>(())
        };
        fetch_tasks.push(Spawner::spawn(task));
    }

    for task in fetch_tasks.drain(..) {
        task.await??;
    }
    Ok(())
}
```

In an application:

```rust,no_run
use malebolgia::Executor;

#[tokio::main]
async fn main() -> Result<(), Error> {
    Executor::Tokio.set()?;

    let keys = vec!["a", "b", "c"];
  
    KeyProcessor::process(keys).await?;

    Ok(())
}
```

## Compatibility

Support is provided for the popular executors.
In order to use malebolgia in a test or an application you MUST select a feature for the executor runtime that you desire and then set an `Executor` before the `Spawner` is used. 

```toml
[dependencies]
malebolgia = { version = "0.4", features = ["tokio-compat"] }
```

 * [async-std](https://docs.rs/async-std)
 * [futures](https://docs.rs/futures)
 * [smol](https://docs.rs/smol)
 * [tokio](https://docs.rs/tokio)

## License

 * MIT license ([LICENSE](LICENSE) or http://opensource.org/licenses/MIT)

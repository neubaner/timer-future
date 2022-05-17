#![deny(rust_2018_idioms)]
use std::{sync::Arc, time::{Duration, Instant}, future::Future};
use timer_future::{Executor, Reactor};

async fn elapsed(future: impl Future) -> Duration {
    let now = Instant::now();
    future.await;
    let elapsed = now.elapsed();
    println!("Took {} seconds", elapsed.as_secs_f64());

    elapsed
}

fn main() -> Result<(), std::io::Error> {
    let reactor = Reactor::new()?;
    let executor = Executor::new(Arc::clone(&reactor));

    let total_elapsed = executor.block_on(async move {
        let elapsed1 = elapsed(timer_future::sleep(Duration::from_secs(3), Arc::clone(&reactor))).await;
        let elapsed2 = elapsed(timer_future::sleep(Duration::from_secs(5), Arc::clone(&reactor))).await;

        elapsed1 + elapsed2
    });

    println!("Total time took {} seconds", total_elapsed.as_secs_f64());

    Ok(())
}

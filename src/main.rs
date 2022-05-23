#![deny(rust_2018_idioms)]
use std::{
    future::Future,
    io,
    time::{Duration, Instant},
};
use timer_future::Executor;

async fn elapsed(future: impl Future) -> Duration {
    let now = Instant::now();
    future.await;
    let elapsed = now.elapsed();
    println!("Took {} seconds", elapsed.as_secs_f64());

    elapsed
}

#[allow(dead_code)]
async fn spawn_test() -> io::Result<()> {
    let executor = Executor::new()?;

    let join_handle = executor.spawn(async {
        timer_future::sleep(Duration::from_secs(3)).await;
        1
    });

    let one = join_handle.await;
    assert_eq!(one, 1);

    Ok(())
}

fn main() -> Result<(), std::io::Error> {
    let executor = Executor::new()?;

    let total_elapsed = executor.block_on(async {
        let join = executor.spawn(async {
            timer_future::sleep(Duration::from_secs(9)).await;
            2
        });

        let elapsed1 = elapsed(timer_future::sleep(Duration::from_secs(3))).await;
        let elapsed2 = elapsed(timer_future::sleep(Duration::from_secs(5))).await;

        join.await;
        
        elapsed1 + elapsed2
    });

    println!("Total time took {} seconds", total_elapsed.as_secs_f64());

    Ok(())
}

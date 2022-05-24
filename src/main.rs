#![deny(rust_2018_idioms)]
use std::{
    cmp,
    future::Future,
    io,
    time::{Duration, Instant},
};
use timer_future::Executor;

async fn elapsed<F: Future>(future: F) -> (F::Output, Duration) {
    let now = Instant::now();
    let result = future.await;
    let elapsed = now.elapsed();

    (result, elapsed)
}

fn main() -> io::Result<()> {
    let executor = Executor::new()?;

    let total_elapsed = executor.block_on(async {
        let join = executor.spawn(elapsed(async {
            timer_future::sleep(Duration::from_secs(9)).await;
            2
        }));

        let (_, elapsed1) = elapsed(timer_future::sleep(Duration::from_secs(3))).await;
        println!("First sleep took {} seconds", elapsed1.as_secs_f64());
        let (_, elapsed2) = elapsed(timer_future::sleep(Duration::from_secs(5))).await;
        println!("Second sleep took {} seconds", elapsed2.as_secs_f64());

        let (join_result, elapsed_join) = join.await;
        println!(
            "Spawned task returned {} and took {}",
            join_result,
            elapsed_join.as_secs_f64()
        );

        cmp::max(elapsed1 + elapsed2, elapsed_join)
    });

    println!("Total time took {} seconds", total_elapsed.as_secs_f64());

    Ok(())
}

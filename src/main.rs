use std::{sync::Arc, time::Duration};

use timer_future::{Executor, Reactor};

fn main() -> Result<(), std::io::Error> {
    let reactor = Reactor::new()?;
    let executor = Executor::new(Arc::clone(&reactor));

    let total_elapsed = executor.block_on(async move {
        let elapsed1 = timer_future::sleep(Duration::from_secs(3), Arc::clone(&reactor))
            .await
            .as_secs_f64();

        println!("Slept for: {elapsed1}");

        let elapsed2 = timer_future::sleep(Duration::from_secs(5), Arc::clone(&reactor))
            .await
            .as_secs_f64();
        println!("Slept for: {elapsed2}");

        elapsed1 + elapsed2
    });

    println!("Total elapsed is {total_elapsed}");

    Ok(())
}

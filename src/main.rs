use std::{sync::Arc, time::Duration};

use timer_future::{Executor, Reactor};

fn main() -> Result<(), std::io::Error> {
    let reactor = Reactor::new()?;
    let executor = Executor::new(Arc::clone(&reactor));

    executor.block_on(async move {
        let elapsed = timer_future::sleep(Duration::from_secs(3), Arc::clone(&reactor))
            .await
            .as_secs_f64();

        println!("Sleeping for: {elapsed}");

        let elapsed = timer_future::sleep(Duration::from_secs(5), Arc::clone(&reactor))
            .await
            .as_secs_f64();
        println!("Sleeping for: {elapsed}");
    });

    Ok(())
}

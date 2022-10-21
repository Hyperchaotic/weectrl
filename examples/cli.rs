use futures::prelude::*;
use std::time::Duration;
use tokio::io::{self, AsyncBufReadExt};
use weectrl::{DiscoveryMode, WeeController};

#[tokio::main]
async fn main() {
    //   Uncomment below to see tracing from the library
    //use tracing_subscriber::fmt::time;
    //tracing_subscriber::fmt()
    //    .with_timer(time::LocalTime::rfc_3339())
    //    .init();

    println!("Instantiating WeeController.\n");
    let controller = WeeController::new();

    // Find switches on the network.
    let mut discovery_future = controller.discover_future(
        DiscoveryMode::CacheAndBroadcast,
        true,
        Duration::from_secs(5),
    );

    println!("Searching for devices...\n");
    while let Some(d) = discovery_future.next().await {
        println!(
            " Found device {}, ID: {}, state: {:?}.",
            d.friendly_name, d.unique_id, d.state
        );
        let res = controller.subscribe(
            &d.unique_id,
            Duration::from_secs(120),
            true,
            Duration::from_secs(5),
        );
        println!(
            " - Subscribed {} - {:?} seconds to resubscribe.\n",
            d.friendly_name,
            res.unwrap()
        );
    }

    // Create notification future.
    let notifier_future = controller.notify_future();
    let notifier = notifier_future.for_each(|o| {
        if let Ok(e) = controller.get_info(&o.unique_id) {
            println!(
                " Device {} sent state update {:?}",
                e.friendly_name, o.state
            );
        }
        future::ready(())
    });

    // Input buffer, for keyboard cancel on Enter.
    let mut buf = String::new();
    let mut reader = io::BufReader::new(io::stdin());
    let quit = reader.read_line(&mut buf);

    println!("\n\nListening for notifications from switches.");
    println!("Press Enter to quit.\n");
    tokio::select! {
        _ = quit => (),

        _ = notifier => (),
    }
}

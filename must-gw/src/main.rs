use std::time::Duration;

use log::error;
use loragw::RxPacket;
use must_gw::{create_concentrator, node};
use must_hop::node::{
    MHNode,
    mesh_router::MeshRouter,
    network_manager::NetworkManager,
    policy::{GatewayPolicy, NodePolicy, RandomAccessMac},
};
use std::io::Write;
use tokio::time::{Instant, sleep};

async fn run_concentrator_task() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Now try and use loragw:");

    let mut conc = match create_concentrator() {
        Ok(concc) => concc,
        Err(e) => {
            log::error!("Error creating concentrator: {:?}", e);
            // We return the error here instead of just returning empty
            return Err(e.into());
        }
    };

    log::info!("check receive status");
    match conc.receive_status() {
        Ok(status) => log::info!("Receive status: {:?}", status),
        Err(e) => log::error!("Error checking receive status: {:?}", e),
    }
    let tty_path = "/dev/serial0";
    let gps_family = "ubx8";

    match conc.enable_gps(tty_path, gps_family) {
        Ok(_) => log::info!("GPS enabled successfully on {}!", tty_path),
        Err(e) => {
            log::error!("Error enabling gps: {:?}", e)
        }
    }

    // loop {
    //     if let Err(e) = conc.process_gps_frames() {
    //         log::error!("Error processing GPS frames: {:?}", e);
    //         sleep(Duration::from_millis(500)).await;
    //         continue;
    //     }
    //     match conc.get_gps() {
    //         Ok((coords, duration)) => {
    //             log::info!(
    //                 "GPS Fix -> Lat: {:.6}, Lon: {:.6}, Alt: {}m",
    //                 coords.lat, coords.lon, coords.alt
    //             );
    //             log::info!("dur: {:?}", duration.as_millis());
    //         }
    //         Err(_) => {
    //             // Ignore the error! It just means the GPS is still searching for satellites
    //             // or we haven't read enough valid frames yet.
    //         }
    //     }
    //     sleep(Duration::from_millis(500)).await;
    // }

    log::info!("now try receive!");
    let node = node::GWNode::new(conc);

    // let mut rec_buf: Vec<RxPacket> = Vec::new(); // Make sure RxPacket is imported
    // log::info!("listening again ...");
    // node.listen(&mut rec_buf, false).await?;
    // let pkt = node.receive((), &rec_buf).await?;
    // log::info!("got pkts: {:?} ", pkt);

    log::info!("Now making mesh router ...");
    let mut router = MeshRouter::new(
        node,
        NetworkManager::new(0, 10, 3),
        RandomAccessMac,
        GatewayPolicy::new(60),
    );
    log::info!("Now start loop..");
    loop {
        let mut rec_buf = Vec::new();
        match router.tick(&mut rec_buf).await {
            Ok(res) => {
                if !res.is_empty() {
                    log::info!("got pkts: {:?}", res)
                }
            }
            Err(e) => error!("Error in ticking: {:?}", e),
        }
        // router.listen(&mut rec_buf).await?;
        // let pkts = router.receive((), &rec_buf).await?;
        // if !pkts.is_empty() {
        //     log::info!("got pkts! : {:?}", pkts);
        // }
    }
}

#[tokio::main]
async fn main() {
    let start_time = Instant::now();
    // To get logging from loragw
    env_logger::Builder::from_default_env()
        .format(move |buf, record| {
            let elapsed = start_time.elapsed();

            let file = record.file().unwrap_or("unknown");
            let line = record.line().unwrap_or(0);

            // 1. Pick the color for the log level
            let level_color = match record.level() {
                log::Level::Error => "\x1b[31m", // Red
                log::Level::Warn => "\x1b[33m",  // Yellow
                log::Level::Info => "\x1b[32m",  // Green
                log::Level::Debug => "\x1b[34m", // Blue
                log::Level::Trace => "\x1b[90m", // Gray (Bright Black)
            };

            // 2. Define the gray color for the file path, and the reset code
            let gray = "\x1b[90m";
            let reset = "\x1b[0m"; // Turns formatting back to normal

            // 3. Paint the string!
            writeln!(
                buf,
                "{}.{:06} [{}{:>5}{}] {} {}({} {}:{}){}",
                elapsed.as_secs(),
                elapsed.subsec_micros(),
                level_color, // Start level color
                record.level(),
                reset,         // Reset after level
                record.args(), // The actual message
                gray,          // Start gray for the file info
                record.target(),
                file,
                line,
                reset // Reset at the very end
            )
        })
        .init();

    log::info!("Spawning concentrator task...");

    // 3. Spawn the task using tokio::spawn
    let task_handle = tokio::spawn(async move {
        // Run the task and catch any errors it throws
        if let Err(e) = run_concentrator_task().await {
            log::error!("Concentrator task shut down with error: {:?}", e);
        }
    });

    // 4. Await the handle. If you don't await something in main,
    // the program will immediately exit and kill your spawned tasks!
    match task_handle.await {
        Ok(_) => log::info!("Task finished cleanly."),
        Err(e) => log::error!("Task panicked or was cancelled: {:?}", e),
    }
}

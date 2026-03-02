use std::time::Duration;

use loragw::RxPacket;
use must_gw::{create_concentrator, node};
use must_hop::node::{
    MHNode,
    mesh_router::MeshRouter,
    network_manager::NetworkManager,
    policy::{NodePolicy, RandomAccessMac},
};
use tokio::time::sleep;

async fn run_concentrator_task() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("Now try and use loragw:");

    let mut conc = match create_concentrator() {
        Ok(concc) => concc,
        Err(e) => {
            eprintln!("Error creating concentrator: {:?}", e);
            // We return the error here instead of just returning empty
            return Err(e.into());
        }
    };

    println!("check receive status");
    match conc.receive_status() {
        Ok(status) => println!("Receive status: {:?}", status),
        Err(e) => eprintln!("Error checking receive status: {:?}", e),
    }
    let tty_path = "/dev/serial0";
    let gps_family = "ubx8";

    match conc.enable_gps(tty_path, gps_family) {
        Ok(_) => println!("GPS enabled successfully on {}!", tty_path),
        Err(e) => {
            eprintln!("Error enabling gps: {:?}", e)
        }
    }

    loop {
        if let Err(e) = conc.process_gps_frames() {
            eprintln!("Error processing GPS frames: {:?}", e);
            sleep(Duration::from_millis(500)).await;
            continue;
        }
        match conc.get_gps() {
            Ok((coords, duration)) => {
                println!(
                    "GPS Fix -> Lat: {:.6}, Lon: {:.6}, Alt: {}m",
                    coords.lat, coords.lon, coords.alt
                );
                println!("dur: {:?}", duration);
            }
            Err(_) => {
                // Ignore the error! It just means the GPS is still searching for satellites
                // or we haven't read enough valid frames yet.
            }
        }
        sleep(Duration::from_millis(500)).await;
    }
    println!("now try receive!");
    let mut node = node::GWNode::new(conc);

    let mut rec_buf: Vec<RxPacket> = Vec::new(); // Make sure RxPacket is imported
    println!("listening again ...");
    node.listen(&mut rec_buf, false).await?;
    let pkt = node.receive((), &rec_buf).await?;
    println!("got pkts: {:?} ", pkt);

    println!("Now making mes router ...");
    let mut router = MeshRouter::new(
        node,
        NetworkManager::new(0, 10, 3),
        RandomAccessMac,
        NodePolicy,
    );
    loop {
        let mut rec_buf = Vec::new();
        router.listen(&mut rec_buf).await?;
        let pkts = router.receive((), &rec_buf).await?;
        if !pkts.is_empty() {
            println!("got pkts! : {:?}", pkts);
        }
    }
}

#[tokio::main]
async fn main() {
    // To get logging from loragw
    env_logger::init();

    println!("Spawning concentrator task...");

    // 3. Spawn the task using tokio::spawn
    let task_handle = tokio::spawn(async move {
        // Run the task and catch any errors it throws
        if let Err(e) = run_concentrator_task().await {
            eprintln!("Concentrator task shut down with error: {:?}", e);
        }
    });

    // 4. Await the handle. If you don't await something in main,
    // the program will immediately exit and kill your spawned tasks!
    match task_handle.await {
        Ok(_) => println!("Task finished cleanly."),
        Err(e) => eprintln!("Task panicked or was cancelled: {:?}", e),
    }
}

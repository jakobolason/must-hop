use loragw::RxPacket;
use must_gw::{create_concentrator, node};
use must_hop::node::MHNode;

async fn run_concentrator_task() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("Now try and use loragw:");

    let conc = match create_concentrator() {
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

    println!("now try receive!");
    let mut node = node::GWNode::new(conc);

    loop {
        let mut rec_buf: Vec<RxPacket> = Vec::new(); // Make sure RxPacket is imported

        // The ? operator works here because the function returns a Result
        node.listen(&mut rec_buf, false).await?;

        // Process _conn and rec_buf here...
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

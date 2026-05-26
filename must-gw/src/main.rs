use std::time::{SystemTime, UNIX_EPOCH};

use log::error;
use loragw::{Bandwidth, Spreading};
use must_gw::{create_concentrator, node};
use must_hop::{mesh_router::MeshRouter, network_manager::NetworkManager, policy::tdma::TdmaMac};
use rppal::gpio::Gpio;
use std::io::Write;
use tokio::time::Instant;

async fn run_concentrator_task() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let spreading_env = std::env::var("SF").unwrap_or("None".to_string());
    log::info!("SPREADING ENV: {}", spreading_env);
    let spreading = match spreading_env.as_str() {
        "5" => Spreading::SF5,
        "6" => Spreading::SF6,
        "7" => Spreading::SF7,
        "8" => Spreading::SF8,
        "9" => Spreading::SF9,
        "10" => Spreading::SF10,
        "11" => Spreading::SF11,
        "12" => Spreading::SF12,
        _ => panic!("SF should be between 5 and 12!"),
    };
    let bandwidth = match std::env::var("BW").as_deref() {
        Ok("125") => Bandwidth::BW125kHz,
        Ok("250") => Bandwidth::BW250kHz,
        Ok("500") => Bandwidth::BW500kHz,
        _ => panic!("BW can either be 125, 250 or 500kHz!"),
    };
    log::info!("radio params: SF={:?} BW={:?}", spreading, bandwidth);

    let conc = match create_concentrator(spreading, bandwidth) {
        Ok(conc) => conc,
        Err(e) => {
            log::error!("Error creating concentrator: {:?}", e);
            return Err(e.into());
        }
    };

    // SF5 and SF6 require a minimum preamble of 12 symbols on SX1262 end-devices
    // (the SX1302 HAL documents this as "aligned with end-device drivers").
    // SF7–SF12 use the standard 8-symbol preamble.
    let preamble = match spreading {
        Spreading::SF5 | Spreading::SF6 => Some(12),
        _ => Some(8),
    };
    let pkt_params = node::PacketParams {
        spreading,
        bandwidth,
        preamble,
        ..node::PacketParams::default()
    };
    let node = node::GWNode::new(conc, pkt_params);

    let gw_source_id = 1;
    let gpio = Gpio::new().expect("Failed to initialize RPPAL GPIO");
    let sync_pin = gpio.get(21).expect("Failed to get GPIO 21").into_output();
    let tau_hb = 10;

    let mac = TdmaMac::default()
        .set_debug_pin(sync_pin)
        .set_node_id(1)
        .set_tx_slot(1)
        .set_time_sync((
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Time went backwards")
                .as_micros() as u64,
            embassy_time::Instant::now(),
        ))
        .set_tau_hb(tau_hb)
        .node_is_leader()
        .build();

    // let mac = RandomAccessMac::new(GatewayPolicy::new(tau_hb));
    log::info!("Now making mesh router ...");
    let mut router = MeshRouter::new(node, NetworkManager::new(gw_source_id, 10, 3), mac);
    log::info!("Now start loop..");
    loop {
        let mut rec_buf = None;
        match router.tick(&mut rec_buf).await {
            Ok(res) => {
                if !res.is_empty() {
                    log::info!("got pkts: {:?}", res)
                }
            }
            Err(e) => error!("Error in ticking: {:?}", e),
        }
    }
}

#[tokio::main]
async fn main() {
    let start_time = Instant::now();
    // To get logging from loragw
    env_logger::Builder::from_default_env()
        .format(move |buf, record| {
            let elapsed = start_time.elapsed();
            // let file = record.file().unwrap_or("unknown");
            let line = record.line().unwrap_or(0);

            let level_color = match record.level() {
                log::Level::Error => "\x1b[31m",
                log::Level::Warn => "\x1b[33m",
                log::Level::Info => "\x1b[32m",
                log::Level::Debug => "\x1b[34m",
                log::Level::Trace => "\x1b[90m",
            };

            let gray = "\x1b[90m";
            let reset = "\x1b[0m";

            writeln!(
                buf,
                "{}.{:06} [{}{:>5}{}] {} {}({}:{} ){}",
                elapsed.as_secs(),
                elapsed.subsec_micros(),
                level_color,
                record.level(),
                reset,
                record.args(),
                gray,
                record.target(),
                // file,
                line,
                reset
            )
        })
        .init();

    log::info!("Spawning concentrator task...");

    let task_handle = tokio::spawn(async move {
        if let Err(e) = run_concentrator_task().await {
            log::error!("Concentrator task shut down with error: {:?}", e);
        }
    });

    match task_handle.await {
        Ok(_) => log::info!("Task finished cleanly."),
        Err(e) => log::error!("Task panicked or was cancelled: {:?}", e),
    }
}

use clap::Parser;
use must_dash::app::NodeConfig;
use must_dash::{
    app::AppEvent, init_logger, shutdown_processes, spawn_log_processes, spawn_pty_reader,
};
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::mpsc;

/// A `node_id:probe_id` pair passed as a positional argument.
/// The probe serial itself may contain colons, so we split on the first one only.
#[derive(Clone, Debug)]
struct NodeArg {
    node_id: String,
    probe_id: String,
}

impl FromStr for NodeArg {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.split_once(':')
            .map(|(id, probe)| NodeArg {
                node_id: id.to_owned(),
                probe_id: probe.to_owned(),
            })
            .ok_or_else(|| format!("Expected node_id:probe_id, got {s:?}"))
    }
}

#[derive(Parser)]
#[command(about = "Headless experiment runner — same log processing as must-dash, no TUI")]
struct Args {
    /// Spreading factor (5–12)
    #[arg(long, default_value = "7")]
    sf: String,

    /// Bandwidth in kHz (125, 250, 500)
    #[arg(long, default_value = "125")]
    bw: String,

    /// Proportional gain
    #[arg(long, default_value = "0.5")]
    kp: String,

    /// Integral gain
    #[arg(long, default_value = "0.4")]
    ki: String,

    /// Run duration in seconds
    #[arg(long, default_value_t = 600)]
    duration: u64,

    /// Nodes in node_id:probe_id format (probe serial may contain colons)
    #[arg(required = true)]
    nodes: Vec<NodeArg>,
}

#[tokio::main]
async fn main() {
    init_logger();

    let args = Args::parse();
    eprintln!(
        "[headless] SF={} BW={} KP={} KI={} nodes={:?} duration={}s",
        args.sf, args.bw, args.kp, args.ki, args.nodes, args.duration,
    );

    let mut app = must_dash::app::App::new(false);
    for (i, node) in args.nodes.iter().enumerate() {
        app.configured_nodes.push(NodeConfig {
            probe_index: i,
            probe_name: format!("probe-{}", node.node_id),
            probe_id: node.probe_id.clone(),
            kp: args.kp.clone(),
            ki: args.ki.clone(),
            source_id: node.node_id.clone(),
            sf: args.sf.clone(),
            bw: args.bw.clone(),
        });
    }
    app.defaults.sf = args.sf;
    app.defaults.bw = args.bw;

    let (tx, mut rx) = mpsc::channel(100);
    let descriptors = app.build_descriptors();
    app.init_sources(&descriptors);
    let mut log_children = spawn_log_processes(&descriptors, tx.clone());
    let delay_child = Some(spawn_pty_reader(
        "just",
        &["run-delay"],
        &[],
        tx.clone(),
        |delay_ms, _| AppEvent::HardwareLog { delay_ms },
    ));

    let deadline = tokio::time::Instant::now() + Duration::from_secs(args.duration);

    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(AppEvent::ProcessLog {
                source,
                text,
                overwrite,
            })) => {
                println!("{source} => {text}");
                app.add_log(&source, text, overwrite);
            }
            Ok(Some(AppEvent::HardwareLog { delay_ms })) => {
                println!("hw delay: {delay_ms}");
                app.add_hw_delay(delay_ms);
            }
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break,
        }
    }

    eprintln!("[headless] Run complete — saving data...");
    let (main_path, hw_path) = app.save_data();
    if let Some(p) = &main_path {
        eprintln!("[headless:data] main_stats={p}");
    }
    if let Some(p) = &hw_path {
        eprintln!("[headless:data] hw_stats={p}");
    }

    shutdown_processes(
        log_children
            .drain(..)
            .map(Some)
            .chain([delay_child])
            .collect(),
    )
    .await;

    eprintln!("[headless] Done.");
}

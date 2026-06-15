use clap::Parser;
use must_dash::app::NodeConfig;
use must_dash::{
    app::AppEvent, init_logger, shutdown_processes, spawn_log_processes, spawn_pty_reader,
};
use std::io::Write;
use std::str::FromStr;
use std::time::Duration;
use strip_ansi_escapes::strip_str;
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

    /// Heartbeat duration
    #[arg(long, default_value = "10")]
    tau: String,

    /// Alternate receive SF
    #[arg(long, default_value = "None")]
    alt_sf: String,

    /// Run duration in seconds
    #[arg(long, default_value_t = 600)]
    duration: u64,

    /// Nodes in node_id:probe_id format (probe serial may contain colons)
    #[arg(required = true)]
    nodes: Vec<NodeArg>,
}

/// Same probe-serial → DIN mapping as main.rs.
const DIN_MAP: &[(&str, u8)] = &[("1366:0101:000801024520", 0), ("1366:0101:000801024472", 1)];

#[tokio::main]
async fn main() {
    init_logger();

    let args = Args::parse();
    eprintln!(
        "[headless] SF={} BW={} KP={} KI={} nodes={:?} tau={} alt_sf={} duration={}s",
        args.sf, args.bw, args.kp, args.ki, args.nodes, args.tau, args.alt_sf, args.duration,
    );
    let alt_sf = if args.alt_sf == "None" {
        String::new()
    } else {
        args.alt_sf
    };

    let mut app = must_dash::app::App::new(false, DIN_MAP);
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
            tau: args.tau.clone(),
            alt_sf: alt_sf.clone(),
        });
    }
    app.gateway_config.sf = args.sf.clone();
    app.gateway_config.bw = args.bw.clone();
    app.gateway_config.tau = args.tau.clone();

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

    let mut last_gw = String::new();
    let mut last_node = String::new();
    let mut last_hw = String::new();
    let mut prev_lines: usize = 0;

    let term_cols = crossterm::terminal::size()
        .map(|(w, _)| w as usize)
        .unwrap_or(200);
    let truncate = |s: &str, prefix_len: usize| -> String {
        let max = term_cols.saturating_sub(prefix_len);
        let s = s.trim_end();
        if s.chars().count() > max {
            s.chars().take(max).collect()
        } else {
            s.to_owned()
        }
    };

    loop {
        tokio::select! {
            result = tokio::time::timeout_at(deadline, rx.recv()) => {
                match result {
                    Ok(Some(AppEvent::ProcessLog { source, text, overwrite })) => {
                        let clean = truncate(&strip_str(&text), source.len() + 4);
                        if source == "gw" {
                            last_gw = format!("{source} => {clean}");
                        } else {
                            last_node = format!("{source} => {clean}");
                        }
                        let status = format!("hw:   {last_hw}\ngw:   {last_gw}\nnode: {last_node}");
                        if prev_lines > 0 { print!("\x1B[{prev_lines}A\r\x1B[J"); }
                        print!("{status}");
                        prev_lines = status.lines().count();
                        let _ = std::io::stdout().flush();
                        app.add_log(&source, text, overwrite);
                    }
                    Ok(Some(AppEvent::HardwareLog { delay_ms })) => {
                        last_hw = format!("{delay_ms}ms");
                        let status = format!("hw:   {last_hw}\ngw:   {last_gw}\nnode: {last_node}");
                        if prev_lines > 0 { print!("\x1B[{prev_lines}A\r\x1B[J"); }
                        print!("{status}");
                        prev_lines = status.lines().count();
                        let _ = std::io::stdout().flush();
                        app.add_hw_delay(delay_ms);
                    }
                    Ok(Some(_)) => {}
                    Ok(None) | Err(_) => break,
                }
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\n[headless] Interrupted — shutting down...");
                break;
            }
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

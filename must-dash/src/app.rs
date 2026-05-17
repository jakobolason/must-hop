use chrono::Local;
use dotenv::dotenv;
use std::fs;
use std::path::Path;
use std::{env, fs::File, io::Write, str::FromStr};

use crossterm::event::KeyCode;

use crate::composables::stats::DashStats;

const KI_DEFAULT: &str = "0.4";
const KP_DEFAULT: &str = "0.5";

pub enum AppEvent {
    Input(KeyCode),
    ProcessLog {
        source: String,
        text: String,
        overwrite: bool,
    },
    HardwareLog {
        delay_ms: String,
    },
    Tick,
}

#[derive(Clone, Copy)]
pub enum LogRole {
    Node,
    Gateway,
}

pub struct LogSource {
    pub id: String,
    pub role: LogRole,
    pub logs: Vec<String>,
}

impl LogSource {
    pub fn new(id: impl Into<String>, role: LogRole) -> Self {
        Self {
            id: id.into(),
            role,
            logs: Vec::new(),
        }
    }
}

pub struct ProcessDescriptor {
    pub source_id: String,
    pub role: LogRole,
    pub command: String,
    pub args: Vec<String>,
    pub envs: Vec<(String, String)>,
}

pub struct ProbeInfo {
    pub index: usize,
    pub name: String,
}

pub struct NodeConfig {
    pub probe_index: usize,
    pub probe_name: String,
    pub kp: String,
    pub ki: String,
    pub source_id: String,
}

/// Which field is active in the ProbeConfig form. Defined here so both
/// app.rs (field editing) and navigator.rs (cursor tracking) can use it
/// without circular imports.
#[derive(Clone, Copy, PartialEq)]
pub enum ProbeConfigFocus {
    Kp,
    Ki,
    SourceId,
    Confirm,
}

/// KP/KI defaults that persist across sessions, loaded from .env.
pub struct Defaults {
    pub kp: String,
    pub ki: String,
}

pub struct App {
    pub defaults: Defaults,
    pub available_probes: Vec<ProbeInfo>,
    pub probe_fetch_error: Option<String>,
    pub configured_nodes: Vec<NodeConfig>,
    pub pending_node: Option<NodeConfig>,
    pub sources: Vec<LogSource>,
    pub dash_stats: DashStats,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        let _ = dotenv();
        let kp = env::var("KP").unwrap_or_else(|_| KP_DEFAULT.to_string());
        let ki = env::var("KI").unwrap_or_else(|_| KI_DEFAULT.to_string());

        let mut app = Self {
            defaults: Defaults { kp, ki },
            available_probes: Vec::new(),
            probe_fetch_error: None,
            configured_nodes: Vec::new(),
            pending_node: None,
            sources: Vec::new(),
            dash_stats: DashStats::new(),
        };
        app.fetch_probes();
        app
    }

    pub fn fetch_probes(&mut self) {
        match std::process::Command::new("just")
            .arg("probe-list")
            .output()
        {
            Ok(output) => {
                let text = String::from_utf8_lossy(&output.stdout);
                self.available_probes = parse_probe_list(&text);
                self.probe_fetch_error = if self.available_probes.is_empty() {
                    Some("No probes found. Connect a probe and press R to refresh.".to_string())
                } else {
                    None
                };
            }
            Err(e) => {
                self.available_probes.clear();
                self.probe_fetch_error = Some(format!("probe-rs not found or failed: {e}"));
            }
        }
    }

    pub fn start_configuring_probe(&mut self, list_index: usize) {
        if let Some(probe) = self.available_probes.get(list_index) {
            self.pending_node = Some(NodeConfig {
                probe_index: probe.index,
                probe_name: probe.name.clone(),
                kp: self.defaults.kp.clone(),
                ki: self.defaults.ki.clone(),
                source_id: String::new(),
            });
        }
    }

    pub fn confirm_pending_node(&mut self) {
        if let Some(node) = self.pending_node.take() {
            self.defaults.kp = node.kp.clone();
            self.defaults.ki = node.ki.clone();
            self.configured_nodes.push(node);
        }
    }

    pub fn pop_configured_node(&mut self) {
        self.configured_nodes.pop();
    }

    pub fn type_char_pending(&mut self, c: char, focus: ProbeConfigFocus) {
        if let Some(node) = &mut self.pending_node {
            match focus {
                ProbeConfigFocus::Kp => node.kp.push(c),
                ProbeConfigFocus::Ki => node.ki.push(c),
                ProbeConfigFocus::SourceId => node.source_id.push(c),
                ProbeConfigFocus::Confirm => {}
            }
        }
    }

    pub fn backspace_pending(&mut self, focus: ProbeConfigFocus) {
        if let Some(node) = &mut self.pending_node {
            match focus {
                ProbeConfigFocus::Kp => {
                    node.kp.pop();
                }
                ProbeConfigFocus::Ki => {
                    node.ki.pop();
                }
                ProbeConfigFocus::SourceId => {
                    node.source_id.pop();
                }
                ProbeConfigFocus::Confirm => {}
            }
        }
    }

    pub fn reset_data(&mut self) {
        self.sources.clear();
        self.dash_stats = DashStats::new();
    }

    pub fn init_sources(&mut self, descriptors: &[ProcessDescriptor]) {
        self.sources = descriptors
            .iter()
            .map(|d| LogSource::new(&d.source_id, d.role))
            .collect();
    }

    pub fn has_data(&self) -> bool {
        self.sources.iter().any(|s| !s.logs.is_empty())
    }

    pub fn build_descriptors(&self) -> Vec<ProcessDescriptor> {
        let cast_f32 = |s: &str, mult: f32| -> String {
            format!("{}", (s.parse::<f32>().unwrap_or(0.0) * mult) as u64)
        };
        let color_envs = || {
            vec![
                ("CARGO_TERM_COLOR".to_string(), "always".to_string()),
                ("CLICOLOR_FORCE".to_string(), "1".to_string()),
                ("DEFMT_LOG_STYLE".to_string(), "always".to_string()),
            ]
        };

        let mut descs = vec![ProcessDescriptor {
            source_id: "gw".to_string(),
            role: LogRole::Gateway,
            command: "just".to_string(),
            args: vec!["run-gw".to_string()],
            envs: color_envs(),
        }];

        for node in &self.configured_nodes {
            let mut envs = color_envs();
            envs.extend([
                ("KP".to_string(), cast_f32(&node.kp, 10.0)),
                ("KI".to_string(), cast_f32(&node.ki, 100.0)),
                ("SOURCEID".to_string(), node.source_id.clone()),
            ]);
            descs.push(ProcessDescriptor {
                source_id: format!("node-{}", node.source_id),
                role: LogRole::Node,
                command: "just".to_string(),
                args: vec!["remote-run".to_string(), node.source_id.clone()],
                envs,
            });
        }

        descs
    }

    pub fn save_to_env_file(&self) {
        let path = Path::new(".env");
        let content = if path.exists() {
            fs::read_to_string(path).unwrap_or_default()
        } else {
            String::new()
        };

        let mut kp_found = false;
        let mut ki_found = false;
        let mut new_content = String::new();

        for line in content.lines() {
            if line.starts_with("KP=") {
                new_content.push_str(&format!("KP={}\n", self.defaults.kp));
                kp_found = true;
            } else if line.starts_with("KI=") {
                new_content.push_str(&format!("KI={}\n", self.defaults.ki));
                ki_found = true;
            } else {
                new_content.push_str(line);
                new_content.push('\n');
            }
        }
        if !kp_found {
            new_content.push_str(&format!("KP={}\n", self.defaults.kp));
        }
        if !ki_found {
            new_content.push_str(&format!("KI={}\n", self.defaults.ki));
        }

        if let Err(e) = fs::write(path, new_content) {
            log::error!("Error saving env file: {:?}", e);
        }
    }

    pub fn save_data(&self) {
        let timestamp = Local::now().format("%d-%m:%H.%M").to_string();

        let prefix = "./analysis/data";
        let main_prefix = format!("{prefix}/main");
        let hw_prefix = format!("{prefix}/full_hw");
        let main_filename = format!("{main_prefix}/main_stats_{timestamp}.csv");
        let hw_filename = format!("{hw_prefix}/hw_stats_{timestamp}.csv");

        if let Err(e) = fs::create_dir_all(&main_prefix) {
            log::error!("Error in dir creation: {:?}", e);
            return;
        }
        if let Err(e) = fs::create_dir_all(&hw_prefix) {
            log::error!("Error in dir creation: {:?}", e);
            return;
        }

        if let Ok(mut f) = File::create(&main_filename) {
            let _ = writeln!(
                f,
                "delay_ms,err_ms,prev_speed,new_speed,\
                 delta_up_ms,delta_down_ms,mean_hw_delay_ms,\
                 gw_time_us,gw_bytes,node_time_us,node_bytes"
            );
            let opt_str = |v: Option<f32>| v.map_or(String::new(), |x| x.to_string());
            for (i, p) in self.dash_stats.packets.iter().enumerate() {
                let gw = self.dash_stats.gw_diff.get(i);
                let nd = self.dash_stats.node_diff.get(i);
                let _ = writeln!(
                    f,
                    "{},{},{},{},{},{},{},{},{},{},{}",
                    p.delay_ms,
                    p.err_ms,
                    p.prev_speed,
                    p.new_speed,
                    opt_str(p.delta_up_ms),
                    opt_str(p.delta_down_ms),
                    p.mean_hw_delay_ms,
                    gw.map_or(String::new(), |d| d.time_us.to_string()),
                    gw.map_or(String::new(), |d| d.bytes.to_string()),
                    nd.map_or(String::new(), |d| d.time_us.to_string()),
                    nd.map_or(String::new(), |d| d.bytes.to_string()),
                );
            }
        }

        if let Ok(mut f) = File::create(&hw_filename) {
            let _ = writeln!(f, "hardware_delay");
            for hw in &self.dash_stats.hardware_delay {
                let _ = writeln!(f, "{}", hw);
            }
        }
    }

    pub fn add_hw_delay(&mut self, log_str: String) {
        let parsed = log_str
            .split(':')
            .nth(1)
            .map(|s| s.trim().trim_end_matches("ms").trim())
            .and_then(|s| s.parse::<f32>().ok());
        if let Some(delay_ms) = parsed {
            self.dash_stats.hardware_delay.push(delay_ms);
        }
    }

    pub fn add_log(&mut self, source_id: &str, text: String, overwrite: bool) {
        let clean = strip_log(&text);
        if clean.trim().is_empty() && overwrite {
            return;
        }
        let role = self
            .sources
            .iter()
            .find(|s| s.id == source_id)
            .map(|s| s.role);
        if let Some(role) = role {
            if let Some(parts) = parse_piped(&clean) {
                self.parse_for_role(role, source_id, &parts);
            }
            if let Some(source) = self.sources.iter_mut().find(|s| s.id == source_id) {
                push_or_overwrite(&mut source.logs, text, overwrite);
            }
        }
    }

    fn parse_for_role(&mut self, role: LogRole, source_id: &str, parts: &[&str]) {
        let tag = parts[0];
        match role {
            LogRole::Node => {
                log::debug!("node({source_id}) | tag={tag:?} parts={:?}", &parts[1..]);
                if tag.contains("[SYNC]") && parts.len() >= 5 {
                    if let (Ok(delay), Ok(err), Ok(prev), Ok(new)) = (
                        extract::<f32>(parts[1]),
                        extract::<f32>(parts[2]),
                        extract::<f32>(parts[3]),
                        extract::<f32>(parts[4]),
                    ) {
                        self.dash_stats.on_sync(delay, err, prev, new);
                    }
                } else if tag.contains("[DELTAS]") && parts.len() >= 3 {
                    if let (Ok(up), Ok(down)) = (extract::<f32>(parts[1]), extract::<f32>(parts[2]))
                    {
                        self.dash_stats.on_deltas(up, down);
                    }
                } else if tag.contains("[SIZE EXPECTED]") && parts.len() >= 2 {
                    if let Ok(size) = extract::<usize>(parts[1]) {
                        log::info!("Got pre size {size}");
                        self.dash_stats.on_node_slice_size(size);
                    }
                } else if tag.contains("[TAU_SLICE]") && parts.len() >= 2 {
                    if let Ok(ts) = extract::<u64>(parts[1]) {
                        self.dash_stats.on_node_slice_pre(ts);
                    }
                } else if tag.contains("[TAU_SLICE_POST]")
                    && parts.len() >= 3
                    && let (Ok(ts), Ok(size)) =
                        (extract::<u64>(parts[1]), extract::<usize>(parts[2]))
                {
                    log::info!("Got post size: {size}");
                    self.dash_stats.on_node_slice_post(ts, size);
                }
            }
            LogRole::Gateway => {
                log::debug!("gw({source_id}) | tag={tag:?} parts={:?}", &parts[1..]);
                if tag.contains("[TAU_SLICE]") && parts.len() >= 2 {
                    if let Ok(ts) = extract::<u64>(parts[1]) {
                        self.dash_stats.on_gw_slice_pre(ts);
                    }
                } else if tag.contains("[TAU_SLICE_POST]")
                    && parts.len() >= 3
                    && let (Ok(ts), Ok(size)) =
                        (extract::<u64>(parts[1]), extract::<usize>(parts[2]))
                {
                    self.dash_stats.on_gw_slice_post(ts, size);
                } else if tag.contains("[SIZE EXPECTED]")
                    && parts.len() >= 2
                    && let Ok(size) = extract::<usize>(parts[1])
                {
                    log::info!("Got pre size {size}");
                    self.dash_stats.on_gw_slice_size(size);
                }
            }
        }
    }
}

fn parse_probe_list(output: &str) -> Vec<ProbeInfo> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if !line.starts_with('[') {
                return None;
            }
            let end = line.find(']')?;
            let index: usize = line[1..end].parse().ok()?;
            let name = line[end + 2..].trim().to_string();
            Some(ProbeInfo { index, name })
        })
        .collect()
}

fn strip_log(log: &str) -> String {
    let stripped = strip_ansi_escapes::strip(log.as_bytes());
    String::from_utf8_lossy(&stripped).into_owned()
}

fn parse_piped(clean: &str) -> Option<Vec<&str>> {
    if !clean.contains('|') {
        return None;
    }
    Some(clean.split('|').collect())
}

fn push_or_overwrite(logs: &mut Vec<String>, log: String, overwrite: bool) {
    if overwrite {
        match logs.last_mut() {
            Some(last) => *last = log,
            None => logs.push(log),
        }
    } else {
        logs.push(log);
    }
}

fn extract<T: FromStr>(part: &str) -> Result<T, T::Err> {
    part.trim().parse()
}

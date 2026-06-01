use chrono::Local;
use dotenv::dotenv;
use std::fs;
use std::path::Path;
use std::{env, fs::File, io::Write, str::FromStr};

use crossterm::event::KeyCode;

use crate::composables::stats::DashStats;

const KI_DEFAULT: &str = "0.4";
const KP_DEFAULT: &str = "0.5";
const SF_DEFAULT: &str = "7";
const BW_DEFAULT: &str = "125";
const TAU_DEFAULT: &str = "10";

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
    pub probe_id: String,
}

pub struct NodeConfig {
    pub probe_index: usize,
    pub probe_name: String,
    pub probe_id: String,
    pub kp: String,
    pub ki: String,
    pub source_id: String,
    pub sf: String,
    pub bw: String,
    pub tau: String,
}

/// Which field is active in the ProbeConfig form. Defined here so both
/// app.rs (field editing) and navigator.rs (cursor tracking) can use it
/// without circular imports.
#[derive(Clone, Copy, PartialEq)]
pub enum ProbeConfigFocus {
    Kp,
    Ki,
    SourceId,
    Sf,
    Bw,
    Tau,
    Confirm,
}

/// Defaults that persist across sessions, loaded from .env.
#[derive(Clone)]
pub struct Defaults {
    pub kp: String,
    pub ki: String,
    pub sf: String,
    pub bw: String,
    pub tau: String,
}

pub struct App {
    pub defaults: Defaults,
    pub available_probes: Vec<ProbeInfo>,
    pub probe_fetch_error: Option<String>,
    pub configured_nodes: Vec<NodeConfig>,
    pub pending_node: Option<NodeConfig>,
    /// Some(i) while editing configured_nodes[i]; None when adding a new node.
    pub editing_node_index: Option<usize>,
    pub sources: Vec<LogSource>,
    pub dash_stats: DashStats,
}

impl Default for App {
    fn default() -> Self {
        Self::new(true)
    }
}

impl App {
    pub fn new(fetch_probes: bool) -> Self {
        let _ = dotenv();
        let kp = env::var("KP").unwrap_or_else(|_| KP_DEFAULT.to_string());
        let ki = env::var("KI").unwrap_or_else(|_| KI_DEFAULT.to_string());
        let sf = env::var("SF").unwrap_or_else(|_| SF_DEFAULT.to_string());
        let bw = env::var("BW").unwrap_or_else(|_| BW_DEFAULT.to_string());
        let tau = env::var("TAU").unwrap_or_else(|_| TAU_DEFAULT.to_string());

        let mut app = Self {
            defaults: Defaults {
                kp,
                ki,
                sf,
                bw,
                tau,
            },
            available_probes: Vec::new(),
            probe_fetch_error: None,
            configured_nodes: Vec::new(),
            pending_node: None,
            editing_node_index: None,
            sources: Vec::new(),
            dash_stats: DashStats::new(),
        };
        if fetch_probes {
            app.fetch_probes();
        }
        app
    }

    pub fn fetch_probes(&mut self) {
        match std::process::Command::new("just")
            .arg("probe-list")
            .output()
        {
            Ok(output) => {
                let text = String::from_utf8_lossy(&output.stdout);
                let mut probes = parse_probe_list(&text);
                let used: std::collections::HashSet<usize> = self
                    .configured_nodes
                    .iter()
                    .map(|n| n.probe_index)
                    .collect();
                probes.retain(|p| !used.contains(&p.index));
                self.available_probes = probes;
                self.probe_fetch_error =
                    if self.available_probes.is_empty() && self.configured_nodes.is_empty() {
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
            self.editing_node_index = None;
            self.pending_node = Some(NodeConfig {
                probe_index: probe.index,
                probe_name: probe.name.clone(),
                probe_id: probe.probe_id.clone(),
                kp: self.defaults.kp.clone(),
                ki: self.defaults.ki.clone(),
                source_id: String::new(),
                sf: self.defaults.sf.clone(),
                bw: self.defaults.bw.clone(),
                tau: self.defaults.tau.clone(),
            });
        }
    }

    pub fn start_editing_node(&mut self, node_index: usize) {
        if let Some(node) = self.configured_nodes.get(node_index) {
            self.editing_node_index = Some(node_index);
            self.pending_node = Some(NodeConfig {
                probe_index: node.probe_index,
                probe_name: node.probe_name.clone(),
                probe_id: node.probe_id.clone(),
                kp: node.kp.clone(),
                ki: node.ki.clone(),
                source_id: node.source_id.clone(),
                sf: node.sf.clone(),
                bw: node.bw.clone(),
                tau: self.defaults.tau.clone(),
            });
        }
    }

    pub fn confirm_pending_node(&mut self) {
        if let Some(node) = self.pending_node.take() {
            self.defaults.kp = node.kp.clone();
            self.defaults.ki = node.ki.clone();
            self.defaults.sf = node.sf.clone();
            self.defaults.bw = node.bw.clone();
            self.defaults.tau = node.tau.clone();
            if let Some(i) = self.editing_node_index.take() {
                if i < self.configured_nodes.len() {
                    self.configured_nodes[i] = node;
                }
            } else {
                let probe_idx = node.probe_index;
                self.configured_nodes.push(node);
                self.available_probes.retain(|p| p.index != probe_idx);
            }
        }
    }

    pub fn remove_configured_node(&mut self, node_index: usize) {
        if node_index < self.configured_nodes.len() {
            let node = self.configured_nodes.remove(node_index);
            self.available_probes.push(ProbeInfo {
                index: node.probe_index,
                name: node.probe_name,
                probe_id: node.probe_id,
            });
            self.available_probes.sort_by_key(|p| p.index);
        }
    }

    pub fn type_char_pending(&mut self, c: char, focus: ProbeConfigFocus) {
        if let Some(node) = &mut self.pending_node {
            match focus {
                ProbeConfigFocus::Kp => node.kp.push(c),
                ProbeConfigFocus::Ki => node.ki.push(c),
                ProbeConfigFocus::SourceId => node.source_id.push(c),
                ProbeConfigFocus::Sf => node.sf.push(c),
                ProbeConfigFocus::Bw => node.bw.push(c),
                ProbeConfigFocus::Tau => node.tau.push(c),
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
                ProbeConfigFocus::Sf => {
                    node.sf.pop();
                }
                ProbeConfigFocus::Bw => {
                    node.bw.pop();
                }
                ProbeConfigFocus::Tau => {
                    node.tau.pop();
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

        // Gateway mirrors the first node's SF/BW so they communicate on the same channel.
        let first = self.configured_nodes.first();
        let gw_sf = first
            .map(|n| n.sf.as_str())
            .unwrap_or(&self.defaults.sf)
            .to_string();
        let gw_bw = first
            .map(|n| n.bw.as_str())
            .unwrap_or(&self.defaults.bw)
            .to_string();
        let gw_tau = first
            .map(|n| n.tau.as_str())
            .unwrap_or(&self.defaults.tau)
            .to_string();
        let mut gw_envs = color_envs();
        gw_envs.extend([
            ("SF".to_string(), gw_sf),
            ("BW".to_string(), gw_bw),
            ("TAU".to_string(), gw_tau),
        ]);
        log::info!("GW ENVS: {:?}", gw_envs);

        let mut descs = vec![ProcessDescriptor {
            source_id: "gw".to_string(),
            role: LogRole::Gateway,
            command: "just".to_string(),
            args: vec!["run-gw".to_string()],
            envs: gw_envs,
        }];

        for node in &self.configured_nodes {
            let mut envs = color_envs();
            envs.extend([
                ("KP".to_string(), node.kp.clone()),
                ("KI".to_string(), node.ki.clone()),
                ("SOURCEID".to_string(), node.source_id.clone()),
                ("SF".to_string(), node.sf.clone()),
                ("BW".to_string(), node.bw.clone()),
                ("TAU".to_string(), node.tau.clone()),
                ("PROBE".to_string(), node.probe_id.clone()),
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

        let mut found = std::collections::HashMap::new();
        let mut new_content = String::new();

        for line in content.lines() {
            let key = line.split('=').next().unwrap_or("");
            match key {
                "KP" => {
                    new_content.push_str(&format!("KP={}\n", self.defaults.kp));
                    found.insert("KP", true);
                }
                "KI" => {
                    new_content.push_str(&format!("KI={}\n", self.defaults.ki));
                    found.insert("KI", true);
                }
                "SF" => {
                    new_content.push_str(&format!("SF={}\n", self.defaults.sf));
                    found.insert("SF", true);
                }
                "BW" => {
                    new_content.push_str(&format!("BW={}\n", self.defaults.bw));
                    found.insert("BW", true);
                }
                "TAU" => {
                    new_content.push_str(&format!("TAU={}\n", self.defaults.tau));
                    found.insert("TAU", true);
                }
                _ => {
                    new_content.push_str(line);
                    new_content.push('\n');
                }
            }
        }
        for (key, val) in [
            ("KP", &self.defaults.kp),
            ("KI", &self.defaults.ki),
            ("SF", &self.defaults.sf),
            ("BW", &self.defaults.bw),
            ("TAU", &self.defaults.tau),
        ] {
            if !found.contains_key(key) {
                new_content.push_str(&format!("{key}={val}\n"));
            }
        }

        if let Err(e) = fs::write(path, new_content) {
            log::error!("Error saving env file: {:?}", e);
        }
    }

    /// Save collected data to CSV files and return the paths that were written.
    /// Returns `(main_stats_path, hw_stats_path)`; a value is `None` if the
    /// file could not be created.
    pub fn save_data(&self) -> (Option<String>, Option<String>) {
        let timestamp = Local::now().format("%d-%m:%H.%M").to_string();
        let first = self.configured_nodes.first();
        let sf = first.map(|n| n.sf.as_str()).unwrap_or(&self.defaults.sf);
        let bw = first.map(|n| n.bw.as_str()).unwrap_or(&self.defaults.bw);
        let n_nodes = self.configured_nodes.len();
        let kp = first.map(|n| n.kp.as_str()).unwrap_or(&self.defaults.kp);
        let ki = first.map(|n| n.ki.as_str()).unwrap_or(&self.defaults.ki);
        let tau = first.map(|n| n.tau.as_str()).unwrap_or(&self.defaults.tau);
        let meta = format!("SF{sf}_BW{bw}_KP{kp}_KI{ki}_TAU{tau}_{n_nodes}nodes");

        let prefix = "./analysis/data";
        let main_prefix = format!("{prefix}/main");
        let hw_prefix = format!("{prefix}/full_hw");
        let main_filename = format!("{main_prefix}/main_stats_{timestamp}_{meta}.csv");
        let hw_filename = format!("{hw_prefix}/hw_stats_{timestamp}_{meta}.csv");

        if let Err(e) = fs::create_dir_all(&main_prefix) {
            log::error!("Error in dir creation: {:?}", e);
            return (None, None);
        }
        if let Err(e) = fs::create_dir_all(&hw_prefix) {
            log::error!("Error in dir creation: {:?}", e);
            return (None, None);
        }

        let main_out = if let Ok(f) = File::create(&main_filename) {
            let mut wtr = csv::Writer::from_writer(f);
            for row in self.dash_stats.csv_rows() {
                let _ = wtr.serialize(row);
            }
            Some(main_filename)
        } else {
            None
        };

        let hw_out = if let Ok(mut f) = File::create(&hw_filename) {
            let _ = writeln!(f, "hardware_delay");
            for hw in &self.dash_stats.hardware_delay {
                let _ = writeln!(f, "{}", hw);
            }
            Some(hw_filename)
        } else {
            None
        };

        (main_out, hw_out)
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
                } else if tag.contains("[STATE_SYNC]") && parts.len() >= 2 {
                    self.dash_stats.on_state_sync(parts[1].trim());
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
            // This is for a J-link debugger, might not work for others
            let start_id = line.find("--")?;
            let end_id = line.find('(')?;
            let probe_id = line[start_id + 2..end_id - 1].trim().to_string();
            Some(ProbeInfo {
                index,
                name,
                probe_id,
            })
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

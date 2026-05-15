use chrono::Local;
use crossterm::event::KeyCode;
use dotenv::dotenv;
use std::path::Path;
use std::{env, fs::File, io::Write, str::FromStr};
use std::{fs, io};

use crate::composables::stats::DashStats;
use crate::navigator::LandingFocus;

const KI_DEFAULT: &str = "0.4";
const KP_DEFAULT: &str = "0.5";

pub enum AppEvent {
    Input(KeyCode),
    NodeLog { text: String, overwrite: bool },
    GwLog { text: String, overwrite: bool },
    HardwareLog { delay_ms: String },
    Tick,
}

pub struct EnvVars {
    pub kp: String,
    pub ki: String,
    pub source_id: String,
    pub alt_mdltn: String,
}

pub struct App {
    pub env_vars: EnvVars,
    pub node_logs: Vec<String>,
    pub gw_logs: Vec<String>,
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
        let kp = env::var("KP").unwrap_or_else(|_| KI_DEFAULT.to_string());
        let ki = env::var("KI").unwrap_or_else(|_| KP_DEFAULT.to_string());
        let source_id = env::var("SOURCEID").unwrap_or_else(|_| "7".to_string());
        let alt_mdltn = env::var("ALT_MDLTN").unwrap_or_else(|_| "false".to_string());

        Self {
            env_vars: EnvVars {
                kp,
                ki,
                source_id,
                alt_mdltn,
            },
            node_logs: Vec::new(),
            gw_logs: Vec::new(),
            dash_stats: DashStats::new(),
        }
    }

    pub fn reset_data(&mut self) {
        self.node_logs.clear();
        self.gw_logs.clear();
        self.dash_stats = DashStats::new();
    }

    pub fn type_char(&mut self, c: char, landing_focus: LandingFocus) {
        if let Some(s) = self.field_for_focus(landing_focus) {
            s.push(c);
        }
    }

    pub fn backspace(&mut self, landing_focus: LandingFocus) {
        if let Some(s) = self.field_for_focus(landing_focus) {
            s.pop();
        }
    }

    pub fn save_to_env_file(&self) {
        let path = Path::new(".env");

        let content = if path.exists() {
            match fs::read_to_string(path) {
                Ok(ss) => ss,
                Err(e) => {
                    log::error!("Error reading path {:?}: {:?}", path, e);
                    String::new()
                }
            }
        } else {
            String::new()
        };

        let mut kp_found = false;
        let mut ki_found = false;
        let mut source_id_found = false;

        let mut new_content = String::new();

        for line in content.lines() {
            if line.starts_with("KP=") {
                new_content.push_str(&format!("KP={}\n", self.env_vars.kp));
                kp_found = true;
            } else if line.starts_with("KI=") {
                new_content.push_str(&format!("KI={}\n", self.env_vars.ki));
                ki_found = true;
            } else if line.starts_with("SOURCEID=") {
                new_content.push_str(&format!("SOURCEID={}\n", self.env_vars.source_id));
                source_id_found = true;
            } else {
                // Keep the original line (this preserves comments and other env vars!)
                new_content.push_str(line);
                new_content.push('\n');
            }
        }

        if !kp_found {
            new_content.push_str(&format!("KP={}\n", self.env_vars.kp));
        }
        if !ki_found {
            new_content.push_str(&format!("KI={}\n", self.env_vars.ki));
        }
        if !source_id_found {
            new_content.push_str(&format!("SOURCEID={}\n", self.env_vars.source_id));
        }

        if let Err(e) = fs::write(path, new_content) {
            log::error!("Error saving env file: {:?}", e);
        }
    }

    fn field_for_focus(&mut self, focus: LandingFocus) -> Option<&mut String> {
        match focus {
            LandingFocus::Kp => Some(&mut self.env_vars.kp),
            LandingFocus::Ki => Some(&mut self.env_vars.ki),
            LandingFocus::SourceId => Some(&mut self.env_vars.source_id),
            LandingFocus::Start | LandingFocus::Save => None,
        }
    }

    /// Saves the data in stats to 2 files, main_stats for the history list of captured data, and
    /// full_hw for every delta captured between the trigger and signal on the discovery board
    pub fn save_data(&self) {
        let timestamp = Local::now().format("%d-%m:%H.%M").to_string();

        let prefix = "./analysis/data";
        let main_prefix = format!("{prefix}/main");
        let hw_prefix = format!("{prefix}/full_hw");
        let main_filename = format!("{main_prefix}/main_stats_{timestamp}.csv");
        let hw_filename = format!("{hw_prefix}/hw_stats_{timestamp}.csv");

        // Make sure the dirs exist
        if let Err(e) = fs::create_dir_all(main_prefix) {
            log::error!("Error in dir creation: {:?}", e);
            return;
        }
        if let Err(e) = fs::create_dir(hw_prefix) {
            log::error!("Error in dir creation: {:?}", e);
            return;
        }

        if let Ok(mut f) = File::create(&main_filename) {
            // TODO: Also save the Kp, Ki values?
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

    pub fn add_node_log(&mut self, log: String, overwrite: bool) {
        let clean = strip_log(&log);
        if clean.trim().is_empty() && overwrite {
            return;
        }

        if let Some(parts) = parse_piped(&clean) {
            let id = parts[0];
            log::debug!("node | id={:?} parts={:?}", id, &parts[1..]);

            if id.contains("[SYNC]") && parts.len() >= 5 {
                if let (Ok(delay), Ok(err), Ok(prev), Ok(new)) = (
                    extract::<f32>(parts[1]),
                    extract::<f32>(parts[2]),
                    extract::<f32>(parts[3]),
                    extract::<f32>(parts[4]),
                ) {
                    self.dash_stats.on_sync(delay, err, prev, new);
                }
            } else if id.contains("[DELTAS]") && parts.len() >= 3 {
                if let (Ok(up), Ok(down)) = (extract::<f32>(parts[1]), extract::<f32>(parts[2])) {
                    self.dash_stats.on_deltas(up, down);
                }
            } else if id.contains("[SIZE EXPECTED]") && parts.len() >= 2 {
                if let Ok(size) = extract::<usize>(parts[1]) {
                    log::info!("Got pre size {size}");
                    self.dash_stats.on_node_slice_size(size);
                }
            } else if id.contains("[TAU_SLICE]") && parts.len() >= 2 {
                if let Ok(ts) = extract::<u64>(parts[1]) {
                    self.dash_stats.on_node_slice_pre(ts);
                }
            } else if id.contains("[TAU_SLICE_POST]")
                && parts.len() >= 3
                && let (Ok(ts), Ok(size)) = (extract::<u64>(parts[1]), extract::<usize>(parts[2]))
            {
                log::info!("Got post size: {size}");
                self.dash_stats.on_node_slice_post(ts, size);
            }
        }

        push_or_overwrite(&mut self.node_logs, log, overwrite);
    }

    pub fn add_gw_log(&mut self, log: String, overwrite: bool) {
        let clean = strip_log(&log);
        if clean.trim().is_empty() && overwrite {
            return;
        }

        if let Some(parts) = parse_piped(&clean) {
            let id = parts[0];
            log::debug!("gw   | id={:?} parts={:?}", id, &parts[1..]);

            if id.contains("[TAU_SLICE]") && parts.len() >= 2 {
                if let Ok(ts) = extract::<u64>(parts[1]) {
                    self.dash_stats.on_gw_slice_pre(ts);
                }
            } else if id.contains("[TAU_SLICE_POST]")
                && parts.len() >= 3
                && let (Ok(ts), Ok(size)) = (extract::<u64>(parts[1]), extract::<usize>(parts[2]))
            {
                self.dash_stats.on_gw_slice_post(ts, size);
            } else if id.contains("[SIZE EXPECTED]")
                && parts.len() >= 2
                && let Ok(size) = extract::<usize>(parts[1])
            {
                log::info!("Got pre size {size}");
                self.dash_stats.on_gw_slice_size(size);
            }
        }

        push_or_overwrite(&mut self.gw_logs, log, overwrite);
    }
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

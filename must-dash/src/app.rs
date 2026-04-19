use crossterm::event::KeyCode;
use regex::Regex;
use std::{collections::VecDeque, env, sync::OnceLock};

pub enum AppEvent {
    Input(KeyCode),
    NodeLog { text: String, overwrite: bool },
    GwLog { text: String, overwrite: bool },
    Tick,
}

#[derive(PartialEq)]
pub enum AppView {
    Landing,
    Dashboard,
}

#[derive(PartialEq, Clone, Copy)]
pub enum LandingFocus {
    Kp,
    Ki,
    SourceId,
    Start,
}

#[derive(PartialEq)]
pub enum DashFocus {
    Data,
    Logs,
}

/// Helper struct to keep a rolling history of floats and compute medians
pub struct RollingStat {
    pub values: VecDeque<f32>,
    capacity: usize,
}

impl RollingStat {
    fn new(capacity: usize) -> Self {
        Self {
            values: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    fn push(&mut self, val: f32) {
        if self.values.len() == self.capacity {
            self.values.pop_front();
        }
        self.values.push_back(val);
    }

    pub fn median(&self) -> Option<f32> {
        if self.values.is_empty() {
            return None;
        }
        let mut sorted: Vec<_> = self.values.iter().copied().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = sorted.len() / 2;
        if sorted.len() % 2 == 0 {
            Some((sorted[mid - 1] + sorted[mid]) / 2.0)
        } else {
            Some(sorted[mid])
        }
    }
}

impl Default for RollingStat {
    fn default() -> Self {
        Self::new(10)
    }
}

pub struct EnvVars {
    pub kp: String,
    pub ki: String,
    pub source_id: String,
}

pub struct DashStats {
    pub delay: RollingStat,
    pub err: RollingStat,
    pub prev_speed: RollingStat,
    pub new_speed: RollingStat,
    pub delta_up: RollingStat,
    pub delta_down: RollingStat,
}

impl DashStats {
    pub fn new() -> Self {
        Self {
            delay: RollingStat::default(),
            err: RollingStat::default(),
            prev_speed: RollingStat::default(),
            new_speed: RollingStat::default(),
            delta_up: RollingStat::default(),
            delta_down: RollingStat::default(),
        }
    }
}

impl Default for DashStats {
    fn default() -> Self {
        Self::new()
    }
}

pub struct App {
    pub view: AppView,
    pub landing_focus: LandingFocus,
    pub dash_focus: DashFocus,

    pub env_vars: EnvVars,

    pub node_logs: VecDeque<String>,
    pub gw_logs: VecDeque<String>,

    pub dash_stats: DashStats,

    pub shutting_down: bool,
}

static DRIFT_REGEX: OnceLock<Regex> = OnceLock::new();
fn get_regex() -> &'static Regex {
    DRIFT_REGEX.get_or_init(|| {
        Regex::new(r"Measured delay:\s*([-\d.]+)\s*ms\s*\|\s*err:\s*([-\d.]+)\s*ms\s*\|\s*prev speed:\s*([-\d.]+)\s*\|\s*new speed:\s*([-\d.]+)").unwrap()
     // Regex::new(    r"Measured delay:\s*([-\d.]+)\s*ms\s*\|\s*err:\s*([-\d.]+)\s*ms\s*\|\s*prev speed:\s*([-\d]+)\s*\|\s*new speed:\s*([-\d]+)").unwrap()

    })
}

static DELTA_REGEX: OnceLock<Regex> = OnceLock::new();
fn get_delta_regex() -> &'static Regex {
    DELTA_REGEX.get_or_init(|| {
        // Matches: "Delta up: 5.287 | Delta down: 8.979"
        Regex::new(r"Delta up:\s*([-\d.]+)\s*\|\s*Delta down:\s*([-\d.]+)").unwrap()
    })
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        let kp = env::var("KP").unwrap_or("20".to_string());
        let ki = env::var("KI").unwrap_or("25".to_string());
        let source_id = env::var("SOURCEID").unwrap_or("7".to_string());

        Self {
            landing_focus: LandingFocus::Kp,
            dash_focus: DashFocus::Logs,
            view: AppView::Landing,
            env_vars: EnvVars { kp, ki, source_id },

            node_logs: VecDeque::with_capacity(500),
            gw_logs: VecDeque::with_capacity(500),

            dash_stats: DashStats::new(),
            shutting_down: false,
        }
    }

    pub fn reset_data(&mut self) {
        self.node_logs.clear();
        self.gw_logs.clear();
        self.dash_stats = DashStats::new();
    }

    pub fn next_landing_focus(&mut self) {
        self.landing_focus = match self.landing_focus {
            LandingFocus::Kp => LandingFocus::Ki,
            LandingFocus::Ki => LandingFocus::SourceId,
            LandingFocus::SourceId => LandingFocus::Start,
            LandingFocus::Start => LandingFocus::Kp,
        }
    }

    pub fn prev_landing_focus(&mut self) {
        self.landing_focus = match self.landing_focus {
            LandingFocus::Kp => LandingFocus::Start,
            LandingFocus::Ki => LandingFocus::Kp,
            LandingFocus::SourceId => LandingFocus::Ki,
            LandingFocus::Start => LandingFocus::SourceId,
        };
    }

    pub fn type_char(&mut self, c: char) {
        let s = match self.landing_focus {
            LandingFocus::Kp => &mut self.env_vars.kp,
            LandingFocus::Ki => &mut self.env_vars.ki,
            LandingFocus::SourceId => &mut self.env_vars.source_id,
            LandingFocus::Start => return,
        };
        s.push(c);
    }

    pub fn backspace(&mut self) {
        let s = match self.landing_focus {
            LandingFocus::Kp => &mut self.env_vars.kp,
            LandingFocus::Ki => &mut self.env_vars.ki,
            LandingFocus::SourceId => &mut self.env_vars.source_id,
            LandingFocus::Start => return,
        };
        s.pop();
    }

    pub fn toggle_dash_focus(&mut self) {
        self.dash_focus = match self.dash_focus {
            DashFocus::Data => DashFocus::Logs,
            DashFocus::Logs => DashFocus::Data,
        };
    }

    pub fn add_node_log(&mut self, log: String, overwrite: bool) {
        // 1. Optimize: Only run expensive regex if the log contains the keyword
        // if log.contains("Measured drift:") {
        //     let stripped_bytes = strip_ansi_escapes::strip(log.as_bytes());
        //     if let Ok(clean_log) = String::from_utf8(stripped_bytes)
        //         && let Some(caps) = get_regex().captures(&clean_log)
        //     {
        //         if let Ok(d) = caps[1].parse::<f32>() {
        //             self.dash_stats.delay.push(d);
        //         }
        //         if let Ok(e) = caps[2].parse::<f32>() {
        //             self.dash_stats.err.push(e);
        //         }
        //         if let Ok(r) = caps[3].parse::<f32>() {
        //             self.dash_stats.prev_speed.push(r);
        //         }
        //         if let Ok(sr) = caps[4].parse::<f32>() {
        //             self.dash_stats.new_speed.push(sr);
        //         }
        //     }
        // } else if log.contains("Delta up:") {
        //     let stripped_bytes = strip_ansi_escapes::strip(log.as_bytes());
        //     if let Ok(clean_log) = String::from_utf8(stripped_bytes)
        //         && let Some(caps) = get_delta_regex().captures(&clean_log)
        //     {
        //         if let Ok(up) = caps[1].parse::<f32>() {
        //             self.dash_stats.delta_up.push(up);
        //         }
        //         if let Ok(down) = caps[2].parse::<f32>() {
        //             self.dash_stats.delta_down.push(down);
        //         }
        //     }
        // } else
        if log.contains("[SYNC]") && log.contains("|") {
            let stripped_bytes = strip_ansi_escapes::strip(log.as_bytes());
            if let Ok(clean_log) = String::from_utf8(stripped_bytes) {
                // 1. Get everything after "[SYNC]   "
                if let Some(data_str) = clean_log.split("[SYNC]").nth(1) {
                    // 2. Split into [" delay: 1.115ms ", " err: -1.115ms ", " v_prev: 17794275 ", " v_new: 17752705"]
                    let parts: Vec<&str> = data_str.split('|').collect();

                    if parts.len() >= 4 {
                        // 3. Extract values using quick string manipulation
                        if let Some(delay_val) = extract_value(parts[0]) {
                            self.dash_stats.delay.push(delay_val);
                        }
                        if let Some(err_val) = extract_value(parts[1]) {
                            self.dash_stats.err.push(err_val);
                        }
                        if let Some(prev_val) = extract_value(parts[2]) {
                            self.dash_stats.prev_speed.push(prev_val);
                        }
                        if let Some(new_val) = extract_value(parts[3]) {
                            self.dash_stats.new_speed.push(new_val);
                        }
                    }
                }
            }
        } else if log.contains("[DELTAS]") && log.contains("|") {
            let stripped_bytes = strip_ansi_escapes::strip(log.as_bytes());
            if let Ok(clean_log) = String::from_utf8(stripped_bytes) {
                //  Get everything after "[DELTAS]   "
                if let Some(data_str) = clean_log.split("[DELTAS]").nth(1) {
                    // split into ["up: {}ms", "down: {}ms", ".."]
                    let parts: Vec<&str> = data_str.split('|').collect();

                    if parts.len() >= 2 {
                        if let Some(delay_val) = extract_value(parts[0]) {
                            self.dash_stats.delta_up.push(delay_val);
                        }
                        if let Some(err_val) = extract_value(parts[1]) {
                            self.dash_stats.delta_down.push(err_val);
                        }
                    }
                }
            }
        }

        if overwrite {
            if let Some(last) = self.node_logs.back_mut() {
                *last = log;
            } else {
                self.node_logs.push_back(log);
            }
        } else {
            self.node_logs.push_back(log);
        }

        if self.node_logs.len() > 500 {
            self.node_logs.pop_front();
        }
    }

    pub fn add_gw_log(&mut self, log: String, overwrite: bool) {
        if overwrite {
            if let Some(last) = self.gw_logs.back_mut() {
                *last = log;
            } else {
                self.gw_logs.push_back(log);
            }
        } else {
            self.gw_logs.push_back(log);
        }

        if self.gw_logs.len() > 500 {
            self.gw_logs.pop_front();
        }
    }
}

fn extract_value(part: &str) -> Option<f32> {
    part.split(':') // Split into " delay" and " 1.115ms "
        .nth(1)? // Get the value side
        .trim() // Remove surrounding spaces
        .trim_end_matches("ms") // Remove 'ms' if it exists
        .trim() // Clean up any remaining space
        .parse::<f32>() // Convert to float
        .ok()
}

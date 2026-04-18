use crossterm::event::KeyCode;
use regex::Regex;
use std::{collections::VecDeque, sync::OnceLock};

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
        Regex::new(r"Measured delay:\s*([-\d.]+)\s*ms\s*\|\s*err:\s*([-\d.]+)\s*ms\s*\|\s*prev speed:\s*([-\d]+)\s*\|\s*new speed:\s*([-\d]+)").unwrap()
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
        Self {
            node_logs: VecDeque::with_capacity(500),
            gw_logs: VecDeque::with_capacity(500),
            focus: Focus::Logs, // Default focus
            delay: RollingStat::new(30),
            err: RollingStat::new(30),
            prev_speed: RollingStat::new(30),
            new_speed: RollingStat::new(30),
            delta_up: RollingStat::new(30),
            delta_down: RollingStat::new(30),
            shutting_down: false,
        }
    }

    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Data => Focus::Logs,
            Focus::Logs => Focus::Data,
        };
    }

    pub fn add_node_log(&mut self, log: String, overwrite: bool) {
        // 1. Optimize: Only run expensive regex if the log contains the keyword
        if log.contains("Measured drift:") {
            let stripped_bytes = strip_ansi_escapes::strip(log.as_bytes());
            if let Ok(clean_log) = String::from_utf8(stripped_bytes)
                && let Some(caps) = get_regex().captures(&clean_log)
            {
                if let Ok(d) = caps[1].parse::<f32>() {
                    self.delay.push(d);
                }
                if let Ok(e) = caps[2].parse::<f32>() {
                    self.err.push(e);
                }
                if let Ok(r) = caps[3].parse::<f32>() {
                    self.prev_speed.push(r);
                }
                if let Ok(sr) = caps[4].parse::<f32>() {
                    self.new_speed.push(sr);
                }
            }
        } else if log.contains("Delta up:") {
            let stripped_bytes = strip_ansi_escapes::strip(log.as_bytes());
            if let Ok(clean_log) = String::from_utf8(stripped_bytes)
                && let Some(caps) = get_delta_regex().captures(&clean_log)
            {
                if let Ok(up) = caps[1].parse::<f32>() {
                    self.delta_up.push(up);
                }
                if let Ok(down) = caps[2].parse::<f32>() {
                    self.delta_down.push(down);
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

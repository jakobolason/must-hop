use crossterm::event::KeyCode;
use std::{
    env,
    fs::File,
    io::Write,
    time::{SystemTime, UNIX_EPOCH},
};

pub enum AppEvent {
    Input(KeyCode),
    NodeLog { text: String, overwrite: bool },
    GwLog { text: String, overwrite: bool },
    HardwareLog { delay_ms: String },
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
    Save,
}

#[derive(PartialEq)]
pub enum DashFocus {
    Data,
    Logs,
}

pub struct EnvVars {
    pub kp: String,
    pub ki: String,
    pub source_id: String,
}

pub struct ChartData {
    pub delay: Vec<(f64, f64)>,
    pub up: Vec<(f64, f64)>,
    pub down: Vec<(f64, f64)>,
    pub hw: Vec<(f64, f64)>,
    pub x_bounds: [f64; 2],
    pub y_bounds: [f64; 2],
}

pub struct DashStats {
    pub delay: Vec<f32>,
    pub err: Vec<f32>,
    pub prev_speed: Vec<f32>,
    pub new_speed: Vec<f32>,
    pub delta_up: Vec<f32>,
    pub delta_down: Vec<f32>,
    pub hardware_delay: Vec<f32>,
    pub mean_hardware_delay: Vec<f32>,
    pub last_hw_idx: usize,
}

impl DashStats {
    pub fn new() -> Self {
        Self {
            delay: Vec::new(),
            err: Vec::new(),
            prev_speed: Vec::new(),
            new_speed: Vec::new(),
            delta_up: Vec::new(),
            delta_down: Vec::new(),
            hardware_delay: Vec::new(),
            mean_hardware_delay: Vec::new(),
            last_hw_idx: 0,
        }
    }
    pub fn get_history_lines(&self, max_lines: usize) -> Vec<Vec<String>> {
        let mut rows = Vec::new();
        let len = self.delay.len();
        // let hw_len = self.hardware_delay.len();
        // let synchronized_hw_len = hw_len - (hw_len % 10);

        let start = len.saturating_sub(max_lines);

        let to_str = |rs: &Vec<f32>, i: usize| {
            rs.get(i)
                .map_or("--".to_string(), |&v| format!("{:.3}ms", v))
        };

        for i in start..len {
            let hb_nr = format!("{:02}", i);
            let delay_str = to_str(&self.delay, i);
            let speed = to_str(&self.new_speed, i);
            let up_str = to_str(&self.delta_up, i);
            let down_str = to_str(&self.delta_down, i);
            let hw_delay = to_str(&self.mean_hardware_delay, i);

            rows.push(vec![hb_nr, delay_str, speed, up_str, down_str, hw_delay]);
        }
        rows
    }

    /// Processes min/max bounds and slices data specifically for the Chart size
    pub fn get_chart_data(&self, max_x_points: usize) -> ChartData {
        // Prevent 0 width division issues
        let num_points = max_x_points.max(1);
        // 10 hw per 1 heartbeat value
        let hw_ratio = 10.0;

        let extract = |deque: &Vec<f32>| -> Vec<(f64, f64)> {
            let start = deque.len().saturating_sub(num_points);
            deque
                .iter()
                .skip(start)
                .enumerate()
                .map(|(i, &v)| (i as f64, v as f64))
                .collect()
        };

        let delay = extract(&self.delay);
        let up = extract(&self.delta_up);
        let down = extract(&self.delta_down);

        let max_hw_points = (num_points as f64 * hw_ratio) as usize;
        let hw_len = self.hardware_delay.len();
        let hw_start = hw_len.saturating_sub(max_hw_points);
        let hw: Vec<(f64, f64)> = self
            .hardware_delay
            .iter()
            .skip(hw_start)
            .enumerate()
            .map(|(i, &v)| {
                let x = i as f64 / hw_ratio; // 0, 0.1, 0.2, 0.3, ...
                (x, v as f64)
            })
            .collect();

        // Calculate Y-axis bounds
        let mut min_val = f64::INFINITY;
        let mut max_val = f64::NEG_INFINITY;
        for &(_, y) in delay
            .iter()
            .chain(up.iter())
            .chain(down.iter())
            .chain(hw.iter())
        {
            if y < min_val {
                min_val = y;
            }
            if y > max_val {
                max_val = y;
            }
        }

        let y_bounds = if min_val.is_infinite() || max_val.is_infinite() {
            [0.0, 10.0]
        } else if (max_val - min_val).abs() < f64::EPSILON {
            [min_val - 1.0, max_val + 1.0]
        } else {
            let padding = (max_val - min_val) * 0.1;
            [min_val - padding, max_val + padding]
        };

        ChartData {
            delay,
            up,
            down,
            hw,
            x_bounds: [0.0, (num_points.saturating_sub(1)) as f64],
            y_bounds,
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

    pub node_logs: Vec<String>,
    pub gw_logs: Vec<String>,

    pub dash_stats: DashStats,

    pub shutting_down: bool,
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

            node_logs: Vec::new(),
            gw_logs: Vec::new(),

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
            LandingFocus::Start => LandingFocus::Save,
            LandingFocus::Save => LandingFocus::Kp,
        }
    }

    pub fn prev_landing_focus(&mut self) {
        self.landing_focus = match self.landing_focus {
            LandingFocus::Kp => LandingFocus::Save,
            LandingFocus::Ki => LandingFocus::Kp,
            LandingFocus::SourceId => LandingFocus::Ki,
            LandingFocus::Start => LandingFocus::SourceId,
            LandingFocus::Save => LandingFocus::Start,
        };
    }

    pub fn type_char(&mut self, c: char) {
        let s = match self.landing_focus {
            LandingFocus::Kp => &mut self.env_vars.kp,
            LandingFocus::Ki => &mut self.env_vars.ki,
            LandingFocus::SourceId => &mut self.env_vars.source_id,
            LandingFocus::Start => return,
            LandingFocus::Save => return,
        };
        s.push(c);
    }

    pub fn backspace(&mut self) {
        let s = match self.landing_focus {
            LandingFocus::Kp => &mut self.env_vars.kp,
            LandingFocus::Ki => &mut self.env_vars.ki,
            LandingFocus::SourceId => &mut self.env_vars.source_id,
            LandingFocus::Start => return,
            LandingFocus::Save => return,
        };
        s.pop();
    }

    pub fn toggle_dash_focus(&mut self) {
        self.dash_focus = match self.dash_focus {
            DashFocus::Data => DashFocus::Logs,
            DashFocus::Logs => DashFocus::Data,
        };
    }

    pub fn save_data(&self) {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let prefix = "./analysis/data/".to_string();
        let main_filename = format!("{prefix}main_stats_{:02}.csv", timestamp);
        let hw_filename = format!("{prefix}hw_stats_{:02}.csv", timestamp);

        // Save the averages and such
        if let Ok(mut main_file) = File::create(&main_filename) {
            // Write the CSV header
            let _ = writeln!(
                main_file,
                "delay,err,prev_speed,new_speed,delta_up,delta_down,mean_hardware_delay"
            );

            let max_len = [
                self.dash_stats.delay.len(),
                self.dash_stats.err.len(),
                self.dash_stats.prev_speed.len(),
                self.dash_stats.new_speed.len(),
                self.dash_stats.delta_up.len(),
                self.dash_stats.delta_down.len(),
                self.dash_stats.mean_hardware_delay.len(),
            ]
            .into_iter()
            .max()
            .unwrap_or(0);

            let get_val = |vec: &Vec<f32>, i: usize| -> String {
                vec.get(i).map_or_else(String::new, |v| v.to_string())
            };

            for i in 0..max_len {
                let _ = writeln!(
                    main_file,
                    "{},{},{},{},{},{},{}",
                    get_val(&self.dash_stats.delay, i),
                    get_val(&self.dash_stats.err, i),
                    get_val(&self.dash_stats.prev_speed, i),
                    get_val(&self.dash_stats.new_speed, i),
                    get_val(&self.dash_stats.delta_up, i),
                    get_val(&self.dash_stats.delta_down, i),
                    get_val(&self.dash_stats.mean_hardware_delay, i),
                );
            }
        }

        // There are about 10 times as many hw timestamps, so save in seperate file
        if let Ok(mut hw_file) = File::create(&hw_filename) {
            // Write header
            let _ = writeln!(hw_file, "hardware_delay");

            // Write all individual hardware delay points
            for hw in &self.dash_stats.hardware_delay {
                let _ = writeln!(hw_file, "{}", hw);
            }
        }
    }
    pub fn add_hw_delay(&mut self, log_str: String) {
        if let Some(delay_ms) = extract_value(&log_str) {
            self.dash_stats.hardware_delay.push(delay_ms);
        }
    }

    pub fn add_node_log(&mut self, log: String, overwrite: bool) {
        let stripped = strip_ansi_escapes::strip(log.as_bytes());
        let clean = String::from_utf8_lossy(&stripped);
        let visible = clean.trim();

        // Drop pure cursor-movement lines with no visible content
        if visible.is_empty() && overwrite {
            return;
        }

        if log.contains("[SYNC]") && log.contains("|") {
            let stripped_bytes = strip_ansi_escapes::strip(log.as_bytes());
            if let Ok(clean_log) = String::from_utf8(stripped_bytes) {
                // 1. Get everything after "[SYNC]   "
                if let Some(data_str) = clean_log.split("[SYNC]").nth(1) {
                    // 2. Split into [" delay: 1.115ms ", " err: -1.115ms ", " v_prev: 17794275 ", " v_new: 17752705"]
                    let parts: Vec<&str> = data_str.split('|').collect();

                    if parts.len() >= 4 {
                        // 3. Extract using quick string manipulation
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
                // at sync we calc mean of hw delays
                let measured_delays: Vec<f32> = self
                    .dash_stats
                    .hardware_delay
                    .iter()
                    .skip(self.dash_stats.last_hw_idx)
                    .copied()
                    .collect();
                let sum: f32 = measured_delays.iter().sum();
                let mean = sum / measured_delays.len() as f32;
                self.dash_stats.mean_hardware_delay.push(mean);
                self.dash_stats.last_hw_idx = self.dash_stats.hardware_delay.len() - 1;
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
            if let Some(last) = self.node_logs.last_mut() {
                *last = log;
            } else {
                self.node_logs.push(log);
            }
        } else {
            self.node_logs.push(log);
        }

        // if self.node_logs.len() > 500 {
        //     self.node_logs.pop_front();
        // }
    }

    pub fn add_gw_log(&mut self, log: String, overwrite: bool) {
        // if overwrite {
        //     if let Some(last) = self.gw_logs.back_mut() {
        //         *last = log;
        //     } else {
        //         self.gw_logs.push_back(log);
        //     }
        // } else {
        self.gw_logs.push(log);
        // }

        // if self.gw_logs.len() > 500 {
        //     self.gw_logs.pop_front();
        // }
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

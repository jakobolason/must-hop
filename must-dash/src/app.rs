use crossterm::event::KeyCode;
use std::{
    env,
    fs::File,
    io::Write,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::navigator::LandingFocus;

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
}

pub struct ChartData {
    /// Clock error series (was `delay` — now correctly uses err_ms)
    pub err: Vec<(f64, f64)>,
    pub up: Vec<(f64, f64)>,
    pub down: Vec<(f64, f64)>,
    pub hw: Vec<(f64, f64)>,
    pub x_bounds: [f64; 2],
    pub y_bounds: [f64; 2],
}

/// One entry is created each time a `[SYNC]` log line is received.
/// Fields that arrive from earlier log lines (e.g. `[DELTAS]`) are
/// accumulated in `PendingPacket` and moved in when the entry is finalised.
pub struct PacketEntry {
    /// my_stamp - gps_time_us  (the raw measured offset, ms)
    pub delay_ms: f32,
    /// Clock error after drift correction (ms)
    pub err_ms: f32,
    pub prev_speed: f32,
    pub new_speed: f32,
    /// None when `[DELTAS]` was absent or REJECTED this cycle
    pub delta_up_ms: Option<f32>,
    pub delta_down_ms: Option<f32>,
    /// Mean of all hardware-delay samples collected since the previous SYNC
    pub mean_hw_delay_ms: f32,
}

/// Accumulated state for the packet that is currently being built.
/// Reset (via `mem::take`) each time `finalize_packet` is called.
#[derive(Default)]
struct PendingPacket {
    delta_up_ms: Option<f32>,
    delta_down_ms: Option<f32>,
}
/// Time and byte-size diff between TAU_SLICE and TAU_SLICE_POST.
/// Displayed by index alongside PacketEntry; may lag or lead.
pub struct SliceDiff {
    /// Elapsed micros between pre- and post-slice timestamp
    pub time_us: u64,
    /// Byte-size difference
    pub bytes: usize,
}

pub struct DashStats {
    /// One entry per completed SYNC cycle
    pub packets: Vec<PacketEntry>,
    /// Raw hardware-delay samples (~10× the packet rate)
    pub hardware_delay: Vec<f32>,
    /// One diff per GW TAU_SLICE_POST
    pub gw_diff: Vec<SliceDiff>,
    /// One diff per node TAU_SLICE_POST
    pub node_diff: Vec<SliceDiff>,

    // --- private scratchpad, not for display ---
    pending: PendingPacket,
    last_hw_idx: usize,
    last_gw_slice_us: u64,
    last_gw_slice_size: usize,
    last_node_slice_us: u64,
    last_node_slice_size: usize,
}

impl Default for DashStats {
    fn default() -> Self {
        Self::new()
    }
}

impl DashStats {
    pub fn new() -> Self {
        Self {
            packets: Vec::new(),
            hardware_delay: Vec::new(),
            gw_diff: Vec::new(),
            node_diff: Vec::new(),
            pending: PendingPacket::default(),
            last_hw_idx: 0,
            last_gw_slice_us: 0,
            last_gw_slice_size: 0,
            last_node_slice_us: 0,
            last_node_slice_size: 0,
        }
    }

    /// Median of the last `n` packets for a given field.
    pub fn median_n(&self, n: usize, f: impl Fn(&PacketEntry) -> f32) -> Option<f32> {
        let start = self.packets.len().saturating_sub(n);
        let mut vals: Vec<f32> = self.packets[start..].iter().map(f).collect();
        if vals.is_empty() {
            return None;
        }
        vals.sort_by(f32::total_cmp);
        let mid = vals.len() / 2;
        Some(if vals.len().is_multiple_of(2) {
            (vals[mid - 1] + vals[mid]) / 2.0
        } else {
            vals[mid]
        })
    }

    /// Returns the most recent `max_lines` packets formatted for the history table.
    ///
    /// Columns: Packet | Error | Speed | Δ Up | Δ Down | HW Delay | GW µs | GW B | Node µs | Node B
    pub fn get_history_lines(&self, max_lines: usize) -> Vec<Vec<String>> {
        let len = self.packets.len();
        let start = len.saturating_sub(max_lines);

        let fmt_ms = |v: f32| format!("{:.3}ms", v);
        let fmt_opt_ms = |v: Option<f32>| v.map_or("--".to_string(), fmt_ms);

        self.packets[start..]
            .iter()
            .enumerate()
            .map(|(offset, p)| {
                let i = start + offset;
                let gw_us = self.gw_diff.get(i).map_or("--".to_string(), |d| {
                    format!("{:.3}ms", d.time_us as f32 / 1_000.0)
                });
                let gw_b = self
                    .gw_diff
                    .get(i)
                    .map_or("--".to_string(), |d| format!("{}B", d.bytes));
                let node_us = self.node_diff.get(i).map_or("--".to_string(), |d| {
                    format!("{:.3}ms", d.time_us as f32 / 1_000.0)
                });
                let node_b = self
                    .node_diff
                    .get(i)
                    .map_or("--".to_string(), |d| format!("{}B", d.bytes));

                vec![
                    format!("{:02}", i),
                    fmt_ms(p.err_ms),
                    format!("{}", p.new_speed as i64),
                    fmt_opt_ms(p.delta_up_ms),
                    fmt_opt_ms(p.delta_down_ms),
                    fmt_ms(p.mean_hw_delay_ms),
                    gw_us,
                    gw_b,
                    node_us,
                    node_b,
                ]
            })
            .collect()
    }

    /// Prepares data for the chart.  Uses `err_ms` (clock error) as the primary series.
    pub fn get_chart_data(&self, max_x_points: usize) -> ChartData {
        let num_points = max_x_points.max(1);
        let hw_ratio = 10.0_f64;

        let start = self.packets.len().saturating_sub(num_points);
        let slice = &self.packets[start..];

        let err: Vec<(f64, f64)> = slice
            .iter()
            .enumerate()
            .map(|(i, p)| (i as f64, p.err_ms as f64))
            .collect();

        let up: Vec<(f64, f64)> = slice
            .iter()
            .enumerate()
            .map(|(i, p)| (i as f64, p.delta_up_ms.unwrap_or(0.0) as f64))
            .collect();

        let down: Vec<(f64, f64)> = slice
            .iter()
            .enumerate()
            .map(|(i, p)| (i as f64, p.delta_down_ms.unwrap_or(0.0) as f64))
            .collect();

        let max_hw_points = (num_points as f64 * hw_ratio) as usize;
        let hw_start = self.hardware_delay.len().saturating_sub(max_hw_points);
        let hw: Vec<(f64, f64)> = self.hardware_delay[hw_start..]
            .iter()
            .enumerate()
            .map(|(i, &v)| (i as f64 / hw_ratio, v as f64))
            .collect();

        // Y-axis bounds across all series
        let (min_val, max_val) = err
            .iter()
            .chain(up.iter())
            .chain(down.iter())
            .chain(hw.iter())
            .map(|&(_, y)| y)
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(mn, mx), y| {
                (mn.min(y), mx.max(y))
            });

        let y_bounds = if min_val.is_infinite() {
            [0.0, 10.0]
        } else if (max_val - min_val).abs() < f64::EPSILON {
            [min_val - 1.0, max_val + 1.0]
        } else {
            let padding = (max_val - min_val) * 0.1;
            [min_val - padding, max_val + padding]
        };

        ChartData {
            err,
            up,
            down,
            hw,
            x_bounds: [0.0, num_points.saturating_sub(1) as f64],
            y_bounds,
        }
    }

    fn on_deltas(&mut self, up_ms: f32, down_ms: f32) {
        self.pending.delta_up_ms = Some(up_ms);
        self.pending.delta_down_ms = Some(down_ms);
    }

    fn on_sync(&mut self, delay_ms: f32, err_ms: f32, prev_speed: f32, new_speed: f32) {
        // Consume all hardware-delay samples collected since last sync
        let hw_slice = &self.hardware_delay[self.last_hw_idx..];
        let mean_hw = if hw_slice.is_empty() {
            0.0
        } else {
            hw_slice.iter().sum::<f32>() / hw_slice.len() as f32
        };
        self.last_hw_idx = self.hardware_delay.len();

        let pending = std::mem::take(&mut self.pending);
        self.packets.push(PacketEntry {
            delay_ms,
            err_ms,
            prev_speed,
            new_speed,
            delta_up_ms: pending.delta_up_ms,
            delta_down_ms: pending.delta_down_ms,
            mean_hw_delay_ms: mean_hw,
        });
    }

    fn on_node_slice_pre(&mut self, ts_us: u64) {
        self.last_node_slice_us = ts_us;
    }

    fn on_node_slice_size(&mut self, size: usize) {
        self.last_node_slice_size = size;
    }

    fn on_node_slice_post(&mut self, ts_us: u64, size: usize) {
        self.node_diff.push(SliceDiff {
            time_us: ts_us.saturating_sub(self.last_node_slice_us),
            bytes: size.saturating_sub(self.last_node_slice_size),
        });
    }

    fn on_gw_slice_pre(&mut self, ts_us: u64) {
        self.last_gw_slice_us = ts_us;
    }

    fn on_gw_slice_post(&mut self, ts_us: u64, size: usize) {
        self.gw_diff.push(SliceDiff {
            time_us: ts_us.saturating_sub(self.last_gw_slice_us),
            bytes: size.saturating_sub(self.last_gw_slice_size),
        });
    }

    fn on_gw_slice_size(&mut self, size: usize) {
        self.last_gw_slice_size = size;
    }
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
        let kp = env::var("KP").unwrap_or_else(|_| "20".to_string());
        let ki = env::var("KI").unwrap_or_else(|_| "25".to_string());
        let source_id = env::var("SOURCEID").unwrap_or_else(|_| "7".to_string());

        Self {
            env_vars: EnvVars { kp, ki, source_id },
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

    fn field_for_focus(&mut self, focus: LandingFocus) -> Option<&mut String> {
        match focus {
            LandingFocus::Kp => Some(&mut self.env_vars.kp),
            LandingFocus::Ki => Some(&mut self.env_vars.ki),
            LandingFocus::SourceId => Some(&mut self.env_vars.source_id),
            LandingFocus::Start | LandingFocus::Save => None,
        }
    }

    pub fn save_data(&self) {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let prefix = "./analysis/data/";
        let main_filename = format!("{prefix}main_stats_{timestamp:02}.csv");
        let hw_filename = format!("{prefix}hw_stats_{timestamp:02}.csv");

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

    pub fn add_node_log(&mut self, log: String, overwrite: bool) {
        let clean = strip_log(&log);
        if clean.trim().is_empty() && overwrite {
            return;
        }

        if let Some(parts) = parse_piped(&clean) {
            let id = parts[0];
            log::debug!("node | id={:?} parts={:?}", id, &parts[1..]);

            if id.contains("[SYNC]") && parts.len() >= 5 {
                // Format: [SYNC]|measured_delay_ms|err_ms|prev_speed|new_speed|
                if let (Ok(delay), Ok(err), Ok(prev), Ok(new)) = (
                    extract::<f32>(parts[1]),
                    extract::<f32>(parts[2]),
                    extract::<f32>(parts[3]),
                    extract::<f32>(parts[4]),
                ) {
                    self.dash_stats.on_sync(delay, err, prev, new);
                }
            } else if id.contains("[DELTAS]") && parts.len() >= 3 {
                // Format: [DELTAS]|up_ms|down_ms|...
                // Present for both ACCEPTED and REJECTED; we store either way.
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

        // Bug fix: was incorrectly writing to self.node_logs on overwrite
        push_or_overwrite(&mut self.gw_logs, log, overwrite);
    }
}
fn strip_log(log: &str) -> String {
    let stripped = strip_ansi_escapes::strip(log.as_bytes());
    String::from_utf8_lossy(&stripped).into_owned()
}

/// Returns `Some(parts)` if `clean` contains at least one `|`, else `None`.
/// `parts[0]` is the identifier, subsequent entries are the fields.
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


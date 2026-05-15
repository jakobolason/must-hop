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
                    fmt_ms(p.mean_hw_delay_ms),
                    fmt_ms(p.err_ms),
                    fmt_ms(p.delay_ms),
                    fmt_opt_ms(p.delta_up_ms),
                    fmt_opt_ms(p.delta_down_ms),
                    format!("{}", p.new_speed as i64),
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

    pub fn on_deltas(&mut self, up_ms: f32, down_ms: f32) {
        self.pending.delta_up_ms = Some(up_ms);
        self.pending.delta_down_ms = Some(down_ms);
    }

    pub fn on_sync(&mut self, delay_ms: f32, err_ms: f32, prev_speed: f32, new_speed: f32) {
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

    pub fn on_node_slice_pre(&mut self, ts_us: u64) {
        self.last_node_slice_us = ts_us;
    }

    pub fn on_node_slice_size(&mut self, size: usize) {
        self.last_node_slice_size = size;
    }

    pub fn on_node_slice_post(&mut self, ts_us: u64, size: usize) {
        self.node_diff.push(SliceDiff {
            time_us: ts_us.saturating_sub(self.last_node_slice_us),
            // bytes: size.saturating_sub(self.last_node_slice_size),
            bytes: size,
        });
    }

    pub fn on_gw_slice_pre(&mut self, ts_us: u64) {
        self.last_gw_slice_us = ts_us;
    }

    pub fn on_gw_slice_post(&mut self, ts_us: u64, size: usize) {
        self.gw_diff.push(SliceDiff {
            time_us: ts_us.saturating_sub(self.last_gw_slice_us),
            // bytes: size.saturating_sub(self.last_gw_slice_size),
            bytes: size,
        });
    }

    pub fn on_gw_slice_size(&mut self, size: usize) {
        self.last_gw_slice_size = size;
    }
}

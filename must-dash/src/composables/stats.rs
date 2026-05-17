pub struct ChartData {
    /// Clock error series (was `delay` — now correctly uses err_ms)
    pub err: Vec<(f64, f64)>,
    pub up: Vec<(f64, f64)>,
    pub down: Vec<(f64, f64)>,
    pub hw: Vec<(f64, f64)>,
    /// Hardware-delay samples collected since the last SYNC (not yet finalized into a packet).
    /// Placed one interval to the right of the last completed packet.  Empty when scrolled back.
    pub hw_live: Vec<(f64, f64)>,
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
    /// Raw hardware-delay samples collected since the previous SYNC
    pub hw_samples: Vec<f32>,
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
    pub fn get_history_lines(&self, max_lines: usize, scroll: usize) -> Vec<Vec<String>> {
        let len = self.packets.len();
        let end = len.saturating_sub(scroll);
        let start = end.saturating_sub(max_lines);

        let fmt_ms = |v: f32| format!("{:.3}ms", v);
        let fmt_opt_ms = |v: Option<f32>| v.map_or("--".to_string(), fmt_ms);

        self.packets[start..end]
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
    ///
    /// All visible packets are stretched evenly across the full chart width `[0, num_points-1]`
    /// so the graph always fills the widget.  `scroll` shifts the visible window back in time
    /// (0 = most recent packets).
    ///
    /// HW delay samples for each packet are spread linearly between that packet's x position
    /// and the previous packet's, so they sit visually within the heartbeat interval.
    pub fn get_chart_data(&self, max_x_points: usize, scroll: usize) -> ChartData {
        let num_points = max_x_points.max(1);

        let end = self.packets.len().saturating_sub(scroll);
        let start = end.saturating_sub(num_points);
        let slice = &self.packets[start..end];

        // Map index i (0..n) → x in [0, num_points-1], stretching data to fill the chart.
        let stretch = |i: usize, n: usize| -> f64 {
            if n <= 1 {
                0.0
            } else {
                i as f64 * (num_points - 1) as f64 / (n - 1) as f64
            }
        };

        let n = slice.len();

        let err: Vec<(f64, f64)> = slice
            .iter()
            .enumerate()
            .map(|(i, p)| (stretch(i, n), p.err_ms as f64))
            .collect();

        let up: Vec<(f64, f64)> = slice
            .iter()
            .enumerate()
            .map(|(i, p)| (stretch(i, n), p.delta_up_ms.unwrap_or(0.0) as f64))
            .collect();

        let down: Vec<(f64, f64)> = slice
            .iter()
            .enumerate()
            .map(|(i, p)| (stretch(i, n), p.delta_down_ms.unwrap_or(0.0) as f64))
            .collect();

        // Spread each packet's hw samples linearly across (x_prev, x_current].
        let hw: Vec<(f64, f64)> = slice
            .iter()
            .enumerate()
            .flat_map(|(i, p)| {
                let x_right = stretch(i, n);
                let x_left = if i == 0 { 0.0 } else { stretch(i - 1, n) };
                let sn = p.hw_samples.len();
                p.hw_samples.iter().enumerate().map(move |(j, &v)| {
                    let x = if sn <= 1 {
                        x_right
                    } else {
                        x_left + (x_right - x_left) * j as f64 / (sn - 1) as f64
                    };
                    (x, v as f64)
                })
            })
            .collect();

        // Width of one inter-packet interval in chart units (used for the live zone).
        let interval = if n > 1 {
            (num_points - 1) as f64 / (n - 1) as f64
        } else {
            (num_points as f64).max(1.0)
        };

        // Pending hw samples (since last SYNC, not yet in any PacketEntry).
        // Only shown when not scrolled back — they represent the live "right edge."
        let hw_live: Vec<(f64, f64)> = if scroll == 0 {
            let pending = &self.hardware_delay[self.last_hw_idx..];
            let x_anchor = if n == 0 { 0.0 } else { stretch(n - 1, n) };
            let x_live_end = x_anchor + interval;
            let pn = pending.len();
            pending
                .iter()
                .enumerate()
                .map(|(j, &v)| {
                    let x = if pn <= 1 {
                        x_anchor
                    } else {
                        x_anchor + (x_live_end - x_anchor) * j as f64 / (pn - 1) as f64
                    };
                    (x, v as f64)
                })
                .collect()
        } else {
            vec![]
        };

        let x_max = if hw_live.is_empty() {
            (num_points - 1) as f64
        } else {
            (num_points - 1) as f64 + interval
        };

        // Y-axis bounds across all series
        let (min_val, max_val) = err
            .iter()
            .chain(up.iter())
            .chain(down.iter())
            .chain(hw.iter())
            .chain(hw_live.iter())
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
            hw_live,
            x_bounds: [0.0, x_max],
            y_bounds,
        }
    }

    pub fn on_deltas(&mut self, up_ms: f32, down_ms: f32) {
        self.pending.delta_up_ms = Some(up_ms);
        self.pending.delta_down_ms = Some(down_ms);
    }

    pub fn on_sync(&mut self, delay_ms: f32, err_ms: f32, prev_speed: f32, new_speed: f32) {
        // Consume all hardware-delay samples collected since last sync
        let hw_samples: Vec<f32> = self.hardware_delay[self.last_hw_idx..].to_vec();
        let mean_hw = if hw_samples.is_empty() {
            0.0
        } else {
            hw_samples.iter().sum::<f32>() / hw_samples.len() as f32
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
            hw_samples,
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

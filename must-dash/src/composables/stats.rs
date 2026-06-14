use serde::Serialize;

/// Flat, serializable row written to the main CSV.  Adding a column means adding a field here
/// and filling it in [`DashStats::csv_rows`] — the header and value order follow automatically.
#[derive(Serialize)]
pub struct MainCsvRow {
    pub node_id: String,
    pub probe_id: String,
    pub delay_ms: f32,
    pub err_ms: f32,
    pub prev_speed: f32,
    pub new_speed: f32,
    pub delta_up_ms: Option<f32>,
    pub delta_down_ms: Option<f32>,
    pub mean_hw_delay_ms: f32,
    pub node_time_us: Option<u64>,
    pub node_bytes: Option<usize>,
    pub tau_hb_high: bool,
    pub pkt_loss: f32,
    pub toa_sum: u64,
}

pub struct ChartData {
    /// Clock error series
    pub err: Vec<(f64, f64)>,
    pub up: Vec<(f64, f64)>,
    pub down: Vec<(f64, f64)>,
    pub hw: Vec<(f64, f64)>,
    /// Hardware-delay samples collected since the last SYNC (not yet finalized into a packet).
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
    pub mean_hw_delay_ms: f32,
    /// Raw hardware-delay samples collected since the previous SYNC
    pub hw_samples: Vec<f32>,
    /// tau_hb mode at transmit time: true = High, false = Low
    pub tau_hb_high: bool,
    pub pkt_loss: f32,
    pub toa_sum: u64,
}

/// Accumulated state for the packet that is currently being built.
/// Reset (via `mem::take`) each time `finalize_packet` is called.
#[derive(Default)]
struct PendingPacket {
    delta_up_ms: Option<f32>,
    delta_down_ms: Option<f32>,
    tau_hb_high: bool,
}

/// Time and byte-size diff between TAU_SLICE and TAU_SLICE_POST.
pub struct SliceDiff {
    pub time_us: u64,
    pub bytes: usize,
}

#[derive(Default)]
pub struct DashStats {
    /// One entry per completed SYNC cycle
    pub packets: Vec<PacketEntry>,
    /// One diff per node TAU_SLICE_POST
    pub node_diff: Vec<SliceDiff>,
    /// Data packets from this node that the gateway received during the run.
    pub pkt_received: u32,
    /// Packet-id gaps inferred from non-contiguous [GW_PKT] events (i.e. lost packets).
    pub pkt_skipped: u32,
    /// Highest packet_id observed at the gateway; `None` until the first packet arrives.
    pub pkt_last_id: Option<u16>,
    // --- private scratchpad, not for display ---
    pending: PendingPacket,
    last_node_slice_us: u64,
    last_node_slice_size: usize,
    last_pkt_loss: f32,
    toa_sum: u64,
}

impl DashStats {
    pub fn new() -> Self {
        Self {
            ..Default::default()
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
    /// Columns: Pkt | HW Avg | Err | Delay | Δ Up | Δ Down | Speed | τ_hb
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
                vec![
                    format!("{:02}", i),
                    fmt_ms(p.mean_hw_delay_ms),
                    fmt_ms(p.err_ms),
                    fmt_ms(p.delay_ms),
                    fmt_opt_ms(p.delta_up_ms),
                    fmt_opt_ms(p.delta_down_ms),
                    format!("{}", p.new_speed as i64),
                    if p.tau_hb_high { "Hi" } else { "Lo" }.to_string(),
                ]
            })
            .collect()
    }

    /// Prepares data for the chart.
    ///
    /// `hw_pending` is the slice of global hardware-delay samples collected since this
    /// node's last SYNC — passed in by the caller since the buffer lives in `App`.
    /// All visible packets are stretched evenly across `[0, num_points-1]`.
    /// `scroll` shifts the visible window back in time (0 = most recent packets).
    pub fn get_chart_data(
        &self,
        max_x_points: usize,
        scroll: usize,
        hw_pending: &[f32],
    ) -> ChartData {
        let num_points = max_x_points.max(1);

        let end = self.packets.len().saturating_sub(scroll);
        let start = end.saturating_sub(num_points);
        let slice = &self.packets[start..end];

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

        let interval = if n > 1 {
            (num_points - 1) as f64 / (n - 1) as f64
        } else {
            (num_points as f64).max(1.0)
        };

        let hw_live: Vec<(f64, f64)> = if scroll == 0 {
            let x_anchor = if n == 0 { 0.0 } else { stretch(n - 1, n) };
            let x_live_end = x_anchor + interval;
            let pn = hw_pending.len();
            hw_pending
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

    pub fn csv_rows<'a>(
        &'a self,
        node_label: &'a str,
        probe_id: &'a str,
    ) -> impl Iterator<Item = MainCsvRow> + 'a {
        self.packets.iter().enumerate().map(move |(i, p)| {
            let nd = self.node_diff.get(i);
            MainCsvRow {
                node_id: node_label.to_string(),
                probe_id: probe_id.to_string(),
                delay_ms: p.delay_ms,
                err_ms: p.err_ms,
                prev_speed: p.prev_speed,
                new_speed: p.new_speed,
                delta_up_ms: p.delta_up_ms,
                delta_down_ms: p.delta_down_ms,
                mean_hw_delay_ms: p.mean_hw_delay_ms,
                node_time_us: nd.map(|d| d.time_us),
                node_bytes: nd.map(|d| d.bytes),
                tau_hb_high: p.tau_hb_high,
                pkt_loss: p.pkt_loss,
                toa_sum: p.toa_sum,
            }
        })
    }

    pub fn on_deltas(&mut self, up_ms: f32, down_ms: f32) {
        self.pending.delta_up_ms = Some(up_ms);
        self.pending.delta_down_ms = Some(down_ms);
    }

    pub fn on_state_sync(&mut self, mode: &str) {
        self.pending.tau_hb_high = mode == "High";
    }

    pub fn on_packet_loss(&mut self, pkt_loss: f32) {
        self.last_pkt_loss = pkt_loss
    }

    pub fn on_toa(&mut self, toa_sum: u64) {
        self.toa_sum = toa_sum;
    }

    /// `hw_mean` and `hw_samples` are provided by the caller from the global HW buffer,
    /// sliced from this node's last consumed index to the current buffer length.
    pub fn on_sync(
        &mut self,
        delay_ms: f32,
        err_ms: f32,
        prev_speed: f32,
        new_speed: f32,
        hw_mean: f32,
        hw_samples: Vec<f32>,
    ) {
        let pending = std::mem::take(&mut self.pending);
        let pkt_loss = self.last_pkt_loss;
        self.packets.push(PacketEntry {
            delay_ms,
            err_ms,
            prev_speed,
            new_speed,
            delta_up_ms: pending.delta_up_ms,
            delta_down_ms: pending.delta_down_ms,
            mean_hw_delay_ms: hw_mean,
            hw_samples,
            tau_hb_high: pending.tau_hb_high,
            pkt_loss,
            toa_sum: self.toa_sum,
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
            bytes: size,
        });
    }

    /// This node's packet was successfully received by the gateway, check if the last packet id was
    /// more than 1 lower than this new id, because that would mean that there were some prior
    /// packets which were not received.
    pub fn on_gw_packet(&mut self, packet_id: u16) {
        self.pkt_received += 1;
        let diff = if let Some(last_id) = self.pkt_last_id {
            packet_id.wrapping_sub(last_id)
        } else {
            0
        };
        if diff > 0 && diff < 1024 {
            // remove the diff for this packet, if we missed one, then diff is 2, and we missed 2-1
            // packets
            if diff > 1 {
                self.pkt_skipped += (diff - 1) as u32;
            }
            self.pkt_last_id = Some(packet_id);
        }
    }
}

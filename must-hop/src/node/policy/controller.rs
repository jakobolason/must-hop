#[cfg(not(feature = "debug"))]
use core::marker::PhantomData;

use crate::node::RxPacket;
use crate::node::policy::tdma::SlotAllocation;

#[cfg(not(feature = "in_std"))]
use defmt::info;
#[cfg(feature = "in_std")]
use log::info;

use embassy_time::Instant;

pub(crate) struct Controller {
    /// speed of clock drift, used to try and mitigate clock drift at nodes with no HSE
    pub v_s: i64,
    ki: i64,
    kp: i64,
    prev_err: i64,
    prev_delay: i64,
}
impl Controller {
    pub(crate) fn new(v_s: i64, kp: i64, ki: i64) -> Self {
        Self {
            v_s,
            ki,
            kp,
            prev_err: 0,
            prev_delay: 0,
        }
    }
    /// v_s is current error, so this maps the error to drift ppb
    pub(crate) fn calc_drift_duration(&self, duration: u64) -> u64 {
        ((duration as i64 * self.v_s) / 1_000_000_000) as u64
    }
    pub(crate) fn run_transferfunction(
        &mut self,
        hb: &SlotAllocation,
        rx_pkt: RxPacket,
        sending_instant: Instant,
        time_sync: Option<(u64, Instant)>,
        node_id: u8,
    ) -> Option<(u64, Instant)> {
        let (v_s, time_sync, error, delay) =
            self.update_skew_and_stamp(hb, rx_pkt, sending_instant, time_sync, node_id);
        self.v_s = v_s;
        self.prev_delay = delay;
        self.prev_err = error;
        time_sync
    }
    /// Given a heartbeat packet from a nearer-gw node, this calculates the new timestamp and the
    /// new skew ratio for the node to be properly synchronized.
    fn update_skew_and_stamp(
        &self,
        hb: &SlotAllocation,
        rx_pkt: RxPacket,
        sending_instant: Instant,
        time_sync: Option<(u64, Instant)>,
        node_id: u8,
    ) -> (i64, Option<(u64, Instant)>, i64, i64) {
        let (old_gps, last_stamp) = match time_sync {
            Some(stamps) => stamps,
            None => {
                info!("[SYNC] Initial epoch set");
                return (self.v_s, Some((hb.gps_time_us, sending_instant)), 0, 0);
            }
        };

        // Calculate skews
        // let my_stamp = self.current_gps_time((old_gps, last_stamp));
        let my_diff = (sending_instant - last_stamp).as_micros();
        let predicted_elapsed = my_diff + self.calc_drift_duration(my_diff);
        let my_stamp = predicted_elapsed + old_gps;
        // instant was just when we received the preamble. But perhaps the difference between that
        // instant and now is the same as ToA
        let now = Instant::now();
        let difference = now - sending_instant;
        info!(
            "[TIMING] now: {}s | send: {}s | diff: {}ms | ",
            now.as_millis() as f32 / 1000.0,
            sending_instant.as_millis() as f32 / 1000.0,
            difference.as_micros() as f32 / 1000.0,
        );

        // Check if a t3 delta is availale for us
        let delay = if let Some((_, delta_up)) = hb.t3_deltas.iter().find(|t| t.0 == node_id) {
            // delta is our T3 - T2
            let delta_down = my_stamp as i64 - hb.gps_time_us as i64;
            let up_ms = *delta_up as f32 / 1000.0;
            let down_ms = delta_down as f32 / 1000.0;
            if delta_down.abs() > 1_000 || delta_up.abs() > 1_000 {
                // 2A. DELTAS REJECTED LOG
                info!(
                    "[DELTAS] up: {}ms | down: {}ms | status: REJECTED",
                    up_ms, down_ms
                );
                0
            } else {
                let nw_delay = (delta_down + *delta_up as i64) / 2;
                // 2B. DELTAS ACCEPTED LOG
                info!(
                    "[DELTAS] up: {}ms | down: {}ms | nw_delay: {}ms",
                    up_ms,
                    down_ms,
                    nw_delay as f32 / 1000.0
                );
                nw_delay
            }
        } else {
            0
        };

        // Simple filter ofr now
        let avg_delay = (self.prev_delay + delay) / 2;

        // Use the network delay to make up for transmission time, etc.
        let current_true_time = hb.gps_time_us as i64 - avg_delay;
        let time_sync = Some(((current_true_time) as u64, sending_instant));

        // Now update drift
        let gw_diff = current_true_time - old_gps as i64;

        let err = gw_diff - predicted_elapsed as i64;

        let delta_err = err - self.prev_err;
        let delta_u = (self.kp * delta_err) + (self.ki * err);

        let new_speed = self.v_s + delta_u;

        // Debug info:
        if my_stamp != hb.gps_time_us {
            let measured_delay: i64 = my_stamp as i64 - hb.gps_time_us as i64;
            info!(
                "[SYNC]   delay: {}ms | err: {}ms | v_prev: {} | v_new: {} |",
                measured_delay as f32 / 1000.0,
                err as f32 / 1000.0,
                self.v_s,
                new_speed,
            );
        } else {
            info!("[SYNC]   Perfectly synced?!");
        }

        (new_speed, time_sync, err, avg_delay)
    }
}

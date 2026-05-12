use crate::RxPacket;
use crate::policy::tdma::SlotAllocation;

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
    pub prev_err: i64,
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

    /// Controller figures out the error and corregates according to the recorded error
    pub(crate) fn run_transferfunction(
        &mut self,
        hb: &SlotAllocation,
        rx_pkt: RxPacket,
        time_sync: Option<(u64, Instant)>,
        node_id: u8,
    ) -> Option<(u64, Instant)> {
        let (v_s, time_sync, error, delay) = self.update_skew_and_stamp(
            hb,
            rx_pkt.rx_done_instant,
            time_sync,
            node_id,
            rx_pkt.payload_size as u64,
        );
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
        sending_instant: Instant,
        time_sync: Option<(u64, Instant)>,
        node_id: u8,
        _payload_size: u64,
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

        // Check if a t3 delta is availale for us
        let delay = if let Some((_, delta_up)) = hb.t3_deltas.iter().find(|t| t.0 == node_id) {
            // delta is our T3 - T2
            let delta_down = hb.gps_time_us as i64 - my_stamp as i64;
            let up_ms = *delta_up as f32 / 1000.0;
            let down_ms = delta_down as f32 / 1000.0;
            if delta_down.abs() > 20_000 || delta_up.abs() > 20_000 {
                info!("[DELTAS]|{}|{}| status: REJECTED", up_ms, down_ms);
                0
            } else {
                let nw_delay = (delta_down + *delta_up as i64) / 2;
                info!(
                    "[DELTAS]|{}|{}|{}|",
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

        let phase_err = gw_diff - predicted_elapsed as i64;
        // let freq_err = {
        //     if predicted_elapsed > ((tau_hb_us * 7) / 6) {
        //         // > 1.16*tau_hb_us
        //         0
        //     } else {
        //         tau_hb_us as i64 - predicted_elapsed as i64
        //     }
        // };

        // let err = freq_err + phase_err;
        let err = phase_err;

        let delta_err = err - self.prev_err;
        let delta_u = (self.kp * delta_err) + (self.ki * err);

        let new_speed = self.v_s + delta_u;

        // Debug info:
        if my_stamp != hb.gps_time_us {
            let measured_delay: i64 = hb.gps_time_us as i64 - my_stamp as i64;
            info!(
                "[SYNC]|{}|{}|{}|{}|",
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

#[cfg(test)]
mod controller_tests {
    use super::*;
    use crate::policy::tdma::SlotAllocation;
    use embassy_time::{Duration, Instant};

    fn make_controller(v_s: i64, kp: i64, ki: i64) -> Controller {
        Controller::new(v_s, kp, ki)
    }

    fn make_alloc(time: u64) -> SlotAllocation {
        let mut alloc = SlotAllocation::new();
        alloc.gps_time_us = time;
        alloc
    }

    fn make_rx_pkt(rx_done_instant: Instant) -> RxPacket {
        RxPacket {
            rx_done_instant,
            payload_size: 0,
        }
    }

    // Make sure the impl hasn't fucked royally up
    #[test]
    fn drift_duration_zero_skew_always_zero() {
        let c = make_controller(0, 0, 0);
        assert_eq!(c.calc_drift_duration(0), 0);
        assert_eq!(c.calc_drift_duration(1_000_000), 0);
        assert_eq!(c.calc_drift_duration(u64::MAX / 2), 0);
    }

    #[test]
    fn drift_duration_known_ppb_exact_result() {
        // v_s = 1_000 ppb → 1 µs of drift per 1_000_000 µs elapsed
        let c = make_controller(1_000, 0, 0);
        // 1_000_000 µs * 1_000 / 1_000_000_000 = 1 µs
        assert_eq!(c.calc_drift_duration(1_000_000), 1);
        // 1_000_000_000 µs * 1_000 / 1_000_000_000 = 1_000 µs
        assert_eq!(c.calc_drift_duration(1_000_000_000), 1_000);
    }

    #[test]
    fn drift_duration_scales_linearly_with_duration() {
        let c = make_controller(500_000, 0, 0);
        let d1 = c.calc_drift_duration(1_000_000);
        let d2 = c.calc_drift_duration(2_000_000);
        // Doubling the duration should double the drift
        assert_eq!(d2, d1 * 2);
    }

    // TODO: Useless
    #[test]
    fn transfer_fn_no_prior_sync_initialises_epoch_from_hb() {
        let mut c = make_controller(0, 0, 0);
        let now = Instant::now();
        let alloc = make_alloc(123_456_789);
        let rx = make_rx_pkt(now);

        let result = c.run_transferfunction(&alloc, rx, now, None, 1);

        // Should return the GPS time from the heartbeat as the initial epoch
        let (gps_us, _instant) = result.expect("Expected an initial time_sync to be returned");
        assert_eq!(
            gps_us, 123_456_789,
            "GPS time should match heartbeat's announcement"
        );
    }

    // Just to check that v_s is not changed, big fuck up in logic if this fails
    #[test]
    fn transfer_fn_no_prior_sync_preserves_v_s() {
        // v_s should not change on the very first packet — there's no error to correct yet
        let initial_v_s = 42_000_i64;
        let mut c = make_controller(initial_v_s, 0, 0);
        let now = Instant::now();
        let alloc = make_alloc(0);
        let rx = make_rx_pkt(now);

        c.run_transferfunction(&alloc, rx, now, None, 1);

        assert_eq!(
            c.v_s, initial_v_s,
            "v_s must not change on epoch initialisation"
        );
    }

    #[test]
    fn transfer_fn_zero_gains_preserves_v_s_when_synced() {
        // With kp=0, ki=0, any error should leave v_s unchanged
        let initial_v_s = 5_000_i64;
        let mut c = make_controller(initial_v_s, 0, 0);

        let base_instant = Instant::now();
        let elapsed = Duration::from_millis(500);
        let rx_instant = base_instant + elapsed;
        let gps_base: u64 = 1_000_000;

        // Simulate a heartbeat that arrives exactly 500ms after our epoch — no drift
        let hb_gps_time = gps_base + elapsed.as_micros();
        let alloc = make_alloc(hb_gps_time);
        let rx = make_rx_pkt(rx_instant);
        let prior_sync = Some((gps_base, base_instant));

        c.run_transferfunction(&alloc, rx, rx_instant, prior_sync, 1);

        assert_eq!(
            c.v_s, initial_v_s,
            "v_s must not change when there is no error and gains are zero"
        );
    }
}

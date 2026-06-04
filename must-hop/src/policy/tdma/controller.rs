use crate::RxPacket;
use crate::policy::tdma::SyncBeacon;

#[cfg(not(feature = "in_std"))]
use defmt::info;
#[cfg(feature = "in_std")]
use log::info;

use embassy_time::Instant;

#[derive(Default)]
pub(crate) struct Controller {
    /// speed of clock drift, used to try and mitigate clock drift at nodes with no HSE
    pub v_s: i64,
    ki: i64,
    kp: i64,
    pub prev_err: i64,
    leader_skip_frames: u8,
}
impl Controller {
    pub(crate) fn new(v_s: i64, kp: i64, ki: i64) -> Self {
        Self {
            v_s,
            ki,
            kp,
            leader_skip_frames: 1,
            ..Default::default()
        }
    }

    /// v_s is current error, so this maps the error to drift ppb
    pub(crate) fn calc_drift_duration(&self, duration: u64) -> i64 {
        (duration as i64 * self.v_s) / 1_000_000_000
    }

    /// Controller figures out the error and corregates according to the recorded error
    pub(crate) fn run_transferfunction(
        &mut self,
        hb: &SyncBeacon,
        rx_pkt: &RxPacket,
        time_sync: Option<(u64, Instant)>,
        node_id: u8,
    ) -> (u64, Instant) {
        // Unpack this to see if we have anything to compare to
        let some_stamps = match time_sync {
            Some(stamps) => stamps,
            None => {
                info!("[SYNC] Initial epoch set");
                return (hb.gps_time_us, rx_pkt.rx_done_instant);
            }
        };
        // Now calculate the error the controller can use
        let (time_sync, error, delay) =
            self.calc_error(hb, rx_pkt, some_stamps.0, some_stamps.1, node_id);
        // info!("LEADER IS IN {}", hb.tau_hb);

        self.leader_skip_frames = hb.tau_hb;
        // Conditional integration anti-windup
        // let tentative_v_s = self.apply_pi_controller(error);
        // let delta_vs = tentative_v_s - self.v_s;
        // let delta_err = error - self.prev_err;
        // let should_sum_error =
        //     delta_err == 0 || delta_vs == 0 || delta_vs.signum() == delta_err.signum();
        //
        // let v_s = if should_sum_error {
        //     info!("ADDING TO SUM");
        //     self.error_sum = (self.error_sum + error).clamp(-10_000, 10_000);
        //     self.apply_pi_controller(error)
        // } else {
        //     info!("NOT ADDING TO SUM");
        //     tentative_v_s
        // };
        let v_s = self.apply_pi_controller(error);
        // Saturate the change of speed
        let diff = v_s - self.v_s;
        let v_s = if diff.abs() > 2_000_000 {
            info!("SATURATED!! sign is {}", diff.signum());
            self.v_s + diff.signum() * 2_000_000
        } else {
            v_s
        };

        // Debug info:
        info!(
            "[SYNC]|{}|{}|{}|{}|",
            delay as f32 / 1000.0,
            error as f32 / 1000.0,
            self.v_s,
            v_s,
        );
        self.v_s = v_s;
        self.prev_err = error;

        time_sync
    }

    /// Calculates the error this node has from it's follower, using given parameters to approximate
    /// the timestamp
    fn calc_error(
        &self,
        hb: &SyncBeacon,
        rx_pkt: &RxPacket,
        old_gps: u64,
        last_stamp: Instant,
        node_id: u8,
    ) -> ((u64, Instant), i64, i64) {
        // Calculate skews
        let my_diff = (rx_pkt.rx_done_instant - last_stamp).as_micros();
        let predicted_elapsed = (my_diff as i64 + self.calc_drift_duration(my_diff)) as u64;
        let my_stamp = predicted_elapsed + old_gps;

        // Check if a t3 delta is availale for us
        let nw_delay = if let Some((_, delta_up)) = hb.feedback_vec.iter().find(|t| t.0 == node_id)
        {
            // delta is our T3 - T2
            let delta_down = hb.gps_time_us as i64 - my_stamp as i64;
            // FIXME:
            // For some reason, this can be 5secs, so filter out those readings
            let delta_down = if delta_down > 1_000_000 {
                0
            } else {
                delta_down
            };
            let up_ms = *delta_up as f32 / 1000.0;
            let down_ms = delta_down as f32 / 1000.0;
            // if delta_down.abs() > 20_000 || delta_up.abs() > 30_000 {
            //     info!("[DELTAS]|{}|{}| status: REJECTED", up_ms, down_ms);
            //     0
            // } else {
            let nw_delay = (delta_down + *delta_up as i64) / 2;
            info!(
                "[DELTAS]|{}|{}|{}|",
                up_ms,
                down_ms,
                nw_delay as f32 / 1000.0
            );
            // nw_delay
            (*delta_up / 2) as i64
            // }
        } else {
            0
        };

        // Simple filter ofr now
        // let avg_delay = (self.prev_delay + nw_delay) / 2;

        // Use the network delay to make up for transmission time, etc.
        let current_true_time = hb.gps_time_us as i64 + nw_delay;

        // Now update drift
        let gw_diff = current_true_time - old_gps as i64;

        let err = gw_diff - predicted_elapsed as i64;

        // Only re-sync if the error is substantially large
        let time_sync = if err.abs() > 70_000 {
            // re-sync means the last error was not enough to put the controller onto the correct speed,
            // to not fuck up the controller, adjust the stamp such that it doesn't get too out of sync
            let adjustment = if err > 0 { -50_000 } else { 50_000 };
            (
                ((current_true_time + adjustment) as u64),
                rx_pkt.rx_done_instant,
            )
        } else {
            ((my_stamp), rx_pkt.rx_done_instant)
        };
        // let time_sync = Some(((my_stamp), sending_instant));

        // let delta_err = err - self.prev_err;
        let delay = hb.gps_time_us as i64 - my_stamp as i64;
        (time_sync, err, delay)
    }

    fn apply_pi_controller(&self, err: i64) -> i64 {
        let kp_term = (self.kp * (err - self.prev_err)) / self.leader_skip_frames as i64;

        let ki_term = (self.ki * err) / self.leader_skip_frames as i64;
        info!("Kp term: {}, Ki term: {}", kp_term, ki_term);
        let delta_u = kp_term + ki_term;
        self.v_s + delta_u
        // (self.kp * err) / 10 + (self.ki * self.error_sum) / 100
    }
}

#[cfg(test)]
mod controller_tests {
    use super::*;
    use crate::policy::tdma::SyncBeacon;
    use embassy_time::{Duration, Instant};

    fn make_controller(v_s: i64, kp: i64, ki: i64) -> Controller {
        Controller::new(v_s, kp, ki)
    }

    fn make_alloc(time: u64) -> SyncBeacon {
        let mut alloc = SyncBeacon::new();
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
        let mut c = make_controller(500_000, 0, 0);
        c.leader_skip_frames = 10;
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

        // Should return the GPS time from the heartbeat as the initial epoch
        let (gps_us, _instant) = c.run_transferfunction(&alloc, &rx, None, 1);

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

        c.run_transferfunction(&alloc, &rx, None, 1);

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

        c.run_transferfunction(&alloc, &rx, prior_sync, 1);

        assert_eq!(
            c.v_s, initial_v_s,
            "v_s must not change when there is no error and gains are zero"
        );
    }
}

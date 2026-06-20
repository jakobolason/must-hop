use crate::RxPacket;
use crate::policy::tdma::SyncBeacon;

#[cfg(not(feature = "in_std"))]
use defmt::info;
#[cfg(feature = "in_std")]
use log::info;

use embassy_time::Instant;
use heapless::Deque;

struct Window<const N: usize> {
    data: Deque<u32, N>,
}

impl<const N: usize> Default for Window<N> {
    fn default() -> Self {
        Self { data: Deque::new() }
    }
}

impl<const N: usize> Window<N> {
    fn add(&mut self, val: u32) {
        if self.data.len() == N {
            self.data.pop_front();
        }
        let _ = self.data.push_back(val);
    }

    fn get(&self) -> u64 {
        let n = self.data.len();
        if n == 0 {
            return 0;
        }
        let sum: u64 = self.data.iter().map(|&x| x as u64).sum();
        sum / n as u64
    }

    fn min(&self) -> u32 {
        self.data.iter().copied().min().unwrap_or(u32::MAX)
    }

    fn len(&self) -> usize {
        self.data.len()
    }
}

pub struct BlueOs<const N: usize> {
    v_bar: Window<N>,
    u_bar: Window<N>,
}

impl<const N: usize> Default for BlueOs<N> {
    fn default() -> Self {
        Self {
            v_bar: Window::default(),
            u_bar: Window::default(),
        }
    }
}

impl<const N: usize> BlueOs<N> {
    pub fn add_uv(&mut self, u: u32, v: u32) {
        self.v_bar.add(v);
        self.u_bar.add(u);
    }

    fn v1(&self) -> u32 {
        self.v_bar.min()
    }

    fn u1(&self) -> u32 {
        self.u_bar.min()
    }

    pub fn parameter_estimation(&self) -> (u32, u32, u32) {
        if self.u_bar.len() < 2 {
            info!("Shorting out! {}, {}", self.u1(), self.v1());
            return (0, 0, 0);
        }
        let u1 = self.u1();
        let v1 = self.v1();
        // info!(
        //     "Calculating *BLUE_OS*: v1={}, u1={}, bv={}, bu={}",
        //     v1,
        //     u1,
        //     self.v_bar.get(),
        //     self.u_bar.get()
        // );
        let u1pv1 = u1 as u64 + v1 as u64;
        let bars_u1pv1 = self.v_bar.get() + self.u_bar.get();
        let n = self.u_bar.len() as u64;
        let scalar: f32 = 1.0 / (2 * (n - 1)) as f32;
        info!(
            "Scalar={}, u1pv1={}, bars={}, n={}",
            scalar, u1pv1, bars_u1pv1, n
        );
        let delay = scalar * (n * u1pv1 - bars_u1pv1) as f32;
        let offset = ((u1 as i64 - v1 as i64) as f32) / 2.0;
        let bias = scalar * (n * (bars_u1pv1 - u1pv1)) as f32;

        (
            delay.max(0.0) as u32,
            offset.max(0.0) as u32,
            bias.max(0.0) as u32,
        )
    }

    pub fn avg_delay(&self) -> u64 {
        (self.u_bar.get() + self.v_bar.get()) / 2
    }

    pub fn calc_offset(&self) -> Option<i64> {
        if self.u_bar.len() < 1 {
            return None;
        }
        let u1 = self.u1() as i64;
        let v1 = self.v1() as i64;
        Some((u1 - v1) / 2)
    }
}

const VF_SATURATION: i64 = 100_000;
// mu s
const DELTA_MAX: i32 = 50_000;

#[derive(Default)]
pub(crate) struct Controller<const N: usize> {
    /// speed of clock drift, used to try and mitigate clock drift at nodes with no HSE
    pub v_s: i64,
    ki: i64,
    kp: i64,
    pub prev_err: i64,
    leader_skip_frames: u8,
    blue_os: BlueOs<N>,
}
impl<const N: usize> Controller<N> {
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
        node_id: u16,
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

        self.leader_skip_frames = hb.skipped_frames;

        let v_s = self.apply_pi_controller(error);
        // Saturate the change of speed
        let diff = v_s - self.v_s;
        let v_s = if diff.abs() > VF_SATURATION {
            info!("SATURATED!! sign is {}", diff.signum());
            self.v_s + diff.signum() * VF_SATURATION
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
        &mut self,
        hb: &SyncBeacon,
        rx_pkt: &RxPacket,
        old_gps: u64,
        last_stamp: Instant,
        node_id: u16,
    ) -> ((u64, Instant), i64, i64) {
        let my_diff = (rx_pkt.rx_done_instant - last_stamp).as_micros();
        let predicted_elapsed = (my_diff as i64 + self.calc_drift_duration(my_diff)) as u64;
        let my_stamp = predicted_elapsed + old_gps;

        // Check if a t3 delta is availale for us
        let (dup, ddown) = if let Some((_, delta_up)) =
            hb.feedback_vec.iter().find(|t| t.0 == node_id)
        // Possible bug fix, 
            && hb.gps_time_us != 0
        {
            let v = my_stamp as i64 - hb.gps_time_us as i64;
            if v > 0 && v.abs() < DELTA_MAX as i64 && *delta_up > 0 && delta_up.abs() < DELTA_MAX {
                self.blue_os.add_uv(*delta_up as u32, v as u32);
            }
            (*delta_up, v)
        } else {
            (0, 0)
        };
        let (nw_delay, offset) = if let Some(offset_blue) = self.blue_os.calc_offset() {
            // let (_delay, _offset, bias) = self.blue_os.parameter_estimation();

            let nw_delay = self.blue_os.avg_delay();
            // info!(
            //     " !!!!! ---- USING BLUE OS = {},\tdelay={}, bias={}",
            //     offset_blue, nw_delay, bias
            // );
            (nw_delay as i64, offset_blue.max(0) as u64)
        } else if dup.abs() < DELTA_MAX && ddown.abs() < DELTA_MAX as i64 {
            ((ddown + dup as i64) / 2, 0)
        } else {
            (0, 0)
        };
        info!(
            "[DELTAS]|{}|{}|{}|",
            dup as f32 / 1000.0,
            ddown as f32 / 1000.0,
            nw_delay as f32 / 1000.0
        );

        // Use the network delay to make up for transmission time, etc.
        let current_true_time = hb.gps_time_us as i64 + nw_delay;
        let gw_diff = current_true_time - old_gps as i64;
        // error ms - drift ms
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
            // let adjusted_instant = rx_pkt.rx_done_instant + Duration::from_micros(offset);
            ((my_stamp + offset), rx_pkt.rx_done_instant)
        };

        (time_sync, err, nw_delay)
    }

    fn apply_pi_controller(&self, err: i64) -> i64 {
        // to ms
        // let err = err / 1000;
        let kp_term = (self.kp * (err - self.prev_err)) / self.leader_skip_frames as i64;

        let ki_term = (self.ki * err) / self.leader_skip_frames as i64;
        // info!("Kp term: {}, Ki term: {}", kp_term, ki_term);
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

    fn make_controller(v_s: i64, kp: i64, ki: i64) -> Controller<10> {
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

use crate::{MHNode, PacketType, RxPacket, policy::tdma::slots::SlotMask};

#[cfg(not(feature = "in_std"))]
use defmt::{debug, error, info};
#[cfg(feature = "in_std")]
use log::{debug, error, info};

#[cfg(feature = "debug")]
use embedded_hal::digital::OutputPin;

use core::{marker::PhantomData, num::NonZeroU8};
use postcard::{from_bytes, ser_flavors::Size, serialize_with_flavor, to_slice};
use serde::{Deserialize, Serialize};

use crate::{
    MHPacket,
    policy::{GATEWAY_ID, MacPolicy},
};
mod controller;
mod slots;

use controller::Controller;

use embassy_time::{Duration, Instant, Timer};
use heapless::Vec;

type VecT3 = Vec<(u8, i32), 5>;

#[derive(Serialize, Deserialize)]
pub(crate) struct SlotAllocation {
    my_slot: u8,
    /// Bit mask for known slots, meaning only 8 nodes can be known at a time
    known_slots: u8,
    // The tau_hb duration in secs, should be converted back to a Duration on followers
    tau_hb: u8,
    pub(crate) gps_time_us: u64,
    /// A list of (node_id, T3 - T2 delta in ms) for PTP
    pub(crate) t3_deltas: VecT3,
}

/// Only used for tests
#[allow(dead_code)]
impl SlotAllocation {
    pub(super) fn new() -> Self {
        Self {
            my_slot: 1,
            known_slots: 0,
            tau_hb: 10,
            gps_time_us: 0,
            t3_deltas: Vec::new(),
        }
    }
}

#[cfg(feature = "debug")]
pub trait DebugPin: OutputPin {}
#[cfg(feature = "debug")]
impl<T: embedded_hal::digital::OutputPin> DebugPin for T {}
#[cfg(not(feature = "debug"))]
pub trait DebugPin {}
#[cfg(not(feature = "debug"))]
impl<T> DebugPin for T {}

impl<P, const SIZE: usize> TdmaMac<Builder, P, SIZE> {
    pub fn new(
        // FIXME: Remove from user, should be set by GW
        slot_duration: Duration,
        // FIXME: Remove from user, should be set by GW
        slots_per_frame: core::num::NonZeroU8,
        time_sync: Option<(u64, Instant)>,
        known_skew_ratio: Option<i64>,
    ) -> Self {
        let controller = Controller::new(known_skew_ratio.unwrap_or(0_i64), 0, 0);
        Self {
            _state: PhantomData,
            slot_manager: SlotManager {
                slot_duration,
                tau_hb: TauHbMode::Low,
                hb_countdown: 0,
                slots_per_frame: slots_per_frame.into(),
                my_tx_slot: None,
                known_slots_mask: SlotMask::default(),
                node_id: 0,
                gw_hops: 255,
                leader_id: None,
            },
            time_manager: TimeManager {
                time_sync,
                last_hb_instant: None,
                hbt_pkt: None,
                t3_deltas: Vec::new(),
                controller,
                err_threshold: ERR_THRESHOLD,
                sync_counter: 0,
                out_of_sync: true,
            },
            counter: 0,

            #[cfg(feature = "debug")]
            debug_pin: None,
            #[cfg(not(feature = "debug"))]
            _marker: PhantomData,
        }
    }

    pub fn set_controller(self, v_s: i64, kp: i64, ki: i64) -> Self {
        let controller = Controller::new(v_s, kp, ki);
        Self {
            time_manager: TimeManager {
                controller,
                ..self.time_manager
            },
            ..self
        }
    }

    pub fn set_time_sync(self, time_sync: (u64, Instant)) -> Self {
        Self {
            time_manager: TimeManager {
                time_sync: Some(time_sync),
                ..self.time_manager
            },
            ..self
        }
    }

    pub fn set_node_id(self, node_id: u8) -> Self {
        Self {
            slot_manager: SlotManager {
                node_id,
                ..self.slot_manager
            },
            ..self
        }
    }

    pub fn set_tx_slot(self, tx_slot: u8) -> Self {
        Self {
            slot_manager: SlotManager {
                my_tx_slot: Some(tx_slot),
                ..self.slot_manager
            },
            ..self
        }
    }

    pub fn set_tau_hb(self, tau_hb: u8) -> Self {
        let tau_hb = TauHbMode::from_skip_count(tau_hb);
        Self {
            slot_manager: SlotManager {
                tau_hb,
                ..self.slot_manager
            },
            ..self
        }
    }

    #[cfg(feature = "debug")]
    pub fn set_debug_pin(self, pin: P) -> Self {
        Self {
            debug_pin: Some(pin),
            ..self
        }
    }

    pub fn build(self) -> TdmaMac<Runner, P, SIZE> {
        TdmaMac::<Runner, P, SIZE> {
            _state: PhantomData,
            slot_manager: self.slot_manager,
            time_manager: self.time_manager,
            counter: self.counter,
            #[cfg(feature = "debug")]
            debug_pin: self.debug_pin,
            #[cfg(not(feature = "debug"))]
            _marker: PhantomData,
        }
    }
}

impl<P, const SIZE: usize> Default for TdmaMac<Builder, P, SIZE> {
    fn default() -> Self {
        TdmaMac::new(
            Duration::from_secs(1),
            NonZeroU8::new(10).unwrap(),
            None,
            None,
        )
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
#[cfg_attr(not(feature = "in_std"), derive(defmt::Format))]
enum TauHbMode {
    High,
    Low,
}

impl TauHbMode {
    pub const fn skip_slots(&self) -> u8 {
        match self {
            Self::High => 3,
            Self::Low => 1,
        }
    }

    pub fn from_skip_count(count: u8) -> Self {
        match count {
            1 => Self::Low,
            _ => Self::High,
        }
    }
}

pub(crate) struct SlotManager {
    slot_duration: Duration,
    slots_per_frame: u8,
    my_tx_slot: Option<u8>,
    tau_hb: TauHbMode,
    hb_countdown: u8,
    /// A mask to know what other node's one know
    known_slots_mask: SlotMask,
    /// Used for the slot allocation. You should convert the MAC address into a u32 with the
    /// biggest chance of two nodes not having the same u32 representation
    node_id: u8,
    gw_hops: u8,
    leader_id: Option<u8>,
}

const ERR_THRESHOLD: u32 = 10_000; // 10ms
// If received 10 synced messages, then go up into high tau mode
const IN_SYNC_THRESHOLD: u8 = 10;
// How long it takes, before a node goes back into full listening mode
const HB_TIMEOUT: Duration = Duration::from_secs(120);
pub(crate) struct TimeManager<const SIZE: usize> {
    /// a tuple of timestamp in micros, and instant when that timestamp was set
    time_sync: Option<(u64, Instant)>,
    last_hb_instant: Option<Instant>,
    hbt_pkt: Option<MHPacket<SIZE>>,
    /// A list of (node_id, T3 - T2 delta in ms) for PTP
    t3_deltas: VecT3,
    /// Handles error correction
    controller: Controller,
    /// The same type as from t3_deltas
    err_threshold: u32,
    /// Sync counter, if a node is out of sync, it adds one to this.
    sync_counter: u8,
    /// If a single node is out of sync, we set this to go into low tau
    out_of_sync: bool,
}

pub struct Builder;
pub struct Runner;

/// A TDMA MAC policy, which synchronizes nodes across the network to only listen when known slots
/// could be transmitting, saving power.
pub struct TdmaMac<State, P, const SIZE: usize> {
    _state: PhantomData<State>,
    slot_manager: SlotManager,
    time_manager: TimeManager<SIZE>,

    // FIXME: Remove later
    counter: u8,
    #[cfg(feature = "debug")]
    pub debug_pin: Option<P>,
    #[cfg(not(feature = "debug"))]
    _marker: PhantomData<P>,
}

impl<P, const SIZE: usize> TdmaMac<Runner, P, SIZE> {
    fn current_gps_time(&self, stamps: (u64, Instant)) -> u64 {
        self.gps_time_at(stamps, Instant::now())
    }

    fn gps_time_at(&self, (base_gps_us, sync_instant): (u64, Instant), at_instant: Instant) -> u64 {
        let elapsed_us = (at_instant - sync_instant).as_micros();
        let gw_elapsed_us =
            elapsed_us + self.time_manager.controller.calc_drift_duration(elapsed_us);
        // let gw_elapsed_ms = ((elapsed_ms as u128 * self.skew_gw_diff as u128)
        //     / self.skew_local_diff as u128) as u64;
        base_gps_us + gw_elapsed_us
    }

    fn calc_nextslot_time(&self, timestamps: (u64, Instant)) -> Instant {
        let current_gps_time = self.current_gps_time(timestamps);
        let slot_dur = self.slot_manager.slot_duration.as_micros();

        let elapsed_in_current_slot = current_gps_time % slot_dur;

        let next_slot_start_offset = slot_dur - elapsed_in_current_slot;
        // let node_offset = ((next_slot_start_offset as u128 * self.skew_local_diff as u128)
        //     / self.skew_gw_diff as u128) as u64;
        let node_offset = next_slot_start_offset
            - self
                .time_manager
                .controller
                .calc_drift_duration(next_slot_start_offset);

        // Wake up just a bit before the slot starts to ensure listening at correct time
        // FIXME: If everyone does this, then everyone just wakes up 5ms before
        // let guard_band = 5000;
        Instant::now() + Duration::from_micros(node_offset /*.saturating_sub(guard_band) */)
    }

    pub fn current_slot(&self, current_time_us: u64) -> u8 {
        let frame_duration_us = (self.slot_manager.slot_duration.as_micros())
            * (self.slot_manager.slots_per_frame as u64);
        let time_in_frame = current_time_us % frame_duration_us;

        // Add half of slot duration to achieve 'round to nearest' int division
        let slot_dur = self.slot_manager.slot_duration.as_micros();
        ((time_in_frame + (slot_dur / 2)) / slot_dur) as u8
    }
    fn reset_sync(&mut self) {
        self.time_manager.time_sync = None;
        self.time_manager.last_hb_instant = None;
        self.time_manager.sync_counter = 0;
        self.time_manager.out_of_sync = true;
        self.slot_manager.my_tx_slot = None;
        self.slot_manager.leader_id = None;
        self.slot_manager.known_slots_mask = SlotMask::default();
        self.slot_manager.tau_hb = TauHbMode::Low;
        self.slot_manager.hb_countdown = 0;
    }

    fn sync_epoch(&mut self, pkts: &[MHPacket<SIZE>], rx_pkt: RxPacket) {
        // Look for a heartbeat and get allocation from byte slices
        let received_hb = pkts
            .iter()
            .filter(|pkt| {
                pkt.packet_type == PacketType::HeartBeat
                // The GW can hear it's own packets
                    && pkt.source_id != self.slot_manager.node_id
            })
            .find_map(|pkt| {
                from_bytes::<SlotAllocation>(&pkt.payload)
                    .ok()
                    .map(|alloc| (pkt, alloc))
            });

        // Check for there not being a heartbeat
        let (pkt, alloc) = match received_hb {
            Some(tple) => tple,
            None => return,
        };
        // Resync node to heartbeat's announced slot, if hb came closer to gw than me
        let (src, val) = if self.slot_manager.node_id != GATEWAY_ID
            && pkt.hop_to_gw < self.slot_manager.gw_hops
        {
            self.time_manager.last_hb_instant = Some(Instant::now());
            // Controller updates internal drift, and returns adjusted stamps
            self.time_manager.time_sync = self.time_manager.controller.run_transferfunction(
                &alloc,
                rx_pkt,
                self.time_manager.time_sync,
                self.slot_manager
                    .my_tx_slot
                    .unwrap_or(self.slot_manager.node_id),
            );
            // TODO:
            // denote this as a leader node. This should only be set once (with a timeout
            // perhaps) such that 2 equal leader nodes don't make this follower node unstable
            let leader_id = match self.slot_manager.leader_id {
                Some(lid) => lid,
                None => {
                    self.slot_manager.leader_id = Some(pkt.source_id);
                    self.time_manager.last_hb_instant = Some(Instant::now());
                    pkt.source_id
                }
            };
            // If this is leader, then we check if the tau_hb matches theirs
            // if leader_id == pkt.source_id && alloc.tau_hb != self.slot_manager.tau_hb.as_secs() {
            //     self.slot_manager.tau_hb = TauHbMode::from_secs(alloc.tau_hb);
            //     info!("Swicthed Tau mode to {:?}", self.slot_manager.tau_hb);
            // }
            (leader_id, self.time_manager.controller.prev_err as i32)
        } else {
            // If a follower's error is above threshold, increase out of sync counter
            if let Some((_, prev_err)) = alloc
                .t3_deltas
                .iter()
                .find(|t| t.0 == self.slot_manager.node_id)
                && prev_err.unsigned_abs() > self.time_manager.err_threshold
            {
                info!("Node out of sync ...{}", prev_err);
                self.time_manager.out_of_sync = true;
            }

            // if we are GW, then we want to update out t3 deltas on this node
            let t3 = if let Some(stamps) = self.time_manager.time_sync {
                // Cast to i64 to not panic at my_time < alloc
                let time_at_rx = self.gps_time_at(stamps, rx_pkt.rx_done_instant) as i64;
                (time_at_rx - alloc.gps_time_us as i64) as i32
            } else {
                0
            };
            (pkt.source_id, t3)
        };
        // Leader calculates t3s, and follower returns calculated error
        match self.time_manager.t3_deltas.iter().position(|t| t.0 == src) {
            Some(idx) => self.time_manager.t3_deltas[idx] = (src, val),
            None => {
                if self.time_manager.t3_deltas.push((src, val)).is_err() {
                    error!("T3 deltas is full!")
                }
            }
        }
        self.slot_manager.known_slots_mask.claim(alloc.my_slot);
        // info!(
        //     "Other node claimed slot at {}. {:?}",
        //     alloc.my_slot, self.slot_manager.known_slots_mask
        // );

        // Only allocate a new slot if we don't have one
        if self.slot_manager.my_tx_slot.is_none() {
            match self.slot_manager.known_slots_mask.slot_assignment_strat(
                self.slot_manager.slots_per_frame,
                alloc.known_slots,
                self.slot_manager.node_id,
            ) {
                Some(free_slot) => {
                    self.slot_manager.my_tx_slot = Some(free_slot);
                    self.slot_manager.known_slots_mask.claim(free_slot);
                }
                None => error!("Network is full!"),
            }
        }
    }

    fn calc_adjust_timestamp(&self, toa: u64) -> u64 {
        let tx_stamp = match self.time_manager.time_sync {
            Some(stamps) => self.current_gps_time(stamps),
            None => {
                error!("There was no stamps in heartbeat updating??");
                0
            }
        };
        info!("[TAU_SLICE]|{}|", Instant::now().as_micros());
        // FIXME: Remember setting this
        let measured_spi_delay = 0;
        tx_stamp + toa + measured_spi_delay
    }

    fn approximate_packet_size<const LEN: usize>(
        &self,
        my_tx_slot: u8,
        t3_deltas: VecT3,
        tx_queue: &Vec<MHPacket<SIZE>, LEN>,
    ) -> usize {
        let dummy_allocation = SlotAllocation {
            my_slot: my_tx_slot,
            known_slots: self.slot_manager.known_slots_mask.as_u32(),
            tau_hb: self.slot_manager.tau_hb.skip_slots(),
            gps_time_us: 1, // Value doesn't matter for size, only the type (u64)
            t3_deltas,
        };
        let alloc_size = serialize_with_flavor(&dummy_allocation, Size::default()).unwrap();

        let mut dummy_pkt: MHPacket<SIZE> = MHPacket {
            destination_id: GATEWAY_ID,
            packet_type: PacketType::HeartBeat,
            packet_id: 0,
            source_id: self.slot_manager.node_id,
            payload: Vec::new(),
            hop_count: 0,
            hop_to_gw: self.slot_manager.gw_hops,
        };

        for _ in 0..alloc_size {
            let _ = dummy_pkt.payload.push(0);
        }
        let hbt_size =
            serialize_with_flavor(&dummy_pkt, Size::default()).expect("failed to size hbt");

        let queue_size =
            serialize_with_flavor(tx_queue, Size::default()).expect("Failed to size tx queue");
        let mut temp_queue = tx_queue.clone();
        let _ = temp_queue.push(dummy_pkt);
        let mut buffer = [0u8; 255];
        let used_slice = match to_slice(&temp_queue, &mut buffer) {
            Ok(slice) => slice,
            Err(e) => {
                error!("Serialization failed: {:?}", e);
                return 0;
            }
        };
        // FIXME: Remember setting this
        let measured_constant_offset = 0;
        let total_size = used_slice.len() + measured_constant_offset;

        info!("[SIZE EXPECTED]|{}|", total_size);
        total_size
    }

    fn update_tau_hb(&mut self) {
        match self.slot_manager.tau_hb {
            TauHbMode::High => {
                // We check if we are out of sync
                if self.time_manager.out_of_sync {
                    self.slot_manager.tau_hb = TauHbMode::Low;
                    self.time_manager.sync_counter = 0;
                    self.time_manager.out_of_sync = false;

                    info!("Drift detected so changed to low");
                }
            }
            TauHbMode::Low => {
                // If in low, we check if enough nodes are synced
                if self.time_manager.out_of_sync {
                    self.time_manager.sync_counter = 0;
                    info!("STILL OUT OF SYNC");
                    // Reset it, so this is tested again in the next run
                    // NOTE: Only reset here, not in sync_epoch
                    self.time_manager.out_of_sync = false;
                } else {
                    self.time_manager.sync_counter =
                        self.time_manager.sync_counter.saturating_add(1);
                    info!("THEY WAS IN SYNC");
                    if self.time_manager.sync_counter >= IN_SYNC_THRESHOLD {
                        self.slot_manager.tau_hb = TauHbMode::High;
                        // Reset sync counter for next time an out of sync happens
                        self.time_manager.sync_counter = 0;
                        info!("Is in sync, so switched to high tau");
                    }
                }
            }
        }
    }

    fn update_heartbeat(
        &self,
        mut hbt: MHPacket<SIZE>,
        my_tx_slot: u8,
        t3_deltas: VecT3,
        adjusted_timestamp: u64,
    ) -> MHPacket<SIZE> {
        let allocation = SlotAllocation {
            my_slot: my_tx_slot,
            known_slots: self.slot_manager.known_slots_mask.as_u32(),
            tau_hb: self.slot_manager.tau_hb.skip_slots(),
            gps_time_us: adjusted_timestamp,
            t3_deltas,
        };
        let mut buf = [0u8; SIZE];
        if let Ok(serialized_slice) = to_slice(&allocation, &mut buf) {
            hbt.payload.clear();
            let _ = hbt.payload.extend_from_slice(serialized_slice);
        }
        hbt
    }
}

impl<Node, const SIZE: usize, const LEN: usize, P> MacPolicy<Node, SIZE, LEN>
    for TdmaMac<Runner, P, SIZE>
where
    Node: MHNode<SIZE, LEN>,
    P: DebugPin,
{
    /// Used to know whether this node is a follower or a leader
    fn set_gw_hops(&mut self, gw_hops: u8) {
        self.slot_manager.gw_hops = gw_hops
    }

    fn should_tx_heartbeat(&mut self) -> bool {
        if self.slot_manager.hb_countdown == 0 {
            self.slot_manager.hb_countdown =
                self.slot_manager.tau_hb.skip_slots().saturating_sub(1);
            true
        } else {
            self.slot_manager.hb_countdown -= 1;
            false
        }
    }

    /// This only sends if you have been given a slot. GW should choose it's own slot, such that it
    /// can start this.
    fn tx_heartbeat(&mut self, hbt: MHPacket<SIZE>) {
        self.time_manager.hbt_pkt = Some(hbt)
    }

    async fn run_mac(
        &mut self,
        node: &mut Node,
        tx_queue: &mut Vec<MHPacket<SIZE>, LEN>,
        rx_buffer: &mut Node::ReceiveBuffer,
    ) -> Result<Option<Vec<MHPacket<SIZE>, LEN>>, Node::Error> {
        let mut received_packets = Vec::new();

        // Don't transmit anything until you have a slot, which you only have once you've heard a
        // heartbeat.
        // TODO: Go back into this, if not heard a heartbeat in some time
        let Some(timestamps) = self.time_manager.time_sync else {
            info!("TDMA: Waiting for first packet to sync");
            let conn = node
                .listen(rx_buffer, Some(core::time::Duration::from_secs(10)))
                .await;
            match conn {
                Ok(conn) => {
                    let (rec, rx_pkt) = node.receive(conn, rx_buffer).await?;
                    // Check for being heartbeat
                    self.sync_epoch(&rec, rx_pkt);
                    return Ok(Some(rec));
                }
                Err(e) => {
                    info!("Error in getting conn: {:?}", e);
                    return Ok(None);
                }
            }
        };

        // Sleep until right before the next slot time, to avoid jitter
        let next_slot_time = self.calc_nextslot_time(timestamps);
        Timer::at(next_slot_time).await;

        // get current slot
        let slot = self.current_slot(self.current_gps_time(timestamps));

        debug!("current slot: {}", slot);
        #[cfg(feature = "debug")]
        if let Some(pin) = self.debug_pin.as_mut() {
            let _ = pin.set_high();
        }

        if let Some(my_tx_slot) = self.slot_manager.my_tx_slot
            && slot == my_tx_slot
        {
            // debug!(" !!!  MY SLOT !!! ");
            // NOTE: Introduce a 100ms delay here, to ensure nodes are listening in your slot
            Timer::after(Duration::from_millis(100)).await;

            if !tx_queue.is_full()
                && let Some(pkt) = self.time_manager.hbt_pkt.take()
            {
                let extracted_deltas: VecT3 = core::mem::take(&mut self.time_manager.t3_deltas);
                let adjusted_timestamp = self.calc_adjust_timestamp(node.calc_tx_delay(
                    self.approximate_packet_size(my_tx_slot, extracted_deltas.clone(), tx_queue),
                ));
                self.update_tau_hb();
                let upd_pkt =
                    self.update_heartbeat(pkt, my_tx_slot, extracted_deltas, adjusted_timestamp);
                if let Err(pkt) = tx_queue.push(upd_pkt) {
                    error!("Queue is full even though we just checked??");
                    self.time_manager.hbt_pkt = Some(pkt);
                }
            }
            if !tx_queue.is_empty() {
                let tx_result = node.transmit(tx_queue).await;
                tx_queue.clear();
                tx_result?;
            }
        } else {
            // debug!(" -- NOT MY SLOT ---   ");
            let conn = node
                .listen(rx_buffer, Some(core::time::Duration::from_millis(500)))
                .await;
            if let Ok(conn) = conn
                && let Ok((pkts, rx_pkt)) = node.receive(conn, rx_buffer).await
            {
                self.sync_epoch(&pkts, rx_pkt);
                received_packets = pkts;
            } else {
                // Check to see if we haven't heard from *leader* in some time
                if let Some(inst) = self.time_manager.last_hb_instant
                    && Instant::now() > inst + HB_TIMEOUT
                {
                    // then we go into listening mode
                    // self.time_manager.time_sync = None;
                    self.reset_sync();
                }
            }
        }
        #[cfg(feature = "debug")]
        if let Some(pin) = self.debug_pin.as_mut() {
            let _ = pin.set_low();
        }
        Ok(Some(received_packets))
    }
}

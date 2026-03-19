#[cfg(not(feature = "debug"))]
use core::marker::PhantomData;

use crate::node::{MHNode, PacketType};

#[cfg(not(feature = "in_std"))]
use defmt::{debug, error, info};
#[cfg(feature = "in_std")]
use log::{debug, error, info};

#[cfg(feature = "debug")]
use embedded_hal::digital::OutputPin;

use core::fmt;
use postcard::{from_bytes, to_slice};
use serde::{Deserialize, Serialize};

use super::{
    MHPacket,
    network_manager::{NetworkManager, NetworkManagerError},
};

use embassy_time::{Duration, Instant, Timer};
use heapless::Vec;

const GATEWAY_ID: u8 = 1;

pub trait RoutingPolicy<const SIZE: usize, const LEN: usize> {
    fn check_heartbeat(
        &mut self,
        manager: &mut NetworkManager<SIZE, LEN>,
    ) -> Result<Option<MHPacket<SIZE>>, NetworkManagerError>;
}

pub struct NodePolicy;
impl<const SIZE: usize, const LEN: usize> RoutingPolicy<SIZE, LEN> for NodePolicy {
    fn check_heartbeat(
        &mut self,
        _manager: &mut NetworkManager<SIZE, LEN>,
    ) -> Result<Option<MHPacket<SIZE>>, NetworkManagerError> {
        Ok(None)
    }
}

/// A gateway sends out periodic heartbeats
#[cfg(feature = "in_std")]
pub struct GatewayPolicy {
    pub last_heartbeat: Option<Instant>,
    pub timeout: u64,
}
#[cfg(feature = "in_std")]
impl GatewayPolicy {
    pub fn new(timeout: u64) -> Self {
        Self {
            last_heartbeat: None,
            timeout,
        }
    }
}

#[cfg(feature = "in_std")]
impl<const SIZE: usize, const LEN: usize> RoutingPolicy<SIZE, LEN> for GatewayPolicy {
    fn check_heartbeat(
        &mut self,
        manager: &mut NetworkManager<SIZE, LEN>,
    ) -> Result<Option<MHPacket<SIZE>>, NetworkManagerError> {
        let now = Instant::now();
        let should_send = match self.last_heartbeat {
            None => true,
            Some(last) => now.duration_since(last) >= Duration::from_secs(self.timeout),
        };
        if should_send {
            self.last_heartbeat = Some(now);
            let pkt = manager.add_heartbeat()?;
            Ok(Some(pkt))
        } else {
            Ok(None)
        }
    }
}

pub trait MacPolicy<Node, const SIZE: usize, const LEN: usize>
where
    Node: MHNode<SIZE, LEN>,
{
    fn run_mac(
        &mut self,
        node: &mut Node,
        tx_queue: &mut Vec<MHPacket<SIZE>, LEN>,
        rx_buffer: &mut Node::ReceiveBuffer,
    ) -> impl Future<Output = Result<Option<Vec<MHPacket<SIZE>, LEN>>, Node::Error>>;

    fn tx_heartbeat(&mut self, hbt: MHPacket<SIZE>);

    fn set_gw_hops(&mut self, gw_hops: u8);
}

pub struct RandomAccessMac<const SIZE: usize> {
    hbt_pkt: Option<MHPacket<SIZE>>,
}

impl<const SIZE: usize> RandomAccessMac<SIZE> {
    pub fn new() -> Self {
        Self { hbt_pkt: None }
    }
}

impl<Node, const SIZE: usize, const LEN: usize> MacPolicy<Node, SIZE, LEN> for RandomAccessMac<SIZE>
where
    Node: MHNode<SIZE, LEN>,
{
    fn set_gw_hops(&mut self, _gw_hops: u8) {}
    fn tx_heartbeat(&mut self, hbt: MHPacket<SIZE>) {
        self.hbt_pkt = Some(hbt);
    }

    async fn run_mac(
        &mut self,
        node: &mut Node,
        tx_queue: &mut Vec<MHPacket<SIZE>, LEN>,
        rx_buffer: &mut Node::ReceiveBuffer,
    ) -> Result<Option<Vec<MHPacket<SIZE>, LEN>>, Node::Error> {
        if !tx_queue.is_empty() || self.hbt_pkt.is_some() {
            if let Some(pkt) = self.hbt_pkt.take()
                && let Err(pkt) = tx_queue.push(pkt)
            {
                // If queue full
                self.hbt_pkt = Some(pkt)
            }
            node.transmit(tx_queue).await?;
            tx_queue.clear();
        }
        match node
            .listen(rx_buffer, Some(core::time::Duration::from_secs(1)))
            .await
        {
            Ok(conn) => match node.receive(conn, rx_buffer).await {
                Ok((pkts, rx_hw_timestamp)) => Ok(Some(pkts)),
                Err(e) => Err(e),
            },
            Err(e) => {
                // error!("Error in listening: {:?}", e);
                Ok(None)
            }
        }
    }
}

pub struct SlotMask {
    mask: u32,
}
impl Default for SlotMask {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for SlotMask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Taken Slots: [")?;
        let mut first = true;

        for i in 0..32 {
            if self.is_taken(i) {
                if !first {
                    write!(f, ", ")?;
                }
                write!(f, "{}", i)?;
                first = false;
            }
        }
        write!(f, "]")
    }
}

#[cfg(not(feature = "in_std"))]
impl defmt::Format for SlotMask {
    fn format(&self, fmt: defmt::Formatter) {
        defmt::write!(fmt, "Taken Slots: [");
        let mut first = true;

        for i in 0..32 {
            if self.is_taken(i) {
                if !first {
                    defmt::write!(fmt, ", ");
                }
                // We use {=u8} to tell defmt exactly what type it is sending over the wire
                defmt::write!(fmt, "{=u8}", i);
                first = false;
            }
        }
        defmt::write!(fmt, "]");
    }
}

impl SlotMask {
    pub const fn new() -> Self {
        Self { mask: 0 }
    }
    /// To set a slot inside mask
    pub fn claim(&mut self, slot: u8) {
        // shift 1 over to slot pos, and or with mask
        self.mask |= 1 << slot;
    }

    /// Check given slot is occupied
    pub fn is_taken(&self, slot: u8) -> bool {
        (self.mask & (1 << slot)) != 0
    }

    pub fn as_u32(&self) -> u32 {
        self.mask
    }

    /// Get the next available slot given another node's mask and yours. Uses the node_id to avoid conflicts in race conditions
    pub fn slot_assignment_strat(
        &self,
        max_slots: u8,
        another_mask: u32,
        node_id: u8,
    ) -> Option<u8> {
        let combined_mask = SlotMask {
            mask: self.mask | another_mask,
        };
        let start_offset = (node_id % max_slots) as u8;

        (0..max_slots).find_map(|i| {
            let slot = (start_offset + i) % max_slots;
            debug!("looking in slot {}", slot);
            if !combined_mask.is_taken(slot) {
                Some(slot)
            } else {
                None
            }
        })
    }
}

/// Used in Heartbeats to convey information about which slots are used, and the time
/// synchronization values needed
#[derive(Serialize, Deserialize)]
struct SlotAllocation {
    my_slot: u8,
    /// Bit mask for known slots, meaning only 32 nodes can be known at a time
    known_slots: u32,
    gps_time_ms: u64,
    /// A list of (node_id, T3 - T2 delta in ms) for PTP
    t3_deltas: Vec<(u8, i16), 5>,
}

#[cfg(feature = "debug")]
pub trait DebugPin: embedded_hal::digital::OutputPin {}
#[cfg(feature = "debug")]
impl<T: embedded_hal::digital::OutputPin> DebugPin for T {}

// When "debug" is OFF, this trait requires nothing, so any type (like `()`) can implement it.
#[cfg(not(feature = "debug"))]
pub trait DebugPin {}
#[cfg(not(feature = "debug"))]
impl<T> DebugPin for T {}

pub struct TdmaMac<P, const SIZE: usize> {
    slot_duration: Duration,
    slots_per_frame: u8,
    my_tx_slot: Option<u8>,
    time_sync: Option<(u64, Instant)>,
    known_slots_mask: SlotMask,
    hbt_pkt: Option<MHPacket<SIZE>>,
    /// Used for the slot allocation. You should convert the MAC address into a u32 with the
    /// biggest chance of two nodes not having the same u32 representation
    node_id: u8,
    /// Ratio to try and mitigate clock drift at nodes with no HSE
    skew_ratio: f32,
    skew_gw_diff: u64,
    skew_local_diff: u64,
    gw_hops: u8,
    /// A list of (node_id, T3 - T2 delta in ms) for PTP
    t3_deltas: Vec<(u8, i16), 5>,
    #[cfg(feature = "debug")]
    pub debug_pin: Option<P>,

    #[cfg(not(feature = "debug"))]
    _marker: PhantomData<P>,
}

impl<P, const SIZE: usize> TdmaMac<P, SIZE> {
    pub fn new(
        // FIXME: Remove from user, should be set by GW
        slot_duration: Duration,
        // FIXME: Remove from user, should be set by GW
        slots_per_frame: core::num::NonZeroU8,
        time_sync: Option<(u64, Instant)>,
        my_tx_slot: Option<u8>,
        #[cfg(feature = "debug")] debug_pin: Option<P>,
        node_id: u8,
    ) -> Self {
        Self {
            slot_duration,
            slots_per_frame: slots_per_frame.into(),
            my_tx_slot,
            node_id,
            time_sync,
            known_slots_mask: SlotMask::default(),
            hbt_pkt: None,
            skew_ratio: 1.0,
            skew_gw_diff: 1,
            skew_local_diff: 1,
            gw_hops: 255,
            t3_deltas: Vec::new(),
            #[cfg(feature = "debug")]
            debug_pin,
            #[cfg(not(feature = "debug"))]
            _marker: PhantomData,
        }
    }

    fn current_gps_time(&self, stamps: (u64, Instant)) -> u64 {
        self.gps_time_at(stamps, Instant::now())
    }

    fn gps_time_at(&self, (base_gps_ms, sync_instant): (u64, Instant), at_instant: Instant) -> u64 {
        // TODO: could add a multiplier here to fix drifting?
        let elapsed_ms = (at_instant - sync_instant).as_millis();
        let gw_elapsed_ms = (elapsed_ms as f32 * self.skew_ratio) as u64;
        // let gw_elapsed_ms = ((elapsed_ms as u128 * self.skew_gw_diff as u128)
        //     / self.skew_local_diff as u128) as u64;
        base_gps_ms + gw_elapsed_ms
    }

    fn calc_nextslot_time(&self, timestamps: (u64, Instant)) -> Instant {
        let current_gps_time = self.current_gps_time(timestamps);
        let slot_dur = self.slot_duration.as_millis();

        let elapsed_in_current_slot = current_gps_time % slot_dur;

        let next_slot_start_offset = slot_dur - elapsed_in_current_slot;
        // let node_offset = ((next_slot_start_offset as u128 * self.skew_local_diff as u128)
        //     / self.skew_gw_diff as u128) as u64;
        let node_offset = (next_slot_start_offset as f32 / self.skew_ratio) as u64;

        // Wake up just a bit before the slot starts to ensure listening at correct time
        let guard_band = 5;
        Instant::now() + Duration::from_millis(node_offset.saturating_sub(guard_band))
    }

    pub fn current_slot(&self, current_time_ms: u64) -> u8 {
        let frame_duration_ms = (self.slot_duration.as_millis()) * (self.slots_per_frame as u64);
        let time_in_frame = current_time_ms % frame_duration_ms;

        // Add half of slot duration to achieve 'round to nearest' int division
        let slot_dur = self.slot_duration.as_millis();
        ((time_in_frame + (slot_dur / 2)) / slot_dur) as u8
    }

    /// Given a heartbeat packet from a nearer-gw node, this calculates the new timestamp and the
    /// new skew ratio for the node to be properly synchronized.
    fn update_skew_and_stamp(
        &self,
        hb: &SlotAllocation,
        rx_hw_timestamp: Instant,
    ) -> (f32, Option<(u64, Instant)>) {
        let (old_gps, last_stamp) = match self.time_sync {
            Some(stamps) => stamps,
            None => {
                // Short circuit from function if not set
                info!("TDMA: Initial epoch set");
                return (1_f32, Some((hb.gps_time_ms, Instant::now())));
            }
        };
        // Calculate skews
        // let my_stamp = self.current_gps_time((old_gps, last_stamp));
        let my_diff = (rx_hw_timestamp - last_stamp).as_millis();
        let my_stamp = my_diff + old_gps;

        // Check if a t3 delta is availale for us
        let (delay, offset) =
            if let Some((_, delta_up)) = hb.t3_deltas.iter().find(|t| t.0 == self.node_id) {
                // delta is our T3 - T2
                let delta_down = my_stamp as i64 - hb.gps_time_ms as i64;
                info!("Delta up:\t{}\t\t\t Delta down:\t{}", delta_up, delta_down);
                let clock_offset = (delta_down - *delta_up as i64) / 2;
                let nw_delay = (delta_down + *delta_up as i64) / 2;
                info!(
                    "clock offset:\t{}\t\tnetwork delay:\t{}",
                    clock_offset, nw_delay
                );
                (nw_delay, clock_offset)
            } else {
                (0, 0)
            };

        // Use the network delay to make up for transmission time, etc.
        let current_true_time = hb.gps_time_ms as i64 + delay;
        let gw_diff = current_true_time - old_gps as i64;
        let skew = (gw_diff as f32) / (my_diff as f32);

        // Debug info:
        if my_stamp != hb.gps_time_ms {
            let skewed_stamp = old_gps + (gw_diff as u128) as u64;
            let delay: i64 = my_stamp as i64 - hb.gps_time_ms as i64;
            let skewed_delay: i64 = skewed_stamp as i64 - hb.gps_time_ms as i64;
            info!(
                "Mesured clock drift: {} ms, skewed drift: {}, ratio: {}\t self ratio: {}",
                delay, skewed_delay, skew, self.skew_ratio
            );
        } else {
            info!("Perfectly synced?!");
        }

        // let skew_ratio = (skew * 0.2) + (self.skew_ratio * 0.8);
        let kp = 0.4;
        let err = skew - self.skew_ratio;
        let skew_ratio = self.skew_ratio + kp * err;

        // self.skew_gw_diff = gw_diff as u64;
        // self.skew_local_diff = my_diff;

        let time_sync = Some((current_true_time as u64, rx_hw_timestamp));
        (skew_ratio, time_sync)
    }

    fn sync_epoch(&mut self, pkts: &[MHPacket<SIZE>], rx_hw_timestamp: Instant) {
        for pkt in pkts {
            if pkt.packet_type == PacketType::HeartBeat
                && let Ok(alloc) = from_bytes::<SlotAllocation>(&pkt.payload)
            {
                // Resync node to heartbeat's announced slot, if hb came closer to gw than me
                if self.node_id != GATEWAY_ID && pkt.hop_to_gw < self.gw_hops {
                    // Calculate updated skew and timestamps
                    let (skew_ratio, time_sync) =
                        self.update_skew_and_stamp(&alloc, rx_hw_timestamp);
                    self.skew_ratio = skew_ratio;
                    self.time_sync = time_sync;
                } else {
                    // if we are GW, then we want to update out t3 deltas on this node
                    let t3 = if let Some(stamps) = self.time_sync {
                        // Cast to i64 to not panic at my_time < alloc
                        (self.gps_time_at(stamps, rx_hw_timestamp) as i64
                            - alloc.gps_time_ms as i64) as i16
                    } else {
                        0
                    };
                    match self.t3_deltas.iter().position(|t| t.0 == pkt.source_id) {
                        Some(idx) => self.t3_deltas[idx] = (pkt.source_id, t3),
                        None => {
                            if self.t3_deltas.push((pkt.source_id, t3)).is_err() {
                                error!("T3 deltas is full!")
                            }
                        }
                    }
                }
                self.known_slots_mask.claim(alloc.my_slot);
                info!(
                    "Other node claimed slot at {}. {:?}",
                    alloc.my_slot, self.known_slots_mask
                );

                // Only allocate a new slot if we don't have one
                if self.my_tx_slot.is_none() {
                    match self.known_slots_mask.slot_assignment_strat(
                        self.slots_per_frame,
                        alloc.known_slots,
                        self.node_id,
                    ) {
                        Some(free_slot) => {
                            self.my_tx_slot = Some(free_slot);
                            self.known_slots_mask.claim(free_slot);
                        }
                        None => error!("Network is full!"),
                    }
                }
                break;
            }
        }
    }

    fn update_heartbeat(
        &self,
        mut hbt: MHPacket<SIZE>,
        my_tx_slot: u8,
        t3_deltas: Vec<(u8, i16), 5>,
    ) -> MHPacket<SIZE> {
        let tx_timestamp = match self.time_sync {
            Some(stamps) => self.current_gps_time(stamps),
            None => {
                error!("In update heartbeat before we've heard a heartbeat??");
                0
            }
        };
        let allocation = SlotAllocation {
            my_slot: my_tx_slot,
            known_slots: self.known_slots_mask.as_u32(),
            gps_time_ms: tx_timestamp,
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

impl<Node, const SIZE: usize, const LEN: usize, P> MacPolicy<Node, SIZE, LEN> for TdmaMac<P, SIZE>
where
    Node: MHNode<SIZE, LEN>,
    P: DebugPin,
{
    fn set_gw_hops(&mut self, gw_hops: u8) {
        self.gw_hops = gw_hops
    }
    /// This only sends if you have been given a slot. GW should choose it's own slot, such that it
    /// can start this.
    fn tx_heartbeat(&mut self, hbt: MHPacket<SIZE>) {
        self.hbt_pkt = Some(hbt)
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
        let Some(timestamps) = self.time_sync else {
            info!("TDMA: Waiting for first packet to sync");
            let conn = node
                .listen(rx_buffer, Some(core::time::Duration::from_secs(10)))
                .await;
            match conn {
                Ok(conn) => {
                    let (rec, rx_hw_timestamp) = node.receive(conn, rx_buffer).await?;
                    // Check for being heartbeat
                    self.sync_epoch(&rec, rx_hw_timestamp);
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

        if let Some(my_tx_slot) = self.my_tx_slot
            && slot == my_tx_slot
        {
            debug!(" !!!  MY SLOT !!! ");
            // NOTE: Introduce a 50ms delay here, to ensure nodes are listening in your slot
            Timer::after(Duration::from_millis(100)).await;
            if !tx_queue.is_empty() || self.hbt_pkt.is_some() {
                // If should send hbt, update it's timestamp
                if !tx_queue.is_full()
                    && let Some(pkt) = self.hbt_pkt.take()
                {
                    // We update these deltas every time a heartbeat is sent
                    let extracted_deltas = core::mem::take(&mut self.t3_deltas);
                    let upd_pkt = self.update_heartbeat(pkt, my_tx_slot, extracted_deltas);
                    if let Err(pkt) = tx_queue.push(upd_pkt) {
                        // If queue full(???), then try and send it next time
                        error!("Queue is full even though we just checked??");
                        self.hbt_pkt = Some(pkt)
                    }
                }
                node.transmit(tx_queue).await?;
                tx_queue.clear();
            }
        } else {
            debug!(" -- NOT MY SLOT ---   ");
            let conn = node
                .listen(rx_buffer, Some(core::time::Duration::from_millis(500)))
                .await;
            if let Ok(conn) = conn
                && let Ok((pkts, rx_hw_timestamp)) = node.receive(conn, rx_buffer).await
            {
                self.sync_epoch(&pkts, rx_hw_timestamp);
                received_packets = pkts;
            }
        }
        #[cfg(feature = "debug")]
        if let Some(pin) = self.debug_pin.as_mut() {
            let _ = pin.set_low();
        }
        Ok(Some(received_packets))
    }
}

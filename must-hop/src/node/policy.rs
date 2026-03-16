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

const GATEWAY_ID: u32 = 1;

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
                Ok(pkts) => Ok(Some(pkts)),
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
        node_id: u32,
    ) -> Option<u8> {
        let combined_mask = SlotMask {
            mask: self.mask | another_mask,
        };
        let start_offset = (node_id % max_slots as u32) as u8;

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

#[derive(Serialize, Deserialize)]
struct SlotAllocation {
    my_slot: u8,
    /// Bit mask for known slots, meaning only 32 nodes can be known at a time
    known_slots: u32,
    gps_time_ms: u64,
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
    node_id: u32,
    /// Ratio to try and mitigate clock drift at nodes with no HSE
    // skew_ratio: f64,
    skew_gw_diff: u64,
    skew_local_diff: u64,
    gw_hops: u8,
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
        node_id: u32,
    ) -> Self {
        Self {
            slot_duration,
            slots_per_frame: slots_per_frame.into(),
            my_tx_slot,
            node_id,
            time_sync,
            known_slots_mask: SlotMask::default(),
            hbt_pkt: None,
            // skew_ratio: 1.0,
            skew_gw_diff: 1,
            skew_local_diff: 1,
            gw_hops: 255,
            #[cfg(feature = "debug")]
            debug_pin,
            #[cfg(not(feature = "debug"))]
            _marker: PhantomData,
        }
    }

    fn current_gps_time(&self, (base_gps_ms, sync_instant): (u64, Instant)) -> u64 {
        // TODO: could add a multiplier here to fix drifting?
        let elapsed_ms = (Instant::now() - sync_instant).as_millis();
        // let gw_elapsed = (elapsed_ms as f64 * self.skew_ratio) as u64;
        let gw_elapsed_ms = ((elapsed_ms as u128 * self.skew_gw_diff as u128)
            / self.skew_local_diff as u128) as u64;
        base_gps_ms + gw_elapsed_ms
    }

    fn calc_nextslot_time(&self, timestamps: (u64, Instant)) -> Instant {
        let current_gps_time = self.current_gps_time(timestamps);
        let slot_dur = self.slot_duration.as_millis();

        let elapsed_in_current_slot = current_gps_time % slot_dur;

        let next_slot_start_offset = slot_dur - elapsed_in_current_slot;
        // A 5ms guard band is used to perhaps fix conversion errors
        let node_offset = ((next_slot_start_offset as u128 * self.skew_local_diff as u128)
            / self.skew_gw_diff as u128) as u64;
        Instant::now() + Duration::from_millis(node_offset)
    }

    pub fn current_slot(&self, current_time_ms: u64) -> u8 {
        let frame_duration_ms = (self.slot_duration.as_millis()) * (self.slots_per_frame as u64);

        let time_in_frame = current_time_ms % frame_duration_ms;
        (time_in_frame / (self.slot_duration.as_millis())) as u8
    }

    fn sync_epoch(&mut self, pkts: &[MHPacket<SIZE>]) {
        for pkt in pkts {
            if pkt.packet_type == PacketType::HeartBeat
                && let Ok(alloc) = from_bytes::<SlotAllocation>(&pkt.payload)
            {
                // Resync node to heartbeat's announced slot, if you are not gateway
                if self.node_id != GATEWAY_ID && pkt.hop_to_gw < self.gw_hops {
                    // TODO: There must be a latency here, which should be adjusted for
                    let now = Instant::now();

                    match self.time_sync {
                        None => {
                            info!("TDMA: Initial epoch set");
                            self.time_sync = Some((alloc.gps_time_ms, now));
                        }
                        Some((old_gps, last_stamp)) => {
                            let my_diff = now.as_millis() - last_stamp.as_millis();
                            let my_stamp = old_gps + my_diff;
                            let gw_diff = alloc.gps_time_ms - old_gps;
                            let skew = ((alloc.gps_time_ms - old_gps) as f32) / (my_diff as f32);
                            let skewed_stamp = old_gps
                                + ((my_diff as u128 * gw_diff as u128) / my_diff as u128) as u64;
                            // self.skew_ratio = skew;
                            self.skew_gw_diff = gw_diff;
                            self.skew_local_diff = my_diff;
                            if my_stamp != alloc.gps_time_ms {
                                let delay: i64 = my_stamp as i64 - alloc.gps_time_ms as i64;
                                let skewed_delay: i64 =
                                    skewed_stamp as i64 - alloc.gps_time_ms as i64;
                                info!(
                                    "Mesured clock drift: {} ms, skewed drift: {}, ratio: {}",
                                    delay, skewed_delay, skew
                                );
                            } else {
                                info!("Perfectly synced!");
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

    fn update_heartbeat<Node, const LEN: usize>(
        &self,
        mut hbt: MHPacket<SIZE>,
        my_tx_slot: u8,
        node: &mut Node,
        len: usize,
    ) -> MHPacket<SIZE>
    where
        Node: MHNode<SIZE, LEN>,
    {
        let toa = node.calc_tx_delay(len);
        let tx_timestamp = match self.time_sync {
            Some(stamps) => {
                // old_gps + (Instant::now().as_millis() - last_stamp.as_millis())
                self.current_gps_time(stamps) + toa.as_millis() as u64
            }
            None => 0,
        };
        let allocation = SlotAllocation {
            my_slot: my_tx_slot,
            known_slots: self.known_slots_mask.as_u32(),
            gps_time_ms: tx_timestamp,
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
        let Some(timestamps) = self.time_sync else {
            info!("TDMA: Waiting for first packet to sync");
            let conn = node
                .listen(rx_buffer, Some(core::time::Duration::from_secs(10)))
                .await;
            match conn {
                Ok(conn) => {
                    received_packets = node.receive(conn, rx_buffer).await?;
                    // Check for being heartbeat
                    self.sync_epoch(&received_packets);
                    return Ok(Some(received_packets));
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
        // TODO: Should move the sleep up to here instead, but this is how it is right now
        #[cfg(feature = "debug")]
        if let Some(pin) = self.debug_pin.as_mut() {
            let _ = pin.set_high();
        }

        if let Some(my_tx_slot) = self.my_tx_slot
            && slot == my_tx_slot
        {
            debug!(" !!!  MY SLOT !!! ");
            if !tx_queue.is_empty() || self.hbt_pkt.is_some() {
                // If should send hbt, update it's timestamp
                if self.hbt_pkt.is_some() && !tx_queue.is_full() {
                    if let Some(pkt) = self.hbt_pkt.take()
                        && let Err(pkt) = tx_queue.push(self.update_heartbeat(
                            pkt,
                            my_tx_slot,
                            node,
                            tx_queue.len() as usize,
                        ))
                    {
                        // If queue full, then try and send it next time
                        self.hbt_pkt = Some(pkt)
                    }
                }
                node.transmit(tx_queue).await?;
                tx_queue.clear();
            }
        } else {
            debug!(" -- NOT MY SLOT ---   ");
            let conn = node
                .listen(rx_buffer, Some(core::time::Duration::from_millis(200)))
                .await;
            if let Ok(conn) = conn {
                match node.receive(conn, rx_buffer).await {
                    Ok(pkts) => received_packets = pkts,
                    Err(_e) => (),
                }
                self.sync_epoch(&received_packets);
            }
            // Redeclare timestamps, which might've been updated in `self.sync_epoch` just above
            let timestamps = match self.time_sync {
                // Uses match to avoid an unwrap here
                Some(timestamps) => timestamps,
                None => timestamps,
            };
        }
        #[cfg(feature = "debug")]
        if let Some(pin) = self.debug_pin.as_mut() {
            let _ = pin.set_low();
        }
        Ok(Some(received_packets))
    }
}

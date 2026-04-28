use crate::node::{MHNode, PacketType, RxPacket};

#[cfg(not(feature = "in_std"))]
use defmt::{debug, error, info};
#[cfg(feature = "in_std")]
use log::{debug, error, info};

#[cfg(feature = "debug")]
use embedded_hal::digital::OutputPin;

use core::{fmt, marker::PhantomData, num::NonZeroU8};
use postcard::{from_bytes, ser_flavors::Size, serialize_with_flavor, to_slice};
use serde::{Deserialize, Serialize};

use crate::node::{
    MHPacket,
    policy::{GATEWAY_ID, MacPolicy},
};

use super::controller::Controller;

use embassy_time::{Duration, Instant, Timer};
use heapless::Vec;

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
        let start_offset = node_id % max_slots;

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
pub(crate) struct SlotAllocation {
    my_slot: u8,
    /// Bit mask for known slots, meaning only 32 nodes can be known at a time
    known_slots: u32,
    pub(crate) gps_time_us: u64,
    /// A list of (node_id, T3 - T2 delta in ms) for PTP
    pub(crate) t3_deltas: Vec<(u8, i32), 5>,
}
/// Onl used for tests
impl SlotAllocation {
    pub(super) fn new() -> Self {
        Self {
            my_slot: 1,
            known_slots: 0,
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

pub struct Builder;
pub struct Runner;

/// A TDMA MAC policy, which synchronizes nodes across the network to only listen when known slots
/// could be transmitting, saving power.
pub struct TdmaMac<State, P, const SIZE: usize> {
    _state: PhantomData<State>,
    slot_duration: Duration,
    slots_per_frame: u8,
    my_tx_slot: Option<u8>,
    /// a tuple of timestamp in micros, and instant when that timestamp was set
    time_sync: Option<(u64, Instant)>,
    /// A mask to know what other node's one know
    known_slots_mask: SlotMask,
    hbt_pkt: Option<MHPacket<SIZE>>,
    /// Used for the slot allocation. You should convert the MAC address into a u32 with the
    /// biggest chance of two nodes not having the same u32 representation
    node_id: u8,
    gw_hops: u8,
    /// A list of (node_id, T3 - T2 delta in ms) for PTP
    t3_deltas: Vec<(u8, i32), 5>,
    controller: Controller,
    #[cfg(feature = "debug")]
    pub debug_pin: Option<P>,
    #[cfg(not(feature = "debug"))]
    _marker: PhantomData<P>,
}

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
            slot_duration,
            slots_per_frame: slots_per_frame.into(),
            my_tx_slot: None,
            node_id: 0,
            time_sync,
            known_slots_mask: SlotMask::default(),
            hbt_pkt: None,
            gw_hops: 255,
            t3_deltas: Vec::new(),
            controller,

            #[cfg(feature = "debug")]
            debug_pin: None,
            #[cfg(not(feature = "debug"))]
            _marker: PhantomData,
        }
    }

    pub fn set_controller(self, v_s: i64, kp: i64, ki: i64) -> Self {
        let controller = Controller::new(v_s, kp, ki);
        Self { controller, ..self }
    }

    pub fn set_time_sync(self, time_sync: (u64, Instant)) -> Self {
        Self {
            time_sync: Some(time_sync),
            ..self
        }
    }

    pub fn set_node_id(self, node_id: u8) -> Self {
        Self { node_id, ..self }
    }

    pub fn set_tx_slot(self, tx_slot: u8) -> Self {
        Self {
            my_tx_slot: Some(tx_slot),
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
        let TdmaMac {
            slot_duration,
            slots_per_frame,
            my_tx_slot,
            time_sync,
            known_slots_mask,
            hbt_pkt,
            node_id,
            gw_hops,
            t3_deltas,
            controller,
            #[cfg(feature = "debug")]
            debug_pin,
            ..
        } = self;

        TdmaMac::<Runner, P, SIZE> {
            _state: PhantomData,
            slot_duration,
            slots_per_frame,
            my_tx_slot,
            time_sync,
            known_slots_mask,
            hbt_pkt,
            node_id,
            gw_hops,
            t3_deltas,
            controller,
            #[cfg(feature = "debug")]
            debug_pin,
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

impl<P, const SIZE: usize> TdmaMac<Runner, P, SIZE> {
    fn current_gps_time(&self, stamps: (u64, Instant)) -> u64 {
        self.gps_time_at(stamps, Instant::now())
    }

    fn gps_time_at(&self, (base_gps_us, sync_instant): (u64, Instant), at_instant: Instant) -> u64 {
        let elapsed_us = (at_instant - sync_instant).as_micros();
        let gw_elapsed_us = elapsed_us + self.controller.calc_drift_duration(elapsed_us);
        // let gw_elapsed_ms = ((elapsed_ms as u128 * self.skew_gw_diff as u128)
        //     / self.skew_local_diff as u128) as u64;
        base_gps_us + gw_elapsed_us
    }

    fn calc_nextslot_time(&self, timestamps: (u64, Instant)) -> Instant {
        let current_gps_time = self.current_gps_time(timestamps);
        let slot_dur = self.slot_duration.as_micros();

        let elapsed_in_current_slot = current_gps_time % slot_dur;

        let next_slot_start_offset = slot_dur - elapsed_in_current_slot;
        // let node_offset = ((next_slot_start_offset as u128 * self.skew_local_diff as u128)
        //     / self.skew_gw_diff as u128) as u64;
        let node_offset =
            next_slot_start_offset - self.controller.calc_drift_duration(next_slot_start_offset);

        // Wake up just a bit before the slot starts to ensure listening at correct time
        // FIXME: If everyone does this, then everyone just wakes up 5ms before
        // let guard_band = 5000;
        Instant::now() + Duration::from_micros(node_offset /*.saturating_sub(guard_band) */)
    }

    pub fn current_slot(&self, current_time_us: u64) -> u8 {
        let frame_duration_us = (self.slot_duration.as_micros()) * (self.slots_per_frame as u64);
        let time_in_frame = current_time_us % frame_duration_us;

        // Add half of slot duration to achieve 'round to nearest' int division
        let slot_dur = self.slot_duration.as_micros();
        ((time_in_frame + (slot_dur / 2)) / slot_dur) as u8
    }

    // Using
    // fn calc_excess_delay(&self) {
    //
    // }

    /// Use ToA calculation together with other delay variables to approximate what our local
    /// instant was, when the heartbeat packet with the timestamp was created.
    fn approximate_transmit_instant(&self, rx_pkt: &RxPacket) -> Instant {
        // let without_toa = match rx_pkt.preamble_instant {
        //     Some(ins) => {
        //         info!("preamble instant was Some!");
        //         ins.checked_sub(Duration::from_micros(
        //             rx_pkt.estimated_toa.0 as u64
        //                 + self
        //                     .controller
        //                     .calc_drift_duration(rx_pkt.estimated_toa.0 as u64),
        //         ))
        //         .unwrap_or(ins)
        //     }
        //     // If no preamble instant, we approximate it
        //     None => {
        //         info!("preamble instant was None!");
        //         rx_pkt
        //             .rx_done_instant
        //             .checked_sub(Duration::from_micros(
        //                 rx_pkt.estimated_toa.1 as u64
        //                     + self
        //                         .controller
        //                         .calc_drift_duration(rx_pkt.estimated_toa.1 as u64),
        //             ))
        //             .unwrap_or(rx_pkt.rx_done_instant)
        //     }
        // };
        // TODO: Byte slicing and SPI1 delay
        // let tau_gw = Duration::from_millis(2);

        // TODO: Own SPI2 delay approximate

        // without_toa //- tau_gw
        rx_pkt.rx_done_instant
    }

    fn sync_epoch(&mut self, pkts: &[MHPacket<SIZE>], rx_pkt: RxPacket) {
        for pkt in pkts {
            if pkt.packet_type == PacketType::HeartBeat
                && let Ok(alloc) = from_bytes::<SlotAllocation>(&pkt.payload)
            {
                // This is meant to approximate the local ticks when the hb packet was sent
                let sending_instant = self.approximate_transmit_instant(&rx_pkt);
                // Resync node to heartbeat's announced slot, if hb came closer to gw than me
                if self.node_id != GATEWAY_ID && pkt.hop_to_gw < self.gw_hops {
                    // Controller updates internal drift, and returns adjusted stamps
                    self.time_sync = self.controller.run_transferfunction(
                        &alloc,
                        rx_pkt,
                        sending_instant,
                        self.time_sync,
                        self.my_tx_slot.unwrap_or(self.node_id),
                    );
                } else {
                    // if we are GW, then we want to update out t3 deltas on this node
                    let t3 = if let Some(stamps) = self.time_sync {
                        // Cast to i64 to not panic at my_time < alloc
                        (self.gps_time_at(stamps, sending_instant) as i64
                            - alloc.gps_time_us as i64) as i32
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

    fn calc_adjust_timestamp(&self, toa: u64) -> u64 {
        let tx_stamp = match self.time_sync {
            Some(stamps) => self.current_gps_time(stamps),
            None => {
                error!("There was no stamps in heartbeat updating??");
                0
            }
        };
        // TODO: Also use the clock drift calc here
        info!("[TAU_SLICE] | {} |", Instant::now().as_micros());
        let measured_spi_delay = 0;
        tx_stamp + toa + measured_spi_delay
    }

    fn approximate_packet_size<const LEN: usize>(
        &self,
        my_tx_slot: u8,
        t3_deltas: Vec<(u8, i32), 5>,
        tx_queue: &Vec<MHPacket<SIZE>, LEN>,
    ) -> usize {
        let tx_timestamp = match self.time_sync {
            Some(stamps) => self.current_gps_time(stamps),
            None => {
                error!("In update heartbeat before we've heart a heartbeat??");
                0
            }
        };

        let dummy_allocation = SlotAllocation {
            my_slot: my_tx_slot,
            known_slots: self.known_slots_mask.as_u32(),
            gps_time_us: tx_timestamp,
            t3_deltas: t3_deltas.clone(),
        };

        let exact_payload_size = serialize_with_flavor(&dummy_allocation, Size::default())
            .expect("failed to size payload");
        let exact_packet_size =
            serialize_with_flavor(&tx_queue, Size::default()).expect("Failed to size tx queue");

        let size = exact_packet_size + exact_payload_size + 7;
        info!("SIZE EXPECTED: {}", size);
        size
    }

    fn update_heartbeat(
        &self,
        mut hbt: MHPacket<SIZE>,
        my_tx_slot: u8,
        t3_deltas: Vec<(u8, i32), 5>,
        adjusted_timestamp: u64,
    ) -> MHPacket<SIZE> {
        let allocation = SlotAllocation {
            my_slot: my_tx_slot,
            known_slots: self.known_slots_mask.as_u32(),
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
                    let adjusted_timestamp = self.calc_adjust_timestamp(node.calc_tx_delay(
                        self.approximate_packet_size(
                            my_tx_slot,
                            // TOOD: Remove clone and just use the size
                            extracted_deltas.clone(),
                            tx_queue,
                        ),
                    ));
                    let upd_pkt = self.update_heartbeat(
                        pkt,
                        my_tx_slot,
                        extracted_deltas,
                        adjusted_timestamp,
                    );
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
                && let Ok((pkts, rx_pkt)) = node.receive(conn, rx_buffer).await
            {
                self.sync_epoch(&pkts, rx_pkt);
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

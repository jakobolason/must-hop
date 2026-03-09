use crate::node::{MHNode, PacketType};

#[cfg(not(feature = "in_std"))]
use defmt::{debug, error, info};
#[cfg(feature = "in_std")]
use log::{debug, error, info};
use postcard::{from_bytes, to_slice};
use serde::{Deserialize, Serialize};

use super::{
    MHPacket,
    network_manager::{NetworkManager, NetworkManagerError},
};

use embassy_time::{Duration, Instant, Timer};
use heapless::Vec;

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
}

pub struct RandomAccessMac<const SIZE: usize> {
    hbt_pkt: Option<MHPacket<SIZE>>,
}

impl<Node, const SIZE: usize, const LEN: usize> MacPolicy<Node, SIZE, LEN> for RandomAccessMac<SIZE>
where
    Node: MHNode<SIZE, LEN>,
{
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
        match node.listen(rx_buffer, true).await {
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

        (0..max_slots).find(|&i| {
            let slot = (start_offset + i) % max_slots;
            !combined_mask.is_taken(slot)
        })
    }
}

#[derive(Serialize, Deserialize)]
struct SlotAllocation {
    my_slot: u8,
    /// Bit mask for known slots, meaning only 32 nodes can be known at a time
    known_slots: u32,
}

pub struct TdmaMac<const SIZE: usize> {
    // FIXME: Remove from user, should be set by GW
    slot_duration: Duration,
    // FIXME: Remove from user, should be set by GW
    slots_per_frame: u8,
    my_tx_slot: Option<u8>,
    epoch: Option<Instant>,
    known_slots_mask: SlotMask,
    hbt_pkt: Option<MHPacket<SIZE>>,
    /// Used for the slot allocation. You should convert the MAC address into a u32 with the
    /// biggest chance of two nodes not having the same u32 representation
    node_id: u32,
}

impl<const SIZE: usize> TdmaMac<SIZE> {
    pub fn new(
        slot_duration: Duration,
        slots_per_frame: u8,
        epoch: Option<Instant>,
        node_id: u32,
    ) -> Self {
        Self {
            slot_duration,
            slots_per_frame,
            epoch,
            node_id,
            known_slots_mask: SlotMask::default(),
            hbt_pkt: None,
            my_tx_slot: None,
        }
    }

    pub fn current_slot(&self, now: Instant, epoch: Instant) -> u8 {
        let elapsed_ms = (now - epoch).as_millis();
        let frame_duration_ms = (self.slot_duration.as_millis()) * (self.slots_per_frame as u64);

        let time_in_frame = elapsed_ms % frame_duration_ms;
        (time_in_frame / (self.slot_duration.as_millis())) as u8
    }

    fn sync_epoch(&mut self, pkts: &[MHPacket<SIZE>]) {
        for pkt in pkts {
            if pkt.packet_type == PacketType::HeartBeat
                && let Ok(alloc) = from_bytes::<SlotAllocation>(&pkt.payload)
            {
                // Resync node to heartbeat's announced slot
                // TODO: There must be a latency here, which should be adjusted for
                let elapsed_in_frame =
                    Duration::from_millis((alloc.my_slot as u64) * self.slot_duration.as_millis());
                self.epoch = Some(Instant::now() - elapsed_in_frame);
                // TODO: alter known slots
                self.known_slots_mask.claim(alloc.my_slot);

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
}

impl<Node, const SIZE: usize, const LEN: usize> MacPolicy<Node, SIZE, LEN> for TdmaMac<SIZE>
where
    Node: MHNode<SIZE, LEN>,
{
    /// This only sends if you have been given a slot. GW should choose it's own slot, such that it
    /// can start this.
    fn tx_heartbeat(&mut self, mut hbt: MHPacket<SIZE>) {
        let my_tx_slot = match self.my_tx_slot {
            Some(slot) => slot,
            // Do not send any heartbeat if you haven't gotten a slot yet.
            None => return,
        };
        let allocation = SlotAllocation {
            my_slot: my_tx_slot,
            known_slots: self.known_slots_mask.as_u32(),
        };
        let mut buf = [0u8; SIZE];
        if let Ok(serialized_slice) = to_slice(&allocation, &mut buf) {
            hbt.payload.clear();
            let _ = hbt.payload.extend_from_slice(serialized_slice);
        }
        self.hbt_pkt = Some(hbt)
    }

    async fn run_mac(
        &mut self,
        node: &mut Node,
        tx_queue: &mut Vec<MHPacket<SIZE>, LEN>,
        rx_buffer: &mut Node::ReceiveBuffer,
    ) -> Result<Option<Vec<MHPacket<SIZE>, LEN>>, Node::Error> {
        let now = Instant::now();
        let mut received_packets = Vec::new();

        if self.epoch.is_none() {
            info!("TDMA: Waiting for first packet to sync");
            let conn = node.listen(rx_buffer, true).await;
            if let Ok(conn) = conn {
                received_packets = node.receive(conn, rx_buffer).await?;
                // Check for being heartbeat
                self.sync_epoch(&received_packets);
                return Ok(Some(received_packets));
            } else {
                info!("Error in getting conn");
                return Ok(None);
            }
        }
        let epoch = self
            .epoch
            .expect("Just checked for none, which should've returned out of function");

        // Calculate when the next slot starts
        let slot = self.current_slot(now, epoch);
        let elapsed_ms = (now - epoch).as_millis();
        let next_slot_start_ms = elapsed_ms + self.slot_duration.as_millis()
            - (elapsed_ms % self.slot_duration.as_millis());
        let next_slot_time = epoch + Duration::from_millis(next_slot_start_ms);

        debug!("current slot: {}", slot);

        if slot == self.my_tx_slot {
            debug!(" !!!  MY SLOT !!! {}", next_slot_time);
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
            Timer::at(next_slot_time).await;
        } else {
            debug!(" -- NOT MY SLOT ---   {}", next_slot_time);
            let conn = node.listen(rx_buffer, true).await;
            if let Ok(conn) = conn {
                received_packets = node.receive(conn, rx_buffer).await?;
                self.sync_epoch(&received_packets);
            } else {
                info!("Error in getting conn");
            }
            Timer::at(next_slot_time).await;
        }
        Ok(Some(received_packets))
    }
}

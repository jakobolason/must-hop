use crate::node::{MHNode, PacketType};

#[cfg(not(feature = "in_std"))]
use defmt::{debug, error, info};
#[cfg(feature = "in_std")]
use log::{debug, error, info};
use postcard::to_slice;
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

#[derive(Serialize, Deserialize)]
struct SlotAllocation {
    my_slot: u8,
    /// Bit mask for known slots, meaning only 32 nodes can be known at a time
    known_slots: u32,
}

pub struct TdmaMac<const SIZE: usize> {
    // FIXME: Remove from user, should be set by GW
    pub slot_duration: Duration,
    // FIXME: Remove from user, should be set by GW
    pub slots_per_frame: u8,
    pub my_tx_slot: u8,
    pub epoch: Option<Instant>,
    pub known_slots_mask: u32,
    hbt_pkt: Option<MHPacket<SIZE>>,
}

impl<const SIZE: usize> TdmaMac<SIZE> {
    pub fn new(slot_duration: Duration, slots_per_frame: u8, epoch: Option<Instant>) -> Self {
        Self {
            slot_duration,
            slots_per_frame,
            my_tx_slot: 0,
            epoch,
            known_slots_mask: 0,
            hbt_pkt: None,
        }
    }

    pub fn current_slot(&self, now: Instant, epoch: Instant) -> u8 {
        let elapsed_ms = (now - epoch).as_millis();
        let frame_duration_ms = (self.slot_duration.as_millis()) * (self.slots_per_frame as u64);

        let time_in_frame = elapsed_ms % frame_duration_ms;
        (time_in_frame / (self.slot_duration.as_millis())) as u8
    }
}

impl<Node, const SIZE: usize, const LEN: usize> MacPolicy<Node, SIZE, LEN> for TdmaMac<SIZE>
where
    Node: MHNode<SIZE, LEN>,
{
    fn tx_heartbeat(&mut self, mut hbt: MHPacket<SIZE>) {
        let allocation = SlotAllocation {
            my_slot: self.my_tx_slot,
            known_slots: self.known_slots_mask,
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
                if !received_packets.is_empty() {
                    for pkt in &received_packets {
                        if pkt.packet_type == PacketType::HeartBeat {
                            self.epoch = Some(Instant::now());
                        }
                    }
                    return Ok(Some(received_packets));
                }
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
            } else {
                info!("Error in getting conn");
            }
            Timer::at(next_slot_time).await;
        }
        Ok(Some(received_packets))
    }
}

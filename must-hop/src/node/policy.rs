use crate::node::{MHNode, PacketType};

#[cfg(not(feature = "in_std"))]
use defmt::{error, info};
#[cfg(feature = "in_std")]
use log::{error, info};

use super::{
    MHPacket,
    network_manager::{NetworkManager, NetworkManagerError},
};
use embassy_time::{Duration, Instant, Timer};
use heapless::Vec;

pub trait RoutingPolicy<const SIZE: usize, const LEN: usize> {
    /// Takes received packets and decides what to send on (TX) and what to keep (RX)
    fn process_packets(
        manager: &mut NetworkManager<SIZE, LEN>,
        pkts: Vec<MHPacket<SIZE>, LEN>,
    ) -> Result<(Vec<MHPacket<SIZE>, LEN>, Vec<MHPacket<SIZE>, LEN>), NetworkManagerError>;
}

pub struct NodePolicy;
impl<const SIZE: usize, const LEN: usize> RoutingPolicy<SIZE, LEN> for NodePolicy {
    fn process_packets(
        manager: &mut NetworkManager<SIZE, LEN>,
        pkts: Vec<MHPacket<SIZE>, LEN>,
    ) -> Result<(Vec<MHPacket<SIZE>, LEN>, Vec<MHPacket<SIZE>, LEN>), NetworkManagerError> {
        // If 1 package or multiple packets should be sent on:
        // let NM get these logged, and perhaps add any timed out packets
        manager.handle_packets(pkts)
    }
}

/// A gateway responds with an ACK to all packages, but the node application should also receive
/// the packet as well
pub struct GatewayPolicy;
impl<const SIZE: usize, const LEN: usize> RoutingPolicy<SIZE, LEN> for GatewayPolicy {
    fn process_packets(
        _manager: &mut NetworkManager<SIZE, LEN>,
        pkts: Vec<MHPacket<SIZE>, LEN>,
    ) -> Result<(Vec<MHPacket<SIZE>, LEN>, Vec<MHPacket<SIZE>, LEN>), NetworkManagerError> {
        let to_send = pkts
            .iter()
            // Filter out GW's own ACKS
            .filter(|pkt| pkt.packet_type != PacketType::Ack && pkt.source_id != 0)
            .map(|pkt| {
                // The rest of the fields don't really matter, because the pid is the first thing that
                // NM checks
                MHPacket {
                    destination_id: pkt.source_id,
                    source_id: pkt.destination_id,
                    packet_type: PacketType::Ack,
                    payload: Vec::new(),
                    packet_id: pkt.packet_id,
                    hop_count: 0,
                    hop_to_gw: 0,
                }
            })
            .collect();

        Ok((to_send, pkts))
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
    ) -> impl Future<Output = Result<Vec<MHPacket<SIZE>, LEN>, Node::Error>>;
}

pub struct RandomAccessMac;

impl<Node, const SIZE: usize, const LEN: usize> MacPolicy<Node, SIZE, LEN> for RandomAccessMac
where
    Node: MHNode<SIZE, LEN>,
{
    async fn run_mac(
        &mut self,
        node: &mut Node,
        tx_queue: &mut Vec<MHPacket<SIZE>, LEN>,
        rx_buffer: &mut Node::ReceiveBuffer,
    ) -> Result<Vec<MHPacket<SIZE>, LEN>, Node::Error> {
        if !tx_queue.is_empty() {
            node.transmit(tx_queue).await?;
            tx_queue.clear();
        }
        let conn = node.listen(rx_buffer, true).await?;
        node.receive(conn, rx_buffer).await
    }
}

pub struct TdmaMac {
    pub slot_duration: Duration,
    pub slots_per_frame: u8,
    pub my_tx_slot: u8,
    pub epoch: Instant,
}

impl TdmaMac {
    pub fn new(
        slot_duration: Duration,
        slots_per_frame: u8,
        my_tx_slot: u8,
        epoch: Instant,
    ) -> Self {
        Self {
            slot_duration,
            slots_per_frame,
            my_tx_slot,
            epoch,
        }
    }

    pub fn current_slot(&self, now: Instant) -> u8 {
        let elapsed_ms = (now - self.epoch).as_millis();
        let frame_duration_ms = (self.slot_duration.as_millis()) * (self.slots_per_frame as u64);

        let time_in_frame = elapsed_ms % frame_duration_ms;
        (time_in_frame / (self.slot_duration.as_millis())) as u8
    }
}

impl<Node, const SIZE: usize, const LEN: usize> MacPolicy<Node, SIZE, LEN> for TdmaMac
where
    Node: MHNode<SIZE, LEN>,
{
    async fn run_mac(
        &mut self,
        node: &mut Node,
        tx_queue: &mut Vec<MHPacket<SIZE>, LEN>,
        rx_buffer: &mut Node::ReceiveBuffer,
    ) -> Result<Vec<MHPacket<SIZE>, LEN>, Node::Error> {
        let now = Instant::now();
        let slot = self.current_slot(now);

        // Calculate when the next slot starts
        let elapsed_ms = (now - self.epoch).as_millis();
        let next_slot_start_ms = elapsed_ms + self.slot_duration.as_millis()
            - (elapsed_ms % self.slot_duration.as_millis());
        let next_slot_time = self.epoch + Duration::from_millis(next_slot_start_ms);

        let mut received_packets = Vec::new();

        if slot == self.my_tx_slot {
            if !tx_queue.is_empty() {
                node.transmit(tx_queue).await?;
                tx_queue.clear();
            }
            Timer::at(next_slot_time).await;
        } else {
            let conn = node.listen(rx_buffer, true).await;
            if let Ok(conn) = conn {
                received_packets = node.receive(conn, rx_buffer).await?;
            } else {
                info!("Error in getting conn");
            }
            Timer::at(next_slot_time).await;
        }
        Ok(received_packets)
    }
}

use crate::node::PacketType;

use super::{
    MHPacket,
    network_manager::{NetworkManager, NetworkManagerError},
};
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
                }
            })
            .collect();

        Ok((to_send, pkts))
    }
}

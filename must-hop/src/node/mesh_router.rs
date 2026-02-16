use defmt::trace;
use heapless::Vec;

use super::{
    MHNode, MHPacket, PacketType,
    network_manager::{MAX_AMOUNT_PACKETS, NetworkManager, NetworkManagerError},
};

#[derive(Debug, defmt::Format)]
pub enum MeshRouterError<E> {
    Manager(NetworkManagerError),
    Node(E),
}

impl<E> From<NetworkManagerError> for MeshRouterError<E> {
    fn from(err: NetworkManagerError) -> Self {
        MeshRouterError::Manager(err)
    }
}

// impl<E> From<E> for MeshRouterError<E> {
//     fn from(err: E) -> Self {
//         MeshRouterError::Node(err)
//     }
// }

/// Mesh Stack(MS) handles the user defined radio which implements MHNode, and a Network Manager,
/// managing the logic necessary to send and receive packets, but the user does not have to think
/// about how packets are received and sent on, if they are not for them.
/// Handles the flow of packets
pub struct MeshRouter<Node, const MAX_PACKET_SIZE: usize>
where
    Node: MHNode<MAX_PACKET_SIZE>,
{
    node: Node,
    manager: NetworkManager<MAX_PACKET_SIZE>,
}

impl<Node, const MAX_PACKET_SIZE: usize> MeshRouter<Node, MAX_PACKET_SIZE>
where
    Node: MHNode<MAX_PACKET_SIZE>,
{
    pub fn new(node: Node, manager: NetworkManager<MAX_PACKET_SIZE>) -> Self {
        Self { node, manager }
    }

    async fn send_payload(&self, payload: &[u8]) -> Result<(), MeshRouterError<Node::Error>> {
        let timeouted_pkts = self.manager.payload_to_send(payload, 2)?;
        for pkt in timeouted_pkts {
            self.node.transmit(pkt).await.map_err(MeshRouterError::Node);
        }
        Ok(())
    }

    async fn receive(
        &mut self,
        conn: Node::Connection,
        receiving_buffer: &[u8],
    ) -> Result<(), Node::Error> {
        // TODO: should be able to receieve multiple packets
        let pkt = self.node.receive(conn, &receiving_buffer).await?;
        let pkts = match pkt.packet_type {
            PacketType::Ack => {
                // This is only for Nodes close to a GW
                // self.manager.receive_ack(pkt)
                return Ok(());
            }
            PacketType::Data => Vec::from_array([pkt]),
            PacketType::DataStream(amount) => {
                // Loop for amount, and add packages to a vec of them
                let mut incoming_pkts: Vec<MHPacket, MAX_AMOUNT_PACKETS> = Vec::new();
                let mut rec_buf = [0u8; MAX_PACKET_SIZE];
                'rec_for: for idx in 1..amount {
                    trace!("Receiving packet nr {}", idx);
                    let conn = self.node.listen(&mut rec_buf, true).await?;
                    let new_pkt = self.node.receive(conn, &rec_buf).await?;
                    if incoming_pkts.push(new_pkt).is_err() {
                        trace!(
                            "BUFFER IS FULL, cannot contain any more packets, so dropping the rest"
                        );
                        break 'rec_for;
                    }
                }
                // Now if there was not too many packets, we should be able to retransmit these
                incoming_pkts
            }
        };
        // If 1 package or multiple packets should be sent on:
        // let NM get these logged, and perhaps add any timed out packets
        // self.manager.receive_multiple_packets(incoming_pkts)?;
        self.manager.handle_packets(pkts);
        Ok(())
    }
}

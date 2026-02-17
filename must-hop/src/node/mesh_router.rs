#[cfg(not(feature = "in_std"))]
use defmt::trace;
#[cfg(feature = "in_std")]
use log::trace;

use super::{
    MHNode, MHPacket, PacketType,
    network_manager::{LEN, NetworkManager, NetworkManagerError},
};
use heapless::Vec;

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
pub struct MeshRouter<Node, const SIZE: usize>
where
    Node: MHNode<SIZE>,
{
    node: Node,
    manager: NetworkManager<SIZE>,
}

impl<Node, const SIZE: usize> MeshRouter<Node, SIZE>
where
    Node: MHNode<SIZE>,
{
    /// Takes ownership of a node and network manager, because this handles those
    pub fn new(node: Node, manager: NetworkManager<SIZE>) -> Self {
        Self { node, manager }
    }

    /// Use to await another node's communication, and can be used in a select or join
    pub async fn listen(
        &mut self,
        rec_buf: &mut [u8; SIZE],
    ) -> Result<Node::Connection, MeshRouterError<Node::Error>> {
        trace!("listening ...");
        self.node
            .listen(rec_buf, false)
            .await
            .map_err(MeshRouterError::Node)
    }

    // Use to send data over the network
    pub async fn send_payload(
        &mut self,
        payload: Vec<u8, SIZE>,
        destination: u8,
    ) -> Result<(), MeshRouterError<Node::Error>> {
        let timeouted_pkts = self.manager.payload_to_send(payload, destination)?;
        trace!("Sending {} packets!", timeouted_pkts.len());

        self.send_packets(timeouted_pkts).await
    }

    async fn send_packets(
        &mut self,
        pkts: Vec<MHPacket<SIZE>, 8>,
    ) -> Result<(), MeshRouterError<Node::Error>> {
        for pkt in pkts {
            self.node
                .transmit(pkt)
                .await
                .map_err(MeshRouterError::Node)?
        }
        Ok(())
    }

    /// Handles that when receiving, the packet type can be stream, therefore this keeps on
    /// listening. Then adds packets to be sent on via the NetworkManager. Lastly, those packets
    /// are sent again if not meant for this node
    pub async fn receive(
        &mut self,
        conn: Node::Connection,
        receiving_buffer: &[u8],
    ) -> Result<Vec<MHPacket<SIZE>, 8>, MeshRouterError<Node::Error>> {
        // TODO: should be able to receieve multiple packets
        let pkt = self
            .node
            .receive(conn, receiving_buffer)
            .await
            .map_err(MeshRouterError::Node)?;
        trace!("Received packet: {:?}", pkt);
        let pkts = match pkt.packet_type {
            PacketType::Ack => {
                // This is only for Nodes close to a GW
                // self.manager.receive_ack(pkt)
                return Ok(Vec::new());
            }
            PacketType::Data => Vec::from_array([pkt]),
            PacketType::DataStream(amount) => {
                trace!("In Data Stream!");
                // Loop for amount, and add packages to a vec of them
                let mut incoming_pkts: Vec<MHPacket<SIZE>, LEN> = Vec::new();
                let mut rec_buf = [0u8; SIZE];
                'rec_for: for idx in 1..amount {
                    trace!("Receiving packet nr {}", idx);
                    let conn = self
                        .node
                        .listen(&mut rec_buf, true)
                        .await
                        .map_err(MeshRouterError::Node)?;
                    let new_pkt = self
                        .node
                        .receive(conn, &rec_buf)
                        .await
                        .map_err(MeshRouterError::Node)?;
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
        trace!("Done receiving, handling {} pkts", pkts.len());
        // If 1 package or multiple packets should be sent on:
        // let NM get these logged, and perhaps add any timed out packets
        // self.manager.receive_multiple_packets(incoming_pkts)?;
        let (to_send, my_pkt) = self.manager.handle_packets(pkts)?;
        trace!("GOT {} packets for me!", my_pkt.len());
        trace!("GOT {} packets which should be sent on!", to_send.len());
        self.send_packets(to_send).await?;
        Ok(my_pkt)
    }
}

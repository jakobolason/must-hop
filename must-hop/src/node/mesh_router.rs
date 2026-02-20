#[cfg(not(feature = "in_std"))]
use defmt::trace;
#[cfg(feature = "in_std")]
use log::trace;

use super::{
    MHNode, MHPacket,
    network_manager::{NetworkManager, NetworkManagerError},
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
pub struct MeshRouter<Node, const SIZE: usize, const LEN: usize>
where
    Node: MHNode<SIZE, LEN>,
{
    node: Node,
    manager: NetworkManager<SIZE, LEN>,
}

impl<Node, const SIZE: usize, const LEN: usize> MeshRouter<Node, SIZE, LEN>
where
    Node: MHNode<SIZE, LEN>,
{
    /// Takes ownership of a node and network manager, because this handles those
    pub fn new(node: Node, manager: NetworkManager<SIZE, LEN>) -> Self {
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
        pkts: Vec<MHPacket<SIZE>, LEN>,
    ) -> Result<(), MeshRouterError<Node::Error>> {
        self.node
            .transmit(&pkts)
            .await
            .map_err(MeshRouterError::Node)?;
        Ok(())
    }

    /// Handles that when receiving, the packet type can be stream, therefore this keeps on
    /// listening. Then adds packets to be sent on via the NetworkManager. Lastly, those packets
    /// are sent again if not meant for this node
    pub async fn receive(
        &mut self,
        conn: Node::Connection,
        receiving_buffer: &[u8],
    ) -> Result<Vec<MHPacket<SIZE>, LEN>, MeshRouterError<Node::Error>> {
        // TODO: should be able to receieve multiple packets
        let pkts = self
            .node
            .receive(conn, receiving_buffer)
            .await
            .map_err(MeshRouterError::Node)?;
        trace!("Done receiving, handling {} pkts", pkts.len());

        // If 1 package or multiple packets should be sent on:
        // let NM get these logged, and perhaps add any timed out packets
        let (to_send, my_pkt) = self.manager.handle_packets(pkts)?;
        trace!("GOT {} packets for me!", my_pkt.len());
        trace!("GOT {} packets which should be sent on!", to_send.len());
        self.send_packets(to_send).await?;
        Ok(my_pkt)
    }

    // only for tests
    #[doc(hidden)]
    pub fn get_pending_count(&self) -> usize {
        self.manager.get_pending_count()
    }
}

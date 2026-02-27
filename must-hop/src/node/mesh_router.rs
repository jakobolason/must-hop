use core::fmt;
use core::marker::PhantomData;
#[cfg(not(feature = "in_std"))]
use defmt::trace;
#[cfg(feature = "in_std")]
use log::trace;

use crate::node::policy::{GatewayPolicy, NodePolicy, RoutingPolicy};

use super::{
    MHNode, MHPacket,
    network_manager::{NetworkManager, NetworkManagerError},
};
use heapless::Vec;

#[derive(Debug)]
#[cfg_attr(not(feature = "in_std"), derive(defmt::Format))]
pub enum MeshRouterError<E> {
    Manager(NetworkManagerError),
    Node(E),
}

impl<E> From<NetworkManagerError> for MeshRouterError<E> {
    fn from(err: NetworkManagerError) -> Self {
        MeshRouterError::Manager(err)
    }
}
impl<E: fmt::Debug> fmt::Display for MeshRouterError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A simple implementation just delegates to the Debug output,
        // but you can customize this to be more human-readable.
        write!(f, "Mesh Router Error: {:?}", self)
    }
}

// 3. Implement the Error trait.
// We bound E to also implement Error so the inner error is valid too.
impl<E: fmt::Debug + core::error::Error> core::error::Error for MeshRouterError<E> {}

// impl<E> From<E> for MeshRouterError<E> {
//     fn from(err: E) -> Self {
//         MeshRouterError::Node(err)
//     }
// }

/// Mesh Stack(MS) handles the user defined radio which implements MHNode, and a Network Manager,
/// managing the logic necessary to send and receive packets, but the user does not have to think
/// about how packets are received and sent on, if they are not for them.
/// Handles the flow of packets
pub struct MeshRouter<Node, const SIZE: usize, const LEN: usize, Policy = NodePolicy>
where
    Node: MHNode<SIZE, LEN>,
    Policy: RoutingPolicy<SIZE, LEN>,
{
    node: Node,
    manager: NetworkManager<SIZE, LEN>,
    policy: PhantomData<Policy>,
}

impl<Node, Policy, const SIZE: usize, const LEN: usize> MeshRouter<Node, SIZE, LEN, Policy>
where
    Node: MHNode<SIZE, LEN>,
    Policy: RoutingPolicy<SIZE, LEN>,
{
    /// Takes ownership of a node and network manager, because this handles those
    pub fn new(node: Node, manager: NetworkManager<SIZE, LEN>, _policy: Policy) -> Self {
        Self {
            node,
            manager,
            policy: PhantomData,
        }
    }

    /// Use to await another node's communication, and can be used in a select or join
    pub async fn listen(
        &mut self,
        rec_buf: &mut Node::ReceiveBuffer,
    ) -> Result<Node::Connection, MeshRouterError<Node::Error>> {
        trace!("listening ...");
        self.node
            .listen(rec_buf, false)
            .await
            .map_err(MeshRouterError::Node)
    }

    // TODO: If an error like buffer overflow occurs, then this should be handled by the NM. I
    // think the payload received should be dropped, and the current packages retransmitted
    // Use to send data over the network
    pub async fn send_payload(
        &mut self,
        payload: Vec<u8, SIZE>,
        destination: u8,
    ) -> Result<(), MeshRouterError<Node::Error>> {
        let timeouted_pkts = self.manager.payload_to_send(payload, destination)?;
        trace!("Sending {} packets!", timeouted_pkts.len());
        self.send_packets(&timeouted_pkts).await
    }

    async fn send_packets(
        &mut self,
        // pkts: Vec<MHPacket<SIZE>, LEN>,
        pkts: &[MHPacket<SIZE>],
    ) -> Result<(), MeshRouterError<Node::Error>> {
        self.node
            .transmit(pkts)
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
        receiving_buffer: &Node::ReceiveBuffer,
    ) -> Result<Vec<MHPacket<SIZE>, LEN>, MeshRouterError<Node::Error>> {
        // TODO: should be able to receieve multiple packets
        let pkts = self
            .node
            .receive(conn, receiving_buffer)
            .await
            .map_err(MeshRouterError::Node)?;
        trace!("Done receiving, handling {} pkts", pkts.len());

        let (to_send, my_pkt) = Policy::process_packets(&mut self.manager, pkts)?;
        trace!("GOT {} packets for me!", my_pkt.len());
        trace!("GOT {} packets which should be sent on!", to_send.len());
        if !to_send.is_empty() {
            self.send_packets(&to_send).await?;
        }
        Ok(my_pkt)
    }

    // only for tests
    #[doc(hidden)]
    pub fn get_pending_count(&self) -> usize {
        self.manager.get_pending_count()
    }
}

impl<Node, const SIZE: usize, const LEN: usize> MeshRouter<Node, SIZE, LEN, GatewayPolicy>
where
    Node: MHNode<SIZE, LEN>,
{
    /// When gateway starts up, it should annonce itself, such that the nodes know their distance
    /// to GW and retransmits messages if they are closer.
    pub async fn bootup(&mut self) -> Result<(), MeshRouterError<Node::Error>> {
        let bootup_pkt = self.manager.handle_bootup()?;
        self.send_packets(&[bootup_pkt]).await
    }
}

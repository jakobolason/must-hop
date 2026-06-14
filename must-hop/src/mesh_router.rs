use core::fmt;
#[cfg(not(feature = "in_std"))]
use defmt::{error, trace};
#[cfg(feature = "in_std")]
use log::{error, trace};

use crate::policy::MacPolicy;

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

// We bound E to also implement Error so the inner error is valid too.
impl<E: fmt::Debug + core::error::Error> core::error::Error for MeshRouterError<E> {}

/// Mesh Router(MR) handles the user defined radio which implements MHNode, and a Network Manager,
/// managing the logic necessary to send and receive packets, but the user does not have to think
/// about how packets are received and sent on, if they are not for them.
/// Handles the flow of packets
pub struct MeshRouter<Node, Mac, const SIZE: usize, const LEN: usize>
where
    Node: MHNode<SIZE, LEN>,
    Mac: MacPolicy<Node, SIZE, LEN>,
{
    node: Node,
    manager: NetworkManager<SIZE, LEN>,
    mac_policy: Mac,
    tx_queue: Vec<MHPacket<SIZE>, LEN>,
}

impl<Node, Mac, const SIZE: usize, const LEN: usize> MeshRouter<Node, Mac, SIZE, LEN>
where
    Node: MHNode<SIZE, LEN>,
    Mac: MacPolicy<Node, SIZE, LEN>,
{
    /// Takes ownership of a node and network manager, because this handles those
    pub fn new(node: Node, manager: NetworkManager<SIZE, LEN>, mac_policy: Mac) -> Self {
        Self {
            node,
            manager,
            mac_policy,
            tx_queue: Vec::new(),
        }
    }

    pub fn queue_payload(
        &mut self,
        payload: Vec<u8, SIZE>,
        destination: u16,
    ) -> Result<(), MeshRouterError<Node::Error>> {
        trace!("Queing payload ...");
        let pkt = self.manager.queue_new_payload(payload, destination)?;
        self.push_queue(pkt)?;
        Ok(())
    }

    fn push_queue(&mut self, pkt: MHPacket<SIZE>) -> Result<(), MeshRouterError<Node::Error>> {
        self.tx_queue
            .push(pkt)
            .map_err(|_| MeshRouterError::Manager(NetworkManagerError::BufferFull))?;
        Ok(())
    }

    pub async fn tick(
        &mut self,
        rx_buf: &mut Node::ReceiveBuffer,
    ) -> Result<Vec<MHPacket<SIZE>, LEN>, MeshRouterError<Node::Error>> {
        if self.mac_policy.should_tx_heartbeat() {
            trace!("SENDING OUT HEARTBEAT!!");
            self.mac_policy.tx_heartbeat(self.manager.add_heartbeat()?);
        }

        let retransmission = self.manager.get_pending_transmissions();
        for pkt in retransmission {
            self.push_queue(pkt)?;
        }

        let received_pkts = self
            .mac_policy
            .run_mac(&mut self.node, &mut self.tx_queue, rx_buf)
            .await
            .map_err(MeshRouterError::Node)?;
        // Short circuit if no packets received
        let received_pkts = match received_pkts {
            Some(pkts) => pkts,
            None => return Ok(Vec::new()),
        };
        let (to_forward, to_me) = self.manager.handle_packets(received_pkts)?;
        self.mac_policy.set_gw_hops(self.manager.get_gw_hops());
        trace!("[PKT_LOSS]|{}|", self.manager.packet_loss_ratio());

        for pkt in to_forward {
            // If buffer is full, break adding packets to it.
            if self.tx_queue.push(pkt).is_err() {
                error!("Tx queue is full, dropping packets ...");
                break;
            }
        }
        Ok(to_me)
    }

    // only for tests
    #[doc(hidden)]
    pub fn get_pending_count(&self) -> usize {
        self.manager.get_pending_count()
    }
    #[doc(hidden)]
    pub fn get_packet_loss_ratio(&self) -> f32 {
        self.manager.packet_loss_ratio()
    }
}

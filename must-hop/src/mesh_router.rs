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
pub enum MeshRouterError<E, Radio> {
    Manager(NetworkManagerError<Radio>),
    Node(E),
}

impl<E, Radio> From<NetworkManagerError<Radio>> for MeshRouterError<E, Radio> {
    fn from(err: NetworkManagerError<Radio>) -> Self {
        MeshRouterError::Manager(err)
    }
}
impl<E: fmt::Debug, Radio: fmt::Debug> fmt::Display for MeshRouterError<E, Radio> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A simple implementation just delegates to the Debug output,
        // but you can customize this to be more human-readable.
        write!(f, "Mesh Router Error: {:?}", self)
    }
}

// We bound E to also implement Error so the inner error is valid too.
impl<E: core::error::Error, Radio: core::error::Error> core::error::Error
    for MeshRouterError<E, Radio>
{
}

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
    ) -> Result<(), MeshRouterError<Node::Error, Node::RadioError>> {
        trace!("Queing payload ...");
        let pkt = self.manager.queue_new_payload(payload, destination)?;
        self.push_queue(pkt)?;
        Ok(())
    }

    fn push_queue(
        &mut self,
        pkt: MHPacket<SIZE>,
    ) -> Result<(), MeshRouterError<Node::Error, Node::RadioError>> {
        self.tx_queue
            .push(pkt)
            .map_err(|_| MeshRouterError::Manager(NetworkManagerError::BufferFull))?;
        Ok(())
    }

    pub fn hops_to_gw(&self) -> u8 {
        self.manager.get_gw_hops()
    }

    pub async fn tick(
        &mut self,
        rx_buf: &mut Node::ReceiveBuffer,
    ) -> Result<Vec<MHPacket<SIZE>, LEN>, MeshRouterError<Node::Error, Node::RadioError>> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        RandomAccessMac,
        node::lora::{LoraNode, RadioPackParams, RatioModParams},
        policy::ra::NodePolicy,
    };
    use core::time::Duration;

    use lora_modulation::{Bandwidth, CodingRate, SpreadingFactor};
    use lora_phy::{DelayNs, LoRa, mock::MockRadio, mod_traits::IrqState};
    // use must_hop::{
    //     MeshRouter, NetworkManager, RandomAccessMac,
    //     node::lora::{LoraNode, RadioPackParams, RatioModParams},
    //     policy::ra::NodePolicy,
    // };
    use tokio::time::sleep;

    const MAX_PACK_LEN: usize = 40;
    const LEN: usize = 8;
    const OUTPUT_POWER: i32 = 0;
    const LORA_FREQUENCY_IN_HZ: u32 = 868_700_000;

    pub struct TokioDelay;

    impl DelayNs for TokioDelay {
        async fn delay_ns(&mut self, ns: u32) {
            sleep(Duration::from_nanos(ns as u64)).await;
        }

        async fn delay_us(&mut self, us: u32) {
            sleep(Duration::from_micros(us as u64)).await;
        }

        async fn delay_ms(&mut self, ms: u32) {
            sleep(Duration::from_millis(ms as u64)).await;
        }
    }

    macro_rules! setup_mock_radio {
        ($lora:expr, $node_id:expr) => {{
            let sf = SpreadingFactor::_5;
            let bw = Bandwidth::_7KHz;
            let cr = CodingRate::_4_5;
            let mp = RatioModParams {
                sf,
                bw,
                cr,
                lora_hz: LORA_FREQUENCY_IN_HZ,
            };
            let tp = RadioPackParams {
                pre_amp: 8,
                imp_hed: false,
                max_pack_len: MAX_PACK_LEN,
                crc: true,
                iq: false,
            };

            let node =
                LoraNode::<_, _, MAX_PACK_LEN, LEN, OUTPUT_POWER>::new(&mut $lora, tp, mp, None)
                    .unwrap();
            let manager = NetworkManager::new($node_id, 0, 5, None);
            let mac = RandomAccessMac::new(NodePolicy);
            MeshRouter::new(node, manager, mac)
        }};
    }

    #[tokio::test]
    async fn test_mock() {
        let mr = MockRadio::new();
        let mut lora = LoRa::new(mr, false, TokioDelay).await.unwrap();
        let mut mr = setup_mock_radio!(lora, 0);
        let mut rb: [u8; 256] = [0_u8; 256];
        let pkts = mr.tick(&mut rb).await.unwrap();
        assert_eq!(pkts.len(), 0);
        let state = mr.node.lora_mut().get_irq_state().await.unwrap().unwrap();
        assert!(state == IrqState::Done);

        let lr = mr.node.lora_mut();
        mr.queue_payload(Vec::from_slice(&[1, 2, 3]).unwrap(), 2)
            .unwrap();
        let state = mr.node.lora_mut().get_irq_state().await.unwrap().unwrap();
        assert!(state == IrqState::Done);
    }
}

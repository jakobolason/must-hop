use core::fmt;
#[cfg(not(feature = "in_std"))]
use defmt::{error, trace};
#[cfg(feature = "in_std")]
use log::{error, trace};

use crate::{
    ll::LinkLayer,
    nw_layer::{NetworkError, NetworkLayer, TransmitBuffer},
    policy::MacPolicy,
};

use super::{MHNode, MeshPacket};
use heapless::Vec;

#[derive(Debug)]
#[cfg_attr(not(feature = "in_std"), derive(defmt::Format))]
pub enum MeshRouterError<E> {
    Network(NetworkError),
    Node(E),
    BufferFull,
}

impl<E> From<NetworkError> for MeshRouterError<E> {
    fn from(err: NetworkError) -> Self {
        MeshRouterError::Network(err)
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
impl<E: core::error::Error> core::error::Error for MeshRouterError<E> {}

/// Mesh Router(MR) handles the user defined radio which implements MHNode, and a Network Layer,
/// managing the logic necessary to send and receive packets, but the user does not have to think
/// about how packets are received and sent on, if they are not for them.
/// Handles the flow of packets
pub struct MeshRouter<Node, Mac, LL, NW, const SIZE: usize, const LEN: usize>
where
    Node: MHNode<SIZE, LEN>,
    Mac: MacPolicy<Node, SIZE, LEN>,
    LL: LinkLayer,
    NW: NetworkLayer<SIZE>,
{
    node: Node,
    nw: NW,
    mac_policy: Mac,
    link_layer: LL,
    tx_queue: TransmitBuffer<SIZE, LEN>,
}

impl<Node, Mac, LL, NW, const SIZE: usize, const LEN: usize>
    MeshRouter<Node, Mac, LL, NW, SIZE, LEN>
where
    Node: MHNode<SIZE, LEN>,
    Mac: MacPolicy<Node, SIZE, LEN>,
    LL: LinkLayer,
    NW: NetworkLayer<SIZE>,
{
    /// Takes ownership of a node and network layer, because this handles those
    pub fn new(node: Node, nw: NW, mac_policy: Mac, link_layer: LL) -> Self {
        Self {
            node,
            nw,
            mac_policy,
            link_layer,
            tx_queue: TransmitBuffer::new(),
        }
    }

    pub fn queue_payload(
        &mut self,
        payload: &[u8],
        destination: u16,
    ) -> Result<(), MeshRouterError<Node::Error>> {
        trace!("Queing payload ...");
        self.nw
            .queue_payload(destination, payload, &mut self.tx_queue)?;
        Ok(())
    }

    pub fn tx_free(&self) -> usize {
        self.tx_queue.capacity() - self.tx_queue.len()
    }

    pub fn hops_to_gw(&self) -> u8 {
        self.nw.gw_hops()
    }

    pub async fn run(&mut self) -> Result<Vec<MeshPacket<SIZE>, LEN>, MeshRouterError<Node::Error>> {
        todo!()
    }

    pub async fn tick(
        &mut self,
        rx_buf: &mut Node::ReceiveBuffer,
    ) -> Result<Vec<MeshPacket<SIZE>, LEN>, MeshRouterError<Node::Error>> {
        if self.mac_policy.should_tx_heartbeat()
            && let Some(hb) = self.nw.build_heartbeat()
        {
            trace!("SENDING OUT HEARTBEAT!!");
            self.mac_policy.tx_heartbeat(hb);
        }

        // TODO: retransmissions used to be driven from here via NetworkManager::get_pending_transmissions.
        // That state now belongs to the LinkLayer; wire LL.tick-equivalent here when the LL
        // retransmission API lands.

        let received_pkts = self
            .mac_policy
            .run_mac(&mut self.node, &mut self.tx_queue, rx_buf)
            .await
            .map_err(MeshRouterError::Node)?;
        // Short circuit if no packets received
        let received_pkts = match received_pkts {
            Some(pkts) => pkts,
            // Compiler should reduce footprint of unecessary call
            None => return Ok(Vec::new()),
        };

        let mut to_me: Vec<MeshPacket<SIZE>, LEN> = Vec::new();
        for pkt in received_pkts {
            let outcome = self.nw.receive(pkt);
            if let Some(fwd) = outcome.forward
                && self.tx_queue.push(fwd).is_err()
            {
                error!("Tx queue is full, dropping forwarded packet ...");
            }
            if let Some(d) = outcome.deliver
                && to_me.push(d).is_err()
            {
                error!("to_me buffer full, dropping delivered packet");
            }
            if let Some((pkt_id, dst)) = outcome.ack {
                self.link_layer.send_ack(pkt_id, dst);
            }
        }

        self.mac_policy.set_gw_hops(self.nw.gw_hops());
        Ok(to_me)
    }
}

#[cfg(all(test, feature = "lora"))]
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

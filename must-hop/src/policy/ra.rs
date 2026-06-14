use crate::policy::MacPolicy;
use crate::{PacketType, node::MHNode};

use super::MHPacket;

use embassy_time::{Duration, Instant};
use heapless::Vec;

// const GATEWAY_ID: u8 = 1;

pub trait NodeRole {
    fn check_heartbeat(&self) -> bool;
}

pub struct NodePolicy;
impl NodeRole for NodePolicy {
    fn check_heartbeat(&self) -> bool {
        false
    }
}

/// A gateway sends out periodic heartbeats
#[cfg(feature = "in_std")]
pub struct GatewayPolicy {
    pub last_heartbeat: Option<Instant>,
    pub timeout: u8,
}
#[cfg(feature = "in_std")]
impl GatewayPolicy {
    pub fn new(timeout: u8) -> Self {
        Self {
            last_heartbeat: None,
            timeout,
        }
    }
}

#[cfg(feature = "in_std")]
impl NodeRole for GatewayPolicy {
    fn check_heartbeat(&self) -> bool {
        let now = Instant::now();
        match self.last_heartbeat {
            None => true,
            Some(last) => now.duration_since(last) >= Duration::from_secs(self.timeout as u64),
        }
    }
}

/// A RA MAC policy which sends when it has a packet to send, and listens otherwise
pub struct RandomAccessMac<const SIZE: usize, NR: NodeRole> {
    hbt_pkt: Option<MHPacket<SIZE>>,
    node_role: NR,
    gw_hops: u8,
    recent_seen_hb: [(u16, u16); 5],
    cursor: usize,
}

impl<const SIZE: usize, NR: NodeRole> RandomAccessMac<SIZE, NR> {
    pub fn new(node_role: NR) -> Self {
        Self {
            hbt_pkt: None,
            node_role,
            gw_hops: 255,
            recent_seen_hb: [(0, 0); 5],
            cursor: 0,
        }
    }

    fn handle_hb(&mut self, pkt: &MHPacket<SIZE>) -> Option<MHPacket<SIZE>> {
        let id = (pkt.source_id, pkt.packet_id);

        // If we haven't flooded this specific heartbeat yet
        if !self.recent_seen_hb.contains(&id) {
            self.recent_seen_hb[self.cursor] = id;
            self.cursor = (self.cursor + 1) % 5;

            // Only forward if we are logically between source and edge
            if pkt.hop_count < self.gw_hops {
                let mut fwd_pkt = pkt.clone();
                fwd_pkt.hop_count += 1;
                // Push it to the queue to be sent on the next tick
                return Some(fwd_pkt);
            }
        }
        None
    }
}

// impl<const SIZE: usize, NR: NodeRole> Default for RandomAccessMac<SIZE, NR> {
//     fn default() -> Self {
//         Self::new(NodePolicy)
//     }
// }

impl<Node, const SIZE: usize, const LEN: usize, NR: NodeRole> MacPolicy<Node, SIZE, LEN>
    for RandomAccessMac<SIZE, NR>
where
    Node: MHNode<SIZE, LEN>,
{
    fn set_gw_hops(&mut self, gw_hops: u8) {
        self.gw_hops = gw_hops
    }

    fn should_tx_heartbeat(&mut self) -> bool {
        self.node_role.check_heartbeat()
    }
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
                // If queue full
                self.hbt_pkt = Some(pkt)
            }
            node.transmit(tx_queue).await?;
            tx_queue.clear();
        }
        match node
            .listen(rx_buffer, Some(core::time::Duration::from_secs(1)))
            .await
        {
            Ok(conn) => match node.receive(conn, rx_buffer).await {
                Ok((pkts, _rx_hw_timestamp)) => {
                    // Heartbeats that should be relayed must go out on the next tick.
                    // Push into tx_queue (not pkts) — pkts feeds handle_packets, which
                    // returns None for HB and would drop the forwarded copy.
                    if let Some(fwd_pkt) = pkts
                        .iter()
                        .filter(|p| p.packet_type == PacketType::HeartBeat)
                        .find_map(|p| self.handle_hb(p))
                    {
                        let _ = tx_queue.push(fwd_pkt);
                    }
                    Ok(Some(pkts))
                }
                Err(e) => Err(e),
            },
            Err(_e) => {
                // error!("Error in listening: {:?}", e);
                Ok(None)
            }
        }
    }
}

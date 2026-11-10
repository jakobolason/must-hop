//! Integration tests for the mesh via `GradientRouter` + `MeshRouter`.

use embassy_time::Instant;
use must_hop::{
    MHNode, MeshPacket, RxPacket,
    ll::LinkLayer,
    mesh_router::MeshRouter,
    nw_layer::prototype::{BROADCAST_ID, GW_SRC_ID, GradientRouter},
    policy::ra::{GatewayPolicy, NodePolicy, RandomAccessMac},
};
use std::sync::{Arc, Mutex};
use std::{collections::HashMap, time::Duration};

const SIZE: usize = 40;
const LEN: usize = 5;

#[derive(Debug)]
pub enum MockError {}

pub struct SimulationEnv<const SIZE: usize> {
    /// Maps a Node ID to a list of Node IDs that can hear its transmissions.
    /// E.g., Node 1 -> [2, 3] means if 1 transmits, 2 and 3 receive it.
    pub topology: HashMap<u16, std::vec::Vec<u16>>,

    /// Each node's personal receiving buffer (their "inbox")
    pub inboxes: HashMap<u16, std::vec::Vec<MeshPacket<SIZE>>>,
}

impl<const SIZE: usize> Default for SimulationEnv<SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const SIZE: usize> SimulationEnv<SIZE> {
    pub fn new() -> Self {
        Self {
            topology: HashMap::new(),
            inboxes: HashMap::new(),
        }
    }

    /// Define that `receiver` is within radio range of `sender`
    pub fn add_link(&mut self, sender: u16, receiver: u16) {
        self.topology.entry(sender).or_default().push(receiver);
        // Make sure the receiver has an inbox ready
        self.inboxes.entry(receiver).or_default();
    }
    pub fn add_bidi_link(&mut self, node_a: u16, node_b: u16) {
        self.add_link(node_a, node_b);
        self.add_link(node_b, node_a);
    }
}

struct MockRadio<const SIZE: usize> {
    pub node_id: u16,
    pub env: Arc<Mutex<SimulationEnv<SIZE>>>,
}

impl<const SIZE: usize, const LEN: usize> MHNode<SIZE, LEN> for MockRadio<SIZE> {
    type Error = MockError;
    type Connection = ();
    type ReceiveBuffer = ();
    type RadioError = MockError;

    async fn transmit(
        &mut self,
        packets: &[MeshPacket<SIZE>],
        hb_pkt: Option<MeshPacket<SIZE>>,
    ) -> Result<(), Self::Error> {
        let mut env = self.env.lock().unwrap();

        // Find all nodes that are in range of THIS transmitting node
        if let Some(neighbors) = env.topology.get(&self.node_id).cloned() {
            for neighbor_id in neighbors {
                // For every neighbor in range, push a clone of the packets into their inbox
                if let Some(inbox) = env.inboxes.get_mut(&neighbor_id) {
                    for pkt in packets {
                        inbox.push(pkt.clone());
                    }
                    if let Some(hb) = hb_pkt.as_ref() {
                        inbox.push(hb.clone());
                    }
                }
            }
        }
        Ok(())
    }

    async fn receive(
        &mut self,
        _conn: Self::Connection,
        _receiving_buffer: &(),
    ) -> Result<(heapless::Vec<MeshPacket<SIZE>, LEN>, RxPacket), Self::Error> {
        let mut env = self.env.lock().unwrap();
        let mut rec_vec: heapless::Vec<MeshPacket<SIZE>, LEN> = heapless::Vec::new();

        // Only look at OUR specific inbox
        if let Some(my_inbox) = env.inboxes.get_mut(&self.node_id) {
            // Drain items from the front of our inbox until our heapless::Vec is full
            while !my_inbox.is_empty() {
                if rec_vec.is_full() {
                    break;
                }
                rec_vec.push(my_inbox.remove(0)).unwrap();
            }
        }
        Ok((
            rec_vec,
            RxPacket {
                rx_done_instant: Instant::now(),
                payload_size: 0,
            },
        ))
    }

    async fn listen(
        &mut self,
        _receiving_buffer: &mut (),
        _with_timeout: Option<Duration>,
    ) -> Result<Self::Connection, Self::Error> {
        Ok(())
    }

    fn calc_tx_delay(&self, _payload_len: usize) -> u64 {
        0
    }
}

/// No-op link layer. Pending-ack tracking will live here once implemented;
/// for now it swallows ACK requests so tests can run without retransmission.
struct NoopLL;

impl LinkLayer for NoopLL {
    fn send_ack(&mut self, _packet_id: u16, _destination: u16) {}
    fn send_packets(&mut self, _packets: &[u8]) {}
    fn receive_packets(&mut self, _packets: &[u8]) {}
}

type TestRouter = MeshRouter<
    MockRadio<SIZE>,
    RandomAccessMac<SIZE, NodePolicy>,
    NoopLL,
    GradientRouter<SIZE>,
    SIZE,
    LEN,
>;
type GwRouter = MeshRouter<
    MockRadio<SIZE>,
    RandomAccessMac<SIZE, GatewayPolicy>,
    NoopLL,
    GradientRouter<SIZE>,
    SIZE,
    LEN,
>;

fn make_node(env: Arc<Mutex<SimulationEnv<SIZE>>>, node_id: u16) -> TestRouter {
    MeshRouter::new(
        MockRadio { node_id, env },
        GradientRouter::<SIZE>::new_node(node_id),
        RandomAccessMac::new(NodePolicy {}),
        NoopLL,
    )
}

fn make_gw(env: Arc<Mutex<SimulationEnv<SIZE>>>, gw_timeout: u8) -> GwRouter {
    MeshRouter::new(
        MockRadio {
            node_id: GW_SRC_ID,
            env,
        },
        GradientRouter::<SIZE>::new_gateway(),
        RandomAccessMac::new(GatewayPolicy::new(gw_timeout)),
        NoopLL,
    )
}

/// Pump GW + a chain of nodes until each node has learned its hop-count to the GW.
/// Heartbeats propagate one hop per round of ticks, so we tick `hops + 2` rounds
/// to leave headroom.
async fn propagate_heartbeats(gw: &mut GwRouter, chain: &mut [&mut TestRouter], hops: usize) {
    for _ in 0..(hops + 2) {
        let _ = gw.tick(&mut ()).await.unwrap();
        for r in chain.iter_mut() {
            let _ = r.tick(&mut ()).await.unwrap();
        }
    }
}

#[tokio::test]
async fn chain_to_gw() {
    // A straight chain where a leaf sends upstream.
    let env = Arc::new(Mutex::new(SimulationEnv::new()));
    let node_a = 2;
    let node_b = 3;
    let node_c = 4;
    let node_d = 5;

    {
        let mut e = env.lock().unwrap();
        // (A) <-> (B) <-> (C) <-> (D) <-> (GW)
        e.add_bidi_link(node_a, node_b);
        e.add_bidi_link(node_b, node_c);
        e.add_bidi_link(node_c, node_d);
        e.add_bidi_link(node_d, GW_SRC_ID);
    }

    let mut router_a = make_node(env.clone(), node_a);
    let mut router_b = make_node(env.clone(), node_b);
    let mut router_c = make_node(env.clone(), node_c);
    let mut router_d = make_node(env.clone(), node_d);
    let mut gw = make_gw(env.clone(), 0);

    // GW floods a heartbeat so every node learns its hop count
    propagate_heartbeats(
        &mut gw,
        &mut [&mut router_d, &mut router_c, &mut router_b, &mut router_a],
        4,
    )
    .await;
    assert_eq!(gw.hops_to_gw(), 0);
    assert_eq!(router_d.hops_to_gw(), 1);
    assert_eq!(router_c.hops_to_gw(), 2);
    assert_eq!(router_b.hops_to_gw(), 3);
    assert_eq!(router_a.hops_to_gw(), 4);

    let msg1 = [0x01u8];

    router_a.queue_payload(&msg1, GW_SRC_ID).unwrap();
    let res_a = router_a.tick(&mut ()).await.unwrap();
    assert_eq!(res_a.len(), 0);

    // Node B now receives these
    let res1 = router_b.tick(&mut ()).await.unwrap();
    assert_eq!(res1.len(), 0);
    let res1 = router_b.tick(&mut ()).await.unwrap();
    // These packages were not meant for us, so we should not receive anything here
    assert_eq!(res1.len(), 0);
    // But router b should have forwarded the packet upstream toward the GW
    // (the pending-ack bookkeeping that used to live on the router is now the
    // link layer's problem, so we no longer assert on it here)
    // And should've sent a package over the air, which router A can also observe —
    // A is farther from the GW and must therefore drop it, not re-forward.

    let res2 = router_a.tick(&mut ()).await.unwrap();
    assert_eq!(res2.len(), 0);

    // Router C should've also received it and does send it on, because the
    // packet is bound for the GW and C is strictly closer to the GW than B.
    let res3 = router_c.tick(&mut ()).await.unwrap();
    assert_eq!(res3.len(), 0);
    let _ = router_c.tick(&mut ()).await.unwrap();
    let _ = router_b.tick(&mut ()).await.unwrap();

    // Router D should've also received it. D is one hop from the GW and still
    // not the destination, so it forwards.
    let res3 = router_d.tick(&mut ()).await.unwrap();
    assert_eq!(res3.len(), 0);
    let _ = router_d.tick(&mut ()).await.unwrap();

    // GW delivers the payload to the application
    let delivered = gw.tick(&mut ()).await.unwrap();
    assert_eq!(delivered.len(), 1);
    assert_eq!(&delivered[0].payload[..], &msg1);
    assert_eq!(delivered[0].source_id, node_a);
}

#[tokio::test]
async fn node_closer_to_gw_forwards_upstream() {
    // B emits, on an upstream-to-GW path, only nodes strictly closer to the GW forward.
    let env = Arc::new(Mutex::new(SimulationEnv::new()));
    let node_a = 2;
    let node_b = 3;
    let node_c = 4;

    {
        let mut e = env.lock().unwrap();
        // (A) <-> (B) <-> (C) <-> (GW)
        e.add_bidi_link(node_a, node_b);
        e.add_bidi_link(node_b, node_c);
        e.add_bidi_link(node_c, GW_SRC_ID);
    }

    let mut router_a = make_node(env.clone(), node_a);
    let mut router_b = make_node(env.clone(), node_b);
    let mut router_c = make_node(env.clone(), node_c);
    let mut gw = make_gw(env.clone(), 0);

    propagate_heartbeats(
        &mut gw,
        &mut [&mut router_c, &mut router_b, &mut router_a],
        3,
    )
    .await;
    assert_eq!(router_a.hops_to_gw(), 3);
    assert_eq!(router_b.hops_to_gw(), 2);
    assert_eq!(router_c.hops_to_gw(), 1);

    let msg1 = [0x01u8];

    router_b.queue_payload(&msg1, GW_SRC_ID).unwrap();
    let resb = router_b.tick(&mut ()).await.unwrap();
    assert_eq!(resb.len(), 0);

    // Both node A and C are in range of B, so they both receive the packet
    let res2 = router_a.tick(&mut ()).await.unwrap();
    assert_eq!(res2.len(), 0);
    // And since A is farther from the GW than B, A does not retransmit

    // Router C should've also received it. C is strictly closer to the GW, so
    // it forwards it toward the GW rather than delivering it.
    let res3 = router_c.tick(&mut ()).await.unwrap();
    assert_eq!(res3.len(), 0);
    // And does not deliver it locally — C is only a forwarder here

    // Carry it the last hop into the GW
    for _ in 0..3 {
        let _ = router_c.tick(&mut ()).await.unwrap();
    }
    let delivered = gw.tick(&mut ()).await.unwrap();
    assert_eq!(delivered.len(), 1);
    assert_eq!(&delivered[0].payload[..], &msg1);
    assert_eq!(delivered[0].source_id, node_b);
}

#[tokio::test]
async fn broadcast_reaches_every_node() {
    // Tests that a flood reaches every node exactly once. The mesh exposes
    // multi-receiver semantic through broadcast.
    let env = Arc::new(Mutex::new(SimulationEnv::new()));
    let node_a = 2;
    let node_b = 3;
    let node_c = 4;
    let node_d = 5;

    {
        let mut e = env.lock().unwrap();
        //           /<-----------.>\
        // (A) <-> (B) <-> (C) <-> (D)
        //   \<----------->/
        e.add_bidi_link(node_a, node_b);
        e.add_bidi_link(node_a, node_c);
        e.add_bidi_link(node_b, node_c);
        e.add_bidi_link(node_b, node_d);
        e.add_bidi_link(node_c, node_d);
    }

    let mut router_a = make_node(env.clone(), node_a);
    let mut router_b = make_node(env.clone(), node_b);
    let mut router_c = make_node(env.clone(), node_c);
    let mut router_d = make_node(env.clone(), node_d);

    let msg1 = [0x01u8];

    router_a.queue_payload(&msg1, BROADCAST_ID).unwrap();
    let resa = router_a.tick(&mut ()).await.unwrap();
    assert_eq!(resa.len(), 0);

    // Both nodes B and C are in range of A, so they both receive the packet.
    // Each delivers the broadcast to its application and re-floods it once,
    // deduplicated on (source_id, packet_id) so the re-emission dies out.
    let mut delivered = HashMap::<u16, usize>::new();
    for _ in 0..6 {
        let _ = router_a.tick(&mut ()).await.unwrap();
        for (id, r) in [
            (node_b, &mut router_b),
            (node_c, &mut router_c),
            (node_d, &mut router_d),
        ] {
            let got = r.tick(&mut ()).await.unwrap();
            *delivered.entry(id).or_default() += got.len();
        }
    }

    // Each neighbour receives the broadcast exactly once
    assert_eq!(delivered.get(&node_b).copied().unwrap_or(0), 1);
    assert_eq!(delivered.get(&node_c).copied().unwrap_or(0), 1);
    // And D, two hops away, also gets it exactly once via B's or C's re-flood
    assert_eq!(delivered.get(&node_d).copied().unwrap_or(0), 1);
}

#[tokio::test]
async fn gw_communication() {
    let env = Arc::new(Mutex::new(SimulationEnv::new()));
    let node_a = 2;
    let node_b = 3;
    let node_c = 4;
    let node_d = 5;

    {
        let mut e = env.lock().unwrap();
        //
        // (A) <-> (B) <-> (C) <-> (D) <-> (GW)
        //
        e.add_bidi_link(node_a, node_b);
        e.add_bidi_link(node_b, node_c);
        e.add_bidi_link(node_c, node_d);
        e.add_bidi_link(node_d, GW_SRC_ID);
    }

    let mut router_a = make_node(env.clone(), node_a);
    let mut router_b = make_node(env.clone(), node_b);
    let mut router_c = make_node(env.clone(), node_c);
    let mut router_d = make_node(env.clone(), node_d);
    let mut gw = make_gw(env.clone(), 0);

    // First GW sends out Bootup (heartbeat); then each node learns its hop count
    propagate_heartbeats(
        &mut gw,
        &mut [&mut router_d, &mut router_c, &mut router_b, &mut router_a],
        4,
    )
    .await;
    assert_eq!(gw.hops_to_gw(), 0);
    assert_eq!(router_d.hops_to_gw(), 1);

    let msg1 = [0x01u8];

    router_a.queue_payload(&msg1, GW_SRC_ID).unwrap();
    let resa = router_a.tick(&mut ()).await.unwrap();
    assert_eq!(resa.len(), 0);

    // Only B is in range of A, so it receives the packet. Since it's not for
    // node B, it forwards upstream.
    let res2 = router_b.tick(&mut ()).await.unwrap();
    assert_eq!(res2.len(), 0);
    let _ = router_b.tick(&mut ()).await.unwrap();
    let _ = router_a.tick(&mut ()).await.unwrap();

    let res3 = router_c.tick(&mut ()).await.unwrap();
    assert_eq!(res3.len(), 0);
    let _ = router_c.tick(&mut ()).await.unwrap();
    let _ = router_b.tick(&mut ()).await.unwrap();

    let d = router_d.tick(&mut ()).await.unwrap();
    assert_eq!(d.len(), 0);
    let _ = router_d.tick(&mut ()).await.unwrap();
    let _ = router_c.tick(&mut ()).await.unwrap();

    // GW delivers to the application
    let res4 = gw.tick(&mut ()).await.unwrap();
    assert_eq!(res4.len(), 1);
    assert_eq!(&res4[0].payload[..], &msg1);
    assert_eq!(res4[0].source_id, node_a);
}

#[tokio::test]
async fn complex_gw_communication() {
    let env = Arc::new(Mutex::new(SimulationEnv::new()));
    let node_a = 2;
    let node_b = 3;
    let node_c = 4;
    let node_d = 5;

    {
        let mut e = env.lock().unwrap();
        //           /<-----------.>\
        // (A) <-> (B) <-> (C) <-> (D) <-> (GW)
        //   \<----------->/
        e.add_bidi_link(node_a, node_b);
        e.add_bidi_link(node_a, node_c);
        e.add_bidi_link(node_b, node_c);
        e.add_bidi_link(node_b, node_d);
        e.add_bidi_link(node_c, node_d);
        e.add_bidi_link(node_d, GW_SRC_ID);
    }

    let mut router_a = make_node(env.clone(), node_a);
    let mut router_b = make_node(env.clone(), node_b);
    let mut router_c = make_node(env.clone(), node_c);
    let mut router_d = make_node(env.clone(), node_d);
    let mut gw = make_gw(env.clone(), 0);

    // First GW sends out Bootup; the denser graph means B and C can both
    // reach D directly, so both end up at hop_to_gw = 2.
    propagate_heartbeats(
        &mut gw,
        &mut [&mut router_d, &mut router_c, &mut router_b, &mut router_a],
        4,
    )
    .await;
    assert_eq!(router_d.hops_to_gw(), 1);
    assert_eq!(router_c.hops_to_gw(), 2);
    assert_eq!(router_b.hops_to_gw(), 2);

    let msg1 = [0x01u8];

    router_a.queue_payload(&msg1, GW_SRC_ID).unwrap();
    let resa = router_a.tick(&mut ()).await.unwrap();
    assert_eq!(resa.len(), 0);

    // Both nodes B and C are in range of A, so they both receive the packet.
    // Both are closer to the GW, so both will forward.
    let res2 = router_b.tick(&mut ()).await.unwrap();
    assert_eq!(res2.len(), 0);
    let res3 = router_c.tick(&mut ()).await.unwrap();
    assert_eq!(res3.len(), 0);
    let _ = router_b.tick(&mut ()).await.unwrap();
    let _ = router_a.tick(&mut ()).await.unwrap();

    let _ = router_c.tick(&mut ()).await.unwrap();
    let _ = router_b.tick(&mut ()).await.unwrap();

    let d = router_d.tick(&mut ()).await.unwrap();
    assert_eq!(d.len(), 0);
    let _ = router_d.tick(&mut ()).await.unwrap();
    let _ = router_c.tick(&mut ()).await.unwrap();

    let res4 = gw.tick(&mut ()).await.unwrap();
    assert_eq!(res4.len(), 1);
    assert_eq!(&res4[0].payload[..], &msg1);
}

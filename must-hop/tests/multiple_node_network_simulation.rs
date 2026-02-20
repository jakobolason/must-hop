use heapless::Vec;
use must_hop::node::{
    MHNode, MHPacket,
    mesh_router::MeshRouter,
    network_manager::{NetworkManager, NetworkManagerError},
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const SIZE: usize = 40;
const LEN: usize = 5;

// We can use standard Vec here since this is just the host-side test simulation,
// not the actual no_std firmware.
pub struct SimulationEnv<const SIZE: usize> {
    /// Maps a Node ID to a list of Node IDs that can hear its transmissions.
    /// E.g., Node 1 -> [2, 3] means if 1 transmits, 2 and 3 receive it.
    pub topology: HashMap<u8, std::vec::Vec<u8>>,

    /// Each node's personal receiving buffer (their "inbox")
    pub inboxes: HashMap<u8, std::vec::Vec<MHPacket<SIZE>>>,
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
    pub fn add_link(&mut self, sender: u8, receiver: u8) {
        self.topology.entry(sender).or_default().push(receiver);
        // Make sure the receiver has an inbox ready
        self.inboxes.entry(receiver).or_default();
    }
    pub fn add_bidi_link(&mut self, node_a: u8, node_b: u8) {
        self.add_link(node_a, node_b);
        self.add_link(node_b, node_a);
    }
}

struct MockRadio<const SIZE: usize> {
    pub node_id: u8,
    pub env: Arc<Mutex<SimulationEnv<SIZE>>>,
}

impl<const SIZE: usize, const LEN: usize> MHNode<SIZE, LEN> for MockRadio<SIZE> {
    type Error = NetworkManagerError;
    type Connection = ();
    type Duration = u16;

    async fn transmit(&mut self, packets: &[MHPacket<SIZE>]) -> Result<(), Self::Error> {
        let mut env = self.env.lock().unwrap();

        // Find all nodes that are in range of THIS transmitting node
        if let Some(neighbors) = env.topology.get(&self.node_id).cloned() {
            for neighbor_id in neighbors {
                // For every neighbor in range, push a clone of the packets into their inbox
                if let Some(inbox) = env.inboxes.get_mut(&neighbor_id) {
                    for pkt in packets {
                        inbox.push(pkt.clone());
                    }
                }
            }
        }
        Ok(())
    }

    async fn receive(
        &mut self,
        _conn: Self::Connection,
        _receiving_buffer: &[u8],
    ) -> Result<heapless::Vec<MHPacket<SIZE>, LEN>, Self::Error> {
        let mut env = self.env.lock().unwrap();
        let mut rec_vec: heapless::Vec<MHPacket<SIZE>, LEN> = heapless::Vec::new();

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
        Ok(rec_vec)
    }

    async fn listen(
        &mut self,
        _receiving_buffer: &mut [u8; SIZE],
        _with_timeout: bool,
    ) -> Result<Self::Connection, Self::Error> {
        Ok(())
    }
}

#[tokio::test]
async fn test_mesh_topology() {
    let env = Arc::new(Mutex::new(SimulationEnv::new()));
    let node_a = 1;
    let node_b = 2;
    let node_c = 3;
    let node_d = 4;

    {
        let mut e = env.lock().unwrap();
        // (A) <-> (B) <-> (C) <-> (D)
        e.add_bidi_link(node_a, node_b);
        e.add_bidi_link(node_b, node_c);
        e.add_bidi_link(node_c, node_d);
    }

    let mut router_a = MeshRouter::new(
        MockRadio {
            node_id: 1,
            env: env.clone(),
        },
        NetworkManager::<SIZE, LEN>::new(1, 5, 3),
    );

    let mut router_b = MeshRouter::new(
        MockRadio {
            node_id: 2,
            env: env.clone(),
        },
        NetworkManager::<SIZE, LEN>::new(2, 5, 3),
    );

    let mut router_c = MeshRouter::new(
        MockRadio {
            node_id: 3,
            env: env.clone(),
        },
        NetworkManager::<SIZE, LEN>::new(3, 5, 3),
    );

    let msg1 = Vec::from_slice(&[0x01]).unwrap();

    router_a.send_payload(msg1, 3).await.unwrap();
    assert_eq!(router_a.get_pending_count(), 1);

    // Node B now receives these
    let res1 = router_b.receive((), &[]).await.unwrap();
    // These packages were not meant for us, so we should not receive anything here
    assert_eq!(res1.len(), 0);
    // But router b should have send a new package, and have a pending ack
    assert_eq!(router_b.get_pending_count(), 1);
    // And shoul've also sent a package over the air, which router A can receive

    let res2 = router_a.receive((), &[]).await.unwrap();
    assert_eq!(res2.len(), 0);
    // And node A should've removed the package now
    assert_eq!(router_a.get_pending_count(), 0);

    // Router C should've also received it, and since this is for it, it receives the data
    let res3 = router_c.receive((), &[]).await.unwrap();
    assert_eq!(res3.len(), 1);
    // And does not send it on
    assert_eq!(router_c.get_pending_count(), 0);
}

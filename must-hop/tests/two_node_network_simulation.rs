use heapless::Vec;
use must_hop::node::{
    MHNode, MHPacket,
    mesh_router::MeshRouter,
    network_manager::{NetworkManager, NetworkManagerError},
    policy::NodePolicy,
};
use std::sync::{Arc, Mutex};

const SIZE: usize = 40;
const LEN: usize = 5;
struct MockRadio {
    air: Arc<Mutex<Vec<MHPacket<SIZE>, 12>>>,
}

impl MHNode<SIZE, LEN> for MockRadio {
    type Error = NetworkManagerError;
    type Connection = ();
    type ReceiveBuffer = ();
    type Duration = u16;

    async fn transmit(&mut self, packets: &[MHPacket<SIZE>]) -> Result<(), Self::Error> {
        {
            let mut vc = self.air.lock().unwrap();
            for pkt in packets {
                vc.push(pkt.clone()).unwrap();
            }
        }
        Ok(())
    }

    async fn receive(
        &mut self,
        _conn: Self::Connection,
        _receiving_buffer: &(),
    ) -> Result<Vec<MHPacket<SIZE>, LEN>, Self::Error> {
        let mut air = self.air.lock().unwrap();
        let mut rec_vec: Vec<MHPacket<SIZE>, LEN> = Vec::new();
        while !air.is_empty() {
            if rec_vec.is_full() {
                break;
            }
            rec_vec.push(air.remove(0)).unwrap();
        }
        Ok(rec_vec)
    }

    async fn listen(
        &mut self,
        _receiving_buffer: &mut (),
        _with_timeout: bool,
    ) -> Result<Self::Connection, Self::Error> {
        println!("listening!");
        Ok(())
    }
}

fn create_air() -> Arc<Mutex<Vec<MHPacket<SIZE>, 12>>> {
    Arc::new(Mutex::new(Vec::new()))
}

// #[tokio::test]
// async fn test_node_to_node_logic() {
//     let air = create_air();
//     let manager_a = NetworkManager::<SIZE, LEN>::new(1, 5, 3); // Source 1
//     let radio_a = MockRadio { air: air.clone() };
//     let mut router_a = MeshRouter::new(radio_a, manager_a, NodePolicy);
//     let msg_to_send = Vec::from_slice(&[0xAA, 0xBB]).unwrap();
//
//     // Let this be for node_b
//     router_a.send_payload(msg_to_send.clone(), 2).await.unwrap();
//     assert_eq!(router_a.get_pending_count(), 1);
//
//     // Now assume node B heard everything perfectly
//     let manager_b = NetworkManager::<SIZE, LEN>::new(2, 5, 3); // Source 1
//     let radio_b = MockRadio { air: air.clone() };
//     // radio_a.get_send_packets();
//     let mut router_b = MeshRouter::new(radio_b, manager_b, NodePolicy);
//     let rec_packets = msg_to_send;
//     // This returns list of packets for me, but more often that not, this will be empty in these
//     // tests. But in this scenario, we set destination as 2, which is node b!
//     let res = router_b.receive((), &rec_packets).await.unwrap();
//     assert_eq!(res.len(), 1);
// }

#[tokio::test]
async fn test_multiple_packets_fifo_order() {
    let air = create_air();
    let mut router_a = MeshRouter::new(
        MockRadio { air: air.clone() },
        NetworkManager::<SIZE, LEN>::new(1, 5, 3),
        NodePolicy,
    );
    let mut router_b = MeshRouter::new(
        MockRadio { air: air.clone() },
        NetworkManager::<SIZE, LEN>::new(2, 5, 3),
        NodePolicy,
    );

    let msg1 = Vec::from_slice(&[0x01]).unwrap();
    let msg2 = Vec::from_slice(&[0x02]).unwrap();
    let msg3 = Vec::from_slice(&[0x03]).unwrap();

    // 1. A sends three packets in sequence
    router_a.send_payload(msg1, 2).await.unwrap();
    assert_eq!(router_a.get_pending_count(), 1);
    router_a.send_payload(msg2, 2).await.unwrap();
    assert_eq!(router_a.get_pending_count(), 2);
    router_a.send_payload(msg3, 2).await.unwrap();
    assert_eq!(router_a.get_pending_count(), 3);

    // 2. B receives them. Should be in order 1 -> 2 -> 3

    // First receive
    let res1 = router_b.receive((), &()).await.unwrap();
    assert_eq!(router_b.get_pending_count(), 0);
    assert_eq!(res1.len(), 3);
    assert_eq!(res1[0].payload[0], 0x01, "Should receive msg1 first");

    assert_eq!(res1[1].payload[0], 0x02, "Should receive msg2 second");

    // Third receive
    assert_eq!(res1[2].payload[0], 0x03, "Should receive msg3 third");
}

#[tokio::test]
async fn test_send_and_ack() {
    let air = create_air();
    let mut router_a: MeshRouter<_, _, _, NodePolicy> = MeshRouter::new(
        MockRadio { air: air.clone() },
        NetworkManager::<SIZE, LEN>::new(1, 5, 3),
        NodePolicy,
    );
    let mut router_b: MeshRouter<_, _, _, NodePolicy> = MeshRouter::new(
        MockRadio { air: air.clone() },
        NetworkManager::<SIZE, LEN>::new(2, 5, 3),
        NodePolicy,
    );
    let msg1 = Vec::from_slice(&[0x01]).unwrap();
    // let msg2 = Vec::from_slice(&[0x02]).unwrap();
    // let msg3 = Vec::from_slice(&[0x03]).unwrap();

    router_a.send_payload(msg1, 3).await.unwrap();
    assert_eq!(router_a.get_pending_count(), 1);
    // router_a.send_payload(msg2, 3).await.unwrap();
    // assert_eq!(router_a.get_pending_count(), 2);
    // router_a.send_payload(msg3, 3).await.unwrap();
    // assert_eq!(router_a.get_pending_count(), 3);
    //
    // Node B now receives these
    let res1 = router_b.receive((), &()).await.unwrap();
    // These packages were not meant for us, so we should not receive anything here
    assert_eq!(res1.len(), 0);
    // But router b should have send a new package, and have a pending ack
    assert_eq!(router_b.get_pending_count(), 1);
    // And shoul've also sent a package over the air, which router A can receive

    let res2 = router_a.receive((), &()).await.unwrap();
    assert_eq!(res2.len(), 0);
    // And node A should've removed the package now
    assert_eq!(router_a.get_pending_count(), 0);
}

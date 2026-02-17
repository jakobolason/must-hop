use heapless::Vec;
use must_hop::node::{
    MHNode, MHPacket,
    mesh_router::MeshRouter,
    network_manager::{NetworkManager, NetworkManagerError},
};
use std::sync::Mutex;

const SIZE: usize = 128;
static AIR_PACKETS: Mutex<Vec<MHPacket<SIZE>, 12>> = Mutex::new(Vec::new());
struct MockRadio {}

impl MHNode<128> for MockRadio {
    type Error = NetworkManagerError;
    type Connection = ();
    type Duration = u16;

    async fn transmit(&mut self, packet: MHPacket<128>) -> Result<(), Self::Error> {
        AIR_PACKETS.lock().unwrap().push(packet).unwrap();
        Ok(())
    }

    async fn receive(
        &mut self,
        _conn: Self::Connection,
        _receiving_buffer: &[u8],
    ) -> Result<MHPacket<128>, Self::Error> {
        match AIR_PACKETS.lock().unwrap().pop() {
            Some(pkt) => Ok(pkt),
            None => Err(NetworkManagerError::InvalidPacket(0)),
        }
    }

    async fn listen(
        &mut self,
        _receiving_buffer: &mut [u8; SIZE],
        _with_timeout: bool,
    ) -> Result<Self::Connection, Self::Error> {
        println!("listening!");
        Ok(())
    }
}

#[tokio::test]
async fn test_node_to_node_logic() {
    let manager_a = NetworkManager::<128>::new(1, 5, 3); // Source 1
    let radio_a = MockRadio {};
    let mut router_a = MeshRouter::new(radio_a, manager_a);
    let msg_to_send = Vec::from_slice(&[0xAA, 0xBB]).unwrap();

    // Let this be for node_b
    router_a.send_payload(msg_to_send.clone(), 2).await.unwrap();

    // assert_eq!(radio_a.sent_packets.len(), 1);
    // let sent = &radio_a.sent_packets[0];
    // assert_eq!(sent.source_id, 1);
    // assert_eq!(sent.destination_id, 2);
    // assert_eq!(sent.payload[0], 0xAA);

    // Now assume node B heard everything perfectly
    let manager_b = NetworkManager::<128>::new(2, 5, 3); // Source 1
    let radio_b = MockRadio {};
    // radio_a.get_send_packets();
    let mut router_b = MeshRouter::new(radio_b, manager_b);
    let rec_packets = msg_to_send;
    // This returns list of packets for me, but more often that not, this will be empty in these
    // tests. But in this scenario, we set destination as 2, which is node b!
    let res = router_b.receive((), &rec_packets).await.unwrap();
    assert_eq!(res.len(), 1);
}

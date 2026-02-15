use core::future::Future;
use heapless::Vec;
use must_hop::node::{MHNode, MHPacket, NetworkManager, NetworkManagerError, PacketType};
use postcard::from_bytes;

struct MockRadio {
    pub sent_packets: std::vec::Vec<MHPacket>,
}

impl MockRadio {
    fn new() -> Self {
        Self {
            sent_packets: std::vec::Vec::new(),
        }
    }
    fn get_send_packets(self) -> std::vec::Vec<MHPacket> {
        self.sent_packets
    }
}

impl MHNode<128> for MockRadio {
    type Error = NetworkManagerError;
    type Connection = ();

    async fn transmit(&mut self, packet: MHPacket<128>) -> Result<(), Self::Error> {
        self.sent_packets.push(packet);
        Ok(())
    }

    async fn receive(
        &mut self,
        _conn: Self::Connection,
        receiving_buffer: &[u8],
    ) -> Result<MHPacket<128>, Self::Error> {
        from_bytes::<MHPacket>(receiving_buffer).map_err(|_| NetworkManagerError::InvalidPacket(0))
    }
}

#[tokio::test]
async fn test_node_to_node_logic() {
    let mut manager_a = NetworkManager::<128>::new(1, 5, 3); // Source 1
    let mut radio_a = MockRadio::new();
    let msg_to_send = &[0xAA, 0xBB];

    let packet = manager_a
        .new_packet(msg_to_send, 2, PacketType::Data)
        .unwrap();

    let packets_to_send = manager_a.send_packet(packet).unwrap();

    for p in packets_to_send {
        radio_a.transmit(p).await.unwrap();
    }

    assert_eq!(radio_a.sent_packets.len(), 1);
    let sent = &radio_a.sent_packets[0];
    assert_eq!(sent.source_id, 1);
    assert_eq!(sent.destination_id, 2);
    assert_eq!(sent.payload[0], 0xAA);

    // Now assume node B heard everything perfectly
    let mut manager_b = NetworkManager::<128>::new(1, 5, 3); // Source 1
    let mut radio_b = radio_a.get_send_packets();
}

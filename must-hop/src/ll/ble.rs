use crate::ll::LinkLayer;

/// Contains a link layer impl for BLE
///
/// struct for BLE
pub struct BleLL {}

impl LinkLayer for BleLL {
    fn send_ack(&mut self, packet_id: u16, destination: u16) {
        todo!()
    }

    fn send_packets(&mut self, packets: &[u8]) {
        todo!()
    }

    fn receive_packets(&mut self, packets: &[u8]) {
        todo!()
    }
}

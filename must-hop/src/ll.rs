pub mod ble;
/// The required functionality of the link layer, mainly consisting of handling a pending
/// acknowledgement buffer, sending ACK packets when told.
/// This sits inbetween the network and the MAC layer, ensure their responsibilites do
/// bleed into this layer. These traits define the contracts with which the layers can talk
/// together.
pub trait LinkLayer {
    /// The NWL should be able to require the transmission of ACKs, for a message that requires a
    /// reply, and where an echo-ACK is not possible
    fn send_ack(&mut self, packet_id: u16, destination: u16);

    /// Main functionality of LL, to send packets and give them to MAC. This should mean adding it to a
    /// `pend_ack` list,
    fn send_packets(&mut self, packets: &[u8]);

    /// Main functionality of LL, to receive packets and send onwards to NWL. This should check
    /// against `pend_ack`, but not do too much error correction.
    /// TODO:  A method of figuring out the fingerprint of a packets, such that it can be removed
    /// from `pend_ack` list.
    fn receive_packets(&mut self, packets: &[u8]);
}

use core::future::Future;
use heapless::Vec;
use serde::{Deserialize, Serialize};

/// MHPacket defines the package sent around the network
#[derive(Serialize, Deserialize, Debug, PartialEq, defmt::Format)]
pub struct MHPacket<const MAX_PACKET_SIZE: usize = 128> {
    /// Destination identifier
    // TODO: Perhaps bigger than u8?
    pub destination_id: u8,
    pub packet_type: PacketType,
    pub packet_id: u8,
    pub source_id: u8,
    /// Your specificed data wanting to send
    // (DE)serialize is only available up to 32 bytes
    pub payload: Vec<u8, MAX_PACKET_SIZE>,
    /// The amount of hops this package has been on
    // TODO: Implement logic for this
    pub hop_count: u8,
}

/// Either this node is ready to receive,
/// it has sent a package but has not heard a reply or
/// it has send a package and has gotten an ACK -> going back to receive
#[derive(Serialize, Deserialize, Debug, PartialEq, defmt::Format)]
pub enum PacketType {
    Data,
    Ack,
}

pub struct NetworkManager {}

/// Any radio wanting to be a node, has to be able to transmit and receive
pub trait MHNode<const N: usize> {
    type Error;
    type Connection;

    /// Takes an MHPacket with a size for the user defined payload. This will be sent to the
    /// appropriate destination_id
    fn transmit(&mut self, packet: MHPacket<N>) -> impl Future<Output = Result<(), Self::Error>>;

    /// Function needed for this lib, for multi hop communication.
    fn receive(
        &mut self,
        conn: Self::Connection,
        receiving_buffer: &[u8],
    ) -> impl Future<Output = Result<MHPacket, Self::Error>>;
}

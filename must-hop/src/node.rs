/// Provides the MHPacket, describing how a packet looks like on this network.
/// The MHNode describes necessary radio function for NM and MS to work. These should be
/// implemented by the radio used on the specific device
use core::future::Future;
use heapless::Vec;
use serde::{Deserialize, Serialize};

pub mod mesh_router;
pub mod network_manager;

/// Either this packet
/// Is Data, and should get an ACK return
/// A Data stream, meaning it wants to send multiple packets(u8 amount). In this case, Node B will
/// continue to listen, until it has receieved (u8) amount of packages
/// ACK should only be sent by a GW, because they will not retransmit
#[derive(Serialize, Deserialize, Debug, PartialEq, defmt::Format, Clone, Copy)]
pub enum PacketType {
    /// To send just a single packet
    Data,
    /// u8 denotes the amount of packages (UP TO 8)
    DataStream(u8),
    /// Payload should be bitmask of received packets
    Ack,
}

/// MHPacket defines the package sent around the network
#[derive(Serialize, Deserialize, Debug, PartialEq, defmt::Format, Clone)]
pub struct MHPacket<const MAX_PACKET_SIZE: usize> {
    /// Destination identifier
    // TODO: Perhaps bigger than u8?
    pub destination_id: u8,
    pub packet_type: PacketType,
    pub packet_id: u16,
    pub source_id: u8,
    /// Your specificed data wanting to send
    // (DE)serialize is only available up to 32 bytes
    pub payload: Vec<u8, MAX_PACKET_SIZE>,
    /// The amount of hops this package has been on
    // TODO: Implement logic for this
    pub hop_count: u8,
}

/// Any radio wanting to be a node, has to be able to transmit and receive
pub trait MHNode<const N: usize> {
    type Error;
    type Connection;
    type Duration;

    /// Takes an MHPacket with a size for the user defined payload. This will be sent to the
    /// appropriate destination_id
    fn transmit(&mut self, packet: MHPacket<N>) -> impl Future<Output = Result<(), Self::Error>>;

    /// Function needed for this lib, for multi hop communication.
    /// The conn and receiving_buffer might be too LoRa specific, so it might change
    fn receive(
        &mut self,
        conn: Self::Connection,
        receiving_buffer: &[u8],
    ) -> impl Future<Output = Result<MHPacket<N>, Self::Error>>;

    fn listen(
        &mut self,
        rec_buf: &mut [u8; N],
        with_timeout: bool,
    ) -> impl Future<Output = Result<Self::Connection, Self::Error>>;
}

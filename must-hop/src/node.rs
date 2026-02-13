use core::future::Future;
use defmt::trace;
use embassy_time::{Duration, Instant};
use heapless::Vec;
use lora_phy::mod_params::RadioError;
use postcard::{Error as PostError, to_slice};
use serde::{Deserialize, Serialize};

/// Either this packet
/// It Data, and should get an ACK return
/// A Data stream, meaning it wants to send multiple packets(u8 amount). In this case, Node B will
/// continue to listen, until it has receieved (u8) amount of packages
/// Or it is to ACK another node's Data packet
#[derive(Serialize, Deserialize, Debug, PartialEq, defmt::Format, Clone)]
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
pub struct MHPacket<const MAX_PACKET_SIZE: usize = 128> {
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

const MAX_AMOUNT_PACKETS: usize = 8;
/// Does not need to be serialized, because only MHPacket will be sent
#[derive(Debug, PartialEq, defmt::Format)]
pub struct PendingPacket {
    /// We keep the whole packet so it can be retransmitted
    packet: MHPacket,
    /// To know if a timeout has occurred
    timeout: Instant,
    /// And don't retry too many times
    retries: u8,
}

#[derive(Debug)]
pub enum NetworkManagerError {
    Hardware(RadioError),
    Serialization(PostError),
    Timeout,
    InvalidPacket(u16),
}

impl From<RadioError> for NetworkManagerError {
    fn from(err: RadioError) -> Self {
        NetworkManagerError::Hardware(err)
    }
}
impl From<PostError> for NetworkManagerError {
    fn from(err: PostError) -> Self {
        NetworkManagerError::Serialization(err)
    }
}

/// Logic to ensure packages arrive
pub struct NetworkManager<const MAX_PACKET_SIZE: usize = 128> {
    pending_acks: Vec<PendingPacket, MAX_PACKET_SIZE>,
    next_packet_id: u16,
    /// Configurations for the manager
    source_id: u8,
    timeout: u8,
    max_retries: u8,
}

impl<const MAX_PACKET_SIZE: usize> NetworkManager<MAX_PACKET_SIZE> {
    fn new_packet<T>(
        &mut self,
        payload: T,
        destination: u8,
        packet_type: PacketType,
    ) -> Result<MHPacket, PostError>
    where
        T: Serialize,
    {
        let mut buffer = [0u8; MAX_PACKET_SIZE];
        let used_slice = to_slice(&payload, &mut buffer)?;
        let payload_bytes = match Vec::from_slice(used_slice) {
            Ok(vec) => vec,
            Err(e) => {
                trace!("[ERROR] Capacity error: {:?}", e);
                return Err(PostError::SerializeBufferFull);
            }
        };
        self.next_packet_id += 1;
        Ok(MHPacket {
            destination_id: destination,
            packet_type: PacketType::Data,
            packet_id: self.next_packet_id,
            source_id: self.source_id,
            payload: payload_bytes,
            hop_count: 0,
        })
    }
    pub fn send_packet<T>(&mut self, payload: T, destination: u8) -> Result<(), NetworkManagerError>
    where
        T: Serialize,
    {
        let curr_time = Instant::now(); // + Instant::from_secs(self.timeout as u64);
        // Look into pending packages,
        let to_send: Vec<MHPacket, MAX_AMOUNT_PACKETS> = self
            .pending_acks
            .iter()
            .filter(|p| p.timeout > curr_time)
            .map(|p| p.packet.clone())
            .collect();
        let pkt_type = if to_send.is_empty() {
            PacketType::Data
        } else {
            PacketType::DataStream(to_send.len() as u8)
        };
        // first transform T into MHPacket
        // let pkt = self.new_packet(payload, destination)?;
        Ok(())
    }
}

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

use core::future::Future;
use defmt::trace;
use embassy_time::{Duration, Instant};
use heapless::Vec;
use lora_phy::mod_params::RadioError;
use postcard::{Error as PostError, to_slice};
use serde::{Deserialize, Serialize};

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

#[derive(Debug, defmt::Format)]
pub enum NetworkManagerError {
    Hardware(RadioError),
    Serialization(PostError),
    Timeout,
    InvalidPacket(u16),
    BufferFull,
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
    pub fn new(source_id: u8, timeout: u8, max_retries: u8) -> Self {
        Self {
            pending_acks: Vec::new(),
            next_packet_id: 0,
            source_id,
            timeout,
            max_retries,
        }
    }

    pub fn from_t<T>(&mut self, payload: T, destination: u8) -> Result<MHPacket, PostError>
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

    pub fn new_packet(
        &mut self,
        payload: &[u8],
        destination: u8,
        packet_type: PacketType,
    ) -> Result<MHPacket, PostError> {
        let payload_bytes = Vec::from_slice(payload).map_err(|_| PostError::SerializeBufferFull)?;
        self.next_packet_id += 1;
        Ok(MHPacket {
            destination_id: destination,
            packet_type,
            packet_id: self.next_packet_id,
            source_id: self.source_id,
            payload: payload_bytes,
            hop_count: 0,
        })
    }
    pub fn send_packet(
        &mut self,
        packet: MHPacket,
    ) -> Result<Vec<MHPacket, MAX_AMOUNT_PACKETS>, NetworkManagerError> {
        let curr_time = Instant::now(); // + Instant::from_secs(self.timeout as u64);
        let pkt_timout = curr_time + Duration::from_secs(self.timeout as u64);
        // First add this package to our vec
        let pend_pkt = PendingPacket {
            packet,
            timeout: pkt_timout,
            retries: 0,
        };
        if self.pending_acks.push(pend_pkt).is_err() {
            return Err(NetworkManagerError::BufferFull);
        }

        // Look into pending packages,
        let mut to_send: Vec<MHPacket, MAX_AMOUNT_PACKETS> = self
            .pending_acks
            .iter()
            .filter(|p| p.timeout > curr_time)
            // reserve a slot for payload
            .map(|p| p.packet.clone())
            .collect();

        let pkt_type = if to_send.len() == 1 {
            PacketType::Data
        } else {
            PacketType::DataStream(to_send.len() as u8)
        };
        for p in to_send.iter_mut() {
            p.packet_type = pkt_type;
        }
        Ok(to_send)
    }

    /// Manages actions which the pakcet might require from a network pov, and returns the packet
    /// if none are required, otherwise returns none
    pub fn receive_packet(
        &mut self,
        pkt: MHPacket,
    ) -> Result<Option<MHPacket>, NetworkManagerError> {
        // Check if it is one of our packets
        if let Some(our_packet_index) = self
            .pending_acks
            .iter()
            .position(|p| p.packet.packet_id == pkt.packet_id)
        {
            // Then remove it from our vec, and return
            self.pending_acks.remove(our_packet_index);
            return Ok(None);
        }
        // Perhaps it should be sent on?
        if pkt.source_id != self.source_id {
            self.send_packet(pkt);
            return Ok(None);
        }

        Ok(Some(pkt))
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

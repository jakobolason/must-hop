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
    // TODO: This should be more random, so each node doesn't start at 0
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
        // Look into packages with expired timeouts,
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
    /// The conn and receiving_buffer might be too LoRa specific, so it might change
    fn receive(
        &mut self,
        conn: Self::Connection,
        receiving_buffer: &[u8],
    ) -> impl Future<Output = Result<MHPacket, Self::Error>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    // A helper to make a dummy manager for testing
    fn setup_manager() -> NetworkManager<128> {
        NetworkManager::new(1, 10, 3) // Source ID 1, Timeout 10s, 3 Retries
    }

    #[test]
    fn test_packet_creation() {
        let mut manager = setup_manager();
        let payload = [0xAB, 0xCD];

        // Test basic packet creation
        let pkt = manager.new_packet(&payload, 2, PacketType::Data).unwrap();

        assert_eq!(pkt.source_id, 1);
        assert_eq!(pkt.destination_id, 2);
        assert_eq!(pkt.packet_id, 1);
        assert_eq!(pkt.payload, payload);
    }

    #[test]
    fn test_send_queue_logic() {
        let mut manager = setup_manager();
        let payload = [1, 2, 3];
        let pkt = manager.new_packet(&payload, 2, PacketType::Data).unwrap();

        // Calling send_packet should queue it and return it for sending
        let to_send = manager
            .send_packet(pkt.clone())
            .expect("Should queue packet");

        // 1. Check it returned the packet to be sent
        assert_eq!(to_send.len(), 1);
        assert_eq!(to_send[0].packet_id, 1);

        // 2. Check it is actually in the pending list (internal inspection)
        assert_eq!(manager.pending_acks.len(), 1);

        // 3. Receive the same packet back (simulating a loopback or re-forwarding)
        // If we receive a packet with Source != Self, we usually forward it.
        // But if we receive an ACK (logic you haven't fully implemented in snippet yet), we remove it.

        // For now, let's test the "BufferFull" error
        // for _ in 0..MAX_AMOUNT_PACKETS {
        //     let _ = manager.send_packet(pkt.clone());
        // }
        // Next one should fail
        // let res = manager.send_packet(pkt);
        // assert!(matches!(res, Err(NetworkManagerError::BufferFull)));
    }

    #[test]
    fn test_serialization_helper() {
        let mut manager = setup_manager();
        let data = [42u32];

        let pkt = manager.from_t(data, 5).expect("Serialization failed");

        // Verify payload contains the serialized bytes of 42u32 (Little Endian: 2A 00 00 00)
        assert_eq!(pkt.payload[0], 42);
    }
}

use super::{MHPacket, PacketType};
use defmt::{error, trace};
use embassy_time::{Duration, Instant};
use heapless::Vec;
use lora_phy::mod_params::RadioError;
use postcard::{Error as PostError, to_slice};
use serde::Serialize;

pub const MAX_AMOUNT_PACKETS: usize = 8;
/// Does not need to be serialized, because only MHPacket will be sent
#[derive(Debug, PartialEq, defmt::Format)]
pub struct PendingPacket<const SIZE: usize> {
    /// We keep the whole packet so it can be retransmitted
    packet: MHPacket<SIZE>,
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

#[derive(Debug, PartialEq)]
enum PayloadType {
    Data,
    Command,
}

/// Maintains record of packages sent, to ensure that they are received.
/// Also handles that packets from other nodes should be sent on
pub struct NetworkManager<const SIZE: usize = 128> {
    pending_acks: Vec<PendingPacket<SIZE>, SIZE>,
    // TODO: This should be more random, so each node doesn't start at 0
    next_packet_id: u16,
    /// Configurations for the manager
    source_id: u8,
    timeout: u8,
    max_retries: u8,
}

impl<const SIZE: usize> NetworkManager<SIZE> {
    pub fn new(source_id: u8, timeout: u8, max_retries: u8) -> Self {
        Self {
            pending_acks: Vec::new(),
            next_packet_id: 0,
            source_id,
            timeout,
            max_retries,
        }
    }

    pub fn from_t<T>(&mut self, payload: T, destination: u8) -> Result<MHPacket<SIZE>, PostError>
    where
        T: Serialize,
    {
        let mut buffer = [0u8; SIZE];
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
    ) -> Result<MHPacket<SIZE>, PostError> {
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

    // TODO: This is not a good function FIX
    pub fn payload_to_send(
        &mut self,
        payload: &[u8],
        destination: u8,
    ) -> Result<Vec<MHPacket<SIZE>, MAX_AMOUNT_PACKETS>, NetworkManagerError> {
        // Look into packages with expired timeouts,
        let pendings_len = self.pending_acks.len() as u8;
        let (mut to_send, pkt_type) = if pendings_len != 0 {
            let curr_time = Instant::now(); // + Instant::from_secs(self.timeout as u64);
            let pkt_type = PacketType::DataStream(pendings_len);
            let to_send: Vec<MHPacket<SIZE>, MAX_AMOUNT_PACKETS> = self
                .pending_acks
                .iter_mut()
                .filter(|p| p.timeout > curr_time)
                .map(|p| {
                    p.packet.packet_type = PacketType::DataStream(pendings_len);
                    p.packet.clone()
                })
                .collect();

            // if self.pending_acks.len() == MAX_AMOUNT_PACKETS {
            //     // return Err(NetworkManagerError::BufferFull);
            //     trace!("BUFFER IS FULL, so data is not being send");
            //     return Ok(to_send);
            // }
            (to_send, pkt_type)
        } else {
            (Vec::new(), PacketType::Data)
        };
        let new_pkt: MHPacket<SIZE> = self.new_packet(payload, destination, pkt_type)?;
        if to_send.push(new_pkt).is_err() {
            error!("Buffer was too full");
        }
        Ok(to_send)
    }

    pub fn add_packet(&mut self, packet: MHPacket<SIZE>) -> Result<(), NetworkManagerError> {
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
        Ok(())
    }

    /// Manages actions which the pakcet might require from a network pov, and returns the packet
    /// if none are required, otherwise returns none
    pub fn receive_packet(
        &mut self,
        pkt: MHPacket<SIZE>,
    ) -> Result<Option<(MHPacket<SIZE>, PayloadType)>, NetworkManagerError> {
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
            self.add_packet(pkt.clone())?;
            Ok(Some((pkt, PayloadType::Data)))
        } else {
            // If this is actually for us, then it is probably a command that the underlying app
            // wants, so this gives it back
            Ok(Some((pkt, PayloadType::Command)))
        }

        // Ok(Some(pkt))
    }

    pub fn handle_packets(
        &mut self,
        pkts: Vec<MHPacket<SIZE>, MAX_AMOUNT_PACKETS>,
    ) -> Result<
        (
            Vec<MHPacket<SIZE>, MAX_AMOUNT_PACKETS>,
            Vec<MHPacket<SIZE>, MAX_AMOUNT_PACKETS>,
        ),
        NetworkManagerError,
    > {
        let mut to_send: Vec<MHPacket<SIZE>, MAX_AMOUNT_PACKETS> = Vec::new();
        let mut commands: Vec<MHPacket<SIZE>, MAX_AMOUNT_PACKETS> = Vec::new();
        for pkt in pkts {
            let _ = match self.receive_packet(pkt) {
                Ok(Some(packet)) => match packet.1 {
                    PayloadType::Data => to_send
                        .push(packet.0)
                        .map_err(|e| error!("Error pusing to to_send: {:?}", e)),
                    PayloadType::Command => commands
                        .push(packet.0)
                        .map_err(|e| error!("Error pusing to commands: {:?}", e)),
                },
                Ok(None) => continue,
                Err(e) => {
                    error!("Error in managing packet: {:?}", e);
                    continue;
                }
            };
        }
        Ok((to_send, commands))
    }
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
            .receive_packet(pkt.clone())
            .expect("Should queue packet");
        assert!(to_send.is_some());
        let (to_send, payload_type) = to_send.unwrap();

        // 1. Check it returned the packet to be sent
        assert_eq!(to_send.packet_id, 1);
        assert_eq!(payload_type, PayloadType::Data);

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

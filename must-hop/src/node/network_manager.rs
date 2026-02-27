use super::{MHPacket, PacketType};
use core::cmp::{max, min};

#[cfg(not(feature = "in_std"))]
use defmt::{error, trace};
#[cfg(feature = "in_std")]
use log::{error, trace};

use embassy_time::{Duration, Instant};
use heapless::Vec;
use lora_phy::mod_params::RadioError;
use postcard::Error as PostError;

// pub const LEN: usize = 5;
/// Does not need to be serialized, because only MHPacket will be sent
#[derive(Debug, PartialEq)]
#[cfg_attr(not(feature = "in_std"), derive(defmt::Format))]
pub struct PendingPacket<const SIZE: usize> {
    /// We keep the whole packet so it can be retransmitted
    packet: MHPacket<SIZE>,
    /// To know if a timeout has occurred
    timeout: Instant,
    /// And don't retry too many times
    retries: u8,
}

#[derive(Debug)]
#[cfg_attr(not(feature = "in_std"), derive(defmt::Format))]
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

/// Ring buffer to hold recently ACK'ed messages, to avoid retransmitting them
pub struct RecentSeen<const N: usize> {
    buffer: [Option<(u8, u16)>; N],
    cursor: usize,
}

impl<const N: usize> RecentSeen<N> {
    pub const fn new() -> Self {
        Self {
            buffer: [None; N],
            cursor: 0,
        }
    }
    /// Takes tuple (source_id, packet_id)
    pub fn push(&mut self, pid: (u8, u16)) {
        self.buffer[self.cursor] = Some(pid);
        self.cursor = (self.cursor + 1) % N;
    }

    /// Checks if an entry matches (source_id, packet_id)
    pub fn contains(&self, pid: (u8, u16)) -> bool {
        self.buffer.contains(&Some(pid))
    }
}

impl<const N: usize> Default for RecentSeen<N> {
    fn default() -> Self {
        RecentSeen::new()
    }
}

#[derive(Debug, PartialEq)]
pub enum PayloadType {
    Data,
    Command,
    ACK,
    Bootup,
}

/// Maintains record of packages sent, to ensure that they are received.
/// Also handles that packets from other nodes should be sent on
pub struct NetworkManager<const SIZE: usize, const LEN: usize> {
    pending_acks: Vec<PendingPacket<SIZE>, LEN>,
    // TODO: This should be more random, so each node doesn't start at 0
    next_packet_id: u16,
    /// Uses the passed in LEN for a ring buffer
    recent_seen: RecentSeen<LEN>,
    /// Hops to gateway, handled by manager
    gw_hops: u8,
    /// Configurations for the manager
    source_id: u8,
    timeout: u8,
    _max_retries: u8,
}

impl<const SIZE: usize, const LEN: usize> NetworkManager<SIZE, LEN> {
    pub fn new(source_id: u8, timeout: u8, max_retries: u8) -> Self {
        Self {
            pending_acks: Vec::new(),
            next_packet_id: 0,
            recent_seen: RecentSeen::default(),
            // Default to max, only have a reasonable count if GW present
            gw_hops: 255,
            source_id,
            timeout,
            _max_retries: max_retries,
        }
    }

    pub fn new_packet(
        &mut self,
        payload: Vec<u8, SIZE>,
        destination: u8,
    ) -> Result<MHPacket<SIZE>, PostError> {
        // let payload_bytes = Vec::from_slice(payload).map_err(|_| PostError::SerializeBufferFull)?;
        self.next_packet_id += 1;
        Ok(MHPacket {
            destination_id: destination,
            packet_type: PacketType::Data,
            packet_id: self.next_packet_id,
            source_id: self.source_id,
            payload,
            hop_count: 0,
            hop_to_gw: self.gw_hops,
        })
    }

    #[doc(hidden)]
    pub fn get_pending_count(&self) -> usize {
        self.pending_acks.len()
    }

    /// This removes retried packets, and checks the pending acks list. Given the data payload in bytes, it is made into a MHPacket
    /// and added to internal acks list. It returns a list of packets to send, which includes the packet with the payload provided.
    /// But it also returns all packets which haven't been ACK'ed before it's timeout.
    pub fn payload_to_send(
        &mut self,
        payload: Vec<u8, SIZE>,
        destination: u8,
    ) -> Result<Vec<MHPacket<SIZE>, LEN>, NetworkManagerError> {
        // Clean up packets with too many retries
        // TODO: Shuold switch SF if this happens
        let curr_time = Instant::now();
        self.pending_acks
            .retain(|p| p.retries < self._max_retries || p.timeout < curr_time);

        // Look into packages with expired timeouts,
        let pendings_len = self.pending_acks.len() as u8;
        trace!("pendings len: {}", pendings_len);
        let mut to_send: Vec<MHPacket<SIZE>, LEN> = self
            .pending_acks
            .iter_mut()
            .filter(|p| p.timeout < curr_time)
            .map(|p| {
                p.retries += 1;
                p.packet.clone()
            })
            .collect();

        let new_pkt: MHPacket<SIZE> = self.new_packet(payload, destination)?;
        if to_send.push(new_pkt.clone()).is_err() {
            error!("Buffer was too full");
        } else {
            // NOTE: Only do this if buffer was not full, otherwise this just errors out
            // Now we add the new_pkt to pending_acks
            self.add_packet(new_pkt)?;
        }
        Ok(to_send)
    }

    /// Adds the packet to the internal list
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
    fn receive_packet(
        &mut self,
        pkt: MHPacket<SIZE>,
    ) -> Result<Option<(MHPacket<SIZE>, PayloadType)>, NetworkManagerError> {
        if pkt.packet_type == PacketType::BootUp {
            // TODO: What about GW failure/node failure, altering this?
            if pkt.hop_count >= self.gw_hops {
                // If incoming route has the same length, then discard this
                return Ok(None);
            }
            // GW sends 0, first node has 1 hop, therefore:
            self.gw_hops = pkt.hop_count + 1;
            // Add to recent seen, to compare later
            self.recent_seen.push((pkt.source_id, pkt.packet_id));
            // Fire and forget
            return Ok(Some((pkt, PayloadType::Bootup)));
        }
        // Check if it is one of our packets
        if let Some(our_packet_index) = self.pending_acks.iter().position(|p| {
            // shortcircuit here
            p.packet.packet_id == pkt.packet_id
                && (p.packet.source_id == pkt.source_id
                    || (pkt.packet_type == PacketType::Ack
            // TODO: Shouldn't this be flipped?
                        && pkt.destination_id == p.packet.source_id))
        }) {
            // Then remove it from our vec, and return
            trace!("RECEIVED KNOWN PACKAGE, REMOVING FROM LIST");
            self.pending_acks.remove(our_packet_index);
            // self.recent_seen.push((pkt.source_id, pkt.packet_id));
            return Ok(None);
        }
        // So we aren't waiting for pkt, perhaps we've seen it before?
        if self.recent_seen.contains((pkt.source_id, pkt.packet_id)) {
            // We do not ACK an ACK
            if pkt.packet_type == PacketType::Ack {
                return Ok(None);
            }
            // A duplicate which we should ACK, but not care about
            return Ok(Some((pkt, PayloadType::ACK)));
        }
        self.recent_seen.push((pkt.source_id, pkt.packet_id));

        // Perhaps it should be sent on?
        let to_us = pkt.destination_id == self.source_id;
        if !to_us {
            let is_gw_bound = pkt.destination_id == 1;
            let should_forward = if is_gw_bound {
                // Are we closer to GW?
                self.gw_hops < pkt.hop_to_gw
            } else {
                // Are we in between source and destination?
                (min(pkt.source_id, pkt.destination_id) <= self.source_id)
                    && (self.source_id <= max(pkt.destination_id, pkt.source_id))
            };

            if !should_forward {
                // If NOT, then we are not in the path of the packet, and do not rebroadcast
                return Ok(None);
            }
            let increased_gw_hops = {
                let mut temp = pkt.clone();
                temp.hop_to_gw = self.gw_hops;
                temp
            };
            self.add_packet(increased_gw_hops.clone())?;
            trace!("PACKAGE SHOULD BE SENT ON");
            Ok(Some((increased_gw_hops, PayloadType::Data)))
        } else {
            // If this is actually for us, then it is probably a command that the underlying app
            // wants, so this gives it back
            Ok(Some((pkt, PayloadType::Command)))
        }
    }

    /// To be used when receiving multiple packets, returns list of packets to send on, and the
    /// other list is a list of packets to the user
    pub fn handle_packets(
        &mut self,
        pkts: Vec<MHPacket<SIZE>, LEN>,
    ) -> Result<(Vec<MHPacket<SIZE>, LEN>, Vec<MHPacket<SIZE>, LEN>), NetworkManagerError> {
        let mut to_send: Vec<MHPacket<SIZE>, LEN> = Vec::new();
        let mut commands: Vec<MHPacket<SIZE>, LEN> = Vec::new();
        for pkt in pkts {
            let (packet, ptype) = match self.receive_packet(pkt) {
                Ok(Some(p)) => p,
                Ok(None) => continue,
                Err(e) => {
                    error!("Error in managing packet: {:?}", e);
                    continue;
                }
            };
            let err_closure = |e| {
                error!("Error pushing to commands: {:?}", e);
                NetworkManagerError::BufferFull
            };
            match ptype {
                PayloadType::Data => to_send.push(packet).map_err(err_closure)?,
                PayloadType::Command => commands.push(packet).map_err(err_closure)?,
                PayloadType::ACK => to_send
                    .push(MHPacket {
                        destination_id: packet.source_id,
                        packet_type: PacketType::Ack,
                        packet_id: packet.packet_id,
                        source_id: self.source_id,
                        payload: Vec::from_slice(&[0u8])
                            .map_err(|_| NetworkManagerError::BufferFull)?,
                        hop_count: 0,
                        hop_to_gw: self.gw_hops,
                    })
                    .map_err(err_closure)?,
                PayloadType::Bootup => to_send
                    .push(MHPacket {
                        destination_id: packet.destination_id,
                        packet_type: PacketType::BootUp,
                        packet_id: packet.packet_id,
                        source_id: self.source_id,
                        payload: Vec::from_slice(&[0u8])
                            .map_err(|_| NetworkManagerError::BufferFull)?,
                        hop_count: packet.hop_count + 1,
                        hop_to_gw: self.gw_hops,
                    })
                    .map_err(err_closure)?,
            };
        }
        Ok((to_send, commands))
    }

    pub fn handle_bootup(&mut self) -> Result<MHPacket<SIZE>, NetworkManagerError> {
        self.next_packet_id += 1;
        Ok(MHPacket {
            destination_id: 0, // broadcast id
            packet_type: PacketType::BootUp,
            packet_id: self.next_packet_id,
            source_id: self.source_id,
            payload: Vec::from_slice(&[]).map_err(|_| NetworkManagerError::BufferFull)?,
            hop_count: 0,
            hop_to_gw: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A helper to make a dummy manager for testing
    fn setup_manager() -> NetworkManager<40, 5> {
        NetworkManager::new(1, 10, 3) // Source ID 1, Timeout 10s, 3 Retries
    }

    #[test]
    fn test_packet_creation() {
        let mut manager = setup_manager();
        let payload = [0xAB, 0xCD];
        let vec = Vec::from_slice(&payload).expect("Could not get vec from slice");

        // Test basic packet creation
        let pkt = manager.new_packet(vec, 2).unwrap();

        assert_eq!(pkt.source_id, 1);
        assert_eq!(pkt.destination_id, 2);
        assert_eq!(pkt.packet_id, 1);
        assert_eq!(pkt.payload, payload);
    }

    #[test]
    fn test_send_queue_logic() {
        let mut manager = setup_manager();
        let payload = [1, 2, 3];
        let pkt = Vec::from_slice(&payload).unwrap();
        let pkt = manager.new_packet(pkt, 2).unwrap();

        // Calling send_packet should queue it and return it for sending
        let to_send = manager
            .receive_packet(pkt.clone())
            .expect("Should queue packet");
        assert!(to_send.is_some());
        let (to_send, payload_type) = to_send.unwrap();

        // Check it returned the packet to be sent
        assert_eq!(to_send.packet_id, 1);
        assert_eq!(payload_type, PayloadType::Data);

        // Check it is actually in the pending list
        assert_eq!(manager.pending_acks.len(), 1);

        // now act as if we transmitted this, and listened, and another node now transmits this.
        // That should mean the previous package gets removed from pendick acks
        let received = to_send;
        // Should be none, because we just received an ACK for a package we sent
        let should_be_none = manager.receive_packet(received).unwrap();

        assert_eq!(should_be_none, None);

        // 3. Receive the same packet back (simulating a loopback or re-forwarding)
        // If we receive a packet with Source != Self, we usually forward it.
        // But if we receive an ACK (logic you haven't fully implemented in snippet yet), we remove it.

        // For now, let's test the "BufferFull" error
        // for _ in 0..LEN {
        //     let _ = manager.send_packet(pkt.clone());
        // }
        // Next one should fail
        // let res = manager.send_packet(pkt);
        // assert!(matches!(res, Err(NetworkManagerError::BufferFull)));
    }
}

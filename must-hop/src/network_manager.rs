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

/// Internal pending-packet state (not serialized); used for retransmission and timeouts.
#[derive(Debug, PartialEq)]
#[cfg_attr(not(feature = "in_std"), derive(defmt::Format))]
struct PendingPacket<const SIZE: usize> {
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

/// Ring buffer to hold recently seen messages, to avoid retransmitting them
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
    HeartBeat,
}

/// Maintains record of packages sent, to ensure that they are received.
/// Also handles that packets from other nodes should be sent on
#[derive(Default)]
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
    pkts_sent: u32,
    pkts_acked: u32,
    pkts_lost: u32,
    pkts_retx: u32,
    _max_retries: u8,
}

impl<const SIZE: usize, const LEN: usize> NetworkManager<SIZE, LEN> {
    pub fn new(source_id: u8, timeout: u8, max_retries: u8) -> Self {
        Self {
            pending_acks: Vec::new(),
            recent_seen: RecentSeen::default(),
            // Default to max, only have a reasonable count if GW present
            gw_hops: 255,
            source_id,
            timeout,
            _max_retries: max_retries,
            ..Default::default()
        }
    }

    pub fn packet_loss_ratio(&self) -> f32 {
        if self.pkts_sent == 0 {
            0_f32
        } else {
            (self.pkts_sent as f32 - self.pkts_acked as f32) / (self.pkts_sent) as f32
        }
    }

    fn new_packet(
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

    pub fn get_pending_transmissions(&mut self) -> Vec<MHPacket<SIZE>, LEN> {
        self.get_pending_transmissions_at(Instant::now())
    }

    /// This removes retried packets, and checks the pending acks list. Given the data payload in bytes, it is made into a MHPacket
    /// and added to internal acks list. It returns a list of packets to send, which includes the packet with the payload provided.
    /// But it also returns all packets which haven't been ACK'ed before it's timeout.
    pub fn get_pending_transmissions_at(&mut self, curr_time: Instant) -> Vec<MHPacket<SIZE>, LEN> {
        // Clean up packets with too many retries
        // TODO: Shuold switch SF if this happens
        let prev_len = self.pending_acks.len();
        self.pending_acks.retain(
            |p| p.retries < self._max_retries, /*|| p.timeout < curr_time*/
        );
        self.pkts_lost += (prev_len - self.pending_acks.len()) as u32;

        // Look into packages with expired timeouts,
        // let pendings_len = self.pending_acks.len() as u8;
        // trace!("pendings len: {}", pendings_len);
        let to_send: Vec<MHPacket<SIZE>, LEN> = self
            .pending_acks
            .iter_mut()
            .filter(|p| p.timeout < curr_time)
            .map(|p| {
                p.retries += 1;
                p.timeout = curr_time + Duration::from_secs(self.timeout as u64);
                p.packet.clone()
            })
            .collect();

        self.pkts_retx += to_send.len() as u32;

        to_send
    }

    /// Adds the packet to the internal list
    fn add_packet(&mut self, packet: MHPacket<SIZE>) -> Result<(), NetworkManagerError> {
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
        // We [only] add it here, because this is called for queueing or receiving a packet
        self.pkts_sent += 1;
        Ok(())
    }

    pub fn queue_new_payload(
        &mut self,
        payload: Vec<u8, SIZE>,
        destination: u8,
    ) -> Result<MHPacket<SIZE>, NetworkManagerError> {
        let new_pkt = self.new_packet(payload, destination)?;
        self.add_packet(new_pkt.clone())?;
        Ok(new_pkt)
    }

    pub fn get_gw_hops(&self) -> u8 {
        self.gw_hops
    }

    fn check_pend_ack(&mut self, pkt: &MHPacket<SIZE>) -> bool {
        if let Some(our_packet_index) = self.pending_acks.iter().position(|p| {
            // shortcircuit here
            p.packet.packet_id == pkt.packet_id
                && (p.packet.source_id == pkt.source_id
                    || (pkt.packet_type == PacketType::Ack
            // TODO: Shouldn't this be flipped? I don't think so
                        && pkt.destination_id == p.packet.source_id))
        }) {
            // Then remove it from our vec, and return
            self.pkts_acked += 1;
            self.pending_acks.remove(our_packet_index);
            self.recent_seen.push((pkt.source_id, pkt.packet_id));
            return true;
        }
        false
    }

    /// Manages actions which the packet might require from a network pov, and returns the packet
    /// if none are required, otherwise returns none
    fn receive_packet(
        &mut self,
        pkt: MHPacket<SIZE>,
    ) -> Result<Option<(MHPacket<SIZE>, PayloadType)>, NetworkManagerError> {
        if pkt.source_id == self.source_id {
            // We return None no matter if its our own or not
            let _ = self.check_pend_ack(&pkt);
            return Ok(None);
        }
        if pkt.packet_type == PacketType::HeartBeat {
            // trace!("!!! RECEIVED A HEARTBEAT {:?} !!!", pkt);
            // TODO: What about GW failure/node failure, altering this?
            if pkt.hop_count >= self.gw_hops
                || self.recent_seen.contains((pkt.source_id, pkt.packet_id))
            {
                // If incoming route has the same length, then discard this
                return Ok(None);
            }
            // trace!("!!! SENDING HEARTBEAT ON {}", pkt.packet_id);
            // GW sends 0, first node has 1 hop, therefore:
            self.gw_hops = pkt.hop_count + 1;
            // Add to recent seen, to compare later
            self.recent_seen.push((pkt.source_id, pkt.packet_id));
            // Fire and forget
            // return Ok(Some((pkt, PayloadType::HeartBeat)));
            // This is handled by the MAC scheme
            return Ok(None);
        }
        // Check if it is one of our packets
        if self.check_pend_ack(&pkt) {
            return Ok(None);
        }
        // FIXME: This shouldn't be necessary
        // Never send an ACK on
        if pkt.packet_type == PacketType::Ack {
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
            // TODO: remove harcoded destination_id for GW
            let is_gw_bound = pkt.destination_id == 1;
            let should_forward = if is_gw_bound {
                // Are we closer to GW?
                self.gw_hops < pkt.hop_to_gw
            } else {
                // FIXME: This does not mean anything anymore, when we use DevEUI
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
            trace!(
                "PACKAGE SHOULD BE SENT ON, id: {}",
                increased_gw_hops.packet_id
            );
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
                PayloadType::Command => {
                    // Then we give it back to the app, but also send an ACK back to the sender
                    commands.push(packet.clone()).map_err(err_closure)?;
                    if packet.packet_type == PacketType::Data {
                        to_send
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
                            .map_err(err_closure)?
                    }
                }
                // Mainly here to stop repeating packets TODO: Check if this ever happens
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
                PayloadType::HeartBeat => to_send
                    .push(MHPacket {
                        destination_id: packet.destination_id,
                        packet_type: PacketType::HeartBeat,
                        packet_id: packet.packet_id,
                        source_id: self.source_id,
                        payload: packet.payload,
                        hop_count: packet.hop_count + 1,
                        hop_to_gw: self.gw_hops,
                    })
                    .map_err(err_closure)?,
            };
        }
        Ok((to_send, commands))
    }

    pub fn add_heartbeat(&mut self) -> Result<MHPacket<SIZE>, NetworkManagerError> {
        self.next_packet_id += 1;
        self.recent_seen.push((self.source_id, self.next_packet_id));
        // trace!(
        //     "----------- Sending Heartbeat with packet id: {} -------------",
        //     self.next_packet_id
        // );
        // If we are calling this, then we are a GW
        // self.gw_hops = 0;
        Ok(MHPacket {
            destination_id: 0, // broadcast id
            packet_type: PacketType::HeartBeat,
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
    use heapless::Vec;

    // A helper to make a dummy manager for testing
    fn setup_manager() -> NetworkManager<40, 5> {
        NetworkManager::new(2, 10, 3)
    }

    #[test]
    fn packet_creation() {
        let mut manager = setup_manager();
        let payload = [0xAB, 0xCD];
        let vec = Vec::from_slice(&payload).expect("Could not get vec from slice");

        let pkt = manager.new_packet(vec, 2).unwrap();

        assert_eq!(pkt.source_id, 2);
        assert_eq!(pkt.destination_id, 2);
        assert_eq!(pkt.packet_id, 1);
        assert_eq!(pkt.payload, payload);
    }

    #[test]
    fn queueing_and_ack_handling() {
        let mut manager = setup_manager();
        let payload = Vec::from_slice(&[1, 2, 3]).unwrap();

        // Queue a new packet bound for node 3
        manager
            .queue_new_payload(payload, 3)
            .expect("Should queue payload");

        // It should now be in the pending list awaiting an ACK
        assert_eq!(manager.get_pending_count(), 1);
        // It has registered a sent packet
        assert_eq!(manager.pkts_sent, 1);
        assert_eq!(manager.pkts_acked, 0);
        // Simulate receiving an ACK from node 2
        let ack_pkt = MHPacket {
            destination_id: 2, // Back to us
            packet_type: PacketType::Ack,
            packet_id: 1, // Matches the ID of the packet we just sent
            source_id: 3, // From the node we sent it to
            payload: Vec::from_slice(&[0]).unwrap(),
            hop_count: 1,
            hop_to_gw: 5,
        };

        // Process the ACK
        let result = manager
            .receive_packet(ack_pkt)
            .expect("Should process packet");

        // The manager should consume the ACK and return None
        assert!(result.is_none());

        assert_eq!(manager.pkts_acked, 1);
        assert_eq!(manager.pkts_lost, 0);
        assert_eq!(manager.pkts_retx, 0);

        // Our pending list should now be empty because the packet was ACK'd
        assert_eq!(manager.get_pending_count(), 0);
    }

    #[test]
    fn forwarding_logic() {
        let mut manager = setup_manager();
        // Simulate that we are 2 hops away from the gateway (GW is ID 1)
        manager.gw_hops = 2;
        assert_eq!(manager.pkts_sent, 0);

        // Simulate a packet coming from node 3, bound for the GW
        let incoming_pkt = MHPacket {
            destination_id: 1, // GW Bound
            packet_type: PacketType::Data,
            packet_id: 42,
            source_id: 3,
            payload: Vec::from_slice(&[9, 9]).unwrap(),
            hop_count: 1,
            hop_to_gw: 4, // Node 3 thinks it is 4 hops away. We are 2 hops away, so we should forward.
        };

        let result = manager
            .receive_packet(incoming_pkt)
            .expect("Should process packet");

        // We expect the manager to modify the packet and tell us to send it on
        assert!(result.is_some());
        let (forward_pkt, payload_type) = result.unwrap();

        assert_eq!(payload_type, PayloadType::Data);
        // It should update the hop_to_gw to our current knowledge (2)
        assert_eq!(forward_pkt.hop_to_gw, 2);

        assert_eq!(manager.pkts_sent, 1);
        assert_eq!(manager.pkts_acked, 0);
        assert_eq!(manager.pkts_lost, 0);
        assert_eq!(manager.pkts_retx, 0);

        // It should also add this to pending_acks since we are forwarding it and expect an ACK
        assert_eq!(manager.get_pending_count(), 1);
    }

    #[test]
    fn packet_loss_ratio() {
        // use timeout of 5 to test retx out
        let mut manager = NetworkManager::<40, 5>::new(4, 0, 5);
        let payload = Vec::from_slice(&[1, 2, 3]).unwrap();
        manager.gw_hops = 2;

        // Queue a new packet bound for node 2
        manager
            .queue_new_payload(payload, 1)
            .expect("Should queue payload");

        // It should now be in the pending list awaiting an ACK
        assert_eq!(manager.get_pending_count(), 1);
        // It has registered a sent packet
        assert_eq!(manager.pkts_sent, 1);
        assert_eq!(manager.pkts_acked, 0);
        // Simulate receiving an ACK from node 2
        let ack_pkt = MHPacket {
            destination_id: 1, // Back to us
            packet_type: PacketType::Data,
            packet_id: 1, // Matches the ID of the packet we just sent
            source_id: 4, // From the node we sent it to
            payload: Vec::from_slice(&[0]).unwrap(),
            hop_count: 1,
            hop_to_gw: 1,
        };

        // Process the ACK
        let result = manager
            .receive_packet(ack_pkt)
            .expect("Should process packet");

        // The manager should consume the ACK and return None
        assert!(result.is_none());
        assert_eq!(manager.get_pending_count(), 0);
        assert_eq!(manager.pkts_acked, 1);

        // now simulate another packet coming downstream
        let incoming_pkt = MHPacket {
            destination_id: 1, // GW Bound
            packet_type: PacketType::Data,
            packet_id: 42,
            source_id: 3,
            payload: Vec::from_slice(&[9, 9]).unwrap(),
            hop_count: 1,
            hop_to_gw: 3, // Node 3 thinks it is 4 hops away. We are 2 hops away, so we should forward.
        };

        let result = manager
            .receive_packet(incoming_pkt)
            .expect("Should process packet");
        assert!(result.is_some());
        assert_eq!(manager.pkts_acked, 1);
        assert_eq!(manager.pkts_lost, 0);
        assert_eq!(manager.pkts_retx, 0);
        assert_eq!(manager.pkts_sent, 2);
        assert_eq!(manager.get_pending_count(), 1);

        // SCENARIO: Upstream neighbour doesn't get relayed packet
        let res = manager.get_pending_transmissions_at(Instant::now() + Duration::from_millis(1));
        // should need a retransmission here
        assert_eq!(res.len(), 1);
        assert_eq!(manager.pkts_retx, 1);
        assert_eq!(manager.pkts_lost, 0);
        assert_eq!(manager.pkts_sent, 2);
        assert_eq!(manager.pkts_acked, 1);

        // And now, upstream neighbour hears us
        let ack_pkt = MHPacket {
            destination_id: 1, // Back to us
            packet_type: PacketType::Data,
            packet_id: 42, // Matches the ID of the packet we just sent
            source_id: 3,  // From the node we sent it to
            payload: Vec::from_slice(&[0]).unwrap(),
            hop_count: 1,
            hop_to_gw: 1,
        };
        let result = manager
            .receive_packet(ack_pkt)
            .expect("Should process packet");

        // The manager should consume the ACK and return None
        assert!(result.is_none());
        assert_eq!(manager.pkts_acked, 2);
        assert_eq!(manager.get_pending_count(), 0);
        let pkt_loss = (manager.pkts_acked as f32)
            / (manager.pkts_sent + manager.pkts_retx + manager.pkts_lost) as f32;
        assert_eq!(pkt_loss, 2.0 / 3.0);
    }
}

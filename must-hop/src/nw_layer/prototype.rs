use heapless::Vec;

use crate::{
    MeshPacket, PacketType,
    log_compat::{error, trace, warn},
    nw_layer::{NetworkError, NetworkLayer, RxOutcome, TransmitBuffer},
};

const RECENT_SEEN_SIZE: usize = 10;

pub const GW_SRC_ID: u16 = 1;
pub const BROADCAST_ID: u16 = 0;

/// Ring buffer to hold recently seen messages, to avoid retransmitting them
pub struct RecentSeen<const N: usize> {
    buffer: [Option<(u16, u16)>; N],
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
    pub fn push(&mut self, pid: (u16, u16)) {
        self.buffer[self.cursor] = Some(pid);
        self.cursor = (self.cursor + 1) % N;
    }

    /// Checks if an entry matches (source_id, packet_id)
    pub fn contains(&self, pid: (u16, u16)) -> bool {
        self.buffer.contains(&Some(pid))
    }
}

impl<const N: usize> Default for RecentSeen<N> {
    fn default() -> Self {
        RecentSeen::new()
    }
}

/// RPL-inspired routing prototype. Supports two modes only:
/// 1. Upstream unicast to the gateway, using the `gw_hops` gradient.
/// 2. Broadcast flood (destination_id == BROADCAST_ID), with (source_id, packet_id) dedup.
///
/// Peer-to-peer routing between arbitrary non-sink nodes is intentionally not
/// supported here; the old `NetworkManager`'s `source_id < self < dest` heuristic
/// baked topology into numeric IDs and does not generalize.
pub struct GradientRouter<const SIZE: usize> {
    recent_seen: RecentSeen<RECENT_SEEN_SIZE>,
    source_id: u16,
    next_packet_id: u16,
    gw_hops: u8,
    is_gateway: bool,
}

impl<const SIZE: usize> GradientRouter<SIZE> {
    pub fn new_node(source_id: u16) -> Self {
        debug_assert!(
            source_id != BROADCAST_ID && source_id != GW_SRC_ID,
            "source_id must not collide with BROADCAST_ID or GW_SRC_ID"
        );
        Self {
            recent_seen: RecentSeen::new(),
            source_id,
            next_packet_id: 0,
            gw_hops: u8::MAX,
            is_gateway: false,
        }
    }

    pub fn new_gateway() -> Self {
        Self {
            recent_seen: RecentSeen::new(),
            source_id: GW_SRC_ID,
            next_packet_id: 0,
            gw_hops: 0,
            is_gateway: true,
        }
    }

    pub fn source_id(&self) -> u16 {
        self.source_id
    }

    fn next_id(&mut self) -> u16 {
        // TODO: should be seeded randomly so colliding IDs across nodes are less likely
        self.next_packet_id = self.next_packet_id.wrapping_add(1);
        self.next_packet_id
    }

    fn rewrite_hop(&self, pkt: MeshPacket<SIZE>) -> MeshPacket<SIZE> {
        MeshPacket {
            hop_to_gw: self.gw_hops,
            ..pkt
        }
    }
}

impl<const SIZE: usize> NetworkLayer<SIZE> for GradientRouter<SIZE> {
    /// Handle payload, ensure it is set to the proper direction. The router should then give the
    /// packet to the link layer
    fn queue_payload<const CAP: usize>(
        &mut self,
        destination: u16,
        payload: &[u8],
        buffer: &mut TransmitBuffer<SIZE, CAP>,
    ) -> Result<(), NetworkError> {
        let payload = Vec::from_slice(payload).map_err(|_| NetworkError::InvalidPayload)?;
        let pkt = MeshPacket {
            destination_id: destination,
            packet_type: PacketType::Data,
            packet_id: self.next_id(),
            source_id: self.source_id,
            payload,
            hop_count: 0,
            hop_to_gw: self.gw_hops,
        };
        // Remember our own emission so a looped-back copy doesn't re-enter processing
        self.recent_seen.push((pkt.source_id, pkt.packet_id));
        buffer.push(pkt).map_err(|_| NetworkError::Full)
    }

    /// Receives the deserialized packets from the link layer, and handle packet
    /// forwarding, and figuring out if it should be returned to the application layer
    fn receive(&mut self, pkt: MeshPacket<SIZE>) -> RxOutcome<SIZE> {
        // Our own emission looped back
        if pkt.source_id == self.source_id {
            warn!("Received message from myself");
            return RxOutcome::empty();
        }

        // We should never receive an ACK at this point, should be resolved by the link layer
        if pkt.packet_type == PacketType::Ack {
            error!("Received ACK at network layer!!");
            return RxOutcome::empty();
        }

        // Heartbeats maintain the gw_hops gradient. First-seen heartbeats with a strictly
        // better route are adopted and flooded; equal/worse routes are dropped.
        if pkt.packet_type == PacketType::HeartBeat {
            if self.recent_seen.contains((pkt.source_id, pkt.packet_id)) {
                return RxOutcome::empty();
            }
            self.recent_seen.push((pkt.source_id, pkt.packet_id));

            if pkt.hop_to_gw >= self.gw_hops {
                return RxOutcome::empty();
            }

            // GW sends 0, first node has 1 hop, therefore:
            self.gw_hops = pkt.hop_to_gw.saturating_add(1);
            trace!("gw_hops updated to {}", self.gw_hops);

            return RxOutcome {
                forward: Some(self.rewrite_hop(pkt)),
                deliver: None,
                ack: None,
            };
        }

        let for_us = pkt.destination_id == self.source_id
            || pkt.destination_id == BROADCAST_ID
        // NOTE: Why should the first check not be the same as the last one here?
            || (self.is_gateway && pkt.destination_id == GW_SRC_ID);

        // Duplicate suppression. If this was already seen AND it was meant for us, our ACK
        // might not have reached the sender, so re-emit the ACK. For non-us duplicates we
        // stay silent — we already forwarded the first copy.
        if self.recent_seen.contains((pkt.source_id, pkt.packet_id)) {
            // They might not have received the ACK
            if for_us && pkt.packet_type == PacketType::Data {
                return RxOutcome {
                    forward: None,
                    deliver: None,
                    ack: Some((pkt.packet_id, pkt.source_id)),
                };
            }
            return RxOutcome::empty();
        }
        self.recent_seen.push((pkt.source_id, pkt.packet_id));

        // Broadcast: both deliver locally and re-emit once
        if pkt.destination_id == BROADCAST_ID {
            let deliver = pkt.clone();
            return RxOutcome {
                forward: Some(self.rewrite_hop(pkt)),
                deliver: Some(deliver),
                ack: None,
            };
        }

        let ack_if_data = |pkt: &MeshPacket<SIZE>| {
            (pkt.packet_type == PacketType::Data).then_some((pkt.packet_id, pkt.source_id))
        };

        // Unicast addressed to us, OR a GW-bound packet reaching the gateway itself
        if pkt.destination_id == self.source_id
            || (self.is_gateway && pkt.destination_id == GW_SRC_ID)
        {
            let ack = ack_if_data(&pkt);
            return RxOutcome {
                forward: None,
                deliver: Some(pkt),
                ack,
            };
        }

        // Upstream forwarding: only if we are strictly closer to the GW
        if pkt.destination_id == GW_SRC_ID && self.gw_hops < pkt.hop_to_gw {
            return RxOutcome {
                forward: Some(self.rewrite_hop(pkt)),
                deliver: None,
                ack: None,
            };
        }

        // Not for us, not on an upward path we can improve — drop
        RxOutcome::empty()
    }

    fn build_heartbeat(&mut self) -> Option<MeshPacket<SIZE>> {
        if !self.is_gateway {
            return None;
        }
        let packet_id = self.next_id();
        self.recent_seen.push((self.source_id, packet_id));
        Some(MeshPacket {
            destination_id: BROADCAST_ID,
            packet_type: PacketType::HeartBeat,
            packet_id,
            source_id: self.source_id,
            payload: Vec::new(),
            hop_count: 0,
            hop_to_gw: self.gw_hops,
        })
    }

    fn gw_hops(&self) -> u8 {
        self.gw_hops
    }

    fn build_ack(&self, packet_id: u16, destination: u16) -> MeshPacket<SIZE> {
        MeshPacket {
            destination_id: destination,
            packet_type: PacketType::Ack,
            packet_id,
            source_id: self.source_id,
            payload: Vec::new(),
            hop_count: 0,
            hop_to_gw: self.gw_hops,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: usize = 32;
    const CAP: usize = 4;

    fn dg(src: u16, dst: u16, id: u16, hops: u8, ty: PacketType) -> MeshPacket<SIZE> {
        MeshPacket {
            source_id: src,
            destination_id: dst,
            packet_id: id,
            packet_type: ty,
            payload: Vec::new(),
            hop_count: 0,
            hop_to_gw: hops,
        }
    }

    #[test]
    fn queue_payload_builds_packet_and_marks_seen() {
        let mut router: GradientRouter<SIZE> = GradientRouter::new_node(3);
        let mut buf: TransmitBuffer<SIZE, CAP> = TransmitBuffer::new();

        router
            .queue_payload(GW_SRC_ID, &[1, 2, 3], &mut buf)
            .unwrap();

        assert_eq!(buf.len(), 1);
        let out = buf.pop().unwrap();
        assert_eq!(out.source_id, 3);
        assert_eq!(out.destination_id, GW_SRC_ID);
        assert_eq!(out.packet_id, 1);
        assert_eq!(&out.payload[..], &[1, 2, 3]);
        assert!(router.recent_seen.contains((3, 1)));
    }

    #[test]
    fn heartbeat_lowers_gw_hops_and_forwards() {
        let mut router: GradientRouter<SIZE> = GradientRouter::new_node(3);
        assert_eq!(router.gw_hops(), u8::MAX);

        let hb = dg(GW_SRC_ID, BROADCAST_ID, 42, 0, PacketType::HeartBeat);
        let out = router.receive(hb);

        assert_eq!(router.gw_hops(), 1);
        let fwd = out.forward.expect("should flood heartbeat");
        assert_eq!(fwd.hop_to_gw, 1);
        assert!(out.deliver.is_none());
    }

    #[test]
    fn heartbeat_worse_route_is_ignored() {
        let mut router: GradientRouter<SIZE> = GradientRouter::new_node(3);
        router.gw_hops = 2;

        let hb = dg(5, BROADCAST_ID, 7, 5, PacketType::HeartBeat);
        let out = router.receive(hb);

        assert_eq!(router.gw_hops(), 2);
        assert!(out.forward.is_none());
    }

    #[test]
    fn gw_bound_data_forwards_when_we_are_closer() {
        let mut router: GradientRouter<SIZE> = GradientRouter::new_node(3);
        router.gw_hops = 2;

        let pkt = dg(5, GW_SRC_ID, 10, 4, PacketType::Data);
        let out = router.receive(pkt);

        let fwd = out.forward.expect("should forward");
        assert_eq!(fwd.hop_to_gw, 2);
        assert!(out.deliver.is_none());
    }

    #[test]
    fn gw_bound_data_dropped_when_we_are_farther() {
        let mut router: GradientRouter<SIZE> = GradientRouter::new_node(3);
        router.gw_hops = 5;

        let pkt = dg(5, GW_SRC_ID, 10, 3, PacketType::Data);
        let out = router.receive(pkt);

        assert!(out.forward.is_none());
        assert!(out.deliver.is_none());
    }

    #[test]
    fn broadcast_delivers_and_forwards() {
        let mut router: GradientRouter<SIZE> = GradientRouter::new_node(3);
        router.gw_hops = 2;

        let pkt = dg(5, BROADCAST_ID, 11, 4, PacketType::Data);
        let out = router.receive(pkt);

        assert!(out.forward.is_some());
        assert!(out.deliver.is_some());
        assert_eq!(out.forward.unwrap().hop_to_gw, 2);
    }

    #[test]
    fn unicast_to_us_delivers_with_ack() {
        let mut router: GradientRouter<SIZE> = GradientRouter::new_node(3);

        let pkt = dg(5, 3, 12, 4, PacketType::Data);
        let out = router.receive(pkt);

        assert!(out.deliver.is_some());
        assert_eq!(out.ack, Some((12, 5)));
    }

    #[test]
    fn duplicate_data_to_us_re_acks() {
        let mut router: GradientRouter<SIZE> = GradientRouter::new_node(3);
        let pkt = dg(5, 3, 12, 4, PacketType::Data);

        let first = router.receive(pkt.clone());
        assert!(first.deliver.is_some());
        assert_eq!(first.ack, Some((12, 5)));

        // Second time: sender didn't see our ACK and retried
        let second = router.receive(pkt);
        assert!(second.deliver.is_none());
        assert_eq!(second.ack, Some((12, 5)));
    }

    #[test]
    fn duplicate_forwarded_packet_is_dropped() {
        let mut router: GradientRouter<SIZE> = GradientRouter::new_node(3);
        router.gw_hops = 2;

        let pkt = dg(5, GW_SRC_ID, 10, 4, PacketType::Data);
        let out1 = router.receive(pkt.clone());
        let out2 = router.receive(pkt);

        assert!(out1.forward.is_some());
        assert!(out2.forward.is_none());
        assert!(out2.ack.is_none());
    }

    #[test]
    fn from_self_is_dropped() {
        let mut router: GradientRouter<SIZE> = GradientRouter::new_node(3);

        let pkt = dg(3, GW_SRC_ID, 10, 4, PacketType::Data);
        let out = router.receive(pkt);

        assert!(out.forward.is_none());
        assert!(out.deliver.is_none());
    }

    #[test]
    fn gateway_delivers_gw_bound_data() {
        let mut router: GradientRouter<SIZE> = GradientRouter::new_gateway();

        let pkt = dg(5, GW_SRC_ID, 10, 4, PacketType::Data);
        let out = router.receive(pkt);

        assert!(out.deliver.is_some());
        assert_eq!(out.ack, Some((10, 5)));
    }

    #[test]
    fn gateway_builds_heartbeat_but_node_does_not() {
        let mut gw: GradientRouter<SIZE> = GradientRouter::new_gateway();
        let hb = gw.build_heartbeat().expect("gateway emits heartbeats");
        assert_eq!(hb.packet_type, PacketType::HeartBeat);
        assert_eq!(hb.source_id, GW_SRC_ID);
        assert_eq!(hb.destination_id, BROADCAST_ID);
        assert_eq!(hb.hop_to_gw, 0);

        let mut node: GradientRouter<SIZE> = GradientRouter::new_node(3);
        assert!(node.build_heartbeat().is_none());
    }
}

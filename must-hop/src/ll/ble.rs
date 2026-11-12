//! BLE link layer: owns the pending-acks table and retransmission logic.
//!
//! - [`BleLL::on_tx`] records every ACK-able outgoing packet with a timeout
//!   and a retry count.
//! - [`BleLL::on_rx`] consumes echoes / explicit ACKs that close out pending
//!   entries.
//! - [`BleLL::poll_retransmissions`] clones timed-out entries back into the
//!   caller's transmit buffer and bumps their retry count; entries that
//!   exceed `max_retries` are dropped as lost.

use embassy_time::{Duration, Instant};
use heapless::Vec;

use crate::ll::{LinkLayer, LinkVerdict};
use crate::nw_layer::TransmitBuffer;
use crate::{MeshPacket, PacketType};

/// One in-flight packet awaiting ACK.
#[derive(Debug, PartialEq)]
#[cfg_attr(not(feature = "in_std"), derive(defmt::Format))]
struct PendingPacket<const SIZE: usize> {
    /// Full packet kept so it can be retransmitted without reconstructing it
    packet: MeshPacket<SIZE>,
    /// When the next retransmit becomes due
    timeout: Instant,
    /// How many times we've already retransmitted it
    retries: u8,
}

/// BLE link-layer implementation.
///
/// Tracks ACK-able outgoing packets, consumes echoes / explicit ACKs on the
/// return path, and re-emits timed-out packets up to `max_retries` times.
///
/// Serialization and raw radio I/O live in the `MHNode` impl; this struct
/// only operates on typed packets.
pub struct BleLL<const SIZE: usize, const LEN: usize> {
    pending_acks: Vec<PendingPacket<SIZE>, LEN>,
    timeout_ms: u32,
    max_retries: u8,
    #[cfg(feature = "ll-stats")]
    pkts_sent: u32,
    #[cfg(feature = "ll-stats")]
    pkts_acked: u32,
    #[cfg(feature = "ll-stats")]
    pkts_lost: u32,
    #[cfg(feature = "ll-stats")]
    pkts_retx: u32,
}

impl<const SIZE: usize, const LEN: usize> BleLL<SIZE, LEN> {
    pub fn new(timeout_ms: u32, max_retries: u8) -> Self {
        Self {
            pending_acks: Vec::new(),
            timeout_ms,
            max_retries,
            #[cfg(feature = "ll-stats")]
            pkts_sent: 0,
            #[cfg(feature = "ll-stats")]
            pkts_acked: 0,
            #[cfg(feature = "ll-stats")]
            pkts_lost: 0,
            #[cfg(feature = "ll-stats")]
            pkts_retx: 0,
        }
    }

    #[doc(hidden)]
    pub fn pending_count(&self) -> usize {
        self.pending_acks.len()
    }

    /// Ratio of "attempts that did not result in an ACK" over all attempts
    /// (first transmits + retransmits + abandoned).
    #[cfg(feature = "ll-stats")]
    pub fn packet_loss_ratio(&self) -> f32 {
        let total = self.pkts_sent + self.pkts_retx + self.pkts_lost;
        if total == 0 {
            0.0
        } else {
            (total as f32 - self.pkts_acked as f32) / total as f32
        }
    }

    /// Only Data / Data-stream style packets need tracking. ACKs are never
    /// ACKed; heartbeats are periodic and should not retransmit.
    fn should_track(pkt: &MeshPacket<SIZE>) -> bool {
        !matches!(pkt.packet_type, PacketType::Ack | PacketType::HeartBeat)
    }

    fn matches_pending(pending: &MeshPacket<SIZE>, incoming: &MeshPacket<SIZE>) -> bool {
        if pending.packet_id != incoming.packet_id {
            return false;
        }
        pending.source_id == incoming.source_id
            || (incoming.packet_type == PacketType::Ack
                && incoming.destination_id == pending.source_id)
    }
}

impl<const SIZE: usize, const LEN: usize> LinkLayer<SIZE> for BleLL<SIZE, LEN> {
    fn on_tx(&mut self, pkt: &MeshPacket<SIZE>) {
        if !Self::should_track(pkt) {
            return;
        }
        let pend = PendingPacket {
            packet: pkt.clone(),
            timeout: Instant::now() + Duration::from_millis(self.timeout_ms as u64),
            retries: 0,
        };
        // If the buffer is full we silently drop — matches NetworkManager::BufferFull
        // which the router previously swallowed too.
        let _ = self.pending_acks.push(pend);
        #[cfg(feature = "ll-stats")]
        {
            self.pkts_sent += 1;
        }
    }

    fn on_rx(&mut self, pkt: &MeshPacket<SIZE>) -> LinkVerdict {
        if let Some(idx) = self
            .pending_acks
            .iter()
            .position(|p| Self::matches_pending(&p.packet, pkt))
        {
            self.pending_acks.remove(idx);
            #[cfg(feature = "ll-stats")]
            {
                self.pkts_acked += 1;
            }
            // Explicit ACK packets have no information for NW, so swallow them.
            // Echo ACKs (Data packets we're seeing come back) are already
            // duplicates in NW's `recent_seen`, but we drop them here so NW
            // doesn't have to do the matching work twice.
            LinkVerdict::Absorbed
        } else {
            LinkVerdict::PassToNw
        }
    }

    fn poll_retransmissions<const CAP: usize>(&mut self, buffer: &mut TransmitBuffer<SIZE, CAP>) {
        // Drop entries that have exhausted retries.
        let prev_len = self.pending_acks.len();
        let max_retries = self.max_retries;
        self.pending_acks.retain(|p| p.retries < max_retries);
        #[cfg(feature = "ll-stats")]
        {
            self.pkts_lost += (prev_len - self.pending_acks.len()) as u32;
        }
        #[cfg(not(feature = "ll-stats"))]
        let _ = prev_len;

        // For entries whose timeout has fired: bump retries, reset timeout,
        // clone the packet into the caller's tx buffer.
        let now = Instant::now();
        let new_timeout = Duration::from_millis(self.timeout_ms as u64);
        for pending in self.pending_acks.iter_mut() {
            if pending.timeout > now {
                continue;
            }
            if buffer.push(pending.packet.clone()).is_err() {
                // Buffer full; leave the pending entry alone and try again next tick.
                break;
            }
            pending.retries += 1;
            pending.timeout = now + new_timeout;
            #[cfg(feature = "ll-stats")]
            {
                self.pkts_retx += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: usize = 32;
    const LEN: usize = 4;
    const CAP: usize = 4;

    fn data(src: u16, dst: u16, id: u16) -> MeshPacket<SIZE> {
        MeshPacket {
            source_id: src,
            destination_id: dst,
            packet_id: id,
            packet_type: PacketType::Data,
            payload: Vec::new(),
            hop_count: 0,
            hop_to_gw: 0,
        }
    }

    fn ack_for(pending: &MeshPacket<SIZE>, from: u16) -> MeshPacket<SIZE> {
        MeshPacket {
            source_id: from,
            destination_id: pending.source_id,
            packet_id: pending.packet_id,
            packet_type: PacketType::Ack,
            payload: Vec::new(),
            hop_count: 0,
            hop_to_gw: 0,
        }
    }

    #[test]
    fn on_tx_tracks_data_and_skips_heartbeat_and_ack() {
        let mut ll: BleLL<SIZE, LEN> = BleLL::new(100, 3);
        ll.on_tx(&data(2, 1, 10));
        assert_eq!(ll.pending_count(), 1);

        let mut hb = data(2, 0, 11);
        hb.packet_type = PacketType::HeartBeat;
        ll.on_tx(&hb);
        assert_eq!(ll.pending_count(), 1);

        let mut a = data(2, 3, 12);
        a.packet_type = PacketType::Ack;
        ll.on_tx(&a);
        assert_eq!(ll.pending_count(), 1);
    }

    #[test]
    fn on_rx_consumes_echo_ack() {
        let mut ll: BleLL<SIZE, LEN> = BleLL::new(100, 3);
        let pkt = data(2, 1, 10);
        ll.on_tx(&pkt);

        // Next hop re-emits our packet — same source_id+packet_id
        let disp = ll.on_rx(&pkt);
        assert_eq!(disp, LinkVerdict::Absorbed);
        assert_eq!(ll.pending_count(), 0);
    }

    #[test]
    fn on_rx_consumes_explicit_ack_from_destination() {
        let mut ll: BleLL<SIZE, LEN> = BleLL::new(100, 3);
        let pkt = data(2, 3, 10);
        ll.on_tx(&pkt);

        let ack = ack_for(&pkt, 3);
        assert_eq!(ll.on_rx(&ack), LinkVerdict::Absorbed);
        assert_eq!(ll.pending_count(), 0);
    }

    #[test]
    fn on_rx_passes_unrelated_packet() {
        let mut ll: BleLL<SIZE, LEN> = BleLL::new(100, 3);
        ll.on_tx(&data(2, 1, 10));

        let other = data(5, 1, 99);
        assert_eq!(ll.on_rx(&other), LinkVerdict::PassToNw);
        assert_eq!(ll.pending_count(), 1);
    }

    // Small sleep between on_tx and the first poll so `Instant::now()` strictly
    // exceeds the recorded deadline. With `timeout_ms=0` and no sleep the two
    // can tie, and `timeout >= now` would skip retransmission (never bumping
    // retries). The retry/timeout semantics are correct for real usage — the
    // tests just need to cross the deadline.
    fn tick_clock() {
        // std::thread::sleep(std::time::Duration::from_millis(1));
    }

    #[test]
    fn poll_retransmissions_requeues_timed_out_entries() {
        let mut ll: BleLL<SIZE, LEN> = BleLL::new(0, 3);
        ll.on_tx(&data(2, 1, 10));
        tick_clock();

        let mut buf: TransmitBuffer<SIZE, CAP> = TransmitBuffer::new();
        ll.poll_retransmissions(&mut buf);
        assert_eq!(buf.len(), 1);
        assert_eq!(ll.pending_count(), 1); // Still tracked; waiting for ACK or next retry
    }

    #[test]
    fn poll_retransmissions_drops_after_max_retries() {
        let mut ll: BleLL<SIZE, LEN> = BleLL::new(0, 2);
        ll.on_tx(&data(2, 1, 10));

        let mut buf: TransmitBuffer<SIZE, CAP> = TransmitBuffer::new();
        tick_clock();
        ll.poll_retransmissions(&mut buf);
        assert_eq!(ll.pending_count(), 1);
        // Second poll: retry 2 (== max_retries). Pending stays for now; it will
        // be dropped on the NEXT poll since the retain check is `retries <
        // max_retries`.
        tick_clock();
        ll.poll_retransmissions(&mut buf);
        tick_clock();
        ll.poll_retransmissions(&mut buf);
        assert_eq!(ll.pending_count(), 0);
    }

    #[cfg(feature = "ll-stats")]
    #[test]
    fn stats_track_sent_acked_retx_lost() {
        let mut ll: BleLL<SIZE, LEN> = BleLL::new(0, 1);
        let pkt = data(2, 1, 10);
        ll.on_tx(&pkt);
        assert_eq!(ll.pkts_sent, 1);

        let mut buf: TransmitBuffer<SIZE, CAP> = TransmitBuffer::new();
        tick_clock();
        ll.poll_retransmissions(&mut buf);
        assert_eq!(ll.pkts_retx, 1);

        // On next poll the entry exceeds max_retries and is dropped as lost.
        tick_clock();
        ll.poll_retransmissions(&mut buf);
        assert_eq!(ll.pkts_lost, 1);

        // Fresh packet that gets an ACK
        let p2 = data(2, 1, 11);
        ll.on_tx(&p2);
        ll.on_rx(&ack_for(&p2, 1));
        assert_eq!(ll.pkts_acked, 1);
    }
}

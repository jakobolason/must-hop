//! Link layer: owns per-link retransmission state (pending-acks) and decides
//! whether an incoming packet completes one of our outstanding transmissions.
//!
//! Sits between the network layer and the MAC. The network layer hands typed
//! [`crate::MeshPacket`]s down (via [`LinkLayer::on_tx`]); the MAC pulls those
//! same typed packets out of the shared [`crate::nw_layer::TransmitBuffer`]
//! and does the actual serialization/radio ops.

use crate::MeshPacket;
use crate::nw_layer::TransmitBuffer;

pub mod ble;

/// The link layer's verdict after inspecting a received packet.
///
/// Intentionally open-ended: today only ACK matching produces [`Absorbed`],
/// but future impls may drop link-level duplicates, consume fragmentation
/// control frames, etc., under the same contract.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(not(feature = "in_std"), derive(defmt::Format))]
pub enum LinkVerdict {
    /// The link layer took responsibility for this packet; the router should
    /// not hand it to the network layer.
    Absorbed,
    /// The link layer has no interest in this packet; forward it to NW.
    PassToNw,
}

/// Contract for a link layer implementation.
///
/// The link layer owns the `pending_acks` table, decides what counts as an
/// ACK (echo from the next hop, explicit [`crate::PacketType::Ack`] packet,
/// …), and emits retransmissions when timeouts expire. Construction of ACK
/// packets themselves belongs to the network layer — see
/// [`crate::nw_layer::NetworkLayer::build_ack`] — because the ACK's
/// `source_id` / `hop_to_gw` are NW state.
pub trait LinkLayer<const SIZE: usize> {
    /// Observe a packet that the router is about to transmit. Impl decides
    /// whether to track it in `pending_acks` (typically skips `Ack` and
    /// `HeartBeat` types — ACKs don't get ACKed, heartbeats are periodic).
    fn on_tx(&mut self, pkt: &MeshPacket<SIZE>);

    /// Inspect an incoming packet and return the LL's verdict. If the LL
    /// absorbs it (e.g. it ACKs a pending transmission) the router skips NW
    /// processing; otherwise the packet continues up the stack.
    fn on_rx(&mut self, pkt: &MeshPacket<SIZE>) -> LinkVerdict;

    /// Push packets whose ACK timeout has expired back into the transmit
    /// buffer for retransmission. Caller (router) invokes this at the top
    /// of each tick.
    fn poll_retransmissions<const CAP: usize>(&mut self, buffer: &mut TransmitBuffer<SIZE, CAP>);
}

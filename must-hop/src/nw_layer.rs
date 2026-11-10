//! Network layer: routing and packet processing.
//!
//! Responsibilities split (post-refactor):
//! - Link layer ([`crate::ll`]) handles physical addressing, per-link retransmission
//!   (`pending_acks`), and serialization.
//! - Network layer handles routing and (source-based) duplicate suppression via
//!   `recent_seen`.
//! - MAC policy ([`crate::policy`]) sits beside the link layer and owns slot/beacon
//!   scheduling. The network layer talks to the MAC when it needs to emit control
//!   frames (e.g. heartbeats for time sync).

use heapless::Vec;

use crate::MeshPacket;

pub mod prototype;

#[derive(Debug, PartialEq)]
#[cfg_attr(not(feature = "in_std"), derive(defmt::Format))]
pub enum NetworkError {
    Full,
    InvalidPayload,
}

/// Staging area the network layer writes into and the link layer drains.
///
/// Kept as a `heapless::Vec` alias so [`crate::policy::MacPolicy`] impls that
/// already treat the staging area as a `Vec<MeshPacket, LEN>` keep working
/// unchanged. The name lives on so downstream code still documents intent at
/// the API boundary.
pub type TransmitBuffer<const SIZE: usize, const CAP: usize> = Vec<MeshPacket<SIZE>, CAP>;

/// Outcome of processing one received packet at the network layer.
///
/// Fields are independent: a broadcast arriving at us can both be delivered to
/// the application and forwarded onto the mesh; a unicast destined to us can
/// be delivered and also need an ACK.
#[derive(Debug, Default)]
pub struct RxOutcome<const SIZE: usize> {
    /// Rewritten packet to retransmit on the mesh.
    pub forward: Option<MeshPacket<SIZE>>,
    /// Packet to hand up to the application layer.
    pub deliver: Option<MeshPacket<SIZE>>,
    /// `(packet_id, destination)` the link layer should emit an ACK for.
    pub ack: Option<(u16, u16)>,
}

impl<const SIZE: usize> RxOutcome<SIZE> {
    pub const fn empty() -> Self {
        Self {
            forward: None,
            deliver: None,
            ack: None,
        }
    }
}

/// Contract for a pluggable routing/processing strategy.
///
/// The buffer is owned by the caller (typically [`crate::MeshRouter`]); the
/// network layer only writes into or reads from the mutable borrow it is
/// given. This keeps `pending_acks`-style state that outlives a single call
/// on the link layer where it belongs.
pub trait NetworkLayer<const SIZE: usize> {
    /// Build a packet for `payload` bound for `destination` and place it into
    /// `buffer` for the link layer to pick up.
    fn queue_payload<const CAP: usize>(
        &mut self,
        destination: u16,
        payload: &[u8],
        buffer: &mut TransmitBuffer<SIZE, CAP>,
    ) -> Result<(), NetworkError>;

    /// Decide what to do with a packet that arrived from the link layer.
    fn receive(&mut self, pkt: MeshPacket<SIZE>) -> RxOutcome<SIZE>;

    /// Produce a heartbeat to emit on this tick, or `None` if this node should
    /// not originate one. Typically only the gateway returns `Some`; nodes
    /// re-flood heartbeats via [`NetworkLayer::receive`].
    fn build_heartbeat(&mut self) -> Option<MeshPacket<SIZE>>;

    /// Current hop count to the gateway. Used by MAC policies that need the
    /// gradient (e.g. TDMA slot assignment).
    fn gw_hops(&self) -> u8;
}

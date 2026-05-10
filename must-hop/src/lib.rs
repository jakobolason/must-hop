//! A `no_std` multi-hop mesh networking library for embedded systems.

#![no_std]
// #![no_main]
#![warn(clippy::todo)]

pub mod node;
pub mod tasks;

#[doc(inline)]
pub use crate::mesh_router::MeshRouter;

#[doc(inline)]
pub use crate::{
    network_manager::NetworkManager,
    node::MHNode,
    policy::{ra::RandomAccessMac, tdma::TdmaMac},
};

pub mod mesh_router;
pub mod network_manager;
pub mod policy;

// TODO: Move these into lib.rs, move all files in node/ into appropriate places
// MEans that mesh_router and network_manager goes into this dir, and tdma goes into policy/ instead
// of node/policy/

use embassy_time::Instant;
use heapless::Vec;
use serde::{Deserialize, Serialize};

/// Either this packet
/// Is Data, and should get an ACK return
/// A Data stream, meaning it wants to send multiple packets(u8 amount). In this case, Node B will
/// continue to listen, until it has receieved (u8) amount of packages
/// ACK should only be sent by a GW, because they will not retransmit
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Copy)]
#[cfg_attr(not(feature = "in_std"), derive(defmt::Format))]
pub enum PacketType {
    /// To send just a single packet
    Data,
    /// Payload should be bitmask of received packets
    Ack,
    /// The GW should send out periodic heartbeats
    HeartBeat,
}

/// MHPacket defines the package sent around the network
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[cfg_attr(not(feature = "in_std"), derive(defmt::Format))]
pub struct MHPacket<const SIZE: usize> {
    /// Destination identifier
    // TODO: Perhaps bigger than u8?
    pub destination_id: u8,
    pub packet_type: PacketType,
    pub packet_id: u16,
    pub source_id: u8,
    /// Your specificed data wanting to send
    // (DE)serialize is only available up to 32 bytes
    pub payload: Vec<u8, SIZE>,
    /// The amount of hops this package has been on
    // TODO: Implement logic for this
    pub hop_count: u8,
    /// Amount of hops the current node has to GW
    pub hop_to_gw: u8,
}

#[derive(Debug, Clone)]
pub struct RxPacket {
    // pub preamble_instant: Option<Instant>,
    pub rx_done_instant: Instant,
    pub payload_size: u8,
    // pub estimated_toa: (u32, u32),
}

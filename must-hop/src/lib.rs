//! A `no_std` multi-hop mesh networking library for embedded systems.

#![no_std]
// #![no_main]
#![warn(clippy::todo)]

pub mod lora;
pub mod node;
pub mod tasks;

#[doc(inline)]
pub use crate::node::mesh_router::MeshRouter;

#[doc(inline)]
pub use crate::node::{
    MHNode, MHPacket,
    network_manager::NetworkManager,
    policy::{RandomAccessMac, RoutingPolicy, tdma::TdmaMac},
};


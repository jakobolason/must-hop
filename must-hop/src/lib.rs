#![no_std]
#![no_main]

use core::module_path;
use defmt::info;
use heapless::Vec;
use serde::{Deserialize, Serialize};

pub fn hello_world() {
    info!("Hello world from must hop!");
}

#[derive(Serialize, Deserialize, Debug, PartialEq, defmt::Format)]
pub struct MHPacket<const MAX_PACKET_SIZE: usize = 128> {
    pub destination_id: u8,
    pub source_id: u8,
    // (DE)serialize is only available up to 32 bytes
    pub paylod: Vec<u8, MAX_PACKET_SIZE>,
    pub hop_count: u8,
}

pub trait MHNode {
    type Error;
    type Payload;
    type Connection;

    fn transmit(
        &mut self,
        packet: MHPacket,
    ) -> impl core::future::Future<Output = Result<(), Self::Error>>;
    fn receive(
        &mut self,
        conn: Self::Connection,
        receiving_buffer: &[u8],
    ) -> impl core::future::Future<Output = Result<Self::Payload, Self::Error>>;
}

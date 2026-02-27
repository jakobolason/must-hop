#[cfg(not(feature = "in_std"))]
use defmt::{error, info};
#[cfg(feature = "in_std")]
use log::{error, info};

use embassy_sync::channel;
use heapless::Vec;
use serde::Serialize;

use crate::{
    lora::{LoraNode, TransmitParameters},
    node::{
        mesh_router::MeshRouter,
        network_manager::NetworkManager,
        policy::{NodePolicy, RandomAccessMac},
    },
};

use lora_phy::mod_traits::RadioKind;
use lora_phy::{DelayNs, LoRa};

// TODO: Should this be a const generic for the user to set? Perhaps a default value?
const TRANSMISSION_BUFFER: usize = 256; // The radio can receive 256 bytes to transmit

// TODO: Ensure SIZE and MAX_PACKET_SIZE are the same
pub async fn lora_task<RK, DLY, T, M, const SIZE: usize, const LEN: usize>(
    lora: &mut LoRa<RK, DLY>,
    channel: channel::Receiver<'static, M, T, 3>,
    tp: TransmitParameters,
    source_id: u8,
    timeout: u8,
    max_retries: u8,
) where
    RK: RadioKind,
    DLY: DelayNs,
    T: Into<Vec<u8, SIZE>> + Serialize + Copy,
    M: embassy_sync::blocking_mutex::raw::RawMutex,
{
    let node = match LoraNode::new(lora, tp) {
        Ok(rx) => rx,
        Err(e) => {
            error!("Error in preparing for RX: {:?}", e);
            return;
        }
    };
    let nm = NetworkManager::<SIZE, LEN>::new(source_id, timeout, max_retries);
    let mut router = MeshRouter::new(node, nm, RandomAccessMac, NodePolicy);
    loop {
        info!("In lora task loop");

        let mut receiving_buffer = [00u8; TRANSMISSION_BUFFER];

        info!("Waiting for packet or sensor data to send");
        // Either sensor data should be sent, or a packet is ready to be received
        match channel.try_receive() {
            Ok(data) => {
                if let Err(e) = router.queu_payload(data.into(), 0) {
                    error!("Error queing sensor data: {:?}", e);
                    continue;
                }
            }
            Err(e) => {
                error!("error in receinv sensor data: {:?}", e);
                continue;
            }
        }
        match router.tick(&mut receiving_buffer).await {
            Ok(my_pkts) => {
                if !my_pkts.is_empty() {
                    info!("received these packets for me!: {}", my_pkts.len())
                }
            }
            Err(e) => error!("Error in ticking router; {:?}", e),
        }
    }
}

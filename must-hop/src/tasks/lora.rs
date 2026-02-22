#[cfg(not(feature = "in_std"))]
use defmt::{error, info};
#[cfg(feature = "in_std")]
use log::{error, info};

use embassy_futures::select::{Either, select};
use embassy_sync::channel;
use heapless::Vec;
use serde::Serialize;

use crate::{
    lora::{LoraNode, TransmitParameters},
    node::{mesh_router::MeshRouter, network_manager::NetworkManager, policy::NodePolicy},
};

use lora_phy::mod_traits::RadioKind;
use lora_phy::{DelayNs, LoRa};

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
    let mut router: MeshRouter<_, _, _, NodePolicy> = MeshRouter::new(node, nm);
    loop {
        info!("In lora task loop");

        let mut receiving_buffer = [00u8; SIZE];

        info!("Waiting for packet or sensor data to send");
        // Either sensor data should be sent, or a packet is ready to be received
        let either = select(channel.receive(), router.listen(&mut receiving_buffer)).await;
        match either {
            Either::First(data) => {
                info!("SENSOR DATA won");
                // destination 0 is the gateway
                if let Err(e) = router.send_payload(data.into(), 0).await {
                    error!("Error in transmitting sensor data: {:?}", e);
                    continue;
                }
            }
            Either::Second(conn) => {
                info!("RECEIVER won, reading ...");
                let conn = match conn {
                    Ok(conn) => conn,
                    Err(e) => {
                        error!("Error in getting connection: {:?}", e);
                        continue;
                    }
                };
                let my_pkts = match router.receive(conn, &receiving_buffer).await {
                    Ok(pkts) => pkts,
                    Err(e) => {
                        error!("Error in receiving packet: {:?}", e);
                        continue;
                    }
                };
                info!("I got these pkts: {}", my_pkts.len());
            }
        }
    }
}

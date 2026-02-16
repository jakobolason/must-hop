use defmt::{error, info};

use embassy_futures::select::{Either, select};
use embassy_sync::channel;
use heapless::Vec;
use serde::Serialize;

use crate::{
    lora::{LoraNode, TransmitParameters},
    node::{mesh_router::MeshRouter, network_manager::NetworkManager},
};

use lora_phy::{DelayNs, LoRa};

use lora_phy::mod_params::{Bandwidth, CodingRate, SpreadingFactor};
use lora_phy::mod_traits::RadioKind;

// TODO: Ensure SIZE and MAX_PACKET_SIZE are the same
pub async fn lora_task<RK, DLY, T, M, const SIZE: usize>(
    lora: &mut LoRa<RK, DLY>,
    channel: channel::Receiver<'static, M, T, 3>,
    lora_hz: u32,
) where
    RK: RadioKind,
    DLY: DelayNs,
    T: Into<Vec<u8, SIZE>> + Serialize + Copy,
    M: embassy_sync::blocking_mutex::raw::RawMutex,
{
    let sf = SpreadingFactor::_12;
    let bw = Bandwidth::_125KHz;
    let cr = CodingRate::_4_8;
    let tp: TransmitParameters = TransmitParameters {
        sf,
        bw,
        cr,
        lora_hz,
        pre_amp: 8,
        imp_hed: false,
        max_pack_len: SIZE,
        crc: true,
        iq: false,
    };
    let node = match LoraNode::new(lora, tp) {
        Ok(rx) => rx,
        Err(e) => {
            error!("Error in preparing for RX: {:?}", e);
            return;
        }
    };
    let nm = NetworkManager::<SIZE>::new(1, 3, 3);
    let mut router = MeshRouter::new(node, nm);
    loop {
        info!("In lora task loop");

        let mut receiving_buffer = [00u8; SIZE];

        info!("Waiting for packet or sensor data to send");
        // Either sensor data should be sent, or a packet is ready to be received
        let either = select(channel.receive(), router.listen(&mut receiving_buffer)).await;
        match either {
            Either::First(data) => {
                if let Err(e) = router.send_payload(data.into()).await {
                    error!("Error in transmitting sensor data: {:?}", e);
                    continue;
                }
            }
            Either::Second(conn) => {
                let conn = match conn {
                    Ok(conn) => conn,
                    Err(e) => {
                        error!("Error in getting connection: {:?}", e);
                        continue;
                    }
                };
                if let Err(e) = router.receive(conn, &receiving_buffer).await {
                    error!("Error in receiving packet: {:?}", e);
                    continue;
                }
            }
        }
    }
}

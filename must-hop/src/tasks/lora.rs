use defmt::{error, info, trace};

use embassy_futures::select::{Either, select};
use embassy_sync::channel;
use serde::Serialize;

use crate::{
    lora::{LoraNode, RadioState, TransmitParameters},
    node::{MHNode, MHPacket, NetworkManager},
};

use lora_phy::{DelayNs, LoRa};

use lora_phy::mod_params::{Bandwidth, CodingRate, SpreadingFactor};
use lora_phy::mod_traits::RadioKind;

// TODO: Ensure MAX_PACK_LEN and MAX_PACKET_SIZE are the same
pub async fn lora_task<RK, DLY, T, M, const MAX_PACK_LEN: usize>(
    lora: &mut LoRa<RK, DLY>,
    // TODO: Should this or `ThreadModeRawMutex` be used?
    channel: channel::Receiver<'static, M, T, 3>,
    lora_hz: u32,
) where
    RK: RadioKind,
    DLY: DelayNs,
    T: Into<MHPacket<MAX_PACK_LEN>> + Serialize + Copy,
    M: embassy_sync::blocking_mutex::raw::RawMutex,
{
    let sf = SpreadingFactor::_12;
    let bw = Bandwidth::_125KHz;
    let cr = CodingRate::_4_8;
    let mut nm = NetworkManager::<MAX_PACK_LEN>::new(1, 3, 3);
    loop {
        info!("In lora task loop");
        let tp: TransmitParameters = TransmitParameters {
            sf,
            bw,
            cr,
            lora_hz,
            pre_amp: 8,
            imp_hed: false,
            max_pack_len: MAX_PACK_LEN,
            crc: true,
            iq: false,
        };
        let mut node = match LoraNode::new(lora, tp) {
            Ok(rx) => rx,
            Err(e) => {
                error!("Error in preparing for RX: {:?}", e);
                continue;
            }
        };
        if let Err(e) = node.prepare_for_rx().await {
            error!("Couuld not prepare for rx: {:?}", e);
            continue;
        }

        let mut receiving_buffer = [00u8; MAX_PACK_LEN];

        info!("Waiting for packet or sensor data to send");
        trace!("Waiting for packet in trace");
        // Either sensor data should be sent, or a packet is ready to be received
        let either = select(channel.receive(), node.listen(&mut receiving_buffer)).await;
        match either {
            Either::First(data) => {
                // Now capture all pending packets from other nodes perhaps, and send 1 (Data) or
                // multiple (Data stream)
                let pkts = match nm.send_packet(data, 2) {
                    Ok(pkts) => pkts,
                    Err(e) => {
                        error!("Error in getting packets to sen: {:?}", e);
                        continue;
                    }
                };
                for pkt in pkts {
                    if let Err(e) = node.transmit(pkt).await {
                        error!("Error in transmitting: {:?}", e);
                        continue;
                    }
                }
            }
            Either::Second(conn) => {
                // nm.receive(packet).await
                if let Err(e) = node.receive(conn, &receiving_buffer).await {
                    error!("Error in receing information: {:?}", e);
                    continue;
                }
            }
        }
    }
}

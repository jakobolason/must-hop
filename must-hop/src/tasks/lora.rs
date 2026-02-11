use defmt::{error, info};

use embassy_futures::select::{Either, select};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel};

use crate::{
    lora::{LoraNode, RadioState, TransmitParameters},
    node::{MHNode, MHPacket},
};

use lora_phy::{DelayNs, LoRa, RxMode};

use lora_phy::mod_params::{Bandwidth, CodingRate, SpreadingFactor};
use lora_phy::mod_traits::RadioKind;

// TODO: Ensure MAX_PACK_LEN and MAX_PACKET_SIZE are the same
pub async fn lora_task<'a, RK, DLY, T, const MAX_PACK_LEN: usize>(
    mut lora: &'a mut LoRa<RK, DLY>,
    // TODO: Should this or `ThreadModeRawMutex` be used?
    channel: channel::Receiver<'static, CriticalSectionRawMutex, T, 3>,
    lora_hz: u32,
) where
    RK: RadioKind,
    DLY: DelayNs,
    T: Into<MHPacket<MAX_PACK_LEN>>,
{
    let sf = SpreadingFactor::_12;
    let bw = Bandwidth::_500KHz;
    let cr = CodingRate::_4_8;
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
        let mut node = match LoraNode::new(&mut lora, tp) {
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
        // Either sensor data should be sent, or a packet is ready to be received
        let either = select(channel.receive(), node.listen(&mut receiving_buffer)).await;
        match either {
            Either::First(data) => {
                if let Err(e) = node.transmit(data.into()).await {
                    error!("Error in transmitting: {:?}", e);
                    continue;
                }
            }
            Either::Second(conn) => {
                if let Err(e) = node.receive(conn, &receiving_buffer).await {
                    error!("Error in receing information: {:?}", e);
                    continue;
                }
            }
        }
    }
}

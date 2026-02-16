use defmt::{error, info, trace};

use embassy_futures::select::{Either, select};
use embassy_sync::channel;
use serde::Serialize;

use crate::{
    lora::{LoraNode, TransmitParameters},
    node::{MHNode, MHPacket, mesh_router::MeshRouter, network_manager::NetworkManager},
};

use lora_phy::{DelayNs, LoRa};

use lora_phy::mod_params::{Bandwidth, CodingRate, SpreadingFactor};
use lora_phy::mod_traits::RadioKind;

// TODO: Ensure MAX_PACK_LEN and MAX_PACKET_SIZE are the same
pub async fn lora_task<RK, DLY, T, M, const MAX_PACK_LEN: usize>(
    lora: &mut LoRa<RK, DLY>,
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
            return;
        }
    };
    let mut nm = NetworkManager::<MAX_PACK_LEN>::new(1, 3, 3);
    let router = MeshRouter::new(node, nm);
    loop {
        info!("In lora task loop");

        let mut receiving_buffer = [00u8; MAX_PACK_LEN];

        info!("Waiting for packet or sensor data to send");
        trace!("Waiting for packet in trace");
        // Either sensor data should be sent, or a packet is ready to be received
        let either = select(channel.receive(), node.listen(&mut receiving_buffer, false)).await;
        match either {
            Either::First(data) => {
                let mhpack = match nm.from_t(data, 2) {
                    Ok(pack) => pack,
                    Err(e) => {
                        error!("Error creating MHPacket: {:?}", e);
                        continue;
                    }
                };
                // Now capture all pending packets from other nodes perhaps, and send 1 (Data) or
                // multiple (Data stream)
                let pkts = match nm.send_packet(mhpack) {
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
                // TODO: should be able to receieve multiple packets
                let pkt = match node.receive(conn, &receiving_buffer).await {
                    Ok(pkt) => pkt,
                    Err(e) => {
                        error!("Error in receing information: {:?}", e);
                        continue;
                    }
                };
                match nm.receive_packet(pkt) {
                    // If there is some, then it means that this was a DataStream
                    Ok(Some(_)) => {}
                    Ok(None) => continue,
                    Err(e) => {
                        error!("Error in Networkmanager receive packet: {:?}", e);
                        continue;
                    }
                }
                match node.listen(&mut receiving_buffer).await {
                    Ok(conn) => todo!("The same loop as before (recursive?)"),
                    Err(e) => {
                        error!("Error in receiving_buffer: {:?}", e);
                        continue;
                    }
                }
            }
        }
    }
}

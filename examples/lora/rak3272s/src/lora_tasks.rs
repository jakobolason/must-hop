// This creates the task which checks for sensor data
// or someone trying to send data. This handles both receive and transmission logic

use crate::iv;
use defmt::{error, info, warn};
use embassy_stm32::{
    gpio::Output,
    spi::{Spi, mode::Master},
};

use embassy_futures::select::{Either, select};
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, channel};
use embassy_time::Delay;

use embassy_stm32::mode::Async;
use heapless::Vec;
use lora_phy::LoRa;
use lora_phy::mod_params::{Bandwidth, CodingRate, SpreadingFactor};
use lora_phy::sx126x::Stm32wl;
use lora_phy::sx126x::Sx126x;
use must_hop::{
    lora::{LoraNode, RadioState, TransmitParameters},
    node::{MHNode, MHPacket},
};
use postcard::to_slice;
use {defmt_rtt as _, panic_probe as _};

use serde::{Deserialize, Serialize};

const LORA_FREQUENCY_IN_HZ: u32 = 868_000_000; // warning: set this appropriately for the region
type Stm32wlLoRa<'d, CM> = LoRa<
    Sx126x<
        iv::SubghzSpiDevice<Spi<'d, Async, CM>>,
        iv::Stm32wlInterfaceVariant<Output<'d>>,
        Stm32wl,
    >,
    Delay,
>;

#[derive(Serialize, Deserialize, Debug, PartialEq, defmt::Format)]
pub struct SensorData {
    pub device_id: u8,
    pub temperate: f32,
    pub voltage: f32,
    pub acceleration_x: f32,
}

// TODO: Shuold not use this, only for prototyping
impl From<SensorData> for MHPacket {
    fn from(data: SensorData) -> Self {
        let mut buffer = [0u8; MAX_PACK_LEN];
        let used_slice = to_slice(&data, &mut buffer).expect("Coudl not serialize sensor data");
        let payload_bytes =
            Vec::from_slice(used_slice).expect("could not get vec, sensor data was too large");
        // let payload_bytes =
        //     to_vec(&data).expect("Serialization failed, struct too large for buffer");

        Self {
            destination_id: 1,
            source_id: 2,
            payload: payload_bytes,
            hop_count: 0,
        }
    }
}

const MAX_PACK_LEN: usize = 128;

#[allow(
    clippy::large_stack_frames,
    reason = "This is the main task, so large size is okay"
)]
#[allow(unused_variables)]
#[embassy_executor::task]
pub async fn lora_task(
    mut lora: Stm32wlLoRa<'static, Master>,
    channel: channel::Receiver<'static, ThreadModeRawMutex, SensorData, 3>,
) {
    let sf = SpreadingFactor::_12;
    let bw = Bandwidth::_500KHz;
    let cr = CodingRate::_4_8;
    loop {
        info!("In lora task loop");
        let tp: TransmitParameters = TransmitParameters {
            sf,
            bw,
            cr,
            lora_hz: LORA_FREQUENCY_IN_HZ,
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
            Either::First(sensor_data) => {
                if let Err(e) = node.transmit(sensor_data.into()).await {
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

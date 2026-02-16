// This creates the task which checks for sensor data
// or someone trying to send data. This handles both receive and transmission logic

use crate::iv;
use embassy_stm32::{
    gpio::Output,
    spi::{Spi, mode::Master},
};

use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, channel};
use embassy_time::Delay;

use embassy_stm32::mode::Async;
use lora_phy::LoRa;
use lora_phy::sx126x::Stm32wl;
use lora_phy::sx126x::Sx126x;
use must_hop::tasks::lora;
use postcard::to_slice;
use {defmt_rtt as _, panic_probe as _};

use serde::{Deserialize, Serialize};

const LORA_FREQUENCY_IN_HZ: u32 = 868_100_000; // warning: set this appropriately for the region

type Stm32wlLoRa<'d, CM> = LoRa<
    Sx126x<
        iv::SubghzSpiDevice<Spi<'d, Async, CM>>,
        iv::Stm32wlInterfaceVariant<Output<'d>>,
        Stm32wl,
    >,
    Delay,
>;

#[derive(Serialize, Deserialize, Debug, PartialEq, defmt::Format, Copy, Clone)]
pub struct SensorData {
    pub device_id: u8,
    pub temperate: f32,
    pub voltage: f32,
    pub acceleration_x: f32,
}

// TODO: Shuold not use this, only for prototyping
impl From<SensorData> for [u8; MAX_PACK_LEN] {
    fn from(data: SensorData) -> Self {
        let mut buffer = [0u8; MAX_PACK_LEN];
        to_slice(&data, &mut buffer).expect("Could not serialize sensor data");
        buffer
        // let payload_bytes =
        //     Vec::from_slice(used_slice).expect("could not get vec, sensor data was too large");
        //
        // Self {
        //     destination_id: 1,
        //     source_id: 2,
        //     packet_id: 3,
        //     packet_type: PacketType::Data,
        //     payload: payload_bytes,
        //     hop_count: 0,
        // }
    }
}

const MAX_PACK_LEN: usize = 128;

#[embassy_executor::task]
pub async fn lora_task(
    mut lora: Stm32wlLoRa<'static, Master>,
    channel: channel::Receiver<'static, ThreadModeRawMutex, SensorData, 3>,
) {
    lora::lora_task(&mut lora, channel, LORA_FREQUENCY_IN_HZ).await;
}

//! This example runs on the STM32WL board, which has a builtin Semtech Sx1262 radio.
//! It demonstrates LORA P2P receive functionality in conjunction with the lora_p2p_send example.
#![no_std]
#![no_main]

#[path = "../iv.rs"]
mod iv;

use defmt::{error, info};
use embassy_executor::Spawner;
use embassy_stm32::peripherals;
use embassy_stm32::rng;
use embassy_stm32::rng::Rng;
use embassy_stm32::{
    Config, bind_interrupts,
    gpio::{Level, Output, Speed},
    rcc::{MSIRange, Sysclk, mux},
    spi::Spi,
};
use embassy_sync::channel;
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, channel::Channel};
use embassy_time::{Delay, Timer};
use heapless::Vec;
use lora_phy::LoRa;
use lora_phy::mod_params::{Bandwidth, CodingRate, SpreadingFactor};
use lora_phy::sx126x;
use lora_phy::sx126x::{Stm32wl, Sx126x};
use {defmt_rtt as _, panic_probe as _};

use self::iv::{InterruptHandler, Stm32wlInterfaceVariant, SubghzSpiDevice};

use embassy_stm32::spi::mode::Master;

use embassy_stm32::mode::Async;
use must_hop::{lora::TransmitParameters, tasks::lora};
use postcard::to_slice;
use {defmt_rtt as _, panic_probe as _};

use serde::{Deserialize, Serialize};

const LORA_FREQUENCY_IN_HZ: u32 = 868_100_000; // warning: set this appropriately for the region

static CHANNEL: Channel<ThreadModeRawMutex, SensorData, 3> = Channel::new();

bind_interrupts!(struct Irqs{
    SUBGHZ_RADIO => InterruptHandler;
    RNG => rng::InterruptHandler<peripherals::RNG>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = Config::default();
    {
        config.rcc.msi = Some(MSIRange::RANGE48M);
        config.rcc.sys = Sysclk::MSI;
        config.rcc.mux.rngsel = mux::Rngsel::MSI;
        config.enable_debug_during_sleep = true;
    }
    let p = embassy_stm32::init(config);

    info!("config done...");
    let tx_pin = Output::new(p.PC13, Level::Low, Speed::VeryHigh);
    let rx_pin = Output::new(p.PB8, Level::Low, Speed::VeryHigh);

    let spi = Spi::new_subghz(p.SUBGHZSPI, p.DMA1_CH1, p.DMA1_CH2);
    let spi = SubghzSpiDevice(spi);
    // TODO: Can it work wit low power?
    let use_high_power_pa = true;
    let config = sx126x::Config {
        chip: Stm32wl { use_high_power_pa },
        tcxo_ctrl: None,
        use_dcdc: true,
        rx_boost: false,
    };
    let iv: Stm32wlInterfaceVariant<Output<'_>> =
        Stm32wlInterfaceVariant::new(Irqs, use_high_power_pa, Some(rx_pin), Some(tx_pin), None)
            .unwrap();
    let lora = LoRa::new(Sx126x::new(spi, iv, config), true, Delay)
        .await
        .unwrap();
    info!("lora setup done ...");
    if let Err(e) = spawner.spawn(lora_task(lora, CHANNEL.receiver())) {
        error!("error in spawning lora task: {:?}", e);
    }
    // TODO: Add sensor data creation task
    let rng = Rng::new(p.RNG, Irqs);
    if let Err(e) = spawner.spawn(sensor_task(CHANNEL.sender(), rng)) {
        error!("Error in spawning lora task: {:?}, ", e);
    }

    loop {
        info!("from main...");
        Timer::after_secs(10u64).await;
    }
}

#[embassy_executor::task]
async fn sensor_task(
    channel: channel::Sender<'static, ThreadModeRawMutex, SensorData, 3>,
    mut rng: Rng<'static, peripherals::RNG>, // Ensure 'mut' is here
) {
    Timer::after_secs(10).await;
    loop {
        let expected_packet = SensorData {
            device_id: 42,
            temperate: 23.5,
            voltage: 3.3,
            acceleration_x: 1.2,
        };
        channel.send(expected_packet).await;

        info!("Send a packet!");
        let random = rng.next_u64();
        // random number between 3 and 8
        let r_num = (random % 5) + 3;
        info!("waiting {} seconds ...", r_num);

        Timer::after_secs(r_num).await;
    }
}

/// From the study, sensor data will likely be between 20-40 bytes per transmission
const MAX_PACK_LEN: usize = 40;
// const MAX_RADIO_BUFFER: usize = 256; // kB
const LEN: usize = 5; // floor(256/MAX_PACK_LEN)

#[embassy_executor::task]
pub async fn lora_task(
    mut lora: Stm32wlLoRa<'static, Master>,
    channel: channel::Receiver<'static, ThreadModeRawMutex, SensorData, 3>,
) {
    let sf = SpreadingFactor::_7;
    let bw = Bandwidth::_125KHz;
    let cr = CodingRate::_4_8;
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
    let source_id = 1;
    lora::lora_task::<_, _, _, _, MAX_PACK_LEN, LEN>(&mut lora, channel, tp, source_id, 3, 3).await;
}

// This creates the task which checks for sensor data
// or someone trying to send data. This handles both receive and transmission logic
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
impl From<SensorData> for Vec<u8, MAX_PACK_LEN> {
    fn from(data: SensorData) -> Self {
        let mut buffer = [0u8; MAX_PACK_LEN];
        let slice = to_slice(&data, &mut buffer).expect("Could not serialize sensor data");
        Vec::from_slice(slice).expect("buffer too small")
    }
}

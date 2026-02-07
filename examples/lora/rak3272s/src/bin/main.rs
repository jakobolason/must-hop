//! This example runs on the STM32WL board, which has a builtin Semtech Sx1262 radio.
//! It demonstrates LORA P2P receive functionality in conjunction with the lora_p2p_send example.
#![no_std]
#![no_main]

#[path = "../iv.rs"]
mod iv;
#[path = "../lora_tasks.rs"]
mod lora_tasks;

use defmt::{error, info};
use embassy_executor::Spawner;
use embassy_stm32::{
    Config, bind_interrupts,
    gpio::{Level, Output, Speed},
    rcc::{MSIRange, Sysclk, mux},
    spi::Spi,
};
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, channel::Channel};
use embassy_time::{Delay, Timer};
use lora_phy::LoRa;
use lora_phy::sx126x;
use lora_phy::sx126x::{Stm32wl, Sx126x};
use {defmt_rtt as _, panic_probe as _};

use self::iv::{InterruptHandler, Stm32wlInterfaceVariant, SubghzSpiDevice};
use self::lora_tasks::{SensorData, lora_task};

static CHANNEL: Channel<ThreadModeRawMutex, SensorData, 3> = Channel::new();

bind_interrupts!(struct Irqs{
    SUBGHZ_RADIO => InterruptHandler;
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

    loop {
        info!("from main...");
        Timer::after_secs(10u64).await;
    }
}

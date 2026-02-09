//! This example runs on the STM32WL board, which has a builtin Semtech Sx1262 radio.
//! It demonstrates LORA P2P receive functionality in conjunction with the lora_p2p_send example.
#![no_std]
#![no_main]

#[path = "../iv.rs"]
mod iv;
#[path = "../lora_tasks.rs"]
mod lora_tasks;

use defmt::{error, info, warn};
use embassy_executor::Spawner;
use embassy_stm32::{
    Config, bind_interrupts,
    gpio::{Level, Output, Speed},
    rcc::{MSIRange, Sysclk, mux},
    spi::Spi,
};
use embassy_time::Delay;
use lora_phy::sx126x::{Stm32wl, Sx126x};
use lora_phy::{LoRa, RxMode};
use lora_phy::{mod_params::*, sx126x};
use postcard::from_bytes;
use {defmt_rtt as _, panic_probe as _};

use self::iv::{InterruptHandler, Stm32wlInterfaceVariant, SubghzSpiDevice};
use self::lora_tasks::SensorData;

const LORA_FREQUENCY_IN_HZ: u32 = 868_000_000; // warning: set this appropriately for the region

bind_interrupts!(struct Irqs{
    SUBGHZ_RADIO => InterruptHandler;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
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
    let mut lora = LoRa::new(Sx126x::new(spi, iv, config), true, Delay)
        .await
        .unwrap();
    info!("lora setup done ...");

    let expected_packet = SensorData {
        device_id: 42,
        temperate: 23.5,
        voltage: 3.3,
        acceleration_x: 1.2,
    };

    loop {
        let mut receiving_buffer = [00u8; 100];
        let mdltn_params = {
            // TODO: Check configuration of these, how much can they be changed?
            match lora.create_modulation_params(
                SpreadingFactor::_12,
                Bandwidth::_500KHz,
                CodingRate::_4_8,
                LORA_FREQUENCY_IN_HZ,
            ) {
                Ok(mp) => mp,
                Err(err) => {
                    info!("Radio error = {}", err);
                    continue;
                }
            }
        };

        let rx_pkt_params = {
            match lora.create_rx_packet_params(
                8,
                false,
                receiving_buffer.len() as u8,
                true,
                false,
                &mdltn_params,
            ) {
                Ok(pp) => pp,
                Err(err) => {
                    info!("Radio error = {}", err);
                    continue;
                }
            }
        };

        match lora
            .prepare_for_rx(RxMode::Single(255), &mdltn_params, &rx_pkt_params)
            .await
        {
            Ok(()) => {}
            Err(err) => {
                info!("Radio error = {}", err);
                continue;
            }
        };
        match lora.rx(&rx_pkt_params, &mut receiving_buffer).await {
            Ok((len, rx_pkt_status)) => {
                info!("rx successful, pkt status: {:?}", rx_pkt_status);
                let valid_data = &receiving_buffer[..len as usize];
                match from_bytes::<SensorData>(valid_data) {
                    Ok(packet) => {
                        info!("Got packet!");
                        if packet == expected_packet {
                            info!("SUCCESS: Packets match");
                        } else {
                            error!("ERROR: Packets do not match!");
                            warn!(" Expected: {:?}", expected_packet);
                            warn!(" Received: {:?}", packet);
                        }
                    }
                    Err(e) => error!("Deserialization failed: {:?}", e),
                }
                // } else {
                //     info!(
                //         "rx unknown packet, status: {:?}: {:?}",
                //         rx_pkt_status, receiving_buffer
                //     );
                // }
            }
            Err(err) => match err {
                RadioError::ReceiveTimeout => continue,
                _ => error!("Error in receiving_buffer: {:?}", err),
            },
        }
    }
}

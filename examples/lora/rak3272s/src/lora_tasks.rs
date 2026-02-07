use crate::iv;
use defmt::{error, info, warn};
// use embassy_embedded_hal::shared_bus::asynch::spi;
// use embassy_futures::select::{Either, select};
use embassy_stm32::{
    gpio::Output,
    spi::{Spi, mode::Master},
};

use embassy_futures::select::{Either, select};
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, channel};
use embassy_time::Delay;

use embassy_stm32::mode::Async;
use lora_phy::mod_params::RadioError;
use lora_phy::mod_params::{Bandwidth, CodingRate, SpreadingFactor};
use lora_phy::sx126x::Stm32wl;
use lora_phy::sx126x::Sx126x;
use lora_phy::{LoRa, RxMode};
use must_hop::SensorData;
use postcard::{from_bytes, to_slice};
use {defmt_rtt as _, panic_probe as _};

const LORA_FREQUENCY_IN_HZ: u32 = 868_000_000; // warning: set this appropriately for the region
type Stm32wlLoRa<'d, CM> = LoRa<
    Sx126x<
        iv::SubghzSpiDevice<Spi<'d, Async, CM>>,
        iv::Stm32wlInterfaceVariant<Output<'d>>,
        Stm32wl,
    >,
    Delay,
>;

#[allow(
    clippy::large_stack_frames,
    reason = "This is the main task, so large size is okay"
)]
#[allow(unused_variables)]
#[embassy_executor::task]
pub async fn lora_task(
    mut lora: Stm32wlLoRa<'static, Master>,
    rx: channel::Receiver<'static, ThreadModeRawMutex, SensorData, 3>,
) {
    loop {
        let sf = SpreadingFactor::_12;
        let bw = Bandwidth::_500KHz;
        let cr = CodingRate::_4_8;
        let mut receiving_buffer = [00u8; 100];
        let mdltn_params = {
            match lora.create_modulation_params(sf, bw, cr, LORA_FREQUENCY_IN_HZ) {
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
        let either = select(rx.receive(), lora.rx(&rx_pkt_params, &mut receiving_buffer)).await;
        match either {
            Either::First(sensor_data) => {
                let mdltn_params = {
                    match lora.create_modulation_params(sf, bw, cr, LORA_FREQUENCY_IN_HZ) {
                        Ok(mp) => mp,
                        Err(err) => {
                            error!("Radio error = {}", err);
                            continue;
                        }
                    }
                };
                let mut tx_pkt_params = {
                    match lora.create_tx_packet_params(8, false, true, false, &mdltn_params) {
                        Ok(pp) => pp,
                        Err(err) => {
                            error!("Radio error = {}", err);
                            continue;
                        }
                    }
                };
                let mut buffer = [0u8; 32];
                let used_slice = match to_slice(&sensor_data, &mut buffer) {
                    Ok(slice) => slice,
                    Err(e) => {
                        error!("Serialization failed: {:?}", e);
                        continue;
                    }
                };

                if let Err(err) = lora
                    .prepare_for_tx(&mdltn_params, &mut tx_pkt_params, 20, used_slice)
                    .await
                {
                    error!("Radio error = {}", err);
                    continue;
                }

                match lora.tx().await {
                    Ok(()) => {
                        info!("TX DONE");
                    }
                    Err(err) => {
                        error!("Radio error = {}", err);
                        continue;
                    }
                };

                match lora.sleep(false).await {
                    Ok(()) => info!("Sleep successful"),
                    Err(err) => error!("Sleep unsuccessful = {}", err),
                }
            }
            Either::Second(conn) => {
                let expected_packet = SensorData {
                    device_id: 42,
                    temperate: 23.5,
                    voltage: 3.3,
                    acceleration_x: 1.2,
                };
                match conn {
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
                    }
                    Err(err) => match err {
                        RadioError::ReceiveTimeout => continue,
                        _ => error!("Error in receiving_buffer: {:?}", err),
                    },
                }
            }
        }
    }
}

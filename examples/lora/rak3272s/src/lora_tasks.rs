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
use lora_phy::mod_params::{Bandwidth, CodingRate, SpreadingFactor};
use lora_phy::mod_params::{PacketStatus, RadioError};
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

struct TransmitParameters {
    sf: SpreadingFactor,
    bw: Bandwidth,
    cr: CodingRate,
    lora_hz: u32,
    pre_amp: u16,
    imp_hed: bool,
    max_pack_len: usize,
    crc: bool,
    iq: bool,
}

const MAX_PACK_LEN: usize = 100;

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
    let sf = SpreadingFactor::_12;
    let bw = Bandwidth::_500KHz;
    let cr = CodingRate::_4_8;
    loop {
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
        let mut receiving_buffer = [00u8; MAX_PACK_LEN];
        let mdltn_params = {
            match lora.create_modulation_params(tp.sf, tp.bw, tp.cr, tp.lora_hz) {
                Ok(mp) => mp,
                Err(err) => {
                    info!("Radio error = {}", err);
                    continue;
                }
            }
        };

        let rx_pkt_params = {
            match lora.create_rx_packet_params(
                tp.pre_amp,
                tp.imp_hed,
                tp.max_pack_len as u8,
                tp.crc,
                tp.iq,
                &mdltn_params,
            ) {
                Ok(pp) => pp,
                Err(err) => {
                    info!("Radio error = {}", err);
                    continue;
                }
            }
        };

        // TODO: Is it a proble using single here? Should it be continouos to not get timeout
        // errors all the time? Can this listening be timed and synchronized for a TDMA?
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
        // Either sensor data should be sent, or a packet is ready to be received
        let either = select(rx.receive(), lora.rx(&rx_pkt_params, &mut receiving_buffer)).await;
        match either {
            Either::First(sensor_data) => {
                if let Err(e) = transmit(&mut lora, sensor_data, tp).await {
                    error!("Error in transmitting: {:?}", e);
                    continue;
                }
            }
            Either::Second(conn) => {
                if let Err(e) = receive(&mut lora, conn, receiving_buffer, tp).await {
                    error!("Error in receing information: {:?}", e);
                    continue;
                }
            }
        }
    }
}

async fn transmit(
    lora: &mut Stm32wlLoRa<'static, Master>,
    sensor_data: SensorData,
    tp: TransmitParameters,
) -> Result<(), RadioError> {
    let mdltn_params = lora.create_modulation_params(tp.sf, tp.bw, tp.cr, tp.lora_hz)?;
    let mut tx_pkt_params = lora.create_tx_packet_params(8, false, true, false, &mdltn_params)?;
    let mut buffer = [0u8; 32];
    let used_slice = match to_slice(&sensor_data, &mut buffer) {
        Ok(slice) => slice,
        Err(e) => {
            error!("Serialization failed: {:?}", e);
            return Err(RadioError::OpError(1));
        }
    };
    lora.prepare_for_tx(&mdltn_params, &mut tx_pkt_params, 20, used_slice)
        .await?;

    lora.tx().await?;
    info!("Transmit successfull!");

    lora.sleep(false).await?;
    info!("Sleep successful");
    Ok(())
}

async fn receive(
    lora: &mut Stm32wlLoRa<'static, Master>,
    conn: Result<(u8, PacketStatus), RadioError>,
    receiving_buffer: [u8; MAX_PACK_LEN],
    tp: TransmitParameters,
) -> Result<(), RadioError> {
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
                        // TODO: Check if this should be retransmitted
                        // if (packet.to != me)
                        // transmit(lora, packet, tp).await;
                        Ok(())
                    } else {
                        error!("ERROR: Packets do not match!");
                        warn!(" Expected: {:?}", expected_packet);
                        warn!(" Received: {:?}", packet);
                        Err(RadioError::ReceiveTimeout)
                    }
                }
                Err(e) => {
                    error!("Deserialization failed: {:?}", e);
                    Err(RadioError::PayloadSizeUnexpected(0))
                }
            }
        }
        Err(err) => match err {
            RadioError::ReceiveTimeout => Ok(()),
            _ => {
                error!("Error in receiving_buffer: {:?}", err);
                Err(err)
            }
        },
    }
}

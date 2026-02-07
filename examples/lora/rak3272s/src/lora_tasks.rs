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
use lora_phy::mod_params::{Bandwidth, CodingRate, SpreadingFactor};
use lora_phy::mod_params::{PacketStatus, RadioError};
use lora_phy::sx126x::Stm32wl;
use lora_phy::sx126x::Sx126x;
use lora_phy::{LoRa, RxMode};
use must_hop::{MHNode, MHPacket};
use postcard::{from_bytes, to_slice};
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

#[derive(Serialize, Deserialize, Debug, PartialEq, defmt::Format)]
pub struct SensorData {
    pub device_id: u8,
    pub temperate: f32,
    pub voltage: f32,
    pub acceleration_x: f32,
}

const MAX_PACK_LEN: usize = 100;

struct LoraRadio {
    lora: Stm32wlLoRa<'static, Master>,
    tp: TransmitParameters,
}

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
                if let Err(e) = receive(&mut lora, conn, &receiving_buffer, tp).await {
                    error!("Error in receing information: {:?}", e);
                    continue;
                }
            }
        }
    }
}

impl MHNode for LoraRadio {
    type Error = RadioError;
    type Payload = SensorData;
    type Connection = Result<(u8, PacketStatus), RadioError>;

    async fn transmit(&mut self, packet: MHPacket) -> Result<(), RadioError> {
        let mdltn_params = self.lora.create_modulation_params(
            self.tp.sf,
            self.tp.bw,
            self.tp.cr,
            self.tp.lora_hz,
        )?;
        let mut tx_pkt_params =
            self.lora
                .create_tx_packet_params(8, false, true, false, &mdltn_params)?;
        let mut buffer = [0u8; 32];
        let used_slice = match to_slice(&packet, &mut buffer) {
            Ok(slice) => slice,
            Err(e) => {
                error!("Serialization failed: {:?}", e);
                return Err(RadioError::OpError(1));
            }
        };
        // Simple listen to talk logic
        loop {
            if self.lora.cad(&mdltn_params).await? {
                info!("cad successfull with activity detected");
                // TODO: Get some random amount of time before continuing loop
            } else {
                info!("cad successfull with NO activity detected");
                break;
            }
        }
        self.lora
            .prepare_for_tx(&mdltn_params, &mut tx_pkt_params, 20, used_slice)
            .await?;

        self.lora.tx().await?;
        info!("Transmit successfull!");

        // NOTE: This might create a delay between transmitting something and being able to receive
        // again
        // lora.sleep(false).await?;
        // info!("Sleep successful");
        Ok(())
    }

    async fn receive(
        &mut self,
        conn: Result<(u8, PacketStatus), RadioError>,
        receiving_buffer: &[u8],
    ) -> Result<SensorData, RadioError> {
        let expected_packet = SensorData {
            device_id: 42,
            temperate: 23.5,
            voltage: 3.3,
            acceleration_x: 1.2,
        };
        // First we check if we actually got something
        let (len, rx_pkt_status) = match conn {
            Ok((len, rx_pkt_status)) => (len, rx_pkt_status),
            Err(err) => match err {
                RadioError::ReceiveTimeout => return Err(err),
                _ => {
                    error!("Error in receiving_buffer: {:?}", err);
                    return Err(err);
                }
            },
        };
        info!("rx successful, pkt status: {:?}", rx_pkt_status);

        // Try to unpack the buffer into expected packet
        let valid_data = &receiving_buffer[..len as usize];
        let packet = match from_bytes::<SensorData>(valid_data) {
            Ok(packet) => packet,
            Err(e) => {
                error!("Deserialization failed: {:?}", e);
                return Err(RadioError::PayloadSizeUnexpected(0));
            }
        };
        info!("Got packet!");

        // TODO: Check if this should be retransmitted
        // if (packet.to != me)
        // transmit(lora, packet, tp).await?;

        // TODO: We can of couse now always expect what the package contents should be..
        if packet == expected_packet {
            info!("SUCCESS: Packets match");

            Ok(packet)
        } else {
            error!("ERROR: Packets do not match!");
            warn!(" Expected: {:?}", expected_packet);
            warn!(" Received: {:?}", packet);
            Err(RadioError::ReceiveTimeout)
        }
    }
}

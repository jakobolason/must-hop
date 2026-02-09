use super::node::{MHNode, MHPacket};
/// This contains node implementations for Lora
/// Sx126x currently
use lora_phy::mod_params::{
    Bandwidth, CodingRate, ModulationParams, PacketParams, SpreadingFactor,
};
use lora_phy::mod_params::{PacketStatus, RadioError};
use lora_phy::mod_traits::RadioKind;
use lora_phy::{DelayNs, LoRa, RxMode};

use defmt::{error, info, warn};
use embassy_time::{Delay, Timer};
use heapless::Vec;
use postcard::{from_bytes, to_slice};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy)]
pub struct TransmitParameters {
    pub sf: SpreadingFactor,
    pub bw: Bandwidth,
    pub cr: CodingRate,
    pub lora_hz: u32,
    pub pre_amp: u16,
    pub imp_hed: bool,
    pub max_pack_len: usize,
    pub crc: bool,
    pub iq: bool,
}

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

enum RadioState {
    Rx,
    Tx,
}

struct LoraNode<'a, RK, DLY>
where
    RK: RadioKind,
    DLY: DelayNs,
{
    lora: &'a mut LoRa<RK, DLY>,
    tp: TransmitParameters,
    rx_pkt_params: PacketParams,
    mdltn_params: ModulationParams,
    radio_state: RadioState,
}

impl<RK, DLY> MHNode for LoraNode<'_, RK, DLY>
where
    RK: RadioKind,
    DLY: DelayNs,
{
    type Error = RadioError;
    type Payload = SensorData;
    type Connection = Result<(u8, PacketStatus), RadioError>;

    // Should transform to Tx if in Rx
    async fn transmit(&mut self, packet: MHPacket) -> Result<(), RadioError> {
        // TODO: Is this necessary?
        let mut tx_pkt_params =
            self.lora
                .create_tx_packet_params(8, false, true, false, &self.mdltn_params)?;
        let mut buffer = [0u8; 32];
        let used_slice = match to_slice(&packet, &mut buffer) {
            Ok(slice) => slice,
            Err(e) => {
                error!("Serialization failed: {:?}", e);
                return Err(RadioError::OpError(1));
            }
        };
        // Simple listen to talk logic
        // TODO: This crashes when in a loop
        // loop {
        info!("preparing for cad ...");
        self.lora.prepare_for_cad(&self.mdltn_params).await?;
        if self.lora.cad(&self.mdltn_params).await? {
            warn!("cad successfull with activity detected");
            // self.lora.sleep(false).await?;
            Timer::after_millis(50).await;
            // TODO: Get some random amount of time before continuing loop
        } else {
            info!("cad successfull with NO activity detected");
            // break;
        }
        // }
        self.lora
            .prepare_for_tx(&self.mdltn_params, &mut tx_pkt_params, 20, used_slice)
            .await?;

        self.lora.tx().await?;
        info!("Transmit successfull!");

        // NOTE: This might create a delay between transmitting something and being able to receive
        // again
        // lora.sleep(false).await?;
        // info!("Sleep successful");
        Ok(())
    }

    // Should transition to Rx if in Tx
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
        let mut buffer = [0u8; MAX_PACK_LEN];
        let used_slice =
            to_slice(&expected_packet, &mut buffer).expect("Coudl not serialize sensor data");
        let payload_bytes =
            Vec::from_slice(used_slice).expect("could not get vec, sensor data was too large");
        let mhpacket = MHPacket {
            destination_id: 1,
            source_id: 2,
            payload: payload_bytes,
            hop_count: 0,
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
        let packet = match from_bytes::<MHPacket>(valid_data) {
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
        if packet == mhpacket {
            info!("SUCCESS: Packets match");
            let payload = match from_bytes::<SensorData>(&packet.payload) {
                Ok(packet) => packet,
                Err(e) => {
                    error!("Deserialization failed: {:?}", e);
                    return Err(RadioError::PayloadSizeUnexpected(0));
                }
            };
            Ok(payload)
        } else {
            error!("ERROR: Packets do not match!");
            warn!(" Expected: {:?}", expected_packet);
            warn!(" Received: {:?}", packet);
            Err(RadioError::ReceiveTimeout)
        }
    }
}

impl<'a, RK, DLY> LoraNode<'a, RK, DLY>
where
    RK: RadioKind,
    DLY: DelayNs,
{
    fn new(lora: &'a mut LoRa<RK, DLY>, tp: TransmitParameters) -> Result<Self, RadioError> {
        let mdltn_params = lora.create_modulation_params(tp.sf, tp.bw, tp.cr, tp.lora_hz)?;

        let rx_pkt_params = lora.create_rx_packet_params(
            tp.pre_amp,
            tp.imp_hed,
            tp.max_pack_len as u8,
            tp.crc,
            tp.iq,
            &mdltn_params,
        )?;
        Ok(Self {
            lora,
            tp,
            rx_pkt_params,
            mdltn_params,
            radio_state: RadioState::Rx,
        })
    }

    async fn prepare_for_rx(&mut self) -> Result<(), RadioError> {
        // TODO: Is it a proble using single here? Should it be continouos to not get timeout
        // errors all the time? Can this listening be timed and synchronized for a TDMA?
        self.lora
            .prepare_for_rx(RxMode::Continuous, &self.mdltn_params, &self.rx_pkt_params)
            .await
    }

    async fn listen(&mut self, rec_buf: &mut [u8]) -> Result<(u8, PacketStatus), RadioError> {
        if let RadioState::Tx = self.radio_state {
            self.prepare_for_rx().await?;
        }
        self.lora.rx(&self.rx_pkt_params, rec_buf).await
    }
}

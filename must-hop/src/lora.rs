/// This contains node implementations for Lora
use super::node::{MHNode, MHPacket};
use lora_phy::mod_params::{
    Bandwidth, CodingRate, ModulationParams, PacketParams, SpreadingFactor,
};
use lora_phy::mod_params::{PacketStatus, RadioError};
use lora_phy::mod_traits::RadioKind;
use lora_phy::{DelayNs, LoRa, RxMode};

use defmt::{error, trace};
use embassy_time::Instant;
use postcard::{from_bytes, to_slice};
use serde::{Deserialize, Serialize};

// Approximately 1 second?
const RECEIVE_TIMEOUT: u16 = 100;

/// Example of payload
#[derive(Serialize, Deserialize, Debug, PartialEq, defmt::Format)]
pub struct SensorData {
    pub device_id: u8,
    pub temperate: f32,
    pub voltage: f32,
    pub acceleration_x: f32,
}

/// Parameters that define send and receive parameters
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

/// Unsure whether this will be used
pub enum RadioState {
    Rx,
    Tx,
}

/// A node implementatino for lora, where a LoRa interface variant type has to be implemented to
/// use. An IV for a SX126x is shown in `/examples`
pub struct LoraNode<'a, RK, DLY, const SIZE: usize>
where
    RK: RadioKind,
    DLY: DelayNs,
{
    lora: &'a mut LoRa<RK, DLY>,
    tp: TransmitParameters,
    pkt_params: PacketParams,
    mdltn_params: ModulationParams,
    radio_state: RadioState,
}

impl<RK, DLY, const SIZE: usize> MHNode<SIZE> for LoraNode<'_, RK, DLY, SIZE>
where
    RK: RadioKind,
    DLY: DelayNs,
{
    type Error = RadioError;
    type Connection = Result<(u8, PacketStatus), RadioError>;
    type Duration = u16;

    // Should transform to Tx if in Rx
    async fn transmit(&mut self, packet: MHPacket<SIZE>) -> Result<(), RadioError> {
        // TODO: Is this necessary?
        let now = Instant::now();

        // TODO: This should be the const generic max pack len
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
        // trace!("preparing for cad ...");
        // self.lora.prepare_for_cad(&self.mdltn_params).await?;
        // if self.lora.cad(&self.mdltn_params).await? {
        //     warn!("cad successfull with activity detected");
        //     // self.lora.sleep(false).await?;
        //     Timer::after_millis(50).await;
        //     // TODO: Get some random amount of time before continuing loop
        // } else {
        //     trace!("cad successfull with NO activity detected");
        //     // break;
        // }
        // }
        let before_tx = Instant::now();
        self.lora
            .prepare_for_tx(&self.mdltn_params, &mut self.pkt_params, 20, used_slice)
            .await?;

        self.lora.tx().await?;
        trace!("Transmit successfull!");
        let after = Instant::now();
        let tx_dur = after - now;
        let only_tx = after - before_tx;
        trace!(
            "[TX DURATION] millis: {},\t ticks: {}",
            tx_dur.as_millis(),
            tx_dur
        );
        trace!(
            "[TX DURATION] millis: {},\t ticks: {}",
            only_tx.as_millis(),
            only_tx
        );
        // Takes around 1.9 seconds for full transmit function, and 1.6 for just transmitting

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
    ) -> Result<MHPacket<SIZE>, RadioError> {
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
        trace!("rx successful, pkt status: {:?}", rx_pkt_status);

        // Try to unpack the buffer into expected packet
        let valid_data = &receiving_buffer[..len as usize];
        let packet = match from_bytes::<MHPacket<SIZE>>(valid_data) {
            Ok(packet) => packet,
            Err(e) => {
                error!("Deserialization failed: {:?}", e);
                return Err(RadioError::PayloadSizeUnexpected(0));
            }
        };
        trace!("Got packet!");

        // TODO: Check if this should be retransmitted
        // if (packet.to != me)
        // transmit(lora, packet, tp).await?;

        Ok(packet)
    }

    async fn listen(
        &mut self,
        rec_buf: &mut [u8; SIZE],
        with_timeout: bool,
    ) -> Result<Self::Connection, RadioError> {
        // if let RadioState::Tx = self.radio_state {
        //     self.prepare_for_rx().await?;
        // }
        let rec_mode = match with_timeout {
            true => RxMode::Single(RECEIVE_TIMEOUT),
            false => RxMode::Continuous,
        };
        self.prepare_for_rx(rec_mode).await?;
        Ok(self.lora.rx(&self.pkt_params, rec_buf).await)
    }
}

impl<'a, RK, DLY, const N: usize> LoraNode<'a, RK, DLY, N>
where
    RK: RadioKind,
    DLY: DelayNs,
{
    pub fn new(lora: &'a mut LoRa<RK, DLY>, tp: TransmitParameters) -> Result<Self, RadioError> {
        let mdltn_params = lora.create_modulation_params(tp.sf, tp.bw, tp.cr, tp.lora_hz)?;

        let pkt_params = lora.create_rx_packet_params(
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
            pkt_params,
            mdltn_params,
            radio_state: RadioState::Rx,
        })
    }

    pub async fn prepare_for_rx(&mut self, rx_mode: RxMode) -> Result<(), RadioError> {
        // TODO: Is it a proble using single here? Should it be continouos to not get timeout
        // errors all the time? Can this listening be timed and synchronized for a TDMA?
        self.lora
            .prepare_for_rx(rx_mode, &self.mdltn_params, &self.pkt_params)
            .await
    }
}

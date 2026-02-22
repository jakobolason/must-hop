/// This contains node implementations for Lora
use super::node::{MHNode, MHPacket};
use lora_phy::mod_params::{
    Bandwidth, CodingRate, ModulationParams, PacketParams, SpreadingFactor,
};
use lora_phy::mod_params::{PacketStatus, RadioError};
use lora_phy::mod_traits::RadioKind;
use lora_phy::{DelayNs, LoRa, RxMode};

#[cfg(not(feature = "in_std"))]
use defmt::{error, trace};
#[cfg(feature = "in_std")]
use log::{error, trace};

use embassy_time::Instant;
use heapless::Vec;
use postcard::{from_bytes, to_slice};
use serde::{Deserialize, Serialize};

// Approximately 1 second?
const RECEIVE_TIMEOUT: u16 = 100;
// TODO: Should this be a const generic for the user to set? Perhaps a default value?
const TRANSMISSION_BUFFER: usize = 256; // The radio can receive 256 bytes to transmit

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
pub struct LoraNode<'a, RK, DLY, const SIZE: usize, const LEN: usize>
where
    RK: RadioKind,
    DLY: DelayNs,
{
    lora: &'a mut LoRa<RK, DLY>,
    _tp: TransmitParameters,
    pkt_params: PacketParams,
    mdltn_params: ModulationParams,
}

impl<RK, DLY, const SIZE: usize, const LEN: usize> MHNode<SIZE, LEN>
    for LoraNode<'_, RK, DLY, SIZE, LEN>
where
    RK: RadioKind,
    DLY: DelayNs,
{
    type Error = RadioError;
    type Connection = Result<(u8, PacketStatus), RadioError>;
    type ReceiveBuffer = [u8; SIZE];
    type Duration = u16;

    async fn transmit(&mut self, packets: &[MHPacket<SIZE>]) -> Result<(), RadioError> {
        let now = Instant::now();

        // TODO: Can this be made opt-in? Such that individual transmission is possible?
        let mut buffer = [0u8; TRANSMISSION_BUFFER];
        trace!("BUFFER SIZE IS: {}", SIZE);
        let used_slice = match to_slice(&packets, &mut buffer) {
            Ok(slice) => slice,
            Err(e) => {
                error!("Serialization failed: {:?}", e);
                return Err(RadioError::OpError(1));
            }
        };
        trace!("used slice size is {}", used_slice.len());
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
        rec_buf: &[u8; SIZE],
    ) -> Result<Vec<MHPacket<SIZE>, LEN>, RadioError> {
        // First we check if we actually got something
        let (len, _rx_pkt_status) = match conn {
            Ok((len, rx_pkt_status)) => (len, rx_pkt_status),
            Err(err) => match err {
                RadioError::ReceiveTimeout => return Err(err),
                _ => {
                    error!("Error in receiving_buffer: {:?}", err);
                    return Err(err);
                }
            },
        };
        // trace!("rx successful, pkt status: {:?}", rx_pkt_status);

        // Try to unpack the buffer into expected packet
        let valid_data = &rec_buf[..len as usize];
        let packets = match from_bytes::<Vec<MHPacket<SIZE>, LEN>>(valid_data) {
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

        Ok(packets)
    }

    async fn listen(
        &mut self,
        rec_buf: &mut [u8; SIZE],
        with_timeout: bool,
    ) -> Result<Self::Connection, RadioError> {
        let rec_mode = match with_timeout {
            true => RxMode::Single(RECEIVE_TIMEOUT),
            false => RxMode::Continuous,
        };
        self.prepare_for_rx(rec_mode).await?;
        Ok(self.lora.rx(&self.pkt_params, rec_buf).await)
    }
}

impl<'a, RK, DLY, const N: usize, const LEN: usize> LoraNode<'a, RK, DLY, N, LEN>
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
            _tp: tp,
            pkt_params,
            mdltn_params,
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

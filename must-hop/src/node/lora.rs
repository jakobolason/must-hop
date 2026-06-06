use crate::node::RxPacket;

/// This contains node implementations for Lora
use super::{MHNode, MHPacket};
use lora_modulation::BaseBandModulationParams;
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

use core::time::Duration;
use embassy_time::Instant;
use heapless::Vec;
use postcard::{from_bytes, to_slice};
use serde::{Deserialize, Serialize};

// Approximately 1 second?
// const RECEIVE_TIMEOUT: u16 = 255;
// TODO: Should this be a const generic for the user to set? Perhaps a default value?
const TRANSMISSION_BUFFER: usize = 256; // The radio can receive 256 bytes to transmit

/// Example of payload
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[cfg_attr(not(feature = "in_std"), derive(defmt::Format))]
pub struct SensorData {
    pub device_id: u8,
    pub temperate: f32,
    pub voltage: f32,
    pub acceleration_x: f32,
}

/// Parameters that define send and receive parameters
#[derive(Clone, Copy)]
pub struct RadioPackParams {
    pub pre_amp: u16,
    pub imp_hed: bool,
    pub max_pack_len: usize,
    pub crc: bool,
    pub iq: bool,
}

#[derive(Clone, Copy)]
pub struct RatioModParams {
    pub sf: SpreadingFactor,
    pub bw: Bandwidth,
    pub cr: CodingRate,
    pub lora_hz: u32,
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
    tx_pkt_params: PacketParams,
    rx_pkt_params: PacketParams,
    mdltn_params: ModulationParams,
    /// Used if different parameters should be use for the Rx
    alt_mdtln_params: Option<ModulationParams>,
    done_instant: Option<Instant>,
    /// Used to calculate ToA
    bb_mod: BaseBandModulationParams,
    pa_len: Option<u8>,
    explicit_header: bool,
}

/// Calculated in calc_tau_spi
const INTERCEPT: u64 = 690;
const SLOPE: u64 = 1;

impl<'a, RK, DLY, const SIZE: usize, const LEN: usize> LoraNode<'a, RK, DLY, SIZE, LEN>
where
    RK: RadioKind,
    DLY: DelayNs,
{
    /// Returns the (preamble, packet) ToA
    fn calc_toa(&self, bytes: u8) -> u32 {
        // Using the formula to calculate time-on-air

        self.bb_mod
            .time_on_air_us(self.pa_len, self.explicit_header, bytes)
    }

    fn avg_slice_delay(&self, payload_len: u8) -> u64 {
        INTERCEPT + SLOPE * payload_len as u64
    }
}

// In dBm, in low power mode is clamped between [-17, 15]
// in HighPowerPA, clamped between [-9, 22]
const OUTPUT_POWER: i32 = 7;

impl<RK, DLY, const SIZE: usize, const LEN: usize> MHNode<SIZE, LEN>
    for LoraNode<'_, RK, DLY, SIZE, LEN>
where
    RK: RadioKind,
    DLY: DelayNs,
{
    type Error = RadioError;
    type Connection = Result<(u8, PacketStatus), RadioError>;
    type ReceiveBuffer = [u8; TRANSMISSION_BUFFER];

    /// Slices up packets into bytes and transmits, does not call `lora.sleep`
    async fn transmit(&mut self, packets: &[MHPacket<SIZE>]) -> Result<(), RadioError> {
        let mut buffer = [0u8; TRANSMISSION_BUFFER];
        let used_slice = match to_slice(&packets, &mut buffer) {
            Ok(slice) => slice,
            Err(e) => {
                error!("Serialization failed: {:?}", e);
                return Err(RadioError::OpError(1));
            }
        };

        let before = Instant::now();
        self.lora
            .prepare_for_tx(
                &self.mdltn_params,
                &mut self.tx_pkt_params,
                OUTPUT_POWER,
                used_slice,
            )
            .await?;
        let now_sending = Instant::now();
        self.lora.tx().await?;
        let after_sending = Instant::now();
        trace!(
            "[TAU_SLICE_POST] |{}|{}|",
            now_sending.as_micros(),
            used_slice.len()
        );
        trace!(
            "ToA for node: {}",
            after_sending.as_millis() - before.as_millis()
        );
        // NOTE: This might create a delay between transmitting something and being able to receive
        // again
        // lora.sleep(false).await?;
        // info!("Sleep successful");
        Ok(())
    }

    async fn receive(
        &mut self,
        conn: Result<(u8, PacketStatus), RadioError>,
        rec_buf: &[u8; TRANSMISSION_BUFFER],
    ) -> Result<(Vec<MHPacket<SIZE>, LEN>, RxPacket), RadioError> {
        // First we check if we actually got something
        let rx_hardware_timestamp = match self.done_instant {
            Some(ins) => ins,
            None => Instant::now(),
        };
        trace!("received pkts!");
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
        // let estimated_toa = self.calc_toa(len);

        let rx_pkt = RxPacket {
            // preamble_instant: self.preamble_instant.take(),
            // preamble_instant: None,
            rx_done_instant: rx_hardware_timestamp,
            payload_size: len,
            // estimated_toa,
        };

        Ok((packets, rx_pkt))
    }

    async fn listen(
        &mut self,
        rec_buf: &mut [u8; TRANSMISSION_BUFFER],
        with_timeout: Option<Duration>,
    ) -> Result<Self::Connection, RadioError> {
        self.prepare_for_rx(RxMode::Continuous).await?;
        self.done_instant = None;
        // TODO: Remove this? I'm not using it anymore, don't plan to do
        let get_done_instant = || {
            if self.done_instant.is_none() {
                self.done_instant = Some(Instant::now())
            }
        };
        match with_timeout {
            Some(timeout) => {
                match embassy_time::with_timeout(
                    embassy_time::Duration::from_micros(timeout.as_micros() as u64),
                    self.lora
                        .rx(&self.rx_pkt_params, rec_buf /*, get_done_instant*/),
                )
                .await
                {
                    Ok(rx_result) => Ok(rx_result),
                    Err(_) => Err(RadioError::ReceiveTimeout),
                }
            }
            None => Ok(self
                .lora
                .rx(&self.rx_pkt_params, rec_buf /*, get_done_instant*/)
                .await),
        }
    }

    fn calc_tx_delay(&self, payload_len: usize) -> u64 {
        self.calc_toa(payload_len as u8) as u64 + self.avg_slice_delay(payload_len as u8)
    }
}

impl<'a, RK, DLY, const N: usize, const LEN: usize> LoraNode<'a, RK, DLY, N, LEN>
where
    RK: RadioKind,
    DLY: DelayNs,
{
    /// SF5 and SF6 require a minimum preamble of 12 symbols on the SX1262 for reliable
    /// preamble detection. SF7–SF12 use the standard 8-symbol preamble.
    fn min_preamble_for_sf(sf: SpreadingFactor, requested: u16) -> u16 {
        match sf {
            SpreadingFactor::_5 | SpreadingFactor::_6 => requested.max(12),
            _ => requested,
        }
    }

    /// Takes a LoRa radio, transmit parameters and optionally receive parameters. If receive
    /// parameters are not given, then tp are used for both Tx and Rx
    pub fn new(
        lora: &'a mut LoRa<RK, DLY>,
        pack_params: RadioPackParams,
        tx_mod: RatioModParams,
        rx_opt: Option<RatioModParams>,
    ) -> Result<Self, RadioError> {
        let mdltn_params =
            lora.create_modulation_params(tx_mod.sf, tx_mod.bw, tx_mod.cr, tx_mod.lora_hz)?;

        let tx_preamble = Self::min_preamble_for_sf(tx_mod.sf, pack_params.pre_amp);
        let tx_pkt_params = lora.create_rx_packet_params(
            tx_preamble,
            pack_params.imp_hed,
            pack_params.max_pack_len as u8,
            pack_params.crc,
            pack_params.iq,
            &mdltn_params,
        )?;

        let alt_mdtln_params = if let Some(rx_mod) = rx_opt {
            let rx_mdltn_params =
                lora.create_modulation_params(rx_mod.sf, rx_mod.bw, rx_mod.cr, rx_mod.lora_hz)?;
            Some(rx_mdltn_params)
        } else {
            None
        };
        let rx_mdltn_params = alt_mdtln_params.as_ref().unwrap_or(&mdltn_params);

        // Derive RX preamble from the RX SF (may differ from TX if alt modulation is set)
        let rx_sf = rx_opt.map(|r| r.sf).unwrap_or(tx_mod.sf);
        let rx_preamble = Self::min_preamble_for_sf(rx_sf, pack_params.pre_amp);
        let rx_pkt_params = lora.create_rx_packet_params(
            rx_preamble,
            pack_params.imp_hed,
            pack_params.max_pack_len as u8,
            pack_params.crc,
            pack_params.iq,
            rx_mdltn_params,
        )?;

        let bb_mod = BaseBandModulationParams::new(tx_mod.sf, tx_mod.bw, tx_mod.cr);

        Ok(Self {
            lora,
            tx_pkt_params,
            rx_pkt_params,
            mdltn_params,
            alt_mdtln_params,
            done_instant: None,
            bb_mod,
            pa_len: Some(pack_params.pre_amp as u8),
            explicit_header: !pack_params.imp_hed,
        })
    }

    pub async fn prepare_for_rx(&mut self, rx_mode: RxMode) -> Result<(), RadioError> {
        // TODO: Is it a proble using single here? Should it be continouos to not get timeout
        // errors all the time? Can this listening be timed and synchronized for a TDMA?
        let mdltn_params = match &self.alt_mdtln_params {
            Some(p) => p,
            None => &self.mdltn_params,
        };
        self.lora
            .prepare_for_rx(rx_mode, mdltn_params, &self.rx_pkt_params)
            .await
    }
}

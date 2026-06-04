use std::collections::VecDeque;

use embassy_time::{Duration, Instant, Timer};
use log::{error, trace};
use lora_modulation::BaseBandModulationParams;
use loragw::{Concentrator, Error, Running, RxPacket, TxPacket, TxPacketLoRa, TxStatus};
use must_hop::{MHNode, MHPacket};
use postcard::to_slice;

const SIZE: usize = 128;
const LEN: usize = 5; // Lets keep it the same as the nodes, make it simple
const LORA_FREQ: usize = 868_700_000;
// Max size that radio can send at all
const TRANSMISSION_BUFFER: usize = 256;

#[derive(Clone)]
pub struct PacketParams {
    /// Center frequency to transmit on.
    pub freq: u32,
    /// When to send this packet.
    pub mode: loragw::TxMode,
    /// Which radio to transmit on.
    pub radio: loragw::FrontRadio,
    /// TX power (in dBm).
    pub power: i8,
    /// Modulation bandwidth.
    pub bandwidth: loragw::Bandwidth,
    /// Spreading factor to use with this packet.
    pub spreading: loragw::Spreading,
    /// Error-correcting-code of the packet.
    pub coderate: loragw::Coderate,
    /// Invert signal polarity for orthogonal downlinks.
    pub invert_polarity: bool,
    /// Preamble length.
    /// Use `None` for default.
    pub preamble: Option<u16>,
    /// Do not send a CRC in the packet.
    pub omit_crc: bool,
    /// Enable implicit header mode.
    pub implicit_header: bool,
}
impl Default for PacketParams {
    fn default() -> Self {
        Self {
            freq: LORA_FREQ as u32,
            mode: loragw::TxMode::Immediate,
            radio: loragw::FrontRadio::R0,
            power: 14,
            bandwidth: loragw::Bandwidth::BW125kHz,
            spreading: loragw::Spreading::SF7,
            coderate: loragw::Coderate::Cr4_8,
            // Do not invert polarity (Gateways only invert if talking to standard
            // LoRaWAN nodes. If talking Gateway-to-Gateway, this usually stays false).
            invert_polarity: false,
            // Standard LoRa preamble length
            preamble: Some(8),
            // preamble: None,
            // Always want CRC for data integrity in Mesh networks
            omit_crc: false,
            // Explicit header mode is standard
            implicit_header: false,
        }
    }
}

impl From<PacketParams> for TxPacketLoRa {
    fn from(params: PacketParams) -> Self {
        Self {
            freq: params.freq,
            mode: params.mode,
            radio: params.radio,
            power: params.power,
            bandwidth: params.bandwidth,
            spreading: params.spreading,
            coderate: params.coderate,
            invert_polarity: params.invert_polarity,
            preamble: params.preamble,
            omit_crc: params.omit_crc,
            implicit_header: params.implicit_header,
            payload: Vec::new(),
        }
    }
}

pub struct GWNode {
    radio: Concentrator<Running>,
    fetched_packets: VecDeque<RxPacket>,
    pkt_params: PacketParams,
}

/// Calculated in calc_tau_spi
const INTERCEPT: u64 = 1478;
const SLOPE: u64 = 4;

impl GWNode {
    pub fn new(concentrator: Concentrator<Running>, pkt_params: PacketParams) -> Self {
        Self {
            radio: concentrator,
            fetched_packets: VecDeque::new(),
            pkt_params,
        }
    }
    fn to_tx_packet(&self, packets: &[MHPacket<SIZE>]) -> Result<(TxPacket, usize), Error> {
        let mut buffer = [0u8; TRANSMISSION_BUFFER];
        let used_slice = match to_slice(&packets, &mut buffer) {
            Ok(slice) => slice,
            Err(e) => {
                log::error!("Serialization failed: {:?}", e);
                return Err(Error::Data);
            }
        };
        Ok((
            TxPacket::LoRa(TxPacketLoRa {
                payload: used_slice.to_vec(),
                ..self.pkt_params.clone().into()
            }),
            used_slice.len(),
        ))
    }

    /// Calculates the ms of ToA in microseconds given a specific payload lengt and uses it's previous transmit
    /// parameters
    pub fn calc_toa(&self, payload_len: u8) -> u32 {
        // Using the formula to calculate time-on-air
        let bb_mod = BaseBandModulationParams::new(
            self.pkt_params.spreading.into(),
            self.pkt_params.bandwidth.into(),
            self.pkt_params.coderate.into(),
        );
        bb_mod.time_on_air_us(
            self.pkt_params.preamble.map(|p| p as u8).or(Some(8)),
            !self.pkt_params.implicit_header,
            payload_len,
        )
    }
    fn avg_slice_delay(&self, payload_len: usize) -> u64 {
        // INTERCEPT + SLOPE * payload_len as u64
        0
    }
}

impl MHNode<SIZE, LEN> for GWNode {
    type Error = loragw::Error;
    type Connection = ();
    type ReceiveBuffer = Option<RxPacket>;

    async fn transmit(&mut self, packets: &[MHPacket<SIZE>]) -> Result<(), Self::Error> {
        let (tx_pkt, payload_size) = self.to_tx_packet(packets)?;
        while self.radio.transmit_status()? != TxStatus::Free {
            embassy_time::Timer::after(Duration::from_millis(5)).await;
        }
        self.radio.transmit(tx_pkt)?;
        let after = Instant::now();
        trace!(
            "[TAU_SLICE_POST] | {} | {} |",
            after.as_micros(),
            payload_size
        );
        let toa = self.calc_toa(payload_size as u8);
        embassy_time::Timer::after(Duration::from_micros(toa as u64)).await;
        Ok(())
    }

    /// The returned instant is the first heartbeat packet captured
    async fn receive(
        &mut self,
        _conn: Self::Connection,
        rec_buf: &Self::ReceiveBuffer,
    ) -> Result<(heapless::Vec<MHPacket<SIZE>, LEN>, must_hop::RxPacket), Self::Error> {
        // This is a hack, but we only want one entry
        let Some(pkt) = rec_buf else {
            return Err(loragw::Error::Generic);
        };

        let mut rec_packets: heapless::Vec<MHPacket<SIZE>, LEN> = heapless::Vec::new();

        let pkt = match pkt {
            RxPacket::LoRa(rx_packet) => rx_packet,
            RxPacket::FSK(_) => return Err(loragw::Error::Generic),
        };
        let raw_bytes = &pkt.payload;
        // log::info!(
        //     "Received LoRa Packet | SF: {:?}, BW: {:?}, Freq: {} Hz, RSSI: {:.1} dBm, SNR: {:.1} dB",
        //     pkt.spreading,
        //     pkt.bandwidth,
        //     pkt.freq,
        //     pkt.rssi,
        //     pkt.snr
        // );

        let packets = postcard::from_bytes::<heapless::Vec<MHPacket<SIZE>, LEN>>(raw_bytes)
            .map_err(|_| {
                error!("Could not convert to bytes!");
                loragw::Error::Generic
            })?;
        // log::info!(
        //     "SUCCESS !!!! Received amount of packets: {:?}",
        //     packets.len()
        // );
        let now_host = Instant::now();
        let rx_heartbeat_timestamp = if let Ok(now_radio) = self.radio.get_instcnt() {
            // Calculate our local ticks when this was captured
            let pkt_hw_us = pkt.timestamp.as_micros() as u32;
            let radio_now_us = now_radio.as_micros() as u32;
            let age_us = radio_now_us.wrapping_sub(pkt_hw_us);
            now_host
                .checked_sub(embassy_time::Duration::from_micros(age_us as u64))
                .unwrap_or(now_host)
        } else {
            now_host
        };
        for packet in packets {
            // log::info!("Packet {:?}", packet);
            rec_packets.push(packet).map_err(|_| loragw::Error::Data)?;
        }

        Ok((
            rec_packets,
            must_hop::RxPacket {
                // preamble_instant: None,
                rx_done_instant: rx_heartbeat_timestamp,
                payload_size: pkt.payload.len() as u8,
                // estimated_toa: self.calc_toa(pkt),
            },
        ))
    }

    async fn listen(
        &mut self,
        rec_buf: &mut Self::ReceiveBuffer,
        with_timeout: Option<core::time::Duration>,
    ) -> Result<Self::Connection, Self::Error> {
        let start_time = Instant::now();
        // let timeout = Duration::from_secs(1);
        // TODO: Make this a select? So either select the loop { radio.receive } or timeout
        loop {
            if let Some(pkt) = self.fetched_packets.pop_front() {
                *rec_buf = Some(pkt);
                return Ok(());
            }
            if let Some(packets) = self.radio.receive()? {
                self.fetched_packets.extend(packets);
                continue;
            }
            if let Some(timeout) = with_timeout
                && start_time.elapsed() >= Duration::from_micros(timeout.as_micros() as u64)
            {
                // TODO: Need better error type here
                // return Err(loragw::Error::Busy);
                return Ok(());
            }
            Timer::after(Duration::from_millis(5)).await;
        }
    }

    /// Calculates the transmission delay associated with this radio. Also the LoRa ToA is
    /// calculated in mciroseconds.
    fn calc_tx_delay(&self, payload_len: usize) -> u64 {
        self.calc_toa(payload_len as u8) as u64 + self.avg_slice_delay(payload_len)
    }
}

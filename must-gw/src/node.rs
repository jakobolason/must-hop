use std::collections::VecDeque;

use embassy_time::{Duration, Instant, Timer};
use log::trace;
use loragw::{Concentrator, Error, Running, RxPacket, TxPacket, TxPacketLoRa, TxStatus};
use must_hop::node::{MHNode, MHPacket, PacketType};
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
            // preamble: Some(8),
            preamble: None,
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
            power: params.radio as i8,
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
    /// Kind of a hack to do it like this, perhaps MHNODE will be altered?
    fetched_packets: VecDeque<RxPacket>,
    pkt_params: PacketParams,
}

impl GWNode {
    pub fn new(concentrator: Concentrator<Running>) -> Self {
        Self {
            radio: concentrator,
            fetched_packets: VecDeque::new(),
            pkt_params: PacketParams::default(),
        }
    }
    fn to_tx_packet(&self, packets: &[MHPacket<SIZE>]) -> Result<TxPacket, Error> {
        let mut buffer = [0u8; TRANSMISSION_BUFFER];
        log::info!("BUFFER SIZE IS: {}", SIZE);
        let used_slice = match to_slice(&packets, &mut buffer) {
            Ok(slice) => slice,
            Err(e) => {
                log::error!("Serialization failed: {:?}", e);
                return Err(Error::Data);
            }
        };
        Ok(TxPacket::LoRa(TxPacketLoRa {
            payload: used_slice.to_vec(),
            ..self.pkt_params.clone().into()
        }))
    }
}

impl MHNode<SIZE, LEN> for GWNode {
    type Error = loragw::Error;
    type Connection = ();
    type ReceiveBuffer = Option<RxPacket>;

    async fn transmit(&mut self, packets: &[MHPacket<SIZE>]) -> Result<(), Self::Error> {
        packets
            .iter()
            .for_each(|p| trace!(" !!!! Sending packet id: {}", p.packet_id));
        let before = Instant::now();
        let tx_pkt = self.to_tx_packet(packets)?;
        while self.radio.transmit_status()? != TxStatus::Free {
            embassy_time::Timer::after(Duration::from_millis(5)).await;
        }
        self.radio.transmit(tx_pkt)?;
        let after = Instant::now();
        let only_tx = after - before;

        trace!(
            "[TX DURATION] millis: {},\t ticks: {}",
            only_tx.as_millis(),
            only_tx
        );
        Ok(())
    }

    /// The returned instant is the first heartbeat packet captured
    async fn receive(
        &mut self,
        _conn: Self::Connection,
        rec_buf: &Self::ReceiveBuffer,
    ) -> Result<(heapless::Vec<MHPacket<SIZE>, LEN>, must_hop::node::RxPacket), Self::Error> {
        // This is a hack, but we only want one entry
        let Some(pkt) = rec_buf else {
            return Err(loragw::Error::Generic);
        };

        let rx_hw_timestamp = Instant::now();
        let mut rx_heartbeat_timestamp = None;
        let mut rec_packets: heapless::Vec<MHPacket<SIZE>, LEN> = heapless::Vec::new();
        let now_host = Instant::now();
        let now_radio = self
            .radio
            .get_instcnt()
            .unwrap_or(std::time::Duration::ZERO);

        let pkt = match pkt {
            RxPacket::LoRa(rx_packet) => rx_packet,
            RxPacket::FSK(_) => return Err(loragw::Error::Generic),
        };
        let raw_bytes = &pkt.payload;
        log::info!(
            "Received LoRa Packet | SF: {:?}, BW: {:?}, Freq: {} Hz, RSSI: {:.1} dBm, SNR: {:.1} dB",
            pkt.spreading,
            pkt.bandwidth,
            pkt.freq,
            pkt.rssi,
            pkt.snr
        );

        let packets = postcard::from_bytes::<heapless::Vec<MHPacket<SIZE>, LEN>>(raw_bytes)
            .map_err(|_| loragw::Error::Generic)?;
        log::info!(
            "SUCCESS !!!! Received amount of packets: {:?}",
            packets.len()
        );
        for packet in packets {
            if packet.packet_type == PacketType::HeartBeat && rx_heartbeat_timestamp.is_none() {
                // The returned instant is the first heartbeat packet captured
                let pkt_hw_us = pkt.timestamp.as_micros() as u32;
                let radio_now_us = now_radio.as_micros() as u32;
                let age_us = radio_now_us.wrapping_sub(pkt_hw_us);
                rx_heartbeat_timestamp =
                    Some(now_host - embassy_time::Duration::from_micros(age_us as u64));
            }
            // log::info!("Packet {:?}", packet);
            rec_packets.push(packet).map_err(|_| loragw::Error::Data)?;
        }

        Ok((
            rec_packets,
            must_hop::node::RxPacket {
                instant: rx_heartbeat_timestamp.unwrap_or(rx_hw_timestamp),
                payload_size: 255,
                estimated_toa: 0,
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
    fn calc_tx_delay(&self, payload_len: usize) -> core::time::Duration {
        core::time::Duration::from_millis(60 * payload_len as u64)
    }
}

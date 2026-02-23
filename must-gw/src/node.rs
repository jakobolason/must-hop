use std::{collections::VecDeque, time::Duration};

use loragw::{Concentrator, Error, Running, RxPacket, TxPacket, TxPacketLoRa, TxStatus};
use must_hop::node::{MHNode, MHPacket};
use postcard::to_slice;
use tokio::time::{self, Instant};

const SIZE: usize = 128;
const LEN: usize = 5; // Lets keep it the same as the nodes, make it simple
const LORA_FREQ: usize = 868_100_000;
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
        println!("BUFFER SIZE IS: {}", SIZE);
        let used_slice = match to_slice(&packets, &mut buffer) {
            Ok(slice) => slice,
            Err(e) => {
                eprintln!("Serialization failed: {:?}", e);
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
    type ReceiveBuffer = Vec<RxPacket>;
    type Duration = u16;

    async fn transmit(&mut self, packets: &[MHPacket<SIZE>]) -> Result<(), Self::Error> {
        let tx_pkt = self.to_tx_packet(packets)?;
        while self.radio.transmit_status()? != TxStatus::Free {
            time::sleep(Duration::from_millis(5)).await;
        }
        self.radio.transmit(tx_pkt)
    }

    async fn receive(
        &mut self,
        _conn: Self::Connection,
        rec_buf: &Self::ReceiveBuffer,
    ) -> Result<heapless::Vec<MHPacket<SIZE>, LEN>, Self::Error> {
        // Check if any packets came in whilst transitioning from listen to receive
        // let pkts: Vec<RxPacket> = match self.radio.receive() {
        //     Ok(Some(packet)) => packet,
        //     _ => Vec::new(),
        // };
        let mut rec_packets: heapless::Vec<MHPacket<SIZE>, LEN> = heapless::Vec::new();
        for pkt in rec_buf
        /*.iter().chain(pkts.iter())*/
        {
            let pkt = match pkt {
                RxPacket::LoRa(rx_packet) => rx_packet,
                _ => continue,
            };
            let raw_bytes = &pkt.payload;
            match postcard::from_bytes::<MHPacket<SIZE>>(raw_bytes) {
                Ok(packet) => {
                    println!("SUCCESS !!!! Received packet: {:?}", packet);
                    rec_packets.push(packet).map_err(|_| loragw::Error::Data)?
                }
                Err(e) => {
                    eprintln!("Error deserializing MHPacket: {:?}", e);
                    continue;
                }
            };
        }
        Ok(rec_packets)
    }

    async fn listen(
        &mut self,
        rec_buf: &mut Self::ReceiveBuffer,
        with_timeout: bool,
    ) -> Result<Self::Connection, Self::Error> {
        let start_time = Instant::now();
        let timeout = Duration::from_secs(5);
        rec_buf.clear();

        loop {
            if !self.fetched_packets.is_empty() {
                rec_buf.extend(self.fetched_packets.drain(..));
                return Ok(());
            }
            if let Some(packets) = self.radio.receive()? {
                self.fetched_packets.extend(packets);
                continue;
            }
            if with_timeout && start_time.elapsed() > timeout {
                // TODO: Need better error type here
                return Err(loragw::Error::Busy);
            }
            time::sleep(Duration::from_millis(10)).await;
        }
    }
}

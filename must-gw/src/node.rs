use std::{collections::VecDeque, time::Duration};

use loragw::{
    BoardConf, ChannelConf, Concentrator, Error, Running, RxPacket, RxRFConf, TxGain, TxStatus,
    cfg::Config,
};
use must_hop::{
    lora::SensorData,
    node::{MHNode, MHPacket},
};
use tokio::time::{self, Instant};

const SIZE: usize = 128;
const LEN: usize = 5; // Lets keep it the same as the nodes, make it simple

pub struct GWNode {
    radio: Concentrator<Running>,
    /// Kind of a hack to do it like this, perhaps MHNODE will be altered?
    fetched_packets: VecDeque<RxPacket>,
}

impl GWNode {
    pub fn new(concentrator: Concentrator<Running>) -> Self {
        Self {
            radio: concentrator,
            fetched_packets: VecDeque::new(),
        }
    }
}

impl MHNode for GWNode {
    type Error = loragw::AppError;
    type Connection = ();
    type ReceiveBuffer = Vec<RxPacket>;
    type Duration = u16;

    async fn transmit(
        &mut self,
        packets: &[MHPacket<SIZE>],
    ) -> impl Future<Output = Result<(), Self::Error>> {
        // TODO: How to transform?
        let tx_pkt = packets.into();
        while self.radio.transmit_status()? != TxStatus::Free {
            time::sleep(Duration::from_millis(5)).await;
        }
        self.radio.transmit(tx_pkt)
    }

    async fn receive(
        &mut self,
        conn: Self::Connection,
        rec_buf: &Self::ReceiveBuffer,
    ) -> Result<heapless::Vec<MHPacket<SIZE>, LEN>, Self::Error> {
        // Check if any packets came in whilst transitioning from listen to receive
        let pkts: Vec<RxPacket> = match self.radio.receive() {
            Ok(Some(packet)) => packet,
            _ => Vec::new(),
        };
        let mut rec_packets: heapless::Vec<MHPacket, LEN> = heapless::Vec::new();
        for pkt in pkts {
            let pkt = match pkt {
                RxPacket::LoRa(rx_packet) => rx_packet,
                _ => continue,
            };
            let raw_bytes = pkt.payload;
            let mh_pack = match postcard::from_bytes::<MHPacket<SIZE>>(&raw_bytes) {
                Ok(packet) => packet,
                Err(e) => {
                    eprintln!("Error deserializing MHPacket: {:?}", e);
                    continue;
                }
            };
            println!("SUCCESS !!!! Received packet: {:?}", mh_pack);
            rec_packets.push(mh_pack)?;
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
                return Err(loragw::AppError::Generic("Timeout".into()));
            }
            time::sleep(Duration::from_millis(10)).await;
        }
    }
}

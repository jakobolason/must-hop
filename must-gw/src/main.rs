use loragw::{
    BoardConf, ChannelConf, Concentrator, Error, Running, RxPacket, RxRFConf, TxGain, cfg::Config,
};
use must_hop::{lora::SensorData, node::MHPacket};

const SIZE: usize = 128;

fn create_concentrator() -> Result<Concentrator<Running>, Error> {
    let conf = Config::from_str_or_default(None)?;

    let board_conf = BoardConf::try_from(conf.board.clone()).map_err(Error::from)?;

    let radios: Vec<RxRFConf> = match &conf.radios {
        Some(r_vec) => r_vec
            .iter()
            .map(|r| RxRFConf::try_from(r.clone()).map_err(Error::from))
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };

    let channels: Vec<(u8, ChannelConf)> = match &conf.multirate_channels {
        Some(ch_vec) => ch_vec
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let conf = ChannelConf::try_from(c).map_err(Error::from)?;
                Ok((i as u8, conf))
            })
            .collect::<Result<Vec<_>, Error>>()?,
        None => Vec::new(),
    };

    // Returns a slice &[TxGain] derived from the owned 'conf'
    let tx_gains: Vec<TxGain> = conf
        .tx_gains
        .as_ref()
        .map(|gains| {
            gains
                .iter()
                .map(|g| TxGain::from(g.clone())) // Convert ConfTxGain -> TxGain
                .collect()
        })
        .unwrap_or_default();

    println!("Starting concentrator...");
    Concentrator::open()?
        .set_config_board(board_conf)
        .set_rx_rfs(radios)
        .set_config_channels(channels)
        .set_config_tx_gains(&tx_gains)
        .connect()?
        .start()
}

fn main() {
    // To get logging from loragw
    env_logger::init();

    println!("Now try and use loragw:");
    let conc = match create_concentrator() {
        Ok(concc) => concc,
        Err(e) => {
            eprintln!("Error creating concentrator: {:?}", e);
            return;
        }
    };
    println!("check receive status");
    match conc.receive_status() {
        Ok(status) => println!("Receive status: {:?}", status),
        Err(e) => eprintln!("Error checking receive status: {:?}", e),
    }
    println!("now try receive!");
    loop {
        let pkts: Vec<RxPacket> = match conc.receive() {
            Ok(Some(packet)) => packet,
            Ok(None) => continue,
            Err(e) => {
                eprintln!("Error receiving packet: {:?}", e);
                continue;
            }
        };
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

            let raw_bytes = mh_pack.payload;
            let sensor_data = match postcard::from_bytes::<SensorData>(&raw_bytes) {
                Ok(packet) => packet,
                Err(e) => {
                    eprintln!("Error deserializing SensorData: {:?}", e);
                    continue;
                }
            };
            println!("SUCCESS !!!! Received packet: {:?}", sensor_data);
        }
    }
}

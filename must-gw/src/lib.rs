use loragw::{
    BoardConf, ChannelConf, Concentrator, Error, Running, RxPacket, RxRFConf, TxGain, cfg::Config,
};
use must_hop::{lora::SensorData, node::MHPacket};

const SIZE: usize = 128;

pub mod node;

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

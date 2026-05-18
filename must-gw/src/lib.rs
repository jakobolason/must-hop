use loragw::{
    Bandwidth, BoardConf, ChannelConf, Concentrator, Error, FrontRadio, Running, RxRFConf,
    Spreading, TxGain, cfg::Config, raspberrypi,
};

pub const SIZE: usize = 128;

pub mod node;

/// Construct and start the SX1302 concentrator for the RAK2287 on a Raspberry Pi 4B.
///
/// Channel strategy depends on bandwidth:
/// - BW=125kHz: configure all multirate_channels entries as multi-SF demodulators (up to 8 SFs simultaneously)
/// - BW=250/500kHz: configure a single Fixed channel on the high-speed demodulator (1 SF only)
pub fn create_concentrator(
    spreading: Spreading,
    bandwidth: Bandwidth,
) -> Result<Concentrator<Running>, Error> {
    let conf = Config::from_str_or_default(None)?;

    let board_conf = BoardConf::try_from(conf.board.clone()).map_err(Error::from)?;

    let radios: Vec<RxRFConf> = match &conf.radios {
        Some(r_vec) => r_vec
            .iter()
            .map(|r| RxRFConf::try_from(r.clone()).map_err(Error::from))
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };

    let channels: Vec<(u8, ChannelConf)> = match bandwidth {
        Bandwidth::BW125kHz => match &conf.multirate_channels {
            Some(ch_vec) => ch_vec
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let conf = ChannelConf::try_from(c).map_err(Error::from)?;
                    Ok((i as u8, conf))
                })
                .collect::<Result<Vec<_>, Error>>()?,
            None => Vec::new(),
        },
        _ => {
            // High-speed demodulator: single Fixed channel.
            // Borrow radio and IF position from the first multirate_channels entry.
            let (radio, freq) = conf
                .multirate_channels
                .as_ref()
                .and_then(|ch| ch.first())
                .map(|c| (c.radio, c.if_))
                .unwrap_or((0, 0));
            let radio = FrontRadio::try_from(radio).map_err(Error::from)?;
            vec![(0u8, ChannelConf::Fixed { radio, freq, bandwidth, spreading })]
        }
    };

    let tx_gains: Vec<TxGain> = conf
        .tx_gains
        .as_ref()
        .map(|gains| gains.iter().map(|g| TxGain::from(g.clone())).collect())
        .unwrap_or_default();

    println!("Resetting board first ...");
    let token = loragw::ResetToken::generate(raspberrypi::reset_lgw)
        .expect("Failed to generate reset token");

    println!("Starting concentrator...");
    Concentrator::open(&token)?
        .set_config_board(board_conf)
        .set_rx_rfs(radios)
        .set_config_channels(channels)
        .set_config_tx_gains(&tx_gains)
        .connect()?
        .start()
}

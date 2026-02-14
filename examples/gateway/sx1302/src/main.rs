use libloragw_sys::{lgw_get_eui, lgw_version_info};
use loragw::{cfg::Config, BoardConf, ChannelConf, Concentrator, Error, Running, RxRFConf, TxGain};
use rppal::gpio::Gpio;
use std::ffi::CStr;
use std::thread;
use std::time::Duration;

/// Replicates the logic of your reset_lgw.sh script natively in Rust
fn reset_lgw() -> Result<(), Box<dyn std::error::Error>> {
    println!("Resetting RAK2287 on GPIO 17 natively via rppal...");

    // Grab access to the Pi's GPIO peripherals
    let gpio = Gpio::new()?;
    // Get pin 17 and configure it as an output
    let mut pin = gpio.get(17)?.into_output();
    // pinctrl set 17 op dh (Drive High)
    pin.set_high();
    thread::sleep(Duration::from_millis(100)); // sleep 0.1
                                               // pinctrl set 17 op dl (Drive Low)
    pin.set_low();
    thread::sleep(Duration::from_millis(100)); // sleep 0.1

    println!("Reset complete.");
    Ok(())
}

fn create_concentrator() -> Result<Concentrator<Running>, Error> {
    let spi_conn = "/dev/spidev0.0";
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

    // 5. Handle Tx Gains
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

    // 6. Build and Start
    Concentrator::open()?
        .connect(spi_conn)?
        .set_config_board(board_conf)
        .set_rx_rfs(radios)
        .set_config_channels(channels)
        .set_config_tx_gains(&tx_gains)
        .start()
}

fn main() {
    // 1. Reset the concentrator hardware
    if let Err(e) = reset_lgw() {
        eprintln!("Failed to reset GPIO: {}", e);
        eprintln!("Are you running with sudo?");
        return;
    }

    // 2. Test the FFI bindings by asking the C library for its version
    println!("Testing libloragw bindings...");
    unsafe {
        // lgw_version_info returns a *const c_char
        let version_ptr = lgw_version_info();
        // let mut test: u64 = 10;
        // let eui_ptr = lgw_get_eui(&mut test);

        if !version_ptr.is_null() {
            // Convert the C string pointer to a safe Rust String
            let version_str = CStr::from_ptr(version_ptr).to_string_lossy();
            println!("Success! libloragw version: {}", version_str);
        } else {
            println!("Failed to get version info pointer.");
        }
        // Convert the C string pointer to a safe Rust String
        // println!("Success! libloragw EUI: {}", eui_ptr);
    }
    println!("Now try and use loragw:");
    let conc = match create_concentrator() {
        Ok(concc) => concc,
        Err(e) => {
            eprintln!("Error creating concentrator: {:?}", e);
            return;
        }
    };
}

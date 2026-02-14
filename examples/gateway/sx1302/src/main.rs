use libloragw_sys::{lgw_get_eui, lgw_version_info};
use loragw::{ChannelConf, Concentrator, Error, Running, RxRFConf, cfg::Config};
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
    let radios = match conf.radios {
        Some(vc) => vc,
        None => Vec::new(),
    };
    let radios: Vec<RxRFConf> = radios.iter().try_for_each(|r| RxRFConf::try_from(*r));
    let mr_chan = match conf.multirate_channels {
        Some(mrc) => mrc,
        None => Vec::new(),
    };
    let mr_chan = mr_chan.iter().map(|c| ChannelConf::try_from(c))?;

    let tx_gains = match conf.tx_gains {
        Some(g) => &g,
        None => &[],
    };
    Concentrator::open()?
        .connect(spi_conn)?
        .set_config_board(conf.board.try_into()?)
        .set_rx_rfs(radios)
        .set_config_channels(mr_chan)
        .set_config_tx_gains(tx_gains)
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

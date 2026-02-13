use libloragw_sys::{lgw_get_eui, lgw_version_info};
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
}

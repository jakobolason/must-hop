/// The reset functionality when using the Raspberry Pi as a LoRa gateway.
use rppal::gpio::Gpio;
use std::thread;
use std::time::Duration;

/// Replicates the logic of your reset_lgw.sh script natively in Rust
pub fn reset_lgw() -> Result<(), Box<dyn std::error::Error>> {
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

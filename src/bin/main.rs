#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use defmt::info;
use esp_hal::{clock::CpuClock, gpio::{Level, Output, OutputConfig}, main, rmt::Rmt, time::{Duration, Instant, Rate};
use esp_hal_smartled::SmartLedsAdapter;
use smart_leds::{brightness, colors, SmartLedsWrite as _};
use panic_rtt_target as _;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[main]
fn main() -> ! {
    // generator version: 1.1.0

    rtt_target::rtt_init_defmt!();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let mut led = {
        let frequency = Rate::from_mhz(80);
        let rmt = Rmt::new(peripherals.RMT, frequency).expect("Failed to initialize RMT0");
        SmartLedsAdapter::new(rmt.channel0, peripherals.GPIO8, smartLedBuffer!(1))
    };

    loop {
        info!("Hello world!");
        led.toggle();
        let delay_start = Instant::now();
        while delay_start.elapsed() < Duration::from_millis(1000) {}
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v~1.0/examples
}

#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

// use esp_backtrace as _;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::{Config, rmt::Rmt, time::Rate, timer::timg::TimerGroup};
use esp_hal_smartled::{SmartLedsAdapterAsync, buffer_size_async};
use panic_rtt_target as _;
use smart_leds::{
    RGB8, SmartLedsWriteAsync, brightness, gamma,
    hsv::{Hsv, hsv2rgb},
};
// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    let p = esp_hal::init(Config::default());
    rtt_target::rtt_init_print!();

    // for executor
    let timg0 = TimerGroup::new(p.TIMG0);
    let sw_interrup = esp_hal::interrupt::software::SoftwareInterruptControl::new(p.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrup.software_interrupt0);
    // esp_rtos::start(timg0.timer0);

    // configure Remote Control Transciever (RCT) peripheral globally
    let rmt: Rmt<'_, esp_hal::Async> = Rmt::new(p.RMT, Rate::from_mhz(80))
        .expect("Failed to initialize RMT")
        .into_async();
    let rmt_channel = rmt.channel0;
    let mut rmt_buffer = [esp_hal::rmt::PulseCode::default(); buffer_size_async(1)];

    let mut led = SmartLedsAdapterAsync::new(rmt_channel, p.GPIO8, &mut rmt_buffer);

    let mut color = Hsv {
        hue: 0,
        sat: 255,
        val: 255,
    };
    let mut data: RGB8;
    let level: u8 = 10;

    loop {
        for hue in 0..=255 {
            color.hue = hue;

            data = hsv2rgb(color);

            led.write(brightness(gamma([data].into_iter()), level))
                .await
                .unwrap();
            Timer::after(Duration::from_millis(100)).await;
        }
    }
}

#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]
/// This binary implements the necessary traits to be used with must-hop
// use esp_backtrace as _;
use defmt::info;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::{Config, rmt::Rmt, time::Rate, timer::timg::TimerGroup};
use esp_radio::ble::controller::BleConnector;
use panic_rtt_target as _;
use rtt_target::rtt_init_defmt;

use trouble_host::prelude::ExternalController;
// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[path = "../must_peripheral.rs"]
mod must_peripheral;

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let p = esp_hal::init(Config::default());
    rtt_init_defmt!();
    info!("Setting up peripherals ...");
    // for executor
    let timg0 = TimerGroup::new(p.TIMG0);
    // Could also use esp_hal_embassy ?
    info!("Setting up BLE");
    esp_rtos::start(timg0.timer0, p.FROM_CPU_INTR0);

    esp_alloc::heap_allocator!(size: 72 * 1024);
    info!("Setting up trouble");
    // For BLE task
    let connector = BleConnector::new(p.BT, Default::default()).unwrap();
    let controller: ExternalController<_, 20> = ExternalController::new(connector);
    info!("And away we go!!");
    spawner.spawn(
        must_peripheral::must_peripheral_run(controller).expect("Task must peripheral failed"),
    );

    // Takes ownership of peripherals
    // let radio_reqs = RadioReqs {
    //     nss_req: p.GPIO7,
    //     sclk: p.GPIO9,
    //     mosi: p.GPIO10,
    //     miso: p.GPIO11,
    //     reset_req: p.GPIO12,
    //     busy_req: p.GPIO13,
    //     dio1_req: p.GPIO14,
    //     spi2: p.SPI2,
    // };
    // spawner
    //     .spawn(radio_task(DATA_CHANNEL.receiver(), radio_reqs))
    //     .expect("RADIO TASK failed");
    loop {
        info!("Bing!");
        Timer::after(Duration::from_millis(1000)).await;
    }
}

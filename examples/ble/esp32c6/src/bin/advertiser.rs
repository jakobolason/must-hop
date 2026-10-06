#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

#[path = "../bas_peripheral.rs"]
mod ble_bas_peripheral_run;
#[path = "../led_runner.rs"]
mod led_runner;

use defmt::info;
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_time::{Duration, Timer};
use esp_hal::{Config, timer::timg::TimerGroup};
use esp_radio::ble::controller::BleConnector;
use panic_rtt_target as _;
use rtt_target::rtt_init_defmt;

use serde::{Deserialize, Serialize};
use trouble_host::{
    Address, HostResources,
    advertise::{AdStructure, Advertisement, BR_EDR_NOT_SUPPORTED, LE_GENERAL_DISCOVERABLE},
    connection::{ConnectConfig, ScanConfig},
    prelude::{DefaultPacketPool, ExternalController},
};
// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

const CONNECTIONS_MAX: usize = 1;
// Probably doesn't matter?
const L2CAP_CHANNELS_MAX: usize = 2; // Signal + att

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    let p = esp_hal::init(Config::default());
    rtt_init_defmt!();
    info!("Setting up peripherals ...");
    let timg0 = TimerGroup::new(p.TIMG0);
    info!("Setting up BLE");
    esp_rtos::start(timg0.timer0, p.FROM_CPU_INTR0);

    esp_alloc::heap_allocator!(size: 72 * 1024);
    info!("Setting up trouble");
    let connector = BleConnector::new(p.BT, Default::default()).unwrap();
    let controller: ExternalController<_, 20> = ExternalController::new(connector);
    let address: Address = Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]);
    info!("our address = {:?}", address);

    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();

    // let config = ConnectConfig {
    //     connect_params: Default::default(),
    //     scan_config: ScanConfig {
    //         filter_accept_list: &targets,
    //         ..Default::default()
    //     },
    // };

    let builder = trouble_host::new(controller, &mut resources).set_random_address(address);
    let stack = builder.build();
    let mut peri = stack.peripheral();
    let mut adv_data = [0; 31];
    let name = b"advertising beacon1";
    let adv_data_len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::CompleteLocalName(name),
        ],
        &mut adv_data[..],
    )
    .unwrap();

    let mut runner = stack.runner();
    let _ = join(runner.run(), async {
        let mut counter: u16 = 0;
        loop {
            let mut payload = [0u8; 24];
            let used = postcard::to_slice(&Beacon { seq: counter }, &mut payload)
                .unwrap()
                .len();
            let len = AdStructure::encode_slice(
                &[
                    AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
                    AdStructure::ManufacturerSpecificData {
                        company_identifier: 0xFFFF,
                        payload: &payload[..used],
                    },
                ],
                &mut adv_data,
            )
            .unwrap();
            let adv = peri
                .advertise(
                    &Default::default(),
                    Advertisement::NonconnectableNonscannableUndirected {
                        adv_data: &adv_data[..len],
                    },
                )
                .await
                .unwrap();
            for _ in 0..6 {
                Timer::after(Duration::from_secs(1)).await;
                let mut payload = [0u8; 24];
                let used = postcard::to_slice(&Beacon { seq: counter }, &mut payload)
                    .unwrap()
                    .len();
                counter += 1;
                let len = AdStructure::encode_slice(
                    &[
                        AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
                        AdStructure::ManufacturerSpecificData {
                            company_identifier: 0xFFFF,
                            payload: &payload[..used],
                        },
                    ],
                    &mut adv_data,
                )
                .unwrap();

                info!("Upodating adv data with couter {}", counter);
                // re-encode counter into adv_data -> len
                peri.update_adv_data(Advertisement::NonconnectableNonscannableUndirected {
                    adv_data: &adv_data[..len],
                })
                .await
                .unwrap();
            }

            drop(adv); // advertising stops
            Timer::after(Duration::from_secs(10)).await;
        }
    })
    .await;
    loop {
        info!("Why the hell am i her??");
        Timer::after(Duration::from_secs(10)).await;
    }
}

#[derive(Serialize, Deserialize)]
struct Beacon {
    seq: u16,
}

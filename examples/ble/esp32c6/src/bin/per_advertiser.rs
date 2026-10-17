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

use defmt::{error, info};
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
    advertise::{AdStructure, AdvHandle, Advertisement, AdvertisementParameters, AdvertisementSet},
    prelude::{DefaultPacketPool, ExternalController},
};
// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

const CONNECTIONS_MAX: usize = 1;
// Probably doesn't matter?
const L2CAP_CHANNELS_MAX: usize = 2; // Signal + att
const DATA_SIZE: usize = 31;

const NAME: &[u8] = b"ESP Periodic Adv";

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
    let config = esp_radio::ble::Config::default()
        .with_ext_adv_max_size(255)
        .with_ll_sync_cnt(2)
        .with_data_length_zero_aux(false);
    let connector = BleConnector::new(p.BT, config).unwrap();
    let controller: ExternalController<_, 20> = ExternalController::new(connector);
    let address: Address = Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]);
    info!("our address = {:?}", address);

    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();

    let builder = trouble_host::new(controller, &mut resources).set_random_address(address);
    let stack = builder.build();
    let mut peri = stack.peripheral();
    let mut adv_data = [0; DATA_SIZE];

    let mut runner = stack.runner();
    let _ = join(runner.run(), async {
        let mut counter: u16 = 0;
        // Sets extended advertisement parameters
        let adv_params = AdvertisementParameters {
            interval_min: Duration::from_secs(5),
            interval_max: Duration::from_secs(6),
            // NOTE: Gives 0x12 error
            // primary_phy: bt_hci::param::PhyKind::Le2M,
            ..Default::default()
        };
        let per_params = core::time::Duration::from_secs(5);

        while let Some(set) = make_sets(counter, &mut adv_data, adv_params) {
            info!("sets ok");
            let sets = &[set];
            let mut handles = AdvertisementSet::handles(sets);
            // let adv = peri.advertise_ext(&sets, &mut handles).await.unwrap();
            let adv = match peri
                .per_adv_ext(&sets[0], &mut handles[0], per_params)
                .await
            {
                Ok(adv) => adv,
                Err(e) => {
                    error!("Error with start_periodic: {:?}", e);
                    break;
                }
            };
            info!("adv ok");
            counter += 1;
            while let Some(len) = encode_beacon(counter, &mut adv_data)
                && counter < 600
            {
                Timer::after(Duration::from_secs(5)).await;

                counter += 1;
                info!("Ok updating counter to {}", counter);

                // info!("Updating adv data with couter {}, ok", counter);
                if let Err(e) = peri
                    .update_per_adv_ext(AdvHandle::new(0), &adv_data[..len])
                    .await
                {
                    error!("Error with update_periodic: {:?}", e);
                    break;
                }
            }
            info!("Outer loop ok");

            drop(adv);
            Timer::after(Duration::from_secs(1)).await;
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

fn encode_beacon(counter: u16, buf: &mut [u8; DATA_SIZE]) -> Option<usize> {
    let mut payload = [0u8; 24];
    let n = postcard::to_slice(&Beacon { seq: counter }, &mut payload)
        .unwrap()
        .len();
    match AdStructure::encode_slice(
        &[
            AdStructure::Flags(
                trouble_host::advertise::LE_GENERAL_DISCOVERABLE
                    | trouble_host::advertise::BR_EDR_NOT_SUPPORTED,
            ),
            AdStructure::CompleteLocalName(NAME),
            AdStructure::ManufacturerSpecificData {
                company_identifier: 0xFFFF,
                payload: &payload[..n],
            },
        ],
        buf,
    ) {
        Ok(len) => Some(len),
        Err(e) => {
            error!("Error: {:?}", e);
            None
        }
    }
}

fn make_sets(
    counter: u16,
    adv_data: &'_ mut [u8; DATA_SIZE],
    adv_params: AdvertisementParameters,
) -> Option<AdvertisementSet<'_>> {
    let data = match encode_beacon(counter, adv_data) {
        Some(data) => data,
        None => {
            error!("Error in encoding beacon");
            return None;
        }
    };
    Some(AdvertisementSet {
        params: adv_params,
        data: Advertisement::ExtNonconnectableNonscannableUndirected {
            anonymous: false,
            adv_data: &adv_data[..data],
            // adv_data: &[],
        },
        address: None,
    })
}

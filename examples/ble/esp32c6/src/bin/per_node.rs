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

use bt_hci::{
    cmd::le::LePeriodicAdvCreateSyncParams,
    param::{AddrKind, BdAddr},
};
use defmt::{error, info};
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_time::{Duration, Timer};
use esp_hal::{Config, timer::timg::TimerGroup};
use esp_radio::ble::controller::BleConnector;
use heapless::Vec;
use must_hop::node::ble::{BleNode, rx::RunConfig};
use panic_rtt_target as _;
use rtt_target::rtt_init_defmt;

use static_cell::StaticCell;

use serde::{Deserialize, Serialize};
use trouble_host::{
    Address, HostResources, Stack,
    advertise::{AdStructure, AdvertisementParameters},
    connection::{PhySet, ScanConfig},
    prelude::{DefaultPacketPool, EventHandler, ExternalController, Runner},
    scan::{LeAdvReportsIter, Scanner},
};
// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

pub type Ctrl = ExternalController<BleConnector<'static>, 20>;
pub type Pool = DefaultPacketPool;
pub type BleStack = Stack<'static, Ctrl, Pool>;

const CONNECTIONS_MAX: usize = 1;
// Probably doesn't matter?
const L2CAP_CHANNELS_MAX: usize = 2; // Signal + att

const DATA_SIZE: usize = 31;

#[cfg(not(feature = "board-b"))]
const OUR_ADDR: [u8; 6] = [0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff];
#[cfg(not(feature = "board-b"))]
const PEER_ADDR: [u8; 6] = [0xff, 0x8d, 0x1a, 0x05, 0xe4, 0xff];

#[cfg(feature = "board-b")]
const OUR_ADDR: [u8; 6] = [0xff, 0x8d, 0x1a, 0x05, 0xe4, 0xff];
#[cfg(feature = "board-b")]
const PEER_ADDR: [u8; 6] = [0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff];

const PEERS: usize = 2;
type Res = HostResources<Pool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX>;
static RESOURCES: StaticCell<Res> = StaticCell::new();

static STACK: StaticCell<BleStack> = StaticCell::new();

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
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
        .with_ll_sync_list_cnt(2)
        .with_data_length_zero_aux(false);
    let connector = BleConnector::new(p.BT, config).unwrap();
    let controller: ExternalController<_, 20> = ExternalController::new(connector);

    let address: Address = Address::random(OUR_ADDR);
    info!("our address = {:?}", address);

    let resources = RESOURCES.init(HostResources::new());

    let adv_params = AdvertisementParameters {
        interval_min: Duration::from_millis(100),
        interval_max: Duration::from_millis(200),

        ..Default::default()
    };

    let builder = trouble_host::new(controller, resources).set_random_address(address);
    let stack = STACK.init(builder.build());
    let ble_node = BleNode::new(stack, adv_params);

    let runner = stack.runner();

    let run_config = RunConfig::new_from_slice(Vec::from_array([BdAddr::new(PEER_ADDR)]));

    spawner.spawn(runner_task(runner, run_config).expect("Runner did not start successfully?"));

    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}

#[embassy_executor::task]
async fn runner_task(mut runner: Runner<'static, Ctrl, Pool>, config: RunConfig<PEERS>) {
    // run_with_handler returns Result; log and park on error
    if let Err(e) = runner.run_with_handler(&config).await {
        defmt::error!("runner exited: {:?}", defmt::Debug2Format(&e));
    }
}

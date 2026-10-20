#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use bt_hci::param::BdAddr;
use defmt::{error, info};
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::{Config, rng::Rng, timer::timg::TimerGroup};
use esp_radio::ble::controller::BleConnector;
use heapless::Vec;
use must_hop::{
    MeshRouter, NetworkManager, RandomAccessMac,
    channels::{MeshService, MeshState},
    node::ble::{BleNode, MAX_ADV, RxShared, rx::RunConfig},
    policy::ra::NodePolicy,
};
use panic_rtt_target as _;
use rtt_target::rtt_init_defmt;

use static_cell::StaticCell;

use trouble_host::{
    Address, HostResources, Stack,
    advertise::AdvertisementParameters,
    connection::{PhySet, ScanConfig},
    prelude::{DefaultPacketPool, ExternalController, Runner},
};
// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

const CMD_SLOTS: usize = 20;
pub type Ctrl = ExternalController<BleConnector<'static>, CMD_SLOTS>;
pub type Pool = DefaultPacketPool;
pub type BleStack = Stack<'static, Ctrl, Pool>;

// This is a network based purely on advertisements, so allocate no space for conns or channels
const CONNECTIONS_MAX: usize = 0;
const L2CAP_CHANNELS_MAX: usize = 0;

// The size of each sensor payload
const DATA_SIZE: usize = 30;

#[cfg(not(feature = "board-b"))]
const OUR_ADDR: [u8; 6] = [0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff];
#[cfg(not(feature = "board-b"))]
const PEER_ADDR: [u8; 6] = [0xff, 0x8d, 0x1a, 0x05, 0xe4, 0xff];

#[cfg(feature = "board-b")]
const OUR_ADDR: [u8; 6] = [0xff, 0x8d, 0x1a, 0x05, 0xe4, 0xff];
#[cfg(feature = "board-b")]
const PEER_ADDR: [u8; 6] = [0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff];

const PEERS: usize = 2;

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

    // TODO: Could be nice to know how much is actually necessary here
    esp_alloc::heap_allocator!(size: 72 * 1024);
    info!("Setting up trouble");
    let config = esp_radio::ble::Config::default()
        .with_ext_adv_max_size(MAX_ADV as u16)
        .with_ll_sync_cnt(2)
        .with_ll_sync_list_cnt(2)
        .with_data_length_zero_aux(false);
    let connector = BleConnector::new(p.BT, config).unwrap();
    let controller: Ctrl = ExternalController::new(connector);

    let address: Address = Address::random(OUR_ADDR);
    info!("our address = {:?}", address);

    type Res = HostResources<Pool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX>;
    static RESOURCES: StaticCell<Res> = StaticCell::new();

    static STACK: StaticCell<BleStack> = StaticCell::new();
    static SHARED: RxShared = RxShared::new();

    static STATE: StaticCell<MeshState<DATA_SIZE, 5>> = StaticCell::new();
    let resources = RESOURCES.init(HostResources::new());

    let adv_params = AdvertisementParameters {
        interval_min: Duration::from_millis(100),
        interval_max: Duration::from_millis(200),
        ..Default::default()
    };
    let config = ScanConfig {
        interval: Duration::from_millis(100),
        window: Duration::from_millis(100),
        // NOTE: M2 is not be possible with periodic adv.
        phys: PhySet::M1,
        ..Default::default()
    };

    let stack = STACK.init(
        trouble_host::new(controller, resources)
            .set_random_address(address)
            .build(),
    );
    let ble_node = BleNode::new(stack, adv_params, &SHARED, config);

    let runner = stack.runner();
    let run_config = RunConfig::new_from_slice(Vec::from_array([BdAddr::new(PEER_ADDR)]), &SHARED);

    spawner.spawn(runner_task(runner, run_config).expect("Runner did not start successfully?"));
    let rx_buf = [0u8; MAX_ADV];
    let manager: NetworkManager<DATA_SIZE, 5> = NetworkManager::new(0, 1, 10, None);
    let mac = RandomAccessMac::new(NodePolicy);
    let router = MeshRouter::new(ble_node, manager, mac);

    let state = STATE.init(MeshState::new());
    let (mail_box, service) = must_hop::channels::init(state, router, rx_buf);
    spawner.spawn(mesh_task(service).expect("Mesh task started successfully"));
    // Now that should run in the background, and the user can now make their business logic to
    // choose when to send what, to whom
    let rng = Rng::new();
    loop {
        let delay_ms = 1000 + (rng.random() % 5000) as u64;
        Timer::after(Duration::from_millis(delay_ms)).await;
        match mail_box.send(0, &[1u8; 8]).await {
            Ok(_) => info!("Sucessfully sent!"),
            Err(_) => error!("Error for sending empty payload"),
        }
        if let Some(data) = mail_box.try_recv() {
            info!(
                "Received a message from {}! {:?}",
                data.source, data.payload
            );
        }
    }
}
const LEN: usize = 5;
const N: usize = 5;

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[embassy_executor::task]
async fn mesh_task(
    service: MeshService<
        BleNode<'static, Ctrl, DefaultPacketPool>,
        RandomAccessMac<DATA_SIZE, NodePolicy>,
        DATA_SIZE,
        LEN,
        N,
    >,
) {
    service.run().await
}

#[embassy_executor::task]
async fn runner_task(mut runner: Runner<'static, Ctrl, Pool>, config: RunConfig<PEERS>) {
    // run_with_handler returns Result; log and park on error
    if let Err(e) = runner.run_with_handler(&config).await {
        defmt::error!("runner exited: {:?}", defmt::Debug2Format(&e));
    }
}

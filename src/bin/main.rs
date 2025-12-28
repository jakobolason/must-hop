#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

// use esp_backtrace as _;
use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_futures::{join::join, select::select};
use embassy_time::{Duration, Timer};
use esp_hal::{
    Config,
    gpio::AnyPin,
    rmt::{ChannelCreator, Rmt},
    time::Rate,
    timer::timg::TimerGroup,
};
use esp_hal_smartled::{SmartLedsAdapterAsync, buffer_size_async};
use esp_radio::ble::controller;
use panic_rtt_target as _;
use rtt_target::rtt_init_defmt;
use smart_leds::{
    RGB8, SmartLedsWriteAsync, brightness, gamma,
    hsv::{Hsv, hsv2rgb},
};
use trouble_host::{advertise, prelude::*};
// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

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
    let sw_interrup = esp_hal::interrupt::software::SoftwareInterruptControl::new(p.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrup.software_interrupt0);

    // configure Remote Control Transciever (RCT) peripheral globally
    let rmt: Rmt<'_, esp_hal::Async> = Rmt::new(p.RMT, Rate::from_mhz(80))
        .expect("Failed to initialize RMT")
        .into_async();
    spawner
        .spawn(slide_rbg_colors(rmt.channel0, p.GPIO8.into()))
        .expect("TASK slide_rbg_colors failed");

    // configure radio for BLE
    esp_alloc::heap_allocator!(size: 72 * 1024);
    let radio = esp_radio::init().expect("ESP radio failed to initialize");
    let bluetooth = p.BT;
    let connector = controller::BleConnector::new(&radio, bluetooth, Default::default())
        .expect("BLE connector failed to initialize");
    let controller: ExternalController<_, 1> = ExternalController::new(connector);

    ble_bas_peripheral_run(controller).await;
    loop {
        info!("Bing!");
        Timer::after(Duration::from_millis(1000)).await;
    }
}

const CONNECTIONS_MAX: usize = 1;
/// Max number of L2CAP Channels
const L2CAP_CHANNELS_MAX: usize = 2; // Signal + att

#[gatt_server]
struct Server {
    battery_service: BatteryService,
}

/// Battery Service
#[gatt_service(uuid = service::BATTERY)]
struct BatteryService {
    /// Battery level
    #[descriptor(uuid = descriptors::VALID_RANGE, read, value = [0, 100])]
    #[descriptor(uuid = descriptors::MEASUREMENT_DESCRIPTION, name = "hello", read, value = "Battery Level")]
    #[characteristic(uuid = characteristic::BATTERY_LEVEL, read, notify, value = 10)]
    level: u8,
    #[characteristic(uuid = "408813df-5dd4-1f87-ec11-cdb001100000", write, read, notify)]
    status: bool,
}

/// Run the BLE stack
pub async fn ble_bas_peripheral_run<C>(controller: C)
where
    C: Controller,
{
    // Using a fixed random address is useful for testing, in real scenarios
    // the MAC 6 byte array can be used as the address
    let address: Address = Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]);
    info!("our address = {:?}", address);

    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let stack = trouble_host::new(controller, &mut resources).set_random_address(address);
    let Host {
        mut peripheral,
        runner,
        ..
    } = stack.build();
    info!("Starting advertising and GATT service");
    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: "TrouBle",
        appearance: &appearance::power_device::GENERIC_POWER_DEVICE,
    }))
    .unwrap();
    let _ = join(ble_task(runner), async {
        loop {
            match advertise("trouBLE example", &mut peripheral, &server).await {
                Ok(conn) => {
                    // these tasks only run after a connection has been established
                    let a = gatt_events_task(&server, &conn);
                    let b = custom_task(&server, &conn, &stack);

                    select(a, b).await;
                }
                Err(e) => {
                    panic!("[adv] error: {:?}", e);
                }
            }
        }
    })
    .await;
}

async fn ble_task<C, P>(mut runner: Runner<'_, C, P>)
where
    C: Controller,
    P: PacketPool,
{
    loop {
        if let Err(e) = runner.run().await {
            panic!("[ble_task] error: {:?}", e);
        }
    }
}

/// Stream Events until the connection closes.
///
/// This function will handle the GATT events and process them.
/// This is how we interact with read and write requests.
async fn gatt_events_task<P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
) -> Result<(), Error> {
    let level = server.battery_service.level;
    let reason = loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { reason } => break reason,
            GattConnectionEvent::Gatt { event } => {
                match &event {
                    GattEvent::Read(event) => {
                        if event.handle() == level.handle {
                            let value = server.get(&level);
                            info!("[gatt] Read Event to Level Characteristic: {:?}", value);
                        }
                    }
                    GattEvent::Write(event) => {
                        if event.handle() == level.handle {
                            info!(
                                "[gatt] Write Event to Level Characteristic: {:?}",
                                event.data()
                            );
                        }
                    }
                    _ => {}
                };
                // This step is also performed at drop(), but writing it explicitly is necessary
                // in order to ensure reply is sent.
                match event.accept() {
                    Ok(reply) => reply.send().await,
                    Err(e) => warn!("[gatt] error sending response: {:?}", e),
                };
            }
            _ => {} // ignore other Gatt Connection Events
        }
    };
    info!("[gatt] disconnected: {:?}", reason);
    Ok(())
}

/// Create an advertiser to use to connect to a BLE Central, and wait for it to connect.
async fn advertise<'values, 'server, C: Controller>(
    name: &'values str,
    peripheral: &mut Peripheral<'values, C, DefaultPacketPool>,
    server: &'server Server<'values>,
) -> Result<GattConnection<'values, 'server, DefaultPacketPool>, BleHostError<C::Error>> {
    let mut advertiser_data = [0; 31];
    let len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::ServiceUuids16(&[[0x0f, 0x18]]),
            AdStructure::CompleteLocalName(name.as_bytes()),
        ],
        &mut advertiser_data[..],
    )?;
    let advertiser = peripheral
        .advertise(
            &Default::default(),
            Advertisement::ConnectableScannableUndirected {
                adv_data: &advertiser_data[..len],
                scan_data: &[],
            },
        )
        .await?;
    info!("[adv] advertising");
    let conn = advertiser.accept().await?.with_attribute_server(server)?;
    info!("[adv] connection established");
    Ok(conn)
}

/// Example task to use the BLE notifier interface.
/// This task will notify the connected central of a counter value every 2 seconds.
/// It will also read the RSSI value every 2 seconds.
/// and will stop when the connection is closed by the central or an error occurs.
async fn custom_task<C: Controller, P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
    stack: &Stack<'_, C, P>,
) {
    let mut tick: u8 = 0;
    let level = server.battery_service.level;
    loop {
        tick = tick.wrapping_add(1);
        info!("[custom_task] notifying connection of tick {}", tick);
        if level.notify(conn, &tick).await.is_err() {
            info!("[custom_task] error notifying connection");
            break;
        };
        // read RSSI (Received Signal Strength Indicator) of the connection.
        if let Ok(rssi) = conn.raw().rssi(stack).await {
            info!("[custom_task] RSSI: {:?}", rssi);
        } else {
            info!("[custom_task] error getting RSSI");
            break;
        };
        Timer::after_secs(2).await;
    }
}

#[embassy_executor::task]
async fn slide_rbg_colors(
    // mut rmt_channel: esp_hal::rmt::Channel<'static, esp_hal::Blocking, esp_hal::rmt::Tx>,
    rmt_channel: ChannelCreator<'static, esp_hal::Async, 0>,
    gpio8: AnyPin<'static>,
) {
    let mut rmt_buffer = [esp_hal::rmt::PulseCode::default(); buffer_size_async(1)];
    let mut led = SmartLedsAdapterAsync::new(rmt_channel, gpio8, &mut rmt_buffer);

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

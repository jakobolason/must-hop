// use esp_backtrace as _;
use defmt::{info, warn};
use embassy_futures::{
    join::{join, join3},
    select::select,
};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Timer};

use panic_rtt_target as _;

use trouble_host::{PacketPool, prelude::*};

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
    // Build host which gives peripheral access and runner to handle radio
    let Host {
        mut peripheral,
        mut central,
        runner,
        ..
    } = stack.build();

    let config = ConnectConfig {
        connect_params: Default::default(),
        scan_config: ScanConfig::default(),
    };

    info!("Starting advertising and GATT service");
    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: "TrouBle",
        appearance: &appearance::power_device::GENERIC_POWER_DEVICE,
    }))
    .unwrap();
    let _ = join(
        ble_task(runner),
        advertise_task(&mut peripheral, server, &stack),
    )
    .await;
}

/// runs whatever tasks are send to the runner, panics if error
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

async fn search_task<'a, C>(
    central: &mut Central<'a, C, DefaultPacketPool>,
    config: ConnectConfig<'a>,
    stack: Stack<'_, C, DefaultPacketPool>,
) where
    C: Controller,
{
    loop {
        let conn = central
            .connect(&config)
            .await
            .expect("Connection unsuccessfull");
        info!("COnnected, creating l2cap channel");
        const PAYLOAD_LEN: usize = 27; // ???
        let config = L2capChannelConfig {
            mtu: Some(PAYLOAD_LEN as u16),
            ..Default::default()
        };
        const PSM_L2CAP_EXAMPLES: u16 = 0x0081;
        let mut ch1 = L2capChannel::create(&stack, &conn, PSM_L2CAP_EXAMPLES, &config)
            .await
            .expect("channel creation failed");
        info!("New l2cap channel created, sending some data!");
        for i in 0..10 {
            let tx = [i; PAYLOAD_LEN];
            ch1.send(&stack, &tx).await.unwrap();
        }
        info!("Sent data, waiting for them to be sent back");
        let mut rx = [0; PAYLOAD_LEN];
        for i in 0..10 {
            let len = ch1.receive(&stack, &mut rx).await.unwrap();
            assert_eq!(len, rx.len());
            assert_eq!(rx, [i; PAYLOAD_LEN]);
        }

        info!("Received successfully!");

        Timer::after(Duration::from_secs(60)).await;
    }
}

async fn advertise_task<'a, C>(
    peripheral: &mut Peripheral<'a, C, DefaultPacketPool>,
    server: Server<'a>,
    stack: &Stack<'_, C, DefaultPacketPool>,
) where
    C: Controller,
{
    // After bootup, wait some time before having sensor data
    Timer::after_secs(10).await;
    loop {
        match advertise("trouBLE example", peripheral, &server).await {
            Ok(conn) => {
                // these tasks only run after a connection has been established
                let a = gatt_events_task(&server, &conn);
                let b = custom_task(&server, &conn, stack);

                select(a, b).await;
            }
            Err(e) => {
                info!("[ERROR] adv error:");
            }
        }
        Timer::after_secs(30);
    }
}

/// Create an advertiser to use to connect to a BLE Central, and wait for it to connect.
async fn advertise<'values, 'server, C>(
    name: &'values str,
    peripheral: &mut Peripheral<'values, C, DefaultPacketPool>,
    server: &'server Server<'values>,
) -> Result<GattConnection<'values, 'server, DefaultPacketPool>, BleHostError<C::Error>>
where
    C: Controller,
{
    let mut advertiser_data = [0; 31];
    let len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            // AdStructure::ServiceUuids16(&[[0x0f, 0x18]]),
            // AdStructure::CompleteLocalName(name.as_bytes()),
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

async fn l2cap_rec<C, P>(conn: &GattConnection<'_, '_, P>, stack: &Stack<'_, C, DefaultPacketPool>)
where
    C: Controller,
    P: PacketPool,
{
    const PAYLOAD_LEN: usize = 27; // ???
    let config = L2capChannelConfig {
        mtu: Some(PAYLOAD_LEN as u16),
        ..Default::default()
    };
    const PSM_L2CAP_EXAMPLES: u16 = 0x0081;
    let mut ch1 = L2capChannel::create(&stack, conn, PSM_L2CAP_EXAMPLES, &config)
        .await
        .expect("channel creation failed");
    info!("New l2cap channel created, sending some data!");
    for i in 0..10 {
        let tx = [i; PAYLOAD_LEN];
        ch1.send(&stack, &tx).await.unwrap();
    }
    info!("Sent data, waiting for them to be sent back");
    let mut rx = [0; PAYLOAD_LEN];
    for i in 0..10 {
        let len = ch1.receive(&stack, &mut rx).await.unwrap();
        assert_eq!(len, rx.len());
        assert_eq!(rx, [i; PAYLOAD_LEN]);
    }

    info!("Received successfully!");

    Timer::after(Duration::from_secs(60)).await;
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

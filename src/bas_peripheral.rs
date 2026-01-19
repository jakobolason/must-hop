// use esp_backtrace as _;
use defmt::{Debug2Format, error, info, warn};
use embassy_futures::join::join3;
use embassy_time::{Duration, Timer};
use postcard::to_slice;
use serde::{Deserialize, Serialize};
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

trait LogExt<T, E> {
    fn log_error(self, msg: &str) -> Option<T>;
}

impl<T, E: core::fmt::Debug> LogExt<T, E> for Result<T, E> {
    fn log_error(self, msg: &str) -> Option<T> {
        match self {
            Ok(v) => Some(v),
            Err(e) => {
                error!("{}: {:?}", msg, Debug2Format(&e));
                None
            }
        }
    }
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
    let target: Address = Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]);
    let config = ConnectConfig {
        connect_params: Default::default(),
        scan_config: ScanConfig {
            filter_accept_list: &[(target.kind, &target.addr)],
            ..Default::default()
        },
    };

    info!("Starting advertising and GATT service");
    // let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
    //     name: "TrouBle",
    //     appearance: &appearance::power_device::GENERIC_POWER_DEVICE,
    // }))
    // .unwrap();

    // This runs 3 jobs: runner handles the radio, search and advertise are periodic tasks
    let _ = join3(
        ble_task(runner),
        search_task(&mut central, config, &stack),
        advertise_task(&mut peripheral, &stack),
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
            error!("[ble_task] error: {:?}", Debug2Format(&e));
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct SensorMessage {
    temperature: i8,
    current_voltage: i8,
}
fn create_sensor_data(buffer: &mut [u8]) -> Result<&mut [u8], postcard::Error> {
    let msg = SensorMessage {
        temperature: 20,
        current_voltage: 5,
    };

    to_slice(&msg, buffer)
}

/// This task searches for sensor data, and afterwards determines if the data Should
/// be saved here, or sent onwards
async fn search_task<'a, C>(
    central: &mut Central<'a, C, DefaultPacketPool>,
    config: ConnectConfig<'a>,
    stack: &'a Stack<'a, C, DefaultPacketPool>,
) where
    C: Controller + 'a,
{
    loop {
        let Some(conn) = central
            .connect(&config)
            .await
            .log_error("Getting connection failed")
        else {
            continue;
        };
        info!("COnnected, creating l2cap channel");
        const PAYLOAD_LEN: usize = 27; // ???
        let config = L2capChannelConfig {
            mtu: Some(PAYLOAD_LEN as u16),
            ..Default::default()
        };
        const PSM_L2CAP_EXAMPLES: u16 = 0x0081;
        let mut ch1 = match L2capChannel::create(stack, &conn, PSM_L2CAP_EXAMPLES, &config).await {
            Ok(ch) => ch,
            Err(e) => {
                error!("Connection error: {:?}", Debug2Format(&e));
                continue;
            }
        };

        // TODO: With a connection now established, the correct thing to do, would be to put the
        // following into a function. This function should handle receiveng information from the
        // channel, dropping the channel and thereafter look at whether the message should be sent
        // onwards

        info!("New l2cap channel created, sending some data!");
        // NOTE: Using this to test that the same created sensor data is received on both ends
        let mut test_buffer = [0u8; PAYLOAD_LEN];
        let test_slice =
            create_sensor_data(&mut test_buffer).expect("Creating sensor data failed?");
        let mut rx = [0; PAYLOAD_LEN];
        let len = match ch1.receive(stack, &mut rx).await {
            Ok(l) => l,
            Err(e) => {
                error!(
                    "Error in getting length of Rx signal: {:?}",
                    Debug2Format(&e)
                );
                continue;
            }
        };
        assert_eq!(len, rx.len());
        assert_eq!(rx, test_slice);

        info!("Received successfully!");
        // Should wait some time before doing this again
        Timer::after(Duration::from_secs(60)).await;
    }
}

/// This task advertises when there are sensor data available
async fn advertise_task<'a, C>(
    peripheral: &mut Peripheral<'a, C, DefaultPacketPool>,
    stack: &'a Stack<'a, C, DefaultPacketPool>,
) where
    C: Controller + 'a,
{
    let mut adv_data = [0; 31];
    let name = "trouBLE tester";
    let adv_data_len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::ServiceUuids16(&[[0x0f, 0x18]]),
            AdStructure::CompleteLocalName(name.as_bytes()),
        ],
        &mut adv_data[..],
    )
    .unwrap();
    Timer::after_secs(10).await; // Wait a bit before starting this 
    loop {
        info!("Advertising, waiting for connection ...");
        let advertiser = match peripheral
            .advertise(
                &Default::default(),
                Advertisement::ConnectableScannableUndirected {
                    adv_data: &adv_data[..adv_data_len],
                    scan_data: &[],
                },
            )
            .await
        {
            Ok(adv) => adv,
            Err(e) => {
                error!("Error in advertising: {:?}", Debug2Format(&e));
                continue;
            }
        };
        let conn = match advertiser.accept().await {
            Ok(conn) => conn,
            Err(error) => {
                error!("Error in accepting connection: {:?}", Debug2Format(&error));
                continue;
            }
        };
        info!("Connected, creating l2cap channel");
        const PAYLOAD_LEN: usize = 27; // NOTE: Look into this
        let config = L2capChannelConfig {
            mtu: Some(PAYLOAD_LEN as u16),
            ..Default::default()
        };
        const PSM_L2CAP_EXAMPLES: u16 = 0x0081; // NOTE: Look into this
        let mut ch1 = match L2capChannel::create(stack, &conn, PSM_L2CAP_EXAMPLES, &config).await {
            Ok(ch) => ch,
            Err(e) => {
                error!("Error in creating adv channel: {:?}", Debug2Format(&e));
                continue;
            }
        };
        info!("New l2cap channel created, receiving some data!");
        // TODO: Should receive the custom sensor data constructed above
        // NOTE: This simply transmits whatever we set into tx
        // Send some basic sensor data:
        let mut tx = [0u8; PAYLOAD_LEN];
        match create_sensor_data(&mut tx) {
            Ok(slice) => {
                if let Err(e) = ch1.send(stack, slice).await {
                    error!("Error in Tx: {:?}", Debug2Format(&e));
                }
            }
            Err(e) => error!("Error in slicing Tx: {:?}", Debug2Format(&e)),
        }
        info!("Sent successfully!");

        Timer::after(Duration::from_secs(60)).await;
    }
}

// async fn advertise_task<'a, C>(
//     peripheral: &mut Peripheral<'a, C, DefaultPacketPool>,
//     server: Server<'a>,
//     stack: &Stack<'_, C, DefaultPacketPool>,
// ) where
//     C: Controller,
// {
//     // After bootup, wait some time before having sensor data
//     Timer::after_secs(10).await;
//     loop {
//         match advertise("trouBLE example", peripheral, &server).await {
//             Ok(conn) => {
//                 // these tasks only run after a connection has been established
//                 let a = gatt_events_task(&server, &conn);
//                 let b = custom_task(&server, &conn, stack);
//
//                 select(a, b).await;
//             }
//             Err(e) => {
//                 info!("[ERROR] adv error:");
//             }
//         }
//         Timer::after_secs(30);
//     }
// }

/// Create an advertiser to use to connect to a BLE Central, and wait for it to connect.
#[allow(unused)]
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

/// Stream Events until the connection closes.
///
/// This function will handle the GATT events and process them.
/// This is how we interact with read and write requests.
#[allow(unused)]
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
#[allow(unused)]
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

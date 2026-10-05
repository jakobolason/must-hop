// use esp_backtrace as _;
use defmt::{Debug2Format, error, info, warn};
use embassy_futures::join::join3;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::peripherals::BT;
use esp_radio::ble::controller::BleConnector;
use heapless::{Vec, vec};
use must_hop::node::ConnectionType;
use must_hop::{MHNode, MHPacket, RxPacket};
use postcard::{from_bytes, to_slice};
use serde::{Deserialize, Serialize};
use trouble_host::{PacketPool, prelude::*};

use trouble_host::prelude::ExternalController;
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
    #[descriptor(uuid = descriptors::MEASUREMENT_DESCRIPTION, name = "hello", read, value = "Battery Level", type = &'static str)]
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

type ExtController<const SLOTS: usize> = ExternalController<BleConnector<'static>, SLOTS>;
type HostRes = HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX>;

pub struct Link {
    node_id: u8,
    hops: u8,
    addr: Address,
}

pub struct BleNode<'a, 'r, const SLOTS: usize, const ADV_LEN: usize> {
    stack: Stack<'r, ExtController<SLOTS>, DefaultPacketPool>,
    config: ConnectConfig<'a>,
    links: Vec<Link, 5>,
    adv_data: &'a [u8; ADV_LEN],
}

impl<'a, 'r, const SLOTS: usize, const ADV_LEN: usize> BleNode<'a, 'r, SLOTS, ADV_LEN> {
    fn new(
        controller: ExtController<SLOTS>,
        address: Address,
        config: ConnectConfig<'a>,
        resources: &'r mut HostRes,
    ) -> Self {
        info!("our address = {:?}", address);

        let builder = trouble_host::new(controller, resources).set_random_address(address);
        // Build host which gives peripheral access and runner to handle radio
        let stack = builder.build();

        Self {
            stack,
            config,
            links: Vec::new(),
            adv_data: &[0u8; ADV_LEN],
        }
    }
}

impl<'a, 'r, const SLOTS: usize, const SIZE: usize, const LEN: usize, const ADV_LEN: usize>
    MHNode<SIZE, LEN> for BleNode<'a, 'r, SLOTS, ADV_LEN>
{
    type RadioError = ();
    type Error = ();
    type Connection = ();
    type ReceiveBuffer = ();
    type Recipient = Link;

    // NOTE: Will probably have to change the flow in tdma, because it requires listen -> receive
    // Which I don't think is a good idea here.

    async fn transmit(
        &mut self,
        packet: &[MHPacket<SIZE>],
        dir: ConnectionType,
    ) -> Result<(), Self::Error> {
        // We get a list of the link indexes we want to send to.
        // NOTE: When do we need to transmit to a new one?
        // NOTE: Instead of sending for each index, perhaps a broadcast should only be an 'advertisement', with
        // limited MTU, where as when given an Id it uses the channel? THus the timing requirements of sending
        // a packet wouldn't be broken
        match dir {
            ConnectionType::Id(id) => {
                // Find the slot in our links, and send to that
                let Some(link) = self.links.iter().find(|l| l.node_id == id) else {
                    return Err(());
                };
                let Some(conn) = self
                    .stack
                    .connections()
                    .find(|c| c.peer_address() == link.addr)
                else {
                    return Err(());
                };
                const PAYLOAD_LEN: usize = 10;
                let buf = &[0u8; PAYLOAD_LEN];
                let config = L2capChannelConfig {
                    mtu: Some(PAYLOAD_LEN as u16),
                    ..Default::default()
                };
                let Ok(mut ch) = L2capChannel::create(&self.stack, &conn, 0x0081, &config).await
                else {
                    error!("Could not create channel!");
                    return Err(());
                };
                if let Err(e) = ch.send(&self.stack, buf).await {
                    error!("Error in transmitting , because: {}", e);
                    return Err(());
                }
            }
            ConnectionType::New => {
                // First advertise
                todo!()
            }
            ConnectionType::Broadcast => {
                // Should advertise, right?
                todo!()
                // And then should send to all known links
            }
        };
        // for i in indexes {
        //     if *i < 0isize {
        //         break;
        //     }
        //     let i = *i as usize;
        //     let Some(conn) = self
        //         .stack
        //         .connections()
        //         .find(|c| &c.peer_address().addr.into_inner() == self.links[i].addr)
        //     else {
        //         error!("Could not find link for address: {:?}", self.links[i].addr);
        //         continue;
        //     };
        //     const payload_len: usize = 10;
        //     let buf = &[0u8; payload_len];
        //     let config = L2capChannelConfig {
        //         mtu: Some(payload_len as u16),
        //         ..Default::default()
        //     };
        //     let Ok(mut ch) = L2capChannel::create(&self.stack, &conn, 0x0081, &config).await else {
        //         error!("Could not create channel!");
        //         continue;
        //     };
        //     if let Err(e) = ch.send(&self.stack, buf).await {
        //         error!("Error in transmitting with {}, because: {}", i, e);
        //         continue;
        //     }
        //     todo!("Transmit!");
        // }
        todo!()
    }

    async fn receive(
        &mut self,
        conn: Self::Connection,
        rec_buf: &Self::ReceiveBuffer,
        dir: ConnectionType,
    ) -> Result<(Vec<MHPacket<SIZE>, LEN>, RxPacket), Self::Error> {
        // To receive a packet from a connection, we should simply listen to it, right?
        match dir {
            ConnectionType::Id(id) => {
                let addr = if let Some(link) = self.links.iter().find(|l| l.node_id == id) {
                    link.addr
                } else {
                    return Err(());
                };
                let Some(conn) = self.stack.connections().find(|c| c.peer_address() == addr) else {
                    return Err(());
                };
                const PAYLOAD_LEN: usize = 20;
                let mut rx = [0; PAYLOAD_LEN];
                let config = L2capChannelConfig {
                    mtu: Some(PAYLOAD_LEN as u16),
                    ..Default::default()
                };
                let Ok(mut ch) = L2capChannel::listen(&self.stack, &conn)
                    .accept(&config)
                    .await
                else {
                    error!("Could not listenf ro some reason");
                    return Err(());
                };
                let len = ch.receive(&self.stack, &mut rx).await.unwrap();
                let packets = match from_bytes::<Vec<MHPacket<SIZE>, LEN>>(&rx) {
                    Ok(packet) => packet,
                    Err(e) => {
                        error!("Deserialization failed: {:?}", e);
                        return Err(());
                    }
                };
                let rxpkt = RxPacket {
                    payload_size: len as u8,
                    rx_done_instant: Instant::now(),
                };
                Ok((packets, rxpkt))
            }
            ConnectionType::New => {
                // TODO: Should first advertise with how many hops this has
                todo!()
            }
            ConnectionType::Broadcast => {
                // TODO: *ONLY* advertise, and don't accept connections, such that we have a advertisement
                // beacon
                todo!()
            }
        }
    }

    async fn listen(
        &mut self,
        rec_buf: &mut Self::ReceiveBuffer,
        with_timeout: Option<core::time::Duration>,
    ) -> Result<Self::Connection, Self::Error> {
        // Here we use our given pool of connections, and listen for any one of those connections to
        // be sending anything right now
        // TODO: Alter some internal state, such that the runner task checks that it should
        // check for an advertisement with stack.runner().run_with_handler(&handler).await
        todo!()
    }

    fn calc_tx_delay(&self, _payload_len: usize) -> u64 {
        // TODO: Figure out if there is any transmission delay for BLE
        0
    }
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
/// Run the BLE stack
#[embassy_executor::task]
pub async fn must_peripheral_run(controller: ExternalController<BleConnector<'static>, 20>) {
    // Using a fixed random address is useful for testing, in real scenarios
    // the MAC 6 byte array can be used as the address
    let address: Address = Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]);
    info!("our address = {:?}", address);

    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();

    let target: Address = Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]);
    let targets = [target];
    let config = ConnectConfig {
        connect_params: Default::default(),
        scan_config: ScanConfig {
            filter_accept_list: &targets,
            ..Default::default()
        },
    };

    let node: BleNode<'_, '_, 20, 64> = BleNode::new(controller, address, config, &mut resources);

    loop {
        Timer::after_secs(5).await;
        info!("BONG!");
    }
}

/// runs whatever tasks are send to the runner, prints out any error
async fn ble_task<C, P>(runner: &mut Runner<'_, C, P>)
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
    central: &mut Central<'_, C, DefaultPacketPool>,
    config: &ConnectConfig<'_>,
    stack: &Stack<'a, C, DefaultPacketPool>,
) where
    C: Controller + 'a,
{
    loop {
        let Some(conn) = central
            .connect(config)
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
        // following into a function. This function should handle receiving information from the
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

async fn advertise_sensordata<'a, C>(
    peripheral: &mut Peripheral<'a, C, DefaultPacketPool>,
    adv_data: &[u8],
    adv_data_len: usize,
) -> Result<Connection<'a, DefaultPacketPool>, BleHostError<C::Error>>
where
    C: Controller,
{
    let advertiser = peripheral
        .advertise(
            &Default::default(),
            Advertisement::ConnectableScannableUndirected {
                adv_data: &adv_data[..adv_data_len],
                scan_data: &[],
            },
        )
        .await?;
    let conn = advertiser.accept().await?;
    Ok(conn)
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
/// This task advertises when there are sensor data available
async fn advertise_task<'a, C>(
    peripheral: &mut Peripheral<'_, C, DefaultPacketPool>,
    stack: &Stack<'a, C, DefaultPacketPool>,
) where
    C: Controller + 'a,
{
    info!("In advertising task!!");
    let mut adv_data = [0; 31];
    let name = b"trouBLE tester";
    let adv_data_len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            // AdStructure::ServiceUuids16(&[[0x0f, 0x18]]), // For battery GATT
            AdStructure::CompleteServiceUuids16(&[[0x00, 0x01]]), // For L2CAP
            AdStructure::CompleteLocalName(name),
        ],
        &mut adv_data[..],
    )
    .unwrap();
    // Timer::after_secs(10).await; // Wait a bit before starting this
    loop {
        info!("Advertising, waiting for connection ...");
        let conn = match advertise_sensordata(peripheral, &adv_data, adv_data_len).await {
            Ok(conn) => conn,
            Err(error) => {
                error!("Error in getting connection: {:?}", Debug2Format(&error));
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
        // TODO: Impl most of this for the laptop too, so that i have 2 on the network
        let mut ch1 = match L2capChannel::create(stack, &conn, PSM_L2CAP_EXAMPLES, &config).await {
            Ok(ch) => ch,
            Err(e) => {
                error!("Error in creating adv channel: {:?}", Debug2Format(&e));
                continue;
            }
        };
        info!("New l2cap channel created, receiving some data!");

        // NOTE: This simply transmits whatever we set into tx
        // Send some basic sensor data:
        let mut tx = [0u8; PAYLOAD_LEN];
        match create_sensor_data(&mut tx) {
            // NOTE: This should be passed to the function (sensor data)
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
                            info!("[gatt] Write Event to Level Characteristic: {:?}", event);
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
        if level.notify(conn, &tick, false).await.is_err() {
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

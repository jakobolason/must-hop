use crate::{
    MHPacket, RxPacket,
    log_compat::{debug, error, info},
};
use bt_hci::param::BdAddr;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Instant, Timer};
use heapless::Vec;
use postcard::{from_bytes, to_slice};
use serde::{Deserialize, Serialize};
/// This contains node implementations for BLE
use trouble_host::{
    BleHostError, Controller, PacketPool, Stack,
    advertise::{AdStructure, Advertisement, AdvertisementParameters, AdvertisementSet},
    connection::ScanConfig,
    prelude::DefaultPacketPool,
    scan::Scanner,
};

#[cfg(not(feature = "in_std"))]
use embassy_futures::select::{Either, select};
#[cfg(feature = "in_std")]
use futures::future::{Either, select};

use crate::{MHNode, node::ble::controller::ExtAdvController};

pub mod controller;
pub mod rx;
pub mod tx;

#[derive(Serialize, Deserialize)]
struct Beacon {
    seq: u16,
}

// TODO: What is the appropriate size?
pub const MAX_ADV: usize = 255;

#[derive(Clone)]
pub struct RxFrame {
    pub addr: BdAddr,
    pub rssi: i8,
    pub data: Vec<u8, MAX_ADV>,
    pub instant: Instant,
}

// Such that the `RunConfig` can tell BleNode that it has found an advertisement
pub struct RxShared {
    // TODO: Could perhaps be a channel, since it isn't guaranteed that there should only be 1
    // message received
    pub signal: Signal<CriticalSectionRawMutex, RxFrame>,
}

impl RxShared {
    pub const fn new() -> Self {
        Self {
            signal: Signal::new(),
        }
    }
}

// TODO: Should use type state pattern, such that the user has to spawn the runner and return a
// token to get the proper working type to start working
pub struct BleNode<'a, C: 'static, P: PacketPool = DefaultPacketPool> {
    stack: &'static Stack<'static, C, P>,
    adv_params: AdvertisementParameters,
    shared: &'static RxShared,
    scan_config: ScanConfig<'a>,
}

impl<'a, C, P: PacketPool> BleNode<'a, C, P> {
    pub fn new(
        stack: &'static Stack<'static, C, P>,
        adv_params: AdvertisementParameters,
        shared: &'static RxShared,
        scan_config: ScanConfig<'a>,
    ) -> Self {
        Self {
            stack,
            adv_params,
            shared,
            scan_config,
        }
    }
}

const OVERHEAD: usize = 0;
const NAME: &[u8] = b"ESP32";

impl<'a, C, P, const SIZE: usize, const LEN: usize> MHNode<SIZE, LEN> for BleNode<'a, C, P>
where
    P: PacketPool,
    C: Controller + ExtAdvController,
{
    type Error = BleHostError<C::Error>;
    type RadioError = ();
    type Connection = RxFrame;
    type ReceiveBuffer = [u8; MAX_ADV];

    // TODO: For TDMA: Add length of time slot duration
    async fn transmit(&mut self, packet: &[crate::MHPacket<SIZE>]) -> Result<(), Self::Error> {
        const { assert!(LEN * (SIZE + OVERHEAD) <= MAX_ADV) }
        let mut peri = self.stack.peripheral();
        let mut scratch = [0u8; MAX_ADV];
        let mut adv_data = [0u8; MAX_ADV];

        let data = match to_slice(packet, &mut scratch) {
            Ok(data) => data,
            Err(_e) => return Err(BleHostError::BleHost(trouble_host::Error::Other)),
        };

        info!("Sending...");

        let data_len = AdStructure::encode_slice(
            &[
                AdStructure::Flags(
                    trouble_host::advertise::LE_GENERAL_DISCOVERABLE
                        | trouble_host::advertise::BR_EDR_NOT_SUPPORTED,
                ),
                AdStructure::CompleteLocalName(NAME),
                AdStructure::ManufacturerSpecificData {
                    company_identifier: 0xFFFF,
                    payload: data,
                },
            ],
            &mut adv_data,
        )
        .unwrap();
        let adv_set = AdvertisementSet {
            params: self.adv_params,
            data: Advertisement::ExtNonconnectableNonscannableUndirected {
                anonymous: false,
                adv_data: &adv_data[..data_len],
            },
            address: None,
        };

        let sets = &[adv_set];
        let mut handles = AdvertisementSet::handles(sets);
        let _adv = match peri.advertise_ext(sets, &mut handles).await {
            Ok(adv) => adv,
            Err(e) => {
                error!("Error with extended advertisement: {:?}", e);
                Timer::after(Duration::from_secs(10)).await;
                return Err(e);
            }
        };
        // TODO: For how long should we advertise?
        Timer::after(Duration::from_secs(1)).await;

        // NOTE: When we return, the peripheral is dropped and it should stop advertising
        Ok(())
    }

    async fn receive(
        &mut self,
        conn: Self::Connection,
        _rec_buf: &Self::ReceiveBuffer,
    ) -> Result<(heapless::Vec<crate::MHPacket<SIZE>, LEN>, crate::RxPacket), Self::Error> {
        // Basically just take the connection with the data, and return that
        let packets = match from_bytes::<Vec<MHPacket<SIZE>, LEN>>(&conn.data) {
            Ok(packet) => packet,
            Err(e) => {
                error!("Deserialization failed: {:?}", e);
                return Err(BleHostError::BleHost(trouble_host::Error::Other));
            }
        };
        let rx_pkt = RxPacket {
            payload_size: conn.data.len() as u8,
            rx_done_instant: conn.instant,
        };
        Ok((packets, rx_pkt))
    }

    async fn listen(
        &mut self,
        _rec_buf: &mut Self::ReceiveBuffer,
        with_timeout: Option<core::time::Duration>,
    ) -> Result<Self::Connection, Self::Error> {
        // NOTE: Here we should start scanning for advertisements, but
        let timeout = with_timeout.and_then(|d| Duration::try_from(d).ok());
        let timer = async {
            match timeout {
                Some(t) => Timer::after(t).await,
                None => core::future::pending::<()>().await,
            }
        };
        // Discard as this would lead to confusing behaviour
        self.shared.signal.reset();
        // Sets up session, such that host starts scanning
        let mut central = self.stack.central();
        let mut scanner = Scanner::new(&mut central);

        info!("Starting scanning");
        let session = match scanner.scan_ext(&self.scan_config).await {
            Ok(s) => s,
            Err(e) => {
                error!("Scanner failed to start: {:?}", e);
                return Err(e);
            }
        };
        match select(timer, self.shared.signal.wait()).await {
            Either::First(_) => {
                // TODO:
                // Means we timed out, return with timeout error
                info!("Timed out after {}!", timeout);
                session.stop().await;
                return Err(BleHostError::BleHost(
                    trouble_host::Error::NoChannelAvailable,
                ));
            }
            Either::Second(data) => {
                // The scan was successfull, so we return the received data
                // NOTE: This is a signal, so only the first advertisement caught is taken
                session.stop().await;
                return Ok(data);
            }
        }
    }

    /// For now it is just 0
    fn calc_tx_delay(&self, _payload_len: usize) -> u64 {
        0
    }
}

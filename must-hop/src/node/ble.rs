use bt_hci::param::BdAddr;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};
use heapless::Vec;
use postcard::to_slice;
use serde::{Deserialize, Serialize};
/// This contains node implementations for BLE
use trouble_host::{
    Controller, PacketPool, Stack,
    advertise::{Advertisement, AdvertisementParameters, AdvertisementSet},
    prelude::DefaultPacketPool,
};

use crate::{MHNode, node::ble::controller::ExtAdvController};

pub mod controller;
pub mod rx;
pub mod tx;

#[derive(Serialize, Deserialize)]
struct Beacon {
    seq: u16,
}

// TODO: What is the appropriate size?
pub const MAX_ADV: usize = 256;

#[derive(Clone)]
pub struct RxFrame {
    pub addr: BdAddr,
    pub rssi: i8,
    pub data: Vec<u8, MAX_ADV>,
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
pub struct BleNode<C: 'static, P: PacketPool = DefaultPacketPool> {
    stack: &'static Stack<'static, C, P>,
    adv_params: AdvertisementParameters,
    shared: &'static RxShared,
}

impl<C, P: PacketPool> BleNode<C, P> {
    pub fn new(
        stack: &'static Stack<'static, C, P>,
        adv_params: AdvertisementParameters,
        shared: &'static RxShared,
    ) -> Self {
        Self {
            stack,
            adv_params,
            shared,
        }
    }
}

const TRANSMISSION_BUFFER: usize = 128;

impl<C, P, const SIZE: usize, const LEN: usize> MHNode<SIZE, LEN> for BleNode<C, P>
where
    P: PacketPool,
    C: Controller + ExtAdvController,
{
    type Error = ();
    type RadioError = ();
    type Connection = ();
    type ReceiveBuffer = [u8; TRANSMISSION_BUFFER];

    async fn transmit(&mut self, packet: &[crate::MHPacket<SIZE>]) -> Result<(), Self::Error> {
        let mut peri = self.stack.peripheral();
        // TODO: Is this just unecessary after data has been made?
        let mut adv_data = [0u8; TRANSMISSION_BUFFER];

        let data = match to_slice(packet, &mut adv_data) {
            Ok(data) => data,
            Err(_e) => return Err(()),
        };
        let adv_set = AdvertisementSet {
            params: self.adv_params,
            data: Advertisement::ExtNonconnectableNonscannableUndirected {
                anonymous: false,
                adv_data: &data,
            },
            address: None,
        };

        let sets = &[adv_set];
        let mut handles = AdvertisementSet::handles(sets);
        let _adv = match peri.advertise_ext(sets, &mut handles).await {
            Ok(adv) => adv,
            Err(_e) => {
                // error!("Error with start_periodic: {:?}", e);
                return Err(());
            }
        };
        // TODO: For how long should we advertise?
        Timer::after(Duration::from_secs(1)).await;

        // NOTE: When we return, the peripheral is dropped and it should stop advertising
        Ok(())
    }

    async fn receive(
        &mut self,
        _conn: Self::Connection,
        _rec_buf: &Self::ReceiveBuffer,
    ) -> Result<(heapless::Vec<crate::MHPacket<SIZE>, LEN>, crate::RxPacket), Self::Error> {
        todo!()
    }

    async fn listen(
        &mut self,
        rec_buf: &mut Self::ReceiveBuffer,
        with_timeout: Option<core::time::Duration>,
    ) -> Result<Self::Connection, Self::Error> {
        // NOTE: Here we should start scanning for advertisements, but
        todo!()
    }

    /// For now it is just 0
    fn calc_tx_delay(&self, _payload_len: usize) -> u64 {
        0
    }
}

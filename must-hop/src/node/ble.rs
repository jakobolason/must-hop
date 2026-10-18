use bt_hci::{
    cmd::le::{LeClearAdvSets, LeSetAdvSetRandomAddr, LeSetExtAdvData, LeSetExtAdvParams},
    controller::ControllerCmdSync,
};
use embassy_time::{Duration, Timer};
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

// TODO: Should use type state pattern, such that the user has to spawn the runner and return a
// token to get the proper working type to start working
pub struct BleNode<C: 'static, P: PacketPool = DefaultPacketPool> {
    stack: &'static Stack<'static, C, P>,
    adv_params: AdvertisementParameters,
}

impl<C, P: PacketPool> BleNode<C, P> {
    pub fn new(stack: &'static Stack<'static, C, P>, adv_params: AdvertisementParameters) -> Self {
        Self { stack, adv_params }
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
            Err(e) => return Err(()),
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
            Err(e) => {
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
        conn: Self::Connection,
        rec_buf: &Self::ReceiveBuffer,
    ) -> Result<(heapless::Vec<crate::MHPacket<SIZE>, LEN>, crate::RxPacket), Self::Error> {
        todo!()
    }

    async fn listen(
        &mut self,
        rec_buf: &mut Self::ReceiveBuffer,
        with_timeout: Option<core::time::Duration>,
    ) -> Result<Self::Connection, Self::Error> {
        todo!()
    }

    fn calc_tx_delay(&self, payload_len: usize) -> u64 {
        todo!()
    }
}

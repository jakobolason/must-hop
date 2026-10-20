use bt_hci::param::BdAddr;
use defmt::{error, info};

use heapless::Vec;
use trouble_host::{
    HostResources, Stack,
    advertise::AdStructure,
    prelude::{DefaultPacketPool, EventHandler},
    scan::LeAdvReportsIter,
};

use crate::node::ble::{Beacon, RxShared};

pub type Pool = DefaultPacketPool;
pub type BleStack<Ctrl> = Stack<'static, Ctrl, Pool>;
pub type Res = HostResources<Pool, 0, 0>;

pub struct RunConfig<const PEERS: usize> {
    peers: Vec<BdAddr, PEERS>,
    shared: &'static RxShared,
}

impl<const PEERS: usize> RunConfig<PEERS> {
    pub fn new(shared: &'static RxShared) -> Self {
        Self {
            peers: Vec::new(),
            shared,
        }
    }

    pub fn new_from_slice(peers: Vec<BdAddr, PEERS>, shared: &'static RxShared) -> Self {
        Self { peers, shared }
    }
}

impl<const PEERS: usize> EventHandler for RunConfig<PEERS> {
    // TODO: Use channels, such that these callback functions can give information back to BleNode
    fn on_ext_adv_reports(&self, mut it: bt_hci::param::LeExtAdvReportsIter) {
        while let Some(Ok(report)) = it.next() {
            let matches_peer = self.peers.contains(&report.addr);
            if matches_peer && report.adv_sid == 0 {
                info!(
                    "OK! Extended adv from {:?}, rssi={}",
                    report.addr, report.rssi
                );
            }
        }
    }

    fn on_periodic_adv_report(&self, report: &bt_hci::event::le::LePeriodicAdvertisingReport) {
        info!("OK! Periodic adv report from {:?}", report);
        for ad in AdStructure::decode(report.data) {
            let ad = match ad {
                Ok(ad) => ad,
                Err(e) => {
                    error!("Error in decoding error: {:?}", e);
                    continue;
                }
            };
            if let AdStructure::ManufacturerSpecificData {
                company_identifier: 0xFFFF,
                payload,
            } = ad
            {
                match postcard::from_bytes::<Beacon>(payload) {
                    Ok(b) => info!("per report: rssi={} seq={}", report.rssi, b.seq),
                    Err(_) => info!("bad payload"),
                }
            }
        }
    }

    fn on_sync(&self, report: &bt_hci::event::le::LePeriodicAdvertisingSyncEstablished) {
        info!("Is synced now!! {}", report.adv_addr);
    }

    fn on_adv_reports(&self, mut it: LeAdvReportsIter<'_>) {
        while let Some(Ok(report)) = it.next() {
            // report.addr, report.rssi, report.data (raw AD bytes)
            for ad in AdStructure::decode(report.data).flatten() {
                if let AdStructure::ManufacturerSpecificData {
                    company_identifier: 0xFFFF,
                    payload,
                } = ad
                {
                    match postcard::from_bytes::<Beacon>(payload) {
                        Ok(b) => info!("report: rssi={} seq={}", report.rssi, b.seq),
                        Err(_) => info!("bad payload from {:?}", report.addr),
                    }
                }
            }
        }
    }
}

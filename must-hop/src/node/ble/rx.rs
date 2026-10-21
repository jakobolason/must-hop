use core::cell::RefCell;

use crate::log_compat::{debug, error, info};
use bt_hci::param::{BdAddr, LeExtAdvDataStatus};

use embassy_time::Instant;
use heapless::Vec;
use trouble_host::{
    HostResources, Stack,
    advertise::AdStructure,
    prelude::{DefaultPacketPool, EventHandler},
    scan::LeAdvReportsIter,
};

use crate::node::ble::{Beacon, MAX_ADV, RxFrame, RxShared};

pub type Pool = DefaultPacketPool;
pub type BleStack<Ctrl> = Stack<'static, Ctrl, Pool>;
pub type Res = HostResources<Pool, 0, 0>;

struct Reassembly {
    addr: BdAddr,
    buf: Vec<u8, MAX_ADV>,
}

impl Reassembly {
    fn new(addr: BdAddr, buf: Vec<u8, MAX_ADV>) -> Self {
        Self { addr, buf }
    }
}

pub struct RunConfig<const PEERS: usize> {
    peers: Vec<BdAddr, PEERS>,
    shared: &'static RxShared,
    reassembly: RefCell<Option<Reassembly>>,
}

impl<const PEERS: usize> RunConfig<PEERS> {
    pub fn new(shared: &'static RxShared) -> Self {
        Self {
            peers: Vec::new(),
            reassembly: RefCell::new(None),
            shared,
        }
    }

    pub fn new_from_slice(peers: Vec<BdAddr, PEERS>, shared: &'static RxShared) -> Self {
        Self {
            peers,
            shared,
            reassembly: RefCell::new(None),
        }
    }
}

impl<const PEERS: usize> EventHandler for RunConfig<PEERS> {
    // NOTE: We really only want a single signal, per time we call listen()
    fn on_ext_adv_reports(&self, it: bt_hci::param::LeExtAdvReportsIter) {
        for report in it.flatten() {
            if !self.peers.contains(&report.addr) {
                continue;
            }
            let status = report.event_kind.data_status();
            let pending = self.reassembly.borrow_mut().take();
            let reassembled;

            let data = match status {
                LeExtAdvDataStatus::Complete => {
                    // If we were accumulating, append this fragment, then decode the whole buffer.
                    // If not, decode this report directly.
                    debug!("in complete");
                    match pending {
                        Some(mut res) if res.addr == report.addr => {
                            if res.buf.extend_from_slice(report.data).is_err() {
                                error!("Report data was too big for the Reassembly buffer!");
                                // TODO: Should probably take res here, because its useless now
                                return;
                            };
                            reassembled = res.buf;
                            &reassembled
                        }
                        _other => {
                            return;
                        }
                    }
                }
                LeExtAdvDataStatus::IncompleteMoreExpected => {
                    debug!("In IncompleteMoreExpected");
                    let res = match pending {
                        Some(mut res) if res.addr == report.addr => {
                            if res.buf.extend_from_slice(report.data).is_err() {
                                error!("Report data was too big for the Reassembly buffer!");
                                // TODO: Should probably take res here, because its useless now
                                return;
                            };
                            res
                        }
                        _ => match Vec::from_slice(report.data) {
                            Ok(buf) => Reassembly::new(report.addr, buf),
                            Err(_) => {
                                error!("Could not create vec from report data!");
                                return;
                            }
                        },
                    };
                    *self.reassembly.borrow_mut() = Some(res);
                    return;
                }
                LeExtAdvDataStatus::Reserved => {
                    // TODO: What is this?
                    info!("Is in LeExtAdvDataStatus Reserved?");
                    return;
                }
                LeExtAdvDataStatus::IncompleteTruncated => {
                    // Discard the reassembly buffer for this address.
                    // Optionally log that we missed a fragment.
                    error!("Advertisements were truncated, what to do?");
                    let _ = self.reassembly.take();
                    return;
                }
            };
            info!("report len={} from {:?}", report.data.len(), report.addr);
            for ad in AdStructure::decode(data).flatten() {
                let AdStructure::ManufacturerSpecificData {
                    company_identifier: _,
                    payload,
                } = ad
                else {
                    error!("No manufacturer data found?");
                    continue;
                };

                info!(
                    "OK! Extended adv from {:?}, rssi={}",
                    report.addr, report.rssi
                );
                let frame = RxFrame {
                    addr: report.addr,
                    data: Vec::from_slice(payload).unwrap_or(Vec::new()),
                    rssi: report.rssi,
                    instant: Instant::now(),
                };
                self.shared.signal(frame);
                return;
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
        info!("Is synced now!! {:?}", report.adv_addr);
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

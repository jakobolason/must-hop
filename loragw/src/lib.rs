#![deny(missing_docs)]

//! This crate provides a high-level interface which serves as
//! building-block for creating LoRa gateways using the
//! [SX1301](https://www.semtech.com/products/wireless-rf/lora-gateways/sx1301)
//! concentrator chip.

#[macro_use]
mod error;
mod types;
pub use crate::error::*;
pub use crate::types::*;
use std::{
    cell::Cell,
    convert::{TryFrom, TryInto},
    marker::PhantomData,
    ops,
    sync::atomic::{AtomicBool, Ordering},
    thread, time,
};

// pub(crate) use libloragw_sys as llg;
pub(crate) use libloragw_sys as llg;

// Ensures we only have 0 or 1 gateway instances opened at a time.
// This is not a great solution, since another process has its
// own count.
static GW_IS_OPEN: AtomicBool = AtomicBool::new(false);

/// A LoRa concentrator.
pub struct Concentrator {
    /// Used to prevent `self` from auto implementing `Sync`.
    ///
    /// This is necessary because the `libloragw` makes liberal use of
    /// globals and is not thread-safe.
    _prevent_sync: PhantomData<Cell<()>>,
}

impl Concentrator {
    /// Open the spidev-connected concentrator.
    pub fn open() -> Result<Self> {
        // We expect `false`, and want to swap to `true`.
        // If it fails (is_err), the lock is already held.
        if GW_IS_OPEN
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            eprintln!("concentrator busy");
            return Err(Error::Busy); // Make sure Error::Busy is properly in scope!
        }

        Ok(Concentrator {
            _prevent_sync: PhantomData,
        })
    }

    /// Configure the gateway board.
    pub fn config_board(&self, conf: &BoardConf) -> Result {
        println!("conf: {:?}", conf);
        unsafe { hal_call!(lgw_board_setconf(&mut conf.into())) }?;
        Ok(())
    }

    /// Configure an RF chain.
    pub fn config_rx_rf(&self, conf: &RxRFConf) -> Result {
        println!("{:?}", conf);
        unsafe { hal_call!(lgw_rxrf_setconf(conf.radio as u8, &mut conf.into())) }?;
        Ok(())
    }

    /// Configure an IF chain + modem (must configure before start).
    pub fn config_channel(&self, chain: u8, conf: &ChannelConf) -> Result {
        println!("chain: {}, conf: {:?}", chain, conf);
        unsafe { hal_call!(lgw_rxif_setconf(chain, &mut conf.into())) }?;
        Ok(())
    }

    /// Configure the Tx gain LUT.
    pub fn config_tx_gain(&self, gains: &[TxGain]) -> Result {
        if gains.is_empty() || gains.len() > 16 {
            eprintln!(
                "gain table must contain 1 to 16 entries, {} provided",
                gains.len()
            );
            return Err(Error::Size);
        }
        println!("gains: {:?}", gains);
        let mut lut = TxGainLUT::default();
        lut.lut[..gains.len()].clone_from_slice(gains);
        lut.size = gains.len() as u8;
        unsafe {
            // TODO: What should this u8 be?
            hal_call!(lgw_txgain_setconf(
                0u8,
                &mut lut as *mut TxGainLUT as *mut llg::lgw_tx_gain_lut_s
            ))
        }?;
        Ok(())
    }

    /// according to previously set parameters.
    pub fn start(&self) -> Result {
        println!("starting concentrator");
        unsafe { hal_call!(lgw_start()) }?;
        Ok(())
    }

    /// Stop the LoRa concentrator and disconnect it.
    pub fn stop(&self) -> Result {
        println!("stopping concentrator");
        unsafe { hal_call!(lgw_stop()) }?;
        Ok(())
    }

    /// Returns the concentrators current receive status.
    pub fn receive_status(&self) -> Result<RxStatus> {
        const RX_STATUS: u8 = 2;
        let mut rx_status = 0xFE;
        unsafe {
            hal_call!(lgw_status(
                {
                    println!("remove hardcoded RF chain argument from status calls");
                    0u8
                },
                RX_STATUS,
                &mut rx_status
            ))
        }?;
        rx_status.try_into()
    }

    /// Perform a non-blocking read of up to 16 packets from
    /// concentrator's FIFO.
    pub fn receive(&self) -> Result<Option<Vec<RxPacket>>> {
        let mut tmp_buf: [llg::lgw_pkt_rx_s; 16] = unsafe { std::mem::zeroed() };
        let len = unsafe { hal_call!(lgw_receive(tmp_buf.len() as u8, tmp_buf.as_mut_ptr())) }?;
        if len > 0 {
            let mut out = Vec::new();
            for tmp in tmp_buf[..len].iter() {
                out.push(RxPacket::try_from(tmp)?);
            }
            Ok(Some(out))
        } else {
            Ok(None)
        }
    }

    // TODO: How to do this
    // /// Transmit `packet` over the air.
    // pub fn transmit(&self, packet: TxPacket) -> Result {
    //     while self.transmit_status()? != TxStatus::Free {
    //         const SLEEP_TIME: time::Duration = time::Duration::from_millis(5);
    //         println!("transmitter is busy, sleeping for {:?}", SLEEP_TIME);
    //         thread::sleep(SLEEP_TIME);
    //     }
    //     unsafe { hal_call!(lgw_send(&mut packet.try_into()?)) }?;
    //     Ok(())
    // }

    /// Attempt to connect to concentrator.
    ///
    /// This function is intended to check if we the concentrator chip
    /// exists and is the correct version.
    pub fn connect(&self, spidev_path: &::std::ffi::CStr) -> Result {
        // TODO: Find out what this u32 should be
        unsafe { hal_call!(lgw_connect(1u32, spidev_path.as_ptr())) }?;
        Ok(())
    }
}

// Private functions.
impl Concentrator {
    /// Returns the concentrators current transmit status.
    ///
    /// We keep this private since `transmit` uses it internally, and
    /// it may lead to confusion about who's responsibility it is to
    /// check TX status.
    fn transmit_status(&self) -> Result<TxStatus> {
        const TX_STATUS: u8 = 1;
        let mut tx_status = 0xFE;
        unsafe {
            hal_call!(lgw_status(
                {
                    println!("[WARN] remove hardcoded RF chain argument from status calls");
                    0u8
                },
                TX_STATUS,
                &mut tx_status
            ))
        }?;
        tx_status.try_into()
    }
}

impl ops::Drop for Concentrator {
    fn drop(&mut self) {
        println!("closing concentrator");
        GW_IS_OPEN.store(false, Ordering::Release);
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use std::sync::Mutex;
//
//     lazy_static! {
//         static ref TEST_MUTEX: Mutex<()> = Mutex::new(());
//     }
//
//     #[test]
//     fn test_open_close_succeeds() {
//         let _lock = TEST_MUTEX.lock().unwrap();
//         assert!(!GW_IS_OPEN.load(Ordering::Relaxed));
//         {
//             let _gw = Concentrator::open().unwrap();
//             assert!(GW_IS_OPEN.load(Ordering::Relaxed));
//             // _gw `drop`ped here
//         }
//         assert!(!GW_IS_OPEN.load(Ordering::Relaxed));
//     }
//
//     #[test]
//     fn test_double_open_fails() {
//         let _lock = TEST_MUTEX.lock().unwrap();
//         assert!(!GW_IS_OPEN.load(Ordering::Relaxed));
//         let _gw1 = Concentrator::open().unwrap();
//         assert!(GW_IS_OPEN.load(Ordering::Relaxed));
//         assert!(Concentrator::open().is_err());
//     }
// }

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
    sync::atomic::{AtomicBool, Ordering},
};

pub mod cfg;
pub mod raspberrypi;
// pub(crate) use libloragw_sys as llg;
pub(crate) use libloragw_sys as llg;

// Ensures we only have 0 or 1 gateway instances opened at a time.
// This is not a great solution, since another process has its
// own count.
static GW_IS_OPEN: AtomicBool = AtomicBool::new(false);
struct GatewayGuard {}

impl Drop for GatewayGuard {
    fn drop(&mut self) {
        log::info!("Cleaning up gateway resources");
        unsafe {
            let _ = hal_call!(lgw_stop());
        }
        GW_IS_OPEN.store(false, Ordering::SeqCst);
    }
}

pub struct Closed {}
#[derive(Default)]
pub struct Builder<'a> {
    connected: bool,
    board: Option<BoardConf>,
    rx_rf_conf: Vec<RxRFConf>,
    gains: &'a [TxGain],
    channel_conf: Vec<(u8, ChannelConf)>,
}
pub struct Running {}

/// A LoRa concentrator.
pub struct Concentrator<State> {
    /// Used to prevent `self` from auto implementing `Sync`.
    ///
    /// This is necessary because the `libloragw` makes liberal use of
    /// globals and is not thread-safe.
    _prevent_sync: PhantomData<Cell<()>>,
    _guard: GatewayGuard,
    state: State,
}

/// To ensure the concentrator is reset before use. Otherwise a HAL error will occur.
pub struct ResetToken {
    _priv: (),
}

impl ResetToken {
    pub fn generate<F, E>(reset_routine: F) -> std::result::Result<Self, E>
    where
        F: FnOnce() -> std::result::Result<(), E>,
    {
        reset_routine()?;
        Ok(ResetToken { _priv: () })
    }

    /// Unsafe bypass if you are sure the concentrator is reset before use
    pub unsafe fn bypass() -> Self {
        ResetToken { _priv: () }
    }
}

impl Concentrator<Closed> {
    // Open the spidev-connected concentrator.
    pub fn open<'a>(_token: &ResetToken) -> Result<Concentrator<Builder<'a>>> {
        // We expect `false`, and want to swap to `true`.
        // If it fails (is_err), the lock is already held.
        if GW_IS_OPEN
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            log::error!("concentrator busy");
            return Err(Error::Busy); // Make sure Error::Busy is properly in scope!
        }
        log::info!("Gateware model initialized");

        Ok(Concentrator {
            _prevent_sync: PhantomData,
            _guard: GatewayGuard {},
            state: Builder {
                ..Default::default()
            },
        })
    }
}

impl<'a> Concentrator<Builder<'a>> {
    /// Attempt to connect to concentrator.
    ///
    /// This function is intended to check if we the concentrator chip
    /// exists and is the correct version.
    pub fn connect(mut self) -> Result<Self> {
        log::info!("self state: {:?}", self.state.board);
        let board_conf = self
            .state
            .board
            .as_ref()
            .ok_or(Error::BuilderError(BuilderError::MissingBoard))?;
        let com_type = board_conf.com_type.clone();
        let spidev_path = board_conf.spidev_path.clone();
        unsafe { hal_call!(lgw_connect(com_type as u32, spidev_path.as_ptr())) }?;
        self.state.connected = true;
        Ok(self)
    }

    /// Configure the gateway board.
    pub fn set_config_board(mut self, conf: BoardConf) -> Self {
        log::info!("conf: {:?}", conf);
        self.state.board = Some(conf);
        self
    }

    /// Configure an RF chain.
    pub fn set_rx_rfs(mut self, conf: Vec<RxRFConf>) -> Self {
        log::info!("{:?}", conf);
        self.state.rx_rf_conf = conf;
        self
    }
    pub fn add_rx_rf(mut self, conf: RxRFConf) -> Self {
        log::info!("conf: {:?}", conf);
        self.state.rx_rf_conf.push(conf);
        self
    }

    /// Configure an IF chain + modem (must configure before start).
    pub fn set_config_channels(mut self, confs: Vec<(u8, ChannelConf)>) -> Self {
        // log::info!("chain: {}, conf: {:?}", chain, conf);
        self.state.channel_conf = confs;
        self
    }
    pub fn add_config_channel(mut self, chain: u8, conf: ChannelConf) -> Self {
        log::info!("chain: {}, conf: {:?}", chain, conf);
        self.state.channel_conf.push((chain, conf));
        self
    }

    /// Configure the Tx gain LUT.
    pub fn set_config_tx_gains(mut self, gains: &'a [TxGain]) -> Self {
        log::info!("gains: {:?}", gains);
        self.state.gains = gains;
        self
    }

    /// according to previously set parameters.
    pub fn start(self) -> Result<Concentrator<Running>> {
        if !self.state.connected {
            return Err(Error::BuilderError(BuilderError::NotConnected));
        }
        log::info!("starting concentrator");
        // board config
        let board = match self.state.board {
            Some(board) => board,
            None => return Err(Error::BuilderError(BuilderError::MissingBoard)),
        };
        unsafe { hal_call!(lgw_board_setconf(&mut board.into())) }?;

        // rx_rf chain
        self.state.rx_rf_conf.iter().try_for_each(|c| unsafe {
            hal_call!(lgw_rxrf_setconf(c.radio as u8, &mut c.into())).map(|_| ())
        })?;

        // configure IF chain + modem
        self.state
            .channel_conf
            .iter()
            .try_for_each(|(chain, chan_conf)| unsafe {
                hal_call!(lgw_rxif_setconf(*chain, &mut chan_conf.into())).map(|_| ())
            })?;

        // conf Tx gain LUT
        let gains = self.state.gains;
        if gains.is_empty() || gains.len() > 16 {
            log::error!(
                "gain table must contain 1 to 16 entries, {} provided",
                gains.len()
            );
            return Err(Error::Size);
        }
        let mut lut = TxGainLUT::default();
        lut.lut[..gains.len()].clone_from_slice(gains);
        lut.size = gains.len() as u8;
        unsafe {
            // TODO: de-hardcode this 0u8 (? from helium)
            hal_call!(lgw_txgain_setconf(
                0u8,
                &mut lut as *mut TxGainLUT as *mut llg::lgw_tx_gain_lut_s
            ))
        }?;

        // Now we ready to start
        unsafe { hal_call!(lgw_start()) }?;
        Ok(Concentrator {
            _prevent_sync: PhantomData,
            _guard: self._guard,
            state: Running {},
        })
    }
}

impl Concentrator<Running> {
    /// Returns the concentrators current receive status.
    pub fn receive_status(&self) -> Result<RxStatus> {
        const RX_STATUS: u8 = 2;
        let mut rx_status = 0xFE;
        unsafe {
            hal_call!(lgw_status(
                {
                    log::info!("remove hardcoded RF chain argument from status calls");
                    0u8
                },
                RX_STATUS,
                &mut rx_status
            ))
        }?;
        log::info!("Received status: {:?}", rx_status);
        rx_status.try_into()
    }

    /// Perform a non-blocking read of up to 16 packets from
    /// concentrator's FIFO.
    pub fn receive(&self) -> Result<Option<Vec<RxPacket>>> {
        log::info!("Setting up receive!");
        let mut tmp_buf: [std::mem::MaybeUninit<llg::lgw_pkt_rx_s>; 16] =
            unsafe { std::mem::MaybeUninit::uninit().assume_init() };

        log::info!("Now calling");
        let len = unsafe {
            hal_call!(lgw_receive(
                tmp_buf.len() as u8,
                tmp_buf.as_mut_ptr() as *mut llg::lgw_pkt_rx_s
            ))
        }?;
        log::info!("Received {} packets", len);
        if len > 0 {
            let mut out = Vec::with_capacity(len as usize);
            for i in 0..(len as usize) {
                // SAFE: We know C initialized up to `len` elements
                let pkt = unsafe { tmp_buf[i].assume_init() };
                out.push(RxPacket::try_from(&pkt)?);
            }
            Ok(Some(out))
        } else {
            Ok(None)
        }
    }

    // TODO: How to do this
    // /// Transmit `packet` over the air.
    pub fn transmit(&self, packet: TxPacket) -> Result {
        unsafe { hal_call!(lgw_send(&mut packet.try_into()?)) }?;
        Ok(())
    }

    /// Stop the LoRa concentrator and disconnect it.
    pub fn stop(self) -> Result<Concentrator<Closed>> {
        log::info!("stopping concentrator");
        unsafe { hal_call!(lgw_stop()) }?;
        Ok(Concentrator {
            _prevent_sync: PhantomData,
            _guard: self._guard,
            state: Closed {},
        })
    }

    /// Returns the concentrators current transmit status.
    ///
    /// We keep this private since `transmit` uses it internally, and
    /// it may lead to confusion about who's responsibility it is to
    /// check TX status.
    pub fn transmit_status(&self) -> Result<TxStatus> {
        const TX_STATUS: u8 = 1;
        let mut tx_status = 0xFE;
        unsafe {
            hal_call!(lgw_status(
                {
                    log::info!("[WARN] remove hardcoded RF chain argument from status calls");
                    0u8
                },
                TX_STATUS,
                &mut tx_status
            ))
        }?;
        tx_status.try_into()
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

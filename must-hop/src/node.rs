/// Provides the MHPacket, describing how a packet looks like on this network.
/// The MHNode describes necessary radio function for NM and MS to work. These should be
/// implemented by the radio used on the specific device
use core::future::Future;
use core::time::Duration;
use heapless::Vec;

/// Implements the `MHNode` trait for all lora-phy compatible radios
pub mod lora;

use crate::{MHPacket, RxPacket};
/// This trait denodes the necessary radio operations a node on the network
/// is required to do, to function properly.
pub trait MHNode<const SIZE: usize, const LEN: usize> {
    #[cfg(not(feature = "in_std"))]
    type Error: core::fmt::Debug + defmt::Format;
    #[cfg(feature = "in_std")]
    type Error: core::fmt::Debug;

    type Connection;
    type ReceiveBuffer;

    /// Takes an MHPacket with a size for the user defined payload. This will be sent to the
    /// appropriate destination_id
    fn transmit(
        &mut self,
        packet: &[MHPacket<SIZE>],
    ) -> impl Future<Output = Result<(), Self::Error>>;

    /// Function needed for this lib, for multi hop communication.
    /// The conn and receiving_buffer might be too LoRa specific, so it might change
    fn receive(
        &mut self,
        conn: Self::Connection,
        rec_buf: &Self::ReceiveBuffer,
    ) -> impl Future<Output = Result<(Vec<MHPacket<SIZE>, LEN>, RxPacket), Self::Error>>;

    /// Make the node listen for a preample, giving a connection relative to the physical layer
    /// used. Optionally a duration to listen in can be given.
    fn listen(
        &mut self,
        rec_buf: &mut Self::ReceiveBuffer,
        with_timeout: Option<Duration>,
    ) -> impl Future<Output = Result<Self::Connection, Self::Error>>;

    /// For time sensitive packets and physical layers with predictable Tx times, this can be used
    /// to send a timestamp which might be closer to the actual timestamp.
    fn calc_tx_delay(&self, payload_len: usize) -> u64;
}

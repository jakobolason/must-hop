use crate::node::MHNode;

#[cfg(not(feature = "in_std"))]
use defmt::{error, info};
#[cfg(feature = "in_std")]
use log::{error, info};

use super::{
    MHPacket,
    network_manager::{NetworkManager, NetworkManagerError},
};

use embassy_time::{Duration, Instant, Timer};
use heapless::Vec;

pub trait RoutingPolicy<const SIZE: usize, const LEN: usize> {
    fn check_heartbeat(
        &mut self,
        manager: &mut NetworkManager<SIZE, LEN>,
    ) -> Result<Option<MHPacket<SIZE>>, NetworkManagerError>;
}

pub struct NodePolicy;
impl<const SIZE: usize, const LEN: usize> RoutingPolicy<SIZE, LEN> for NodePolicy {
    fn check_heartbeat(
        &mut self,
        _manager: &mut NetworkManager<SIZE, LEN>,
    ) -> Result<Option<MHPacket<SIZE>>, NetworkManagerError> {
        Ok(None)
    }
}

/// A gateway sends out periodic heartbeats
#[cfg(feature = "in_std")]
pub struct GatewayPolicy {
    pub last_heartbeat: Option<Instant>,
    pub timeout: u64,
}
#[cfg(feature = "in_std")]
impl GatewayPolicy {
    pub fn new(timeout: u64) -> Self {
        Self {
            last_heartbeat: None,
            timeout,
        }
    }
}

#[cfg(feature = "in_std")]
impl<const SIZE: usize, const LEN: usize> RoutingPolicy<SIZE, LEN> for GatewayPolicy {
    fn check_heartbeat(
        &mut self,
        manager: &mut NetworkManager<SIZE, LEN>,
    ) -> Result<Option<MHPacket<SIZE>>, NetworkManagerError> {
        let now = Instant::now();
        let should_send = match self.last_heartbeat {
            None => true,
            Some(last) => now.duration_since(last) >= Duration::from_secs(self.timeout),
        };
        if should_send {
            self.last_heartbeat = Some(now);
            let pkt = manager.add_heartbeat()?;
            Ok(Some(pkt))
        } else {
            Ok(None)
        }
    }
}

pub trait MacPolicy<Node, const SIZE: usize, const LEN: usize>
where
    Node: MHNode<SIZE, LEN>,
{
    fn run_mac(
        &mut self,
        node: &mut Node,
        tx_queue: &mut Vec<MHPacket<SIZE>, LEN>,
        rx_buffer: &mut Node::ReceiveBuffer,
    ) -> impl Future<Output = Result<Option<Vec<MHPacket<SIZE>, LEN>>, Node::Error>>;
}

pub struct RandomAccessMac;

impl<Node, const SIZE: usize, const LEN: usize> MacPolicy<Node, SIZE, LEN> for RandomAccessMac
where
    Node: MHNode<SIZE, LEN>,
{
    async fn run_mac(
        &mut self,
        node: &mut Node,
        tx_queue: &mut Vec<MHPacket<SIZE>, LEN>,
        rx_buffer: &mut Node::ReceiveBuffer,
    ) -> Result<Option<Vec<MHPacket<SIZE>, LEN>>, Node::Error> {
        if !tx_queue.is_empty() {
            node.transmit(tx_queue).await?;
            tx_queue.clear();
        }
        match node.listen(rx_buffer, true).await {
            Ok(conn) => match node.receive(conn, rx_buffer).await {
                Ok(pkts) => Ok(Some(pkts)),
                Err(e) => Err(e),
            },
            Err(e) => {
                // error!("Error in listening: {:?}", e);
                Ok(None)
            }
        }
    }
}

pub struct TdmaMac {
    pub slot_duration: Duration,
    pub slots_per_frame: u8,
    pub my_tx_slot: u8,
    pub epoch: Instant,
}

impl TdmaMac {
    pub fn new(
        slot_duration: Duration,
        slots_per_frame: u8,
        my_tx_slot: u8,
        epoch: Instant,
    ) -> Self {
        Self {
            slot_duration,
            slots_per_frame,
            my_tx_slot,
            epoch,
        }
    }

    pub fn current_slot(&self, now: Instant) -> u8 {
        let elapsed_ms = (now - self.epoch).as_millis();
        let frame_duration_ms = (self.slot_duration.as_millis()) * (self.slots_per_frame as u64);

        let time_in_frame = elapsed_ms % frame_duration_ms;
        (time_in_frame / (self.slot_duration.as_millis())) as u8
    }
}

impl<Node, const SIZE: usize, const LEN: usize> MacPolicy<Node, SIZE, LEN> for TdmaMac
where
    Node: MHNode<SIZE, LEN>,
{
    async fn run_mac(
        &mut self,
        node: &mut Node,
        tx_queue: &mut Vec<MHPacket<SIZE>, LEN>,
        rx_buffer: &mut Node::ReceiveBuffer,
    ) -> Result<Option<Vec<MHPacket<SIZE>, LEN>>, Node::Error> {
        let now = Instant::now();
        let slot = self.current_slot(now);

        // Calculate when the next slot starts
        let elapsed_ms = (now - self.epoch).as_millis();
        let next_slot_start_ms = elapsed_ms + self.slot_duration.as_millis()
            - (elapsed_ms % self.slot_duration.as_millis());
        let next_slot_time = self.epoch + Duration::from_millis(next_slot_start_ms);

        let mut received_packets = Vec::new();

        if slot == self.my_tx_slot {
            if !tx_queue.is_empty() {
                node.transmit(tx_queue).await?;
                tx_queue.clear();
            }
            Timer::at(next_slot_time).await;
        } else {
            let conn = node.listen(rx_buffer, true).await;
            if let Ok(conn) = conn {
                received_packets = node.receive(conn, rx_buffer).await?;
            } else {
                info!("Error in getting conn");
            }
            Timer::at(next_slot_time).await;
        }
        Ok(Some(received_packets))
    }
}

use crate::node::MHNode;

use super::MHPacket;

use heapless::Vec;

const GATEWAY_ID: u8 = 1;

pub mod ra;
pub mod tdma;
// pub use ra::{GatewayPolicy, NodePolicy, RandomAccessMac};
pub use tdma::{Builder, Runner, TdmaMac};

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

    fn should_tx_heartbeat(&mut self) -> bool;
    fn tx_heartbeat(&mut self, hbt: MHPacket<SIZE>);

    fn set_gw_hops(&mut self, gw_hops: u8);
}

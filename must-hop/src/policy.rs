use crate::{node::MHNode, nw_layer::TransmitBuffer};

use super::MeshPacket;

use heapless::Vec;

const GATEWAY_ID: u16 = 1;

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
        tx_queue: &mut TransmitBuffer<SIZE, LEN>,
        rx_buffer: &mut Node::ReceiveBuffer,
    ) -> impl Future<Output = Result<Option<Vec<MeshPacket<SIZE>, LEN>>, Node::Error>>;

    fn should_tx_heartbeat(&mut self) -> bool;
    fn tx_heartbeat(&mut self, hbt: MeshPacket<SIZE>);

    fn set_gw_hops(&mut self, gw_hops: u8);
}

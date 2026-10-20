/// This file describes the channels used for the API
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex as Raw,
    channel::{Channel, Receiver, Sender},
};
use heapless::Vec;

use crate::{MHNode, MeshRouter, log_compat::info, policy::MacPolicy};

pub struct Outgoing<const S: usize> {
    pub dest: u16,
    pub payload: Vec<u8, S>,
}

pub struct Incoming<const S: usize> {
    pub source: u16,
    pub payload: Vec<u8, S>,
}

/// User has to initialize this struct, and hand it to init
pub struct MeshState<const S: usize, const N: usize> {
    out: Channel<Raw, Outgoing<S>, N>,
    inc: Channel<Raw, Incoming<S>, N>,
    events: Channel<Raw, MeshEvent, N>,
}

impl<const S: usize, const N: usize> MeshState<S, N> {
    pub fn new() -> Self {
        Self {
            out: Channel::new(),
            inc: Channel::new(),
            events: Channel::new(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum MeshEvent {
    Dropped,     // router refused the payload
    InboxFull,   // user isn't reading fast enough
    RouterError, // tick failed
}

#[derive(Debug, Clone, Copy)]
pub enum SendError {
    TooLarge,
    QueueFull,
}

pub struct MeshService<Node, Mac, const S: usize, const LEN: usize, const N: usize>
where
    Node: MHNode<S, LEN>,
    Mac: MacPolicy<Node, S, LEN>,
{
    router: MeshRouter<Node, Mac, S, LEN>,
    rx_buf: Node::ReceiveBuffer,
    out: Receiver<'static, Raw, Outgoing<S>, N>,
    inc: Sender<'static, Raw, Incoming<S>, N>,
    events: Sender<'static, Raw, MeshEvent, N>,
}

#[derive(Clone, Copy)]
pub struct Mailbox<const S: usize, const N: usize> {
    tx: Sender<'static, Raw, Outgoing<S>, N>,
    rx: Receiver<'static, Raw, Incoming<S>, N>,
    events: Receiver<'static, Raw, MeshEvent, N>,
}

impl<const S: usize, const N: usize> Mailbox<S, N> {
    /// Waits if the outbox is full (backpressure)
    pub async fn send(&self, dest: u16, data: &[u8]) -> Result<(), SendError> {
        let payload = Vec::from_slice(data).map_err(|_| SendError::TooLarge)?;
        self.tx.send(Outgoing { dest, payload }).await;
        Ok(())
    }

    /// Fails immediately if the outbox is full
    pub fn try_send(&self, dest: u16, data: &[u8]) -> Result<(), SendError> {
        let payload = Vec::from_slice(data).map_err(|_| SendError::TooLarge)?;
        self.tx
            .try_send(Outgoing { dest, payload })
            .map_err(|_| SendError::QueueFull)
    }

    /// Wait until there is a message to receive
    pub async fn recv(&self) -> Incoming<S> {
        self.rx.receive().await
    }

    /// Try to immediately receive a value
    pub fn try_recv(&self) -> Option<Incoming<S>> {
        self.rx.try_receive().ok()
    }

    /// Check if any pending messages
    pub fn pending(&self) -> usize {
        self.rx.len()
    }

    /// Drain the channel
    pub fn drain(&self) -> impl Iterator<Item = Incoming<S>> + '_ {
        core::iter::from_fn(move || self.try_recv())
    }

    /// Wait until the next event
    pub async fn next_event(&self) -> MeshEvent {
        self.events.receive().await
    }

    /// Try to immediately fetch an event
    pub fn try_event(&self) -> Option<MeshEvent> {
        self.events.try_receive().ok()
    }
}

impl<Node, Mac, const S: usize, const LEN: usize, const N: usize> MeshService<Node, Mac, S, LEN, N>
where
    Node: MHNode<S, LEN>,
    Mac: MacPolicy<Node, S, LEN>,
{
    pub async fn run(mut self) -> ! {
        info!("In run!");
        loop {
            // Move user messages into the router while it has room
            while self.router.tx_free() > 0
                && let Ok(msg) = self.out.try_receive()
            {
                info!("Queing payload!");
                if self.router.queue_payload(msg.payload, msg.dest).is_err() {
                    let _ = self.events.try_send(MeshEvent::Dropped);
                }
            }

            info!("Ticking now...");
            match self.router.tick(&mut self.rx_buf).await {
                Ok(to_me) => {
                    for pkt in to_me {
                        info!("pkt to me! {}", pkt.packet_id);
                        let msg = Incoming {
                            source: pkt.source_id,
                            payload: pkt.payload,
                        };
                        if self.inc.try_send(msg).is_err() {
                            let _ = self.events.try_send(MeshEvent::InboxFull);
                        }
                    }
                }
                Err(_) => {
                    let _ = self.events.try_send(MeshEvent::RouterError);
                }
            }
        }
    }
}

/// initialize Mailbox for sending and receiving packets, and the MeshService which should be put in
/// a seperate task from the main loop.
pub fn init<Node, Mac, const S: usize, const LEN: usize, const N: usize>(
    state: &'static MeshState<S, N>,
    router: MeshRouter<Node, Mac, S, LEN>,
    rx_buf: Node::ReceiveBuffer,
) -> (Mailbox<S, N>, MeshService<Node, Mac, S, LEN, N>)
where
    Node: MHNode<S, LEN>,
    Mac: MacPolicy<Node, S, LEN>,
{
    let state: &'static MeshState<S, N> = state;
    let mesh = Mailbox {
        tx: state.out.sender(),
        rx: state.inc.receiver(),
        events: state.events.receiver(),
    };
    let service = MeshService {
        router,
        rx_buf,
        out: state.out.receiver(),
        inc: state.inc.sender(),
        events: state.events.sender(),
    };
    (mesh, service)
}

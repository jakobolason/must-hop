/// This file describes the channels used for the API
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex as Raw,
    channel::{Channel, Receiver, Sender},
};
use heapless::Vec;

// #[cfg(not(feature = "in_std"))]
// use embassy_time::Instant;
// #[cfg(feature = "in_std")]
// use tokio::time::Instant;

use crate::{MHNode, MeshRouter, policy::MacPolicy};

pub struct Outgoing<const SIZE: usize> {
    pub dest: u16,
    pub payload: Vec<u8, SIZE>,
}

pub struct Incoming<const SIZE: usize> {
    pub source: u16,
    pub payload: Vec<u8, SIZE>,
}

/// User has to initialize this struct, and hand it to init
pub struct MeshState<const SIZE: usize, const N: usize> {
    out: Channel<Raw, Outgoing<SIZE>, N>,
    inc: Channel<Raw, Incoming<SIZE>, N>,
    events: Channel<Raw, MeshEvent, 4>,
}

impl<const SIZE: usize, const N: usize> MeshState<SIZE, N> {
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

pub struct MeshService<Node, Mac, const SIZE: usize, const LEN: usize, const N: usize>
where
    Node: MHNode<SIZE, LEN>,
    Mac: MacPolicy<Node, SIZE, LEN>,
{
    router: MeshRouter<Node, Mac, SIZE, LEN>,
    rx_buf: Node::ReceiveBuffer,
    out: Receiver<'static, Raw, Outgoing<SIZE>, N>,
    inc: Sender<'static, Raw, Incoming<SIZE>, N>,
    events: Sender<'static, Raw, MeshEvent, 4>,
}

#[derive(Clone, Copy)]
pub struct Mailbox<const SIZE: usize, const N: usize> {
    tx: Sender<'static, Raw, Outgoing<SIZE>, N>,
    rx: Receiver<'static, Raw, Incoming<SIZE>, N>,
    events: Receiver<'static, Raw, MeshEvent, 4>,
}

impl<const SIZE: usize, const N: usize> Mailbox<SIZE, N> {
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

    pub async fn recv(&self) -> Incoming<SIZE> {
        self.rx.receive().await
    }

    pub fn try_recv(&self) -> Option<Incoming<SIZE>> {
        self.rx.try_receive().ok()
    }

    pub fn pending(&self) -> usize {
        self.rx.len()
    }

    pub fn drain(&self) -> impl Iterator<Item = Incoming<SIZE>> + '_ {
        core::iter::from_fn(move || self.try_recv())
    }

    pub async fn next_event(&self) -> MeshEvent {
        self.events.receive().await
    }

    pub fn try_event(&self) -> Option<MeshEvent> {
        self.events.try_receive().ok()
    }
}

impl<Node, Mac, const SIZE: usize, const LEN: usize, const N: usize>
    MeshService<Node, Mac, SIZE, LEN, N>
where
    Node: MHNode<SIZE, LEN>,
    Mac: MacPolicy<Node, SIZE, LEN>,
{
    pub async fn run(mut self) -> ! {
        loop {
            // Move user messages into the router while it has room
            while self.router.tx_free() > 0
                && let Ok(msg) = self.out.try_receive()
            {
                if self.router.queue_payload(msg.payload, msg.dest).is_err() {
                    let _ = self.events.try_send(MeshEvent::Dropped);
                }
            }

            match self.router.tick(&mut self.rx_buf).await {
                Ok(to_me) => {
                    for pkt in to_me {
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
pub fn init<Node, Mac, const SIZE: usize, const LEN: usize, const N: usize>(
    state: &'static MeshState<SIZE, N>,
    router: MeshRouter<Node, Mac, SIZE, LEN>,
    rx_buf: Node::ReceiveBuffer,
) -> (Mailbox<SIZE, N>, MeshService<Node, Mac, SIZE, LEN, N>)
where
    Node: MHNode<SIZE, LEN>,
    Mac: MacPolicy<Node, SIZE, LEN>,
{
    let state: &'static MeshState<SIZE, N> = state;
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

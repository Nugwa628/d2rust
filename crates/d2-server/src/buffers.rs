//! Server → client path: per-client buffers of up to 0x200 bytes, local
//! delivery that splits them back into messages, and the client's receive
//! lists.
//!
//! [`ClientBuffers`] is the [`MessageSink`] the sim queues into; the host
//! flushes it into [`Inbox`], which `d2-client` reads. [`crate::packets`]
//! reads the tap kept here.

use std::collections::{BTreeMap, VecDeque};

use d2_proto::envelope::{self, Wire};
use d2_proto::schema::Size;

use crate::seams::{ClientId, MessageSink, MessageSizes};
use crate::transport::MAX_MESSAGE;

/// Data bytes per client buffer.
pub const BUFFER_SIZE: usize = 0x200;
/// Bytes queued for every S→C 0x3E.
const PAD_3E: usize = 0x22;

/// The bytes queued for `msg`. The 0x3E message is always queued as 0x22
/// bytes, the unused tail of its stream zero (a tome's quantity shows it);
/// every other message is queued as built.
pub fn queued_form(msg: &[u8]) -> Vec<u8> {
    let mut v = msg.to_vec();
    if msg.first() == Some(&0x3E) && msg.len() < PAD_3E {
        v.resize(PAD_3E, 0);
    }
    v
}

/// First S→C id of the system receive list.
pub const FIRST_SYSTEM_ID: u8 = 0xAF;

/// Number of S→C ids the generated table defines (0x00..=0xB6).
pub fn server_ids() -> u8 {
    d2_proto::SERVER_MESSAGES.len() as u8
}

/// A queue or delivery that cannot be done: the message is oversized,
/// malformed, or the sink cannot take it.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum QueueError {
    /// The message is larger than one client buffer.
    #[error("message of {0} bytes does not fit a 0x200-byte client buffer")]
    TooLarge(usize),
    /// The first byte is not a table S→C id (or is the reserved envelope
    /// id on D2RUST's own wire).
    #[error("S->C id {0:#04x} is not a table id in local delivery")]
    BadId(u8),
    /// The message is empty or longer than the transport maximum.
    #[error("S->C message of {0} bytes (local delivery asserts 1..=0x204)")]
    BadSize(usize),
    /// A direct send or a client flush on a sink without receive lists.
    #[error("S->C id {0:#04x}: this sink has no direct send or flush")]
    NoDirect(u8),
}

/// One server → client message or buffer the packet tap saw, in order
/// (read by [`crate::packets`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tapped {
    /// A message queued for a known client.
    Queued(ClientId, Vec<u8>),
    /// A direct send, straight to the client's receive list.
    Direct(ClientId, Vec<u8>),
    /// A buffer popped by a flush and handed to local delivery.
    Flushed(ClientId, Vec<u8>),
}

/// The per-client buffer lists. Clients are known
/// once [`ClientBuffers::add_client`] ran; messages for any other id are
/// ignored.
#[derive(Debug, Default)]
pub struct ClientBuffers {
    clients: BTreeMap<ClientId, VecDeque<Vec<u8>>>,
    /// The packet tap: `Some` while a recorder listens; a copy of every
    /// queued message, direct send and popped buffer. Never read by the
    /// game.
    tap: Option<Vec<Tapped>>,
}

impl ClientBuffers {
    /// An empty set with no clients and the tap off.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates the client's (empty) buffer list.
    pub fn add_client(&mut self, client: ClientId) {
        self.clients.entry(client).or_default();
    }

    /// Drops the client and anything still buffered for it.
    pub fn remove_client(&mut self, client: ClientId) {
        self.clients.remove(&client);
    }

    /// The client's buffers, head first (sizes and bytes).
    pub fn buffers(&self, client: ClientId) -> Option<&VecDeque<Vec<u8>>> {
        self.clients.get(&client)
    }

    /// Pops the client's head buffer. Every pop is a
    /// flush's send (the tap records it as [`Tapped::Flushed`]).
    pub fn pop(&mut self, client: ClientId) -> Option<Vec<u8>> {
        let buf = self.clients.get_mut(&client)?.pop_front()?;
        self.note(|| Tapped::Flushed(client, buf.clone()));
        Some(buf)
    }

    /// Turns the packet tap on (an empty log) or off (dropped).
    pub fn set_tap(&mut self, on: bool) {
        self.tap = on.then(Vec::new);
    }

    /// What the tap saw since the last take, in order (empty when off).
    pub fn take_tap(&mut self) -> Vec<Tapped> {
        self.tap.as_mut().map(std::mem::take).unwrap_or_default()
    }

    /// Appends to the tap when it is on (the closure runs only then).
    pub(crate) fn note(&mut self, t: impl FnOnce() -> Tapped) {
        if let Some(tap) = self.tap.as_mut() {
            tap.push(t());
        }
    }
}

impl MessageSink for ClientBuffers {
    /// Appends to the tail buffer, or to a new one when there is no tail or
    /// tail + size > 0x200. Messages are never split across buffers.
    fn queue(&mut self, client: ClientId, msg: &[u8]) -> Result<(), QueueError> {
        let padded = queued_form(msg);
        let msg = &padded[..];
        if self.tap.is_some() && self.clients.contains_key(&client) {
            self.note(|| Tapped::Queued(client, msg.to_vec()));
        }
        let Some(list) = self.clients.get_mut(&client) else {
            return Ok(());
        };
        if msg.len() > BUFFER_SIZE {
            return Err(QueueError::TooLarge(msg.len()));
        }
        match list.back_mut() {
            Some(tail) if tail.len() + msg.len() <= BUFFER_SIZE => tail.extend_from_slice(msg),
            _ => {
                let mut buf = Vec::with_capacity(BUFFER_SIZE);
                buf.extend_from_slice(msg);
                list.push_back(buf);
            }
        }
        Ok(())
    }

    fn has_queued(&self, client: ClientId) -> bool {
        self.clients.get(&client).is_some_and(|l| !l.is_empty())
    }
}

/// The client's two receive lists: one message per node, returned at once
/// by local delivery.
#[derive(Debug, Default)]
pub struct Inbox {
    /// Ids 0xAF..=0xB6 (all but the reserved envelope id 0xB5).
    pub system: VecDeque<Vec<u8>>,
    /// Ids < 0xAF, and mod messages (S→C 0xB5) on a mod game's wire.
    pub game: VecDeque<Vec<u8>>,
}

impl Inbox {
    /// Appends one whole message to its list (also the direct-send path).
    pub fn push(&mut self, msg: &[u8]) -> Result<(), QueueError> {
        self.push_on(Wire::Original, msg)
    }

    /// [`Inbox::push`] on `wire`: on [`Wire::Mod`] a mod message (the S→C
    /// envelope) goes to the game list.
    pub fn push_on(&mut self, wire: Wire, msg: &[u8]) -> Result<(), QueueError> {
        if msg.is_empty() || msg.len() > MAX_MESSAGE {
            return Err(QueueError::BadSize(msg.len()));
        }
        match msg[0] {
            envelope::SERVER_ID if wire.envelopes() => self.game.push_back(msg.to_vec()),
            // The envelope id is not a game message on D2RUST's own wire.
            envelope::SERVER_ID => return Err(QueueError::BadId(envelope::SERVER_ID)),
            id if id >= server_ids() => return Err(QueueError::BadId(id)),
            id if id >= FIRST_SYSTEM_ID => self.system.push_back(msg.to_vec()),
            _ => self.game.push_back(msg.to_vec()),
        }
        Ok(())
    }

    /// Local delivery: split `buffer`
    /// by the S→C size rule; a size-rule failure ends the split and the
    /// rest of the buffer is lost. Returns the bytes discarded.
    pub fn deliver(
        &mut self,
        sizes: &impl MessageSizes,
        buffer: &[u8],
    ) -> Result<usize, QueueError> {
        self.deliver_on(Wire::Original, sizes, buffer)
    }

    /// [`Inbox::deliver`] on `wire`: on [`Wire::Mod`] a mod message is
    /// sized by its length field.
    pub fn deliver_on(
        &mut self,
        wire: Wire,
        sizes: &impl MessageSizes,
        buffer: &[u8],
    ) -> Result<usize, QueueError> {
        let mut rest = buffer;
        while !rest.is_empty() {
            let size = match rest[0] {
                envelope::SERVER_ID if wire.envelopes() => match envelope::size(rest) {
                    Size::Bytes(n) => Ok(n),
                    _ => Ok(0),
                },
                _ => sizes.server_size(rest),
            };
            let n = match size {
                Ok(n) if n != 0 && n <= rest.len() => n,
                // Size 0, or a rule larger than the bytes left: the split
                // ends (a whole-buffer copy never runs past the buffer).
                _ => break,
            };
            self.push_on(wire, &rest[..n])?;
            rest = &rest[n..];
        }
        Ok(rest.len())
    }

    /// Client receive: the system list
    /// until empty, then the game list.
    pub fn receive(&mut self) -> Vec<Vec<u8>> {
        self.system.drain(..).chain(self.game.drain(..)).collect()
    }
}

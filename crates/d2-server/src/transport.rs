//! Client → server local transport: the client's duplicate filter, the
//! classifier, the three server queues and the drain.
//!
//! [`crate::host::Host`] owns a [`ServerQueues`] and one
//! [`DuplicateFilter`] per client; `d2-client` sends through the host and
//! the host drains the queues once per frame.

use std::collections::VecDeque;

use d2_proto::envelope::{self, Wire};
use d2_proto::schema::Size;

use crate::seams::{ClientId, MessageSizes, SizeError};

/// Largest message the transport accepts.
pub const MAX_MESSAGE: usize = 0x204;

/// The drain copies at most 0x200 bytes including the 4-byte client id.
pub const DRAIN_COPY: usize = 0x200 - 4;

/// Size of the queue-2 message 0xFF.
pub const ADMIN_SIZE: usize = 16;

/// The client game-message sender requires size < 0x200.
pub const MAX_GAME_SEND: usize = 0x200;

/// Server queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Queue {
    /// 0: system messages 0x67..=0x72.
    System = 0,
    /// 1: game messages < 0x67.
    Game = 1,
    /// 2: realm/admin 0xFF.
    Admin = 2,
}

/// Classifier result. Results 3 and 4 drop the message
/// silently in local mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Classified {
    /// Results 1 and 2: appended to this queue.
    Queued(Queue),
    /// Result 3: incomplete (empty, size rule 0, > 0x204 or > the given
    /// size).
    Incomplete,
    /// Result 4: invalid id (0x71, 0x73..=0xFE, or 0xFF with the gate closed).
    Invalid,
    /// The chat size rule came out negative ([`crate::seams::SizeError::Negative`]):
    /// what the transport should do with it is undecided; not queued.
    NegativeSize(i32),
}

/// A send that breaks a transport size limit.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SendError {
    /// A game message of 0x200 bytes or more.
    #[error("game message of {0} bytes (the sender asserts < 0x200)")]
    GameTooLarge(usize),
    /// A message larger than [`MAX_MESSAGE`].
    #[error("message of {0} bytes (the transport asserts <= 0x204)")]
    TooLarge(usize),
}

/// Classifier: picks the queue for a message, or says why it is dropped.
/// `admin_gate` is the net object's gate for id 0xFF.
pub fn classify(sizes: &impl MessageSizes, msg: &[u8], admin_gate: bool) -> Classified {
    classify_on(Wire::Original, sizes, msg, admin_gate)
}

/// [`classify`] on `wire`. On [`Wire::Mod`] a mod message (the C→S
/// envelope, `d2_proto::envelope`) whose length field fits `msg` goes to
/// the game queue, in order with the client's game messages; on
/// [`Wire::Original`] its id is invalid like the rest of 0x71..=0xFE.
pub fn classify_on(
    wire: Wire,
    sizes: &impl MessageSizes,
    msg: &[u8],
    admin_gate: bool,
) -> Classified {
    if wire.envelopes() && msg.first() == Some(&envelope::CLIENT_ID) {
        return match envelope::size(msg) {
            Size::Bytes(n) if n <= msg.len() => Classified::Queued(Queue::Game),
            _ => Classified::Incomplete,
        };
    }
    let Some(&id) = msg.first() else {
        return Classified::Incomplete;
    };
    if (0x73..=0xFE).contains(&id) || id == envelope::CLIENT_ID {
        return Classified::Invalid;
    }
    let rule = if id == 0xFF {
        Ok(ADMIN_SIZE)
    } else {
        sizes.client_size(msg)
    };
    match rule {
        Ok(n) if n != 0 && n <= MAX_MESSAGE && n <= msg.len() => {}
        Err(SizeError::Negative(n)) => return Classified::NegativeSize(n),
        _ => return Classified::Incomplete,
    }
    match id {
        0x00..=0x66 => Classified::Queued(Queue::Game),
        0x67..=0x72 => Classified::Queued(Queue::System),
        _ if admin_gate => Classified::Queued(Queue::Admin),
        _ => Classified::Invalid,
    }
}

/// One drained message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drained {
    /// The queue it came from.
    pub queue: Queue,
    /// The sending client.
    pub client: ClientId,
    /// The drain copy: the first `min(size, 0x1FC)` bytes.
    pub msg: Vec<u8>,
    /// The full queued size, which handlers see.
    pub size: usize,
}

/// The three server queues: FIFO, the whole given
/// buffer is queued (not the rule size).
#[derive(Debug)]
pub struct ServerQueues {
    queues: [VecDeque<(ClientId, Vec<u8>)>; 3],
    /// Gate for queue 2.
    pub admin_gate: bool,
    /// The game's wire: [`Wire::Mod`] admits mod messages.
    pub wire: Wire,
}

impl Default for ServerQueues {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerQueues {
    /// Empty queues; the admin gate passes (0xFF with 16 bytes goes to
    /// queue 2).
    pub fn new() -> Self {
        Self {
            queues: Default::default(),
            admin_gate: true,
            wire: Wire::Original,
        }
    }

    /// Net send in local mode: rejects size > 0x204, then classifies and
    /// enqueues.
    pub fn send(
        &mut self,
        sizes: &impl MessageSizes,
        client: ClientId,
        msg: &[u8],
    ) -> Result<Classified, SendError> {
        if msg.len() > MAX_MESSAGE {
            return Err(SendError::TooLarge(msg.len()));
        }
        let class = classify_on(self.wire, sizes, msg, self.admin_gate);
        if let Classified::Queued(q) = class {
            self.queues[q as usize].push_back((client, msg.to_vec()));
        }
        Ok(class)
    }

    /// Messages waiting in `queue`.
    pub fn len(&self, queue: Queue) -> usize {
        self.queues[queue as usize].len()
    }

    /// True when all three queues are empty.
    pub fn is_empty(&self) -> bool {
        self.queues.iter().all(VecDeque::is_empty)
    }

    /// Drain: queue 0 until empty, then
    /// queue 1, then queue 2; each message truncated to the drain copy.
    pub fn drain(&mut self) -> Vec<Drained> {
        let mut out = Vec::new();
        for queue in [Queue::System, Queue::Game, Queue::Admin] {
            for (client, mut msg) in self.queues[queue as usize].drain(..) {
                let size = msg.len();
                msg.truncate(DRAIN_COPY);
                out.push(Drained {
                    queue,
                    client,
                    msg,
                    size,
                });
            }
        }
        out
    }
}

/// Duplicate-filter window for a game message id;
/// `None` = never filtered.
pub fn duplicate_window(id: u8) -> Option<u32> {
    match id {
        0x05..=0x0A | 0x0C..=0x11 => Some(50),
        0x3A => None,
        _ => Some(200),
    }
}

/// The client game-message sender: drops a repeat of the last sent
/// message within its window. The comparison runs over the new message's
/// size against a 0x200-byte store that each send overwrites only for its
/// own length (the bytes past a shorter message stay from earlier ones);
/// store and time start zeroed.
#[derive(Debug)]
pub struct DuplicateFilter {
    stored: [u8; MAX_GAME_SEND],
    at: u32,
}

impl Default for DuplicateFilter {
    fn default() -> Self {
        Self {
            stored: [0; MAX_GAME_SEND],
            at: 0,
        }
    }
}

impl DuplicateFilter {
    /// True when `msg` sent at `now_ms` passes; a sent message replaces
    /// the stored copy and time, a filtered one leaves both as they were.
    pub fn pass(&mut self, msg: &[u8], now_ms: u32) -> Result<bool, SendError> {
        if msg.len() >= MAX_GAME_SEND {
            return Err(SendError::GameTooLarge(msg.len()));
        }
        if let Some(window) = msg.first().and_then(|&id| duplicate_window(id)) {
            if self.stored[..msg.len()] == *msg && now_ms.wrapping_sub(self.at) < window {
                return Ok(false);
            }
        }
        self.stored[..msg.len()].copy_from_slice(msg);
        self.at = now_ms;
        Ok(true)
    }
}

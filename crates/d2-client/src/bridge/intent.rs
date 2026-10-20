//! Send path: C→S bytes built with `d2-proto`, checked with the
//! transport's classifier before they reach the link, so the bridge never
//! sends what the server's queues would refuse.
//!
//! On a mod game's wire ([`Wire::Mod`]) the C→S mod envelope is sendable
//! too (`docs/plans/mods/seams.md` MS13); on D2RUST's own wire its id is
//! refused like any unknown id.

use d2_proto::envelope::{self, Direction, Wire};
use d2_proto::transport::{classify_client_on, Classified, ClientQueue, MAX_MESSAGE};
use d2_proto::FixedMessage;

use super::link::SendQueue;

/// The client's game-message sender takes messages under 0x200 bytes.
pub const MAX_GAME_SEND: usize = 0x200;

/// A message the bridge refuses to send.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IntentError {
    #[error("message refused by the classifier: {0:?}")]
    NotSendable(Classified),
    #[error("id 0xFF (admin queue) is out of scope")]
    AdminQueue,
    #[error("game message of {0} bytes (the sender asserts < 0x200)")]
    GameTooLarge(usize),
    /// A system message the classifier queues (it accepts a buffer longer
    /// than the size rule) but the transport refuses (over 0x204 bytes).
    #[error("message of {0} bytes (the transport asserts <= 0x204)")]
    TooLarge(usize),
    /// A mod message payload larger than one envelope carries.
    #[error(transparent)]
    ModTooLarge(#[from] envelope::TooLarge),
}

/// Encodes a typed C→S message (the TSV layout; unlisted bytes 0).
pub fn encode<M: FixedMessage>(msg: &M) -> Vec<u8> {
    let mut b = vec![0; M::SIZE];
    msg.write(&mut b);
    b
}

/// The send queue of `msg` on D2RUST's own wire, or why it may not be sent.
pub fn route(msg: &[u8]) -> Result<SendQueue, IntentError> {
    route_on(Wire::Original, msg)
}

/// The send queue of `msg` on `wire`, or why it may not be sent: the
/// classifier's queue, within the game sender's and the transport's size
/// limits.
pub fn route_on(wire: Wire, msg: &[u8]) -> Result<SendQueue, IntentError> {
    match classify_client_on(wire, msg) {
        Classified::Queue(q) => {
            let queue = SendQueue::of(q).ok_or(IntentError::AdminQueue)?;
            if q == ClientQueue::Game && msg.len() >= MAX_GAME_SEND {
                return Err(IntentError::GameTooLarge(msg.len()));
            }
            if msg.len() > MAX_MESSAGE {
                return Err(IntentError::TooLarge(msg.len()));
            }
            Ok(queue)
        }
        other => Err(IntentError::NotSendable(other)),
    }
}

/// The C→S envelope of a mod message `payload`.
pub fn mod_message(payload: &[u8]) -> Result<Vec<u8>, IntentError> {
    Ok(envelope::encode(Direction::ToServer, payload)?)
}

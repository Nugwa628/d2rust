//! How `d2-proto` messages travel between a client and the server.
//!
//! D2RUST always plays against an authoritative server. In single player
//! that server runs in the same process; online it runs elsewhere. Both
//! cases move the same messages, so both sit behind one seam: a
//! [`Transport`] is one end of one client's connection. The client end
//! sends its intents (C→S messages) and reads what the server delivers
//! (S→C messages and frame marks); the server end, held by the server's
//! hub (`d2_server::hub`), does the opposite.
//!
//! - [`memory`]: the in-memory channel, for single player and in-process
//!   tests.
//! - `d2-quic` (its own crate; the game exe links it only when built with
//!   its `online` feature): every online connection. Encrypted, with
//!   keepalive, timeouts and connection migration.
//!
//! The network transport speaks the [`wire`] format defined here: frames, a
//! versioned handshake (protocol version, game build, tick rate, resume
//! token) and goodbyes.
//!
//! A transport only carries whole packets in order. It never reads,
//! changes or reorders the game messages inside them, and it decides no
//! game outcome.

pub mod memory;
pub mod msg;
pub mod wire;

pub use memory::{pair, MemoryEnd};
pub use wire::{ConnectError, Hello, Refusal, ResumeToken, Timing, Welcome};

/// A client's id on its server (the server's connection index; the
/// single-player client is 0).
pub type ClientId = u32;

/// Which sender a C→S message goes through on the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lane {
    /// Game messages (ids below 0x67): the sender with the duplicate
    /// filter, then the game queue.
    Game,
    /// System messages (ids 0x67..=0x70): no filter, the system queue.
    System,
}

/// One unit a transport carries, in either direction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Packet {
    /// C→S: one client message for the server's `lane` sender.
    Send {
        /// The sender it goes through.
        lane: Lane,
        /// The message bytes as the client built them.
        msg: Vec<u8>,
    },
    /// S→C: one whole server message, as the server's delivery split it
    /// out of the client's buffers (system messages of a delivery first).
    Deliver(Vec<u8>),
    /// S→C: the server finished one frame; `ticked` says whether a game
    /// tick ran in it (the client's update pass runs only after a tick).
    Frame {
        /// A tick ran in this frame.
        ticked: bool,
    },
    /// Either way: one message on an extension channel. A mod game adds
    /// its own messages here instead of changing the protocol: each
    /// channel belongs to one extension of the game (see
    /// `d2-server`'s `extension` module), and the bytes are that
    /// extension's own format. D2RUST's own game uses no channel.
    Ext {
        /// The channel: which extension the message is for.
        channel: u16,
        /// The message bytes, in the extension's format.
        msg: Vec<u8>,
    },
}

/// A transport failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NetError {
    /// The other end is gone (dropped, disconnected or timed out).
    #[error("the other end of the connection is closed")]
    Closed,
    /// Nothing was heard from the other end for the timeout; the
    /// connection is closed.
    #[error("the connection timed out")]
    TimedOut,
    /// The other end broke the wire format; the connection is closed.
    #[error("malformed data on the connection")]
    Malformed,
}

/// One end of one client's connection to the server.
///
/// Packets arrive in the order they were sent; nothing is lost while the
/// connection is open. `recv` never blocks: it returns the next packet
/// that has arrived, or `None`.
pub trait Transport {
    /// Sends one packet to the other end.
    fn send(&mut self, packet: Packet) -> Result<(), NetError>;

    /// The next packet that has arrived, or `None` when none is waiting.
    /// [`NetError::Closed`] once the other end is gone and everything it
    /// sent has been read.
    fn recv(&mut self) -> Result<Option<Packet>, NetError>;

    /// Ends one server frame: the hub calls it for every client after the
    /// frame's deliveries. A network transport sends the frame's packets
    /// together and marks its end; the in-memory transport has nothing to
    /// do.
    fn flush(&mut self) -> Result<(), NetError> {
        Ok(())
    }
}

impl<T: Transport + ?Sized> Transport for Box<T> {
    fn send(&mut self, packet: Packet) -> Result<(), NetError> {
        (**self).send(packet)
    }

    fn recv(&mut self) -> Result<Option<Packet>, NetError> {
        (**self).recv()
    }

    fn flush(&mut self) -> Result<(), NetError> {
        (**self).flush()
    }
}

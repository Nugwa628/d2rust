//! Client<->server message types, shared by `d2-server` and `d2-client`.
//!
//! This crate is the wire protocol of D2RUST: every byte that crosses between
//! the client and the server is described here, and nothing else in the
//! workspace knows message ids or layouts.
//!
//! The message ids, names, size rules and field layouts of both directions
//! are generated from the machine tables
//! (`tables/sim/client-messages.tsv`, `server-messages.tsv`) into
//! [`generated`] by `data-tool gen-proto`; never edit that file.
//!
//! - [`schema`]: descriptor types and the size-rule evaluation.
//! - [`transport`]: size lookup per direction, the C→S classifier and the
//!   S→C buffer split.
//! - [`generated::client`] / [`generated::server`]: typed decode/encode of
//!   the fixed layouts ([`wire::FixedMessage`]).
//! - [`s2c`]: typed S→C builders, the client-side parser and the audit
//!   of every S→C id (which layouts are known).
//! - [`item_bits`]: reader of the item bit stream (S→C 0x9C / 0x9D and the
//!   `.d2s` item entries).
//! - [`envelope`]: the mod message envelope (C→S 0x71, S→C 0xB5) a mod
//!   game's wire adds; D2RUST's own wire refuses both ids.
//! - [`dev`]: D2RUST's dev command channel, console to local server (not
//!   part of the game protocol).
//! - [`tsv`]: strict TSV parser and the TSV-vs-code check; [`codegen`]: the
//!   generator.

pub mod codegen;
pub mod dev;
pub mod envelope;
pub mod item_bits;
// Generated layout (one row per line); `cargo fmt` leaves it alone.
#[rustfmt::skip]
pub mod generated;
pub mod s2c;
pub mod schema;
pub mod transport;
pub mod tsv;
pub mod wire;

#[cfg(test)]
mod mutant_tests;

pub use generated::{client, server, CLIENT_MESSAGES, SERVER_MESSAGES};
pub use wire::{DecodeError, FixedMessage};

/// Protocol version. Covers the message tables generated from the TSVs and
/// the typed layouts. Bump on any incompatible message change; client and
/// server refuse to talk across versions.
pub const PROTOCOL_VERSION: u32 = 2;

/// The rate time on the wire is stated at, whatever rate the server's
/// game runs at: every tick count, duration and per-tick amount a message
/// carries (the day clock of S→C 0x53, the stats of state and unit
/// messages, the potion predictions of 0x18) counts base ticks of 1/25 s,
/// the rate the game's tables are stated at. A server at a faster tick
/// rate converts to it, so the messages don't change with the rate.
pub const WIRE_TICKS_PER_SECOND: u32 = 25;

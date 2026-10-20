//! The seams of [`crate::seams`] on the real crates: [`ProtoSizes`] on
//! `d2-proto`'s size lookup, [`SimGame`] on `d2_sim::game::Game` and
//! `d2_sim::tick`. `d2-sim` may not depend on `d2-server` (depcheck), so the
//! adapters live here.
//!
//! Also here: the single-player session sequence ([`session`],
//! [`session_flow`]), character saving ([`storage`]), the intent handlers
//! ([`handlers`]) and the player-character loader ([`character`]). The app
//! and the server host build their game from these.

pub mod character;
pub mod handlers;
pub mod item_bits;
pub mod session;
pub mod session_flow;
mod sim;
mod sizes;
pub mod storage;

pub use sim::{
    AdapterError, HostSync, PlayerData, PlayerFields, SimGame, UnitFacts, Unspecified, WorldSync,
};
pub use sizes::ProtoSizes;

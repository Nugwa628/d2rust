//! Debug read-outs of a game: nothing here decides behaviour or writes to
//! the game (the sim stays deterministic: no I/O, no RNG step, no list
//! change).
//!
//! - [`state`]: the `state-1` game-state snapshot (one record per server
//!   tick) that `d2-client state-dump` writes.
//! - `rng_trace` (`rng-trace` feature, off by default): the log of every
//!   seeded draw.
//! - [`coverage`]: per-game behaviour counters.

pub mod coverage;
#[cfg(feature = "rng-trace")]
pub mod rng_trace;
pub mod state;

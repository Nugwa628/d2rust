//! Deterministic D2RUST game logic at a fixed rate (60 Hz for base play).
//!
//! Input: the previous state plus this tick's player intents.
//! Output: the new state plus events. The crate holds the seeded RNG ([`rng`]), unit lists
//! ([`units`]), the tick core ([`tick`], [`game`]), combat ([`combat`]), skills ([`skills`])
//! and the other game systems listed below.
//!
//! Determinism rules: no I/O, no wall-clock time, no global or thread RNG, no unordered
//! iteration affecting outcomes, no floating point. Some of these are enforced by
//! `clippy.toml` and `[lints]` in this crate.

/// Combat: hit chance, block and avoid, damage, resistances, leech and death triggers.
pub mod combat;
/// Read-only debug views of a game, such as state snapshots and the RNG draw log.
pub mod debug;
/// Developer switches (god mode, no cooldown, free mana, speed bonus).
pub mod dev;
/// Dungeon level generation (DRLG): levels, rooms and their collision.
pub mod drlg;
/// One game's state: frame counter, unit lists, timers and the operations across them.
pub mod game;
/// Items: creation, quality rolls, affixes and properties.
pub mod items;
/// Server missiles: creation, per-tick flight, hits and lifetime.
pub mod missiles;
/// Monsters: population, init and the AI think step.
pub mod monsters;
/// Unit position, collision, placement and walk / run movement.
pub mod path;
/// Player state, such as the pet lists.
pub mod player;
/// Pokes: directives that set game state directly, for debug and test checks.
pub mod poke;
/// The seeded random number generator.
pub mod rng;
pub mod rules;
pub mod skills;
/// Unit stats: values, lists, operations and derived stats.
pub mod stats;
/// The tick core: frame counter, step order, timer queue and room and client passes.
pub mod tick;
/// Treasure classes and drops from monsters and chests.
pub mod treasure;
/// Units: identity, lists and lifecycle.
pub mod units;
/// Wiring: connects the sim modules to their real providers; rules stay in the modules.
pub mod wiring;
/// World systems: quests, waypoints and the Horadric Cube.
pub mod world;

/// The 25 Hz unit used by table speeds and durations. The base game runs
/// at [`tick::TickRate::GAME`]; a game's actual rate is
/// [`tick::TickRate`].
pub const TICKS_PER_SECOND: u32 = 25;

/// The synthetic fixtures of the unit tests, for `benches/` (criterion).
/// Only with the `bench-fixtures` feature; no game logic lives here.
#[cfg(feature = "bench-fixtures")]
pub mod bench_fixtures {
    pub use crate::items::tests::{
        item, item_rec, push_item, tables as item_tables, FakeGame, FakeStats,
    };
    /// The synthetic stat tables the unit tests use.
    pub fn stat_data() -> std::sync::Arc<crate::stats::StatData> {
        crate::stats::tests::data()
    }
    pub use crate::wiring::worldgen::tests::{ds1, Ds1s, Fx, ISLE, ISLE_DEF};
    /// The combat / missile fixtures of the e2e combat path and the
    /// "fight" tick fixture (`wiring::action::tests::fight`).
    pub mod combat {
        pub use crate::wiring::action::tests::fight::*;
        pub use crate::wiring::action::tests::Fx as ActionFx;
    }
}

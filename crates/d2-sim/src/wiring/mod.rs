//! Wiring: the seam traits of the Phase 3 modules implemented on their
//! real providers. Each submodule wires one group of seams; the modules'
//! own code stays as written and keeps its fake-based tests. Nothing here
//! decides behaviour: rules stay in the modules; an adapter maps a seam
//! call to a provider call.
//!
//! - [`economy`]: items ↔ stats / stat lists, treasure → item creation,
//!   cube and quests → items and unit fields.
//! - [`action`]: combat, missiles, monster AI, DRLG and waypoints ↔ units,
//!   stats and rooms; the combined timer-event dispatcher the tick runs.
//! - [`worldgen`]: DRLG level types (maze, outdoor, preset), population
//!   and monster init ↔ DRLG rooms, units and stats.
//! - [`interaction`]: NPCs ↔ vendors, quests, items; skill use ↔ missiles
//!   and combat; vitals ↔ stats and the tick.
//! - [`inventory`]: the inventory model and item-move intents ↔ units,
//!   unit lists, stats and the item store.
//! - [`path`]: positions, collision, placement, warps and walk / run
//!   movement ↔ units, rooms and the tick.
//!
//! How the pieces meet: [`action::ActionHooks`] holds the state every
//! action adapter shares (DRLG, tables, item store, combat lists, event
//! records). Some of it is lent out for one call and is `None` / empty
//! meanwhile: the missile store during a missile call, the AI store during
//! a think, the item store to an economy. [`worldgen::dispatch::WorldSim`]
//! in turn lends its world state to the action hooks around timer events
//! and tick hooks. Calls a group cannot answer from d2-sim's own state go
//! to its pending trait, whose defaults do nothing; a host fills them in.
//!
//! Watch out for: code that runs while a piece is lent out sees the
//! empty stand-in, so a nested call must go through the lent copy, not the
//! owner's field. Message and seed order are part of the result; moving a
//! call can change replays even when the end state looks the same.

pub mod action;
pub mod economy;
pub mod interaction;
pub mod inventory;
pub mod path;
pub mod worldgen;

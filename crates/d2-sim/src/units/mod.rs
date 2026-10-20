//! Server units. Phase 3 so far: identity and list bookkeeping
//! ([`lists`]); unit data, stats and behaviour follow with their specs.

pub mod anim;
pub mod anim_rate;
pub mod dispatch;
pub mod event_records;
#[cfg(test)]
mod gap_tests;
pub mod hooks;
pub mod inactive;
pub mod lifecycle;
pub mod lists;
pub mod messages;
pub mod mode_set;
pub mod modes;
#[cfg(test)]
mod mutant_tests;
pub mod record;
pub mod replenish;
pub mod sound;
#[cfg(test)]
mod tests;

pub use lists::{
    ActEntry, ClientEntry, ClientId, GuidCounters, ListError, RoomEntry, RoomId, UnitEntry, UnitId,
    UnitLists, UnitType,
};

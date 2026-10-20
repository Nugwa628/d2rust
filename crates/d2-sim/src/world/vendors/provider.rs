//! A mod game's stores (`docs/plans/mods/seams.md` MS8e): which store
//! column an NPC class sells from, and which classes repair, keep their
//! store at a trade open, or carry the record's two flag bytes.
//!
//! The mod answers first and D2RUST's lists apply when it doesn't:
//! [`StoreProvider::roles`] gives a class's store roles; `None` keeps
//! D2RUST's ([`StoreRoles::d2rust`]). A mod's own NPC classes get a store
//! this way, and a mod may change a D2RUST NPC's store.
//!
//! Gambling and hire lists are NPC roles ([`crate::world::npc::NpcRoles`],
//! MS8c): the record gets gamble lists when the class gambles, and a trade
//! open makes the hire list when the class hires. A mod sets those once,
//! in its NPC provider.
//!
//! Store columns are the item tables' vendor columns (`<Vendor>Min` and its
//! neighbours in weapons, armor and misc rows), a fixed record layout of
//! [`COLUMNS`] columns, so there is no slot count to extend: a mod's
//! vendors share those columns, and a column past them is ignored.

use std::fmt::Debug;

use super::{column_of, global_column_of, COLUMNS, FLAGGED, NO_STORE_REFRESH, REPAIRERS};
use crate::rules::Ruleset;

/// What an NPC class's store does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StoreRoles {
    /// The column a trader's record copies its store list from.
    pub column: Option<usize>,
    /// The column built for the class in the global lists at game start.
    /// D2RUST's: as `column`, but Nihlathak builds column 16 while his
    /// record copies Larzuk's 15.
    pub global_column: Option<usize>,
    /// The record's two flag bytes are set (no reader yet).
    pub flagged: bool,
    /// A trade open never refreshes the store.
    pub no_refresh: bool,
    /// It repairs (C→S 0x35).
    pub repairs: bool,
}

impl StoreRoles {
    /// D2RUST's store roles of `class`.
    pub fn d2rust(class: u16) -> Self {
        Self {
            column: column_of(class),
            global_column: global_column_of(class),
            flagged: FLAGGED.contains(&class),
            no_refresh: NO_STORE_REFRESH.contains(&class),
            repairs: REPAIRERS.contains(&class),
        }
    }

    /// The store roles of `class` under `rules`: the mod's, else D2RUST's.
    /// Columns past [`COLUMNS`] are dropped.
    pub fn of(rules: &Ruleset, class: u16) -> Self {
        let mut r = rules
            .stores()
            .and_then(|p| p.roles(class))
            .unwrap_or_else(|| Self::d2rust(class));
        r.column = r.column.filter(|&i| i < COLUMNS);
        r.global_column = r.global_column.filter(|&i| i < COLUMNS);
        r
    }
}

/// A mod game's store roles. The default is "not the mod's", so a
/// provider only writes what its game adds.
pub trait StoreProvider: Send + Sync + Debug {
    /// The store roles of NPC `class`; `None`: D2RUST's.
    fn roles(&self, class: u16) -> Option<StoreRoles> {
        let _ = class;
        None
    }
}

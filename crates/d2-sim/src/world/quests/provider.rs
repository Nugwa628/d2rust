//! A mod game's quest callbacks (`docs/plans/mods/seams.md` MS8d).
//!
//! A quest record has four kinds of function: event callbacks, an active
//! function (does an NPC have something to say), a status function (the
//! quest log state) and a sequence function (the walk to the next quest).
//! Each reaches the record's code through one router in [`super::act1`].
//! Under a mod game the [`QuestProvider`] answers first for the record's
//! quest chain; `None` runs D2RUST's code for it, so a mod replaces single
//! quests (or single events of a quest) and keeps the rest.
//!
//! Which events reach a record is the record's callback bits, read from
//! the quest table rows (a mod game's rows come with its table data). The
//! quest rows and the flag records are the save format, so there is no
//! slot count here: a mod's quests reuse D2RUST's chains.
//!
//! Providers get the quest world as `dyn QuestWorld` and must stay
//! deterministic: no I/O, no clock, no floats in game state, draws only
//! from the game's seeds (the quest seed in [`QuestControl`]).

use std::fmt::Debug;

use super::{EventArgs, QuestControl, QuestFlags, QuestWorld, TextList};
use crate::units::UnitId;

/// A mod game's quest callbacks. Every method defaults to "not the mod's",
/// so a provider only writes what its game adds.
pub trait QuestProvider: Send + Sync + Debug {
    /// Run event `args.event` of record `i` (quest chain
    /// `ctl.records[i].chain`). `list` is the NPC text list of event 0
    /// (NPC activate); `force` is set when the event reaches an inactive
    /// record. `Some(true)`: handled; `Some(false)`: not handled (the
    /// world's `unhandled` report runs); `None`: D2RUST's callback runs.
    fn callback(
        &self,
        ctl: &mut QuestControl,
        w: &mut dyn QuestWorld,
        i: usize,
        args: EventArgs,
        list: Option<&mut TextList>,
        force: bool,
    ) -> Option<bool> {
        let _ = (ctl, w, i, args, list, force);
        None
    }

    /// The active function of record `i`: whether NPC class `npc_class`
    /// has quest talk for `player`. `None`: D2RUST's.
    fn active(
        &self,
        ctl: &QuestControl,
        w: &mut dyn QuestWorld,
        i: usize,
        player: UnitId,
        npc_class: u16,
    ) -> Option<bool> {
        let _ = (ctl, w, i, player, npc_class);
        None
    }

    /// The status function of record `i` for `player` with flag record
    /// `pf`: `Some(Some(s))` reports quest-log state `s`, `Some(None)`
    /// reports nothing, `None` runs D2RUST's.
    fn status(
        &self,
        ctl: &QuestControl,
        w: &mut dyn QuestWorld,
        i: usize,
        player: UnitId,
        pf: &QuestFlags,
    ) -> Option<Option<u8>> {
        let _ = (ctl, w, i, player, pf);
        None
    }

    /// The sequence function of quest chain `chain`: `Some(r)` is the
    /// function's result (true: pass), `None` runs D2RUST's.
    fn sequence(&self, ctl: &mut QuestControl, w: &mut dyn QuestWorld, chain: u8) -> Option<bool> {
        let _ = (ctl, w, chain);
        None
    }
}

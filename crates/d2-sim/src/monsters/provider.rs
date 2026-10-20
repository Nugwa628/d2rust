//! A mod game's monster overrides (`docs/plans/mods/seams.md` MS5): its
//! AI table rows and AI bodies, and its unique-monster mods (umods).
//!
//! The mod answers first and D2RUST's own code runs when it doesn't:
//! - **AI table.** [`MonsterProvider::ai_record`] gives the row for a
//!   monstats `AI` index, including indices past D2RUST's table;
//!   [`MonsterProvider::special_record`] the row for a special AI state.
//!   A row names its functions by id; a mod game picks ids D2RUST doesn't
//!   use for its own bodies, or reuses D2RUST's ids to reuse its bodies.
//! - **AI bodies.** Every AI init and think call goes through
//!   [`crate::monsters::ai::run_init`] / [`crate::monsters::ai::run_function`],
//!   which ask [`MonsterProvider::ai_init`] / [`MonsterProvider::ai_think`]
//!   first, so a mod body replaces D2RUST's for that id wherever it is
//!   reached (install, think, the special states).
//! - **Umods.** [`MonsterProvider::umod_init`] runs when a umod is applied
//!   to a monster (champion and unique packs, superuniques, restores) and
//!   [`MonsterProvider::umod_callback`] for each umod in the monster's
//!   umod list on a umod event (modes 0-5), before D2RUST's own.
//!
//! The bodies get the same world D2RUST's do, as `dyn` hosts, so they
//! can call D2RUST's AI and init helpers. They must stay deterministic:
//! no I/O, no clock, no floats in game state, draws only from the game's
//! seeds.

use std::fmt::Debug;

use crate::game::Game;
use crate::monsters::ai::{self, AiHost, AiRecord, TickParam};
use crate::monsters::init::{self, InitHost};
use crate::units::UnitId;

/// A mod game's monster AI and umods (MS5). Every method's default
/// leaves D2RUST's behaviour.
pub trait MonsterProvider: Send + Sync + Debug {
    /// The AI table row for monstats `AI` index `ai`; `None`: D2RUST's
    /// (row 0 past its table).
    fn ai_record(&self, ai: u16) -> Option<AiRecord> {
        let _ = ai;
        None
    }
    /// The row for special AI state `state` (non-zero) of a monster of
    /// monstats row `class`; `None`: D2RUST's rule for the state.
    fn special_record(&self, state: u32, class: i32) -> Option<AiRecord> {
        let _ = (state, class);
        None
    }
    /// Runs init function `function` on `unit`; `false`: D2RUST's.
    fn ai_init(
        &self,
        game: &mut Game,
        cx: &mut ai::Ctx<'_, dyn AiHost + '_>,
        function: u32,
        unit: UnitId,
    ) -> bool {
        let _ = (game, cx, function, unit);
        false
    }
    /// Runs think function `function` for `unit` with the think's tick
    /// record; `false`: D2RUST's.
    fn ai_think(
        &self,
        game: &mut Game,
        cx: &mut ai::Ctx<'_, dyn AiHost + '_>,
        function: u32,
        unit: UnitId,
        p: &TickParam,
    ) -> bool {
        let _ = (game, cx, function, unit, p);
        false
    }
    /// Applies umod `umod` to `unit` (`unique`: the monster is a unique
    /// or superunique, not a champion or minion); `false`: D2RUST's.
    fn umod_init(
        &self,
        cx: &init::Ctx<'_>,
        h: &mut dyn InitHost,
        unit: UnitId,
        umod: u8,
        unique: bool,
    ) -> bool {
        let _ = (cx, h, unit, umod, unique);
        false
    }
    /// Umod `umod`'s event `mode` (0-5) on `unit` (for mode 5 the missile
    /// the monster fired); `false`: D2RUST's callback for the umod, if any.
    #[allow(clippy::too_many_arguments)]
    fn umod_callback(
        &self,
        cx: &init::Ctx<'_>,
        h: &mut dyn InitHost,
        unit: UnitId,
        umod: u8,
        unique: bool,
        mode: u8,
    ) -> bool {
        let _ = (cx, h, unit, umod, unique, mode);
        false
    }
}

//! Monster AI: think scheduling, dispatch and the prechecks, the AI
//! control record and tables, AI parameters (monstats `aip1`-`aip8`),
//! target selection, distances, tactics helpers, commands and the per-AI
//! bodies.
//!
//! Each monster's AI control holds the id of its current think function;
//! a think event runs the prechecks and then that function. The row a
//! monster installs comes from [`AI_TABLE`] by its monstats `AI` column,
//! or from [`SPECIAL_TABLE`] in a special AI state. A mod game's rows and
//! bodies (`docs/plans/mods/seams.md` MS5) are asked first, through the
//! game's rules ([`AiUnits::rules`]).
//!
//! Mode changes, movement, collision, target-node lists, skills and
//! damage belong to other systems; they are reached through [`seams`]. AI
//! timer events (type 2 think, type 10 reset) plug into the tick through
//! [`MonsterDispatch`].

mod bodies;
mod bodies2;
mod bodies3;
mod bodies4;
mod bodies5;
mod bodies6;
mod bodies7;
mod common;
pub mod forced;
mod functions;
mod npc;
pub mod scans;
pub mod seams;
pub mod skill_check;
pub mod table;
mod tactics;
mod target;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use d2_data::tables::{Levels, Missiles, Monstats, Monstats2, Skills};

use crate::game::Game;
use crate::tick::timer::{TimerClass, TimerRun};
use crate::tick::EventDispatch;
use crate::units::{UnitId, UnitType};

pub use functions::{implemented, run_function, run_init, INIT_IMPLEMENTED};
pub use seams::{
    AiActs, AiHost, AiModes, AiQuests, AiSummons, AiTargets, AiUnits, AiWorld, AsDynAi, HireRow,
    ModeTarget, PortalNpc, QuestCall, QuestHook,
};
pub use table::{AiRecord, AI_TABLE, SPECIAL_TABLE};
pub use tactics::*;
pub use target::{main_search, main_search_with, precheck_a, precheck_b, precheck_c};

/// Event types the AI handles.
pub const EVENT_THINK: u8 = 2;
pub const EVENT_AI_RESET: u8 = 10;

/// Unit modes named by the AI.
pub mod mode {
    pub const DEATH: u8 = 0;
    pub const NEUTRAL: u8 = 1;
    pub const WALK: u8 = 2;
    pub const GETHIT: u8 = 3;
    pub const ATTACK1: u8 = 4;
    pub const ATTACK2: u8 = 5;
    pub const SKILL1: u8 = 8;
    pub const SKILL2: u8 = 9;
    pub const DEAD: u8 = 12;
    pub const SEQUENCE: u8 = 14;
    pub const RUN: u8 = 15;
}

/// State ids (`states.txt` rows) the AI reads.
pub mod state {
    pub const FREEZE: u16 = 1;
    pub const STUNNED: u16 = 21;
    pub const PREVENTHEAL: u16 = 52;
    pub const UNINTERRUPTABLE: u16 = 54;
    pub const DECREPIFY: u16 = 60;
}

/// AI control flags (control +0x08).
pub mod flag {
    pub const TARGET_SEEN: u16 = 0x08;
    pub const BOSS_SOUND_DONE: u16 = 0x10;
    pub const MAY_TELEPORT: u16 = 0x20;
    pub const FORCE_LOS: u16 = 0x40;
}

/// A unit by (type, GUID), as the control stores owners.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitRef {
    pub ty: UnitType,
    pub guid: u32,
}

/// An AI command: five params, param 0 = type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AiCommand {
    pub params: [i32; 5],
}

/// The AI control record that holds each monster's think state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AiControl {
    /// +0x00.
    pub special_state: u32,
    /// +0x04: current function (the AI function id), 0 = none.
    pub function: u32,
    /// +0x08.
    pub flags: u16,
    /// +0x0C/+0x10: leash owner; `None` = GUID −1.
    pub owner: Option<UnitRef>,
    /// +0x14..+0x1C.
    pub params: [i32; 3],
    /// +0x20/+0x24: the command list; `commands[cur]` is the current one
    /// and the elements after it its successors.
    pub commands: Vec<AiCommand>,
    pub cur: usize,
    /// +0x2C/+0x30: minion owner (pack leader).
    pub minion_owner: Option<UnitRef>,
    /// +0x34: GUIDs of this leader's minions (written by population
    /// code).
    pub minions: Vec<u32>,
    /// +0x38: the Npc map-AI nodes; `None` = no record. Built from
    /// the preset's DS1 path by the preset spawn (open question 8,
    /// the preset's map file).
    pub map_ai: Option<Vec<MapNode>>,
    /// +0x3C: the minion spawn class (Nihlathak's think step
    /// step 8).
    pub spawn_class: i32,
}

/// A map-AI node (12 bytes: action, x, y).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MapNode {
    pub action: i32,
    pub x: i32,
    pub y: i32,
}

/// The AI param record's velocity request (monster data +0x2C).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VelocityRequest {
    /// +0x18.
    pub method: i32,
    /// +0x1C.
    pub speed: i32,
    /// +0x20, capped at 77.
    pub steps: i32,
}

/// Per-monster AI state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MonsterAi {
    /// `None` until an AI is installed (monster data +0x28 = 0).
    pub control: Option<AiControl>,
    pub velocity: VelocityRequest,
}

/// A call in the game's rules that this code does not model yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unhandled {
    /// AI think/init/alternate function by address (stub; status other
    /// than `spec'd-here` in `ai-functions.tsv`, or a special-state
    /// function).
    Function { addr: u32, unit: UnitId },
    /// A think on a monster with no AI control.
    NoControl { unit: UnitId },
    /// A velocity request with a speed outside −126…126 (fatal assert in
    /// the game's rules).
    VelocityAssert { unit: UnitId, speed: i32 },
}

/// The AI system's state.
#[derive(Clone, Debug, Default)]
pub struct AiStore {
    units: BTreeMap<UnitId, MonsterAi>,
    pub unhandled: Vec<Unhandled>,
    /// The Npc command counter G (the Npc commands step 1).
    ///
    /// G is process-wide in the game's rules: it counts from 0 at process
    /// start across every game of the process and is never reset or saved.
    /// `d2-sim` has no process state, so G is
    /// a host input: the host (one per server process) hands the value
    /// left by its previous game to the next game's store
    /// ([`AiStore::with_npc_walk_counter`]) and reads it back from here
    /// when the game ends. A fresh process starts at 0 ([`AiStore::new`]).
    pub npc_walk_counter: u32,
}

impl AiStore {
    /// The store of the first game of a fresh process (G = 0).
    pub fn new() -> Self {
        Self::default()
    }
    /// The store of a later game of the same process: G continues from
    /// `g`, the value the previous game left.
    pub fn with_npc_walk_counter(g: u32) -> Self {
        Self {
            npc_walk_counter: g,
            ..Self::default()
        }
    }
    pub fn get(&self, u: UnitId) -> Option<&MonsterAi> {
        self.units.get(&u)
    }
    pub fn entry(&mut self, u: UnitId) -> &mut MonsterAi {
        self.units.entry(u).or_default()
    }
    pub fn control(&self, u: UnitId) -> Option<&AiControl> {
        self.units.get(&u)?.control.as_ref()
    }
    pub fn control_mut(&mut self, u: UnitId) -> Option<&mut AiControl> {
        self.units.get_mut(&u)?.control.as_mut()
    }
    /// Drops a removed unit's state.
    pub fn remove(&mut self, u: UnitId) {
        self.units.remove(&u);
    }
}

/// Game fields the AI reads (game +0x6A, +0x74, +0x6D).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GameInfo {
    pub game_type: u8,
    pub game_type_ex: u32,
    /// 0 Normal, 1 Nightmare, 2 Hell.
    pub difficulty: u8,
}

/// The data tables the AI reads, typed `d2-data` records. `skill_modes`
/// are the compiled `Sk1mode..Sk8mode` bytes per monstats row (record
/// +0x180..+0x187; a callback column, so not in the typed record).
/// `skills` (`aurastate`, `auratargetstate`, `attackrank`, `Param5`) and
/// `missiles` (`Range`) are read by the Act II–V bodies.
#[derive(Clone, Copy)]
pub struct AiTables<'a> {
    pub monstats: &'a [Monstats],
    pub monstats2: &'a [Monstats2],
    pub levels: &'a [Levels],
    pub skill_modes: &'a [[u8; 8]],
    pub skills: &'a [Skills],
    pub missiles: &'a [Missiles],
}

/// The `Sk1mode..Sk8mode` bytes of every `monstats.bin` record (+0x180;
/// `Sk4mode` at +0x183 is read by Vampire, `Sk5mode`..`Sk8mode`
/// by the Act II–V bodies).
pub fn skill_modes(t: &d2_data::bin::BinTable) -> Vec<[u8; 8]> {
    t.iter()
        .map(|r| {
            r.get(0x180..0x188)
                .and_then(|b| b.try_into().ok())
                .unwrap_or([0; 8])
        })
        .collect()
}

/// What AI code works with.
pub struct Ctx<'a, W: ?Sized> {
    pub tables: AiTables<'a>,
    pub info: GameInfo,
    pub store: &'a mut AiStore,
    pub world: &'a mut W,
}

/// The AI tick parameter record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickParam {
    pub target: Option<UnitId>,
    pub distance: i32,
    pub combat: bool,
    /// Monstats row.
    pub class: usize,
    /// Monstats2 row.
    pub class2: usize,
}

impl<W: AiHost + ?Sized> Ctx<'_, W> {
    /// This context with its world as `dyn AiHost`, for a mod game's AI
    /// bodies (`docs/plans/mods/seams.md` MS5).
    pub fn as_dyn(&mut self) -> Ctx<'_, dyn AiHost + '_> {
        Ctx {
            tables: self.tables,
            info: self.info,
            store: &mut *self.store,
            world: self.world.as_dyn_ai(),
        }
    }

    /// The spawn with the AI store lent to the host, so the created
    /// unit gets its AI control and install inside the creation.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_monster(
        &mut self,
        game: &mut Game,
        room: crate::units::RoomId,
        x: i32,
        y: i32,
        class: i32,
        mode: u8,
        spread: i32,
        flags: u32,
    ) -> Option<UnitId> {
        if let Some(back) = self.world.lend_ai_store(std::mem::take(&mut *self.store)) {
            *self.store = back;
        }
        let unit = self
            .world
            .spawn_monster(game, room, x, y, class, mode, spread, flags);
        if let Some(store) = self.world.return_ai_store() {
            *self.store = store;
        }
        unit
    }

    pub fn monstats(&self, class: i32) -> Option<&Monstats> {
        usize::try_from(class)
            .ok()
            .and_then(|c| self.tables.monstats.get(c))
    }

    /// `aipN` (1…8) of the difficulty, signed.
    pub fn aip(&self, p: &TickParam, n: usize) -> i32 {
        let Some(r) = self.tables.monstats.get(p.class) else {
            return 0;
        };
        let v = match (n, self.info.difficulty) {
            (1, 0) => r.aip1,
            (1, 1) => r.aip1_n,
            (1, _) => r.aip1_h,
            (2, 0) => r.aip2,
            (2, 1) => r.aip2_n,
            (2, _) => r.aip2_h,
            (3, 0) => r.aip3,
            (3, 1) => r.aip3_n,
            (3, _) => r.aip3_h,
            (4, 0) => r.aip4,
            (4, 1) => r.aip4_n,
            (4, _) => r.aip4_h,
            (5, 0) => r.aip5,
            (5, 1) => r.aip5_n,
            (5, _) => r.aip5_h,
            (6, 0) => r.aip6,
            (6, 1) => r.aip6_n,
            (6, _) => r.aip6_h,
            (7, 0) => r.aip7,
            (7, 1) => r.aip7_n,
            (7, _) => r.aip7_h,
            (8, 0) => r.aip8,
            (8, 1) => r.aip8_n,
            (8, _) => r.aip8_h,
            _ => 0,
        };
        i32::from(v as i16)
    }

    /// `aidel`: the Normal column unless game +0x6A or +0x74 is
    /// nonzero; 0 → 15.
    pub fn aidel(&self, class: i32) -> i32 {
        let Some(r) = self.monstats(class) else {
            return 15;
        };
        let v = if self.info.game_type == 0 && self.info.game_type_ex == 0 {
            r.aidel
        } else {
            match self.info.difficulty {
                0 => r.aidel,
                1 => r.aidel_n,
                _ => r.aidel_h,
            }
        };
        if v == 0 {
            15
        } else {
            i32::from(v)
        }
    }

    /// `aidist` of the difficulty (no game-type gate); 0 → 35.
    pub fn aidist(&self, class: i32) -> i32 {
        let v = self
            .monstats(class)
            .map_or(0, |r| match self.info.difficulty {
                0 => r.aidist,
                1 => r.aidist_n,
                _ => r.aidist_h,
            });
        if v == 0 {
            35
        } else {
            i32::from(v)
        }
    }

    /// Base class (monstats `BaseId`).
    pub fn base_class(&self, unit: UnitId) -> i32 {
        let class = self.world.class(unit);
        self.monstats(class)
            .map_or(-1, |r| i32::from(r.baseid as i16))
    }

    /// Skill `n` (1…8) of the row as a signed id (< 0 = none) and its mode.
    pub fn skill(&self, p: &TickParam, n: usize) -> (i32, u8) {
        self.class_skill(p.class as i32, n)
    }

    /// Skill `n` (1…8) of monstats row `class`, signed, and its mode;
    /// (−1, 0) without a row.
    pub fn class_skill(&self, class: i32, n: usize) -> (i32, u8) {
        let Some(r) = self.monstats(class) else {
            return (-1, 0);
        };
        let s = match n {
            1 => r.skill1,
            2 => r.skill2,
            3 => r.skill3,
            4 => r.skill4,
            5 => r.skill5,
            6 => r.skill6,
            7 => r.skill7,
            _ => r.skill8,
        };
        let m = usize::try_from(class)
            .ok()
            .and_then(|c| self.tables.skill_modes.get(c))
            .map_or(0, |m| m[(n - 1).min(7)]);
        (i32::from(s as i16), m)
    }

    /// `aipN` (1…8) of monstats row `class` for the difficulty; 0
    /// without a row.
    pub fn class_aip(&self, class: i32, n: usize) -> i32 {
        match usize::try_from(class) {
            Ok(c) if c < self.tables.monstats.len() => self.aip(
                &TickParam {
                    target: None,
                    distance: 0,
                    combat: false,
                    class: c,
                    class2: 0,
                },
                n,
            ),
            _ => 0,
        }
    }

    /// "P(v)": one step of the unit seed, `lo' % 100 < v` signed.
    pub fn chance(&mut self, unit: UnitId, v: i32) -> bool {
        ((self.world.seed(unit).step() % 100) as i32) < v
    }

    /// AI state 3 or 19.
    pub fn ai_state_set(&self, unit: UnitId) -> bool {
        matches!(self.world.ai_state(unit), 3 | 19)
    }

    /// Whether the class can walk.
    pub fn can_walk(&self, unit: UnitId) -> bool {
        let c = self.world.class(unit);
        self.world.class_has_mode(c, mode::WALK)
    }

    /// The unit's AI table row. A special AI state (set by curses,
    /// possession and the like) picks its row from [`SPECIAL_TABLE`];
    /// states 10-12 only for classes with monstats `SwitchAI`. Otherwise
    /// the monstats `AI` column picks the [`AI_TABLE`] row, row 0 when
    /// it's past the table. A mod game's rows (MS5) come first.
    pub fn record(&self, unit: UnitId) -> AiRecord {
        let special = self.store.control(unit).map_or(0, |c| c.special_state);
        let class = self.world.class(unit);
        let row = self.monstats(class);
        let provider = self.world.rules().monsters();
        if special != 0 {
            if let Some(r) = provider.and_then(|p| p.special_record(special, class)) {
                return r;
            }
            if (special as usize) < SPECIAL_TABLE.len() {
                let gated = matches!(special, 10..=12);
                if !gated || row.is_some_and(|r| r.switchai) {
                    return SPECIAL_TABLE[special as usize];
                }
            }
        }
        if let Some(r) = row.and_then(|r| provider?.ai_record(r.ai)) {
            return r;
        }
        let ai = row.map_or(0, |r| r.ai as usize);
        AI_TABLE.get(ai).copied().unwrap_or(AI_TABLE[0])
    }
}

/// Deletes the unit's pending thinks.
pub fn delete_thinks(game: &mut Game, unit: UnitId) {
    game.timers.cancel_unit_events(unit, EVENT_THINK, None);
}

/// The last call of the monster death
/// clean-up (the rule that keeps a
/// dead monster dead): cancel the unit's type-2 (think) events, then
/// its type-3 (stat regeneration) events, any argument.
pub fn cancel_think_and_regen(game: &mut Game, unit: UnitId) {
    delete_thinks(game, unit);
    game.timers
        .cancel_unit_events(unit, crate::tick::events::event::STAT_REGEN, None);
}

/// The first pending think's expire frame, 0 when none.
pub fn pending_think(game: &Game, unit: UnitId) -> i32 {
    game.timers
        .unit_timers(unit)
        .into_iter()
        .find(|&t| game.timers.event(t).is_some_and(|e| e.0 == EVENT_THINK))
        .and_then(|t| game.timers.expire(t))
        .unwrap_or(0)
}

/// Schedules a think at `expire` (type 2) with the
/// state-54 rule: a monster with state 54
/// first runs, which clears the state and cancels
/// all its type-2 events.
pub fn schedule_think<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    unit: UnitId,
    expire: i32,
) {
    if cx.world.has_state(unit, state::UNINTERRUPTABLE) {
        cx.world.clear_uninterruptable(game, unit);
        delete_thinks(game, unit);
    }
    // A monster has a timer class: the error cannot happen.
    let _ = game.schedule_event(unit, u32::from(EVENT_THINK), expire, None, 0, 0);
}

/// A think delay of `n` base ticks as ticks at the game's rate. A delay
/// of 0 or 1 means "the next tick" and stays one tick.
fn think_delay(game: &Game, n: i32) -> i32 {
    if n == 0 || n == 1 {
        1
    } else {
        game.ticks(n)
    }
}

/// Delete thinks, then schedule one at frame + n (`n` already in ticks
/// at the game's rate).
fn reschedule<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, unit: UnitId, n: i32) {
    delete_thinks(game, unit);
    let at = game.frame.wrapping_add(n);
    schedule_think(game, cx, unit, at);
}

/// An AI mode request: the
/// mode change with the monster's velocity request, which the
/// mode set's movement set-up consumes.
pub fn request_mode<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    unit: UnitId,
    m: u8,
    target: ModeTarget,
) -> bool {
    request_mode_byte(game, cx, unit, m, target, None)
}

/// [`request_mode`] with the request's path-type byte overwritten
/// (`path_byte`).
pub fn request_mode_byte<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    unit: UnitId,
    m: u8,
    target: ModeTarget,
    path_byte: Option<u8>,
) -> bool {
    // The request builder clears the current skill.
    cx.world.clear_current_skill(unit);
    request_mode_keep_skill(game, cx, unit, m, target, path_byte)
}

/// [`request_mode_byte`] for a caller that has set the current skill
/// after the builder's clear: the skill set
/// before this call is the one the mode start sees.
pub fn request_mode_keep_skill<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    unit: UnitId,
    m: u8,
    target: ModeTarget,
    path_byte: Option<u8>,
) -> bool {
    let mut v = cx.store.get(unit).map(|e| e.velocity).unwrap_or_default();
    let ok = cx
        .world
        .change_mode_with(game, unit, m, target, path_byte, &mut v);
    if let Some(e) = cx.store.units.get_mut(&unit) {
        e.velocity = v;
    }
    ok
}

/// `AITACTICS_IdleInNeutralMode` ("idle N"): N 0 → 1;
/// a non-neutral unit first gets a mode change to neutral targeting
/// itself; then delete + schedule.
pub fn idle<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, unit: UnitId, n: i32) {
    let n = think_delay(game, n);
    idle_raw(game, cx, unit, n);
}

/// [`idle`] with a delay already counted in ticks at the game's rate.
pub fn idle_raw<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, unit: UnitId, n: i32) {
    let n = if n == 0 { 1 } else { n };
    if cx.world.anim_mode(unit) != mode::NEUTRAL {
        request_mode(game, cx, unit, mode::NEUTRAL, ModeTarget::Unit(unit));
    }
    reschedule(game, cx, unit, n);
}

/// `AITACTICS_Idle`: delete + schedule, mode unchanged.
pub fn idle_keep_mode<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    unit: UnitId,
    n: i32,
) {
    let n = think_delay(game, n);
    idle_keep_mode_raw(game, cx, unit, n);
}

/// [`idle_keep_mode`] with a delay already counted in ticks at the game's
/// rate.
pub fn idle_keep_mode_raw<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    unit: UnitId,
    n: i32,
) {
    let n = if n == 0 { 1 } else { n };
    reschedule(game, cx, unit, n);
}

/// As [`idle_keep_mode`] but only if no think is pending,
/// the pending one is due this frame, or it is due at frame + N or later.
pub fn idle_if_later<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    unit: UnitId,
    n: i32,
) {
    let n = think_delay(game, n);
    let pending = game
        .timers
        .unit_timers(unit)
        .into_iter()
        .find(|&t| game.timers.event(t).is_some_and(|e| e.0 == EVENT_THINK))
        .and_then(|t| game.timers.expire(t));
    let ok = match pending {
        None => true,
        Some(e) => e == game.frame || e >= game.frame.wrapping_add(n),
    };
    if ok {
        reschedule(game, cx, unit, n);
    }
}

/// The neutral mode's start, called by the mode
/// machinery when a monster enters neutral (also after a failed mode
/// start). Combat mode 1 is the caller's. Adds (no delete) a think at
/// frame + 45 when stunned, else + `aidel`, only if the pending think's
/// frame (0 when none) is ≤ the current frame.
pub fn neutral_mode_start<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, unit: UnitId) {
    if pending_think(game, unit) > game.frame {
        return;
    }
    let delay = if cx.world.has_state(unit, state::STUNNED) {
        45
    } else {
        cx.aidel(cx.world.class(unit))
    };
    let at = game.after(delay);
    schedule_think(game, cx, unit, at);
}

/// The knockback mode's end.
pub fn knockback_end<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, unit: UnitId) {
    let class = cx.world.class(unit);
    let delay = if !cx.world.class_has_mode(class, mode::GETHIT) {
        1
    } else if cx.base_class(unit) == 78 {
        if cx.world.has_state(unit, state::STUNNED) {
            45
        } else {
            15
        }
    } else {
        cx.world.knockback_to_gethit(game, unit);
        return;
    };
    // A delay of 1 is the next tick; the others are 15 and 45 base ticks.
    let at = game
        .frame
        .wrapping_add(if delay == 1 { 1 } else { game.ticks(delay) });
    schedule_think(game, cx, unit, at);
}

/// `MONSTER_UpdateAiCallbackEvent`.
pub fn update_ai_callback<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, unit: UnitId) {
    delete_thinks(game, unit);
    let m = cx.world.anim_mode(unit);
    let schedule = match m {
        mode::NEUTRAL => true,
        mode::DEATH | mode::DEAD => false,
        _ => matches!(cx.base_class(unit), 110 | 118 | 136 | 247),
    };
    if schedule {
        // Two base ticks (80 ms) after the room wakes.
        let at = game.after(2);
        schedule_think(game, cx, unit, at);
    }
}

/// The first client entered `room`; every
/// monster of the room's unit list gets [`update_ai_callback`]. The
/// room +0x78 < 2 test is the caller's.
pub fn client_entered_room<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    room: crate::units::RoomId,
) {
    for u in game.lists.room_units(room) {
        if game
            .lists
            .unit(u)
            .is_some_and(|e| e.ty == UnitType::Monster)
        {
            update_ai_callback(game, cx, u);
        }
    }
}

/// NPC interaction (`start`, then the update): for
/// classes with monstats `npc` and `interact`: stop the path; on start
/// AI param 0 := 40; delete + schedule +1.
pub fn npc_interaction<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    unit: UnitId,
    start: bool,
) {
    let class = cx.world.class(unit);
    if !cx.monstats(class).is_some_and(|r| r.npc && r.interact) {
        return;
    }
    cx.world.stop_path(unit);
    if start {
        if let Some(c) = cx.store.control_mut(unit) {
            c.params[0] = 40;
        }
    }
    reschedule(game, cx, unit, 1);
}

/// Freeze apply: delete + schedule at frame +
/// length + 1.
pub fn schedule_after_freeze<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    unit: UnitId,
    length: i32,
) {
    let n = game.ticks(length).wrapping_add(1);
    reschedule(game, cx, unit, n);
}

/// The freeze gate: state 1 on a living monster.
pub fn frozen<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, unit: UnitId) -> bool {
    cx.world.has_state(unit, state::FREEZE) && !cx.world.is_dead(unit)
}

/// The end of modes 3–9 and 14: the `SplEndGeneric`
/// cases and the walk/run ends run the think inline; every other case
/// requests a mode change to neutral.
pub fn mode_end<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, unit: UnitId, ended: u8) {
    const INLINE: [u8; 16] = [0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
    let class = cx.world.class(unit);
    let generic = cx.monstats(class).is_some_and(|r| r.splendgeneric != 0);
    let base = cx.base_class(unit);
    // The `SplEndGeneric` branch is tested first and does not set
    // the anim mode; a matching class whose mode does not match falls to
    // the table test.
    let special = generic
        && match base {
            110 => ended == 8,
            247 => ended == 14,
            136 => matches!(ended, 10 | 11),
            230 | 231 | 403 => true,
            118 => ended == 2,
            _ => false,
        };
    if special {
        if !frozen(cx, unit) {
            think(game, cx, unit);
        }
        return;
    }
    if INLINE.get(ended as usize) == Some(&1) {
        cx.world.set_anim_mode(game, unit, mode::NEUTRAL);
        if !frozen(cx, unit) {
            think(game, cx, unit);
        }
        return;
    }
    // The generic end: a unit with state 54 first runs
    // the uninterruptable clear and a dead one stops there. The request
    // record: target unit := the path target unit (the unit itself
    // counting as none), point (0, 0); so without a
    // target unit the path target point becomes (0, 0). Its
    // builder also clears the used skill entry
    // ([`request_mode_byte`]).
    if cx.world.has_state(unit, state::UNINTERRUPTABLE) {
        cx.world.clear_uninterruptable(game, unit);
        delete_thinks(game, unit);
        if cx.world.is_dead(unit) {
            return;
        }
    }
    let target = match cx.world.path_target(unit) {
        Some(t) => ModeTarget::Unit(t),
        None => ModeTarget::Point(0, 0),
    };
    request_mode(game, cx, unit, mode::NEUTRAL, target);
}

/// The trapped soul's A1/A2 and S1/S2 ends
/// (per-class records): a mode request to S1 targeting
/// itself, then the think unless frozen.
pub fn trapped_soul_end<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, unit: UnitId) {
    request_mode(game, cx, unit, mode::SKILL1, ModeTarget::Unit(unit));
    if !frozen(cx, unit) {
        think(game, cx, unit);
    }
}

/// Installs the AI for special state `state` (0: the monster's own row)
/// on `unit`:
/// 1. never on an uninterruptable monster (a fatal error);
/// 2. only monsters, and only states with a row; a class past monstats
///    installs state 1;
/// 3. a monster already running the row's think switches to the row's
///    alternate function when it has one, and stops there;
/// 4. otherwise the control's params and commands are cleared, the row's
///    init runs and its think (Idle when it has none) becomes current.
pub fn install<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, unit: UnitId, state: u32) {
    // Step 1.
    assert!(
        !cx.world.has_state(unit, state::UNINTERRUPTABLE),
        "AI install on an uninterruptable unit (state 54)"
    );
    // Step 2: only monsters, and only the special states with a row (a
    // mod game's rows count, MS5).
    let mod_state = state as usize >= SPECIAL_TABLE.len()
        && cx
            .world
            .rules()
            .monsters()
            .is_some_and(|p| p.special_record(state, cx.world.class(unit)).is_some());
    if (state as usize >= SPECIAL_TABLE.len() && !mod_state)
        || game
            .lists
            .unit(unit)
            .is_none_or(|e| e.ty != UnitType::Monster)
    {
        return;
    }
    let class = cx.world.class(unit);
    let state = if usize::try_from(class).map_or(true, |c| c >= cx.tables.monstats.len()) {
        1
    } else {
        state
    };
    let Some(control) = cx.store.control(unit).cloned() else {
        // TODO: the control is allocated at creation;
        // without one there is nothing to install into.
        cx.store.unhandled.push(Unhandled::NoControl { unit });
        return;
    };
    // Step 3.
    if control.function != 0 {
        let cur = cx.record(unit);
        if cur.think == control.function && cur.alt != 0 {
            let c = cx.store.control_mut(unit).expect("control");
            c.special_state = state;
            c.function = cur.alt;
            return;
        }
    }
    // Step 4.
    {
        let c = cx.store.control_mut(unit).expect("control");
        c.params = [0; 3];
        c.commands.clear();
        c.cur = 0;
    }
    let rec = record_for(cx, unit, state);
    // The init function runs with a fresh tick record; nothing when either
    // record is missing.
    let rows = cx
        .monstats(class)
        .is_some_and(|r| (r.monstatsex as usize) < cx.tables.monstats2.len());
    if rec.init != 0 && rows {
        functions::run_init(game, cx, rec.init, unit);
    }
    let c = cx.store.control_mut(unit).expect("control");
    c.special_state = state;
    c.function = if rec.think == 0 {
        table::IDLE_FN
    } else {
        rec.think
    };
}

/// The record for `state` (the state lookup with a given state).
fn record_for<W: AiHost + ?Sized>(cx: &mut Ctx<'_, W>, unit: UnitId, state: u32) -> AiRecord {
    let saved = cx.store.control(unit).map_or(0, |c| c.special_state);
    if let Some(c) = cx.store.control_mut(unit) {
        c.special_state = state;
    }
    let r = cx.record(unit);
    if let Some(c) = cx.store.control_mut(unit) {
        c.special_state = saved;
    }
    r
}

/// The think handler.
pub fn think<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, unit: UnitId) {
    let class = cx.world.class(unit);
    let Some(row) = cx.monstats(class) else {
        return;
    };
    let class2 = row.monstatsex as usize;
    if class2 >= cx.tables.monstats2.len() {
        return;
    }
    let Some(function) = cx.store.control(unit).map(|c| c.function) else {
        cx.store.unhandled.push(Unhandled::NoControl { unit });
        return;
    };
    let mut p = TickParam {
        target: None,
        distance: 0,
        combat: false,
        class: class as usize,
        class2,
    };
    if precheck_a(game, cx, unit, &p) {
        return;
    }
    if precheck_b(game, cx, unit, &mut p) {
        return;
    }
    if precheck_c(game, cx, unit, &mut p) {
        return;
    }
    // The prechecks may have reinstalled the AI: call the current one.
    let f = cx.store.control(unit).map_or(function, |c| c.function);
    crate::cov!(MonsterAi, f, class);
    run_function(game, cx, f, unit, &p);
}

/// The monster class handler's AI events:
/// type 2 runs [`think`] unless the monster is frozen and alive; type 10
/// runs the AI reset.
pub fn handle_event<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    run: &TimerRun,
) -> bool {
    let unit = run.owner.unit;
    match run.event {
        EVENT_THINK => {
            if !frozen(cx, unit) {
                think(game, cx, unit);
            }
            true
        }
        EVENT_AI_RESET => {
            cx.world.ai_reset(unit);
            true
        }
        _ => false,
    }
}

/// [`EventDispatch`] for the tick: monster-class type-2 and type-10
/// events without an explicit callback run the AI; everything else goes
/// to `next`.
pub struct MonsterDispatch<'a, W: ?Sized, N: ?Sized> {
    pub cx: Ctx<'a, W>,
    pub next: &'a mut N,
}

impl<W: AiHost + ?Sized, N: EventDispatch + ?Sized> EventDispatch for MonsterDispatch<'_, W, N> {
    fn run_event(&mut self, game: &mut Game, run: &TimerRun) {
        if run.class == TimerClass::Monster
            && run.callback.is_none()
            && handle_event(game, &mut self.cx, run)
        {
            return;
        }
        self.next.run_event(game, run);
    }
}

//! Modes: setting a mode, preparing the animation, the player
//! mode starts and their event-0/event-1 handlers, the monster
//! mode set with its mode table and the neutral start, the player
//! join sequence and the missile setup.

use thiserror::Error;

use crate::game::GameError;
use crate::stats::stat;
use crate::stats::states::state;
use crate::tick::events::event;

use super::anim::{self, AnimError, Form};
use super::hooks::{Sim, UnitHooks};
use super::record::flags;
use super::{UnitId, UnitType};

/// Player modes (plrmode.txt rows).
pub mod player_mode {
    pub const DT: u32 = 0;
    pub const NU: u32 = 1;
    pub const WL: u32 = 2;
    pub const RN: u32 = 3;
    pub const GH: u32 = 4;
    pub const TN: u32 = 5;
    pub const TW: u32 = 6;
    pub const A1: u32 = 7;
    pub const A2: u32 = 8;
    pub const BL: u32 = 9;
    pub const TH: u32 = 11;
    pub const DD: u32 = 17;
    pub const SEQUENCE: u32 = 18;
    pub const KB: u32 = 19;
    pub const COUNT: u32 = 20;
}

/// Monster modes (monmode.txt rows).
pub mod monster_mode {
    pub const DT: u32 = 0;
    pub const NU: u32 = 1;
    pub const GH: u32 = 3;
    pub const DD: u32 = 12;
    pub const SEQUENCE: u32 = 14;
    pub const COUNT: u32 = 16;
}

/// Errors of the unit operations (D2RUST API misuse or a fatal assertion
/// of the base game).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum UnitError {
    #[error("unknown unit {0:?}")]
    UnknownUnit(UnitId),
    #[error("mode {mode} out of range for {ty:?}")]
    BadMode { ty: UnitType, mode: u32 },
    /// Asserts the monster has no state 54.
    #[error("monster mode set with state 54")]
    Uninterruptable,
    /// A null object or item handler (fatal in the base game).
    #[error("null {ty:?} handler for event {event}")]
    NullHandler { ty: UnitType, event: u8 },
    /// A timer callback D2RUST does not know.
    #[error("unknown timer callback {0:#x}")]
    UnknownCallback(u32),
    #[error(transparent)]
    Anim(#[from] AnimError),
    #[error(transparent)]
    Game(#[from] GameError),
    /// Stat-list expiry an expired extended list stops (edge case 4:
    /// endless in the base game).
    #[error(transparent)]
    Stats(#[from] crate::stats::lists::StatListError),
}

fn record_mut<'a>(
    sim: &'a mut Sim<'_>,
    unit: UnitId,
) -> Result<&'a mut super::record::UnitRecord, UnitError> {
    sim.units.get_mut(unit).ok_or(UnitError::UnknownUnit(unit))
}

///`(game, unit, mode): drop the unit's combat
/// entries, then`(unit, mode).
pub fn set_mode<H: UnitHooks>(
    sim: &mut Sim<'_>,
    hooks: &mut H,
    unit: UnitId,
    mode: u32,
) -> Result<(), UnitError> {
    hooks.drop_combat_entries(sim, unit);
    write_mode(sim, hooks, unit, mode)
}

///`(unit, mode), the part of [`set_mode`] after the
/// combat drop: the mode written, the unit queued for update, flag 0x1,
/// and on a new mode the TEMPONLY lists freed and the animation fields
/// re-initialised. A monster staying in mode 1 and a tile: nothing.
pub fn write_mode<H: UnitHooks>(
    sim: &mut Sim<'_>,
    hooks: &mut H,
    unit: UnitId,
    mode: u32,
) -> Result<(), UnitError> {
    let rec = record_mut(sim, unit)?;
    if rec.ty == UnitType::Tile {
        return Ok(());
    }
    let changed = rec.mode != mode;
    if !changed && rec.ty == UnitType::Monster && mode == monster_mode::NU {
        return Ok(());
    }
    rec.mode = mode;
    rec.flags |= flags::CHANGED;
    sim.game.lists.queue_update(unit).map_err(GameError::from)?;
    if changed {
        // A new mode frees the
        // unit's TEMPONLY lists (the path's run list, the
        // attack-rate lists of the skill bodies).
        sim.stats.remove_temporary_lists(hooks, unit);
        hooks.reinit_anim(sim, unit);
    }
    Ok(())
}

/// Prepare animation. The frame count +0x48: the
/// sequence's count · 256 in the sequence branch (step 2 of the
/// sequence; that branch leaves it), the
/// AnimData record's frames · 256 otherwise.
pub fn prepare_animation<H: UnitHooks>(
    sim: &mut Sim<'_>,
    hooks: &mut H,
    unit: UnitId,
) -> Result<(), UnitError> {
    let rec = sim.units.get(unit).ok_or(UnitError::UnknownUnit(unit))?;
    let sequence_mode = match rec.ty {
        UnitType::Player => rec.mode == player_mode::SEQUENCE,
        UnitType::Monster => rec.mode == monster_mode::SEQUENCE,
        _ => false,
    };
    let sequence = if sequence_mode {
        hooks.load_sequence(sim, unit)
    } else {
        None
    };
    let rate = if sequence.is_none() && hooks.has_path(sim, unit) {
        let r = hooks.anim_rate(sim, unit);
        hooks.anim_velocity(sim, unit);
        Some(r)
    } else {
        None
    };
    // The sequence branch computes the rate whether or not the unit has a
    // path.
    let seq_rate = match &sequence {
        Some(seq) => {
            let r = hooks.sequence_rate(sim, unit, seq.frame_count);
            hooks.anim_velocity(sim, unit);
            Some(r)
        }
        None => None,
    };
    let record = hooks.anim_record(sim, unit);
    let a = &mut record_mut(sim, unit)?.anim;
    a.action_frame = 0;
    if let Some(mut seq) = sequence {
        if let Some((speed, seq_speed)) = seq_rate {
            a.speed = speed;
            if let Some(v) = seq_speed {
                seq.speed = v;
            }
        }
        a.frame_count = seq.frame_count;
        // Load step 3: +0x44 := the drawn frame of
        // frame 0 · 256 when the list carries the drawn frames; +0x4E :=
        // frame 0's event byte (measured, gargoyle trap: the event on
        // frame 0 of `seq_gargoyletrap` fires the do on the next tick).
        if let Some(&d) = seq.drawn.first() {
            a.frame = i32::from(d) << 8;
        }
        a.action_frame = seq.events.first().copied().unwrap_or(0);
        a.sequence = Some(seq);
    } else {
        a.sequence = None;
        a.frame = 0;
        if let Some(r) = rate {
            a.speed = r;
        }
    }
    a.record = record;
    if let (None, Some(r)) = (&a.sequence, record) {
        a.frame_count = (r.frames as i32).wrapping_mul(256);
    }
    Ok(())
}

/// The animated mode start tail: prepare, cancel 0/1, main
/// form with the frame bonus.
pub fn animate<H: UnitHooks>(
    sim: &mut Sim<'_>,
    hooks: &mut H,
    unit: UnitId,
) -> Result<(), UnitError> {
    prepare_animation(sim, hooks, unit)?;
    anim::cancel_mode_events(sim.game, unit);
    let bonus = hooks.frame_bonus(sim, unit);
    let rec = sim
        .units
        .get_mut(unit)
        .ok_or(UnitError::UnknownUnit(unit))?;
    anim::run(sim.game, unit, &mut rec.anim, Form::Main { bonus })?;
    Ok(())
}

// ---- players --------------------------------------------------------

/// The player's start-function class (table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerStart {
    /// 0 DT.
    Death,
    /// 1 NU, 5 TN.
    Neutral,
    /// 2 WL, 3 RN, 6 TW, 19 KB.
    Move,
    /// 4 GH, 9 BL.
    Animate,
    /// 7, 8, 10–16, 18.
    Skill,
    /// 17 DD.
    Corpse,
}

/// The start function class of a player mode.
pub fn player_start_kind(mode: u32) -> Option<PlayerStart> {
    Some(match mode {
        0 => PlayerStart::Death,
        1 | 5 => PlayerStart::Neutral,
        2 | 3 | 6 | 19 => PlayerStart::Move,
        4 | 9 => PlayerStart::Animate,
        7 | 8 | 10..=16 | 18 => PlayerStart::Skill,
        17 => PlayerStart::Corpse,
        _ => return None,
    })
}

/// The request check, then the
/// mode's start function. Returns whether the request was accepted.
pub fn player_start<H: UnitHooks>(
    sim: &mut Sim<'_>,
    hooks: &mut H,
    unit: UnitId,
    mode: u32,
) -> Result<bool, UnitError> {
    let kind = player_start_kind(mode).ok_or(UnitError::BadMode {
        ty: UnitType::Player,
        mode,
    })?;
    if !hooks.player_request_check(sim, unit, mode) {
        return Ok(false);
    }
    player_start_function(sim, hooks, unit, mode, kind)?;
    Ok(true)
}

fn player_start_function<H: UnitHooks>(
    sim: &mut Sim<'_>,
    hooks: &mut H,
    unit: UnitId,
    mode: u32,
    kind: PlayerStart,
) -> Result<(), UnitError> {
    match kind {
        PlayerStart::Death => {
            hooks.player_death(sim, unit);
            set_mode(sim, hooks, unit, mode)?;
            animate(sim, hooks, unit)?;
        }
        PlayerStart::Neutral => player_neutral(sim, hooks, unit)?,
        PlayerStart::Move => {
            let town = hooks.room_flag(sim, unit);
            let mut m = mode;
            if m == player_mode::RN && sim.stats.unit_total(unit, stat::STAMINA, 0) == 0 {
                m = player_mode::WL;
            }
            if matches!(m, player_mode::WL | player_mode::TW) {
                m = if town {
                    player_mode::TW
                } else {
                    player_mode::WL
                };
            }
            if m == player_mode::KB {
                hooks.player_knockback_path(sim, unit);
            }
            set_mode(sim, hooks, unit, m)?;
            anim::cancel_mode_events(sim.game, unit);
            anim::every_tick_movement(sim.game, unit)?;
        }
        PlayerStart::Animate => {
            set_mode(sim, hooks, unit, mode)?;
            animate(sim, hooks, unit)?;
        }
        PlayerStart::Skill => {
            set_mode(sim, hooks, unit, mode)?;
            animate(sim, hooks, unit)?;
            record_mut(sim, unit)?.flags &= !flags::ATTACK_PENDING;
            hooks.player_skill_start(sim, unit);
        }
        PlayerStart::Corpse => {
            set_mode(sim, hooks, unit, player_mode::DD)?;
            hooks.player_corpse(sim, unit);
        }
    }
    Ok(())
}

/// Cancel 0/1, mode 1, or 5 in town.
fn player_neutral<H: UnitHooks>(
    sim: &mut Sim<'_>,
    hooks: &mut H,
    unit: UnitId,
) -> Result<(), UnitError> {
    anim::cancel_mode_events(sim.game, unit);
    let m = if hooks.room_flag(sim, unit) {
        player_mode::TN
    } else {
        player_mode::NU
    };
    set_mode(sim, hooks, unit, m)
}

/// Player event 0: the mode's action function (from the mode
/// table); result 2 runs the ENDANIM handler at once.
pub fn player_event0<H: UnitHooks>(
    sim: &mut Sim<'_>,
    hooks: &mut H,
    unit: UnitId,
    a1: u32,
    a2: u32,
) -> Result<(), UnitError> {
    let mode = sim
        .units
        .get(unit)
        .ok_or(UnitError::UnknownUnit(unit))?
        .mode;
    let result = match player_start_kind(mode) {
        Some(PlayerStart::Move) => hooks.player_movement_step(sim, unit, a1, a2),
        Some(PlayerStart::Skill) => hooks.player_action_frame(sim, unit, a1, a2),
        Some(_) => 1,
        None => {
            return Err(UnitError::BadMode {
                ty: UnitType::Player,
                mode,
            })
        }
    };
    if result == 2 {
        player_event1(sim, hooks, unit)?;
    }
    Ok(())
}

/// Player event 1 (ENDANIM).
pub fn player_event1<H: UnitHooks>(
    sim: &mut Sim<'_>,
    hooks: &mut H,
    unit: UnitId,
) -> Result<(), UnitError> {
    let mode = sim
        .units
        .get(unit)
        .ok_or(UnitError::UnknownUnit(unit))?
        .mode;
    if matches!(mode, player_mode::A1 | player_mode::A2 | player_mode::TH)
        || hooks.player_item_row_flagged(sim, unit)
    {
        hooks.player_attack_cleanup(sim, unit);
    }
    match mode {
        player_mode::DT => {
            player_start_function(sim, hooks, unit, player_mode::DD, PlayerStart::Corpse)
        }
        player_mode::KB => {
            // TODO: what a failed check
            // does is not written; nothing here.
            if hooks.player_request_check(sim, unit, player_mode::GH) {
                player_start_function(sim, hooks, unit, player_mode::GH, PlayerStart::Animate)?;
            }
            Ok(())
        }
        _ => player_neutral(sim, hooks, unit),
    }
}

/// Join: neutral mode start (mode 1),
/// then event 3 at f + 1, then event 11 at f + 250.
pub fn player_join<H: UnitHooks>(
    sim: &mut Sim<'_>,
    hooks: &mut H,
    unit: UnitId,
) -> Result<(), UnitError> {
    player_start(sim, hooks, unit, player_mode::NU)?;
    let f = sim.game.frame;
    sim.game.schedule_event(
        unit,
        u32::from(event::STAT_REGEN),
        f.wrapping_add(1),
        None,
        0,
        0,
    )?;
    sim.game.schedule_event(
        unit,
        u32::from(event::DELAYED_PORTAL),
        sim.game.after(250),
        None,
        0,
        0,
    )?;
    Ok(())
}

// ---- monsters ---------------------------------------------------------

/// A monster mode record (+ 16·mode): start, event-0 and
/// event-1 function addresses (0: null) and the schedule flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonsterModeRecord {
    pub start: u32,
    pub event0: u32,
    pub event1: u32,
    pub schedules: bool,
}

/// Fallback for a null record or function.
pub const MONSTER_MODE_FALLBACK: u32 = 0x005A_7B30;

/// The neutral start, implemented here.
pub const MONSTER_NEUTRAL_START: u32 = 0x005A_73E0;

const fn rec(start: u32, event0: u32, event1: u32, schedules: bool) -> MonsterModeRecord {
    MonsterModeRecord {
        start,
        event0,
        event1,
        schedules,
    }
}

/// The monster mode table.
pub const MONSTER_MODES: [MonsterModeRecord; 16] = [
    rec(0x005A6FF0, 0x005A7350, 0x005A72B0, true),
    rec(0x005A73E0, 0x005A6DE0, 0, false),
    rec(0x005A7520, 0x005A8490, 0, true),
    rec(0x005A7580, 0x005A6DE0, 0x005A8030, true),
    rec(0x005A75C0, 0x005A7670, 0x005A8030, true),
    rec(0x005A75C0, 0x005A7670, 0x005A8030, true),
    rec(0x005A77C0, 0x005A6DE0, 0x005A8030, true),
    rec(0x005A75C0, 0x005A7670, 0x005A8030, true),
    rec(0x005A75C0, 0x005A7670, 0x005A8030, true),
    rec(0x005A75C0, 0x005A7670, 0x005A8030, true),
    rec(0x005A7490, 0x005A74A0, 0x005A74D0, false),
    rec(0x005A74E0, 0x005A6DE0, 0, false),
    rec(0x005A7390, 0x005A6DE0, 0, false),
    rec(0x005A77D0, 0x005A8630, 0x005A8520, true),
    rec(0x005A7870, 0x005A8670, 0x005A8030, true),
    rec(0x005A7550, 0x005A84F0, 0, true),
];

/// Whether a monster mode moves (table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moves {
    No,
    Always,
    /// The monstats2 `*mv` bit of the mode.
    Bit,
}

/// The moving table.
pub const MONSTER_MOVES: [Moves; 16] = [
    Moves::No,
    Moves::No,
    Moves::Always,
    Moves::No,
    Moves::Bit,
    Moves::Bit,
    Moves::No,
    Moves::Bit,
    Moves::Bit,
    Moves::Bit,
    Moves::Bit,
    Moves::Bit,
    Moves::No,
    Moves::Always,
    Moves::Always,
    Moves::Always,
];

/// Event 1 of the trapped soul's per-class A1/A2 record.
pub const TRAPPED_SOUL_END_A: u32 = 0x005A_8330;

/// Event 1 of the trapped soul's per-class S1/S2 record.
pub const TRAPPED_SOUL_END_S: u32 = 0x005A_83E0;

/// The per-class mode records for a class whose
/// `SplGetModeChart` is set: `None` → the table's record.
pub fn class_mode_record(class: u32, mode: u32) -> Option<MonsterModeRecord> {
    // The
    // A-family record.
    let attack = MONSTER_MODES[4];
    match (class, mode) {
        // diablo, diabloclone, uberdiablo: S4 and S3.
        (243 | 333 | 705, 10 | 11) => Some(attack),
        // trappedsoul1: own mode ends.
        (403, 4 | 5) => Some(MonsterModeRecord {
            event1: TRAPPED_SOUL_END_A,
            ..attack
        }),
        (403, 8 | 9) => Some(MonsterModeRecord {
            event1: TRAPPED_SOUL_END_S,
            ..attack
        }),
        // shadowwarrior, shadowmaster: S4.
        (417 | 418, 11) => Some(attack),
        // baalthrone, baalcrab, baalclone, uberbaal: S3.
        (543 | 544 | 570 | 709, 10) => Some(attack),
        _ => None,
    }
}

fn monster_record<H: UnitHooks>(
    sim: &Sim<'_>,
    hooks: &mut H,
    unit: UnitId,
    mode: u32,
) -> Option<MonsterModeRecord> {
    if mode as usize >= MONSTER_MODES.len() {
        return None;
    }
    hooks.monster_class_record(sim, unit, mode).or_else(|| {
        let class = sim.units.get(unit)?.class;
        sim.data
            .monster(class)
            .filter(|m| m.mode_chart)
            .and_then(|_| class_mode_record(class, mode))
            .or(Some(MONSTER_MODES[mode as usize]))
    })
}

/// The mode moves.
pub fn monster_moves(sim: &Sim<'_>, unit: UnitId, mode: u32) -> bool {
    match MONSTER_MOVES.get(mode as usize) {
        Some(Moves::Always) => true,
        Some(Moves::Bit) => sim
            .units
            .get(unit)
            .and_then(|r| sim.data.monster(r.class))
            .is_some_and(|m| m.moves & 1 << mode != 0),
        _ => false,
    }
}

/// Monster mode set.
pub fn monster_set_mode<H: UnitHooks>(
    sim: &mut Sim<'_>,
    hooks: &mut H,
    unit: UnitId,
    mode: u32,
) -> Result<(), UnitError> {
    monster_set_mode_started(sim, hooks, unit, mode).map(|_| ())
}

/// [`monster_set_mode`] with its result: whether the requested mode's
/// start ran without falling back to the neutral start.
///
/// The result is the mode set's result: for a live unit
/// it is the requested mode's start result itself, so a start that
/// returns 0 (e.g. a walk whose path has no point)
/// makes it return 0 and the AI's "mode change failed" branches run
/// (measured with a rogue hireling, frame 33).
pub fn monster_set_mode_started<H: UnitHooks>(
    sim: &mut Sim<'_>,
    hooks: &mut H,
    unit: UnitId,
    mode: u32,
) -> Result<bool, UnitError> {
    if mode >= monster_mode::COUNT {
        return Err(UnitError::BadMode {
            ty: UnitType::Monster,
            mode,
        });
    }
    if sim.stats.has_state(unit, state::UNINTERRUPTABLE) {
        return Err(UnitError::Uninterruptable);
    }
    // A live unit's footprint is stamped again for
    // every requested mode, GH too, before the AI-state half.
    if !sim.units.is_dead(unit) {
        hooks.monster_mode_footprint(sim, unit);
    }
    // The AI-state half runs for every mode, GH too).
    hooks.monster_leave_mode(sim, unit);
    if mode != monster_mode::GH {
        hooks.monster_mode_bookkeeping(sim, unit, mode);
        // The mode damage rewrite, then
        // umod mode 0, old mode still set.
        hooks.monster_mode_damage(sim, unit, mode);
        hooks.monster_umods(sim, unit, 0);
    }
    let start = monster_record(sim, hooks, unit, mode)
        .map(|r| r.start)
        .filter(|&s| s != 0)
        .unwrap_or(MONSTER_MODE_FALLBACK);
    let dead = sim.units.is_dead(unit);
    let started = if start == MONSTER_NEUTRAL_START {
        monster_neutral(sim, hooks, unit)?;
        true
    } else {
        hooks.monster_mode_function(sim, unit, start)
    };
    if !started {
        // "What keeps a dead monster dead": a start that refuses a
        // request other than 0 or 12 on a dead unit leaves its mode as is
        // (returns 1); no neutral start.
        if dead && mode != monster_mode::DT && mode != monster_mode::DD {
            return Ok(true);
        }
        monster_neutral(sim, hooks, unit)?;
    }
    record_mut(sim, unit)?.flags |= flags::MODE_CHANGING;
    prepare_animation(sim, hooks, unit)?;
    // Umod mode 1, new mode set; before
    // the cancel of events 0 / 1 and the animation schedule.
    hooks.monster_umods(sim, unit, 1);
    anim::cancel_mode_events(sim.game, unit);
    let now = record_mut(sim, unit)?.mode;
    if monster_record(sim, hooks, unit, now).is_some_and(|r| r.schedules) {
        if monster_moves(sim, unit, now) {
            anim::every_tick_movement(sim.game, unit)?;
        } else {
            let bonus = hooks.frame_bonus(sim, unit);
            let rec = sim
                .units
                .get_mut(unit)
                .ok_or(UnitError::UnknownUnit(unit))?;
            anim::run(sim.game, unit, &mut rec.anim, Form::Main { bonus })?;
        }
    }
    Ok(started)
}

/// The neutral AI delay of the mode set: `aidel` (Normal column, or the
/// difficulty's column when game +0x6A / +0x74 is set), 0 → 15.
pub fn neutral_ai_delay(sim: &Sim<'_>, class: u32) -> i32 {
    let col = if sim.data.aidel_by_difficulty {
        usize::from(sim.data.difficulty).min(2)
    } else {
        0
    };
    match sim.data.monster(class).map_or(0, |m| m.aidel[col]) {
        0 => 15,
        d => i32::from(d),
    }
}

/// Neutral start: mode 1 and, unless the unit has a
/// type-2 timer with expire > f, event 2 at f + 45 (state 21) or
/// f + aidel.
pub fn monster_neutral<H: UnitHooks>(
    sim: &mut Sim<'_>,
    hooks: &mut H,
    unit: UnitId,
) -> Result<(), UnitError> {
    set_mode(sim, hooks, unit, monster_mode::NU)?;
    let f = sim.game.frame;
    let pending = sim
        .game
        .timers
        .unit_timers(unit)
        .into_iter()
        .filter(|&t| {
            sim.game
                .timers
                .event(t)
                .is_some_and(|e| e.0 == event::AI_THINK)
        })
        .filter_map(|t| sim.game.timers.expire(t))
        .filter(|&e| e > 0)
        .min()
        .unwrap_or(0);
    if pending > f {
        return Ok(());
    }
    let expire = if sim.stats.has_state(unit, state::AI_DELAY) {
        sim.game.after(45)
    } else {
        let class = sim
            .units
            .get(unit)
            .ok_or(UnitError::UnknownUnit(unit))?
            .class;
        f.wrapping_add(sim.game.ticks(neutral_ai_delay(sim, class)))
    };
    if sim.stats.has_state(unit, state::UNINTERRUPTABLE) {
        hooks.uninterruptable_check(sim, unit);
    }
    sim.game
        .schedule_event(unit, u32::from(event::AI_THINK), expire, None, 0, 0)?;
    Ok(())
}

/// Monster events 0 and 1: the current
/// mode's function (null:).
pub fn monster_event<H: UnitHooks>(
    sim: &mut Sim<'_>,
    hooks: &mut H,
    unit: UnitId,
    end_anim: bool,
) -> Result<(), UnitError> {
    let mode = sim
        .units
        .get(unit)
        .ok_or(UnitError::UnknownUnit(unit))?
        .mode;
    let f = monster_record(sim, hooks, unit, mode)
        .map(|r| if end_anim { r.event1 } else { r.event0 })
        .filter(|&a| a != 0)
        .unwrap_or(MONSTER_MODE_FALLBACK);
    hooks.monster_mode_function(sim, unit, f);
    Ok(())
}

// ---- missiles -----------------------------------------------------------

/// Missile setup: cancel all the missile's timers, then the
/// every-tick type-0 event.
pub fn missile_init(sim: &mut Sim<'_>, unit: UnitId) -> Result<(), UnitError> {
    sim.game.timers.cancel_unit_timers(unit);
    let owner = sim.game.owner(unit)?;
    sim.game
        .timers
        .schedule_every_tick(owner, u32::from(event::MODE_CHANGE), None, 0, 0)
        .map_err(GameError::from)?;
    Ok(())
}

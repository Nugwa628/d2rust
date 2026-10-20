//! The skill timer events of the unit dispatch on the skill use
//! pipeline: event 5 (active state), 8 (periodic skills and auras) and 9
//! (item auras) reach [`crate::skills::use_`] through [`UseView`].
//!
//! The unit dispatch does the checks it owns (skill id range,
//! `srvactivefunc` < 191, item-aura level from stat 151, the type-9
//! cancel) and calls the action hooks, which hand the event to
//! [`Pending::skill_event`]. A seam value that also implements
//! [`UseRest`] routes it here:
//!
//! ```text
//! fn skill_event(h: &mut ActionHooks<Self>, sim: &mut Sim<'_>, ev: SkillEvent) {
//!     d2_sim::wiring::interaction::skill_events::route(h, sim, ev)
//! }
//! ```
//!
//! Player event 0 in an attack, cast or skill mode (the action frame)
//! reaches [`crate::skills::use_::attack_frame_event`] the same way,
//! through [`Pending::action_frame`] and [`action_frame`].
//!
//! Event 14 is not routed: it is never scheduled in the game and its body
//! is not known.

use crate::combat::apply_melee;
use crate::skills::levels::skill_level;
use crate::skills::use_::bodies::b4_mon::monster_mode_missile;
use crate::skills::use_::bodies::{self, melee_setup, mode_damage, BodyWorld};
use crate::skills::use_::{
    active_state_event, attack_frame_event, do_skill, item_aura_event, periodic_event, start,
    SkillFunctions, UseWorld, FLAG_MISSILE_FIRED, SKILL_ARRIVED, SKILL_MOVING,
};
use crate::skills::SkillUnits;
use crate::units::hooks::Sim;
use crate::units::{UnitId, UnitType};
use crate::wiring::action::combat::CombatView;
use crate::wiring::action::{ActionHooks, Pending, SkillEvent, View};

use super::{UseRest, UseView};

/// Runs one skill timer event on the skill use pipeline.
pub fn route<X: Pending + UseRest>(h: &mut ActionHooks<X>, sim: &mut Sim<'_>, ev: SkillEvent) {
    let t = h.tables.clone();
    let mut w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    match ev {
        // The active-state function is called with (game, unit, skill, a2).
        // TODO: a2 is read as the do function's level argument (the
        // game's use of it is not known).
        SkillEvent::ActiveState {
            unit,
            f,
            skill,
            arg2,
        } => {
            active_state_event(&mut w, unit, f, skill as i32, arg2 as i32);
        }
        // Arg −1 (the aura form) or a skill id; the level of the second
        // form comes from stat 351, not from a2.
        SkillEvent::Periodic { unit, arg1, .. } => {
            periodic_event(&mut w, &t.skills, unit, arg1 as i32);
        }
        // The item-aura do runs with (game, unit, skill, level, 1, 1, 0).
        //
        // TODO: the game also makes a second call (game, unit, a1, skill,
        // level, 0) whose body is not known; it is not run, so nothing
        // reschedules the type-9 event.
        SkillEvent::ItemAura {
            unit, skill, level, ..
        } => {
            item_aura_event(&mut w, &t.skills, unit, skill as i32, level);
        }
        // The freed list's callback, run by the skill bodies' dispatcher.
        SkillEvent::ListRemoved {
            unit,
            state,
            cb,
            list,
        } => {
            bodies::remove_callback(&mut w, &t.skills, &t.combat, unit, state as i32, cb, list);
        }
    }
}

/// The player action frame on the skill use pipeline: the used skill's
/// do function on an action event (a1 1 or 2); returns 1, or 2 when the
/// unit died.
pub fn action_frame<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    unit: UnitId,
    a1: u32,
    a2: u32,
) -> u32 {
    let t = h.tables.clone();
    let mut w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    attack_frame_event(&mut w, &t.skills, unit, a1 as i32, a2 as i32) as u32
}

/// The skill start of a monster's attack / skill and sequence starts on
/// the skill use pipeline; its result.
pub fn monster_skill_start<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    unit: UnitId,
) -> i32 {
    let t = h.tables.clone();
    let mut w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    start(&mut w, &t.skills, unit)
}

/// `apply_state` for the monster umod 7 curse.
pub fn apply_state_req<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    req: crate::skills::use_::bodies::helpers::StateRequest<UnitId>,
) -> Option<crate::stats::ListId> {
    let ct = h.tables.combat.clone();
    let mut w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    crate::skills::use_::bodies::helpers::apply_state(&mut w, &ct, req)
}

/// A monster's aura as its right skill (monster umod 30, Duriel): the
/// skill is given at `level` (the entry's base level), then assigned to
/// the right hand: the aura state or the immediate do, and the type-8
/// aura timer whose do rolls the aura's damage on the monster's seed
/// every `perdelay` frames (Duriel's Holy Freeze at frames 51, 101, 151
/// of gen-lvl-73).
pub fn monster_right_aura<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    unit: UnitId,
    skill: i32,
    level: i32,
) {
    if sim.units.get(unit).map(|r| r.ty) != Some(UnitType::Monster) {
        return;
    }
    h.monster_skills
        .entry(unit)
        .or_default()
        .insert(skill, level);
    let mut w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    w.monster_aura_select(unit, skill);
}

/// The refresh of a monster's skill entry (the Shadow Master's class
/// skills are each refreshed on init): a passive skill's state list on
/// the monster (Claw Mastery, Weapon Block ... in the Shadow Master's
/// 0xAA).
pub fn monster_passive_refresh<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    unit: UnitId,
    skill: i32,
    level: i32,
) {
    if sim.units.get(unit).map(|r| r.ty) != Some(UnitType::Monster) {
        return;
    }
    let t = h.tables.clone();
    let mut w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    // First switches the skill's passive state on, then the entry is
    // refreshed.
    let p = t
        .skills
        .skill(skill)
        .map_or(0, |r| i32::from(r.passivestate as i16));
    if p > 0 && p < BodyWorld::state_count(&w) {
        BodyWorld::state_on(&mut w, unit, p, true);
    }
    crate::skills::use_::bodies::passive::refresh_at(&mut w, &t.skills, unit, skill, level);
}

/// The right skill's aura part of the save load's assign: the loaded
/// right skill is an `aura` skill → `immediate` (Might, Resist Fire, ...)
/// runs the do core once, else its aura state is switched on (stats 350 /
/// 351); then the aura form of `schedule_periodic`. The load selects the
/// hand before the unit has a room, so the first periodic do runs on the
/// first tick after game entry.
///
/// This is the join's post-load step, after the player's 0xAA
/// state message: the aura's stats are in effect for
/// the join's later messages (the first 0x95 life/mana update), but
/// the 0xAA never lists its state, which the first update sends as 0xA8.
// D2RUST sends the player's 0xAA at game entry, after the load, so the aura state's
// unit bit is switched off here (its list and its changed bit stay) and
// [`passive_states_on`] switches it back on after the join messages: the
// join order: the player's 0xAA goes out first, then the assign starts
// Might, whose apply-state sets state 33
// (check `w3-05-join-aura`).
pub fn assign_right_aura<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    unit: UnitId,
) {
    let t = h.tables.clone();
    let mut w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    let Some(e) = w.right_skill(unit) else {
        return;
    };
    let Some(r) = t.skills.skill(e.skill).filter(|r| r.aura) else {
        return;
    };
    let l = skill_level(&w, &t.skills, Some(unit), Some(&e), true);
    if l == 0 {
        return;
    }
    if r.immediate {
        crate::skills::use_::do_core(&mut w, &t.skills, unit, e.skill, l, true, false, false);
    } else {
        w.set_aura_state(unit, r.aurastate, e.skill, l);
    }
    crate::skills::use_::schedule_periodic(&mut w, &t.skills, unit, e.skill, l, true);
    if let Ok(s) = u16::try_from(r.aurastate as i16) {
        if w.cv.v.state_list(unit, s).is_some() && w.cv.v.stats.has_state(unit, u32::from(s)) {
            w.cv.v.set_state(unit, s, false);
        }
    }
}

/// The skill part of the monster sequence event 0 on the skill use
/// pipeline: E := the used skill, f := its E flags (0 without a used
/// skill), "do left" := 1. f bit 0 (a moving skill): the target check and
/// step (the path provider's step when it is on); result 2 → E flags :=
/// f | 2, the do, do left := 0. Then by the frame code (the action frame
/// of the unit's animation): 4 → the do; else do left, unit flag 0x40
/// clear and code 1 or 2 → the do. The animation refresh that follows is
/// the caller's.
pub fn monster_sequence_frame<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    unit: UnitId,
) {
    let t = h.tables.clone();
    let paths = h.paths.is_some();
    let mut w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    let do_it = |w: &mut UseView<'_, X>| {
        if let Some(e) = w.used_skill(unit) {
            let l = skill_level(w, &t.skills, Some(unit), Some(&e), true);
            do_skill(w, &t.skills, unit, e.skill, l);
        }
    };
    let f = if w.used_skill(unit).is_some() {
        w.used_skill_flags(unit)
    } else {
        0
    };
    let mut do_left = true;
    if f & SKILL_MOVING != 0 {
        let stopped = if paths {
            let game = &mut *w.cv.game;
            crate::wiring::path::PathCtx::of(&mut w.cv.v, game).step(unit)
                == Some(crate::path::walk::Step::Stopped)
        } else {
            w.step_path(unit) == 2
        };
        if stopped {
            w.set_used_skill_flags(unit, f | SKILL_ARRIVED);
            do_it(&mut w);
            do_left = false;
        }
    }
    let (code, flags) =
        w.cv.v
            .units
            .get(unit)
            .map_or((0, 0), |r| (r.anim.action_frame, r.flags));
    if code == 4 || (do_left && flags & FLAG_MISSILE_FIRED == 0 && matches!(code, 1 | 2)) {
        do_it(&mut w);
    }
}

/// trigger(U) of the monster attack event: U's mode is 14 ? unit flag
/// 0x40 set : frame code = 1.
pub fn monster_trigger(units: &crate::units::record::Units, unit: UnitId) -> bool {
    units.get(unit).is_some_and(|r| {
        if r.mode == 14 {
            r.flags & FLAG_MISSILE_FIRED != 0
        } else {
            r.anim.action_frame == 1
        }
    })
}

/// The used-skill branch of the monster attack-family event 0 on the
/// skill use pipeline: E flags bit 0 (a moving skill): the target check
/// and step; stopped → E flags |= 2, the do, and a second do when
/// trigger(U); done. Otherwise (and for a moving skill still on its way)
/// the do, on every event: no frame-code test. The animation refresh that
/// follows is the caller's.
pub fn monster_attack_skill<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    unit: UnitId,
) {
    let t = h.tables.clone();
    let mut w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    let do_it = |w: &mut UseView<'_, X>| {
        if let Some(e) = w.used_skill(unit) {
            let l = skill_level(w, &t.skills, Some(unit), Some(&e), true);
            do_skill(w, &t.skills, unit, e.skill, l);
        }
    };
    if w.used_skill(unit).is_none() {
        return;
    }
    let f = w.used_skill_flags(unit);
    if f & SKILL_MOVING != 0 && w.step_path(unit) == 2 {
        w.set_used_skill_flags(unit, f | SKILL_ARRIVED);
        do_it(&mut w);
        // trigger(U) is read after the first do (which may set flag 0x40).
        if monster_trigger(w.cv.v.units, unit) {
            do_it(&mut w);
        }
        return;
    }
    do_it(&mut w);
}

/// The strike of the monster attack-family event 0 with no used skill:
/// the mode missile (its argument the moving flag); none → the melee
/// set-up and `apply_melee` on the path target unit, when there is one.
pub fn monster_attack_strike<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    unit: UnitId,
    moving: bool,
) {
    let t = h.tables.clone();
    let ct = h.tables.combat.clone();
    let mut w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    if monster_mode_missile(&mut w, &ct, unit, moving) != 0 {
        return;
    }
    // The melee set-up does nothing without the path target (`gen-su-56`
    // frame 141: the suicide minion's mode-8 start cleared its target,
    // its strike event makes no hit draw).
    if let Some(tg) = crate::wiring::path::monsters::path_target(&*w.cv.v.h, unit) {
        melee_setup(&mut w, &t.skills, &ct, unit);
        apply_melee(w.combat(), &ct, unit, tg);
    }
}

/// The monster mode damage for `mode` (the mode set), on the unit's base
/// list (flag 1).
pub fn monster_mode_damage<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    unit: UnitId,
    mode: u32,
) {
    let t = h.tables.clone();
    let ct = h.tables.combat.clone();
    let mut w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    mode_damage(&mut w, &t.skills, &ct, unit, mode as i32);
}

/// The join's Iron Golem re-summon: the player's skill 90 level L (base +
/// bonuses) and the golem spawned at the player. The saved item has no
/// unit yet, so the body runs without it. True when a golem was made.
/// D2RUST's own rule, not yet checked in play.
pub fn golem_resummon<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    player: UnitId,
) -> bool {
    const IRON_GOLEM: i32 = 90;
    let t = h.tables.clone();
    let Some(entry) = h
        .skill_lists
        .get(&player)
        .and_then(|l| l.view().into_iter().find(|e| e.skill == IRON_GOLEM))
    else {
        return false;
    };
    let ct = h.tables.combat.clone();
    let mut w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    let l = skill_level(&w, &t.skills, Some(player), Some(&entry), true);
    crate::skills::use_::bodies::b3_lvl24::golem_summon(
        &mut w, &t.skills, &ct, player, IRON_GOLEM, l, None,
    ) == 1
}

/// The save load's skill section: every loaded entry whose skill has a
/// `passivestate` p > 0 gets state p on and its passive stat list
/// refreshed (the state-changed bit only), in list order (the masteries,
/// Increased Stamina, Iron Skin, ... count from the join on).
pub fn passive_refresh_all<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    unit: UnitId,
) {
    let t = h.tables.clone();
    let entries = h
        .skill_lists
        .get(&unit)
        .map(|l| l.view())
        .unwrap_or_default();
    let mut w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    for e in entries {
        let p = t
            .skills
            .skill(e.skill)
            .map_or(-1, |r| i32::from(r.passivestate as i16));
        if let Ok(p @ 1..) = u16::try_from(p) {
            // The refresh sets only the state-changed bit, not the unit's
            // state bit, so the join's 0xAA does not list the state and
            // the first player update sends it as 0xA8 (`gen-missile-149`,
            // frame 2).
            w.cv.v.stats.set_state_changed(unit, u32::from(p), true);
            let _ = w.cv.game.lists.queue_update(unit);
            crate::skills::use_::bodies::BodyWorld::passive_state_apply(&mut w, unit, &e);
        }
    }
}

/// The passive states' unit bits (state on `(unit, p, 1)`) for every
/// skill entry with a `passivestate` p > 0, at the end of the join: the
/// skill load sets only the state-changed bit, so the join's 0xAA
/// omits the states and the first player update sends them as 0xA8 with
/// their lists.
// The bits go on in the join right after the player's 0xAA
// (a Barbarian with every skill at 20: the 0xAA, then ten
// state-ons, states 74-83, from the skill add,
// before the other units' messages; check
// `w3-05-join-passive`). The end of the join is the D2RUST
// place for the same order, equal on the wire.
pub fn passive_states_on<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    unit: UnitId,
) {
    let t = h.tables.clone();
    let entries = h
        .skill_lists
        .get(&unit)
        .map(|l| l.view())
        .unwrap_or_default();
    let mut w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    for e in entries {
        let p = t
            .skills
            .skill(e.skill)
            .map_or(-1, |r| i32::from(r.passivestate as i16));
        if let Ok(p @ 1..) = u16::try_from(p) {
            w.cv.v.set_state(unit, p, true);
        }
    }
    // The right aura's state hidden by `assign_right_aura`.
    let aura = UseWorld::right_skill(&w, unit)
        .and_then(|e| t.skills.skill(e.skill))
        .filter(|r| r.aura)
        .and_then(|r| u16::try_from(r.aurastate as i16).ok());
    if let Some(s) = aura {
        if w.cv.v.state_list(unit, s).is_some() && !w.cv.v.stats.has_state(unit, u32::from(s)) {
            w.cv.v.set_state(unit, s, true);
        }
    }
}

/// The save load's right-skill selection: the selected right skill's
/// aura start ([`crate::skills::use_::right_aura_start`]), so an aura
/// saved on the right button is on from the join (`pal-vigor` /
/// `pal-fanaticism` checks: the aura's stats at frame 2).
pub fn right_aura_select<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    player: UnitId,
) {
    let t = h.tables.clone();
    let mut w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    let Some(e) = UseWorld::right_skill(&w, player) else {
        return;
    };
    let l = skill_level(&w, &t.skills, Some(player), Some(&e), true);
    crate::skills::use_::right_aura_start(&mut w, &t.skills, player, &e, l);
}

/// The Bone Wall maker's `summon_class` for `owner` on the skill
/// pipeline.
pub fn missile_summon_class<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    owner: UnitId,
    skill: i32,
    _level: i32,
) -> (i32, i32) {
    let t = h.tables.clone();
    let ct = h.tables.combat.clone();
    let w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    crate::skills::use_::bodies::summon_class(&w, &t.skills, &ct, owner, skill)
}

/// The skill stat callbacks queued by the stat write that just returned
/// (each stat write's refreshes run here, after the write, rather than
/// inside it), each as [`skill_stat_refresh`], in their
/// order. Callbacks the refreshes themselves raise stay queued for the
/// next drain.
pub fn drain_skill_refresh<X: Pending + UseRest>(h: &mut ActionHooks<X>, sim: &mut Sim<'_>) {
    for (u, stat, layer) in std::mem::take(&mut h.skill_refresh) {
        skill_stat_refresh(h, sim, u, stat, layer);
    }
}

/// The skill handlers of stats 83, 126, 127 and 188: refresh all, every
/// skill entry with a `passivestate` p > 0 in list order: state p on,
/// then the passive refresh. Stats 83 / 188: only for a player of the
/// layer's class c, a hireling or a unit whose draw identity is a player
/// of class c.
pub fn skill_stat_refresh<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    unit: UnitId,
    stat: u16,
    layer: u16,
) {
    if matches!(stat, 83 | 188) {
        let c = u32::from(if stat == 83 { layer } else { layer >> 3 });
        let own = sim.units.get(unit).is_some_and(|r| {
            (r.ty == UnitType::Player && r.class == c)
                || (r.ty == UnitType::Monster
                    && crate::wiring::action::combat::HIRELING_CLASSES.contains(&r.class))
        });
        let drawn = || {
            h.draw_identity(sim, unit)
                .is_some_and(|(ty, class, _)| ty == UnitType::Player && class == c)
        };
        if !own && !drawn() {
            return;
        }
    }
    let t = h.tables.clone();
    let entries = h
        .skill_lists
        .get(&unit)
        .map(|l| l.view())
        .unwrap_or_default();
    let mut w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    for e in entries {
        let p = t
            .skills
            .skill(e.skill)
            .map_or(-1, |r| i32::from(r.passivestate as i16));
        if p > 0 {
            crate::skills::use_::bodies::BodyWorld::state_on(&mut w, unit, p, true);
            crate::skills::use_::bodies::BodyWorld::passive_state_apply(&mut w, unit, &e);
        }
    }
}

/// The missile area bodies' scan: the units `scan_unit(game, owner, x, y,
/// r, f, …, noaura)` accepts, in scan order, for the per-unit area hit
/// the missile body then runs on each.
/// PROVISIONAL: the units are gathered before the first hit is applied;
/// the game hits each in the scan callback (measured: run-queue
/// `w3-05-area-scan`, `probe_calls.py` on `w1-w7-sor-fireball-pack`,
/// probe frame 27: inside one scan each acceptance test that accepts is
/// followed by the hit on that unit before the next unit's test, three
/// times; no test after a hit refused a unit). The game's scan reads the
/// next unit of the room list before the callback and runs the
/// acceptance test on each unit at its turn, after the earlier units'
/// hits; that test reads only the candidate (type, mode 0 / 12, unit
/// flags bits 2 / 3, its room, its position) and the owner, so the two
/// orders differ only when a hit changes a later unit's mode, flags,
/// room or position, or frees it. Measured equal:
/// `w1-w7-sor-fireball-pack` (three Fallen die in one frame, f30;
/// packets and state equal). Left undone: doing the hit inside the scan
/// callback (the area loop of `missiles/bodies.rs` `area_damage` over
/// `MissileWorld::area_units`, outside this file).
pub fn missile_area_units<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    owner: UnitId,
    at: (i32, i32),
    r: i32,
    f: u32,
    noaura: bool,
) -> Vec<UnitId> {
    let t = h.tables.clone();
    let ct = h.tables.combat.clone();
    let mut w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    let mut out = Vec::new();
    crate::skills::use_::bodies::scan_unit(
        &mut w,
        &t.skills,
        &ct,
        owner,
        at,
        r,
        f,
        noaura,
        &mut |_, u| {
            out.push(u);
            1
        },
    );
    out
}

/// The ally test for the missile bodies.
pub fn missile_ally_test<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    a: UnitId,
    b: UnitId,
) -> bool {
    let w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    BodyWorld::allied(&w, a, b)
}

/// The shout missile's hit: `shout_state` on `unit` from `owner`.
pub fn missile_shout_state<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    unit: UnitId,
    owner: UnitId,
    skill: i32,
    level: i32,
) {
    let t = h.tables.clone();
    let mut w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    crate::skills::use_::bodies::shout_state(&mut w, &t.skills, unit, owner, skill, level);
}

/// Terror install for a missile hit.
pub fn missile_terror<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    source: UnitId,
    unit: UnitId,
    b: i32,
) {
    let t = h.tables.clone();
    let mut w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    crate::skills::use_::bodies::dos::terror_install(&mut w, &t.combat, source, unit, b);
}

/// Skill server-do `index` with a missile as the caster: entry `index`
/// of the server-do table when `index` ≤ 190 (unsigned), as (game,
/// caster, skill, level); the result is ignored. Grim Ward's ward calls
/// entry 30, Curse, every 6 frames.
pub fn missile_skill_srv_do<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    caster: UnitId,
    index: i32,
    skill: i32,
    level: i32,
) {
    let Ok(index) = u16::try_from(index) else {
        return;
    };
    if index > 190 {
        return;
    }
    let mut w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    SkillFunctions::srvdo(&mut w, index, caster, skill, level, false, false, false);
}

/// The Bone Wall maker's summon spawn: `summon_spawn` with flags 0xD, AI
/// special state 0, pet max 0.
#[allow(clippy::too_many_arguments)]
pub fn missile_summon_spawn<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    owner: UnitId,
    class: i32,
    mode: i32,
    at: (i32, i32),
    pet_type: i32,
) -> Option<UnitId> {
    let mut w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    crate::skills::use_::bodies::spawn(
        &mut w,
        crate::skills::use_::bodies::Summon {
            flags: 0xD,
            owner,
            class,
            ai: 0,
            mode,
            x: at.0,
            y: at.1,
            pet_type,
            pet_max: 0,
        },
    )
}

/// The Bone Wall maker's piece binding, in order: owner data (anchor, 0,
/// 0), the anchor's minion list, umod 15, `skill_stats` (ilvl 0), the
/// source-unit link to `owner`, target-node slot 9.
#[allow(clippy::too_many_arguments)]
pub fn missile_bone_wall_piece<X: Pending + UseRest>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    owner: UnitId,
    anchor: UnitId,
    piece: UnitId,
    skill: i32,
    level: i32,
) {
    use crate::skills::use_::bodies::{self as b, BodyEffect};
    let t = h.tables.clone();
    let mut w = UseView {
        cv: CombatView {
            game: sim.game,
            v: View::of(sim.units, sim.stats, sim.data, h),
        },
    };
    BodyWorld::effect(
        &mut w,
        BodyEffect::OwnerData {
            m: piece,
            owner: Some(anchor),
            a: 0,
            b: 0,
        },
    );
    BodyWorld::effect(
        &mut w,
        BodyEffect::AddMinion {
            leader: anchor,
            m: piece,
        },
    );
    BodyWorld::effect(
        &mut w,
        BodyEffect::Umod {
            m: piece,
            umod: 15,
            arg: 0,
        },
    );
    b::skill_stats(&mut w, &t.skills, owner, piece, skill, level, 0);
    b::link_source(&mut w, piece, Some(owner));
    b::node_prepend(&mut w, piece, 9);
}

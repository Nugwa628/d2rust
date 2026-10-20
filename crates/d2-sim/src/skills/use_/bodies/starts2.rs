//! The start functions of batch 2: the shared slots of several
//! class skills. Each returns the value the start core reads.

use super::helpers::*;
use super::helpers2::*;
use super::helpers3::next_unit;
use super::BodyWorld;
use crate::combat::{start_combat, CombatTables, CombatWorld, RoomKind};
use crate::skills::{roll_elemental, SkillTables};
use crate::units::UnitType;

/// The corpse test: a monster in mode 12 with no
/// `udead` state and monstats2 `corpseSel`.
pub fn corpse_any<W: BodyWorld>(w: &W, ct: &CombatTables, tg: W::Unit) -> bool {
    w.unit_type(tg) == UnitType::Monster
        && w.mode(tg) == MODE_DEAD
        && !w.has_group(tg, group::UDEAD)
        && ct.monstats2(w.class_id(tg)).is_some_and(|m| m.corpsesel)
}

/// The raise corpse test: the
/// corpse test with monstats `Velocity` ≠ 0.
pub fn corpse_raise<W: BodyWorld>(w: &W, ct: &CombatTables, tg: W::Unit) -> bool {
    corpse_any(w, ct, tg) && ct.monstats(w.class_id(tg)).is_some_and(|m| m.velocity != 0)
}

/// The corpse test: with monstats2
/// `soft`.
pub fn corpse_soft<W: BodyWorld>(w: &W, ct: &CombatTables, tg: W::Unit) -> bool {
    corpse_any(w, ct, tg) && ct.monstats2(w.class_id(tg)).is_some_and(|m| m.soft)
}

/// srvst 23 Tiger Strike, ….
pub fn charge_strike<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    let Some(tg) = target(w, u) else {
        return 0;
    };
    i32::from(w.combat().in_melee_range(u, tg, 0))
}

/// srvst 6 Power Strike, Charged Strike.
pub fn power_strike<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let (calc1, srcdam) = (r.calc1, i32::from(r.srcdam));
    let mut record = melee_rec(w, t, ct, u, tg, 0, 0);
    if record.result & 1 != 0 {
        record.enh_pct = eval(w, t, u, calc1, skill, lvl);
        convert(w, t, u, &mut record, skill, lvl);
        roll_elemental(w, t, u, &mut record, skill, lvl);
    }
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, srcdam);
    1
}

/// srvst 11 Inferno, Arctic Blast.
pub fn inferno<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let (sm, m) = (i32::from(r.startmana), s16(r.srvmissilea));
    if !w.has_state(u, st::INFERNO as u16) && w.stat(u, sid::MANA, 0) < sm.wrapping_shl(8) {
        return 0;
    }
    if !missile_ok(t, m) {
        return 0;
    }
    inferno_start(w, t, ct, u, skill, lvl, m)
}

/// srvst 12 Telekinesis, Dragon Flight.
pub fn telekinesis<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let range = r.aurarangecalc;
    let r = eval(w, t, u, range, skill, lvl);
    if dist_sq(w.position(u), w.position(tg)) > r.wrapping_mul(r) {
        return 0;
    }
    if matches!(w.unit_type(tg), UnitType::Player | UnitType::Monster)
        && (!w.combat().hostile(u, tg)
            || w.room(tg) == RoomKind::Town
            || w.room(u) == RoomKind::Town)
    {
        return 0;
    }
    1
}

/// srvst 17 Corpse Explosion, Poison Explosion.
pub fn corpse_explosion<W: BodyWorld>(w: &mut W, ct: &CombatTables, u: W::Unit) -> i32 {
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if w.room(tg) == RoomKind::Town {
        return 0;
    }
    i32::from(corpse_any(w, ct, tg))
}

/// srvst 37 Zeal, Fury.
pub fn zeal<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let calc1 = r.calc1;
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    let r = w.combat().melee_range(u).wrapping_add(4);
    let tg = match target(w, u) {
        Some(x) => Some(x),
        None => next_unit(w, t, ct, u, (0, 0), r, 0x20003, u32::MAX).0,
    };
    let Some(tg) = tg else {
        w.set_entry_param_of(u, &e, 1, 0);
        return 0;
    };
    let n = eval(w, t, u, calc1, skill, lvl);
    w.set_entry_param_of(u, &e, 1, n);
    let (ty, g) = (type_index(w, tg), guid(w, tg));
    w.set_entry_param_of(u, &e, 2, ty);
    w.set_entry_param_of(u, &e, 3, g as i32);
    1
}

/// srvst 56 Feral Rage, Maul.
pub fn feral_rage<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let (calc1, s) = (r.calc1, srcdam_or_128(r));
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let mut record = skill_melee(w, t, ct, u, tg, skill, lvl);
    if record.result & 1 != 0 {
        convert(w, t, u, &mut record, skill, lvl);
        record.enh_pct = eval(w, t, u, calc1, skill, lvl);
    }
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, s);
    let hit = record.result & 1 != 0;
    w.set_entry_param_of(u, &e, 1, i32::from(hit));
    1
}

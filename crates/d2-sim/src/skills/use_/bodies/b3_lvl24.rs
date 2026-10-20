//! Batch 3 bodies of required level 24: Strafe, Dopplezon, Fend,
//! Thunder Storm, Attract, Bone Prison, Iron Golem, Conversion, Holy
//! Shield, Frenzy, Grim Ward, Hunger, Volcano, Mind Blast, Dragon
//! Flight.

use super::dos::can_switch;
use super::effects::{BodyEffect, PathOp};
use super::helpers::*;
use super::helpers2::*;
use super::helpers3::*;
use super::starts2::corpse_soft;
use super::{callback, init_cb, BodyWorld, MissileRequest};
use crate::combat::{
    apply, apply_melee, fill, pct, start_combat, CombatTables, CombatWorld, DamageRecord, RoomKind,
};
use crate::skills::{dm, roll_elemental, roll_physical, SkillTables};
use crate::units::UnitType;

// -----------------------------------------------------------------

/// srvst 8 Strafe.
pub fn strafe_start<W: BodyWorld>(
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
    let (range, calc1, calc3) = (r.aurarangecalc, r.calc1, r.calc3);
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    if !has_ammo(w, u) {
        w.attack_cleanup(u);
        w.weapon_cleanup(u);
        return 0;
    }
    let rr = eval(w, t, u, range, skill, lvl);
    let (tg, count) = match target(w, u) {
        None => next_unit(w, t, ct, u, (0, 0), rr, 3, u32::MAX),
        Some(k) => (Some(k), count_units(w, t, ct, u, (0, 0), rr, 3)),
    };
    let c3 = eval(w, t, u, calc3, skill, lvl);
    let c1 = eval(w, t, u, calc1, skill, lvl);
    let lo = c3.min(c1);
    let n = strafe_count(count, lo, c1);
    w.set_entry_param_of(u, &e, 1, n);
    match tg {
        Some(k) => {
            let (ty, g) = (type_index(w, k), guid(w, k));
            w.set_entry_param_of(u, &e, 2, ty);
            w.set_entry_param_of(u, &e, 3, g as i32);
        }
        None => {
            w.set_entry_param_of(u, &e, 2, 1);
            w.set_entry_param_of(u, &e, 3, -1);
        }
    }
    dec_quantity(w, u);
    1
}

/// The Strafe arrow count: count clamped to lo…hi.
pub fn strafe_count(count: i32, lo: i32, hi: i32) -> i32 {
    if count < lo {
        lo
    } else if count > hi {
        hi
    } else {
        count
    }
}

/// srvdo 12 Strafe.
pub fn strafe<W: BodyWorld>(
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
    let (range, calc2) = (r.aurarangecalc, r.calc2);
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    let rr = eval(w, t, u, range, skill, lvl);
    let n = w.entry_param(u, &e, 1);
    if n == 0 {
        return 0;
    }
    let (p2, p3) = (w.entry_param(u, &e, 2), w.entry_param(u, &e, 3));
    let k = match w.find_unit(p2 as u32, p3 as u32) {
        Some(k) => Some(k),
        None => next_unit(w, t, ct, u, (0, 0), rr, 3, p3 as u32).0,
    };
    let Some(k) = k else {
        return 1;
    };
    w.path_op(u, PathOp::TargetUnit(Some(k)));
    // The result is not tested.
    let (tx, ty) = w.target_position(u).unwrap_or((0, 0));
    let m = super::dos2::arrow_missile(w, t, u, skill);
    if !missile_ok(t, m) {
        return 0;
    }
    let a = eval(w, t, u, calc2, skill, lvl);
    let (x, y) = w.position(u);
    w.spawn_missile(MissileRequest {
        flags: 0x21,
        x,
        y,
        target_x: tx,
        target_y: ty,
        skill,
        level: lvl,
        init: Some((init_cb::DAMAGE_PERCENT, a as u32)),
        ..MissileRequest::new(u, m)
    });
    let n = n.wrapping_sub(1);
    w.set_entry_param_of(u, &e, 1, n);
    if n <= 0 {
        // Edge case 24.
        return 0;
    }
    let g = guid(w, k);
    // The rewind runs only when K2 exists; the body returns 1 either way.
    if let Some(k2) = next_unit(w, t, ct, u, (0, 0), rr, 3, g).0 {
        let (ty, g2) = (type_index(w, k2), guid(w, k2));
        w.set_entry_param_of(u, &e, 2, ty);
        w.set_entry_param_of(u, &e, 3, g2 as i32);
        w.anim_rewind(u, param(t, skill, 6));
    }
    1
}

// -----------------------------------------------------------------

/// srvdo 15 Dopplezon.
pub fn dopplezon<W: BodyWorld>(
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
    let (pet, petmax, calc2, calc3) = (i32::from(r.pettype), r.petmax, r.calc2, r.calc3);
    let (c, mode) = summon_class(w, t, ct, u, skill);
    if c < 0 {
        return 0;
    }
    // Edge case 25: kept as 0.
    let pt = if pet >= w.pettype_count() { 0 } else { pet };
    flag_40(w, u);
    let pm = eval(w, t, u, petmax, skill, lvl);
    let Some(m) = spawn(
        w,
        Summon {
            flags: 0,
            owner: u,
            class: c,
            ai: 0,
            mode,
            x: 0,
            y: 0,
            pet_type: pt,
            pet_max: pm,
        },
    ) else {
        return 0;
    };
    link_source(w, m, Some(u));
    let h = pct(
        w.stat_max(u, sid::LIFE),
        eval(w, t, u, calc3, skill, lvl),
        100,
    );
    w.set_stat(m, sid::LIFE, h);
    w.set_stat(m, sid::MAXHP, h);
    let ul = w.stat(u, sid::LEVEL, 0);
    w.set_stat(m, sid::LEVEL, ul);
    base_stats(w, u, m, 0, lvl);
    skill_stats(w, t, u, m, skill, lvl, 0);
    // The summon lives `calc2` base ticks.
    let d = eval(w, t, u, calc2, skill, lvl);
    w.schedule(m, 7, super::after(w, d), 0, 0);
    w.effect(BodyEffect::Umod {
        m,
        umod: 21,
        arg: 0,
    });
    w.combat().overlay(m, 171);
    node_insert_owner(w, m, u);
    1
}

// -----------------------------------------------------------------

/// srvst 9 Fend.
pub fn fend_start<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(calc1) = rec(t, skill).map(|r| r.calc1) else {
        return 0;
    };
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    let r = w.combat().melee_range(u).wrapping_add(4);
    let (tg, count) = match target(w, u) {
        None => match next_unit(w, t, ct, u, (0, 0), r, 0x20003, u32::MAX) {
            (Some(k), c) => (k, c),
            (None, _) => {
                w.set_entry_param_of(u, &e, 1, 0);
                return 1;
            }
        },
        Some(k) => (k, count_units(w, t, ct, u, (0, 0), r, 0x20003)),
    };
    let c1 = eval(w, t, u, calc1, skill, lvl);
    w.set_entry_param_of(u, &e, 1, count.min(c1));
    let (ty, g) = (type_index(w, tg), guid(w, tg));
    w.set_entry_param_of(u, &e, 2, ty);
    w.set_entry_param_of(u, &e, 3, g as i32);
    1
}

// -----------------------------------------------------------------

/// srvst 13 Thunder Storm.
pub fn thunder_storm_start<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
) -> i32 {
    if rec(t, skill).is_none() {
        return 0;
    }
    let Some(e) = w.find_entry(u, skill) else {
        return 0;
    };
    w.set_entry_param_of(u, &e, 1, -1);
    w.set_entry_param_of(u, &e, 2, 1);
    1
}

/// srvdo 29 Thunder Storm.
pub fn thunder_storm<W: BodyWorld>(
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
    let (m, a, len) = (s16(r.srvmissilea), s16(r.aurastate), r.auralencalc);
    if !missile_ok(t, m) || !state_ok(w, a) {
        return 0;
    }
    let Some(e) = w.find_entry(u, skill) else {
        return 0;
    };
    if !w.has_state(u, a as u16) || w.entry_param(u, &e, 2) != 0 {
        let d = eval(w, t, u, len, skill, lvl);
        if let Some(l) = apply_state(
            w,
            ct,
            StateRequest {
                source: u,
                target: u,
                skill,
                level: lvl,
                duration: d,
                stat: -1,
                value: 0,
                state: a,
                callback: callback::DEFAULT,
            },
        ) {
            w.list_set(l, stat::MODIFIERLIST_SKILL, skill);
            w.list_set(l, stat::MODIFIERLIST_LEVEL, lvl);
        }
    } else {
        let p1 = w.entry_param(u, &e, 1);
        let k = next_unit(w, t, ct, u, (0, 0), param(t, skill, 7), 3, p1 as u32).0;
        match k {
            None => w.set_entry_param_of(u, &e, 1, -1),
            Some(k) => {
                if w.room(u) != RoomKind::Town && w.room(k) != RoomKind::Town {
                    let at = w.position(k);
                    if let Some(mm) = missile_at(w, u, skill, lvl, m, at) {
                        w.effect(BodyEffect::MissileHit {
                            missile: mm,
                            unit: k,
                        });
                        w.effect(BodyEffect::RemoveUnit(mm));
                        w.effect(BodyEffect::MsgA3 {
                            u,
                            target: Some(k),
                            skill,
                            lvl,
                            x: 0,
                            y: 0,
                            v: 0,
                        });
                    }
                }
                let g = guid(w, k);
                w.set_entry_param_of(u, &e, 1, g as i32);
            }
        }
    }
    w.set_entry_param_of(u, &e, 2, 0);
    1
}

// -----------------------------------------------------------------

/// Attract test.
fn attract_test<W: BodyWorld>(w: &mut W, ct: &CombatTables, u: W::Unit, x: W::Unit) -> bool {
    w.combat().alignment(x) == 0
        && can_switch(w, ct, x, 19)
        && w.is_alive(x)
        && w.room(x) != RoomKind::Town
        && w.combat().hostile(u, x)
}

/// srvdo 59 Attract.
pub fn attract<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill).cloned() else {
        return 0;
    };
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if !attract_test(w, ct, u, tg) {
        return 0;
    }
    let (a1, ts) = (s16(r.aurastat1), s16(r.auratargetstate));
    if a1 < -1 || a1 >= t.stat_count || !state_ok(w, ts) {
        return 0;
    }
    flag_40(w, u);
    w.effect(BodyEffect::Alignment { u: tg, a: 1, v: 1 });
    node_prepend(w, tg, 9);
    leave_pack(w, tg);
    let range = eval(w, t, u, r.aurarangecalc, skill, lvl);
    let mut d = eval(w, t, u, r.auralencalc, skill, lvl);
    let div = ct
        .difficulty(w.combat().difficulty())
        .map_or(0, |x| x.aicursedivisor as i32);
    if div != 0 {
        d /= div;
    }
    let (tty, tg_guid) = (w.unit_type(tg), guid(w, tg));
    scan_point(w, t, ct, r.aurafilter, u, range, &mut |w, x| {
        if !attract_test(w, ct, u, x) {
            return 0;
        }
        w.effect(BodyEffect::TargetOverride {
            m: x,
            kind: if tty == UnitType::Player { 1 } else { 2 },
            guid: tg_guid as i32,
        });
        // The conversion lasts `d` base ticks (`auralencalc`).
        w.schedule(x, 10, super::after(w, d), 0, 0);
        1
    });
    apply_state(
        w,
        ct,
        StateRequest {
            source: u,
            target: tg,
            skill,
            level: lvl,
            duration: d,
            stat: -1,
            value: 0,
            state: ts,
            callback: callback::ATTRACT,
        },
    );
    1
}

// -----------------------------------------------------------------

/// srvst 19 Bone Prison.
pub fn bone_prison_start<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    let Some(tg) = target(w, u) else {
        return 0;
    };
    i32::from(w.room(tg) != RoomKind::Town)
}

/// Bone Prison offsets (X, Y).
pub const PRISON_X: [i32; 12] = [-1, 1, 3, 4, 4, 3, -1, 1, -3, -4, -4, -3];
pub const PRISON_Y: [i32; 12] = [-4, -4, -3, -1, 1, 3, 4, 4, 3, -1, 1, -3];

/// Bone Prison segment.
fn prison_segment<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    (x, y): (i32, i32),
    skill: i32,
    lvl: i32,
) -> Option<W::Unit> {
    if skill == 0 {
        return None;
    }
    let (c, mode) = summon_class(w, t, ct, u, skill);
    if c < 0 {
        return None;
    }
    let r = rec(t, skill)?;
    let (pet, petmax) = (i32::from(r.pettype), r.petmax);
    // answer 8: `pettype` ≥ count → pt := 0.
    let pt = if pet >= w.pettype_count() { 0 } else { pet };
    if let Some(room) = w.unit_room(u).and_then(|r| w.room_at(r, x, y)) {
        if w.room_in_town(room) {
            return None;
        }
    }
    let pm = eval(w, t, u, petmax, skill, lvl);
    let m = spawn(
        w,
        Summon {
            flags: 9,
            owner: u,
            class: c,
            ai: 0,
            mode,
            x,
            y,
            pet_type: pt,
            pet_max: pm,
        },
    )?;
    skill_stats(w, t, u, m, skill, lvl, 0);
    w.effect(BodyEffect::Umod {
        m,
        umod: 15,
        arg: 0,
    });
    node_prepend(w, m, 9);
    Some(m)
}

/// srvdo 62 Bone Prison.
pub fn bone_prison<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some((tx, ty)) = tpos(w, u) else {
        return 0;
    };
    flag_40(w, u);
    let mut leader: Option<W::Unit> = None;
    for k in 0..12 {
        let at = (tx.wrapping_add(PRISON_X[k]), ty.wrapping_add(PRISON_Y[k]));
        let Some(m) = prison_segment(w, t, ct, u, at, skill, lvl) else {
            continue;
        };
        match leader {
            None => {
                leader = Some(m);
                w.effect(BodyEffect::OwnerData {
                    m,
                    owner: Some(u),
                    a: 0,
                    b: 1,
                });
            }
            Some(l) => {
                w.effect(BodyEffect::OwnerData {
                    m,
                    owner: Some(l),
                    a: 0,
                    b: 0,
                });
                w.effect(BodyEffect::AddMinion { leader: l, m });
            }
        }
        link_source(w, m, Some(u));
        w.path_op(m, PathOp::Face(tx, ty));
    }
    1
}

// ------------------------------------------------------------------

/// srvst 20 Iron Golem.
pub fn iron_golem_start<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    i32::from(target(w, u).is_some_and(|x| w.golem_item(x)))
}

/// srvdo 57 Iron Golem.
pub fn iron_golem<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    if iron_golem_start(w, u) == 0 {
        return 0;
    }
    let tg = target(w, u);
    golem_summon(w, t, ct, u, skill, lvl, tg)
}

/// Steps 2–6 of the body with `tg` as the item. The join's re-summon
/// calls it with no item unit: the saved item is not
/// made as a unit yet, so the golem is spawned without it (D2RUST's own,
/// unverified).
pub fn golem_summon<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
    tg: Option<W::Unit>,
) -> i32 {
    flag_40(w, u);
    if skill == 0 {
        return 0;
    }
    let (c, mode) = summon_class(w, t, ct, u, skill);
    if c < 0 {
        return 0;
    }
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let (pet, petmax) = (i32::from(r.pettype), r.petmax);
    if pet >= w.pettype_count() {
        return 0;
    }
    let (x, y) = match tg {
        Some(k) => w.position(k),
        None => w.position(u),
    };
    let pm = eval(w, t, u, petmax, skill, lvl);
    let Some(m) = spawn(
        w,
        Summon {
            flags: 1,
            owner: u,
            class: c,
            ai: 0,
            mode,
            x,
            y,
            pet_type: pet,
            pet_max: pm,
        },
    ) else {
        return 0;
    };
    if let Some(k) = tg {
        w.effect(BodyEffect::ItemLeaveRoom(k));
        let loc = w.item_first_loc(k);
        w.effect(BodyEffect::ItemMode { item: k, mode: 4 });
        w.effect(BodyEffect::Equip { m, item: k, loc });
    }
    golem_stats(w, t, u, m, skill, lvl);
    w.effect(BodyEffect::AllyInfo { u, m });
    node_insert_owner(w, m, u);
    1
}

// ------------------------------------------------------------------

/// srvdo 79 Conversion.
pub fn conversion_do<W: BodyWorld>(
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
    let (ts, calc1, len) = (s16(r.auratargetstate), r.calc1, r.auralencalc);
    flag_40(w, u);
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let mut ok = false;
    if pair_record(w, u, tg).is_some() {
        // Only a hostile, non-hireling monster that can switch sides converts;
        // the chance is calc1 percent.
        if !w.combat().is_hireling(tg)
            && w.unit_type(tg) == UnitType::Monster
            && w.combat().alignment(tg) == 0
            && can_switch(w, ct, tg, 11)
        {
            let p = eval(w, t, u, calc1, skill, lvl);
            ok = (w.seed(u).roll(100) as i32) < p;
        }
    }
    if !state_ok(w, ts) || !ok {
        apply_melee(w.combat(), ct, u, tg);
        return 1;
    }
    let f = w.frame();
    // The conversion lasts `len` base ticks, at least until the next tick.
    let len = eval(w, t, u, len, skill, lvl);
    let e = f.wrapping_add(super::ticks(w, len)).max(f.wrapping_add(1));
    conversion(w, tg, u, ts, e, callback::CONVERSION, false);
    1
}

// ------------------------------------------------------------------

/// srvst 36 Holy Shield.
pub fn holy_shield_start<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    i32::from(w.has_inventory(u) && w.shield(u).is_some())
}

// ------------------------------------------------------------------

/// srvdo 9 Frenzy.
pub fn frenzy<W: BodyWorld>(
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
    if w.frame_event_index(u) % 2 == 0 {
        apply_melee(w.combat(), ct, u, tg);
        w.effect(BodyEffect::FreeCombat { u, target: tg });
        w.attack_cleanup(u);
        w.weapon_cleanup(u);
        frenzy_charge(w, t, u, skill, lvl);
        return frenzy_swing(w, t, ct, u, Some(tg), skill, lvl);
    }
    flag_40(w, u);
    apply_melee(w.combat(), ct, u, tg);
    w.attack_cleanup(u);
    w.weapon_cleanup(u);
    frenzy_charge(w, t, u, skill, lvl);
    let r = w.combat().melee_range(u).wrapping_add(4);
    let g = guid(w, tg);
    let t2 = next_unit(w, t, ct, u, (0, 0), r, 0x20003, g).0;
    frenzy_swing(w, t, ct, u, t2, skill, lvl)
}

// ------------------------------------------------------------------

/// srvdo 75 Grim Ward.
pub fn grim_ward<W: BodyWorld>(
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
    let (ma, mb, mc) = (s16(r.srvmissilea), s16(r.srvmissileb), s16(r.srvmissilec));
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if !corpse_soft(w, ct, tg) {
        return 0;
    }
    let (x, y) = w.position(tg);
    let Some(room) = w.unit_room(tg).and_then(|r| w.room_at(r, x, y)) else {
        return 0;
    };
    let Some((_, (x, y))) = w.free_point(room, (x, y), 2, 0x1000, true) else {
        return 0;
    };
    if w.unit_type(tg) != UnitType::Monster {
        return 0;
    }
    let Some(ms2) = ct.monstats2(w.class_id(tg)) else {
        return 0;
    };
    let m = if ms2.large {
        mc
    } else if ms2.small {
        mb
    } else {
        ma
    };
    if !missile_ok(t, m) {
        return 0;
    }
    w.spawn_missile(MissileRequest {
        flags: 1,
        x,
        y,
        skill,
        level: lvl,
        ..MissileRequest::new(u, m)
    });
    w.state_on(tg, st::CORPSE_NODRAW, true);
    w.state_on(tg, st::CORPSE_NOSELECT, true);
    w.queue_update(tg);
    1
}

// ------------------------------------------------------------------

/// srvdo 122 Hunger.
pub fn hunger<W: BodyWorld>(
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
    let (rf, hf, hc, sd) = (r.resultflags, r.hitflags, r.hitclass, i32::from(r.srcdam));
    let (c1, c2, c3) = (r.calc1, r.calc2, r.calc3);
    let Some(tg) = target(w, u) else {
        return 0;
    };
    flag_40(w, u);
    let mut record = DamageRecord::default();
    skill_result(w, t, ct, u, tg, skill, lvl, &mut record, 0);
    if record.result & 1 != 0 {
        record.hit_flags |= 2 | hf;
        record.result |= rf;
        if hc != 0 {
            record.hit_class = hc;
        }
        fill(w.combat(), t, ct, u, tg, &mut record, false, sd);
        roll_elemental(w, t, u, &mut record, skill, lvl);
        let p = eval(w, t, u, c1, skill, lvl);
        record.physical = record.physical.wrapping_add(pct(record.physical, p, 100));
        let ll = eval(w, t, u, c2, skill, lvl);
        record.life_leech = record.life_leech.wrapping_add(ll);
        let ml = eval(w, t, u, c3, skill, lvl);
        record.mana_leech = record.mana_leech.wrapping_add(ml);
        start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, 128);
        apply_melee(w.combat(), ct, u, tg);
    }
    1
}

// ------------------------------------------------------------------

/// srvdo 123 Volcano.
pub fn volcano<W: BodyWorld>(w: &mut W, t: &SkillTables, u: W::Unit, skill: i32, lvl: i32) -> i32 {
    if rec(t, skill).is_none() {
        return 0;
    }
    let m = prog_missile(w, t, u, skill);
    if m <= 0 {
        return 0;
    }
    let Some((tx, ty)) = tpos(w, u) else {
        return 0;
    };
    if !point_free(w, t, u, m) {
        return 0;
    }
    flag_40(w, u);
    let s = w.seed(u).roll(256) as i32;
    let Some(mm) = w.spawn_missile(MissileRequest {
        flags: 1,
        x: tx,
        y: ty,
        skill,
        level: lvl,
        ..MissileRequest::new(u, m)
    }) else {
        return 0;
    };
    w.effect(BodyEffect::MissileData28 { missile: mm, v: s });
    w.effect(BodyEffect::MsgA3 {
        u,
        target: None,
        skill,
        lvl,
        x: tx,
        y: ty,
        v: s,
    });
    1
}

// ------------------------------------------------------------------

/// srvdo 51 Mind Blast.
pub fn mind_blast<W: BodyWorld>(
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
    let (range, rf, hf, filter) = (r.aurarangecalc, r.resultflags, r.hitflags, r.aurafilter);
    let Some((cx, cy)) = tpos(w, u) else {
        return 0;
    };
    let mut rr = prog_count(w, t, u, skill, lvl);
    if rr == 0 {
        rr = eval(w, t, u, range, skill, lvl);
    }
    let mut d = DamageRecord::default();
    roll_physical(w, t, u, &mut d, skill, lvl);
    roll_elemental(w, t, u, &mut d, skill, lvl);
    d.result |= rf;
    d.hit_flags |= hf;
    let chance = if lvl <= 0 {
        0
    } else {
        dm(lvl, param(t, skill, 5), param(t, skill, 6))
    };
    let (p3, p4) = (param(t, skill, 3), param(t, skill, 4));
    scan_unit(w, t, ct, u, (cx, cy), rr, filter, false, &mut |w, x| {
        // Convert with its test.
        let ok = w.unit_type(x) == UnitType::Monster
            && !w.combat().is_hireling(x)
            && w.combat().alignment(x) == 0
            && can_switch(w, ct, x, 11)
            && (w.seed(u).roll(100) as i32) <= chance;
        if ok {
            // The conversion lasts `p3` plus up to `p4` base ticks.
            let len = (w.seed(u).roll(p4) as i32).wrapping_add(p3);
            let e = super::after(w, len);
            conversion(w, x, u, st::CONVERSION, e, callback::MIND_BLAST, true);
        } else {
            let mut c = d;
            apply(w.combat(), ct, u, x, true, &mut c);
            w.combat().reaction(u, x, &mut c);
        }
        1
    });
    1
}

// ------------------------------------------------------------------

/// srvdo 52 Dragon Flight.
pub fn dragon_flight<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(s) = rec(t, skill).map(srcdam_or_128) else {
        return 0;
    };
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if w.frame_event_index(u) % 2 == 0 {
        let Some(room) = w.unit_room(u) else {
            return 0;
        };
        flags_clear(w, u, FLAG_40);
        let Some(at) = tpos(w, u) else {
            return 0;
        };
        match w.room_teleport(room) {
            None | Some(0) => return 0,
            Some(2) if w.line_blocked(room, w.position(u), at, 0x804) => return 0,
            _ => {}
        }
        w.place_unit(u, None, at);
        return 1;
    }
    let bonus =
        skill_to_hit(w, t, u, skill, lvl).wrapping_add(w.stat(u, sid::PROGRESSIVE_TOHIT, 0));
    let mut record = melee_rec(w, t, ct, u, tg, bonus, 0);
    if record.result & 1 != 0 {
        record.enh_pct = ln12(t, skill, lvl);
        kick_damage(w, t, ct, u, &mut record, tg, skill, lvl);
    }
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, s);
    if let Some(p) = pair_damage(w, u, tg) {
        finisher(w, t, ct, u, &p);
    }
    apply_melee(w.combat(), ct, u, tg);
    1
}

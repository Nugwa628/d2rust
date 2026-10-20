//! Batch 3 bodies of required level 18: Charged Strike, Fire Wall,
//! Enchant, Chain Lightning, Teleport, Confuse, Poison Explosion,
//! Vengeance, Blessed Hammer, Holy Freeze, Leap Attack, Rabies, Fire
//! Claws, Blade Fury, Dragon Tail.

use super::dos::can_switch;
use super::effects::{BodyEffect, PathOp};
use super::helpers::*;
use super::helpers2::*;
use super::helpers3::*;
use super::starts2::corpse_any;
use super::{callback, init_cb, BodyWorld, MissileRequest};
use crate::combat::{
    apply_melee, bonuses, fill, melee_result, pct, start_combat, CombatTables, CombatWorld,
    DamageRecord,
};
use crate::skills::{concentration, consume_mana, elem_len, roll_elemental, SkillTables};
use crate::units::UnitType;

/// srvdo 11 Charged Strike.
pub fn charged_strike<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    flag_40(w, u);
    let Some(calc1) = rec(t, skill).map(|r| r.calc1) else {
        return 0;
    };
    let Some(tg) = target(w, u) else {
        return 0;
    };
    apply_melee(w.combat(), ct, u, tg);
    let n = eval(w, t, u, calc1, skill, lvl);
    let (tx, ty) = w.position(tg);
    let (ux, uy) = w.position(u);
    let m = prog_missile(w, t, u, skill);
    if !missile_ok(t, m) {
        return 0;
    }
    let mut req = MissileRequest {
        flags: 0x21,
        x: tx,
        y: ty,
        target_x: tx.wrapping_mul(2).wrapping_sub(ux),
        target_y: ty.wrapping_mul(2).wrapping_sub(uy),
        skill,
        level: lvl,
        ..MissileRequest::new(u, m)
    };
    for i in 0..n {
        req.init = Some((init_cb::JITTER, i as u32));
        w.spawn_missile(req);
    }
    1
}

/// srvdo 24 Fire Wall.
pub fn fire_wall<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let (a, b) = (s16(r.srvmissilea), s16(r.srvmissileb));
    if !missile_ok(t, a) || !point_free(w, t, u, a) {
        return 0;
    }
    let (cx, cy) = match target(w, u) {
        Some(tg) => w.position(tg),
        None => w.path_target_point(u),
    };
    if cx == 0 || cy == 0 {
        return 0;
    }
    if let Some(room) = w.unit_room(u).and_then(|r| w.room_at(r, cx, cy)) {
        if w.room_in_town(room) {
            return 0;
        }
    }
    let (ux, uy) = w.position(u);
    let mut req = MissileRequest {
        flags: 0x21,
        x: cx,
        y: cy,
        skill,
        level: lvl,
        ..MissileRequest::new(u, a)
    };
    let (ex, ey) = (uy.wrapping_sub(cy), ux.wrapping_sub(cx));
    for (x, y) in [
        (cx.wrapping_sub(ex), cy.wrapping_add(ey)),
        (cx.wrapping_add(ex), cy.wrapping_sub(ey)),
    ] {
        req.target_x = x;
        req.target_y = y;
        w.spawn_missile(req);
    }
    if missile_ok(t, b) {
        req.flags = 1;
        req.target_x = 0;
        req.target_y = 0;
        req.class = b;
        w.spawn_missile(req);
    }
    1
}

/// srvdo 25 Enchant.
pub fn enchant<W: BodyWorld>(
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
    let (a1, a) = (s16(r.aurastat1), s16(r.aurastate));
    if a1 < -1 || a1 >= t.stat_count || !state_ok(w, a) {
        return 0;
    }
    let x = match target(w, u) {
        Some(tg) if w.allied(u, tg) => tg,
        _ => u,
    };
    let d = eval(w, t, u, r.auralencalc, skill, lvl);
    let Some(l) = apply_state(
        w,
        ct,
        StateRequest {
            source: u,
            target: x,
            skill,
            level: lvl,
            duration: d,
            stat: -1,
            value: 0,
            state: a,
            callback: callback::DEFAULT,
        },
    ) else {
        return 0;
    };
    for (s, c) in [
        (r.aurastat1, r.aurastatcalc1),
        (r.aurastat2, r.aurastatcalc2),
        (r.aurastat3, r.aurastatcalc3),
        (r.aurastat4, r.aurastatcalc4),
        (r.aurastat5, r.aurastatcalc5),
        (r.aurastat6, r.aurastatcalc6),
    ] {
        let s = s16(s);
        if !stat_ok(t, s) {
            continue;
        }
        let v = eval(w, t, u, c, skill, lvl);
        if v != 0 {
            w.list_set(l, s, v);
        }
    }
    w.mark_state_changed(x, a);
    1
}

/// srvdo 26 Chain Lightning.
pub fn chain_lightning<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let (m, calc1) = (s16(r.srvmissilea), r.calc1);
    if !missile_ok(t, m) {
        return 0;
    }
    flag_40(w, u);
    let n = eval(w, t, u, calc1, skill, lvl);
    let Some(mm) = skill_missile_unit(w, m, u, skill, lvl, (0, 0), (0, 0), false, false) else {
        return 0;
    };
    w.effect(BodyEffect::MissileData28 { missile: mm, v: n });
    1
}

/// srvdo 27 Teleport; R is not read (Edge case 21).
pub fn teleport<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    // The target position's result is not tested.
    let at = w.target_position(u).unwrap_or((0, 0));
    let Some(room) = w.unit_room(u) else {
        return 0;
    };
    match w.room_teleport(room) {
        None | Some(0) => return 0,
        Some(2) if w.line_blocked(room, w.position(u), at, 0x804) => return 0,
        _ => {}
    }
    i32::from(w.place_unit(u, None, at))
}

/// srvdo 61 Confuse.
pub fn confuse<W: BodyWorld>(
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
    let (a1, ts) = (s16(r.aurastat1), s16(r.auratargetstate));
    if a1 < -1 || a1 >= t.stat_count || !state_ok(w, ts) {
        return 0;
    }
    flag_40(w, u);
    let range = eval(w, t, u, r.aurarangecalc, skill, lvl);
    let mut d = eval(w, t, u, r.auralencalc, skill, lvl);
    let div = ct
        .difficulty(w.combat().difficulty())
        .map_or(0, |x| x.aicursedivisor as i32);
    if div != 0 {
        d /= div;
    }
    // Edge case 18: slots 2…6 from `aurastat2`; slot 1 stays (0, 0).
    let mut stats = [0; 6];
    let mut values = [0; 6];
    let mut upd = false;
    let pairs = [
        (r.aurastat2, r.aurastatcalc2),
        (r.aurastat3, r.aurastatcalc3),
        (r.aurastat4, r.aurastatcalc4),
        (r.aurastat5, r.aurastatcalc5),
        (r.aurastat6, r.aurastatcalc6),
    ];
    for (i, (s, c)) in pairs.into_iter().enumerate() {
        let s = s16(s);
        if !stat_ok(t, s) {
            stats[i + 1] = -1;
            break;
        }
        let Some(info) = w.stat_info(s) else {
            break;
        };
        if info.updateanimrate {
            upd = true;
        }
        stats[i + 1] = s;
        values[i + 1] = eval(w, t, u, c, skill, lvl);
    }
    scan_point(w, t, ct, r.aurafilter, u, range, &mut |w, x| {
        if w.unit_type(x) != UnitType::Monster
            || w.combat().alignment(x) != 0
            || !w.combat().hostile(u, x)
            || !w.is_alive(x)
            || !can_switch(w, ct, x, 11)
        {
            return 0;
        }
        let v = scaled(w, x, stats[0], values[0]);
        let Some(l) = apply_state(
            w,
            ct,
            StateRequest {
                source: u,
                target: x,
                skill,
                level: lvl,
                duration: d,
                stat: if v == 0 { -1 } else { stats[0] },
                value: v,
                state: ts,
                callback: callback::CONFUSE,
            },
        ) else {
            return 0;
        };
        for i in 1..6 {
            if stat_ok(t, stats[i]) {
                let v = scaled(w, x, stats[i], values[i]);
                if v != 0 {
                    w.list_set(l, stats[i], v);
                }
            }
        }
        if upd {
            w.combat().refresh_anim_rate(x);
        }
        w.effect(BodyEffect::Alignment { u: x, a: 1, v: 1 });
        node_prepend(w, x, 9);
        w.effect(BodyEffect::TargetOverride {
            m: x,
            kind: 3,
            guid: 0,
        });
        // The conversion lasts `d` base ticks (`auralencalc`).
        w.schedule(x, 10, super::after(w, d), 0, 0);
        1
    })
}

/// srvdo 63 Poison Explosion.
pub fn poison_explosion<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    if skill == 0 {
        return 0;
    }
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let m = s16(r.srvmissilea);
    if !missile_ok(t, m) {
        return 1;
    }
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if !corpse_any(w, ct, tg) {
        return 0;
    }
    w.state_on(tg, st::CORPSE_NOSELECT, true);
    w.queue_update(tg);
    burst(w, t, u, tg, m, skill, lvl, (0, 2, 0));
    1
}

/// srvst 35 Vengeance.
pub fn vengeance<W: BodyWorld>(
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
    let (rf, hf, srcdam) = (r.resultflags, r.hitflags, i32::from(r.srcdam));
    let calcs = [
        (r.calc1, sid::PASSIVE_FIRE_MASTERY),
        (r.calc2, sid::PASSIVE_COLD_MASTERY),
        (r.calc3, sid::PASSIVE_LTNG_MASTERY),
    ];
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let mut record = skill_melee(w, t, ct, u, tg, skill, lvl);
    if record.result & 1 != 0 {
        record.physical = bonuses(w.combat(), t, u, true, None, 0, 0, 0, 0, srcdam);
        record.result |= rf;
        record.hit_flags = hf | 1;
        let b = base_roll(w, u);
        let mut add = [0; 3];
        for (i, (c, m)) in calcs.into_iter().enumerate() {
            let mut p = eval(w, t, u, c, skill, lvl);
            let q = w.stat(u, m, 0);
            if p != 0 && q != 0 {
                p = p.wrapping_add(pct(p, q, 100));
            }
            add[i] = pct(b, p, 100);
        }
        record.fire = record.fire.wrapping_add(add[0]);
        record.cold = record.cold.wrapping_add(add[1]);
        record.lightning = record.lightning.wrapping_add(add[2]);
        let len = elem_len(w, t, Some(u), skill, lvl);
        record.cold_len = record.cold_len.wrapping_add(len);
        let k = w.entry_param(u, &e, 1);
        // answer 8: k outside 0…2 sets no hit class; E
        // param 1 := (k + 1) rem 3 (C remainder: a negative k can stay
        // negative).
        match k {
            0 => record.hit_class = 0x20,
            1 => record.hit_class = 0x30,
            2 => record.hit_class = 0x40,
            _ => {}
        }
        w.set_entry_param_of(u, &e, 1, k.wrapping_add(1) % 3);
    }
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, 128);
    1
}

/// srvdo 73 Blessed Hammer.
pub fn blessed_hammer<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
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
    flag_40(w, u);
    let Some(mm) = w.spawn_missile(MissileRequest {
        flags: 0x20,
        origin: Some(u),
        target_x: tx,
        target_y: ty,
        skill,
        level: lvl,
        ..MissileRequest::new(u, m)
    }) else {
        return 0;
    };
    w.path_op(mm, PathOp::Type(14));
    w.path_op(mm, PathOp::Compute);
    let c = concentration(w, t, u, skill);
    if c != 0 {
        for s in [52, 53] {
            let b = w.base_stat(mm, s, 0);
            w.set_stat(mm, s, pct(b, 100i32.wrapping_add(c), 100));
        }
    }
    1
}

/// srvst 41 Leap Attack.
pub fn leap_attack_start<W: BodyWorld>(w: &mut W, u: W::Unit, skill: i32) -> i32 {
    let Some(e) = w.find_entry(u, skill) else {
        return 0;
    };
    let tg = target(w, u);
    let Some((x, y)) = leap_aim(w, u, tg) else {
        if let Some(k) = tg {
            if w.combat().in_melee_range(u, k, 0) {
                swing(w, u, k);
                return 1;
            }
        }
        return 0;
    };
    let Some(room) = w.unit_room(u) else {
        return 0;
    };
    w.effect(BodyEffect::PatternStamp {
        room,
        x,
        y,
        u,
        mask: 0x80,
    });
    set_uninterruptable(w, u, true);
    w.set_entry_param_of(u, &e, 1, x);
    w.set_entry_param_of(u, &e, 2, y);
    match tg {
        Some(k) => {
            let (ty, g) = (type_index(w, k), guid(w, k));
            w.set_entry_param_of(u, &e, 3, ty);
            w.set_entry_param_of(u, &e, 4, g as i32);
        }
        None => w.set_entry_param_of(u, &e, 3, 6),
    }
    w.set_entry_flags(u, &e, 0x1080);
    w.state_on(u, st::SKILL_MOVE, true);
    // Step 7: only acts on a target without state 54, and is
    // only called for one with it (Edge case 17): nothing to do.
    1
}

/// srvdo 78 Leap Attack.
pub fn leap_attack<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(e) = w.find_entry(u, skill) else {
        return 0;
    };
    let f = w.entry_flags(u, &e);
    if f & 0x100 != 0 {
        if leap_land(w, ct, u, &e) == 0 {
            return 1;
        }
        let Some(_) = leap_pick(w, t, ct, u, Some(&e)) else {
            // No landing spot: try again 4 base ticks later.
            w.delete_timers(u, 1, 0);
            w.schedule(u, 1, super::after(w, 4), 0, 0);
            return 0;
        };
        w.anim_from(u, 16);
        return 1;
    }
    if f & 0x80 != 0 {
        return leap_launch(w, ct, u, &e);
    }
    if f & 0x200 != 0 {
        return leap_strike(w, t, ct, u, skill, lvl);
    }
    w.set_entry_flags(u, &e, 0x1000);
    1
}

/// srvst 57 Rabies.
pub fn rabies_start<W: BodyWorld>(
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
    let (a, calc1, calc4, etype) = (s16(r.aurastate), r.calc1, r.calc4, r.etype);
    let Some(e) = w.used_skill(u).filter(|e| e.skill == skill) else {
        return 0;
    };
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if state_ok(w, a) && !w.has_state(u, a as u16) {
        return 0;
    }
    w.set_entry_param_of(u, &e, 1, 0);
    let h = skill_to_hit(w, t, u, skill, lvl);
    let res = melee_result(w.combat(), t, ct, Some(u), Some(tg), h, 0);
    if res & 1 == 0 {
        return 0;
    }
    // Edge case 20: evaluated and discarded.
    eval(w, t, u, calc1, skill, lvl);
    if etype != 0 {
        eval(w, t, u, calc4, skill, lvl);
    }
    w.set_entry_param_of(u, &e, 1, 1);
    1
}

/// srvdo 121 Rabies.
pub fn rabies<W: BodyWorld>(
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
    let (calc1, hf, hc) = (r.calc1, r.hitflags, r.hitclass);
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let Some(e) = w.used_skill(u).filter(|e| e.skill == skill) else {
        return 0;
    };
    if w.entry_param(u, &e, 1) == 0 {
        return 0;
    }
    w.set_entry_param_of(u, &e, 1, 0);
    shape_start(w, t, ct, u, lvl);
    apply_melee(w.combat(), ct, u, tg);
    let len = elem_len(w, t, Some(u), skill, lvl);
    plague(w, t, u, tg, len, skill, lvl);
    // Second hit.
    let len = len.max(10);
    let mut record = DamageRecord::default();
    skill_result(w, t, ct, u, tg, skill, lvl, &mut record, 0);
    if record.result & 1 != 0 {
        record.hit_flags |= hf;
        if hc != 0 {
            record.hit_class = hc;
        }
        record.enh_pct = eval(w, t, u, calc1, skill, lvl);
        roll_elemental(w, t, u, &mut record, skill, lvl);
    }
    set_len(t, &mut record, len, skill);
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, 128);
    apply_melee(w.combat(), ct, u, tg);
    1
}

/// srvst 58 Fire Claws.
pub fn fire_claws<W: BodyWorld>(
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
    let (calc1, hf, hc, srcdam) = (r.calc1, r.hitflags, r.hitclass, i32::from(r.srcdam));
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let mut record = DamageRecord::default();
    skill_result(w, t, ct, u, tg, skill, lvl, &mut record, 0);
    if record.result & 1 != 0 {
        record.hit_flags |= 2 | hf;
        if hc != 0 {
            record.hit_class = hc;
        }
        record.enh_pct = eval(w, t, u, calc1, skill, lvl);
        fill(w.combat(), t, ct, u, tg, &mut record, false, srcdam);
        roll_elemental(w, t, u, &mut record, skill, lvl);
        start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, 128);
    }
    1
}

/// srvst 26 Blade Fury.
pub fn blade_fury_start<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(sm) = rec(t, skill).map(|r| i32::from(r.startmana)) else {
        return 0;
    };
    if prog_missile(w, t, u, skill) < 0 {
        return 0;
    }
    let Some(e) = w.find_entry(u, skill) else {
        return 0;
    };
    let f = w.frame();
    if let Some(l) = w.state_list(u, st::INFERNO) {
        // Each strike keeps the channel 7 base ticks longer.
        let until = super::after(w, 7);
        w.set_list_expire(l, until);
        w.combat().schedule_timer(u, 12, until);
        if w.entry_param(u, &e, 1) <= f {
            let r = blade_fury(w, t, ct, u, skill, lvl);
            if r != 0 {
                consume_mana(w, t, Some(u), skill, lvl);
            }
            return r;
        }
        w.anim_restart(u, 1);
        return 1;
    }
    if sm > 0 && w.stat(u, sid::MANA, 0) < sm.wrapping_shl(8) {
        return 0;
    }
    // The channel starts with 21 base ticks.
    let until = super::after(w, 21);
    let Some(l) = w.alloc_list(2, until, Some(u)) else {
        return 0;
    };
    w.combat().schedule_timer(u, 12, until);
    w.attach(u, l);
    w.set_remove_callback(l, callback::BLADE_FURY);
    w.set_list_state(l, st::INFERNO);
    w.state_on(u, st::INFERNO, true);
    w.set_entry_param_of(u, &e, 1, 0);
    1
}

/// srvdo 48 Blade Fury.
pub fn blade_fury<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    _ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(e) = w.find_entry(u, skill) else {
        return 0;
    };
    let m = prog_missile(w, t, u, skill);
    if m < 0 {
        return 0;
    }
    let n = prog_count(w, t, u, skill, lvl);
    if n.wrapping_sub(1) <= 0 {
        return 0;
    }
    flags_clear(w, u, FLAG_40);
    let Some((tx, ty)) = tpos(w, u) else {
        return 0;
    };
    let mut made = 0;
    let f = w.frame();
    if w.entry_param(u, &e, 1) < f {
        w.spawn_missile(MissileRequest {
            flags: 0x20,
            origin: Some(u),
            target_x: tx,
            target_y: ty,
            skill,
            level: lvl,
            ..MissileRequest::new(u, m)
        });
        made = 1;
        // The next blade after `n` base ticks.
        let next = f.wrapping_add(super::ticks(w, n)).wrapping_sub(1);
        w.set_entry_param_of(u, &e, 1, next);
    }
    if w.has_state(u, st::INFERNO as u16) {
        w.anim_restart(u, 1);
    }
    made
}

/// srvst 27 Dragon Tail.
pub fn dragon_tail_start<W: BodyWorld>(
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
    let rate = param(t, skill, 4);
    if let Some(l) = w.alloc_list(4, 0, Some(u)) {
        w.attach(u, l);
        w.list_set(l, stat::ATTACKRATE, rate);
        w.combat().refresh_anim_rate(u);
    }
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let bonus =
        skill_to_hit(w, t, u, skill, lvl).wrapping_add(w.stat(u, sid::PROGRESSIVE_TOHIT, 0));
    let mut record = melee_rec(w, t, ct, u, tg, bonus, 0);
    if record.result & 1 == 0 {
        return 0;
    }
    kick_damage(w, t, ct, u, &mut record, tg, skill, lvl);
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, s);
    1
}

/// srvdo 50 Dragon Tail.
pub fn dragon_tail<W: BodyWorld>(
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
    let (calc1, range) = (r.calc1, r.aurarangecalc);
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let Some(p) = pair_damage(w, u, tg) else {
        return 0;
    };
    finisher(w, t, ct, u, &p);
    apply_melee(w.combat(), ct, u, tg);
    // Tests the unit, not the target (as the base game does): a target
    // the kick killed still gets the explosion.
    if w.is_alive(u) {
        let pc =
            eval(w, t, u, calc1, skill, lvl).wrapping_add(w.stat(u, sid::PASSIVE_FIRE_MASTERY, 0));
        let d = DamageRecord {
            fire: pct(p.physical, pc, 100),
            result: 9,
            ..DamageRecord::default()
        };
        let at = w.position(tg);
        let rr = eval(w, t, u, range, skill, lvl);
        area_damage(w, t, ct, u, at, rr, &d, 0);
    }
    1
}

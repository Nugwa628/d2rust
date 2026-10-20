//! Batch 3 bodies of required level 12: Impale, Bone Wall, Charge,
//! Double Throw, Find Item, Cloak of Shadows.

use super::dos::can_switch;
use super::effects::{BodyEffect, PathOp};
use super::helpers::*;
use super::helpers2::*;
use super::helpers3::*;
use super::starts2::corpse_any;
use super::{callback, BodyWorld, MissileRequest};
use crate::combat::{
    apply_melee, bonuses, melee_result, pct, start_combat, CombatTables, CombatWorld, DamageRecord,
};
use crate::skills::{roll_elemental, SkillTables};
use crate::units::UnitType;

// -----------------------------------------------------------------

/// srvst 7 Impale.
pub fn impale<W: BodyWorld>(
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
    let (calc1, calc2, calc3, srcdam) = (r.calc1, r.calc2, r.calc3, i32::from(r.srcdam));
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if !w.combat().hostile(u, tg) {
        return 0;
    }
    let mut record = skill_melee(w, t, ct, u, tg, skill, lvl);
    if record.result & 1 != 0 {
        let p = eval(w, t, u, calc1, skill, lvl);
        // Edge case 16: the raw `SrcDam`.
        record.physical = bonuses(w.combat(), t, u, true, None, 0, 0, p, 0, srcdam);
        convert(w, t, u, &mut record, skill, lvl);
        roll_elemental(w, t, u, &mut record, skill, lvl);
        record.hit_flags = 1;
        if let Some(wpn) = w.current_weapon(u) {
            if w.item_breakable(wpn) {
                let chance = eval(w, t, u, calc2, skill, lvl);
                let amount = eval(w, t, u, calc3, skill, lvl);
                wear(w, u, wpn, chance, amount);
            }
        }
    }
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, 128);
    1
}

// -----------------------------------------------------------------

/// srvdo 60 Bone Wall.
pub fn bone_wall<W: BodyWorld>(
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
    match w.unit_room(u).and_then(|r| w.room_at(r, tx, ty)) {
        Some(room) if !w.room_in_town(room) => {}
        _ => return 0,
    }
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
    let (pet, petmax, calc2, sa) = (i32::from(r.pettype), r.petmax, r.calc2, s16(r.srvmissilea));
    if pet >= w.pettype_count() {
        return 0;
    }
    let pm = eval(w, t, u, petmax, skill, lvl);
    let Some(m) = spawn(
        w,
        Summon {
            flags: 9,
            owner: u,
            class: c,
            ai: 0,
            mode,
            x: tx,
            y: ty,
            pet_type: pet,
            pet_max: pm,
        },
    ) else {
        return 0;
    };
    w.effect(BodyEffect::OwnerData {
        m,
        owner: Some(u),
        a: 0,
        b: 1,
    });
    w.effect(BodyEffect::Umod {
        m,
        umod: 15,
        arg: 0,
    });
    skill_stats(w, t, u, m, skill, lvl, 0);
    node_prepend(w, m, 9);
    let (mx, my) = w.position(m);
    let n = eval(w, t, u, calc2, skill, lvl) / 2;
    if n <= 1 || !missile_ok(t, sa) {
        return 1;
    }
    let (ux, uy) = w.position(u);
    let (mut dx, dy) = (ux.wrapping_sub(mx), uy.wrapping_sub(my));
    if dx == 0 && dy == 0 {
        dx = 1;
    }
    let mut req = MissileRequest {
        flags: 0x21,
        x: mx,
        y: my,
        skill,
        level: lvl,
        ..MissileRequest::new(u, sa)
    };
    let g = guid(w, m) as i32;
    for (ax, ay) in [
        (mx.wrapping_sub(dy), my.wrapping_add(dx)),
        (mx.wrapping_add(dy), my.wrapping_sub(dx)),
    ] {
        req.target_x = ax;
        req.target_y = ay;
        if let Some(mm) = w.spawn_missile(req) {
            w.effect(BodyEffect::MissileData28 { missile: mm, v: g });
            w.effect(BodyEffect::MissileData2C { missile: mm, v: n });
        }
    }
    1
}

// -----------------------------------------------------------------

/// The monstats `BaseId` of a monster; −1
/// otherwise.
fn base_id<W: BodyWorld>(w: &W, ct: &CombatTables, u: W::Unit) -> i32 {
    if w.unit_type(u) != UnitType::Monster {
        return -1;
    }
    ct.monstats(w.class_id(u))
        .map_or(-1, |m| i32::from(m.baseid as i16))
}

/// srvst 31 Charge.
pub fn charge_start<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
) -> i32 {
    if rec(t, skill).is_none() {
        return 0;
    }
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    if !w.has_path(u) {
        return 0;
    }
    let tg = target(w, u);
    let player = w.unit_type(u) == UnitType::Player;
    let b = base_id(w, ct, u);
    if let Some(k) = tg {
        if w.combat().in_melee_range(u, k, 0) {
            if player {
                return swing(w, u, k);
            }
            if w.unit_type(u) == UnitType::Monster {
                w.set_mode(u, 1);
                w.set_used_skill(u, None);
                let m = if b == 73 || b == 211 { 5 } else { 4 };
                return w.mode_request(u, m, Some(k));
            }
        }
    }
    match tg {
        Some(k) if !w.has_state(k, st::UNINTERRUPTABLE as u16) => {
            let (ty, g) = (type_index(w, k), guid(w, k));
            w.set_entry_param_of(u, &e, 1, ty);
            w.set_entry_param_of(u, &e, 2, g as i32);
        }
        _ => {
            w.set_entry_param_of(u, &e, 1, 6);
            w.set_entry_param_of(u, &e, 2, 0);
        }
    }
    w.set_entry_param_of(u, &e, 4, 7);
    let mut v = 0x100;
    if player {
        if let Some(c) = ct.charstats(w.class_id(u)) {
            v = i32::from(c.runvelocity).wrapping_shl(8);
        }
    } else if w.unit_type(u) == UnitType::Monster {
        //–: with a monstats row, class 211 (the
        // class itself, unit +4) takes hit mode 15, and every monster's
        // path gets +0x14/+0x16 := 0 and the step counts 20.
        if let Some(m) = ct.monstats(w.class_id(u)) {
            v = i32::from(m.run as i16).wrapping_shl(8);
            if w.class_id(u) == 211 {
                w.set_entry_param_of(u, &e, 4, 15);
            }
            w.path_op(u, PathOp::Clear14);
            w.path_op(u, PathOp::Steps(20));
        }
    }
    let vp = w.stat(u, sid::VELOCITYPERCENT, 0).max(50);
    let vel = pct(v, param(t, skill, 1).wrapping_add(vp), 100);
    w.path_op(u, PathOp::Velocity(vel));
    w.path_op(u, PathOp::Compute);
    w.set_entry_flags(u, &e, 0x1001);
    w.state_on(u, st::SKILL_MOVE, true);
    1
}

/// srvdo 67 Charge.
pub fn charge<W: BodyWorld>(
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
    let (calc1, srcdam) = (r.calc1, i32::from(r.srcdam));
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    let f = hit_frame(w, u);
    let (p1, p2) = (w.entry_param(u, &e, 1), w.entry_param(u, &e, 2));
    let mut k = if p1 == 6 {
        None
    } else {
        w.find_unit(p1 as u32, p2 as u32)
    };
    let monster = w.unit_type(u) == UnitType::Monster;
    let flags = w.entry_flags(u, &e);
    let now = w.frame();
    if flags & 1 == 0 {
        w.set_entry_flags(u, &e, 0);
        w.set_entry_param_of(u, &e, 1, 0);
        w.set_entry_param_of(u, &e, 2, 0);
        landing_msg(w, u, skill);
        w.anim_from(u, f);
        flag_40(w, u);
        if k.is_none() {
            k = next_unit(w, t, ct, u, (0, 0), 3, 3, u32::MAX).0;
        }
        let Some(k) = k else {
            w.delete_timers(u, 1, 0);
            w.schedule(u, 1, now.wrapping_add(1), 0, 0);
            return 0;
        };
        let range = if monster { 3 } else { 0 };
        if w.combat().in_melee_range(u, k, range) && w.is_alive(u) {
            let mut record = DamageRecord::default();
            if w.unit_type(u) == UnitType::Player {
                let h = skill_to_hit(w, t, u, skill, lvl);
                record.result = melee_result(w.combat(), t, ct, Some(u), Some(k), h, 0) | 8;
            } else {
                record.result = 9;
            }
            record.enh_pct = eval(w, t, u, calc1, skill, lvl);
            convert(w, t, u, &mut record, skill, lvl);
            roll_elemental(w, t, u, &mut record, skill, lvl);
            if monster {
                // The class itself, not its `BaseId`: a clawviper6
                // charges with A1.
                let m = match w.class_id(u) {
                    73 => 8,
                    211 | 436 => 5,
                    _ => 4,
                };
                mode_damage(w, t, ct, u, m);
            }
            record.hit_class = 0x70;
            start_combat(w.combat(), t, ct, Some(u), Some(k), &mut record, srcdam);
            apply_melee(w.combat(), ct, u, k);
            w.combat().overlay(k, 147);
        }
        return 1;
    }
    let r = if monster && flags & 2 != 0 { 3 } else { 0 };
    if let Some(k) = k {
        if w.combat().in_melee_range(u, k, r) {
            w.set_entry_flags(u, &e, 0);
            w.anim_from(u, f.wrapping_add(1));
            landing_msg(w, u, skill);
            return 1;
        }
    }
    if flags & 2 == 0 {
        if now.wrapping_sub(w.anim_frame(u) >> 8) >= f {
            w.anim_from(u, 0);
            w.set_entry_flags(u, &e, 1);
        }
        w.delete_timers(u, 0, 0);
        w.schedule(u, 0, now.wrapping_add(1), 1, 0);
        flags_clear(w, u, FLAG_40);
        return 1;
    }
    w.set_entry_flags(u, &e, 0);
    landing_msg(w, u, skill);
    let k2 = next_unit(w, t, ct, u, (0, 0), 3, 3, p2 as u32).0;
    if let Some(k2) = k2.filter(|&x| w.combat().in_melee_range(u, x, 0)) {
        let (ty, g) = (type_index(w, k2), guid(w, k2));
        w.set_entry_param_of(u, &e, 1, ty);
        w.set_entry_param_of(u, &e, 2, g as i32);
        return 1;
    }
    w.delete_timers(u, 1, 0);
    w.schedule(u, 1, now.wrapping_add(1), 0, 0);
    0
}

// -----------------------------------------------------------------

/// srvdo 74 Double Throw.
pub fn double_throw<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(calc1) = rec(t, skill).map(|r| r.calc1) else {
        return 0;
    };
    let Some(wpn) = w.current_weapon(u) else {
        return 0;
    };
    let m = w.item_missile_type(wpn);
    if m <= 0 || m >= t.missiles.len() as i32 {
        return 0;
    }
    let lob = w.item_is(wpn, 38);
    if let Some(mm) = skill_missile_unit(w, m, u, skill, lvl, (0, 0), (0, 0), true, lob) {
        let h = skill_to_hit(w, t, u, skill, lvl);
        w.add_stat(mm, sid::TOHIT, h);
        let p = eval(w, t, u, calc1, skill, lvl);
        w.add_stat(mm, sid::DAMAGEPERCENT, p);
    }
    1
}

// -----------------------------------------------------------------

/// srvst 34 Find Item.
pub fn find_item_start<W: BodyWorld>(w: &mut W, ct: &CombatTables, u: W::Unit) -> i32 {
    let Some(tg) = target(w, u) else {
        return 0;
    };
    i32::from(corpse_any(w, ct, tg))
}

/// srvdo 72 Find Item.
pub fn find_item<W: BodyWorld>(
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
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if !corpse_any(w, ct, tg) || w.has_state(tg, st::CORPSE_NOSELECT as u16) {
        return 0;
    }
    w.state_on(tg, st::CORPSE_NOSELECT, true);
    w.queue_update(tg);
    let p = eval(w, t, u, calc1, skill, lvl);
    if (w.seed(u).roll(100) as i32) >= p {
        return 0;
    }
    let r2 = w.seed(u).roll(100) as i32;
    let q = find_item_quality(r2, [1, 2, 3, 4].map(|i| param(t, skill, i)));
    w.effect(BodyEffect::TreasureDrop {
        corpse: tg,
        killer: u,
        q,
    });
    1
}

/// The Find Item bands of the body's step 4: q = 1, or 2 / 3 / 4 when falls
/// in the `Param2` / `Param3` / `Param4` band after `Param1`.
pub fn find_item_quality(r2: i32, p: [i32; 4]) -> i32 {
    let mut lo = p[0];
    for (q, &width) in (2..=4).zip(&p[1..]) {
        let hi = lo.wrapping_add(width);
        if lo <= r2 && r2 < hi {
            return q;
        }
        lo = hi;
    }
    1
}

// -----------------------------------------------------------------

/// srvdo 47 Cloak of Shadows.
pub fn cloak<W: BodyWorld>(
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
    let a = s16(r.aurastate);
    if !state_ok(w, a) || w.has_state(u, a as u16) {
        return 0;
    }
    let d = eval(w, t, u, r.auralencalc, skill, lvl);
    let v1 = eval(w, t, u, r.passivecalc1, skill, lvl);
    let Some(l) = apply_state(
        w,
        ct,
        StateRequest {
            source: u,
            target: u,
            skill,
            level: lvl,
            duration: d,
            stat: s16(r.passivestat1),
            value: v1,
            state: a,
            callback: callback::DEFAULT,
        },
    ) else {
        return 0;
    };
    for (s, c) in [
        (r.passivestat2, r.passivecalc2),
        (r.passivestat3, r.passivecalc3),
        (r.passivestat4, r.passivecalc4),
        (r.passivestat5, r.passivecalc5),
    ] {
        let s = s16(s);
        if s < 0 {
            continue;
        }
        let v = eval(w, t, u, c, skill, lvl);
        if v != 0 {
            w.list_set(l, s, v);
        }
    }
    w.list_set(l, stat::MODIFIERLIST_SKILL, skill);
    w.list_set(l, stat::MODIFIERLIST_LEVEL, lvl);
    w.mark_state_changed(u, a);
    let ts = s16(r.auratargetstate);
    if !state_ok(w, ts) {
        // Edge case 15.
        return 0;
    }
    let mut stats = [0; 6];
    let mut values = [0; 6];
    for (i, (s, c)) in [
        (r.aurastat1, r.aurastatcalc1),
        (r.aurastat2, r.aurastatcalc2),
        (r.aurastat3, r.aurastatcalc3),
        (r.aurastat4, r.aurastatcalc4),
        (r.aurastat5, r.aurastatcalc5),
        (r.aurastat6, r.aurastatcalc6),
    ]
    .into_iter()
    .enumerate()
    {
        stats[i] = s16(s);
        if stats[i] >= 0 {
            values[i] = eval(w, t, u, c, skill, lvl);
        }
    }
    let range = eval(w, t, u, r.aurarangecalc, skill, lvl);
    scan_unit(
        w,
        t,
        ct,
        u,
        (0, 0),
        range,
        r.aurafilter,
        true,
        &mut |w, x| {
            if !w.is_alive(x) {
                return 0;
            }
            let Some(l) = apply_state(
                w,
                ct,
                StateRequest {
                    source: u,
                    target: x,
                    skill,
                    level: lvl,
                    duration: d,
                    stat: if stats[0] > 0 { stats[0] } else { -1 },
                    value: values[0],
                    state: ts,
                    callback: callback::AI_CURSE,
                },
            ) else {
                return 0;
            };
            for i in 1..6 {
                if stats[i] >= 0 {
                    w.list_set(l, stats[i], values[i]);
                }
            }
            if w.unit_type(x) == UnitType::Monster && can_switch(w, ct, x, 10) {
                w.set_ai_state(x, 10);
            }
            1
        },
    );
    1
}

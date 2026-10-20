//! The do functions of batch 2: the shared slots of several class
//! skills (summons, shouts, sentries, novas, damage auras, Multiple Shot,
//! the Assassin charges, Fend / Zeal, Meteor, Inner Sight, Raise
//! Skeleton, shape shifts, Inferno, Blaze, Feral Rage, Guided Arrow,
//! Twister, Shadow Warrior, Armageddon). Each returns the value the do
//! core reads.

use super::dos::{aura_unit, AuraCtx};
use super::effects::BodyEffect;
use super::helpers::*;
use super::helpers2::*;
use super::helpers3::*;
use super::starts2::corpse_raise;
use super::{callback, init_cb, BodyWorld, MissileRequest};
use crate::combat::{
    apply, apply_melee, start_combat, CombatTables, CombatWorld, DamageRecord, RoomKind,
};
use crate::skills::use_::{period, set_delay};
use crate::skills::{mana_cost, roll_elemental, SkillTables};
use crate::units::UnitType;

/// The pet type read as a signed byte; `None` outside 0…count − 1.
fn pet_signed<W: BodyWorld>(w: &W, v: u8) -> Option<i32> {
    let p = i32::from(v as i8);
    (0..w.pettype_count()).contains(&p).then_some(p)
}

// -----------------------------------------------------------------

/// srvdo 119 Druid summon.
pub fn druid_summon<W: BodyWorld>(
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
    let (pet, petmax, calc2) = (r.pettype, r.petmax, r.calc2);
    let (c, mode) = summon_class(w, t, ct, u, skill);
    if c < 0 {
        return 0;
    }
    let Some(pt) = pet_signed(w, pet) else {
        return 0;
    };
    flag_40(w, u);
    if tpos(w, u).is_none() {
        return 0;
    }
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
    node_insert_owner(w, m, u);
    let p = eval(w, t, u, calc2, skill, lvl).max(1);
    base_stats(w, u, m, p, lvl);
    skill_stats(w, t, u, m, skill, lvl, 0);
    1
}

// -----------------------------------------------------------------

/// srvdo 68 Basic shout.
pub fn shout<W: BodyWorld>(w: &mut W, t: &SkillTables, u: W::Unit, skill: i32, lvl: i32) -> i32 {
    flag_40(w, u);
    if rec(t, skill).is_none() {
        return 0;
    }
    let m = prog_missile(w, t, u, skill);
    if !missile_ok(t, m) {
        return 0;
    }
    ring(w, u, u, m, skill, lvl, 0);
    shout_state(w, t, u, u, skill, lvl);
    1
}

// -----------------------------------------------------------------

/// srvdo 45 Sentry.
pub fn sentry_do<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    if rec(t, skill).is_none() {
        return 0;
    }
    flag_40(w, u);
    let Some(at) = tpos(w, u) else {
        return 0;
    };
    // The trap's think, shots and death: its AI (,
    // the `Skill1` entry's `calc4`).
    i32::from(sentry(w, t, ct, u, at, skill, lvl).is_some())
}

// -----------------------------------------------------------------

/// srvdo 22 Nova attack.
pub fn nova<W: BodyWorld>(w: &mut W, t: &SkillTables, u: W::Unit, skill: i32, lvl: i32) -> i32 {
    flag_40(w, u);
    let Some(calc1) = rec(t, skill).map(|r| r.calc1) else {
        return 0;
    };
    let m = prog_missile(w, t, u, skill);
    let Some(row) = usize::try_from(m).ok().and_then(|i| t.missiles.get(i)) else {
        return 0;
    };
    let (vel, vl) = (i32::from(row.vel), i32::from(row.vellev));
    let v = vel
        .wrapping_add(vl.wrapping_mul(lvl) / 8)
        .wrapping_add(eval(w, t, u, calc1, skill, lvl));
    ring(w, u, u, m, skill, lvl, v);
    1
}

// -----------------------------------------------------------------

/// srvdo 66 Holy Fire, Holy Shock, Sanctuary, Conviction;
/// with `freeze`, srvdo 81 Holy Freeze.
pub fn damage_aura<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
    freeze: bool,
) -> i32 {
    let Some(r) = rec(t, skill).cloned() else {
        return 0;
    };
    let a = s16(r.aurastate);
    if !state_ok(w, a) {
        return 0;
    }
    let cost = mana_cost(&r, lvl);
    let mana = w.stat(u, sid::MANA, 0);
    let d = period(w, t, u, skill, lvl)
        .wrapping_sub(w.frame())
        .wrapping_add(1);
    let player = w.unit_type(u) == UnitType::Player;
    let mut own = AuraCtx {
        source_state: a,
        skill,
        lvl,
        duration: d,
        stats: [0; 6],
        values: [0; 6],
        count: 0,
        list: None,
        passivestate: 0,
        callback: if freeze { callback::HOLY_FREEZE } else { 0 },
    };
    let passive = [
        (r.passivestat1, r.passivecalc1),
        (r.passivestat2, r.passivecalc2),
        (r.passivestat3, r.passivecalc3),
        (r.passivestat4, r.passivecalc4),
        (r.passivestat5, r.passivecalc5),
    ];
    for (i, (s, c)) in passive.into_iter().enumerate() {
        own.stats[i] = s16(s);
        if !player || mana >= cost {
            own.values[i] = eval(w, t, u, c, skill, lvl);
        }
    }
    aura_unit(w, t, ct, u, &mut own, u);
    if w.room(u) == RoomKind::Town {
        return 1;
    }
    let ts = s16(r.auratargetstate);
    let mut stats = [0; 6];
    let mut values = [0; 6];
    let mut any = false;
    if state_ok(w, ts) {
        let aura = [
            (r.aurastat1, r.aurastatcalc1),
            (r.aurastat2, r.aurastatcalc2),
            (r.aurastat3, r.aurastatcalc3),
            (r.aurastat4, r.aurastatcalc4),
            (r.aurastat5, r.aurastatcalc5),
            (r.aurastat6, r.aurastatcalc6),
        ];
        for (i, (s, c)) in aura.into_iter().enumerate() {
            stats[i] = s16(s);
            if mana >= cost {
                values[i] = eval(w, t, u, c, skill, lvl);
                if values[i] != 0 {
                    any = true;
                }
            }
        }
    }
    let mut dr = DamageRecord::default();
    if roll_elemental(w, t, u, &mut dr, skill, lvl) != 0 {
        dr.hit_class |= 0xD;
    }
    if r.hitclass != 0 {
        dr.hit_class = r.hitclass;
    }
    dr.result |= r.resultflags | 0x20;
    dr.hit_flags |= r.hitflags;
    let range = eval(w, t, u, r.aurarangecalc, skill, lvl);
    let diff = w.combat().difficulty();
    scan_unit(
        w,
        t,
        ct,
        u,
        (0, 0),
        range,
        r.aurafilter,
        false,
        &mut |w, x| {
            if freeze && w.unit_type(x) == UnitType::Monster {
                let cold = ct.monstats(w.class_id(x)).map(|m| {
                    i32::from(match diff {
                        0 => m.coldeffect,
                        1 => m.coldeffect_n,
                        _ => m.coldeffect_h,
                    } as i8)
                });
                if cold.is_none_or(|c| c >= 0) {
                    return 0;
                }
            }
            if any {
                let mut cx = AuraCtx {
                    source_state: ts,
                    skill,
                    lvl,
                    duration: d,
                    stats,
                    values,
                    count: 0,
                    list: None,
                    passivestate: 0,
                    callback: 0,
                };
                aura_unit(w, t, ct, u, &mut cx, x);
            }
            let mut c = dr;
            apply(w.combat(), ct, u, x, true, &mut c);
            w.combat().reaction(u, x, &mut c);
            if freeze {
                let on = w.seed(x).step() % 100 < 20;
                w.state_on(x, st::SHATTER, on);
            }
            1
        },
    );
    // Edge case 9: nothing counts the targets.
    if cost > 0 && player {
        w.state_on(u, state::NOMANAREGEN, false);
    }
    1
}

// -----------------------------------------------------------------

/// The arrow missile: `srvmissilea`, or `srvmissileb` for a non-bow hand.
pub fn arrow_missile<W: BodyWorld>(w: &W, t: &SkillTables, u: W::Unit, skill: i32) -> i32 {
    let Some(r) = rec(t, skill) else {
        return -1;
    };
    let (a, b) = (s16(r.srvmissilea), s16(r.srvmissileb));
    if w.hand_class(u) != 1 && b >= 0 {
        b
    } else {
        a
    }
}

/// The side step of Multiple Shot: (px, py).
pub fn side_step((dx, dy): (i32, i32)) -> (i32, i32) {
    let (mut dx, mut dy) = (dx, dy);
    let s = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
    if s < 4 {
        dx = dx.wrapping_mul(4);
        dy = dy.wrapping_mul(4);
    }
    let s = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
    if s < 16 {
        dx = dx.wrapping_mul(2);
        dy = dy.wrapping_mul(2);
    }
    let (mut a, mut b) = (dx.wrapping_neg(), dy);
    while a.wrapping_mul(a).wrapping_add(b.wrapping_mul(b)) > 3 {
        a /= 2;
        b /= 2;
    }
    (b, a)
}

/// srvdo 8 Multiple Shot, Teeth, Shock Wave.
pub fn multiple_shot<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    flag_40(w, u);
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let (calc1, calc2, calc3) = (r.calc1, r.calc2, r.calc3);
    let Some((mut tx, mut ty)) = tpos(w, u) else {
        return 1;
    };
    let n = eval(w, t, u, calc1, skill, lvl);
    let (ux, uy) = w.position(u);
    let (px, py) = side_step((tx.wrapping_sub(ux), ty.wrapping_sub(uy)));
    tx = tx.wrapping_sub(px.wrapping_mul(n) / 2);
    ty = ty.wrapping_sub(py.wrapping_mul(n) / 2);
    let m = arrow_missile(w, t, u, skill);
    if !missile_ok(t, m) {
        return 0;
    }
    let mut c = eval(w, t, u, calc3, skill, lvl);
    if c == 0 {
        c = n;
    }
    let mut req = MissileRequest {
        flags: 0x820 | 0x1_0000,
        origin: Some(u),
        skill,
        level: lvl,
        ..MissileRequest::new(u, m)
    };
    req.activate = eval(w, t, u, calc2, skill, lvl);
    let k = n.wrapping_sub(c) / 2;
    let mut shoot = |w: &mut W, req: &mut MissileRequest<W::Unit>, times: i32| {
        for _ in 0..times {
            req.target_x = tx;
            req.target_y = ty;
            w.spawn_missile(*req);
            tx = tx.wrapping_add(px);
            ty = ty.wrapping_add(py);
        }
    };
    shoot(w, &mut req, k);
    req.flags &= !0x1_0000;
    shoot(w, &mut req, c);
    req.flags |= 0x1_0000;
    shoot(w, &mut req, n.wrapping_sub(k).wrapping_sub(c));
    1
}

// -----------------------------------------------------------------

/// srvdo 115 Plague Poppy, Cycle of Life, Vines.
pub fn vines<W: BodyWorld>(
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
    let (pet, petmax, calc2) = (r.pettype, r.petmax, r.calc2);
    let (c, _) = summon_class(w, t, ct, u, skill);
    if c < 0 {
        return 0;
    }
    let Some(pt) = pet_signed(w, pet) else {
        return 0;
    };
    flag_40(w, u);
    let pm = eval(w, t, u, petmax, skill, lvl);
    // Edge case 13: mode 8 whatever `summode` says.
    let Some(m) = spawn(
        w,
        Summon {
            flags: 0,
            owner: u,
            class: c,
            ai: 0,
            mode: 8,
            x: 0,
            y: 0,
            pet_type: pt,
            pet_max: pm,
        },
    ) else {
        return 0;
    };
    node_insert_owner(w, m, u);
    w.state_on(m, st::VINE_BEAST, true);
    let v = eval(w, t, u, calc2, skill, lvl).max(1);
    w.set_stat(m, sid::LEVEL, v);
    skill_stats(w, t, u, m, skill, lvl, 0);
    1
}

// -----------------------------------------------------------------

/// srvdo 34 Tiger Strike, Cobra Strike, Royal Strike.
pub fn charge_hit<W: BodyWorld>(
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
    let (a, a1, s) = (s16(r.aurastate), s16(r.aurastat1), srcdam_or_128(r));
    if !state_ok(w, a) || !stat_ok(t, a1) {
        return 0;
    }
    let mut record = skill_melee(w, t, ct, u, tg, skill, lvl);
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, s);
    if pair_damage(w, u, tg).is_some_and(|p| p.result & 1 != 0) {
        charge_add(w, t, u, skill, lvl, (a, a1));
    }
    apply_melee(w.combat(), ct, u, tg);
    1
}

// -----------------------------------------------------------------

/// srvdo 56 Clay Golem, BloodGolem, FireGolem.
pub fn golem<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    flag_40(w, u);
    if skill == 0 {
        return 0;
    }
    // An out-of-range skill reads a null record: fatal; refused.
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let (c, pet, sm, petmax) = (s16(r.summon), r.pettype, r.summode, r.petmax);
    if c < 0 || c >= ct.monstats.len() as i32 {
        return 0;
    }
    let pt = if i32::from(pet) >= w.pettype_count() {
        0
    } else {
        i32::from(pet)
    };
    let mode = if sm >= 16 { 1 } else { i32::from(sm) };
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
    golem_stats(w, t, u, m, skill, lvl);
    w.effect(BodyEffect::AllyInfo { u, m });
    node_insert_owner(w, m, u);
    1
}

// ------------------------------------------------------------------

/// srvdo 35 Fists of Fire, Claws of Thunder, Blades of Ice.
pub fn claws<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let mut dual = false;
    if w.has_inventory(u) {
        if let (Some(a), Some(b)) = (w.item_at(u, 4), w.item_at(u, 5)) {
            dual = a != b
                && w.item_usable(a)
                && w.item_usable(b)
                && w.item_is(a, 45)
                && w.item_is(b, 45);
        }
    }
    if dual && w.frame_event_index(u) % 2 == 0 {
        flags_clear(w, u, FLAG_40);
    } else {
        flag_40(w, u);
    }
    charge_hit(w, t, ct, u, skill, lvl);
    1
}

// ------------------------------------------------------------------

/// srvdo 13 Fend, Zeal, Fury.
pub fn fend<W: BodyWorld>(
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
    let (calc2, s) = (r.calc2, srcdam_or_128(r));
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    let range = w.combat().melee_range(u).wrapping_add(4);
    let (p2, p3) = (w.entry_param(u, &e, 2), w.entry_param(u, &e, 3));
    let mut tg = w.find_unit(p2 as u32, p3 as u32);
    if !tg.is_some_and(|x| w.combat().in_melee_range(u, x, 0)) {
        tg = next_unit(w, t, ct, u, (0, 0), range, 0x20003, p3 as u32).0;
    }
    let Some(tg) = tg else {
        return 0;
    };
    let mut record = skill_melee(w, t, ct, u, tg, skill, lvl);
    if record.result & 1 != 0 {
        record.enh_pct = eval(w, t, u, calc2, skill, lvl);
        if convert(w, t, u, &mut record, skill, lvl) {
            roll_elemental(w, t, u, &mut record, skill, lvl);
        }
    }
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, s);
    apply_melee(w.combat(), ct, u, tg);
    let n = w.entry_param(u, &e, 1).wrapping_sub(1);
    w.set_entry_param_of(u, &e, 1, n);
    if n <= 0 {
        return 0;
    }
    let g = guid(w, tg);
    let Some(t2) = next_unit(w, t, ct, u, (0, 0), range, 0x20003, g).0 else {
        return 0;
    };
    let (ty, g2) = (type_index(w, t2), guid(w, t2));
    w.set_entry_param_of(u, &e, 2, ty);
    w.set_entry_param_of(u, &e, 3, g2 as i32);
    w.anim_rewind(u, param(t, skill, 2));
    0
}

// ------------------------------------------------------------------

/// srvdo 28 Meteor, Blizzard, Eruption, ….
pub fn meteor<W: BodyWorld>(w: &mut W, t: &SkillTables, u: W::Unit, skill: i32, lvl: i32) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let m = s16(r.srvmissilea);
    if !missile_ok(t, m) {
        return 0;
    }
    flag_40(w, u);
    if !point_free(w, t, u, m) {
        return 0;
    }
    i32::from(missile_at(w, u, skill, lvl, m, (0, 0)).is_some())
}

// ------------------------------------------------------------------

/// srvdo 6 Inner Sight, Slow Missiles.
pub fn inner_sight<W: BodyWorld>(
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
    let (ts, a1) = (s16(r.auratargetstate), s16(r.aurastat1));
    if !state_ok(w, ts) || !stat_ok(t, a1) {
        return 0;
    }
    let d = eval(w, t, u, r.auralencalc, skill, lvl);
    let v = eval(w, t, u, r.aurastatcalc1, skill, lvl);
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
            apply_state(
                w,
                ct,
                StateRequest {
                    source: u,
                    target: x,
                    skill,
                    level: lvl,
                    duration: d,
                    stat: a1,
                    value: v,
                    state: ts,
                    callback: callback::DEFAULT,
                },
            );
            1
        },
    );
    1
}

// ------------------------------------------------------------------

/// srvdo 31 Raise Skeleton, Raise Skeletal Mage.
pub fn raise_skeleton<W: BodyWorld>(
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
    if !corpse_raise(w, ct, tg) {
        return 0;
    }
    raise_penalty(w, ct, u);
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let (pet, petmax) = (r.pettype, r.petmax);
    let (c, mode) = summon_class(w, t, ct, u, skill);
    if c < 0 {
        return 0;
    }
    let Some(pt) = pet_signed(w, pet) else {
        return 0;
    };
    let (x, y) = w.position(tg);
    w.effect(BodyEffect::RoomDelete(tg));
    w.effect(BodyEffect::RemoveUnit(tg));
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
            pet_type: pt,
            pet_max: pm,
        },
    ) else {
        return 0;
    };
    base_stats(w, u, m, 0, lvl);
    components(w, t, u, m, skill, lvl);
    skill_stats(w, t, u, m, skill, lvl, 0);
    summon_resist(w, u, m);
    node_insert_owner(w, m, u);
    1
}

// ------------------------------------------------------------------

/// srvdo 116 Werewolf, Werebear.
pub fn shape_shift<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let (a, delay, len) = (s16(r.aurastate), r.delay, r.auralencalc);
    if !state_ok(w, a) {
        return 0;
    }
    flag_40(w, u);
    if clear_group(w, u, a, true) {
        let d = eval(w, t, u, delay, skill, lvl);
        if d > 0 {
            set_delay(w, u, d);
        }
        return 0;
    }
    let d = eval(w, t, u, len, skill, lvl);
    if w.state_list(u, a).is_some() {
        return 0;
    }
    let e = super::after(w, d);
    let Some(l) = w.alloc_list(2, e, Some(u)) else {
        return 0;
    };
    w.set_list_state(l, a);
    w.set_remove_callback(l, callback::SHAPE);
    w.attach(u, l);
    w.state_on(u, a, true);
    w.combat().schedule_timer(u, 12, e);
    aura_fill(w, t, u, l, skill, lvl);
    w.list_set(l, stat::MODIFIERLIST_SKILL, skill);
    w.list_set(l, stat::MODIFIERLIST_LEVEL, lvl);
    if let Some(en) = w.find_entry(u, skill) {
        w.set_entry_mode(u, &en, 10);
    }
    1
}

// ------------------------------------------------------------------

/// srvdo 19 Inferno, Arctic Blast.
pub fn inferno_cast<W: BodyWorld>(
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
    let m = s16(r.srvmissilea);
    if !missile_ok(t, m) {
        return 0;
    }
    inferno_do(w, t, ct, u, skill, lvl, m)
}

// ------------------------------------------------------------------

/// srvdo 23 Blaze, Energy Shield.
pub fn blaze<W: BodyWorld>(
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
    let (a1, a, len) = (s16(r.aurastat1), s16(r.aurastate), r.auralencalc);
    if a1 < -1 || a1 >= t.stat_count || !state_ok(w, a) {
        return 0;
    }
    let d = eval(w, t, u, len, skill, lvl);
    let Some(l) = apply_state(
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
    ) else {
        return 0;
    };
    passive_fill(w, t, u, l, skill, lvl);
    w.list_set(l, stat::MODIFIERLIST_SKILL, skill);
    w.list_set(l, stat::MODIFIERLIST_LEVEL, lvl);
    reregister(w, t, u, skill, lvl, a);
    1
}

// ------------------------------------------------------------------

/// srvdo 120 Feral Rage, Maul.
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
    let (a, len, calc2) = (s16(r.aurastate), r.auralencalc, r.calc2);
    if !state_ok(w, a) {
        return 0;
    }
    let Some(e) = w.used_skill(u).filter(|e| e.skill == skill) else {
        return 0;
    };
    flag_40(w, u);
    if let Some(tg) = target(w, u) {
        apply_melee(w.combat(), ct, u, tg);
    }
    if w.entry_param(u, &e, 1) == 0 {
        return 1;
    }
    let len = eval(w, t, u, len, skill, lvl);
    let ex = super::after(w, len);
    let Some(l) = list_or_new(w, u, a, 2, ex, u, callback::DEFAULT) else {
        return 1;
    };
    w.set_list_expire(l, ex);
    w.combat().schedule_timer(u, 12, ex);
    w.mark_state_changed(u, a);
    let cap = eval(w, t, u, calc2, skill, lvl);
    let n = cap.min(w.list_get(l, sid::SKILL_FRENZY).wrapping_add(1));
    w.list_set(l, sid::SKILL_FRENZY, n);
    w.list_set(l, stat::MODIFIERLIST_SKILL, skill);
    w.list_set(l, stat::MODIFIERLIST_LEVEL, lvl);
    // Edge case 19: the charge count as the level.
    aura_fill(w, t, u, l, skill, n);
    1
}

// ------------------------------------------------------------------

/// srvdo 10 Guided Arrow, Bone Spirit.
pub fn guided_arrow<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    flag_40(w, u);
    let Some(calc1) = rec(t, skill).map(|r| r.calc1) else {
        return 0;
    };
    let tg = target(w, u);
    let Some((tx, ty)) = tpos(w, u) else {
        return 0;
    };
    let v = eval(w, t, u, calc1, skill, lvl);
    let m = arrow_missile(w, t, u, skill);
    if !missile_ok(t, m) {
        return 0;
    }
    let mut req = MissileRequest {
        flags: 0x20,
        origin: Some(u),
        target: tg,
        target_x: tx,
        target_y: ty,
        skill,
        level: lvl,
        ..MissileRequest::new(u, m)
    };
    if v != 0 {
        req.init = Some((init_cb::DAMAGE_PERCENT, v as u32));
    }
    if tg.is_none() {
        req.flags = 0x420;
    }
    let Some(mm) = w.spawn_missile(req) else {
        return 1;
    };
    let (ux, uy) = w.position(u);
    w.effect(BodyEffect::MissileData28 {
        missile: mm,
        v: if tg.is_some() { 1 } else { 2 },
    });
    // `(dy as i16) * 0x10000 + (dx as i16)`: a negative dx borrows from the
    // high half (dx = -1, dy = 3 packs 0x0002_FFFF; measured: the base game reads
    // back (-1, 2) at `aim` in gen-skill-nec-93).
    let dy = i32::from(ty.wrapping_sub(uy) as i16);
    let dx = i32::from(tx.wrapping_sub(ux) as i16);
    w.effect(BodyEffect::MissileData2C {
        missile: mm,
        v: dy.wrapping_mul(0x10000).wrapping_add(dx),
    });
    1
}

// ------------------------------------------------------------------

/// srvdo 118 Twister, Tornado.
pub fn twister<W: BodyWorld>(w: &mut W, t: &SkillTables, u: W::Unit, skill: i32, lvl: i32) -> i32 {
    let Some(calc1) = rec(t, skill).map(|r| r.calc1) else {
        return 0;
    };
    let m = prog_missile(w, t, u, skill);
    if !missile_ok(t, m) {
        return 0;
    }
    flag_40(w, u);
    let n = eval(w, t, u, calc1, skill, lvl);
    if n <= 0 {
        return 0;
    }
    fan(w, u, n, m, skill, lvl, false)
}

// ------------------------------------------------------------------

/// srvdo 49 Shadow Warrior, Shadow Master.
pub fn shadow<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    flag_40(w, u);
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let (pet, petmax, a, len) = (r.pettype, r.petmax, s16(r.aurastate), r.auralencalc);
    let (c, mode) = summon_class(w, t, ct, u, skill);
    if c < 0 {
        return 0;
    }
    if i32::from(pet) >= w.pettype_count() {
        return 0;
    }
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
            pet_type: i32::from(pet),
            pet_max: pm,
        },
    ) else {
        return 0;
    };
    let ul = w.stat(u, sid::LEVEL, 0);
    w.set_stat(m, sid::LEVEL, ul);
    shadow_stats(w, t, m, skill, lvl);
    let mut ilvl = if lvl <= 0 {
        0
    } else {
        param(t, skill, 5).wrapping_add(lvl.wrapping_sub(1).wrapping_mul(param(t, skill, 6)))
    };
    if ilvl < 1 {
        ilvl = 1;
    }
    if ilvl > t.level_cap {
        ilvl = t.level_cap;
    }
    w.effect(BodyEffect::Equipment {
        owner: u,
        m,
        skill,
        lvl,
        ilvl,
    });
    // The summon first thinks 20 base ticks after appearing.
    w.delete_timers(m, 2, 0);
    w.schedule(m, 2, super::after(w, 20), 0, 0);
    if a >= 1 && a < w.state_count() {
        w.state_on(m, a, true);
    }
    link_source(w, m, Some(u));
    let d = eval(w, t, u, len, skill, lvl);
    if d > 0 {
        w.schedule(m, 7, super::after(w, d), 0, 0);
        w.effect(BodyEffect::Umod {
            m,
            umod: 21,
            arg: 0,
        });
    }
    node_insert_owner(w, m, u);
    1
}

// ------------------------------------------------------------------

/// srvdo 124 Armageddon, Hurricane.
pub fn armageddon<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let (a, len) = (s16(r.aurastate), r.auralencalc);
    if !state_ok(w, a) {
        return 0;
    }
    let Some(e) = w.used_skill(u).filter(|e| e.skill == skill) else {
        return 0;
    };
    flag_40(w, u);
    let roll = w.seed(u).roll(0x1_0000) as i32;
    // The state lasts `len` base ticks.
    let len = eval(w, t, u, len, skill, lvl).max(1);
    let ex = super::after(w, len);
    let Some(l) = list_or_new(w, u, a, 2, ex, u, callback::DEFAULT) else {
        return 0;
    };
    w.set_list_expire(l, ex);
    w.combat().schedule_timer(u, 12, ex);
    aura_fill(w, t, u, l, skill, lvl);
    w.list_set(l, stat::MODIFIERLIST_SKILL, skill);
    w.list_set(l, stat::MODIFIERLIST_LEVEL, lvl);
    // The aura acts again `Param4` base ticks later.
    let tt = super::after(w, param(t, skill, 4));
    w.delete_timers(u, 5, skill);
    w.schedule(u, 5, tt, skill, lvl);
    w.set_entry_param_of(u, &e, 1, roll);
    1
}

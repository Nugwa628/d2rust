//! Batch 3 bodies of required level 30: Valkyrie, Lightning
//! Strike, Hydra, Revive, Fist of the Heavens, Redemption, Whirlwind,
//! Berserk, Blade Shield.

use super::dos::{aura_unit, can_switch, spend_aura_mana, AuraCtx};
use super::effects::{BodyEffect, PathOp};
use super::helpers::*;
use super::helpers2::*;
use super::helpers3::*;
use super::starts2::{corpse_any, corpse_raise};
use super::{callback, BodyWorld, MissileRequest};
use crate::combat::{apply_melee, pct, start_combat, CombatTables, CombatWorld, RoomKind};
use crate::monsters::init::stats_by_level;
use crate::skills::use_::period;
use crate::skills::{elem_max, elem_min, mana_cost, roll_elemental, SkillTables};
use crate::units::UnitType;

// -----------------------------------------------------------------

/// srvdo 16 Valkyrie.
pub fn valkyrie<W: BodyWorld>(
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
    let (pet, petmax, calc2) = (i32::from(r.pettype), r.petmax, r.calc2);
    if w.unit_type(u) != UnitType::Player {
        return 0;
    }
    let (c, mode) = summon_class(w, t, ct, u, skill);
    if c < 0 {
        return 0;
    }
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
    base_stats(w, u, m, 0, lvl);
    let ilvl = eval(w, t, u, calc2, skill, lvl);
    skill_stats(w, t, u, m, skill, lvl, ilvl);
    // The Valkyrie first thinks 20 base ticks after appearing.
    w.delete_timers(m, 2, 0);
    w.schedule(m, 2, super::after(w, 20), 0, 0);
    w.state_on(m, st::VALKYRIE, true);
    link_source(w, m, Some(u));
    node_insert_owner(w, m, u);
    1
}

// -----------------------------------------------------------------

/// srvst 10 Lightning Strike.
pub fn lightning_strike_start<W: BodyWorld>(
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
    if w.used_skill(u).is_none() {
        return 0;
    }
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let mut record = melee_rec(w, t, ct, u, tg, 0, 0);
    if record.result & 1 != 0 {
        let a = elem_min(w, t, Some(u), skill, lvl, true);
        let b = elem_max(w, t, Some(u), skill, lvl, true);
        let x = w.seed(u).roll(b.wrapping_sub(a)) as i32;
        record.lightning = a.wrapping_add(x);
        record.enh_pct = eval(w, t, u, calc1, skill, lvl);
        convert(w, t, u, &mut record, skill, lvl);
    }
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, s);
    1
}

/// srvdo 14 Lightning Strike.
pub fn lightning_strike<W: BodyWorld>(
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
    let (calc1, calc2) = (r.calc1, r.calc2);
    let Some(tg) = target(w, u) else {
        return 0;
    };
    apply_melee(w.combat(), ct, u, tg);
    let n = eval(w, t, u, calc1, skill, lvl);
    let at = w.position(tg);
    let g = guid(w, tg);
    let Some(k) = next_unit(w, t, ct, u, at, n, 3, g).0 else {
        return 0;
    };
    let m = prog_missile(w, t, u, skill);
    if !missile_ok(t, m) {
        return 0;
    }
    let j = eval(w, t, u, calc2, skill, lvl);
    let (kx, ky) = w.position(k);
    if let Some(mm) = w.spawn_missile(MissileRequest {
        flags: 0x20,
        origin: Some(tg),
        target_x: kx,
        target_y: ky,
        skill,
        level: lvl,
        ..MissileRequest::new(u, m)
    }) {
        w.effect(BodyEffect::MissileData28 { missile: mm, v: j });
    }
    1
}

// -----------------------------------------------------------------

/// srvst 14 Hydra.
pub fn hydra_start<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    let Some(room) = w.unit_room(u) else {
        return 0;
    };
    let Some((x, y)) = tpos(w, u) else {
        return 0;
    };
    let Some(r) = w.room_at(room, x, y) else {
        return 0;
    };
    i32::from(!w.room_in_town(r))
}

/// Hydra offsets (X, Y).
pub const HYDRA_X: [i32; 3] = [-1, 0, 1];
pub const HYDRA_Y: [i32; 3] = [-1, 0, -1];

/// srvdo 144 Hydra.
pub fn hydra<W: BodyWorld>(
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
    let (pet, petmax) = (i32::from(r.pettype as i8), r.petmax);
    let (c, mode) = summon_class(w, t, ct, u, skill);
    if c < 0 || pet < 0 || pet >= w.pettype_count() || hydra_start(w, u) == 0 {
        return 0;
    }
    let Some((tx, ty)) = tpos(w, u) else {
        return 0;
    };
    let mut o = u;
    if w.unit_type(u) == UnitType::Monster && w.combat().is_hireling(u) {
        if let Some(x) = w.minion_owner(u) {
            o = x;
        }
    }
    // The summons' lifetime, `ln12` base ticks.
    let tt = super::after(w, ln12(t, skill, lvl));
    let mut made = 0;
    for i in 0..3 {
        let pm = eval(w, t, o, petmax, skill, lvl);
        let Some(m) = spawn(
            w,
            Summon {
                flags: 1,
                owner: o,
                class: c.wrapping_add(i as i32),
                ai: 0,
                mode,
                x: tx.wrapping_add(HYDRA_X[i]),
                y: ty.wrapping_add(HYDRA_Y[i]),
                pet_type: pet,
                pet_max: pm,
            },
        ) else {
            continue;
        };
        w.effect(BodyEffect::SetSkill { m, skill, lvl: 1 });
        skill_stats(w, t, o, m, skill, lvl, 0);
        made = 1;
        w.effect(BodyEffect::AiParams {
            m,
            p0: tt,
            p1: -666,
            p2: -666,
        });
        if w.unit_type(o) == UnitType::Monster {
            let a = w.combat().alignment(o);
            w.effect(BodyEffect::Alignment { u: m, a, v: 1 });
        }
    }
    made
}

// -----------------------------------------------------------------

/// The revive test.
fn revive_test<W: BodyWorld>(w: &mut W, ct: &CombatTables, x: W::Unit) -> bool {
    w.unit_type(x) == UnitType::Monster
        && ct.monstats2(w.class_id(x)).is_some_and(|m| m.revive)
        && !w.is_alive(x)
        && corpse_raise(w, ct, x)
        && can_switch(w, ct, x, 7)
}

/// srvst 21 Revive.
pub fn revive_start<W: BodyWorld>(w: &mut W, ct: &CombatTables, u: W::Unit) -> i32 {
    let Some(tg) = target(w, u) else {
        return 0;
    };
    i32::from(revive_test(w, ct, tg))
}

/// srvdo 58 Revive.
pub fn revive<W: BodyWorld>(
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
    let (pet, petmax, calc2) = (i32::from(r.pettype), r.petmax, r.calc2);
    if pet >= w.pettype_count() {
        return 0;
    }
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if !revive_test(w, ct, tg) {
        return 0;
    }
    raise_penalty(w, ct, u);
    // Stand up.
    let at = w.position(tg);
    if let Some(room) = w.unit_room(tg) {
        w.effect(BodyEffect::PatternClear {
            room,
            x: at.0,
            y: at.1,
            u: tg,
            mask: 0x8000,
        });
        let size = w.unit_size(tg);
        // Edge case 26: no free point → the rest still runs.
        if let Some((r2, p)) = w.free_point(room, at, size, 0x3C01, true) {
            flags_or(w, tg, 0x0402_000E);
            w.effect(BodyEffect::AiRefresh(tg));
            w.mode_request(tg, 1, None);
            leave_pack(w, tg);
            w.delete_timers(tg, 8, 0);
            if w.right_skill(tg).is_some() {
                w.effect(BodyEffect::Op643C50(tg));
            }
            w.place_unit(tg, Some(r2), p);
        }
    }
    let d = w.combat().difficulty();
    let lv = w.stat(tg, sid::LEVEL, 0);
    let (mn, mx) = match ct.monstats(w.class_id(tg)) {
        Some(ms) => {
            let s = stats_by_level(ms, w.monlvl(), w.l_flag(), d, lv);
            (s.min_hp, s.max_hp)
        }
        None => (0, 0),
    };
    let roll = w.seed(tg).roll(mx.wrapping_sub(mn).wrapping_add(1)) as i32;
    let mut h = mn.wrapping_add(roll).wrapping_shl(8);
    w.set_stat(tg, sid::MAXHP, h);
    w.set_stat(tg, sid::LIFE, h);
    let cl = w.stat(u, sid::LEVEL, 0);
    if lv != 0 && cl < lv {
        h = pct(h, cl, lv).max(1);
        w.set_stat(tg, sid::MAXHP, h);
        w.set_stat(tg, sid::LIFE, h);
        w.set_stat(tg, sid::LEVEL, cl);
    }
    skill_stats(w, t, u, tg, skill, lvl, 0);
    // Revive setup.
    w.effect(BodyEffect::OwnerData {
        m: tg,
        owner: Some(u),
        a: 0,
        b: 0,
    });
    w.effect(BodyEffect::LeashOwner {
        m: tg,
        owner: Some(u),
    });
    // The revived unit thinks 15 base ticks later.
    w.delete_timers(tg, 2, 0);
    w.schedule(tg, 2, super::after(w, 15), 0, 0);
    w.effect(BodyEffect::Alignment { u: tg, a: 2, v: 1 });
    w.effect(BodyEffect::ClearTargetOverride(tg));
    flags_or(w, tg, 0x8000_0000);
    w.state_on(tg, st::REVIVE, true);
    let dd = eval(w, t, u, calc2, skill, lvl);
    if dd > 0 {
        w.effect(BodyEffect::Umod {
            m: tg,
            umod: 21,
            arg: 0,
        });
        w.schedule(tg, 7, super::after(w, dd), 0, 0);
    }
    let pm = eval(w, t, u, petmax, skill, lvl);
    w.effect(BodyEffect::PetAdd {
        owner: u,
        pet: tg,
        t: pet,
        max: pm,
    });
    node_insert_owner(w, tg, u);
    w.path_op(tg, PathOp::Reset);
    1
}

// -----------------------------------------------------------------

/// srvdo 80 Fist of the Heavens.
pub fn fist_of_heavens<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(ov) = rec(t, skill).map(|r| s16(r.srvoverlay)) else {
        return 0;
    };
    let m = prog_missile(w, t, u, skill);
    if m <= 0 {
        return 0;
    }
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let (x, y) = w.position(tg);
    let Some(mm) = w.spawn_missile(MissileRequest {
        flags: 1,
        x,
        y,
        skill,
        level: lvl,
        ..MissileRequest::new(u, m)
    }) else {
        return 0;
    };
    let (ty, g) = (type_index(w, tg), guid(w, tg));
    w.effect(BodyEffect::MissileData28 { missile: mm, v: ty });
    w.effect(BodyEffect::MissileData2C {
        missile: mm,
        v: g as i32,
    });
    // Edge case 28: overlay 0 is accepted.
    if (0..w.overlay_count()).contains(&ov) {
        w.combat().overlay(tg, ov);
    }
    1
}

// -----------------------------------------------------------------

/// srvdo 82 Redemption.
pub fn redemption<W: BodyWorld>(
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
        callback: 0,
    };
    for (i, (s, c)) in [
        (r.passivestat1, r.passivecalc1),
        (r.passivestat2, r.passivecalc2),
        (r.passivestat3, r.passivecalc3),
        (r.passivestat4, r.passivecalc4),
        (r.passivestat5, r.passivecalc5),
    ]
    .into_iter()
    .enumerate()
    {
        own.stats[i] = s16(s);
        if own.stats[i] >= 0 && (!player || mana >= cost) {
            own.values[i] = eval(w, t, u, c, skill, lvl);
        }
    }
    aura_unit(w, t, ct, u, &mut own, u);
    if w.room(u) == RoomKind::Town {
        return 1;
    }
    let mut count = 0;
    let range = eval(w, t, u, r.aurarangecalc, skill, lvl);
    let (c1, c2, c3) = (r.calc1, r.calc2, r.calc3);
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
            if !corpse_any(w, ct, x) {
                return 0;
            }
            let p = eval(w, t, u, c1, skill, lvl);
            if (w.seed(u).roll(100) as i32) >= p {
                return 0;
            }
            let life = w
                .stat(u, sid::LIFE, 0)
                .wrapping_add(eval(w, t, u, c2, skill, lvl).wrapping_shl(8))
                .min(w.stat_max(u, sid::LIFE));
            w.set_stat(u, sid::LIFE, life);
            let mana = w
                .stat(u, sid::MANA, 0)
                .wrapping_add(eval(w, t, u, c3, skill, lvl).wrapping_shl(8))
                .min(w.stat_max(u, sid::MANA));
            w.set_stat(u, sid::MANA, mana);
            w.state_on(x, st::REDEEMED, true);
            flags_clear(w, x, 6);
            count += 1;
            1
        },
    );
    if cost > 0 && player {
        if count > 0 {
            w.state_on(u, state::NOMANAREGEN, true);
            spend_aura_mana(w, u, cost);
        } else {
            w.state_on(u, state::NOMANAREGEN, false);
        }
    }
    1
}

// ------------------------------------------------------------------

/// Walk velocity.
pub(crate) fn walk_velocity<W: BodyWorld>(w: &W, ct: &CombatTables, u: W::Unit) -> i32 {
    match w.unit_type(u) {
        UnitType::Player => ct
            .charstats(w.class_id(u))
            .map_or(0x600, |c| i32::from(c.walkvelocity).wrapping_shl(8)),
        UnitType::Monster => ct
            .monstats(w.class_id(u))
            .map_or(0x600, |m| i32::from(m.velocity as i16).wrapping_shl(8)),
        _ => 0x600,
    }
}

/// srvst 38 Whirlwind.
pub fn whirlwind_start<W: BodyWorld>(
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
    let e = w.find_entry(u, skill);
    let (Some(e), Some(_)) = (e, tpos(w, u)) else {
        landing_msg(w, u, skill);
        return 0;
    };
    if w.has_state(u, st::INFERNO as u16) {
        return 0;
    }
    if let Some(tg) = target(w, u) {
        if w.combat().in_melee_range(u, tg, 0) {
            landing_msg(w, u, skill);
            return swing(w, u, tg);
        }
    }
    if !w.has_path(u) {
        landing_msg(w, u, skill);
        return 0;
    }
    let player = w.unit_type(u) == UnitType::Player;
    let m0 = if player { 0x1C09 } else { 0x3C01 };
    w.path_op(u, PathOp::MoveMask(0xC01));
    w.path_op(u, PathOp::Type(7));
    if w.path_op(u, PathOp::Compute) == 0 {
        landing_msg(w, u, skill);
        w.path_op(u, PathOp::MoveMask(m0));
        return 0;
    }
    let v = walk_velocity(w, ct, u);
    w.path_op(u, PathOp::Velocity(v));
    w.path_op(u, PathOp::MoveMask(0x401));
    if w.path_point_count(u) < 1 {
        landing_msg(w, u, skill);
        w.path_op(u, PathOp::MoveMask(m0));
        return 0;
    }
    let (x, y) = w.path_last_point(u);
    if let Some(room) = w.unit_room(u) {
        w.effect(BodyEffect::PatternStamp {
            room,
            x,
            y,
            u,
            mask: if player { 0x80 } else { 0x100 },
        });
    }
    set_uninterruptable(w, u, true);
    w.state_on(u, st::SKILL_MOVE, true);
    w.set_entry_flags(u, &e, 1);
    w.set_entry_param_of(u, &e, 1, x);
    w.set_entry_param_of(u, &e, 2, y);
    w.set_entry_param_of(u, &e, 3, -1);
    w.set_entry_param_of(u, &e, 4, 0);
    let a = s16(r.aurastate);
    if state_ok(w, a) {
        clear_group(w, u, a, false);
        let d = eval(w, t, u, r.auralencalc, skill, lvl);
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
                callback: callback::WHIRLWIND,
            },
        ) else {
            return 0;
        };
        aura_fill(w, t, u, l, skill, lvl);
        passive_fill(w, t, u, l, skill, lvl);
        w.list_set(l, stat::MODIFIERLIST_SKILL, skill);
        w.list_set(l, stat::MODIFIERLIST_LEVEL, lvl);
        reregister(w, t, u, skill, lvl, a);
        w.mark_state_changed(u, a);
    }
    1
}

/// srvdo 76 Whirlwind.
pub fn whirlwind<W: BodyWorld>(
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
    let (rf, hf, hc, calc1, s) = (
        r.resultflags,
        r.hitflags,
        r.hitclass,
        r.calc1,
        srcdam_or_128(r),
    );
    let Some(e) = w.find_entry(u, skill) else {
        return 0;
    };
    let (x, y) = (w.entry_param(u, &e, 1), w.entry_param(u, &e, 2));
    if x == 0 || y == 0 {
        ww_end(w, t, u, &e, skill, (x, y));
        return 0;
    }
    let flags = w.entry_flags(u, &e);
    if flags & 3 == 3 {
        ww_end(w, t, u, &e, skill, (x, y));
        return 1;
    }
    if flags & 1 == 0 || !w.is_alive(u) {
        return 0;
    }
    if w.unit_type(u) != UnitType::Player {
        w.set_frame_event_index(u, 0);
        w.set_frame_count(u, 0x400);
    } else {
        w.anim_from(u, 3);
    }
    let n = ww_pacing(w, u, &e);
    for _ in 0..n {
        let p3 = w.entry_param(u, &e, 3);
        let k = next_unit(w, t, ct, u, (0, 0), 5, 3, p3 as u32).0;
        match k {
            None => w.set_entry_param_of(u, &e, 3, -1),
            Some(k) => {
                let g = guid(w, k);
                w.set_entry_param_of(u, &e, 3, g as i32);
                let mut record = skill_melee(w, t, ct, u, k, skill, lvl);
                if record.result & 1 != 0 {
                    record.result |= rf;
                    record.hit_flags |= hf;
                    if hc != 0 {
                        record.hit_class = hc;
                    }
                    record.enh_pct = eval(w, t, u, calc1, skill, lvl);
                    roll_elemental(w, t, u, &mut record, skill, lvl);
                }
                start_combat(w.combat(), t, ct, Some(u), Some(k), &mut record, s);
                apply_melee(w.combat(), ct, u, k);
            }
        }
        let f = w.entry_flags(u, &e);
        w.set_entry_flags(u, &e, f ^ 0x2000);
        if k.is_none() {
            break;
        }
    }
    1
}

// ------------------------------------------------------------------

/// srvst 39 Berserk.
pub fn berserk<W: BodyWorld>(
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
    let (rf, hf, hc, calc1, calc2, s, a) = (
        r.resultflags,
        r.hitflags,
        r.hitclass,
        r.calc1,
        r.calc2,
        srcdam_or_128(r),
        s16(r.aurastate),
    );
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let mut record = skill_melee(w, t, ct, u, tg, skill, lvl);
    if record.result & 1 != 0 {
        record.result |= rf;
        record.hit_flags |= hf;
        if hc != 0 {
            record.hit_class = hc;
        }
        record.enh_pct = eval(w, t, u, calc1, skill, lvl);
        convert(w, t, u, &mut record, skill, lvl);
        roll_elemental(w, t, u, &mut record, skill, lvl);
    }
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, s);
    if state_ok(w, a) {
        let f = w.frame();
        // `calc2` base ticks, or 10 when that is not in the future.
        let len = eval(w, t, u, calc2, skill, lvl);
        let mut e = f.wrapping_add(super::ticks(w, len));
        if e <= f {
            e = super::after(w, 10);
        }
        let Some(l) = list_or_new(w, u, a, 2, e, u, callback::DEFAULT) else {
            return 1;
        };
        w.set_list_expire(l, e);
        w.combat().schedule_timer(u, 12, e);
        aura_fill(w, t, u, l, skill, lvl);
    }
    1
}

// ------------------------------------------------------------------

/// srvst 28 Blade Shield.
pub fn blade_shield_start<W: BodyWorld>(
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
    let (len, a) = (r.auralencalc, s16(r.aurastate));
    if w.find_entry(u, skill).is_none() || prog_missile(w, t, u, skill) <= 0 {
        return 0;
    }
    if eval(w, t, u, len, skill, lvl) <= 0 || !state_ok(w, a) {
        return 0;
    }
    clear_group(w, u, a, false);
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
    aura_fill(w, t, u, l, skill, lvl);
    w.list_set(l, stat::MODIFIERLIST_SKILL, skill);
    w.list_set(l, stat::MODIFIERLIST_LEVEL, lvl);
    w.mark_state_changed(u, a);
    1
}

/// srvdo 54 Blade Shield.
pub fn blade_shield<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(a) = rec(t, skill).map(|r| s16(r.aurastate)) else {
        return 0;
    };
    if !state_ok(w, a) || w.find_entry(u, skill).is_none() {
        return 0;
    }
    if w.room(u) != RoomKind::Town {
        blade_pulse(w, t, ct, u, skill, lvl);
    }
    1
}

//! Batch 3 bodies of required level 6: Static Field, Telekinesis,
//! Poison Dagger, Corpse Explosion, Leap, Double Swing, Taunt, Shock
//! Field, Blade Sentinel, Dragon Claw.

use super::dos::{can_switch, curse_unit, CurseCtx};
use super::effects::BodyEffect;
use super::helpers::*;
use super::helpers2::*;
use super::helpers3::*;
use super::starts::bash;
use super::starts2::corpse_any;
use super::BodyWorld;
use crate::combat::{
    apply, apply_melee, pct, start_combat, CombatTables, CombatWorld, DamageRecord, RoomKind,
};
use crate::monsters::init::stats_by_level;
use crate::skills::{add_element, elem_len, roll_elemental, roll_physical, SkillTables};
use crate::units::UnitType;

// -----------------------------------------------------------------

/// srvdo 20 Static Field.
pub fn static_field<W: BodyWorld>(
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
    let Some(dl) = ct.difficulty(w.combat().difficulty()) else {
        return 0;
    };
    let cap0 = dl.staticfieldmin as i32;
    let floor = eval(w, t, u, r.calc2, skill, lvl);
    let pc = eval(w, t, u, r.calc1, skill, lvl);
    let cap = if w.combat().expansion() { cap0 } else { 0 };
    let len = elem_len(w, t, Some(u), skill, lvl);
    let e = i32::from(r.etype);
    let range = eval(w, t, u, r.aurarangecalc, skill, lvl);
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
            let h = w.stat(x, sid::LIFE, 0) >> 8;
            if h < 1 {
                return 0;
            }
            if cap != 0 && h <= pct(w.stat_max(x, sid::LIFE) >> 8, cap, 100) {
                return 0;
            }
            let mut v = pct(h, pc, 100);
            if h.wrapping_sub(v) < 1 {
                v = h - 1;
            }
            v = v.wrapping_shl(8);
            if v < floor {
                v = floor;
            }
            let mut probe = DamageRecord::default();
            let added = add_element(w, u, &mut probe, e, v, len);
            if added.resist >= 0 {
                let res = w.stat(x, added.resist as u16, 0);
                if res < 0 {
                    v = pct(v, 100, 100 - res);
                }
            }
            let mut record = DamageRecord::default();
            add_element(w, u, &mut record, added.element, v, len);
            record.hit_class |= 0xD;
            record.hit_class_fixed = 1;
            record.result = 0x4001;
            apply(w.combat(), ct, u, x, true, &mut record);
            w.combat().reaction(u, x, &mut record);
            1
        },
    );
    1
}

// -----------------------------------------------------------------

/// srvdo 21 Telekinesis.
pub fn telekinesis<W: BodyWorld>(
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
    let (p2, rf, hf, hc) = (r.param2 as i32, r.resultflags, r.hitflags, r.hitclass);
    if w.unit_type(u) != UnitType::Player || w.inventory_busy(u) {
        return 0;
    }
    let Some(tg) = target(w, u) else {
        return 0;
    };
    flag_40(w, u);
    match w.unit_type(tg) {
        UnitType::Player | UnitType::Monster => {
            if !w.combat().hostile(u, tg)
                || w.room(tg) == RoomKind::Town
                || w.room(u) == RoomKind::Town
            {
                return 0;
            }
            let knock = (w.seed(u).roll(100) as i32) < p2;
            let mut record = DamageRecord::default();
            roll_physical(w, t, u, &mut record, skill, lvl);
            roll_elemental(w, t, u, &mut record, skill, lvl);
            record.result |= rf;
            record.hit_flags |= hf;
            if hc != 0 {
                record.hit_class = hc;
            }
            if knock {
                record.result |= 9;
            }
            apply(w.combat(), ct, u, tg, true, &mut record);
            w.combat().reaction(u, tg, &mut record);
        }
        UnitType::Object => w.effect(BodyEffect::OperateObject { u, object: tg }),
        UnitType::Item => {
            let pick = w
                .as_item(tg)
                .is_some_and(|i| [22, 4, 38, 56, 9, 41].iter().any(|&ty| w.item_is(i, ty)));
            if pick {
                w.effect(BodyEffect::AutoPickup { u, item: tg });
            } else {
                w.effect(BodyEffect::Sound { u, id: 0x13 });
            }
        }
        _ => {}
    }
    1
}

// -----------------------------------------------------------------

/// srvst 16 Poison Dagger.
pub fn poison_dagger_start<W: BodyWorld>(
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
    if w.room(tg) == RoomKind::Town {
        return 0;
    }
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let (calc1, s) = (r.calc1, srcdam_or_128(r));
    let mut record = skill_melee(w, t, ct, u, tg, skill, lvl);
    if record.result & 1 != 0 {
        record.enh_pct = eval(w, t, u, calc1, skill, lvl);
        if convert(w, t, u, &mut record, skill, lvl) {
            roll_elemental(w, t, u, &mut record, skill, lvl);
        }
    }
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, s);
    1
}

/// srvdo 32 Poison Dagger.
pub fn poison_dagger<W: BodyWorld>(w: &mut W, ct: &CombatTables, u: W::Unit) -> i32 {
    flag_40(w, u);
    let Some(tg) = target(w, u) else {
        return 0;
    };
    apply_melee(w.combat(), ct, u, tg);
    1
}

// -----------------------------------------------------------------

/// srvdo 55 Corpse Explosion.
pub fn corpse_explosion<W: BodyWorld>(
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
    if !corpse_any(w, ct, tg) {
        return 0;
    }
    w.state_on(tg, st::CORPSE_NODRAW, true);
    w.queue_update(tg);
    if skill == 0 {
        return 0;
    }
    // An out-of-range skill reads a null record: fatal; refused.
    let Some(r) = rec(t, skill).cloned() else {
        return 0;
    };
    let mut h = w.base_stat(tg, sid::MAXHP, 0);
    if w.unit_type(tg) == UnitType::Monster {
        if let Some(ms) = ct.monstats(w.class_id(tg)) {
            let d = w.combat().difficulty();
            let lv = w.stat(tg, sid::LEVEL, 0);
            let s = stats_by_level(ms, w.monlvl(), w.l_flag(), d, lv);
            h = s.min_hp.wrapping_add(s.max_hp).wrapping_shl(7);
        }
    }
    let (x, y) = w.position(tg);
    let range = eval(w, t, u, r.aurarangecalc, skill, lvl);
    let (r1, r2) = (range / 2, range.wrapping_add(1) / 2);
    let lo = pct(h, eval(w, t, u, r.calc1, skill, lvl), 100);
    let hi = pct(h, eval(w, t, u, r.calc2, skill, lvl), 100);
    let mut v = lo.wrapping_add(w.seed(tg).roll(hi.wrapping_sub(lo)) as i32);
    let tl = w.stat(tg, sid::LEVEL, 0);
    let ul = w.stat(u, sid::LEVEL, 0);
    if tl != 0 && ul < tl {
        v = pct(v, ul, tl);
    }
    let mut d = DamageRecord::default();
    let p = eval(w, t, u, r.calc3, skill, lvl).clamp(0, 100);
    if p > 0 && r.etype != 0 {
        let len = elem_len(w, t, Some(u), skill, lvl);
        add_element(w, u, &mut d, i32::from(r.etype), pct(v, p, 100), len);
        v = pct(v, 100 - p, 100);
    }
    d.physical = v;
    let rr = r1.wrapping_mul(r1);
    scan_unit(w, t, ct, u, (x, y), r2, r.aurafilter, false, &mut |w, k| {
        let mut c = d;
        if dist_sq((x, y), w.position(k)) > rr {
            c.physical = 0;
        }
        apply(w.combat(), ct, u, k, true, &mut c);
        w.combat().reaction(u, k, &mut c);
        1
    });
    1
}

// -----------------------------------------------------------------

/// srvst 40 Leap.
pub fn leap_start<W: BodyWorld>(
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
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    let monster = w.unit_type(u) == UnitType::Monster;
    let mut tg = None;
    let (x, y) = if !monster {
        let Some(p) = tpos(w, u) else {
            return 0;
        };
        if !leap_room_test(w, t, u, skill, p) {
            return 0;
        }
        let Some(p) = leap_clamp(w, t, u, skill, lvl, p) else {
            return 0;
        };
        p
    } else {
        tg = target(w, u);
        let p = match tg {
            None => {
                let Some(p) = tpos(w, u) else {
                    w.set_used_skill(u, None);
                    return 0;
                };
                w.set_entry_param_of(u, &e, 4, -1);
                p
            }
            Some(k) => {
                monster_prehit(w, t, ct, u, k, &e);
                if !w.is_alive(u) {
                    return 0;
                }
                let (kx, ky) = w.position(k);
                let (ux, uy) = w.position(u);
                (
                    kx.wrapping_mul(2).wrapping_sub(ux),
                    ky.wrapping_mul(2).wrapping_sub(uy),
                )
            }
        };
        let size = w.unit_size(u);
        let found = w
            .unit_room(u)
            .and_then(|room| w.free_point(room, p, size, 0x3C01, false));
        let Some((_, p)) = found else {
            w.set_used_skill(u, None);
            return 0;
        };
        p
    };
    if let Some(room) = w.unit_room(u).and_then(|r| w.room_at(r, x, y)) {
        w.effect(BodyEffect::PatternStamp {
            room,
            x,
            y,
            u,
            mask: 0x80,
        });
    }
    set_uninterruptable(w, u, true);
    w.state_on(u, st::SKILL_MOVE, true);
    w.set_entry_param_of(u, &e, 1, x);
    w.set_entry_param_of(u, &e, 2, y);
    if let (true, Some(k)) = (monster, tg) {
        let (ty, g) = (type_index(w, k), guid(w, k));
        w.set_entry_param_of(u, &e, 3, ty);
        w.set_entry_param_of(u, &e, 4, g as i32);
    }
    w.set_entry_flags(u, &e, leap::LAUNCH);
    1
}

/// srvdo 77 Leap.
pub fn leap<W: BodyWorld>(
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
    let Some(e) = w.find_entry(u, skill) else {
        return 0;
    };
    w.delete_timers(u, 1, 0);
    let f = w.entry_flags(u, &e);
    if f & 0x100 != 0 {
        let landed = leap_land(w, ct, u, &e) != 0;
        if landed && w.unit_type(u) == UnitType::Player {
            let record = DamageRecord {
                result: 9,
                ..DamageRecord::default()
            };
            let r = eval(w, t, u, calc1, skill, lvl);
            area_damage(w, t, ct, u, (0, 0), r, &record, 0);
        }
        return 1;
    }
    if f & 0x80 != 0 {
        return leap_launch(w, ct, u, &e);
    }
    if w.unit_type(u) == UnitType::Player {
        w.set_entry_flags(u, &e, 0);
    }
    1
}

// -----------------------------------------------------------------

/// srvdo 70 Double Swing.
pub fn double_swing<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(mut tg) = target(w, u) else {
        return 0;
    };
    if w.frame_event_index(u) % 2 != 0 {
        flag_40(w, u);
        let r = w.combat().melee_range(u).wrapping_add(4);
        let g = guid(w, tg);
        let Some(t2) = next_unit(w, t, ct, u, (0, 0), r, 0x20003, g).0 else {
            return 0;
        };
        w.path_op(u, super::effects::PathOp::TargetUnit(Some(t2)));
        tg = t2;
    }
    bash(w, t, ct, u, skill, lvl);
    apply_melee(w.combat(), ct, u, tg);
    1
}

// -----------------------------------------------------------------

/// Taunt test.
fn taunt_test<W: BodyWorld>(w: &mut W, ct: &CombatTables, x: W::Unit) -> bool {
    w.unit_type(x) == UnitType::Monster
        && can_switch(w, ct, x, 12)
        && w.is_alive(x)
        && w.minion_owner_ident(x).is_none_or(|(_, ty)| ty != 0)
}

/// srvdo 71 Taunt.
pub fn taunt<W: BodyWorld>(
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
    let ts = s16(r.auratargetstate);
    if !state_ok(w, ts) {
        return 0;
    }
    let mut tg = target(w, u);
    if let Some(x) = tg {
        if !taunt_test(w, ct, x) || !w.combat().hostile(u, x) {
            tg = None;
        }
    }
    let tg = match tg {
        Some(x) => x,
        None => {
            let test = |w: &mut W, x: W::Unit| taunt_test(w, ct, x);
            let Some(x) = nearest(w, t, ct, u, 20, &test) else {
                return 0;
            };
            x
        }
    };
    let d = eval(w, t, u, r.auralencalc, skill, lvl);
    let mut cx = CurseCtx {
        ai: true,
        upd: false,
        skill,
        lvl,
        duration: d,
        stats: [-1; 6],
        values: [0; 6],
        state: ts,
        events: [(0, 0); 3],
    };
    let pairs = [
        (r.aurastat1, r.aurastatcalc1),
        (r.aurastat2, r.aurastatcalc2),
        (r.aurastat3, r.aurastatcalc3),
        (r.aurastat4, r.aurastatcalc4),
        (r.aurastat5, r.aurastatcalc5),
        (r.aurastat6, r.aurastatcalc6),
    ];
    for (i, (s, c)) in pairs.into_iter().enumerate() {
        let s = s16(s);
        if !stat_ok(t, s) {
            cx.stats[i] = -1;
            break;
        }
        let Some(info) = w.stat_info(s) else {
            break;
        };
        if info.updateanimrate {
            cx.upd = true;
        }
        cx.stats[i] = s;
        cx.values[i] = eval(w, t, u, c, skill, lvl);
    }
    curse_unit(w, t, ct, u, &cx, tg);
    if w.unit_type(tg) == UnitType::Monster {
        w.effect(BodyEffect::AiParams {
            m: tg,
            p0: -666,
            p1: -666,
            p2: -666,
        });
        w.effect(BodyEffect::LeashOwner { m: tg, owner: None });
    }
    let f = w.frame();
    w.delete_timers(tg, 2, 0);
    w.schedule(tg, 2, f.wrapping_add(1), 0, 0);
    w.path_op(tg, super::effects::PathOp::TargetUnit(Some(u)));
    1
}

// ------------------------------------------------------------------

/// srvdo 43 Shock Field.
pub fn shock_field<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(range) = rec(t, skill).map(|r| r.aurarangecalc) else {
        return 0;
    };
    let m = prog_missile(w, t, u, skill);
    if m < 0 {
        return 0;
    }
    let n = prog_count(w, t, u, skill, lvl);
    if n <= 0 {
        return 0;
    }
    let r = eval(w, t, u, range, skill, lvl);
    scatter(w, u, m, n, r, skill, lvl)
}

// ------------------------------------------------------------------

/// srvdo 44 Blade Sentinel.
pub fn blade_sentinel<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    if rec(t, skill).is_none() || summon_class(w, t, ct, u, skill).0 < 0 {
        return 0;
    }
    flag_40(w, u);
    let Some((tx, ty)) = tpos(w, u) else {
        return 0;
    };
    let Some(m) = sentry(w, t, ct, u, (0, 0), skill, lvl) else {
        return 0;
    };
    let ul = w.stat(u, sid::LEVEL, 0);
    w.set_stat(m, sid::LEVEL, ul);
    let (ux, uy) = w.position(u);
    let room = w.unit_room(u);
    w.place_unit(m, room, (ux, uy));
    w.effect(BodyEffect::AiCommand {
        m,
        kind: 0,
        x: ux,
        y: uy,
        tx,
        ty,
    });
    let f = w.frame();
    w.delete_timers(m, 2, 0);
    w.schedule(m, 2, f.wrapping_add(1), 0, 0);
    flags_clear(w, m, 0x8);
    1
}

// ------------------------------------------------------------------

/// srvst 25 Dragon Claw.
pub fn dragon_claw_start<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    i32::from(target(w, u).is_some())
}

/// srvdo 46 Dragon Claw.
pub fn dragon_claw<W: BodyWorld>(
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
    claw_hit(w, t, ct, u, tg, skill, lvl);
    if let Some(p) = pair_damage(w, u, tg) {
        finisher(w, t, ct, u, &p);
    }
    apply_melee(w.combat(), ct, u, tg);
    if w.frame_event_index(u) % 2 == 0 {
        flags_clear(w, u, FLAG_40);
    } else {
        flag_40(w, u);
    }
    1
}

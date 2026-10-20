//! Batch 3 bodies of required level 1: Jab, Charged Bolt,
//! Sacrifice, Smite, Find Potion, Raven, Firestorm, Psychic Hammer,
//! Dragon Talon.

use super::effects::BodyEffect;
use super::helpers::*;
use super::helpers2::*;
use super::helpers3::*;
use super::starts2::corpse_soft;
use super::{init_cb, BodyWorld, MissileRequest};
use crate::combat::{
    apply, apply_melee, bonuses, melee_result, pct, start_combat, CombatTables, CombatWorld,
    DamageRecord, RoomKind,
};
use crate::skills::{phys_max, phys_min, roll_elemental, roll_physical, SkillTables};
use crate::units::UnitType;

/// srvdo 7 Jab.
pub fn jab<W: BodyWorld>(
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
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let h = skill_to_hit(w, t, u, skill, lvl);
    if w.unit_type(u) == UnitType::Monster {
        mode_damage(w, t, ct, u, 8);
    }
    let mut record = melee_rec(w, t, ct, u, tg, h, 0);
    if record.result & 1 != 0 {
        record.enh_pct = eval(w, t, u, calc1, skill, lvl);
        convert(w, t, u, &mut record, skill, lvl);
        roll_elemental(w, t, u, &mut record, skill, lvl);
    }
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, s);
    apply_melee(w.combat(), ct, u, tg);
    1
}

/// srvdo 17 Charged Bolt.
pub fn charged_bolt<W: BodyWorld>(
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
    let m = prog_missile(w, t, u, skill);
    if !missile_ok(t, m) {
        return 0;
    }
    let n = eval(w, t, u, calc1, skill, lvl);
    let (x, y) = w.position(u);
    let mut req = MissileRequest {
        flags: 0x21,
        x,
        y,
        skill,
        level: lvl,
        ..MissileRequest::new(u, m)
    };
    for i in 0..n {
        req.init = Some((init_cb::JITTER, i as u32));
        if let Some((tx, ty)) = tpos(w, u) {
            req.target_x = tx;
            req.target_y = ty;
            w.spawn_missile(req);
        }
    }
    1
}

/// srvdo 64 Sacrifice.
pub fn sacrifice<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(calc2) = rec(t, skill).map(|r| r.calc2) else {
        return 0;
    };
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let Some(p) = pair_damage(w, u, tg) else {
        return 1;
    };
    let v = p.physical.min(w.stat(tg, sid::LIFE, 0));
    let s = pct(v, eval(w, t, u, calc2, skill, lvl), 100);
    let mut record = DamageRecord {
        hit_flags: 0x1000,
        physical: s,
        total: s,
        ..DamageRecord::default()
    };
    apply(w.combat(), ct, u, u, false, &mut record);
    w.combat().reaction(u, u, &mut record);
    apply_melee(w.combat(), ct, u, tg);
    1
}

/// srvdo 150 Smite.
pub fn smite<W: BodyWorld>(
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
    let (calc1, calc2, rf, hf) = (r.calc1, r.calc2, r.resultflags, r.hitflags);
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if !w.combat().in_melee_range(u, tg, 0) {
        return 0;
    }
    let mut record = DamageRecord::default();
    if w.unit_type(u) == UnitType::Player {
        let Some(sh) = w.shield(u) else {
            return 0;
        };
        let Some((smin, smax)) = w.shield_damage(sh) else {
            return 0;
        };
        let (mut mn, mut mx) = (smin.wrapping_shl(8), smax.wrapping_shl(8));
        if w.has_state(u, st::HOLYSHIELD as u16) {
            if let Some(h) = w.state_list(u, st::HOLYSHIELD) {
                let (k, l) = w.list_skill(h);
                mn = mn.wrapping_add(phys_min(w, t, Some(u), k, l, true));
                mx = mx.wrapping_add(phys_max(w, t, Some(u), k, l, true));
            }
        }
        let p = eval(w, t, u, calc1, skill, lvl);
        record.physical = bonuses(w.combat(), t, u, false, Some(sh), mn, mx, p, 0, 128);
        let stun = eval(w, t, u, calc2, skill, lvl);
        convert(w, t, u, &mut record, skill, lvl);
        roll_elemental(w, t, u, &mut record, skill, lvl);
        record.stun_len = stun;
        record.hit_flags = 2;
        record.result = melee_result(w.combat(), t, ct, Some(u), Some(tg), 0, 0) | 1;
    } else {
        mode_damage(w, t, ct, u, 5);
        record.stun_len = eval(w, t, u, calc2, skill, lvl);
        record.result = melee_result(w.combat(), t, ct, Some(u), Some(tg), 0, 0);
    }
    if record.result & 1 != 0 {
        record.result |= rf;
        record.hit_flags |= hf;
    }
    record.hit_class = 0x65;
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, 128);
    apply_melee(w.combat(), ct, u, tg);
    1
}

/// srvdo 69 Find Potion.
pub fn find_potion<W: BodyWorld>(
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
    if !corpse_soft(w, ct, tg) || w.has_state(tg, st::CORPSE_NOSELECT as u16) {
        return 0;
    }
    w.state_on(tg, st::CORPSE_NOSELECT, true);
    w.queue_update(tg);
    let p = eval(w, t, u, calc1, skill, lvl);
    let r = w.seed(u).roll(100) as i32;
    if r < p {
        // A code of 0 has no items index: nothing is dropped.
        if let Some(code) = potion_code(w, t, u, skill) {
            w.effect(BodyEffect::DropItem {
                at: tg,
                code,
                quality: 2,
            });
        }
    }
    1
}

/// srvdo 114 Raven.
pub fn raven<W: BodyWorld>(
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
    let (pet, petmax, calc2) = (i32::from(r.pettype as i8), r.petmax, r.calc2);
    let (c, mode) = summon_class(w, t, ct, u, skill);
    if c < 0 || pet < 0 || pet >= w.pettype_count() {
        return 0;
    }
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
            pet_type: pet,
            pet_max: pm,
        },
    ) else {
        return 0;
    };
    flags_clear(w, m, 0x4);
    w.set_stat(m, sid::HPREGEN, 0);
    let p = eval(w, t, u, calc2, skill, lvl).max(1);
    base_stats(w, u, m, p, lvl);
    skill_stats(w, t, u, m, skill, lvl, 0);
    1
}

/// srvdo 117 Firestorm.
pub fn firestorm<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
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
    fan(w, u, n, m, skill, lvl, true)
}

/// srvst 22 Psychic Hammer.
pub fn psychic_hammer_start<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if w.room(u) == RoomKind::Town || w.room(tg) == RoomKind::Town {
        return 0;
    }
    i32::from(matches!(
        w.unit_type(tg),
        UnitType::Player | UnitType::Monster
    ))
}

/// srvdo 33 Psychic Hammer.
pub fn psychic_hammer<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    if rec(t, skill).is_none() || psychic_hammer_start(w, u) == 0 {
        return 0;
    }
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let mut record = DamageRecord::default();
    roll_physical(w, t, u, &mut record, skill, lvl);
    roll_elemental(w, t, u, &mut record, skill, lvl);
    apply(w.combat(), ct, u, tg, true, &mut record);
    record.result |= 1;
    let k = knock_chance(w, t, u, tg, skill, lvl, None);
    if k > 0 && ((w.seed(u).step() % 100) as i32) < k {
        record.result |= 8;
    }
    if w.stat(tg, sid::LIFE, 0) < 256 {
        record.result |= 2;
    }
    w.combat().reaction(u, tg, &mut record);
    1
}

/// srvst 24 Dragon Talon.
pub fn dragon_talon_start<W: BodyWorld>(
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
    let Some(tg) = target(w, u) else {
        w.set_entry_param_of(u, &e, 1, 0);
        return 0;
    };
    if !w.combat().in_melee_range(u, tg, 0) {
        return 0;
    }
    let n = eval(w, t, u, calc1, skill, lvl);
    w.set_entry_param_of(u, &e, 1, n.wrapping_sub(1));
    kick_hit(w, t, ct, u, tg, skill, lvl, n.wrapping_sub(1) == 0);
    1
}

/// srvdo 42 Dragon Talon.
pub fn dragon_talon<W: BodyWorld>(
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
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if let Some(p) = pair_damage(w, u, tg) {
        finisher(w, t, ct, u, &p);
    }
    apply_melee(w.combat(), ct, u, tg);
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    let n = w.entry_param(u, &e, 1).wrapping_sub(1);
    if n < 0 {
        return 0;
    }
    w.set_entry_param_of(u, &e, 1, n);
    let mut knock = false;
    if n == 0 {
        let k = knock_chance(w, t, u, tg, skill, lvl, Some(100));
        if k >= 1 && (w.seed(u).roll(100) as i32) < k {
            knock = true;
        }
    }
    kick_hit(w, t, ct, u, tg, skill, lvl, knock);
    apply_melee(w.combat(), ct, u, tg);
    if n > 0 {
        flags_clear(w, u, FLAG_40);
        w.anim_rewind(u, 100);
    }
    1
}

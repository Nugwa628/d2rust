//! The start functions (`srvstfunc`) of `functions.tsv` status
//! `spec'd-here`. Each returns the value the start core reads (0 =
//! refused).

use super::helpers::*;
use super::BodyWorld;
use crate::combat::{
    bonuses, melee_result, start_combat, CombatTables, CombatWorld, DamageRecord, RoomKind,
};
use crate::skills::{roll_elemental, to_hit, SkillTables};
use crate::units::UnitType;

/// The zeroed record with the melee result of `to_hit(unit, skill, L)`.
fn melee<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    tg: W::Unit,
    skill: i32,
    lvl: i32,
) -> DamageRecord {
    let bonus = to_hit(w, t, Some(u), skill, lvl);
    DamageRecord {
        result: melee_result(w.combat(), t, ct, Some(u), Some(tg), bonus, 0),
        ..DamageRecord::default()
    }
}

/// `EType` ≠ 0 and `eval(calc4)` > 0 → conversion % and element.
fn conversion<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    rec_: &mut DamageRecord,
    skill: i32,
    lvl: i32,
) {
    let Some(r) = rec(t, skill) else { return };
    let (etype, calc4) = (r.etype, r.calc4);
    if etype != 0 {
        let c = eval(w, t, u, calc4, skill, lvl);
        if c > 0 {
            rec_.conv_pct = c;
            rec_.conv_elem = etype as i8;
        }
    }
}

/// 1 Attack, Left Hand Swing.
pub fn attack<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    lvl: i32,
) -> i32 {
    let fb = w.frame_bonus(u);
    w.set_anim_frame(u, fb.wrapping_shl(8));
    if w.has_group(u, group::MELEEONLY) {
        return shape_start(w, t, ct, u, lvl);
    }
    if is_bow(w, u) && !has_ammo(w, u) {
        w.attack_cleanup(u);
        w.weapon_cleanup(u);
        return 0;
    }
    1
}

/// 2 Kick.
pub fn kick<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
) -> i32 {
    let Some(tg) = target(w, u) else {
        return 0;
    };
    // An invalid skill reads a null record: fatal in the base game; D2RUST
    // refuses.
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let mut record = DamageRecord {
        hit_flags: 2,
        result: 9,
        physical: (r.mindam as i32).wrapping_shl(u32::from(r.hitshift)),
        hit_class: 1,
        ..DamageRecord::default()
    };
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, 128);
    1
}

/// 3 Unsummon.
pub fn unsummon<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if w.unit_type(tg) != UnitType::Monster
        || w.unit_type(u) != UnitType::Player
        || w.minion_owner(tg) != Some(u)
        || !w.pet_unsummonable(u, tg)
        || w.used_skill(u).is_none()
    {
        return 0;
    }
    let guid = w.combat().ident(tg).1;
    w.set_entry_param(u, 1, guid as i32);
    1
}

/// 4 Arrow/Bolt, 65 Throw.
pub fn ammo<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    if has_ammo(w, u) {
        return 1;
    }
    w.attack_cleanup(u);
    w.weapon_cleanup(u);
    0
}

/// 5 Jab.
pub fn jab<W: BodyWorld>(w: &mut W, t: &SkillTables, u: W::Unit, skill: i32) -> i32 {
    if rec(t, skill).is_none() {
        return 0;
    }
    i32::from(target(w, u).is_some())
}

/// A monster corpse with no `udead` state and monstats2 `corpseSel`
/// (common part).
fn corpse<W: BodyWorld>(w: &W, ct: &CombatTables, tg: W::Unit) -> bool {
    w.unit_type(tg) == UnitType::Monster
        && w.mode(tg) == 12
        && !w.has_group(tg, group::UDEAD)
        && ct.monstats2(w.class_id(tg)).is_some_and(|m| m.corpsesel)
}

/// 15 Raise Skeleton, Raise Skeletal Mage.
pub fn raise<W: BodyWorld>(w: &mut W, ct: &CombatTables, u: W::Unit) -> i32 {
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if w.room(tg) == RoomKind::Town {
        return 0;
    }
    let ok = corpse(w, ct, tg) && ct.monstats(w.class_id(tg)).is_some_and(|m| m.velocity != 0);
    i32::from(ok)
}

/// 29 Sacrifice.
pub fn sacrifice<W: BodyWorld>(
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
    let (calc1, srcdam, resultflags, hitflags) = (r.calc1, r.srcdam, r.resultflags, r.hitflags);
    if w.used_skill(u).is_none() {
        return 0;
    }
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if !w.combat().in_melee_range(u, tg, 0) {
        return 0;
    }
    let mut record = melee(w, t, ct, u, tg, skill, lvl);
    if record.result & 1 != 0 {
        let p = eval(w, t, u, calc1, skill, lvl);
        record.physical = bonuses(w.combat(), t, u, true, None, 0, 0, p, 0, i32::from(srcdam));
        conversion(w, t, u, &mut record, skill, lvl);
        roll_elemental(w, t, u, &mut record, skill, lvl);
        record.result |= resultflags;
        record.hit_flags = hitflags | 1;
    }
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, 128);
    1
}

/// 32 Bash, Stun, Concentrate, Conversion, BearSmite.
pub fn bash<W: BodyWorld>(
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
    let (calc1, calc2, calc3) = (r.calc1, r.calc2, r.calc3);
    let (resultflags, hitflags, hitclass) = (r.resultflags, r.hitflags, r.hitclass);
    let (srcdam, aurastate) = (i32::from(r.srcdam), s16(r.aurastate));
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if w.room(tg) == RoomKind::Town {
        return 0;
    }
    // A TEMPONLY (flag 4) list: freed at the unit's next mode change.
    let rate = eval(w, t, u, calc3, skill, lvl);
    if let Some(l) = w.alloc_list(4, 0, Some(u)) {
        w.attach(u, l);
        w.list_set(l, stat::ATTACKRATE, rate);
        w.combat().refresh_anim_rate(u);
    }
    let mut record = melee(w, t, ct, u, tg, skill, lvl);
    if record.result & 1 != 0 {
        record.result |= resultflags;
        record.hit_flags |= hitflags;
        if hitclass != 0 {
            record.hit_class = hitclass;
        }
        record.enh_pct = eval(w, t, u, calc1, skill, lvl);
        conversion(w, t, u, &mut record, skill, lvl);
        roll_elemental(w, t, u, &mut record, skill, lvl);
    }
    let s = if srcdam == 0 { 128 } else { srcdam };
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, s);
    if let Some(i) = pair_record(w, u, tg) {
        let add = eval(w, t, u, calc2, skill, lvl).wrapping_shl(8);
        if let Some(e) = w.combat().combat_list(u).get_mut(i) {
            e.record.physical = e.record.physical.wrapping_add(add);
        }
    }
    if state_ok(w, aurastate) {
        let l = match w.state_list(u, aurastate) {
            Some(l) => Some(l),
            None => w.alloc_list(4, 0, Some(u)).inspect(|&l| {
                w.set_list_state(l, aurastate);
                w.attach(u, l);
                w.set_remove_callback(l, super::callback::DEFAULT);
            }),
        };
        if let Some(l) = l {
            w.state_on(u, aurastate, true);
            aura_fill(w, t, u, l, skill, lvl);
        }
    }
    1
}

/// 33 Find Potion, Grim Ward.
pub fn find_potion<W: BodyWorld>(w: &mut W, ct: &CombatTables, u: W::Unit) -> i32 {
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let v = corpse(w, ct, tg) && ct.monstats2(w.class_id(tg)).is_some_and(|m| m.soft);
    if v && w.has_state(tg, state::CORPSE_NOSELECT as u16) {
        return 0;
    }
    i32::from(v)
}

/// 46 AndrialSpray.
pub fn andrial_spray<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if w.used_skill(u).is_none() {
        return 0;
    }
    let (x, y) = w.position(tg);
    w.set_entry_param(u, 1, x);
    w.set_entry_param(u, 2, y);
    1
}

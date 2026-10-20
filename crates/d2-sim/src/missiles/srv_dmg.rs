//! The 14 server-damage functions (`pSrvDmgFunc` 1…14): each adjusts the
//! damage record of a missile hit before the damage application. Draws:
//! function 3 one inline step and function 14 one `roll(100)`, both on
//! the missile's seed; 5, 12 and 13 make `elem_roll`'s draws.

use crate::combat::{hitflag, result, DamageRecord};
use crate::game::Game;
use crate::units::{UnitId, UnitType};

use super::bodies::elem_roll;
use super::bodies_ext::skill_lin;
use super::hit::pct;
use super::{stat, Ctx, MissileWorld, SkillField};

/// Hit class the stun functions write (+0x60).
pub const STUN_HIT_CLASS: u32 = 0x60;
/// Unit flags (+0xC4) bits of function 6.
const OWNER_FLAG_0X200: u32 = 0x200;
const UNIT_FLAG_0X20000: u32 = 0x2_0000;

/// `add_elem(M, R, a)`: adds `a` to the record field of
/// the row's `EType`; 12 sets cold; 10 and > 12 nothing.
pub fn add_elem(rec: &mut DamageRecord, etype: u8, a: i32) {
    let f = match etype {
        0 => &mut rec.physical,
        1 => &mut rec.fire,
        2 => &mut rec.lightning,
        3 => &mut rec.magic,
        4 => &mut rec.cold,
        5 => &mut rec.poison,
        6 => &mut rec.life_leech,
        7 => &mut rec.mana_leech,
        8 => &mut rec.stamina_leech,
        9 => &mut rec.stun_len,
        11 => &mut rec.burn,
        12 => {
            rec.cold = a;
            return;
        }
        _ => return,
    };
    *f = f.wrapping_add(a);
}

/// `clear_elems(M, R)`: the bypass bits and every amount
/// to 0, then the lengths by `EType` (4 / 12 keep cold and freeze, 5
/// keeps poison, 11 keeps burn; any other clears all four and freeze).
pub fn clear_elems(rec: &mut DamageRecord, etype: u8) {
    rec.hit_flags &= !(hitflag::BYPASS_UNDEAD | hitflag::BYPASS_DEMONS | hitflag::BYPASS_BEASTS);
    rec.physical = 0;
    rec.fire = 0;
    rec.burn = 0;
    rec.lightning = 0;
    rec.magic = 0;
    rec.cold = 0;
    rec.poison = 0;
    rec.life_leech = 0;
    rec.mana_leech = 0;
    rec.stamina_leech = 0;
    rec.stun_len = 0;
    match etype {
        4 | 12 => {
            rec.poison_len = 0;
            rec.burn_len = 0;
        }
        5 => {
            rec.cold_len = 0;
            rec.freeze_len = 0;
            rec.burn_len = 0;
        }
        11 => {
            rec.cold_len = 0;
            rec.poison_len = 0;
            rec.freeze_len = 0;
        }
        _ => {
            rec.cold_len = 0;
            rec.poison_len = 0;
            rec.burn_len = 0;
            rec.freeze_len = 0;
        }
    }
}

/// The part of the physical damage converted to the element (functions
/// 1 and 12): c := min(c, 100), f := min(pct(phys, c), phys).
fn converted(phys: i32, c: i32) -> i32 {
    pct(phys, c.min(100)).min(phys)
}

/// Runs server-damage function `index` (1…14) on `rec` (missile `m`
/// hitting `unit`).
pub fn run<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    index: i16,
    m: UnitId,
    unit: UnitId,
    rec: &mut DamageRecord,
) {
    // "Row valid": class in range and record found.
    let row = cx.row_of(m).cloned();
    let (skill, level, data28) = cx.store.get(m).map_or((0, 0, 0), |d| {
        (i32::from(d.skill), i32::from(d.level), d.target.0)
    });
    let owner = cx.owner(game, m);
    let present = game.lists.unit(unit).is_some();
    let ty = game.lists.unit(unit).map(|e| e.ty);
    match index {
        // firearrow, magicarrow, coldarrow.
        1 => {
            let Some(row) = row else { return };
            let c = cx.world.missile_calc(game, m, owner, row.dmgcalc1, -1, -1);
            if c > 0 {
                let f = converted(rec.physical, c);
                rec.physical = rec.physical.wrapping_sub(f);
                add_elem(rec, row.etype, f);
            }
        }
        // icearrow, royalstrikechaosice.
        2 => {
            let Some(row) = row else { return };
            let len = cx.world.stat(m, stat::COLDLENGTH);
            rec.freeze_len = pct(len, (row.dparam1 as i32).max(0));
            rec.cold_len = 0;
        }
        // blaze, firewall and kin: one inline step of the missile's seed.
        3 => {
            let Some(row) = row else { return };
            let lo = cx.world.seed(m).step();
            if ((lo & 0x7F) as i32) < row.dparam1 as i32 {
                rec.result |= result::SOFT_HIT;
            }
        }
        // iceblast: no checks.
        4 => {
            rec.freeze_len = rec.cold_len;
            rec.cold_len = 0;
        }
        // blessedhammer.
        5 => {
            if row.is_some() {
                hammer(cx, m, unit, rec);
            }
        }
        // No live row.
        6 => {
            if let Some(o) = owner {
                if !cx.world.unit_flag(o, OWNER_FLAG_0X200) && present {
                    cx.world.set_unit_flag(unit, UNIT_FLAG_0X20000, true);
                }
            }
        }
        // warcry, shockwave.
        7 => {
            let Some(row) = row else { return };
            let p1 = row.dparam1 as i32;
            rec.stun_len = if p1 > 0 {
                p1
            } else {
                skill_lin(cx, skill, level, 1, 2)
            };
            rec.hit_class = STUN_HIT_CLASS;
        }
        // erruption crack 1, 2.
        8 => {
            let Some(row) = row else { return };
            let ov = row.progoverlay as i16;
            if present && ov > 0 {
                cx.world.overlay(game, unit, i32::from(ov));
            }
        }
        // twister.
        9 => {
            let Some(row) = row else { return };
            let p1 = row.dparam1 as i32;
            rec.stun_len = if p1 > 0 {
                p1
            } else if cx.world.skill_exists(skill) {
                cx.world.skill_field(skill, SkillField::Param(2))
            } else {
                0
            };
            rec.hit_class = STUN_HIT_CLASS;
        }
        // bladesoficecubes: the cold length is kept.
        10 => {
            if row.is_some() {
                rec.freeze_len = rec.cold_len;
            }
        }
        // rabiescontagion.
        11 => match (owner, present) {
            (Some(o), true) => {
                // The poison lasts what is left of the carrier's rabies
                // state (its expiry frame is in the missile data), as base
                // ticks like every length of the record and the skill's.
                let t = game.tick_rate.base_ticks(data28.wrapping_sub(game.frame));
                if t >= 10 && t <= cx.world.skill_elem_len(game, o, skill, level) {
                    rec.poison_len = t;
                } else {
                    rec.poison = 0;
                    rec.poison_len = 0;
                }
            }
            _ => {
                rec.poison = 0;
                rec.poison_len = 0;
            }
        },
        // lightningjavelin.
        12 => {
            let Some(row) = row else { return };
            let c = cx.world.missile_calc(game, m, owner, row.dmgcalc1, -1, -1);
            if c > 0 {
                let f = converted(rec.physical, c);
                let e = elem_roll(cx, m, Some(unit), rec);
                clear_elems(rec, row.etype);
                add_elem(rec, row.etype, f.wrapping_add(e));
            }
        }
        // blessedhammerex.
        13 => {
            if ty == Some(UnitType::Monster) {
                if let Some(o) = owner {
                    let p = cx.world.stat(o, 25);
                    rec.physical = rec
                        .physical
                        .wrapping_add(rec.physical.wrapping_mul(p) / 100);
                }
                if cx.row_of(m).is_some() {
                    hammer(cx, m, unit, rec);
                }
            }
        }
        // moltenboulder.
        14 => {
            let Some(row) = row else { return };
            if !present {
                return;
            }
            let (p1, p2) = (row.dparam1 as i32, row.dparam2 as i32);
            let p = match ty {
                Some(UnitType::Player) => {
                    if p1 >= 1 {
                        p2
                    } else {
                        0
                    }
                }
                Some(UnitType::Monster) => {
                    let small = cx.world.is_small_monster(unit);
                    let large = cx.world.is_large_monster(unit);
                    match (p1.cmp(&1), small, large) {
                        (std::cmp::Ordering::Less, true, _) => p2,
                        (std::cmp::Ordering::Less, false, _) => 0,
                        (std::cmp::Ordering::Equal, true, _) => 2 * p2,
                        (std::cmp::Ordering::Equal, false, true) => 0,
                        (std::cmp::Ordering::Equal, false, false) => p2,
                        (std::cmp::Ordering::Greater, true, _) => 3 * p2,
                        (std::cmp::Ordering::Greater, false, true) => p2,
                        (std::cmp::Ordering::Greater, false, false) => 2 * p2,
                    }
                }
                _ => 0,
            };
            if p > 0 && (cx.world.seed(m).roll(100) as i32) < p {
                rec.result |= result::KNOCKBACK;
            }
        }
        _ => {}
    }
}

/// Function 5: e := `elem_roll`; `dParam1` > 0 and an
/// undead unit, then `dParam2` > 0 and a demon: add pct(e, dParam).
fn hammer<W: MissileWorld + ?Sized>(
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: UnitId,
    rec: &mut DamageRecord,
) {
    let e = elem_roll(cx, m, Some(unit), rec);
    let Some(row) = cx.row_of(m) else { return };
    let (etype, p1, p2) = (row.etype, row.dparam1 as i32, row.dparam2 as i32);
    if p1 > 0 && cx.world.is_undead(unit) {
        add_elem(rec, etype, pct(e, p1));
    }
    if p2 > 0 && cx.world.is_demon(unit) {
        add_elem(rec, etype, pct(e, p2));
    }
}

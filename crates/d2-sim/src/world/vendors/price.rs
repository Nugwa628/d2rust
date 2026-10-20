// Spec: specs/world/vendors.md §9.2–§9.4
//! The item value function (`0x0062EFB0`) and the gamble price. Every
//! product is 32-bit signed and wraps (§9.2); "x/1024" truncates toward
//! zero.

use super::{flag, tx, ty, VendorTables, AMU, CHARGE_BASE, RIN};
use crate::stats::muldiv;

/// One stat entry of an item list: (layer, value).
pub type Entry = (u16, i32);

/// A bonus-stat entry (§9.2 (B)): the stat, its layer, and the bonus
/// `0x00625560(I, stat, layer)` (Open question 1; the provider computes
/// it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Bonus {
    pub stat: u16,
    pub layer: u16,
    pub value: i32,
}

/// The price inputs of one item (§9.2 Inputs), read by the provider.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PriceItem {
    /// Combined item index.
    pub record: usize,
    pub quality: u8,
    pub flags: u32,
    pub file_index: i32,
    /// Magic affix ids (combined index + 1; 0 none).
    pub prefix: [u16; 3],
    pub suffix: [u16; 3],
    pub auto_affix: u16,
    pub ear_level: i32,
    /// Item data +0x30 (§9.4).
    pub format: u16,
    /// Stats 70, base 31, 152, 72, 73, 252, 253, 254.
    pub quantity: i32,
    pub armor_base: i32,
    pub indestructible: i32,
    pub durability: i32,
    pub max_durability: i32,
    pub replenish_durability: i32,
    pub replenish_quantity: i32,
    pub extra_stack: i32,
    /// Stat 107 entries (≤ 64 used).
    pub item_skills: Vec<Entry>,
    /// Stat 204 entries.
    pub charges: Vec<Entry>,
    /// Bonus-stat entries (≤ 511 used).
    pub bonuses: Vec<Bonus>,
    /// Combined indices of the items in the item's inventory (sockets).
    pub sockets: Vec<usize>,
}

/// The player and NPC side of a price (§9.2 inputs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PriceCtx {
    pub difficulty: u8,
    pub npc_class: u16,
    /// Player stat 87.
    pub reduced_prices: i32,
    /// Player stat 12 (§9.4).
    pub player_level: i32,
    /// The player's quest slot words for the `npc.txt` row's flags A, B, C
    /// (0 when the flag is 0).
    pub quest_slots: [u16; 3],
}

/// `npc.txt` has no row for the NPC class: the original's fatal assert
/// (edge case 2, Open question 5); also the missing `books` row of §9.2
/// rule 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PriceFatal {
    #[error("no npc.txt row for class {0}")]
    NoNpcRow(u16),
    #[error("no books row {0}")]
    NoBook(u16),
    /// §3.1 rule 2: the store item creation returned none; the code read
    /// `0x00628590(none)` asserts (line 0x61A).
    #[error("store item creation returned none (line 0x61A)")]
    NullStoreItem,
    /// §9.4: the item's normal code has no record (V6: the format-0 read
    /// faults, `0x00629370` asserts 0xB1A).
    #[error("no record for the normal code of item record {0}")]
    NoNormalRecord(usize),
}

/// guard(x, m) (§9.2).
fn guard(x: i32, m: i32) -> i32 {
    if x >= 0x1_0000 && m != 0 {
        (x / 1024).wrapping_mul(m)
    } else {
        x.wrapping_mul(m) / 1024
    }
}

/// Max stack `0x006295B0`: `maxstack` + stat 254, at most 511.
pub fn max_stack(t: &VendorTables, it: &PriceItem) -> i32 {
    let m = t.item(it.record).map_or(0, |r| r.maxstack as i32);
    m.wrapping_add(it.extra_stack).min(511)
}

/// Durability-applicable `0x00629930`.
pub fn durability_applicable(t: &VendorTables, it: &PriceItem) -> bool {
    t.item(it.record)
        .is_some_and(|r| r.nodurability == 0 && r.durability != 0)
        && it.max_durability != 0
        && it.indestructible < 1
}

/// Charges not all full: a stat 204 entry with current < max.
pub fn charges_not_full(it: &PriceItem) -> bool {
    it.charges.iter().any(|&(_, v)| (v & 0xFF) < (v >> 8))
}

fn ethereal(it: &PriceItem) -> bool {
    it.flags & flag::ETHEREAL != 0
}

/// Replenishable stack: `repair`, `throwable`, `stackable`, not ethereal.
pub fn replenishable_stack(t: &VendorTables, it: &PriceItem) -> bool {
    let ty = t.type_of(it.record);
    ty.is_some_and(|y| y.repair != 0 && y.throwable != 0)
        && t.item(it.record).is_some_and(|r| r.stackable != 0)
        && !ethereal(it)
}

/// Repairable `0x0062E660` (§9.2 rule 0).
pub fn repairable(t: &VendorTables, it: &PriceItem) -> bool {
    if it.flags & flag::IDENTIFIED == 0 || ethereal(it) {
        return false;
    }
    charges_not_full(it)
        || (t.type_of(it.record).is_some_and(|y| y.repair != 0)
            && (replenishable_stack(t, it) || durability_applicable(t, it)))
}

/// Ammunition: the primary type's `quiver` ≠ 0.
pub fn ammunition(t: &VendorTables, it: &PriceItem) -> bool {
    t.type_of(it.record).is_some_and(|y| y.quiver != 0)
}

/// Throwable: the primary type's `throwable`.
pub fn throwable(t: &VendorTables, it: &PriceItem) -> bool {
    t.type_of(it.record).is_some_and(|y| y.throwable != 0)
}

/// S, B, R and their deltas.
#[derive(Clone, Copy, Debug, Default)]
struct Sbr {
    s: i32,
    b: i32,
    r: i32,
}

/// dX += add + guard'(X, mult) for an affix-like modifier (guard' tests S
/// for all three).
fn add_mod(d: &mut Sbr, v: &Sbr, mult: i32, add: i32) {
    let g = |x: i32| {
        if v.s >= 0x1_0000 && mult != 0 {
            (x / 1024).wrapping_mul(mult)
        } else {
            x.wrapping_mul(mult) / 1024
        }
    };
    d.s = d.s.wrapping_add(add.wrapping_add(g(v.s)));
    d.b = d.b.wrapping_add(add.wrapping_add(g(v.b)));
    d.r = d.r.wrapping_add(add.wrapping_add(g(v.r)));
}

/// X += dX / div, unsigned (§9.2 (A), (B)).
fn add_unsigned(v: &mut Sbr, d: &Sbr, div: i32) {
    let q = |x: i32| (x as u32 / div as u32) as i32;
    v.s = v.s.wrapping_add(q(d.s));
    v.b = v.b.wrapping_add(q(d.b));
    v.r = v.r.wrapping_add(q(d.r));
}

/// Magic affix id (combined index + 1) → its modifier; 0 → none.
fn affix(t: &VendorTables, id: u16, d: &mut Sbr, v: &Sbr) {
    // §9.2 rule 4: an empty slot (id 0) adds no delta.
    if let Some(m) = usize::from(id).checked_sub(1).and_then(|i| t.magic.get(i)) {
        add_mod(d, v, m.mult, m.add);
    }
}

/// (A) item-skill costs `0x0062EDD0`.
fn item_skills(t: &VendorTables, it: &PriceItem, v: &mut Sbr, div: i32) {
    if t.type_of(it.record).is_some_and(|y| y.staffmods == 7) {
        return;
    }
    let mut d = Sbr::default();
    for &(skill, value) in it.item_skills.iter().take(64) {
        // §9.2 (A): a layer without a skills row is skipped.
        let Some(sk) = t.skills.get(usize::from(skill)) else {
            continue;
        };
        let (m, a) = (sk.cost.mult, sk.cost.add);
        let k = value.wrapping_mul(2).wrapping_sub(1);
        let (ds, db, dr) = if v.s < 0x1_0000 || m == 0 {
            (
                m.wrapping_mul(v.s) / 1024,
                v.b.wrapping_mul(m) / 4096,
                v.r.wrapping_mul(m) / 1024,
            )
        } else {
            (
                (m / 1024).wrapping_mul(v.s),
                (m / 4096).wrapping_mul(v.b),
                v.r.wrapping_mul(m / 1024),
            )
        };
        d.s = d.s.wrapping_add(ds.wrapping_add(a).wrapping_mul(k));
        d.b = d.b.wrapping_add(db.wrapping_add(a).wrapping_mul(k));
        d.r = d.r.wrapping_add(dr.wrapping_add(a).wrapping_mul(k));
    }
    add_unsigned(v, &d, div);
}

/// (min, max) of a packed by-time value (`0x0065CA30`, `sim/stats.md`
/// §8): its low and high fields.
// §9.2 (B) encode 4: "(min, max) of the by-time value" are the two packed
// fields `stats::by_time` decodes.
fn by_time_range(v: i32) -> (i32, i32) {
    (((v >> 2) & 0x3FF) - 256, ((v >> 12) & 0x3FF) - 256)
}

/// One (B) term: small or large form of the guard on S·u, with B divided
/// by `bdiv`.
fn bonus_term(v: &Sbr, u: i32, m: i32, a: i32, bdiv: i32) -> Sbr {
    let su = v.s.wrapping_mul(u);
    if su >= 0x1_0000 && m != 0 {
        Sbr {
            s: (su / 1024).wrapping_mul(m).wrapping_add(a),
            b: (v.b.wrapping_mul(u) / bdiv).wrapping_mul(m).wrapping_add(a),
            r: (v.r.wrapping_mul(u) / 1024).wrapping_mul(m).wrapping_add(a),
        }
    } else {
        Sbr {
            s: m.wrapping_mul(su) / 1024 + a,
            b: v.b.wrapping_mul(m).wrapping_mul(u) / bdiv + a,
            r: v.r.wrapping_mul(m).wrapping_mul(u) / 1024 + a,
        }
    }
}

/// (B) bonus-stat costs `0x00628E70`.
fn bonus_stats(t: &VendorTables, it: &PriceItem, v: &mut Sbr, div: i32) {
    let mut d = Sbr::default();
    for b in it.bonuses.iter().take(511) {
        let Some(sc) = t.stats.get(usize::from(b.stat)) else {
            continue;
        };
        if b.value == 0 {
            continue;
        }
        let bv = b.value >> sc.valshift;
        let skill_mod = |skill: u32| {
            t.skills
                .get(skill as usize)
                .map_or((0, 0), |s| (s.cost.mult, s.cost.add))
        };
        let term = match sc.encode {
            1 => {
                let (m, a) = skill_mod(u32::from(b.layer));
                bonus_term(v, bv, m, a, 4096)
            }
            2 | 3 => {
                let layer = u32::from(b.layer);
                let (m, a) = skill_mod(layer >> t.stat_shift);
                let u = (layer & t.stat_mask) as i32;
                bonus_term(v, u, m, a, 4096)
            }
            4 => {
                let (lo, hi) = by_time_range(bv);
                let u = (lo + hi) / 2;
                bonus_term(v, u, sc.cost.mult, sc.cost.add, 1024)
            }
            _ => bonus_term(v, bv, sc.cost.mult, sc.cost.add, 1024),
        };
        d.s = d.s.wrapping_add(term.s);
        d.b = d.b.wrapping_add(term.b);
        d.r = d.r.wrapping_add(term.r);
    }
    add_unsigned(v, &d, div);
}

/// (C) charged skills `0x00628D30(I, base)`.
pub fn charged_skills(t: &VendorTables, it: &PriceItem, base: i32) -> i32 {
    let mut total = 0i32;
    for &(layer, value) in &it.charges {
        let current = value & 0xFF;
        let max = value >> 8;
        if current >= max {
            continue;
        }
        let layer = u32::from(layer);
        let skill = (layer >> t.stat_shift) as usize;
        let lvl = (layer & t.stat_mask) as i32;
        let Some(sk) = t.skills.get(skill) else {
            continue;
        };
        let (m, a) = (sk.cost.mult, sk.cost.add);
        let tt = lvl + 2 + (sk.reqlevel / 6) * 2;
        let x = tt.wrapping_mul(base);
        let c = if x < 0x1_0000 || m == 0 {
            m.wrapping_mul(tt).wrapping_mul(base) / 1024
        } else {
            (x / 1024).wrapping_mul(m)
        };
        total = total.wrapping_add((max - current).wrapping_mul(c.wrapping_add(a)) / max);
    }
    total
}

/// cost(t) (§9.2): the price of `it` for transaction `t`. `None` item →
/// 0x7FFFFFFF.
pub fn cost(
    t: &VendorTables,
    ctx: &PriceCtx,
    it: Option<&PriceItem>,
    txn: u32,
) -> Result<i32, PriceFatal> {
    // Rule 0.
    let Some(it) = it else {
        return Ok(0x7FFF_FFFF);
    };
    if txn == tx::REPAIR && !repairable(t, it) {
        return Ok(0);
    }
    if it.flags & flag::STARTITEM != 0 {
        return Ok(1);
    }
    // Rule 1.
    let qty = it.quantity.max(1);
    let rp = ctx.reduced_prices.min(99);
    if txn == tx::GAMBLE {
        return gamble_price(t, it, ctx.player_level, rp);
    }
    let rec = t.item(it.record);
    let item_cost = rec.map_or(0, |r| r.cost as i32);
    let is_book = t.is_type(it.record, ty::BOOK);
    let is_ammo = ammunition(t, it);
    let mstack = max_stack(t, it);
    // Rule 2.
    let mut div = 1;
    let mut v = if it.flags & flag::EAR != 0 {
        let x = (it.ear_level & 0xFF).wrapping_mul(item_cost);
        Sbr { s: x, b: x, r: 0 }
    } else if t.is_type(it.record, ty::BODY) {
        let lvl = usize::try_from(it.file_index)
            .ok()
            .and_then(|i| t.monster_levels.get(i))
            .map_or(0, |l| i32::from(l[usize::from(ctx.difficulty.min(2))]));
        let x = item_cost.wrapping_add(8 * lvl);
        Sbr { s: x, b: x, r: 0 }
    } else if is_book {
        let per = t
            .books
            .get(usize::from(it.suffix[0]))
            .ok_or(PriceFatal::NoBook(it.suffix[0]))?;
        let x = item_cost.wrapping_add(qty.wrapping_mul(*per));
        Sbr { s: x, b: x, r: 0 }
    } else if is_ammo {
        let x = item_cost.wrapping_mul(qty) / 1024;
        Sbr {
            s: x,
            b: x,
            r: mstack.wrapping_mul(item_cost) / 1024,
        }
    } else {
        if rec.is_some_and(|r| r.stackable != 0) && mstack >= 2 {
            div = mstack;
        }
        Sbr {
            s: item_cost,
            b: item_cost,
            r: item_cost,
        }
    };
    if t.is_type(it.record, ty::ARMO) {
        if let Some(r) = rec {
            let (min_ac, max_ac) = (r.minac as i32, r.maxac as i32);
            if max_ac.wrapping_sub(min_ac) != -1 && max_ac != 0 {
                let x = item_cost.wrapping_mul(it.armor_base) / max_ac;
                v = Sbr { s: x, b: x, r: x };
            }
        }
    }
    // Rule 3.
    let mt = (4..=9).contains(&it.quality);
    if !mt {
        item_skills(t, it, &mut v, div);
    }
    // Rule 4.
    if it.flags & flag::IDENTIFIED != 0 {
        let mut d = Sbr::default();
        affix(t, it.auto_affix, &mut d, &v);
        let magic = |d: &mut Sbr, v: &mut Sbr| {
            affix(t, it.prefix[0], d, v);
            affix(t, it.suffix[0], d, v);
            bonus_stats(t, it, v, div);
        };
        match it.quality {
            1 => {
                d = Sbr {
                    s: -(v.s / 2),
                    b: -(v.b / 2),
                    r: -(v.r / 2),
                };
            }
            3 | 9 => bonus_stats(t, it, &mut v, div),
            4 => magic(&mut d, &mut v),
            5 => {
                if let Some(m) = usize::try_from(it.file_index)
                    .ok()
                    .and_then(|i| t.setitems.get(i))
                {
                    add_mod(&mut d, &v, m.mult, m.add);
                }
            }
            6 | 8 => {
                for k in 0..3 {
                    affix(t, it.prefix[k], &mut d, &v);
                    affix(t, it.suffix[k], &mut d, &v);
                }
                bonus_stats(t, it, &mut v, div);
            }
            7 => match usize::try_from(it.file_index)
                .ok()
                .and_then(|i| t.uniques.get(i))
            {
                Some(u) => add_mod(&mut d, &v, u.cost.mult, u.cost.add),
                None => magic(&mut d, &mut v),
            },
            _ => {}
        }
        // "apply" (signed).
        v.s = v.s.wrapping_add(d.s / div);
        v.b = v.b.wrapping_add(d.b / div);
        v.r = v.r.wrapping_add(d.r / div);
        if mt {
            item_skills(t, it, &mut v, div);
        }
    }
    // Rule 6.
    for &s in &it.sockets {
        // §9.2 rule 6: "cost/2" is the socketed item's record `cost` / 2.
        let c = t.item(s).map_or(0, |r| r.cost as i32) / 2;
        v.s = v.s.wrapping_add(c);
        v.b = v.b.wrapping_add(c);
        v.r = v.r.wrapping_add(c);
    }
    // Rule 7.
    if ethereal(it) {
        v.b /= 4;
    }
    if t.type_of(it.record).is_some_and(|y| y.class < 7) {
        v.b /= 4;
    }
    // Rule 8.
    let dur_ok = durability_applicable(t, it);
    if txn == tx::SELL && ethereal(it) && dur_ok && it.durability < 1 {
        v.b = 0;
    }
    if txn == tx::REPAIR && !is_ammo && !throwable(t, it) && dur_ok {
        let (max, dur) = (it.max_durability, it.durability);
        v.r = if max == 0 || max <= dur {
            0
        } else if it.replenish_durability == 0 {
            (max - dur).wrapping_mul(v.r) / max
        } else if dur < max - 1 {
            (max - 1).wrapping_mul(v.r) / max
        } else {
            0
        };
    }
    // Rule 9.
    let row = t
        .npc_row(ctx.npc_class)
        .ok_or(PriceFatal::NoNpcRow(ctx.npc_class))?;
    v.s = guard(v.s, row.sell);
    v.r = guard(v.r, row.rep);
    v.b = if v.b >= 0x1_0000 && row.sell != 0 {
        (v.b / 1024).wrapping_mul(row.buy)
    } else {
        v.b.wrapping_mul(row.buy) / 1024
    };
    for (k, &(f, sm, bm, rm)) in row.quests.iter().enumerate() {
        if f != 0 && ctx.quest_slots[k] & 3 != 0 {
            v.s = guard(v.s, sm);
            v.b = guard(v.b, bm);
            v.r = guard(v.r, rm);
        }
    }
    // Rule 10.
    if !is_book && !is_ammo {
        v.s = v.s.wrapping_mul(qty);
    }
    if rec.is_none_or(|r| r.stackable == 0) || !repairable(t, it) {
        v.b = v.b.wrapping_mul(qty);
    } else if qty < mstack && it.replenish_quantity == 0 {
        v.r = v.r.wrapping_mul(mstack - qty);
        v.b = mstack.wrapping_mul(v.b).wrapping_sub(v.r);
    } else {
        v.b = mstack.wrapping_mul(v.b);
        v.r = 0;
    }
    // Rule 11.
    if txn == tx::REPAIR && !ethereal(it) {
        v.r = v.r.wrapping_add(charged_skills(t, it, CHARGE_BASE));
    }
    // Rule 12.
    v.b = v.b.min(row.max_buy[usize::from(ctx.difficulty.min(2))]);
    // Rule 13.
    let less = |x: i32| {
        if rp != 0 {
            x.wrapping_sub(muldiv(x, rp, 100))
        } else {
            x
        }
    };
    Ok(match txn {
        tx::SELL => {
            if v.b <= 0 {
                1
            } else {
                v.b
            }
        }
        tx::REPAIR => {
            let r = less(v.r);
            if r <= 0 {
                1
            } else {
                r
            }
        }
        _ => less(v.s).max(1),
    })
}

/// Gamble price (§9.4). The normal code (`0x006287D0`) is items
/// `normcode` when ≠ 0, else `code`; one missing from the code map is
/// fatal (V6).
pub fn gamble_price(
    t: &VendorTables,
    it: &PriceItem,
    player_level: i32,
    rp: i32,
) -> Result<i32, PriceFatal> {
    let normal = t
        .item(it.record)
        .and_then(|r| {
            t.find_code(if r.normcode != [0; 4] {
                r.normcode
            } else {
                r.code
            })
        })
        .and_then(|i| t.item(i));
    let Some(n) = normal else {
        return Err(PriceFatal::NoNormalRecord(it.record));
    };
    let price = if it.format == 0 || n.code == RIN || n.code == AMU {
        n.gamble_cost as i32
    } else {
        let l = player_level;
        let level = i32::from(n.level);
        let st = ((n.minstack as i32).wrapping_add(n.maxstack as i32) / 2).max(1);
        let tier = |code: [u8; 4], per: i32| {
            if code == [0; 4] || code == *b"0   " {
                return (0, 0);
            }
            match t.find_code(code).and_then(|i| t.item(i)) {
                Some(r) => (
                    ((l - i32::from(r.level)).wrapping_mul(100) / per + 1).max(0),
                    r.cost as i32,
                ),
                None => (0, 0),
            }
        };
        let (w_u, c_u) = tier(n.ubercode, 2);
        let (w_x, c_x) = tier(n.ultracode, 4);
        let lp = if l < 6 { 5 } else { l };
        let base = ((level - 45).max(0) - level / 2 + lp).wrapping_mul(250) / 3;
        let mix = (10000 - w_x - w_u)
            .wrapping_mul(n.cost as i32)
            .wrapping_mul(st)
            .wrapping_add(c_x.wrapping_mul(w_x))
            .wrapping_add(c_u.wrapping_mul(w_u))
            / 10000;
        base.wrapping_add(mix).wrapping_mul((2 * lp + 1) / 3 + 20) / 15
    };
    Ok(if rp != 0 {
        price.wrapping_sub(muldiv(price, rp, 100))
    } else {
        price
    })
}

#[cfg(test)]
mod mutant_tests;

// Spec: specs/world/vendors.md §9.2 (A), (B); specs/sim/stats.md §8
//! Mutation-testing kills (METHODS M08): each test pins an outcome the
//! spec decides that no earlier test checked. A child of `price` to check
//! S, B and R of the private cost terms, not only the one a transaction
//! returns.

use super::super::tests::{index, item, tables, T_THROW, T_WEAP};
use super::super::{CostMod, StatCost, CHARGE_BASE};
use super::*;

fn sbr(s: i32, b: i32, r: i32) -> Sbr {
    Sbr { s, b, r }
}

fn parts(v: &Sbr) -> (i32, i32, i32) {
    (v.s, v.b, v.r)
}

fn with_skill(m: i32, a: i32, value: i32) -> (VendorTables, PriceItem) {
    let mut t = tables();
    t.skills[0].cost = CostMod { mult: m, add: a };
    let it = PriceItem {
        record: index(&t, "lax"),
        item_skills: vec![(0, value)],
        ..PriceItem::default()
    };
    (t, it)
}

// From specs/world/vendors.md §9.2 (A): k = 2v − 1; S < 0x10000 or m = 0
// → (m·S/1024 + a)·k, (B·m/4096 + a)·k, (R·m/1024 + a)·k; else
// ((m/1024)·S + a)·k, ((m/4096)·B + a)·k, (R·(m/1024) + a)·k.
#[test]
fn item_skill_costs_both_forms() {
    // Small S: m 1536, a 10, k 3.
    let (t, it) = with_skill(1536, 10, 2);
    let mut v = sbr(100, 5000, 3000);
    item_skills(&t, &it, &mut v, 1);
    assert_eq!(parts(&v), (100 + 160 * 3, 5000 + 1885 * 3, 3000 + 4510 * 3));
    // Large S: m 5000 (m/1024 = 4, m/4096 = 1).
    let (t, it) = with_skill(5000, 10, 2);
    let mut v = sbr(0x1_0005, 50_000, 70_000);
    item_skills(&t, &it, &mut v, 1);
    assert_eq!(
        parts(&v),
        (
            0x1_0005 + (4 * 0x1_0005 + 10) * 3,
            50_000 + (50_000 + 10) * 3,
            70_000 + (70_000 * 4 + 10) * 3
        )
    );
    // S = 0x10000 exactly is the large form: (1536/1024)·S = S.
    let (t, it) = with_skill(1536, 0, 1);
    let mut v = sbr(0x1_0000, 100, 100);
    item_skills(&t, &it, &mut v, 1);
    assert_eq!(parts(&v), (0x2_0000, 100, 200));
}

// From specs/sim/stats.md §8 r1: lo = ((v >> 2) & 0x3FF) − 256, hi =
// ((v >> 12) & 0x3FF) − 256 (the by-time (min, max) of §9.2 (B) encode 4).
#[test]
fn by_time_fields() {
    let v = 1 | ((10 + 256) << 2) | ((50 + 256) << 12);
    assert_eq!(by_time_range(v), (10, 50));
    let v = 3 | ((-20 + 256) << 2) | ((700 + 256) << 12);
    assert_eq!(by_time_range(v), (-20, 700));
}

// From specs/world/vendors.md §9.2 (B): guard on S·u; small → m·S·u/1024
// + a, B·m·u/bdiv + a, R·m·u/1024 + a; large → (S·u/1024)·m + a,
// (B·u/bdiv)·m + a, (R·u/1024)·m + a.
#[test]
fn bonus_term_both_forms() {
    let t = bonus_term(&sbr(100, 200, 300), 3, 2048, 5, 4096);
    assert_eq!(parts(&t), (605, 305, 1805));
    let v = sbr(0x8000, 0x9000, 0xA000);
    let t = bonus_term(&v, 3, 2048, 5, 4096);
    assert_eq!(parts(&t), (96 * 2048 + 5, 27 * 2048 + 5, 120 * 2048 + 5));
    let t = bonus_term(&v, 3, 2048, 5, 1024);
    assert_eq!(t.b, 108 * 2048 + 5);
    // S·u = 98307 (not a multiple of 1024): the large form truncates first.
    let t = bonus_term(&sbr(0x8001, 0, 0), 3, 2048, 5, 4096);
    assert_eq!(t.s, (98_307 / 1024) * 2048 + 5);
}

// From specs/world/vendors.md §9.2 (B): b >>= valshift; encode 1 → skill
// = layer, skills cost; 2, 3 → skill = layer >> shift, u = layer & mask;
// 4 → u = (min + max)/2 of the by-time value, itemstatcost cost; other →
// itemstatcost cost.
#[test]
fn bonus_stats_by_encode() {
    let mut t = tables();
    t.skills[0].cost = CostMod { mult: 1536, add: 7 };
    t.skills[1].cost = CostMod { mult: 3072, add: 9 };
    let stat = |encode: u8, valshift: u8| StatCost {
        cost: CostMod { mult: 2048, add: 3 },
        valshift,
        encode,
    };
    t.stats[10] = stat(0, 2);
    t.stats[11] = stat(1, 1);
    t.stats[12] = stat(2, 0);
    t.stats[13] = stat(4, 0);
    let v0 = sbr(100, 200, 300);
    let run = |b: Bonus| {
        let it = PriceItem {
            bonuses: vec![b],
            ..PriceItem::default()
        };
        let mut v = v0;
        bonus_stats(&t, &it, &mut v, 1);
        parts(&v)
    };
    let plus = |d: Sbr| (v0.s + d.s, v0.b + d.b, v0.r + d.r);
    let bonus = |stat, layer, value| Bonus { stat, layer, value };
    // other: 12 >> 2 = 3, itemstatcost (2048, 3), B / 1024.
    assert_eq!(
        run(bonus(10, 0, 12)),
        plus(bonus_term(&v0, 3, 2048, 3, 1024))
    );
    // encode 1: 6 >> 1 = 3, skill 0 (layer 0), B / 4096.
    assert_eq!(
        run(bonus(11, 0, 6)),
        plus(bonus_term(&v0, 3, 1536, 7, 4096))
    );
    // encode 2: layer (1 << 6) | 5 → skill 1, u 5.
    assert_eq!(
        run(bonus(12, 69, 1)),
        plus(bonus_term(&v0, 5, 3072, 9, 4096))
    );
    // encode 4: by-time (10, 50) → u 30.
    let bt = 1 | ((10 + 256) << 2) | ((50 + 256) << 12);
    assert_eq!(
        run(bonus(13, 0, bt)),
        plus(bonus_term(&v0, 30, 2048, 3, 1024))
    );
}

// From specs/world/vendors.md §9.2 (C): t = lvl + 2 + (reqlevel/6)·2; x =
// t·base; c = x < 0x10000 or m = 0 ? m·t·base/1024 : (x/1024)·m; total +=
// (max − current)·(c + a)/max.
#[test]
fn charged_skill_terms() {
    let mut t = tables();
    t.skills[0].cost = CostMod {
        mult: 1536,
        add: 10,
    };
    t.skills[0].reqlevel = 18;
    // Layer 1: skill 0, level 1 → t = 1 + 2 + 6 = 9; current 2 of 5.
    let it = PriceItem {
        charges: vec![(1, (5 << 8) | 2)],
        ..PriceItem::default()
    };
    // x = 900: c = 1536·9·100/1024 = 1350.
    assert_eq!(charged_skills(&t, &it, 100), 3 * 1360 / 5);
    // x = 90000: c = (90000/1024)·1536 = 87·1536.
    assert_eq!(charged_skills(&t, &it, 10_000), 3 * (87 * 1536 + 10) / 5);
}

fn ctx() -> PriceCtx {
    // npc 600: sell 1024, buy 512, rep 1024.
    PriceCtx {
        difficulty: 0,
        npc_class: 600,
        reduced_prices: 0,
        player_level: 1,
        quest_slots: [0; 3],
    }
}

fn ident(t: &VendorTables, c: &str) -> PriceItem {
    PriceItem {
        record: index(t, c),
        quality: 2,
        flags: flag::IDENTIFIED,
        file_index: -1,
        format: 101,
        durability: 20,
        max_durability: 20,
        ..PriceItem::default()
    }
}

fn price(t: &VendorTables, it: &PriceItem, txn: u32) -> i32 {
    cost(t, &ctx(), Some(it), txn).unwrap()
}

// From specs/world/vendors.md §9.2 rule 4: inferior replaces the deltas
// with −(S/2), −(B/2), −(R/2).
#[test]
fn inferior_halves_s_b_r() {
    let t = tables();
    let mut it = ident(&t, "lax"); // cost 100
    it.quality = 1;
    it.durability = 5;
    assert_eq!(price(&t, &it, tx::BUY), 50);
    // B = 50, ·512/1024.
    assert_eq!(price(&t, &it, tx::SELL), 25);
    // R = 50, durability 5 of 20: 15·50/20.
    assert_eq!(price(&t, &it, tx::REPAIR), 15 * 50 / 20);
}

// From specs/world/vendors.md §9.2 rule 2: div := max stack only for a
// stackable item.
#[test]
fn div_needs_stackable() {
    let mut t = tables();
    let lax = index(&t, "lax");
    t.items[lax].maxstack = 5;
    let mut it = ident(&t, "lax");
    it.quality = 4;
    it.prefix[0] = 1; // mult 2048, add 100: 100 + 100 + 200
    assert_eq!(price(&t, &it, tx::BUY), 400);
}

/// A stackable throwing item `tax` (cost 100, max stack 50), magic with
/// prefix 1: div 50, every delta 300 → S = B = R = 106.
fn tax() -> (VendorTables, PriceItem) {
    let mut t = tables();
    let mut r = item("tax", 100, T_THROW);
    r.stackable = 1;
    r.maxstack = 50;
    t.items.push(r);
    let mut it = ident(&t, "tax");
    it.quality = 4;
    it.prefix[0] = 1;
    it.quantity = 1;
    (t, it)
}

// From specs/world/vendors.md §9.2 rules 4, 8, 10: deltas / div; no
// durability step for a throwable; R·(M − qty), or B := M·B with
// replenished quantity.
#[test]
fn stackable_apply_and_stack_values() {
    let (t, mut it) = tax();
    assert_eq!(price(&t, &it, tx::BUY), 106);
    // Durability 10 of 20 is ignored for a throwable: R = 106·(50 − 1).
    it.durability = 10;
    assert_eq!(price(&t, &it, tx::REPAIR), 106 * 49);
    // B = 106·512/1024 = 53; replenished quantity: B := 50·53.
    it.replenish_quantity = 1;
    assert_eq!(price(&t, &it, tx::SELL), 50 * 53);
}

// From specs/world/vendors.md §9.2 rule 8: only a repair scales R by
// durability (a sale of a stackable reads the unscaled R in rule 10).
#[test]
fn sale_keeps_r_unscaled() {
    let mut t = tables();
    let lax = index(&t, "lax");
    t.items[lax].stackable = 1;
    t.items[lax].maxstack = 10;
    let mut it = ident(&t, "lax");
    it.durability = 5;
    it.quantity = 9;
    // div 10, no deltas: B = 50, R = 100; R·(10 − 9) = 100; B = 10·50 − 100.
    assert_eq!(price(&t, &it, tx::SELL), 400);
}

// From specs/world/vendors.md §9.2 rule 8: an ethereal sale zeroes B only
// at durability < 1.
#[test]
fn ethereal_sale_at_durability_1() {
    let t = tables();
    let mut it = ident(&t, "lax");
    it.flags |= flag::ETHEREAL;
    it.durability = 1;
    // B = 100/4 = 25, ·512/1024.
    assert_eq!(price(&t, &it, tx::SELL), 12);
}

// From specs/world/vendors.md §9.2 rules 8, 11: max ≤ durability → R := 0,
// then the charged skills are added.
#[test]
fn repair_over_max_durability_is_charges_only() {
    let t = tables();
    let mut it = ident(&t, "lax");
    it.durability = 25;
    it.charges = vec![(0, (5 << 8) | 3)];
    let c = charged_skills(&t, &it, CHARGE_BASE);
    assert!(c > 0);
    assert_eq!(price(&t, &it, tx::REPAIR), c);
}

// From specs/world/vendors.md §9.2 rules 2, 10, 11: ammunition R = max
// stack·cost/1024, then R·(M − qty) for a repairable stack.
#[test]
fn ammunition_repair_value() {
    let t = tables();
    let mut it = ident(&t, "aqv"); // cost 256, max stack 500
    it.max_durability = 0;
    it.quantity = 100;
    it.charges = vec![(0, (5 << 8) | 3)];
    let c = charged_skills(&t, &it, CHARGE_BASE);
    assert_eq!(price(&t, &it, tx::REPAIR), 500 * 256 / 1024 * 400 + c);
}

/// §9.4 price from its parts: (level, cost, st) of the normal record,
/// L, and (level, cost) of the uber and ultra records when present.
fn gp(n: (i32, i32, i32), l: i32, uber: Option<(i32, i32)>, ultra: Option<(i32, i32)>) -> i32 {
    let (level, cost, st) = n;
    let (w_u, c_u) = uber.map_or((0, 0), |(lv, c)| (((l - lv) * 100 / 2 + 1).max(0), c));
    let (w_x, c_x) = ultra.map_or((0, 0), |(lv, c)| (((l - lv) * 100 / 4 + 1).max(0), c));
    let lp = if l < 6 { 5 } else { l };
    ((((level - 45).max(0) - level / 2 + lp) * 250 / 3)
        + ((10000 - w_x - w_u) * cost * st + c_x * w_x + c_u * w_u) / 10000)
        * ((2 * lp + 1) / 3 + 20)
        / 15
}

// From specs/world/vendors.md §9.4.
#[test]
fn gamble_price_parts() {
    let mut t = tables();
    let hax = ident(&t, "hax");
    let parts = (Some((31, 810)), Some((54, 14033)));
    // The helper reproduces the spec vectors (hax: level 3, cost 170).
    assert_eq!(gp((3, 170, 1), 10, parts.0, parts.1), 1656);
    assert_eq!(gp((3, 170, 1), 99, parts.0, parts.1), 57986);
    // L = 6: L' = 6.
    assert_eq!(
        gamble_price(&t, &hax, 6, 0).unwrap(),
        gp((3, 170, 1), 6, parts.0, parts.1)
    );
    // Level 50 and stacks 10..30 (st 20), no upgrade codes.
    let mut r = item("hlv", 1000, T_WEAP);
    r.level = 50;
    r.minstack = 10;
    r.maxstack = 30;
    t.items.push(r);
    let h = ident(&t, "hlv");
    assert_eq!(
        gamble_price(&t, &h, 60, 0).unwrap(),
        gp((50, 1000, 20), 60, None, None)
    );
    // ubercode `0   ` is no upgrade even when an item has that code.
    t.items.push(item("0", 5000, T_WEAP));
    let i = index(&t, "hlv");
    t.items[i].ubercode = *b"0   ";
    assert_eq!(
        gamble_price(&t, &h, 60, 0).unwrap(),
        gp((50, 1000, 20), 60, None, None)
    );
}

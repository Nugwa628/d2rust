//! Gap tests: one test per stats or stat-list rule that no
//! other test covered, each on the smallest synthetic case that tells the
//! rule from its plausible misreadings (M14). Tables are the
//! synthetic itemstatcost of [`super::tests`], with extra op rows where a
//! rule needs them.

use std::sync::Arc;

use d2_data::bin::BinTable;
use d2_data::tables::{Itemstatcost, Record};

use super::lists::{flag, owner, RemoveCallback, StatListError};
use super::tests::{data, item_list, itemstatcost_with, player, set_u16, Log, ITEM, N, P};
use super::*;
use crate::units::{UnitId, UnitType};

// ---- synthetic data ------------------------------------------------------------

fn rec(records: &mut [u8], s: usize) -> &mut [u8] {
    let size = Itemstatcost::SIZE;
    &mut records[s * size..(s + 1) * size]
}

fn set_op(records: &mut [u8], s: usize, op: u8, param: u8, base: u16, stats: &[u16]) {
    let r = rec(records, s);
    r[0x54] = op;
    r[0x55] = param;
    set_u16(r, 0x56, base);
    for (j, &t) in stats.iter().enumerate() {
        set_u16(r, 0x58 + 2 * j, t);
    }
}

/// [`data`] with another itemstatcost table.
fn data_from(t: &BinTable) -> Arc<StatData> {
    let mut d = (*data()).clone();
    d.stats = StatTable::from_fixed(t).expect("itemstatcost");
    Arc::new(d)
}

/// The synthetic table plus one op row per op the base-game rows of
/// [`super::tests::itemstatcost`] do not show, and a few flag rows.
fn custom() -> Arc<StatData> {
    data_from(&itemstatcost_with(|r| {
        set_op(r, 301, 1, 0, NO_STAT, &[41]);
        set_op(r, 303, 3, 3, 12, &[42]);
        set_op(r, 305, 5, 3, 12, &[43]);
        set_op(r, 306, 6, 0, NO_STAT, &[44]);
        set_op(r, 307, 7, 0, NO_STAT, &[44]);
        set_op(r, 310, 10, 0, NO_STAT, &[45]);
        set_op(r, 312, 12, 0, NO_STAT, &[45]);
        set_op(r, 320, 2, 0, 0, &[46]);
        set_op(r, 321, 4, 0, 0, &[47]);
        // A53 without op stats (A51 = A52 = 0): the add-full path.
        set_op(r, 330, 4, 0, 12, &[]);
        set_op(r, 331, 4, 0, 12, &[48]);
        rec(r, 45)[0x50] = 1; // keepzero
        set_u16(rec(r, 340), 0x48, 5); // itemevent1
        for s in [340, 341] {
            rec(r, s)[5] |= 0x08; // fCallback
        }
    }))
}

fn lists_with(d: Arc<StatData>) -> (StatLists, Log, ListId) {
    let mut log = Log::default();
    let mut lists = StatLists::new(d);
    let p = player(&mut lists, &mut log);
    (lists, log, p)
}

/// An extended list of another unit.
fn ext(
    lists: &mut StatLists,
    log: &mut Log,
    unit: UnitId,
    ty: UnitType,
    stats: &[(u16, i32)],
) -> ListId {
    let l = lists.alloc_extended(log, unit, ty, unit.0, 0, 0, None);
    for &(s, v) in stats {
        lists.set(log, l, s, v, 0, None);
    }
    l
}

fn plain(lists: &mut StatLists, log: &mut Log, stats: &[(u16, i32)]) -> ListId {
    let l = lists.alloc(0, 0, owner::PLAYER, 1);
    for &(s, v) in stats {
        lists.set(log, l, s, v, 0, None);
    }
    l
}

const I2: UnitId = UnitId(3);
const I3: UnitId = UnitId(4);

/// Packed by-time value: period 0, lo, hi.
const fn by_time_value(lo: i32, hi: i32) -> i32 {
    ((hi + 256) << 12) | ((lo + 256) << 2)
}

// ---- ------------------------------------------------------------------

#[test]
fn layers_are_independent_keys() {
    let (mut lists, mut log, p) = lists_with(data());
    for (layer, v) in [(0, 5), (1, 7), (0xFFFF, 9)] {
        lists.set(&mut log, p, 20, v, layer, None);
    }
    assert_eq!(
        [0, 1, 0xFFFF, 2].map(|layer| lists.total(p, 20, layer)),
        [5, 7, 9, 0]
    );
    let i = ext(&mut lists, &mut log, ITEM, UnitType::Item, &[]);
    lists.set(&mut log, i, 20, 100, 1, None);
    lists.attach(&mut log, P, i, true);
    assert_eq!(
        [0, 1, 0xFFFF].map(|layer| lists.total(p, 20, layer)),
        [5, 107, 9]
    );
}

#[test]
fn op_sources_and_bases_are_read_at_layer_0() {
    let (mut lists, mut log, p) = lists_with(data());
    // Vitality at layer 5 is not the op 9 source of maxhp.
    let i = ext(&mut lists, &mut log, ITEM, UnitType::Item, &[]);
    lists.set(&mut log, i, 3, 10, 5, None);
    lists.attach(&mut log, P, i, true);
    assert_eq!(lists.total(p, 3, 5), 10);
    assert_eq!(lists.total(p, 7, 0), 12800);
    // Per-level at layer 2 is not the op 2 source either.
    lists.set(&mut log, p, 216, 8, 2, None);
    assert_eq!(lists.total(p, 7, 0), 12800);
    // A caller's layer does not move the sources: eval of (7, layer 3)
    // reads vitality at layer 0.
    let j = ext(&mut lists, &mut log, I2, UnitType::Item, &[(3, 10)]);
    lists.attach(&mut log, P, j, true);
    assert_eq!(lists.eval(&log, p, key(7, 3)), (16 * 10) << 6);
    // The op base of op 2 too: level at layer 1 is not read.
    let (mut lists, mut log, p) = lists_with(data());
    lists.set(&mut log, p, 12, 0, 0, None);
    lists.set(&mut log, p, 12, 40, 1, None);
    lists.set(&mut log, p, 216, 8, 0, None);
    assert_eq!(lists.total(p, 7, 0), 12800);
}

#[test]
fn values_wrap() {
    let (mut lists, mut log, p) = lists_with(data());
    lists.set(&mut log, p, 20, i32::MAX, 0, None);
    let i = ext(&mut lists, &mut log, ITEM, UnitType::Item, &[(20, 1)]);
    lists.attach(&mut log, P, i, true);
    assert_eq!(lists.total(p, 20, 0), i32::MIN);
    assert_eq!(lists.eval(&log, p, key(20, 0)), i32::MIN);
    let c = plain(&mut lists, &mut log, &[]);
    lists.add(&mut log, c, 20, i32::MAX, 0);
    lists.add(&mut log, c, 20, 2, 0);
    assert_eq!(lists.base(c, 20, 0), i32::MIN + 1);
}

#[test]
fn shift_only_where_named() {
    // Sums of a shifted stat are raw: +100 is 100/256 of a point.
    let (mut lists, mut log, p) = lists_with(data());
    let i = ext(&mut lists, &mut log, ITEM, UnitType::Item, &[(7, 100)]);
    lists.attach(&mut log, P, i, true);
    assert_eq!(lists.total(p, 7, 0), 12900);
    // The op base is shifted by its own ValShift (level given shift 2):
    // x = 40 >> 2 = 10, (8 · 10) >> 3 = 10.
    let d = data_from(&itemstatcost_with(|r| rec(r, 12)[0x18] = 2));
    let (mut lists, mut log, p) = lists_with(d);
    lists.set(&mut log, p, 12, 40, 0, None);
    assert_eq!(lists.total(p, 12, 0), 40);
    lists.set(&mut log, p, 216, 8, 0, None);
    assert_eq!(lists.total(p, 7, 0), 12810);
}

/// Every column lists as not read: the
/// simulation's view of the table is the same whatever they hold.
#[test]
fn encoding_and_unread_columns_change_nothing() {
    let clean = StatTable::from_fixed(&itemstatcost_with(|_| {})).unwrap();
    let noisy = StatTable::from_fixed(&itemstatcost_with(|records| {
        for s in 0..N {
            let r = rec(records, s);
            // send/csv bits and params, divide, multiply, add.
            for (o, b) in r.iter_mut().enumerate().take(0x18).skip(0x08) {
                *b = (o as u8).wrapping_mul(37) ^ s as u8;
            }
            // save bits, 1.09-save bits, save add, 1.09-save add, save
            // param bits, up to MinAccr.
            for (o, b) in r.iter_mut().enumerate().take(0x2C).skip(0x19) {
                *b = (o as u8).wrapping_mul(11) ^ s as u8;
            }
            // encode, MaxStat, descriptions.
            for (o, b) in r.iter_mut().enumerate().take(0x48).skip(0x30) {
                *b = (o as u8).wrapping_mul(5) ^ s as u8;
            }
            // send other, signed, itemspecific, direct, updateanimrate,
            // csvsigned.
            r[4] |= 0x01 | 0x02 | 0x08 | 0x10;
            r[5] |= 0x02 | 0x20;
        }
    }))
    .unwrap();
    assert_eq!(clean, noisy);
}

#[test]
fn itemstatcost_columns_read() {
    let t = StatTable::from_fixed(&itemstatcost_with(|records| {
        let r = rec(records, 50);
        r[4] |= 0x04; // damagerelated, bit 2
        r[5] |= 0x04 | 0x08 | 0x10; // fMin, fCallback, Saved: bits 10, 11, 12
        r[0x2C..0x30].copy_from_slice(&3u32.to_le_bytes());
        r[0x18] = 5;
        r[0x50] = 1;
        set_u16(r, 0x48, 0x0102);
        set_op(records, 50, 5, 4, 12, &[51, 52]);
        set_op(records, 53, 1, 0, NO_STAT, &[51]);
        // The same bits elsewhere in the dword mean nothing here.
        let r = rec(records, 60);
        r[4] |= 0xFB;
        r[6] = 0xFF;
        r[7] = 0xFF;
        // Unread columns ("Not read"), and not decoded: same row.
        let r = rec(records, 61);
        r[4] |= 0x08 | 0x10; // itemspecific, direct
        r[5] |= 0x02; // updateanimrate
        set_u16(r, 0x32, 7); // MaxStat
        r[0x0C..0x18].fill(0x55); // Divide, Multiply, Add
    }))
    .unwrap();
    let i = t.get(50).unwrap();
    assert!(i.damagerelated && i.fmin && i.fcallback && i.saved);
    assert_eq!(
        (i.minaccr, i.valshift, i.keepzero, i.itemevent1),
        (3, 5, true, 0x0102)
    );
    assert_eq!(
        (i.op, i.op_param, i.op_base, i.op_stats),
        (5, 4, 12, [51, 52, NO_STAT])
    );
    // A51 (has op stats), A53 (op 5, valid base); 12 gets A51 and the
    // dep; 51 gets A52 and its entries in source order.
    assert!(i.a51 && !i.a52 && i.a53);
    let level = t.get(12).unwrap();
    assert!(level.a51 && level.deps.contains(&50));
    let target = t.get(51).unwrap();
    assert!(target.a52 && !target.a51 && !target.a53);
    assert_eq!(
        target.entries,
        [
            OpEntry {
                base: 12,
                source: 50,
                op: 5,
                param: 4
            },
            OpEntry {
                base: NO_STAT,
                source: 53,
                op: 1,
                param: 0
            }
        ]
    );
    let other = t.get(60).unwrap();
    assert!(!other.damagerelated && !other.fmin && !other.fcallback && !other.saved);
    let clean = StatTable::from_fixed(&itemstatcost_with(|_| {})).unwrap();
    assert_eq!(t.get(61), clean.get(61));
}

#[test]
fn readers() {
    let (mut lists, mut log, p) = lists_with(data());
    let i = ext(
        &mut lists,
        &mut log,
        ITEM,
        UnitType::Item,
        &[(0, 10), (20, 50)],
    );
    lists.attach(&mut log, P, i, true);
    // Base: the base array; total: the full array of an extended list.
    assert_eq!((lists.base(p, 0, 0), lists.total(p, 0, 0)), (30, 40));
    assert_eq!(
        (lists.unit_base(P, 0, 0), lists.unit_total(P, 0, 0)),
        (30, 40)
    );
    assert_eq!(lists.unit_bonus(P, 0, 0), 10);
    // Total of a plain list: its base array.
    let c = plain(&mut lists, &mut log, &[(0, 5)]);
    assert_eq!((lists.base(c, 0, 0), lists.total(c, 0, 0)), (5, 5));
    // Bonus = total − base, each with its own minimum rule: base −5
    // reads 256, total 295 reads 295.
    lists.set(&mut log, p, 7, -5, 0, None);
    let j = ext(&mut lists, &mut log, I2, UnitType::Item, &[(7, 300)]);
    lists.attach(&mut log, P, j, true);
    assert_eq!(
        (lists.unit_base(P, 7, 0), lists.unit_total(P, 7, 0)),
        (256, 295)
    );
    assert_eq!(lists.unit_bonus(P, 7, 0), 39);
    // Max life / mana / stamina: unit total of 7 / 9 / 11.
    lists.set(&mut log, p, 9, 700, 0, None);
    lists.set(&mut log, p, 11, 900, 0, None);
    assert_eq!(
        (lists.max_life(P), lists.max_mana(P), lists.max_stamina(P)),
        (295, 700, 900)
    );
    // Percent-adjusted: base + MulDiv(base, total(pct), 100) (+ bonus).
    assert_eq!(lists.percent_adjusted(p, 0, 20, false), 30 + 15);
    assert_eq!(lists.percent_adjusted(p, 0, 20, true), 30 + 15 + 10);
    // A unit without a list reads 0.
    let none = UnitId(99);
    assert_eq!(
        (
            lists.unit_total(none, 0, 0),
            lists.unit_base(none, 0, 0),
            lists.unit_bonus(none, 0, 0),
            lists.max_life(none)
        ),
        (0, 0, 0, 0)
    );
}

#[test]
fn muldiv_is_integer_signed_and_truncating() {
    assert_eq!(muldiv(-7, 1, 2), -3);
    assert_eq!(muldiv(7, 1, -2), -3);
    // Signed compares: −0x200002 is not > 0x100000, so (a · b) / c, not
    // (a / c) · b (= −3495255).
    assert_eq!(muldiv(-0x20_0002, 5, 3), -3_495_256);
    // ... and a negative a takes the 32-bit product, which wraps to 0.
    assert_eq!(muldiv(-0x20_0000, 0x1_0000, 3), 0);
    // Percents use it, not floating point: 100 · (29 / 100.0) = 28.99….
    let (mut lists, mut log, p) = lists_with(data());
    lists.set(&mut log, p, 7, 100, 0, None);
    let i = ext(&mut lists, &mut log, ITEM, UnitType::Item, &[(76, 29)]);
    lists.attach(&mut log, P, i, true);
    assert_eq!(lists.total(p, 7, 0), 129);
}

#[test]
fn evaluation_has_no_side_effects() {
    let (mut lists, mut log, p) = lists_with(data());
    let i = item_list(&mut lists, &mut log, &[(214, 8)]);
    lists.attach(&mut log, P, i, true);
    lists.set(&mut log, p, 12, 20, 0, None);
    log.callbacks.clear();
    let before = format!("{lists:?}");
    // Stale: the stored value is 10, a fresh evaluation gives 20.
    assert_eq!(lists.eval(&log, i, key(31, 0)), 20);
    assert_eq!(lists.eval(&log, p, key(7, 0)), 12800);
    assert_eq!(format!("{lists:?}"), before);
    assert_eq!(lists.total(i, 31, 0), 10);
    assert!(log.callbacks.is_empty());
}

#[test]
fn op_table_contributions() {
    // 1: MulDiv(prev, r, 100); prev = the item's own base of T.
    let (mut lists, mut log, p) = lists_with(custom());
    lists.set(&mut log, p, 41, 200, 0, None);
    lists.set(&mut log, p, 301, 50, 0, None);
    assert_eq!(lists.total(p, 41, 0), 300);
    let i = ext(
        &mut lists,
        &mut log,
        ITEM,
        UnitType::Item,
        &[(41, 80), (301, 50)],
    );
    assert_eq!(lists.total(i, 41, 0), 120);
    let c = plain(&mut lists, &mut log, &[(41, 20)]);
    lists.attach(&mut log, ITEM, c, true);
    // Sum 100, prev 80 (not 100): 140.
    assert_eq!(lists.total(i, 41, 0), 140);

    // 2: (r · level) >> param.
    let (mut lists, mut log, p) = lists_with(custom());
    lists.set(&mut log, p, 216, 8, 0, None);
    assert_eq!(lists.total(p, 7, 0), 12800 + ((8 * 10) >> 3));

    // 3: MulDiv(prev, (r · level) >> param, 100).
    let (mut lists, mut log, p) = lists_with(custom());
    lists.set(&mut log, p, 42, 1000, 0, None);
    lists.set(&mut log, p, 303, 8, 0, None);
    assert_eq!(lists.total(p, 42, 0), 1000 + 100);

    // 4: (r · wearer level) >> param.
    let (mut lists, mut log, _) = lists_with(custom());
    let i = ext(&mut lists, &mut log, ITEM, UnitType::Item, &[(214, 8)]);
    lists.attach(&mut log, P, i, true);
    assert_eq!(lists.total(i, 31, 0), (8 * 10) >> 3);

    // 5: MulDiv(prev, (r · wearer level) >> param, 100).
    let (mut lists, mut log, p) = lists_with(custom());
    let i = ext(
        &mut lists,
        &mut log,
        ITEM,
        UnitType::Item,
        &[(43, 1000), (305, 8)],
    );
    assert_eq!(lists.total(i, 43, 0), 1000);
    lists.attach(&mut log, P, i, true);
    assert_eq!(lists.total(i, 43, 0), 1100);
    assert_eq!(lists.total(p, 43, 0), 1100);

    // 6: ByTime(r); 7: MulDiv(acc, ByTime(r), 100), acc after op 6.
    let (mut lists, mut log, p) = lists_with(custom());
    log.act_time = Some(0);
    lists.set(&mut log, p, 270, by_time_value(50, 100), 0, None);
    assert_eq!(lists.total(p, 7, 0), 12900);
    lists.set(&mut log, p, 44, 1000, 0, None);
    lists.set(&mut log, p, 306, by_time_value(50, 100), 0, None);
    lists.set(&mut log, p, 307, by_time_value(50, 50), 0, None);
    assert_eq!(lists.total(p, 44, 0), 1100 + 550);

    // 8: (ManaPerMagic · bonus energy) << 6; 9: (LifePerVitality or
    // StaminaPerVitality · bonus vitality) << 6.
    let (mut lists, mut log, p) = lists_with(custom());
    lists.set(&mut log, p, 9, 5000, 0, None);
    let i = ext(
        &mut lists,
        &mut log,
        ITEM,
        UnitType::Item,
        &[(1, 4), (3, 10)],
    );
    lists.attach(&mut log, P, i, true);
    assert_eq!(lists.total(p, 9, 0), 5000 + ((8 * 4) << 6));
    assert_eq!(lists.total(p, 7, 0), 12800 + ((16 * 10) << 6));
    assert_eq!(lists.total(p, 11, 0), (4 * 10) << 6);

    // 10, 12: nothing.
    let (mut lists, mut log, p) = lists_with(custom());
    lists.set(&mut log, p, 45, 500, 0, None);
    lists.set(&mut log, p, 310, 7, 0, None);
    lists.set(&mut log, p, 312, 9, 0, None);
    assert_eq!(lists.total(p, 45, 0), 500);

    // 11: MulDiv(prev, r, 100) on a player; not on an item.
    let (mut lists, mut log, p) = lists_with(custom());
    let i = ext(
        &mut lists,
        &mut log,
        ITEM,
        UnitType::Item,
        &[(7, 1000), (76, 50)],
    );
    assert_eq!(lists.total(i, 7, 0), 1000);
    lists.attach(&mut log, P, i, true);
    // prev = 12800 + 1000.
    assert_eq!(lists.total(p, 7, 0), 13800 + 6900);

    // 13: MulDiv(item base of T, r, 100) on an item; not on a player.
    let (mut lists, mut log, p) = lists_with(custom());
    lists.set(&mut log, p, 73, 40, 0, None);
    lists.set(&mut log, p, 75, 50, 0, None);
    assert_eq!(lists.total(p, 73, 0), 40);
    let i = ext(
        &mut lists,
        &mut log,
        ITEM,
        UnitType::Item,
        &[(73, 40), (75, 50)],
    );
    assert_eq!(lists.total(i, 73, 0), 60);
}

#[test]
fn reads_never_evaluate() {
    let (mut lists, mut log, p) = lists_with(data());
    let i = item_list(&mut lists, &mut log, &[(214, 8)]);
    lists.attach(&mut log, P, i, true);
    assert_eq!((lists.total(i, 31, 0), lists.total(p, 31, 0)), (10, 10));
    // The wearer's level changes: the item's op 4 value stays stale.
    lists.set(&mut log, p, 12, 20, 0, None);
    assert_eq!((lists.total(i, 31, 0), lists.total(p, 31, 0)), (10, 10));
    // ... until deps(level) runs on the item's own list.
    lists.set(&mut log, i, 12, 1, 0, None);
    assert_eq!(lists.total(i, 31, 0), 20);
    // By-time values move only with a refresh.
    let (mut lists, mut log, p) = lists_with(data());
    let j = item_list(&mut lists, &mut log, &[(270, by_time_value(50, 100))]);
    lists.attach(&mut log, P, j, true);
    log.act_time = Some(0);
    assert_eq!(lists.total(p, 7, 0), 12800);
    lists.by_time_refresh(&mut log, P, j);
    assert_eq!(lists.total(p, 7, 0), 12900);
    log.act_time = Some(180);
    assert_eq!(lists.total(p, 7, 0), 12900);
    lists.by_time_refresh(&mut log, P, j);
    assert_eq!(lists.total(p, 7, 0), 12850);
}

#[test]
fn max_getters_read_layer_0_with_minimum_rule() {
    let (mut lists, mut log, p) = lists_with(data());
    for s in [7, 9, 11] {
        lists.set(&mut log, p, s, -5, 0, None);
        lists.set(&mut log, p, s, 999, 1, None);
    }
    // MinAccr 1 (7) → 256; MinAccr 0 (9, 11) → 0.
    assert_eq!(
        (lists.max_life(P), lists.max_mana(P), lists.max_stamina(P)),
        (256, 0, 0)
    );
}

#[test]
fn max_changes_rescale_current() {
    let (mut lists, mut log, p) = lists_with(data());
    lists.set(&mut log, p, 11, 2560, 0, None);
    lists.set(&mut log, p, 10, 1280, 0, None);
    lists.set(&mut log, p, 9, 1000, 0, None);
    lists.set(&mut log, p, 8, 500, 0, None);
    lists.set(&mut log, p, 11, 5120, 0, None);
    lists.set(&mut log, p, 9, 2000, 0, None);
    lists.set(&mut log, p, 7, 6400, 0, None);
    assert_eq!([6, 8, 10].map(|s| lists.total(p, s, 0)), [6400, 1000, 2560]);
}

#[test]
fn op_bases_raw_on_the_list_minimum_rule_on_the_unit() {
    let (mut lists, mut log, p) = lists_with(custom());
    // Op 2 (320 → 46, base strength): raw strength −5 → skipped,
    // although strength reads 1 by the minimum rule.
    lists.set(&mut log, p, 0, -5, 0, None);
    assert_eq!(lists.total(p, 0, 0), 1);
    lists.set(&mut log, p, 320, 8, 0, None);
    assert_eq!(lists.total(p, 46, 0), 0);
    // Op 4 (321 → 47, base strength) on an item: the wearer's strength
    // with its minimum rule, x = 1.
    let i = ext(&mut lists, &mut log, ITEM, UnitType::Item, &[(321, 8)]);
    assert_eq!(lists.total(i, 47, 0), 0);
    lists.attach(&mut log, P, i, true);
    assert_eq!(lists.total(i, 47, 0), 8);
    // Op 2 with raw strength 3.
    lists.set(&mut log, p, 0, 3, 0, None);
    assert_eq!(lists.total(p, 46, 0), 24);
}

// ---- -------------------------------------------------------------

#[test]
fn list_records() {
    let (mut lists, mut log, p) = lists_with(data());
    let l = lists.alloc(0x40, 7, owner::MONSTER, 99);
    assert!(!lists.is_extended(l));
    assert_eq!(
        (lists.owner_type(l), lists.owner_guid(l), lists.flags(l)),
        (owner::MONSTER, 99, 0x40)
    );
    assert_eq!(
        (lists.state(l), lists.expire(l), lists.skill(l)),
        (0, 7, (0, 0))
    );
    assert_eq!(
        (
            lists.parent(l),
            lists.prev(l),
            lists.next(l),
            lists.attached_unit(l)
        ),
        (None, None, None, None)
    );
    assert_eq!(lists.owner(l), None);
    assert!(lists.full_entries(l).is_empty() && lists.mods(l).is_empty());
    lists.set_state(l, 30);
    lists.set_skill(l, 5, 3);
    assert_eq!((lists.state(l), lists.skill(l)), (30, (5, 3)));
    // Array entry: the key is (stat << 16) + layer.
    lists.set(&mut log, l, 20, 4, 3, None);
    assert_eq!(lists.base_entries(l), [(0x0014_0003, 4)]);
    // Extended: owner unit, chains, full and mod arrays, 2 × W state words.
    assert!(lists.is_extended(p));
    assert_eq!(lists.owner(p), Some(P));
    let (on, changed) = lists.state_bits(P).unwrap();
    assert_eq!((on.len(), changed.len()), (6, 6));
    // Chains: the head is the newest child; prev older, next newer.
    let a = plain(&mut lists, &mut log, &[]);
    let b = plain(&mut lists, &mut log, &[]);
    lists.attach(&mut log, P, a, true);
    lists.attach(&mut log, P, b, true);
    assert_eq!(lists.heads(p), (Some(b), None));
    assert_eq!((lists.prev(b), lists.next(a)), (Some(a), Some(b)));
    assert_eq!((lists.prev(a), lists.next(b)), (None, None));
    assert_eq!(
        (lists.parent(a), lists.attached_unit(a)),
        (Some(p), Some(P))
    );
}

#[test]
fn list_flags() {
    assert_eq!(
        [
            flag::BASIC,
            flag::NEWLENGTH,
            flag::TEMPONLY,
            flag::OVERLAY,
            flag::REMOVE_OVERLAY,
            flag::SET,
            flag::PERMANENT,
            flag::DYNAMIC,
            flag::EXTENDED
        ],
        [
            1,
            2,
            4,
            0x80,
            0x100,
            0x2000,
            0x2000_0000,
            0x4000_0000,
            0x8000_0000
        ]
    );
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    // BASIC is the only allocation bit an extended list keeps.
    let r = lists.alloc_extended(&mut log, P, UnitType::Player, 1, 0, u32::MAX, None);
    assert_eq!(lists.flags(r), flag::BASIC | flag::EXTENDED);
    // NEWLENGTH with an expire frame > 0 only.
    let l = lists.alloc(0, 0, owner::PLAYER, 1);
    lists.set_expire(l, 0);
    assert_eq!(lists.flags(l) & flag::NEWLENGTH, 0);
    lists.set_expire(l, 1);
    assert_ne!(lists.flags(l) & flag::NEWLENGTH, 0);
    // PERMANENT is never cleared, even when the A53 stat goes.
    let i = item_list(&mut lists, &mut log, &[(214, 8)]);
    lists.set(&mut log, i, 214, 0, 0, None);
    assert!(lists.full_entries(i).is_empty());
    assert_ne!(lists.flags(i) & flag::PERMANENT, 0);
    // DYNAMIC: attach without reset sets it, with reset clears it.
    let j = ext(&mut lists, &mut log, I2, UnitType::Item, &[]);
    lists.attach(&mut log, P, j, false);
    assert_ne!(lists.flags(j) & flag::DYNAMIC, 0);
    lists.attach(&mut log, P, j, true);
    assert_eq!(lists.flags(j) & flag::DYNAMIC, 0);
    // Other bits are the callers' lookups.
    let k = lists.alloc(0x40, 0, owner::PLAYER, 1);
    lists.attach(&mut log, P, k, true);
    assert_eq!(lists.list_by_flags(r, 0x40), Some(k));
}

#[test]
fn arrays_sorted_by_signed_key() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let l = lists.alloc(0, 0, owner::PLAYER, 1);
    for (s, layer) in [(5, 2), (3, 0), (0x8000, 0), (5, 0), (0xFFFF, 0), (3, 0)] {
        lists.set(&mut log, l, s, 1 + layer as i32, layer, None);
    }
    // Stat ids ≥ 0x8000 (stored unvalidated) have negative keys: first.
    let keys: Vec<i32> = lists.base_entries(l).into_iter().map(|(k, _)| k).collect();
    assert_eq!(
        keys,
        [
            key(0x8000, 0),
            key(0xFFFF, 0),
            key(3, 0),
            key(5, 0),
            key(5, 2)
        ]
    );
    // Remove from the middle; insert into it.
    lists.set(&mut log, l, 3, 0, 0, None);
    lists.set(&mut log, l, 4, 9, 0, None);
    assert_eq!(
        lists.base_entries(l),
        [
            (key(0x8000, 0), 1),
            (key(0xFFFF, 0), 1),
            (key(4, 0), 9),
            (key(5, 0), 1),
            (key(5, 2), 3)
        ]
    );
}

#[test]
fn mod_array_sorted_unique() {
    let (mut lists, mut log, p) = lists_with(data());
    lists.clear_mods(P);
    for (s, v, layer) in [(12, 11, 0), (3, 1, 2), (3, 26, 0), (0, 31, 0), (12, 12, 0)] {
        lists.set(&mut log, p, s, v, layer, None);
    }
    assert_eq!(lists.mods(p), [key(0, 0), key(3, 0), key(3, 2), key(12, 0)]);
}

#[test]
fn zero_values_leave_the_arrays() {
    let (mut lists, mut log, p) = lists_with(data());
    lists.set(&mut log, p, 20, 5, 0, None);
    lists.add(&mut log, p, 20, -5, 0);
    lists.set(&mut log, p, 8, 100, 0, None);
    lists.set(&mut log, p, 8, 0, 0, None);
    assert!(!lists.base_entries(p).iter().any(|&(_, v)| v == 0));
    // Full: 0 kept only for keepzero (8), not for 20.
    let f = lists.full_entries(p);
    assert!(f.contains(&(key(8, 0), 0)));
    assert!(!f.iter().any(|&(k, _)| k == key(20, 0)));
    assert_eq!(f.iter().filter(|&&(_, v)| v == 0).count(), 1);
    // Merge into 0 and remove all: no 0 entry ever, so remove-all ends.
    let c = plain(&mut lists, &mut log, &[(0, 2), (20, -3)]);
    let d = plain(&mut lists, &mut log, &[(0, -2), (2, 7)]);
    lists.merge(&mut log, c, d);
    assert_eq!(lists.base_entries(c), [(key(2, 0), 7), (key(20, 0), -3)]);
    lists.remove_all(&mut log, c);
    assert!(lists.base_entries(c).is_empty());
}

#[test]
fn plain_allocation() {
    let mut lists = StatLists::new(data());
    let l = lists.alloc(0, 20, owner::ITEM, 9);
    assert_eq!((lists.expire(l), lists.flags(l)), (20, 0));
    assert_eq!((lists.owner_type(l), lists.owner_guid(l)), (owner::ITEM, 9));
}

#[test]
fn extended_allocation() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let old = lists.alloc_extended(&mut log, P, UnitType::Player, 1, 0, 0, None);
    lists.toggle_state(P, 30, true);
    let l = lists.alloc_extended(&mut log, P, UnitType::Monster, 77, 0, 0x5, None);
    assert!(!lists.is_live(old));
    assert_eq!(lists.unit_list(P), Some(l));
    assert_eq!(
        (lists.owner_type(l), lists.owner_guid(l), lists.owner(l)),
        (owner::MONSTER, 77, Some(P))
    );
    assert_eq!(lists.flags(l), flag::BASIC | flag::EXTENDED);
    let (on, changed) = lists.state_bits(P).unwrap();
    assert!(on.iter().chain(&changed).all(|&w| w == 0));
}

#[test]
fn base_writes_use_stat_and_layer_key() {
    let (mut lists, mut log, p) = lists_with(data());
    lists.set(&mut log, p, 20, 4, 7, None);
    lists.add(&mut log, p, 20, 3, 7);
    assert!(lists.base_entries(p).contains(&(0x0014_0007, 7)));
    assert_eq!(lists.base(p, 20, 0), 0);
}

#[test]
fn unit_set_wrapper() {
    let (mut lists, mut log, p) = lists_with(data());
    lists.clear_mods(P);
    // Unchanged value: set itself returns early, the wrapper still
    // inserts the mod key.
    lists.set(&mut log, p, 0, 30, 0, None);
    assert!(lists.mods(p).is_empty());
    lists.unit_set(&mut log, P, 0, 30, 0);
    assert_eq!(lists.mods(p), [key(0, 0)]);
    lists.unit_set(&mut log, P, 2, 9, 0);
    assert_eq!(lists.base(p, 2, 0), 9);
    // No list: nothing.
    lists.unit_set(&mut log, UnitId(99), 2, 9, 0);
    assert_eq!(lists.unit_total(UnitId(99), 2, 0), 0);
}

#[test]
fn dynamic_extended_list_keeps_its_damage_related_changes() {
    let (mut lists, mut log, p) = lists_with(data());
    let i = ext(&mut lists, &mut log, ITEM, UnitType::Item, &[]);
    lists.attach(&mut log, P, i, false);
    lists.set(&mut log, i, 19, 5, 0, None);
    lists.set(&mut log, i, 20, 5, 0, None);
    assert_eq!((lists.total(i, 19, 0), lists.total(p, 19, 0)), (5, 0));
    assert_eq!(lists.total(p, 20, 0), 5);
    // A plain DYNAMIC child: the change reaches the parent.
    let c = plain(&mut lists, &mut log, &[]);
    lists.attach(&mut log, P, c, false);
    lists.set(&mut log, c, 19, 4, 0, None);
    assert_eq!(lists.total(p, 19, 0), 4);
}

#[test]
fn propagation_starts_at_the_list_or_its_parent() {
    let (mut lists, mut log, p) = lists_with(data());
    // A plain list without a parent: base only.
    let c = plain(&mut lists, &mut log, &[(20, 3)]);
    assert_eq!(lists.base(c, 20, 0), 3);
    assert_eq!(lists.total(p, 20, 0), 0);
    // A plain list with a parent: the parent's full entry.
    lists.attach(&mut log, P, c, true);
    lists.set(&mut log, c, 20, 5, 0, None);
    assert_eq!(lists.total(p, 20, 0), 5);
    assert!(lists.full_entries(c).is_empty());
    // An extended list: its own full entry first, then its parent.
    let i = ext(&mut lists, &mut log, ITEM, UnitType::Item, &[]);
    lists.attach(&mut log, P, i, true);
    lists.set(&mut log, i, 20, 2, 0, None);
    assert_eq!((lists.total(i, 20, 0), lists.total(p, 20, 0)), (2, 7));
}

#[test]
fn full_values_are_the_evaluation() {
    let (mut lists, mut log, p) = lists_with(data());
    lists.set(&mut log, p, 216, 8, 0, None);
    let i = item_list(&mut lists, &mut log, &[(3, 10), (76, 20), (19, 2)]);
    lists.attach(&mut log, P, i, true);
    for s in [3, 7, 11, 19] {
        assert_eq!(lists.total(p, s, 0), lists.eval(&log, p, key(s, 0)), "{s}");
    }
}

#[test]
fn set_full_and_add_full() {
    let (mut lists, mut log, p) = lists_with(custom());
    // Add-full (A51 = A52 = 0): PERMANENT only when new > 0.
    let i = ext(&mut lists, &mut log, ITEM, UnitType::Item, &[(330, -3)]);
    assert_eq!(lists.full_entries(i), [(key(330, 0), -3)]);
    assert_eq!(lists.flags(i) & flag::PERMANENT, 0);
    lists.add(&mut log, i, 330, 5, 0);
    assert_ne!(lists.flags(i) & flag::PERMANENT, 0);
    // Set-full (recompute): PERMANENT on any stored A53 value.
    let j = ext(&mut lists, &mut log, I2, UnitType::Item, &[(331, -5)]);
    assert_eq!(lists.full_entries(j), [(key(331, 0), -5)]);
    assert_ne!(lists.flags(j) & flag::PERMANENT, 0);
    // Add-full notifies always (fCallback stat 341).
    log.callbacks.clear();
    lists.set(&mut log, p, 341, 5, 0, None);
    assert_eq!(log.callbacks, [(341, 0, 5)]);
    // Set-full notifies only on a change: MulDiv(50, 1, 100) = 0.
    lists.set(&mut log, p, 7, 50, 0, None);
    log.callbacks.clear();
    let k = ext(&mut lists, &mut log, I3, UnitType::Item, &[(76, 1)]);
    lists.attach(&mut log, P, k, true);
    assert_eq!(lists.total(p, 7, 0), 50);
    assert!(log.callbacks.is_empty());
    // Set-full of 0: removed, unless keepzero (45).
    lists.set(&mut log, p, 45, 500, 0, None);
    lists.set(&mut log, p, 45, 0, 0, None);
    assert!(lists.full_entries(p).contains(&(key(45, 0), 0)));
    lists.set(&mut log, p, 42, 500, 0, None);
    lists.set(&mut log, p, 42, 0, 0, None);
    assert!(!lists.full_entries(p).iter().any(|&(k, _)| k == key(42, 0)));
}

#[test]
fn recompute_returns_the_evaluation() {
    let (mut lists, mut log, p) = lists_with(data());
    lists.set(&mut log, p, 216, 8, 0, None);
    // Blocked (strength 30 > 0): not stored, still returned.
    assert!(!lists.full_entries(p).iter().any(|&(k, _)| k == key(216, 0)));
    assert_eq!(lists.recompute(&mut log, p, key(216, 0), None), 8);
    // A51 = 0 (maxhp): stored as evaluated. Make it stale first.
    lists.set(&mut log, p, 12, 0, 0, None);
    assert_eq!(lists.total(p, 7, 0), 12810);
    assert_eq!(lists.recompute(&mut log, p, key(7, 0), None), 12800);
    assert_eq!(lists.total(p, 7, 0), 12800);
}

#[test]
fn recompute_deps_only_when_updated_and_nonzero() {
    let (mut lists, mut log, p) = lists_with(data());
    lists.set(&mut log, p, 216, 8, 0, None);
    assert_eq!(lists.total(p, 7, 0), 12800 + 10);
    // Level 20: deps(12) recomputed → 216 → maxhp.
    lists.set(&mut log, p, 12, 20, 0, None);
    assert_eq!(lists.total(p, 7, 0), 12800 + 20);
    // Level 0: v = 0 → set-full(12, 0), no deps.
    lists.set(&mut log, p, 12, 0, 0, None);
    assert!(!lists.full_entries(p).iter().any(|&(k, _)| k == key(12, 0)));
    assert_eq!(lists.total(p, 7, 0), 12800 + 20);
    assert_eq!(lists.eval(&log, p, key(7, 0)), 12800);
}

#[test]
fn recompute_blocks() {
    // none (op 9): vitality is stored.
    let (mut lists, mut log, p) = lists_with(custom());
    assert!(lists.full_entries(p).contains(&(key(3, 0), 25)));
    // always (op 1, op 12): never stored.
    lists.set(&mut log, p, 41, 10, 0, None);
    lists.set(&mut log, p, 301, 50, 0, None);
    lists.set(&mut log, p, 45, 10, 0, None);
    lists.set(&mut log, p, 312, 9, 0, None);
    lists.set(&mut log, p, 310, 7, 0, None);
    let f = lists.full_entries(p);
    let stored = |f: &[(i32, i32)], s: u16| f.iter().any(|&(k, _)| k == key(s, 0));
    assert!(!stored(&f, 301) && !stored(&f, 312) && stored(&f, 310));
    // listtype_pm (op 6): blocked on a player, not on an item.
    lists.set(&mut log, p, 270, by_time_value(50, 100), 0, None);
    assert!(!stored(&lists.full_entries(p), 270));
    let i = ext(
        &mut lists,
        &mut log,
        ITEM,
        UnitType::Item,
        &[(7, 100), (270, 9)],
    );
    assert!(stored(&lists.full_entries(i), 270));
    // listtype_item (op 13): blocked on an item, not on a player.
    let j = ext(
        &mut lists,
        &mut log,
        I2,
        UnitType::Item,
        &[(73, 40), (75, 50)],
    );
    assert!(!stored(&lists.full_entries(j), 75));
    lists.set(&mut log, p, 73, 40, 0, None);
    lists.set(&mut log, p, 75, 50, 0, None);
    assert!(stored(&lists.full_entries(p), 75));

    // listtype_pm_and_entrybase_list_total_pos (op 2): the unused entry
    // reads base 0 = strength, raw on the list.
    let (mut lists, mut log, p) = lists_with(data());
    lists.set(&mut log, p, 216, 8, 0, None);
    assert!(!lists.full_entries(p).iter().any(|&(k, _)| k == key(216, 0)));
    let (mut lists, mut log, p) = lists_with(data());
    lists.set(&mut log, p, 0, -5, 0, None);
    lists.set(&mut log, p, 216, 8, 0, None);
    assert!(lists.full_entries(p).contains(&(key(216, 0), 8)));

    // unit_pm_and_entrybase_unit_total_pos (op 4): the wearer's strength
    // with its minimum rule.
    for (strength, blocked) in [(30, true), (-5, true), (0, false)] {
        let (mut lists, mut log, p) = lists_with(data());
        lists.set(&mut log, p, 0, strength, 0, None);
        let i = item_list(&mut lists, &mut log, &[(214, 8)]);
        lists.attach(&mut log, P, i, true);
        lists.set(&mut log, i, 214, 16, 0, None);
        let v = if blocked { 8 } else { 16 };
        assert!(
            lists.full_entries(i).contains(&(key(214, 0), v)),
            "strength {strength}"
        );
    }
    // Not attached: no unit, not blocked.
    let (mut lists, mut log, _) = lists_with(data());
    let i = item_list(&mut lists, &mut log, &[(214, 8)]);
    lists.set(&mut log, i, 214, 16, 0, None);
    assert!(lists.full_entries(i).contains(&(key(214, 0), 16)));
}

/// A host recording item-event registrations.
#[derive(Default)]
struct Events {
    log: Log,
    item_events: Vec<(UnitId, u16, i32)>,
}

impl StatHost for Events {
    fn item_event(&mut self, _lists: &mut StatLists, owner: UnitId, stat: u16, new: i32) {
        self.item_events.push((owner, stat, new));
    }
}

#[test]
fn item_event_registration() {
    let mut h = Events::default();
    let mut lists = StatLists::new(custom());
    let p = player(&mut lists, &mut h.log);
    lists.set(&mut h, p, 340, 5, 0, None);
    lists.set(&mut h, p, 340, 0, 0, None);
    // itemevent1 = 0xFFFF (−1 as i16): no registration.
    lists.set(&mut h, p, 341, 5, 0, None);
    assert_eq!(h.item_events, [(P, 340, 5), (P, 340, 0)]);
}

#[test]
fn server_callback_on_players_and_monsters_only() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let m = UnitId(5);
    let l = lists.alloc_extended(
        &mut log,
        m,
        UnitType::Monster,
        3,
        0,
        0,
        Some(ValueCallback::Server),
    );
    lists.set(&mut log, l, 7, 25600, 0, None);
    lists.set(&mut log, l, 6, 12800, 0, None);
    lists.set(&mut log, l, 7, 51200, 0, None);
    assert_eq!(lists.total(l, 6, 0), 25600);
    // An item list has no callback: nothing is rescaled.
    let i = ext(
        &mut lists,
        &mut log,
        ITEM,
        UnitType::Item,
        &[(7, 25600), (6, 12800)],
    );
    lists.set(&mut log, i, 7, 51200, 0, None);
    assert_eq!(lists.total(i, 6, 0), 12800);
}

#[test]
fn attach_needs_the_units_extended_list() {
    let (mut lists, mut log, p) = lists_with(data());
    let c = plain(&mut lists, &mut log, &[(20, 3)]);
    lists.attach(&mut log, UnitId(99), c, true);
    assert_eq!((lists.parent(c), lists.attached_unit(c)), (None, None));
    assert_eq!(lists.total(p, 20, 0), 0);
}

#[test]
fn attach_detaches_first() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    let q = ext(&mut lists, &mut log, I2, UnitType::Player, &[]);
    let c = plain(&mut lists, &mut log, &[(20, 3)]);
    lists.attach(&mut log, P, c, true);
    lists.attach(&mut log, I2, c, true);
    assert_eq!((lists.total(p, 20, 0), lists.total(q, 20, 0)), (0, 3));
    assert!(lists.active_chain(p).is_empty());
    assert_eq!(lists.active_chain(q), [c]);
    assert_eq!(lists.attached_unit(c), Some(I2));
}

#[test]
fn detach_moves_the_heads() {
    let (mut lists, mut log, p) = lists_with(data());
    let a = plain(&mut lists, &mut log, &[]);
    let b = plain(&mut lists, &mut log, &[]);
    lists.attach(&mut log, P, a, true);
    lists.attach(&mut log, P, b, true);
    lists.detach(&mut log, b);
    assert_eq!(lists.heads(p), (Some(a), None));
    assert_eq!(lists.parent(b), None);
    let s = lists.alloc(flag::SET, 0, owner::PLAYER, 1);
    let t = lists.alloc(flag::SET, 0, owner::PLAYER, 1);
    lists.attach(&mut log, P, s, true);
    lists.attach(&mut log, P, t, true);
    assert_eq!(lists.heads(p), (Some(a), Some(t)));
    lists.detach(&mut log, t);
    assert_eq!(lists.heads(p), (Some(a), Some(s)));
    lists.detach(&mut log, s);
    lists.detach(&mut log, a);
    assert_eq!(lists.heads(p), (None, None));
}

#[test]
fn detaching_a_parked_list_moves_nothing() {
    let (mut lists, mut log, p) = lists_with(data());
    let s = lists.alloc(flag::SET, 0, owner::PLAYER, 1);
    lists.set(&mut log, s, 20, 3, 0, None);
    lists.set_state(s, 30);
    lists.set_remove_callback(s, Some(RemoveCallback(1)));
    lists.attach(&mut log, P, s, true);
    lists.set(&mut log, p, 20, 10, 0, None);
    lists.detach(&mut log, s);
    assert_eq!(lists.total(p, 20, 0), 10);
    assert!(log.removed.is_empty());
    assert_eq!(lists.attached_unit(s), None);
}

#[test]
fn free_detaches_then_releases() {
    let (mut lists, mut log, p) = lists_with(data());
    let c = plain(&mut lists, &mut log, &[(20, 3)]);
    lists.set_state(c, 30);
    lists.set_remove_callback(c, Some(RemoveCallback(1)));
    lists.attach(&mut log, P, c, true);
    assert_eq!(lists.total(p, 20, 0), 3);
    lists.free(&mut log, c);
    assert_eq!(lists.total(p, 20, 0), 0);
    assert_eq!(log.removed, [(P, 30)]);
    assert!(lists.active_chain(p).is_empty());
    assert!(!lists.is_live(c));
    // The released slot is reused under a new id.
    let d = lists.alloc(0, 0, owner::PLAYER, 1);
    assert_ne!(d, c);
    assert!(!lists.is_live(c));
}

#[test]
fn free_leaves_parked_children_pointing_at_the_parent() {
    let (mut lists, mut log, p) = lists_with(data());
    let s = lists.alloc(flag::SET, 0, owner::PLAYER, 1);
    lists.attach(&mut log, P, s, true);
    lists.free(&mut log, p);
    assert!(!lists.is_live(p));
    assert!(lists.is_live(s));
    assert_eq!(
        (lists.parent(s), lists.attached_unit(s)),
        (Some(p), Some(P))
    );
}

// ---- edge case 4 and stale handles (handoff 7n) ----------------------------------

/// Edge case 4: an expired extended list in the active chain
/// loops forever in the base game. D2RUST stops the walk there with an error, in
/// the state the base game spins in: the plain due list met first is
/// freed (its remove callback runs), the extended one and every list
/// behind it stay.
#[test]
fn expired_extended_list_stops_the_walk() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    // Deepest: a plain due list behind the extended one.
    let behind = lists.alloc(0, 0, owner::PLAYER, 1);
    lists.set_state(behind, 40);
    lists.set_expire(behind, 10);
    lists.attach(&mut log, P, behind, true);
    let i = item_list(&mut lists, &mut log, &[(0, 4)]);
    lists.set_expire(i, 10);
    lists.attach(&mut log, P, i, true);
    // Head: a plain due list with a remove callback.
    let s = lists.alloc(0, 0, owner::PLAYER, 1);
    lists.set_state(s, 30);
    lists.set_expire(s, 10);
    lists.set(&mut log, s, 0, 5, 0, None);
    lists.set_remove_callback(s, Some(RemoveCallback(1)));
    lists.attach(&mut log, P, s, true);
    assert_eq!(lists.active_chain(p), [s, i, behind]);

    assert_eq!(
        lists.expire_lists(&mut log, P, 10),
        Err(StatListError::EndlessExpiry(i))
    );
    assert!(!lists.is_live(s));
    assert_eq!(log.removed, [(P, 30)]);
    assert_eq!(lists.active_chain(p), [i, behind]);
    assert_eq!(lists.total(p, 0, 0), 34);
    // Not due yet: the walk passes the extended list.
    let mut lists2 = lists.clone();
    assert_eq!(lists2.expire_lists(&mut log, P, 9), Ok(()));
    assert_eq!(lists2.active_chain(p), [i, behind]);
}

/// Stale handles: a freed [`ListId`] (also once its slot is reused) reads
/// as a null list (reads 0;: writes
/// do nothing;: a missing item list does nothing) and changes no list.
#[test]
fn stale_list_handles_act_as_null_lists() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    let stale = item_list(&mut lists, &mut log, &[(0, 4)]);
    lists.attach(&mut log, P, stale, true);
    lists.free(&mut log, stale);
    let plain = lists.alloc(0, 0, owner::PLAYER, 1);
    let stale_plain = plain;
    lists.free(&mut log, plain);
    // Reuse the freed slots with live lists that must stay untouched.
    let reused = lists.alloc(0, 7, owner::MONSTER, 9);
    lists.set(&mut log, reused, 0, 3, 0, None);
    lists.attach(&mut log, P, reused, true);
    let before = format!("{lists:?}");
    log.callbacks.clear();
    for l in [stale, stale_plain] {
        assert!(!lists.is_live(l));
        assert_eq!(lists.flags(l), 0);
        assert!(!lists.is_extended(l));
        assert_eq!((lists.owner_type(l), lists.owner_guid(l)), (0, 0));
        assert_eq!(lists.owner(l), None);
        assert_eq!(lists.attached_unit(l), None);
        assert_eq!(
            (lists.parent(l), lists.prev(l), lists.next(l)),
            (None, None, None)
        );
        assert_eq!(lists.heads(l), (None, None));
        assert_eq!(
            (lists.state(l), lists.expire(l), lists.skill(l)),
            (0, 0, (0, 0))
        );
        assert!(lists.base_entries(l).is_empty());
        assert!(lists.full_entries(l).is_empty());
        assert!(lists.mods(l).is_empty());
        assert!(lists.active_chain(l).is_empty() && lists.parked_chain(l).is_empty());
        assert_eq!((lists.base(l, 0, 0), lists.total(l, 0, 0)), (0, 0));
        assert_eq!(lists.percent_adjusted(l, 0, 3, true), 0);
        assert_eq!(lists.eval(&log, l, 0), 0);
        assert_eq!(lists.list_of_state(l, 0), None);
        assert_eq!(lists.list_by_flags(l, 0), None);

        lists.set_flags(l, flag::DYNAMIC, true);
        lists.set_state(l, 30);
        lists.set_expire(l, 5);
        lists.set_skill(l, 1, 2);
        lists.set_remove_callback(l, Some(RemoveCallback(1)));
        lists.propagate(&mut log, l, 0, 5, None);
        assert_eq!(lists.recompute(&mut log, l, 0, None), 0);
        assert!(!lists.set(&mut log, l, 0, 5, 0, None));
        lists.add(&mut log, l, 0, 5, 0);
        lists.remove_all(&mut log, l);
        lists.merge(&mut log, l, reused);
        lists.merge(&mut log, reused, l);
        lists.attach(&mut log, P, l, true);
        lists.detach(&mut log, l);
        lists.unit_detach(&mut log, l);
        lists.equip(&mut log, P, Some(l), false, true);
        lists.equip(&mut log, P, Some(l), true, true);
        lists.make_static(&mut log, P, l, false);
        lists.make_dynamic(&mut log, P, l, false);
        lists.by_time_refresh(&mut log, P, l);
        lists.free_plain(&mut log, l);
        lists.free(&mut log, l);
    }
    assert_eq!(format!("{lists:?}"), before);
    assert!(log.callbacks.is_empty());
    assert_eq!(lists.total(p, 0, 0), 33);
}

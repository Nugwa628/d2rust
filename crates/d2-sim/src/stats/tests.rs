//! Test vectors of the stats and stat lists on a synthetic
//! itemstatcost table built through the d2-data load fix-up.

use std::sync::Arc;

use d2_data::bin::BinTable;
use d2_data::fixup::records::stat_ops;
use d2_data::tables::Itemstatcost;
use d2_data::tables::Record;

use super::lists::{flag, owner, NoHost};
use super::states::state;
use super::*;
use crate::units::UnitId;

pub(crate) const N: usize = 359;

pub(super) fn set_u16(r: &mut [u8], o: usize, v: u16) {
    r[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

/// A synthetic itemstatcost with the base-game columns the test vectors use,
/// fixed up by `d2_data::fixup::records::stat_ops`.
pub(crate) fn itemstatcost() -> BinTable {
    itemstatcost_with(|_| {})
}

/// [`itemstatcost`] with `edit` applied to the records (0x144 bytes
/// each) before the load fix-up.
pub(crate) fn itemstatcost_with(edit: impl FnOnce(&mut [u8])) -> BinTable {
    let size = Itemstatcost::SIZE;
    let mut records = vec![0u8; N * size];
    {
        fn rec(records: &mut [u8], s: usize) -> &mut [u8] {
            let size = Itemstatcost::SIZE;
            &mut records[s * size..(s + 1) * size]
        }
        let records = &mut records[..];
        for s in 0..N {
            let r = rec(records, s);
            for o in [0x32, 0x48, 0x4A, 0x56, 0x58, 0x5A, 0x5C] {
                set_u16(r, o, 0xFFFF);
            }
            set_u16(r, 0, s as u16);
        }
        for s in [6, 7, 8, 9, 10, 11, 216, 217] {
            rec(records, s)[0x18] = 8;
        }
        for (s, min) in [(0, 1), (1, 1), (2, 1), (3, 1), (7, 1), (9, 0), (11, 0)] {
            let r = rec(records, s);
            r[5] |= 0x04;
            r[0x2C..0x30].copy_from_slice(&(min as u32).to_le_bytes());
        }
        for s in 0..16 {
            rec(records, s)[5] |= 0x10; // Saved
        }
        for s in [7, 9, 11] {
            rec(records, s)[5] |= 0x08; // fCallback
        }
        for s in [19, 21, 22] {
            rec(records, s)[4] |= 0x04; // damagerelated
        }
        for s in [8, 10] {
            rec(records, s)[0x50] = 1; // keepzero
        }
        let mut op = |s: usize, op: u8, param: u8, base: u16, stats: &[u16]| {
            let r = rec(records, s);
            r[0x54] = op;
            r[0x55] = param;
            set_u16(r, 0x56, base);
            for (j, &t) in stats.iter().enumerate() {
                set_u16(r, 0x58 + 2 * j, t);
            }
        };
        op(1, 8, 0, 0xFFFF, &[9]);
        op(3, 9, 0, 0xFFFF, &[7, 11]);
        op(75, 13, 0, 0xFFFF, &[73]);
        op(76, 11, 0, 0xFFFF, &[7]);
        op(77, 11, 0, 0xFFFF, &[9]);
        op(214, 4, 3, 12, &[31]);
        op(216, 2, 3, 12, &[7]);
        op(217, 2, 3, 12, &[9]);
        op(270, 6, 0, 0xFFFF, &[7]);
        op(269, 7, 0, 0xFFFF, &[9]);
    }
    edit(&mut records[..]);
    let mut t = BinTable {
        name: "itemstatcost".into(),
        source: "synthetic".into(),
        count: N,
        record_size: size,
        records,
    };
    stat_ops(&mut t);
    t
}

pub(crate) fn data() -> Arc<StatData> {
    Arc::new(StatData {
        stats: StatTable::from_fixed(&itemstatcost()).expect("itemstatcost"),
        classes: vec![ClassStats {
            mana_regen: 0,
            life_per_vitality: 16,
            stamina_per_vitality: 4,
            mana_per_magic: 8,
        }],
        states: StateTable::synthetic(
            185,
            &[
                (30, 0),
                (40, 16),
                (41, 16),
                (45, 12),
                (46, 12),
                (50, 32),
                (60, 14),
                (61, 15),
            ],
        ),
        damage_regen: vec![0, 8],
        aurastate: vec![0; 10],
        rescale_precision: DEFAULT_RESCALE_PRECISION,
    })
}

/// A host logging callbacks with the unit's base life at the time.
#[derive(Default)]
pub(crate) struct Log {
    pub callbacks: Vec<(u16, i32, i32)>,
    pub removed: Vec<(UnitId, u32)>,
    pub act_time: Option<i32>,
}

impl StatHost for Log {
    fn act_time(&self, _unit: UnitId) -> Option<i32> {
        self.act_time
    }
    fn on_callback(&mut self, _lists: &StatLists, ev: &CallbackEvent) {
        self.callbacks.push((key_stat(ev.key), ev.old, ev.new));
    }
    fn list_removed(
        &mut self,
        lists: &mut StatLists,
        unit: UnitId,
        state: u32,
        _list: ListId,
        _cb: lists::RemoveCallback,
    ) {
        lists.toggle_state(unit, state, false);
        self.removed.push((unit, state));
    }
}

pub(super) const P: UnitId = UnitId(1);
pub(super) const ITEM: UnitId = UnitId(2);

pub(super) fn full(lists: &StatLists, l: ListId) -> Vec<(u16, i32)> {
    lists
        .full_entries(l)
        .into_iter()
        .map(|(k, v)| (key_stat(k), v))
        .collect()
}

/// test vector step 1: the player.
pub(crate) fn player(lists: &mut StatLists, log: &mut Log) -> ListId {
    let l = lists.alloc_extended(
        log,
        P,
        crate::units::UnitType::Player,
        1,
        0,
        0,
        Some(ValueCallback::Server),
    );
    for (s, v) in [(0, 30), (12, 10), (3, 25), (7, 12800), (6, 12800)] {
        lists.set(log, l, s, v, 0, None);
    }
    l
}

pub(super) fn item_list(lists: &mut StatLists, log: &mut Log, stats: &[(u16, i32)]) -> ListId {
    let l = lists.alloc_extended(log, ITEM, crate::units::UnitType::Item, 7, 0, 0, None);
    for &(s, v) in stats {
        lists.set(log, l, s, v, 0, None);
    }
    l
}

#[test]
fn muldiv_vectors() {
    assert_eq!(muldiv(250, 40, 100), 100);
    assert_eq!(muldiv(-250, 40, 100), -100);
    assert_eq!(muldiv(7, 9, 0), 0);
    assert_eq!(muldiv(0x20_0000, 3, 0x10), 0x6_0000);
    assert_eq!(muldiv(0x20_0000, 0x100, 0x10_0000), 0x200);
    assert_eq!(muldiv(100, 0x2_0000, 0x1000), 0xC80);
    assert_eq!(muldiv(0x10_0000, 0x1_0000, 1), 0);
}

#[test]
fn by_time_vectors() {
    let v = (356 << 12) | (306 << 2);
    assert_eq!(by_time(v, 0), 100);
    assert_eq!(by_time(v, 180), 50);
    assert_eq!(by_time(v | 1, 0), 75);
    assert_eq!(act_base_time(1000, 0), 0);
    assert_eq!(act_base_time(1000, 10), 100);
}

#[test]
fn keys_order_stat_then_layer() {
    assert_eq!(key(7, 0), 0x0007_0000);
    assert!(key(7, 0xFFFF) < key(8, 0));
    assert_eq!((key_stat(key(216, 3)), key_layer(key(216, 3))), (216, 3));
}

#[test]
fn op_tables_from_the_fixup() {
    let d = data();
    let e7: Vec<(u16, u16, u8, u8)> = d
        .stats
        .get(7)
        .unwrap()
        .entries
        .iter()
        .map(|e| (e.base, e.source, e.op, e.param))
        .collect();
    assert_eq!(
        e7,
        [
            (0xFFFF, 3, 9, 0),
            (0xFFFF, 76, 11, 0),
            (12, 216, 2, 3),
            (0xFFFF, 270, 6, 0)
        ]
    );
    assert_eq!(d.stats.get(12).unwrap().deps, [214, 216, 217]);
    let a53: Vec<u16> = (0..N as u16)
        .filter(|&s| d.stats.get(s).unwrap().a53)
        .collect();
    assert_eq!(a53, [214]);
}

#[test]
fn life_fraction_and_clamp_rule() {
    assert_eq!(life_fraction(12800, 25600), 64);
    assert_eq!(life_fraction(25600, 25600), 128);
    assert_eq!(life_fraction(100, 0), 128);
    assert!(fraction_changed(70, 0x140));
    assert!(!fraction_changed(68, 0x140));
}

#[test]
fn x87_rescale_exact_cases() {
    // 23050 / 12800 = 1.80078125 exactly: no rounding anywhere.
    assert_eq!(x87_rescale(23050, 12800, 12800, 53), 23050);
    assert_eq!(x87_rescale(12800, 23050, 23050, 53), 12800);
    assert_eq!(x87_rescale(12800, 23050, 23050, 64), 12800);
    // 1/3 · 3 rounds back to 1 at every precision.
    for p in [24, 53, 64] {
        assert_eq!(x87_rescale(1, 3, 3, p), 1);
    }
    assert_eq!(x87_rescale(-512, 256, 100, 53), -200);
}

#[test]
fn x87_rescale_rounds_to_float32() {
    // 16777217 is not a float32: it rounds to 16777216 (even).
    assert_eq!(x87_rescale(16_777_217, 1, 1, 64), 16_777_216);
    // 16777219 rounds up to 16777220.
    assert_eq!(x87_rescale(16_777_219, 1, 1, 64), 16_777_220);
}

/// Test vectors, steps 1–4 and 6.
#[test]
fn stat_list_vectors() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    assert_eq!(
        full(&lists, p),
        [(0, 30), (3, 25), (6, 12800), (7, 12800), (12, 10)]
    );
    assert_eq!(log.callbacks, [(7, 0, 12800)]);
    let mods: Vec<u16> = lists.mods(p).into_iter().map(key_stat).collect();
    assert_eq!(mods, [0, 3, 7, 12]);

    log.callbacks.clear();
    let i = item_list(&mut lists, &mut log, &[(3, 10), (216, 8), (19, 5)]);
    lists.attach(&mut log, P, i, true);
    assert_eq!(log.callbacks, [(7, 12800, 23050), (11, 0, 2560)]);
    assert_eq!(lists.base(p, 6, 0), 23050);
    assert_eq!(
        full(&lists, p),
        [
            (0, 30),
            (3, 35),
            (6, 23050),
            (7, 23050),
            (11, 2560),
            (12, 10),
            (19, 5)
        ]
    );

    log.callbacks.clear();
    lists.detach(&mut log, i);
    assert_eq!(log.callbacks, [(7, 23050, 12800), (11, 2560, 0)]);
    assert_eq!(lists.base(p, 6, 0), 12800);
    assert_eq!(
        full(&lists, p),
        [(0, 30), (3, 25), (6, 12800), (7, 12800), (12, 10)]
    );

    let s = lists.alloc(0, 0, owner::PLAYER, 1);
    lists.set_state(s, 30);
    lists.set_expire(s, 20);
    assert_ne!(lists.flags(s) & flag::NEWLENGTH, 0);
    lists.set(&mut log, s, 0, 5, 0, None);
    lists.set_remove_callback(s, Some(lists::RemoveCallback(1)));
    lists.attach(&mut log, P, s, true);
    lists.toggle_state(P, 30, true);
    assert_eq!(lists.total(p, 0, 0), 35);
    assert!(lists.has_state(P, 30));

    lists.expire_lists(&mut log, P, 19).unwrap();
    assert!(lists.is_live(s));
    lists.expire_lists(&mut log, P, 20).unwrap();
    assert!(!lists.is_live(s));
    assert_eq!(lists.total(p, 0, 0), 30);
    assert_eq!(log.removed, [(P, 30)]);
    assert!(!lists.has_state(P, 30));
}

/// vector: maxhp and maxstamina of the player with the item.
#[test]
fn evaluation_vector() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    let i = item_list(&mut lists, &mut log, &[(3, 10), (216, 8)]);
    lists.attach(&mut log, P, i, true);
    assert_eq!(
        lists.eval(&log, p, key(7, 0)),
        12800 + ((16 * 10) << 6) + ((8 * 10) >> 3)
    );
    assert_eq!(lists.total(p, 11, 0), (4 * 10) << 6);
}

#[test]
fn minimum_rule() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    lists.set(&mut log, p, 7, 100, 0, None);
    assert_eq!(lists.total(p, 7, 0), 100);
    lists.set(&mut log, p, 7, -5, 0, None);
    assert_eq!(lists.total(p, 7, 0), 256);
    assert_eq!(lists.base(p, 7, 0), 256);
    // Absent: 0, no minimum rule.
    assert_eq!(lists.total(p, 1, 0), 0);
    // Item lists have no minimum rule.
    let i = item_list(&mut lists, &mut log, &[(7, -5)]);
    assert_eq!(lists.total(i, 7, 0), -5);
    // Invalid stats read 0 but are stored (edge case 1).
    assert!(lists.set(&mut log, p, 400, 9, 0, None));
    assert_eq!(lists.base(p, 400, 0), 0);
    assert!(lists.base_entries(p).contains(&(key(400, 0), 9)));
}

#[test]
fn base_writes() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    assert!(!lists.set(&mut log, p, 20, 0, 0, None));
    assert!(!lists.set(&mut log, p, 0, 30, 0, None));
    lists.add(&mut log, p, 20, 4, 1);
    lists.add(&mut log, p, 20, -4, 1);
    assert_eq!(lists.base(p, 20, 1), 0);
    assert!(!lists.base_entries(p).iter().any(|&(k, _)| k == key(20, 1)));
    // Mod array: 6 and non-Saved stats are excluded.
    let mods: Vec<u16> = lists.mods(p).into_iter().map(key_stat).collect();
    assert_eq!(mods, [0, 3, 7, 12]);
    let src = lists.alloc(0, 0, owner::PLAYER, 1);
    lists.set(&mut log, src, 0, 2, 0, None);
    lists.set(&mut log, src, 2, 7, 0, None);
    lists.merge(&mut log, p, src);
    assert_eq!((lists.base(p, 0, 0), lists.base(p, 2, 0)), (32, 7));
    assert_eq!(lists.mod_values(P)[0], (key(0, 0), 32));
    // (recorded on a town arrival): sent only
    // while the key is in the mod array.
    assert_eq!(lists.single_stat(P, 12), Some(10));
    lists.clear_mods(P);
    assert_eq!(lists.single_stat(P, 12), None);
    lists.remove_all(&mut log, src);
    assert!(lists.base_entries(src).is_empty());
}

#[test]
fn dynamic_lists_and_damage_related() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    let i = item_list(&mut lists, &mut log, &[(19, 5), (20, 3)]);
    lists.equip(&mut log, P, Some(i), false, false);
    assert_ne!(lists.flags(i) & flag::DYNAMIC, 0);
    assert_eq!((lists.total(p, 19, 0), lists.total(p, 20, 0)), (0, 3));
    lists.make_static(&mut log, P, i, false);
    assert_eq!(lists.total(p, 19, 0), 5);
    lists.make_dynamic(&mut log, P, i, false);
    assert_eq!(lists.total(p, 19, 0), 0);
    // Swap location: detached.
    lists.equip(&mut log, P, Some(i), true, true);
    assert_eq!(lists.total(p, 20, 0), 0);
    assert_eq!(lists.parent(i), None);

    // Edge case 3: a plain DYNAMIC child's damage-related change still
    // reaches its parent.
    let c = lists.alloc(0, 0, owner::PLAYER, 1);
    lists.attach(&mut log, P, c, false);
    lists.set(&mut log, c, 19, 4, 0, None);
    assert_eq!(lists.total(p, 19, 0), 4);
    // ... but the sum skips it.
    assert_eq!(lists.eval(&log, p, key(19, 0)), 0);
}

#[test]
fn parked_lists_count_nowhere() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    let s = lists.alloc(0, 0, owner::PLAYER, 1);
    lists.set_state(s, 30);
    lists.set(&mut log, s, 0, 5, 0, None);
    lists.attach(&mut log, P, s, true);
    assert_eq!(lists.total(p, 0, 0), 35);
    assert!(lists.park(&mut log, P, 30, true));
    assert!(!lists.park(&mut log, P, 30, true));
    assert_eq!(lists.total(p, 0, 0), 30);
    assert_eq!(lists.parked_chain(p), [s]);
    assert!(lists.active_chain(p).is_empty());
    // A parked list ignores base writes.
    lists.set(&mut log, s, 0, 9, 0, None);
    assert_eq!(lists.total(p, 0, 0), 30);
    assert!(lists.park(&mut log, P, 30, false));
    assert_eq!(lists.total(p, 0, 0), 39);
    assert_eq!(lists.list_of_state(p, 30), Some(s));
}

#[test]
fn chains_link_newest_first() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    let a = lists.alloc(0, 0, owner::PLAYER, 1);
    let b = lists.alloc(0, 0, owner::PLAYER, 1);
    let c = lists.alloc(0, 0, owner::PLAYER, 1);
    for l in [a, b, c] {
        lists.attach(&mut log, P, l, true);
    }
    assert_eq!(lists.active_chain(p), [c, b, a]);
    assert_eq!((lists.prev(b), lists.next(b)), (Some(a), Some(c)));
    lists.detach(&mut log, b);
    assert_eq!(lists.active_chain(p), [c, a]);
    assert_eq!((lists.prev(c), lists.next(a)), (Some(a), Some(c)));
    // Attaching a list to itself or an ancestor stops.
    lists.attach(&mut log, P, p, true);
    assert_eq!(lists.parent(p), None);
    // Free of an extended list frees its plain children, keeps extended.
    let i = item_list(&mut lists, &mut log, &[(20, 1)]);
    lists.attach(&mut log, P, i, true);
    lists.free(&mut log, p);
    assert!(!lists.is_live(a) && !lists.is_live(c) && !lists.is_live(p));
    assert!(lists.is_live(i));
    assert_eq!((lists.parent(i), lists.attached_unit(i)), (None, None));
    assert_eq!(lists.unit_list(P), None);
}

#[test]
fn temponly_sets_newlength_on_the_unit_list() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    let t = lists.alloc(flag::TEMPONLY, 0, owner::PLAYER, 1);
    lists.attach(&mut log, P, t, true);
    assert_ne!(lists.flags(p) & flag::NEWLENGTH, 0);
}

#[test]
fn per_level_of_the_wearer() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    let i = item_list(&mut lists, &mut log, &[(214, 8)]);
    assert_ne!(lists.flags(i) & flag::PERMANENT, 0);
    assert_eq!(lists.total(i, 31, 0), 0);
    lists.attach(&mut log, P, i, true);
    assert_eq!(lists.total(i, 31, 0), (8 * 10) >> 3);
    assert_eq!(lists.total(p, 31, 0), 10);
    lists.detach(&mut log, i);
    assert_eq!(lists.total(i, 31, 0), 0);
    assert_eq!(lists.total(p, 31, 0), 0);
}

#[test]
fn energy_and_percent_ops() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    lists.set(&mut log, p, 9, 5000, 0, None);
    let i = item_list(&mut lists, &mut log, &[(1, 4), (77, 50)]);
    lists.attach(&mut log, P, i, true);
    // 5000 + (8·4 << 6) = 7048, then +50 % of prev (5000) = 2500.
    assert_eq!(lists.total(p, 9, 0), 7048 + 2500);
}

#[test]
fn item_own_base_percent() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let i = item_list(&mut lists, &mut log, &[(73, 40), (75, 50)]);
    assert_eq!(lists.total(i, 73, 0), 60);
}

#[test]
fn by_time_ops_need_an_act() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    let v = (356 << 12) | (306 << 2);
    let i = item_list(&mut lists, &mut log, &[(270, v)]);
    lists.attach(&mut log, P, i, true);
    assert_eq!(lists.total(p, 7, 0), 12800);
    log.act_time = Some(0);
    lists.by_time_refresh(&mut log, P, i);
    assert_eq!(lists.total(p, 7, 0), 12900);
    log.act_time = Some(180);
    lists.by_time_refresh(&mut log, P, i);
    assert_eq!(lists.total(p, 7, 0), 12850);
}

#[test]
fn death_frees_plain_lists_except_basic() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    let a = lists.alloc(0, 0, owner::PLAYER, 1);
    let b = lists.alloc(flag::BASIC, 0, owner::PLAYER, 1);
    let c = lists.alloc(0, 0, owner::ITEM, 9);
    for l in [a, b, c] {
        lists.set(&mut log, l, 0, 1, 0, None);
        lists.attach(&mut log, P, l, true);
    }
    assert_eq!(lists.total(p, 0, 0), 33);
    lists.death(&mut log, P);
    assert!(!lists.is_live(a) && lists.is_live(b) && lists.is_live(c));
    assert_eq!(lists.total(p, 0, 0), 32);
}

#[test]
fn overlay_removal() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    let o = lists.alloc(flag::OVERLAY, 0, owner::PLAYER, 1);
    lists.set(&mut log, o, 0, 3, 0, None);
    lists.attach(&mut log, P, o, true);
    lists.set_flags(p, flag::REMOVE_OVERLAY, true);
    lists.remove_overlay(&mut log, P);
    assert_eq!(lists.flags(p) & flag::REMOVE_OVERLAY, 0);
    assert_eq!(lists.total(p, 0, 0), 30);
    assert!(lists.is_live(o));
}

#[test]
fn overlay_to_send_is_list_total() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    assert_eq!(lists.overlay_to_send(P), None);
    let o = lists.alloc(flag::OVERLAY, 0, owner::PLAYER, 1);
    lists.set(&mut log, o, 178, 7, 0, None);
    lists.attach(&mut log, P, o, true);
    assert_eq!(lists.overlay_to_send(P), None);
    lists.set_flags(p, flag::REMOVE_OVERLAY, true);
    assert_eq!(lists.overlay_to_send(P), Some(7));
    assert_eq!(lists.overlay_to_send(P), Some(lists.total(o, 178, 0)));
}

#[test]
fn state_toggles_and_groups() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    player(&mut lists, &mut log);
    let t = lists.toggle_state(P, 40, true);
    assert_eq!((t.changed, t.disguise), (true, Some(true)));
    assert_eq!(lists.toggle_state(P, 40, true), Default::default());
    lists.toggle_state(P, 41, true);
    let t = lists.toggle_state(P, 40, false);
    assert_eq!(t.disguise, Some(true));
    let t = lists.toggle_state(P, 41, false);
    assert_eq!(t.disguise, Some(false));
    let t = lists.toggle_state(P, 30, true);
    assert_eq!(t.disguise, None);
    let (on, changed) = lists.state_bits(P).unwrap();
    assert_eq!(on[0], 1 << 30);
    assert_eq!(changed[1], 0b11 << 8);
    assert!(!lists.has_group(P, 32));
    lists.toggle_state(P, 50, true);
    assert!(lists.has_group(P, 32));
    assert!(!lists.has_state(P, 999));
    assert!(!lists.has_state(UnitId(77), state::POISON));
}

#[test]
fn clamp_current_to_max() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    lists.set(&mut log, p, 6, 20000, 0, None);
    lists.set(&mut log, p, 8, 300, 0, None);
    lists.clamp_to_max(&mut log, P);
    assert_eq!(lists.total(p, 6, 0), 12800);
    assert_eq!(lists.total(p, 8, 0), 0);
    // keepzero: mana 0 stays in the full array.
    assert!(lists.full_entries(p).contains(&(key(8, 0), 0)));
}

#[test]
fn rescale_callback_on_max_life() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    lists.set(&mut log, p, 6, 6400, 0, None);
    lists.set(&mut log, p, 7, 25600, 0, None);
    // 25600 / 12800 · 6400 = 12800.
    assert_eq!(lists.total(p, 6, 0), 12800);
    // old ≤ 256 counts as 256.
    lists.set(&mut log, p, 9, 100, 0, None);
    lists.set(&mut log, p, 8, 50, 0, None);
    lists.set(&mut log, p, 9, 1000, 0, None);
    assert_eq!(lists.total(p, 8, 0), 195);
}

#[test]
fn monster_damage_regen_from_max_life() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let m = UnitId(5);
    let l = lists.alloc_extended(
        &mut log,
        m,
        crate::units::UnitType::Monster,
        3,
        1,
        0,
        Some(ValueCallback::Server),
    );
    lists.set(&mut log, l, 7, 25600, 0, None);
    assert_eq!(lists.total(l, 74, 0), (100 * 8) >> 4);
    let _ = NoHost;
}

#[test]
fn allocation_flags_and_freed_handle() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    // Callers pass flags 0: the extended list keeps only EXTENDED.
    assert_eq!(lists.flags(p), flag::EXTENDED);
    assert_eq!(
        (lists.owner_type(p), lists.owner_guid(p)),
        (owner::PLAYER, 1)
    );
    assert_eq!(lists.owner(p), Some(P));
    // A flags value keeps only bit 0x1 of the allocation flags.
    let it = lists.alloc_extended(
        &mut log,
        ITEM,
        crate::units::UnitType::Item,
        7,
        0,
        0xFFFF_FFFF,
        None,
    );
    assert_eq!(lists.flags(it), flag::BASIC | flag::EXTENDED);
    // A freed handle behaves as the null list: reads 0, writes do nothing.
    let plain = lists.alloc(0, 0, owner::PLAYER, 1);
    lists.set(&mut log, plain, 0, 5, 0, None);
    lists.free(&mut log, plain);
    assert!(!lists.is_live(plain));
    assert_eq!(lists.base(plain, 0, 0), 0);
    assert!(!lists.set(&mut log, plain, 0, 9, 0, None));
    lists.attach(&mut log, P, plain, true);
    assert_eq!(lists.total(p, 0, 0), 30);
    lists.detach(&mut log, plain);
    lists.free(&mut log, plain);
}

#[test]
fn recompute_per_level_stat_leaves_own_entry_stale() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    // 216 (op 2, entry base 0 = strength 30 > 0) is blocked: a change of
    // the per-level stat itself leaves its full entry unwritten ...
    lists.set(&mut log, p, 216, 8, 0, None);
    assert!(!full(&lists, p).iter().any(|&(s, _)| s == 216));
    // ... while the stat it targets (7) is still recomputed from it.
    let item = item_list(&mut lists, &mut log, &[(216, 8)]);
    lists.attach(&mut log, P, item, true);
    assert!(!full(&lists, p).iter().any(|&(s, _)| s == 216));
    assert_eq!(lists.base(p, 6, 0), lists.total(p, 7, 0));
}

#[test]
fn state_queries() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let _p = player(&mut lists, &mut log);
    // Has state: type check, range check, bit.
    lists.toggle_state(P, 40, true);
    assert!(lists.has_state(P, 40));
    assert!(!lists.has_state(P, 41));
    assert!(!lists.has_state(P, 185));
    assert!(!lists.has_state(P, 100_000));
    lists.toggle_state(P, 30, true);
    // Flag group 16 holds states 40 and 41, group 32 state 50.
    assert!(lists.has_group(P, 16));
    assert!(!lists.has_group(P, 32));
    lists.toggle_state(P, 50, true);
    assert!(lists.has_group(P, 32));
    assert!(!lists.has_group(P, 40));
    assert!(!lists.has_group(ITEM, 16));
    // Non player/monster/3 unit types never have a state.
    item_list(&mut lists, &mut log, &[]);
    lists.toggle_state(ITEM, 40, true);
    assert!(!lists.has_state(ITEM, 40));

    // List of a state / by state and flags.
    let r = lists.unit_list(P).unwrap();
    let a = lists.alloc(0x20, 0, owner::PLAYER, 1);
    let b = lists.alloc(0x40, 0, owner::PLAYER, 1);
    let c = lists.alloc(0x40 | flag::SET, 0, owner::PLAYER, 1);
    for (l, s) in [(a, 30), (b, 30), (c, 41)] {
        lists.set_state(l, s);
        lists.attach(&mut log, P, l, true);
    }
    // Active chain is newest first: b before a.
    assert_eq!(lists.list_of_state(r, 30), Some(b));
    // The parked list is found when the active chain has none.
    assert_eq!(lists.list_of_state(r, 41), Some(c));
    assert_eq!(lists.list_of_state(r, 42), None);
    // f = 0 matches on the state alone.
    assert_eq!(lists.list_by_state_flags(P, 30, 0), Some(b));
    // Any shared bit, not every bit.
    assert_eq!(lists.list_by_state_flags(P, 30, 0x20 | 0x1000), Some(a));
    assert_eq!(lists.list_by_state_flags(P, 30, 0x1000), None);
    // The SET bit selects the parked chain and is dropped from f.
    assert_eq!(lists.list_by_state_flags(P, 41, flag::SET), Some(c));
    assert_eq!(lists.list_by_state_flags(P, 41, flag::SET | 0x40), Some(c));
    assert_eq!(lists.list_by_state_flags(P, 41, 0), None);
    // A unit without a list, or a non-extended one, yields none.
    assert_eq!(lists.list_by_state_flags(UnitId(99), 30, 0), None);
}

#[test]
fn mode_change_frees_temp_only_lists() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    let t = lists.alloc(flag::TEMPONLY, 0, owner::PLAYER, 1);
    lists.set(&mut log, t, 0, 5, 0, None);
    lists.set_state(t, 30);
    lists.attach(&mut log, P, t, true);
    lists.toggle_state(P, 30, true);
    let keep = lists.alloc(0, 0, owner::PLAYER, 1);
    lists.set(&mut log, keep, 0, 2, 0, None);
    lists.attach(&mut log, P, keep, true);
    let t2 = lists.alloc(flag::TEMPONLY, 0, owner::PLAYER, 1);
    lists.set(&mut log, t2, 0, 1, 0, None);
    lists.attach(&mut log, P, t2, true);
    assert_ne!(lists.flags(p) & flag::NEWLENGTH, 0);
    assert_eq!(lists.total(p, 0, 0), 38);
    lists.remove_temporary_lists(&mut log, P);
    // Both TEMPONLY lists are gone (state bit off for the first), the
    // other list stays, NEWLENGTH is cleared.
    assert!(!lists.is_live(t) && !lists.is_live(t2));
    assert!(lists.is_live(keep));
    assert!(!lists.has_state(P, 30));
    assert_eq!(lists.total(p, 0, 0), 32);
    assert_eq!(lists.flags(p) & flag::NEWLENGTH, 0);
    // Without NEWLENGTH nothing is touched.
    let t3 = lists.alloc(flag::TEMPONLY, 0, owner::PLAYER, 1);
    lists.attach(&mut log, P, t3, true);
    lists.set_flags(p, flag::NEWLENGTH, false);
    lists.remove_temporary_lists(&mut log, P);
    assert!(lists.is_live(t3));
}

#[test]
fn attach_collects_at_most_16_a53_keys() {
    let mut log = Log::default();
    let t = itemstatcost_with(|rs| {
        for s in 100..120 {
            rs[s * Itemstatcost::SIZE + 0x53] = 1;
        }
    });
    let mut d = (*data()).clone();
    d.stats = StatTable::from_fixed(&t).expect("itemstatcost");
    let mut lists = StatLists::new(Arc::new(d));
    let _p = player(&mut lists, &mut log);
    let i = item_list(&mut lists, &mut log, &[]);
    for s in 100..120 {
        lists.set(&mut log, i, s, 1, 0, None);
    }
    assert_eq!(lists.a53_keys(i).len(), 16);
}

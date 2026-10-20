//! Tests for the item copy (`copy.rs`) on the real item: the save stream
//! with children, the first record read back into a new item unit, the
//! flags, the source's mark and the per-item reset.

use super::*;
use crate::items::bitstream::Isc;
use crate::items::{flag, stat, ListKey};
use crate::wiring::inventory::copy::COPIED;
use crate::wiring::inventory::InvError;

/// Save columns for the stats the tests write (`Save Bits` > 0).
fn isc() -> Vec<Isc> {
    let mut t = vec![Isc::default(); N_STATS];
    let bits = |b: u8, add: u32| Isc {
        valshift: 0,
        save_bits: b,
        save_add: add,
        save_param_bits: 0,
    };
    t[19] = bits(10, 0);
    t[usize::from(stat::ARMORCLASS)] = bits(11, 10);
    t[usize::from(stat::DURABILITY)] = bits(9, 0);
    t[usize::from(stat::MAXDURABILITY)] = bits(8, 0);
    t[usize::from(stat::NUMSOCKETS)] = bits(4, 0);
    t
}

fn world() -> World {
    let mut w = World::new();
    w.tables.isc = isc();
    w
}

/// Copying a held cap (the cursor item) with defense,
/// durability and a main list: a new unit (the next GUID) of the same
/// class in no room, in the source's mode and position, the record's
/// fields and stats, the init seed,
/// item flag 0x80000 set and 0x2000 cleared, the source marked
/// 0x8000000, the per-item reset (item flags 0x1 … cleared).
#[test]
fn copy_of_a_held_item() {
    let mut w = world();
    let s = w.cursor_item(CAP);
    let su = w.unit(s).unwrap();
    w.set_stat(su, stat::ARMORCLASS, 7);
    w.set_stat(su, stat::MAXDURABILITY, 12);
    w.set_stat(su, stat::DURABILITY, 9);
    {
        let it = w.items.get_mut(su).unwrap();
        it.ilvl = 17;
        it.flags |= flag::INSTORE | 0x1;
    }
    w.econ().with_stats(|ctx| {
        let mut st = crate::wiring::economy::UnitStats::new(ctx, su);
        crate::items::ItemStats::list_set(&mut st, ListKey::ITEM, 19, 0, 33);
    });
    let init = w.units.get(su).unwrap().init_seed;
    let c = w.desk(|d| d.copy_of(su, true)).expect("copied");
    assert_ne!(c, su);
    let cg = w.units.get(c).unwrap().guid;
    assert_eq!(cg, s + 1, "the next item GUID");
    // The save's 32 bits become the copy's init seed, and `init_low` of
    // it the unit seed; the item seed is the allocation's.
    let cr = w.units.get(c).unwrap();
    assert_eq!(cr.init_seed, init);
    assert_eq!(cr.seed, crate::rng::Seed::init_low(init));
    assert_ne!(cr.item_seed.unwrap().0, crate::rng::Seed::init_low(init));
    assert_eq!(w.units.get(c).unwrap().mode, w.units.get(su).unwrap().mode);
    assert_eq!(w.game.lists.unit(c).unwrap().room(), None);
    assert_eq!((w.data(cg).x, w.data(cg).y), (w.data(s).x, w.data(s).y));
    let (a, b) = (w.items.get(su).unwrap(), w.items.get(c).unwrap());
    assert_eq!((b.record, b.ilvl, b.quality), (a.record, 17, a.quality));
    assert_ne!(b.flags & flag::INIT, 0);
    assert_eq!(b.flags & (flag::INSTORE | 0x1), 0, "0x2000 and the reset");
    assert_ne!(a.flags & COPIED, 0);
    for s_ in [stat::ARMORCLASS, stat::MAXDURABILITY, stat::DURABILITY] {
        assert_eq!(
            w.stats.unit_total(c, s_, 0),
            w.stats.unit_total(su, s_, 0),
            "stat {s_}"
        );
    }
    let list = w.econ().with_stats(|ctx| {
        let st = crate::wiring::economy::UnitStats::new(ctx, c);
        crate::items::ItemStats::list_get(&st, ListKey::ITEM, 19, 0)
    });
    assert_eq!(list, 33);
    assert!(w.state.errors.is_empty());
}

/// A source on the ground (in a room) is a caller error: no
/// copy, no new unit, the source unmarked, the error recorded.
#[test]
fn a_ground_source_is_a_caller_error() {
    let mut w = world();
    let s = w.ground_item(CAP, 20, 21);
    let su = w.unit(s).unwrap();
    let before = w.items.len();
    assert_eq!(w.desk(|d| d.copy_of(su, true)), None);
    assert_eq!(w.items.len(), before);
    assert_eq!(w.items.get(su).unwrap().flags & COPIED, 0);
    assert_eq!(w.state.errors, [InvError::GroundCopySource(su)]);
}

/// A stream that does not fit 1,024 bytes gives length 0 and
/// the read fails: no copy.
#[test]
fn a_stream_over_the_buffer_copies_nothing() {
    let mut w = world();
    let s = w.cursor_item(CAP);
    let su = w.unit(s).unwrap();
    w.econ().with_stats(|ctx| {
        let mut st = crate::wiring::economy::UnitStats::new(ctx, su);
        for p in 0..500u16 {
            crate::items::ItemStats::list_set(&mut st, ListKey::ITEM, 19, p, 1);
        }
    });
    let before = w.items.len();
    assert_eq!(w.desk(|d| d.copy_of(su, true)), None);
    assert_eq!(w.items.len(), before);
}

/// With fillers 0 the children are not read: the copy keeps the
/// socketed flag and its socket count, without fillers; with fillers 1
/// each child is read, allocated (two more game-seed steps) and linked
/// into the copy in mode 6 (the arguments of that child read are a
/// provisional reading).
#[test]
fn children_and_the_fillers_argument() {
    let mut w = world();
    super::load::allow_sockets(&mut w.tables, SWORD, 2);
    let s = w.cursor_item(SWORD);
    let su = w.unit(s).unwrap();
    w.items.get_mut(su).unwrap().flags |= 0x800;
    w.set_stat(su, stat::NUMSOCKETS, 2);
    w.state.add_inventory(su, UnitKind::Item, s);
    let k = w.ground_item(KEY, 22, 21);
    let ku = w.unit(k).unwrap();
    w.desk(|d| {
        let mut inv = d.state.inventories.remove(&su).unwrap();
        inv.link(d, ku, None);
        d.state.inventories.insert(su, inv);
    });
    // The stream's filled count is the inventory's only with `hasinv`.
    w.tables.items[SWORD].hasinv = 1;
    let c = w.desk(|d| d.copy_of(su, false)).expect("copied");
    assert_ne!(w.items.get(c).unwrap().flags & 0x800, 0);
    assert_eq!(w.stats.unit_total(c, stat::NUMSOCKETS, 0), 2);
    assert!(!w.state.inventories.contains_key(&c), "no fillers");
    let seed = w.desk(|d| d.econ.fields.seed);
    let c2 = w
        .desk(|d| d.copy_of(su, true))
        .expect("copied with fillers");
    let mut steps = seed;
    for _ in 0..4 {
        steps.step();
    }
    assert_eq!(w.desk(|d| d.econ.fields.seed), steps, "2 · (1 + 1) steps");
    let kids = w
        .state
        .inventories
        .get(&c2)
        .expect("fillers")
        .items()
        .to_vec();
    assert_eq!(kids.len(), 1);
    assert_ne!(kids[0], ku);
    assert_eq!(w.state.items.get(&kids[0]).unwrap().mode, 6);
    assert_eq!(w.state.errors, Vec::new());
}

/// A copy whose total of stat 252 is 10 holds exactly one event 3
/// at frame + 2500 / 10 + 1 = frame + 251.
#[test]
fn copy_schedules_one_replenish_event() {
    let mut w = world();
    w.tables.isc[usize::from(stat::REPLENISH_DURABILITY)] = Isc {
        valshift: 0,
        save_bits: 6,
        save_add: 0,
        save_param_bits: 0,
    };
    w.game.frame = 1000;
    let s = w.cursor_item(CAP);
    let su = w.unit(s).unwrap();
    w.econ().with_stats(|ctx| {
        let mut st = crate::wiring::economy::UnitStats::new(ctx, su);
        crate::items::ItemStats::list_set(
            &mut st,
            ListKey::ITEM,
            stat::REPLENISH_DURABILITY,
            0,
            10,
        );
    });
    let c = w.desk(|d| d.copy_of(su, true)).expect("copied");
    assert_eq!(w.stats.unit_total(c, stat::REPLENISH_DURABILITY, 0), 10);
    let due: Vec<_> = w
        .game
        .timers
        .unit_timers(c)
        .into_iter()
        .filter(|&id| w.game.timers.event(id).is_some_and(|(e, _, _)| e == 3))
        .filter_map(|id| w.game.timers.expire(id))
        .collect();
    assert_eq!(due, [1251]);
}

fn bits_of(b: u8) -> Isc {
    Isc {
        valshift: 0,
        save_bits: b,
        save_add: 0,
        save_param_bits: 0,
    }
}

/// The copy of a weapon gets its base speed and damage
/// from the items columns (stat 68 := −speed, 22, 21, 24, 23 := maxdam,
/// mindam, 2handmaxdam, 2handmindam), ⌊3v / 4⌋ with the floors for
/// quality 1, × 3 / 2 for item flag 0x400000.
#[test]
fn copy_rebuilds_weapon_base_damage() {
    let mut w = world();
    let r = &mut w.tables.items[SWORD];
    (r.speed, r.mindam, r.maxdam, r.mindam2, r.maxdam2) = (10, 3, 7, 5, 9);
    let base = |w: &World, c| [68u16, 21, 22, 23, 24].map(|s| w.stats.unit_base(c, s, 0));
    let s = w.cursor_item(SWORD);
    let su = w.unit(s).unwrap();
    let c = w.desk(|d| d.copy_of(su, true)).expect("copied");
    assert_eq!(base(&w, c), [-10, 3, 7, 5, 9]);
    // Ethereal source: × 3 / 2 (signed), speed untouched.
    w.items.get_mut(su).unwrap().flags |= flag::ETHEREAL;
    let c = w.desk(|d| d.copy_of(su, true)).expect("copied");
    assert_eq!(base(&w, c), [-10, 4, 10, 7, 13]);
    // Quality 1 (low): ⌊3v / 4⌋, maxima at least 2, minima at least 1.
    w.items.get_mut(su).unwrap().flags &= !flag::ETHEREAL;
    w.items.get_mut(su).unwrap().quality = 1;
    let c = w.desk(|d| d.copy_of(su, true)).expect("copied");
    assert_eq!(base(&w, c), [-10, 2, 5, 3, 6]);
    // Floors: a column of 1 → ⌊3 / 4⌋ = 0 → 1 (min), 2 (max).
    let r = &mut w.tables.items[SWORD];
    (r.mindam, r.maxdam, r.mindam2, r.maxdam2) = (1, 1, 0, 0);
    let c = w.desk(|d| d.copy_of(su, true)).expect("copied");
    assert_eq!(base(&w, c), [-10, 1, 2, 1, 2]);
}

/// An armor copy gets stat 20 := block and 67 := −speed.
#[test]
fn copy_rebuilds_armor_block_and_speed() {
    let mut w = world();
    let r = &mut w.tables.items[CAP];
    (r.block, r.speed) = (20, 5);
    let s = w.cursor_item(CAP);
    let su = w.unit(s).unwrap();
    let c = w.desk(|d| d.copy_of(su, true)).expect("copied");
    assert_eq!(w.stats.unit_base(c, 20, 0), 20);
    assert_eq!(w.stats.unit_base(c, 67, 0), -5);
}

/// The stat lists: an entry for stat 17 raises the base
/// maxima, its partner 18 the base minima, to the items column when
/// below it (a low-quality weapon's ¾ values come back to the full
/// column); an entry for stat 57 sets stat 326 := 1 in that list.
#[test]
fn copy_list_entries_raise_base_damage_and_set_poison_count() {
    let mut w = world();
    for s in [17usize, 18, 57, 58, 59] {
        w.tables.isc[s] = bits_of(9);
    }
    let r = &mut w.tables.items[SWORD];
    (r.speed, r.mindam, r.maxdam, r.mindam2, r.maxdam2) = (10, 3, 7, 5, 9);
    let s = w.cursor_item(SWORD);
    let su = w.unit(s).unwrap();
    w.items.get_mut(su).unwrap().quality = 1;
    let plain = w.desk(|d| d.copy_of(su, true)).expect("copied");
    assert_eq!(w.stats.unit_base(plain, 22, 0), 5);
    w.econ().with_stats(|ctx| {
        let mut st = crate::wiring::economy::UnitStats::new(ctx, su);
        for (id, v) in [(17u16, 40), (18, 30), (57, 3), (58, 5), (59, 2)] {
            crate::items::ItemStats::list_set(&mut st, ListKey::ITEM, id, 0, v);
        }
    });
    let c = w.desk(|d| d.copy_of(su, true)).expect("copied");
    // Full columns: 21, 22, 23, 24 = 3, 7, 5, 9.
    assert_eq!(
        [21u16, 22, 23, 24].map(|s| w.stats.unit_base(c, s, 0)),
        [3, 7, 5, 9]
    );
    let get = |w: &mut World, id| {
        w.econ().with_stats(|ctx| {
            let st = crate::wiring::economy::UnitStats::new(ctx, c);
            crate::items::ItemStats::list_get(&st, ListKey::ITEM, id, 0)
        })
    };
    assert_eq!(get(&mut w, 17), 40);
    assert_eq!(get(&mut w, 326), 1, "poison_count := 1 beside stat 57");
}

/// The copy's save stream changes the source and its children like any
/// save (item level < 1 → 1, quality outside 1–9 → 2).
#[test]
fn the_copy_stream_writes_back_to_the_source_and_its_children() {
    let mut w = world();
    let s = w.cursor_item(SWORD);
    let su = w.unit(s).unwrap();
    w.items.get_mut(su).unwrap().ilvl = 0;
    w.state.add_inventory(su, UnitKind::Item, s);
    let k = w.ground_item(KEY, 22, 21);
    let ku = w.unit(k).unwrap();
    w.desk(|d| {
        let mut inv = d.state.inventories.remove(&su).unwrap();
        inv.link(d, ku, None);
        d.state.inventories.insert(su, inv);
    });
    w.items.get_mut(ku).unwrap().quality = 0;
    w.items.get_mut(ku).unwrap().ilvl = 4;
    w.desk(|d| d.copy_of(su, false)).expect("copied");
    let (a, b) = (w.items.get(su).unwrap(), w.items.get(ku).unwrap());
    assert_eq!((a.ilvl, b.ilvl, b.quality), (1, 4, 2));
}

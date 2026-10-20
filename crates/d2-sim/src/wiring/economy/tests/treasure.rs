//! Treasure → items: a treasure-class walk whose drops are real item units.
//! The walk is `crate::treasure`'s; item creation and the drop sink are the
//! economy wiring's.

use super::*;
use crate::items::q;
use crate::treasure::runtime::{TreasureClass, TreasureClasses, FLAG_UNIQUE};
use crate::treasure::{walk, DropSink, DropperKind, ItemData, TcEntry, TreasureData, WalkArgs};
use crate::wiring::economy::{dropper, find_list, recipient, DropPlacer, DropSpot, ItemDrops};

/// Places every drop at the dropper's position in no room.
struct Here;

impl<H> DropPlacer<H> for Here {
    fn place(&mut self, _: &mut Economy<'_, H>, x: i32, y: i32) -> Option<DropSpot> {
        Some(DropSpot { room: None, x, y })
    }
}

/// The treasure item list of the synthetic items (same index order).
fn treasure_items(t: &ItemTables) -> Vec<ItemData> {
    t.items
        .iter()
        .map(|r| ItemData {
            code: r.code,
            ubercode: r.ubercode,
            ultracode: r.ultracode,
            version: r.version,
            level: r.level,
            type_: r.type_ as u16,
            type2: r.type2 as u16,
            unique: r.unique,
            quest: r.quest,
            spawnable: 1,
        })
        .collect()
}

/// TC 0 is the empty filler; TC 1 has `entries` (item id, prob, flags,
/// row), picks `picks`, no NoDrop.
fn classes(picks: i32, entries: &[(u16, i32, u8, u16)]) -> TreasureClasses {
    let empty = |name: &[u8]| TreasureClass {
        name: name.to_vec(),
        group: 0,
        level: 0,
        total_classic: 0,
        total_expansion: 0,
        picks: 1,
        nodrop: 0,
        mods: [0; 6],
        entries: Vec::new(),
    };
    let mut tc = empty(b"t");
    tc.picks = picks;
    for &(id, p, flags, row) in entries {
        tc.entries.push(TcEntry {
            start_classic: tc.total_classic,
            start_expansion: tc.total_expansion,
            id,
            row,
            flags,
            mods: [0; 6],
        });
        tc.total_expansion += p;
        tc.total_classic += p;
    }
    TreasureClasses {
        tcs: vec![empty(b"none"), tc],
        group_offset: 0,
        chest: [None; 45],
        notes: Vec::new(),
    }
}

fn args(quality: u8) -> WalkArgs {
    WalkArgs {
        tc: Some(1),
        quality,
        level: 0,
        find_item: false,
        list: true,
        max: 6,
    }
}

/// A monster's walk picks the cap with forced
/// quality 7; the drop request becomes a real unique item whose
/// property sits in its stat list; the item level is the monster's
/// `level` stat, the spawn mode 3.
#[test]
fn monster_walk_creates_a_real_unique() {
    let mut w = World::new();
    let monster = w.spawn(UnitType::Monster, MONSTER_CLASS);
    w.set_stat(monster, 12, 30);
    w.set_stat(monster, 100, 1);
    let tcs = classes(1, &[(CAP as u16, 1, 0, 0)]);
    let items = treasure_items(&w.tables);
    let (types, equiv, ratio) = (
        w.tables.itemtypes.clone(),
        w.tables.equiv.clone(),
        w.tables.itemratio.clone(),
    );
    let data = TreasureData {
        tcs: &tcs,
        items: &items,
        itemtypes: &types,
        equiv: &equiv,
        itemratio: &ratio,
    };
    let facts = w.fields.treasure_facts(1, 0);
    let d = dropper(&w.units, &w.stats, Some(monster), 0, 7, 9);
    assert_eq!(
        d.kind,
        DropperKind::Monster {
            class: MONSTER_CLASS,
            level: 30,
            playercount: 1
        }
    );
    let mut seed = w.units.get(monster).unwrap().seed;
    let mut e = w.econ();
    let mut sink = ItemDrops::new(&mut e, Here);
    let out = walk(
        &data,
        &facts,
        &d,
        &mut seed,
        None,
        &args(q::UNIQUE),
        &mut sink,
    )
    .unwrap();
    assert!(sink.failures.is_empty(), "{:?}", sink.failures);
    assert_eq!(out.len(), 1);
    assert_eq!(
        sink.placed,
        [(
            out[0],
            DropSpot {
                room: None,
                x: 7,
                y: 9
            }
        )]
    );
    drop(sink);
    let u = out[0];
    let it = w.items.get(u).unwrap();
    assert_eq!(
        (it.record, it.quality, it.ilvl, it.file_index),
        (CAP, q::UNIQUE, 30, 0)
    );
    assert_eq!(w.units.get(u).unwrap().mode, 3);
    let l = find_list(&w.stats, u, ListKey::ITEM).unwrap();
    let v = w.stats.base(l, PROP_STAT, 0);
    assert!((10..=20).contains(&v));
    assert_eq!(w.stats.unit_total(u, PROP_STAT, 0), v);
    assert!(w.fields.uniques.get(0));
}

/// A unique entry (quality 7, index row + 1) reaches the unique
/// pick as the preferred record.
#[test]
fn unique_entry_prefers_its_row() {
    let mut w = World::new();
    let tcs = classes(1, &[(CAP as u16, 1, FLAG_UNIQUE, 0)]);
    let items = treasure_items(&w.tables);
    let (types, equiv, ratio) = (
        w.tables.itemtypes.clone(),
        w.tables.equiv.clone(),
        w.tables.itemratio.clone(),
    );
    let data = TreasureData {
        tcs: &tcs,
        items: &items,
        itemtypes: &types,
        equiv: &equiv,
        itemratio: &ratio,
    };
    let facts = w.fields.treasure_facts(1, 0);
    let d = dropper(&w.units, &w.stats, None, 0, 0, 0);
    let mut seed = Seed::init_low(99);
    let mut e = w.econ();
    let mut sink = ItemDrops::new(&mut e, Here);
    let out = walk(&data, &facts, &d, &mut seed, None, &args(0), &mut sink).unwrap();
    drop(sink);
    let it = w.items.get(out[0]).unwrap();
    assert_eq!((it.quality, it.file_index, it.ilvl), (q::UNIQUE, 0, 1));
}

/// Gold: base `roll(5 × ilvl) + ilvl` on the new unit's seed (in item
/// creation), the TC multiplier (`row` / 256) and the recipient's gold
/// find (stat 79 of R plus its owner's), all through stat 14 of the real
/// item.
#[test]
fn gold_amount_through_real_stats() {
    let mut w = World::new();
    let player = w.spawn(UnitType::Player, 0);
    let owner = w.spawn(UnitType::Player, 1);
    w.set_stat(player, 12, 20);
    w.set_stat(player, 79, 30);
    w.set_stat(owner, 79, 20);
    w.set_stat(player, 80, 7);
    let tcs = classes(1, &[(GOLD as u16, 1, 0, 512)]);
    let items = treasure_items(&w.tables);
    let (types, equiv, ratio) = (
        w.tables.itemtypes.clone(),
        w.tables.equiv.clone(),
        w.tables.itemratio.clone(),
    );
    let data = TreasureData {
        tcs: &tcs,
        items: &items,
        itemtypes: &types,
        equiv: &equiv,
        itemratio: &ratio,
    };
    let facts = w.fields.treasure_facts(1, 0);
    let d = dropper(&w.units, &w.stats, Some(player), 0, 0, 0);
    assert_eq!(d.kind, DropperKind::Player { level: 20 });
    let r = recipient(&w.units, &w.stats, player, Some(owner), None);
    assert_eq!((r.magic_find, r.gold_find), (7, 50));
    // Expected: the item's unit seed is the first game-seed step's.
    let mut g = w.fields.seed;
    let mut unit_seed = g.derive();
    let base = unit_seed.roll(100) as i32 + 20;
    let want = ((base * 512) >> 8) * 150 / 100;
    let mut seed = Seed::init_low(5);
    let mut e = w.econ();
    let mut sink = ItemDrops::new(&mut e, Here);
    let out = walk(
        &data,
        &facts,
        &d,
        &mut seed,
        Some(&r),
        &args(q::NORMAL),
        &mut sink,
    )
    .unwrap();
    assert_eq!(sink.gold(out[0]), want);
    drop(sink);
    assert_eq!(w.stats.unit_base(out[0], 14, 0), want);
    assert_eq!(w.items.get(out[0]).unwrap().record, GOLD);
}

/// Identical inputs give identical drops (item data, records, stats).
#[test]
fn walk_is_deterministic() {
    let run = || {
        let mut w = World::new();
        let monster = w.spawn(UnitType::Monster, MONSTER_CLASS);
        w.set_stat(monster, 12, 12);
        let tcs = classes(-3, &[(CAP as u16, 1, 0, 0), (RING as u16, 1, 0, 0)]);
        let items = treasure_items(&w.tables);
        let (types, equiv, ratio) = (
            w.tables.itemtypes.clone(),
            w.tables.equiv.clone(),
            w.tables.itemratio.clone(),
        );
        let data = TreasureData {
            tcs: &tcs,
            items: &items,
            itemtypes: &types,
            equiv: &equiv,
            itemratio: &ratio,
        };
        let facts = w.fields.treasure_facts(1, 0);
        let d = dropper(&w.units, &w.stats, Some(monster), 0, 0, 0);
        let mut seed = w.units.get(monster).unwrap().seed;
        let mut e = w.econ();
        let mut sink = ItemDrops::new(&mut e, Here);
        let out = walk(
            &data,
            &facts,
            &d,
            &mut seed,
            None,
            &args(q::NORMAL),
            &mut sink,
        )
        .unwrap();
        drop(sink);
        out.iter()
            .map(|&u| {
                let full = w.stats.full_entries(w.stats.unit_list(u).unwrap());
                (w.items.get(u).cloned(), w.units.get(u).cloned(), full)
            })
            .collect::<Vec<_>>()
    };
    let a = run();
    assert_eq!(a.len(), 2);
    assert_eq!(a, run());
}

/// The request's source unit is the dropper U, so a `body` item (type 40)
/// gets U's class as its file index; without a dropper, the request's index.
#[test]
fn a_body_drop_takes_the_droppers_class() {
    let mut w = World::new();
    let body = w.tables.items.len();
    w.tables.items.push(ItemRec {
        code: *b"bod ",
        type_: ty::BODY as i16,
        level: 1,
        ..ItemRec::default()
    });
    let monster = w.spawn(UnitType::Monster, MONSTER_CLASS);
    let req = |index| crate::treasure::DropRequest {
        id: body as u16,
        quality: q::NORMAL,
        index,
        item_level: 1,
        spot: DropSpot {
            room: None,
            x: 0,
            y: 0,
        },
        spawn_type: 3,
        init_flags: 1,
        item_format: 101,
        drop_flags: 0,
    };
    let (with, without) = {
        let mut e = w.econ();
        let mut sink = ItemDrops::new(&mut e, Here).with_unit(monster);
        let with = sink.create(req(0)).expect("created");
        let mut sink = ItemDrops::new(&mut e, Here);
        let without = sink.create(req(5)).expect("created");
        (with, without)
    };
    assert_eq!(w.items.get(with).unwrap().file_index, MONSTER_CLASS as i32);
    assert_eq!(w.items.get(without).unwrap().file_index, 5);
}

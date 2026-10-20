//! Gap tests for the quality roll: one rule (or a few tightly linked rules)
//! per test, synthetic tables, exact draw counts through `Seed` compares.

use d2_data::tables::{Itemratio, Record, Uniqueitems};

use super::*;
use crate::items::create::class_skill_mods;
use crate::items::quality::{dispatch, low_quality, roll_quality, set_item, superior};
use crate::items::tables::{QualityRec, RareRec, SetItemRec, UniqueRec};
use crate::items::{flag, q, req, stat, Fatal, ItemRequest};

/// `Seed::init_low(seed)` after `n` steps.
fn after(seed: u32, n: usize) -> Seed {
    let mut s = Seed::init_low(seed);
    for _ in 0..n {
        s.step();
    }
    s
}

fn rq(quality: u8) -> ItemRequest {
    ItemRequest {
        quality,
        flags2: req::NEVER_ETHEREAL,
        ..Default::default()
    }
}

fn run(t: &ItemTables, it: &mut Item<FakeStats>, rq: &mut ItemRequest) -> Result<bool, Fatal> {
    dispatch(t, &mut FakeGame::default(), it, rq)
}

/// Turns on class skill mods for `itype` (class 1, 30 skills from 36).
fn staffmods_on(t: &mut ItemTables, itype: u16) {
    t.itemtypes[itype as usize].staffmods = 1;
    t.skill_lists.counts[1] = 30;
    t.skill_lists.max = 30;
    t.skill_lists.lists = vec![0; 7 * 30];
    t.skill_lists.lists[30] = 36;
}

/// A ratio row: (base, divisor) for unique, rare, set, magic, superior,
/// normal.
fn ratio(version: u16, cols: [(u32, u32); 6]) -> Itemratio {
    let mut r = Itemratio::decode(&[0u8; Itemratio::SIZE]);
    r.version = version;
    (r.unique, r.uniquedivisor) = cols[0];
    (r.rare, r.raredivisor) = cols[1];
    (r.set, r.setdivisor) = cols[2];
    (r.magic, r.magicdivisor) = cols[3];
    (r.hiquality, r.hiqualitydivisor) = cols[4];
    (r.normal, r.normaldivisor) = cols[5];
    r
}

/// Ring rows: 2 suffixes, 2 prefixes; rare names `rare` per part.
fn ring_affixes(t: &mut ItemTables, rare: usize) {
    t.magic = (0..4).map(|k| affix_row(RING, k + 1)).collect();
    t.n_suffix = 2;
    t.n_prefix = 2;
    t.rare = (0..2 * rare)
        .map(|_| RareRec {
            itype: [RING as i16, 0, 0, 0, 0, 0, 0],
            ..Default::default()
        })
        .collect();
    t.n_rare_suffix = rare;
}

fn unique_row(code: &[u8; 4]) -> UniqueRec {
    UniqueRec {
        code: *code,
        enabled: true,
        rarity: 1,
        lvl: 1,
        props: [PropRec::NONE; 12],
        ..Default::default()
    }
}

fn set_row(code: &[u8; 4]) -> SetItemRec {
    SetItemRec {
        item: *code,
        set: 1,
        lvl: 1,
        rarity: 1,
        props: [PropRec::NONE; 9],
        aprops: [PropRec::NONE; 10],
        ..Default::default()
    }
}

// ---------------------------------------------------------------

/// Clear, Save and `D2`.
#[test]
fn terms_clear_save_d2() {
    // Clear: slots, rare names, file index; not the auto affix.
    let mut it = item(0, 1);
    (it.prefix, it.suffix) = ([1, 2, 3], [4, 5, 6]);
    (it.rare_prefix, it.rare_suffix, it.file_index, it.auto_affix) = (7, 8, 9, 10);
    it.clear();
    assert_eq!((it.prefix, it.suffix), ([0; 3], [0; 3]));
    assert_eq!((it.rare_prefix, it.rare_suffix, it.file_index), (0, 0, -1));
    assert_eq!(it.auto_affix, 10);

    // Save = the item seed's low word (not the start seed): a failed
    // magic item replays from {lo, 666}.
    let mut t = tables();
    let ring = push_item(&mut t, item_rec(RING, b"rin "));
    let mut it = item(ring, 7);
    it.item_seed = Seed::new(0x1234_5678, 999);
    run(&t, &mut it, &mut rq(q::MAGIC)).unwrap();
    assert_eq!(it.start_seed, 0x1234_5678);
    assert_eq!(it.item_seed, Seed::init_low(0x1234_5678));

    // D2 on a failed set (F = 2): stat 72 from the unit total, stat 73
    // from the base, each capped at 255.
    let mut r = item_rec(ty::HELM, b"cap ");
    r.durability = 20;
    let helm = push_item(&mut t, r);
    let d2 = |t: &ItemTables, format: u16| {
        let mut it = item(helm, 3);
        it.format = format;
        it.stats.set_base(stat::DURABILITY, 0, 10);
        it.stats.list_set(ListKey::ITEM, stat::DURABILITY, 0, 5);
        it.stats.set_base(stat::MAXDURABILITY, 0, 200);
        run(t, &mut it, &mut rq(q::SET)).unwrap();
        assert_eq!(it.quality, q::NORMAL);
        (
            it.stats.base(stat::DURABILITY, 0),
            it.stats.base(stat::MAXDURABILITY, 0),
        )
    };
    assert_eq!(d2(&t, 101), (30, 255));
    // Format 0: no D2 (the format-0 roll runs: give it a version-0 row
    // that returns at once). The format-0 set routine doubles the
    // durability itself (72 := min(15 × 2, 255), 73 :=
    // min(200 × 2, 255)); a D2 after it would give (35 × 2, 255).
    let mut t0 = t.clone();
    t0.itemratio = vec![ratio(0, [(0, 1); 6])];
    assert_eq!(d2(&t0, 0), (30, 255));
    // No durability: no D2.
    t.items[helm].durability = 0;
    assert_eq!(d2(&t, 101), (10, 200));
}

// ---------------------------------------------------------------

/// The roll draws on the item seed only; the `*Min` columns are unused.
#[test]
fn roll_on_item_seed_without_min_columns() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(AXE, b"axe "));
    t.itemratio = vec![ratio(1, [(50, 1); 6])];
    let mut m = t.clone();
    let r = &mut m.itemratio[0];
    (r.uniquemin, r.raremin, r.setmin, r.magicmin) = (1, 2, 3, 4);
    let rq = ItemRequest {
        ilvl: 10,
        ..Default::default()
    };
    for seed in 0..32 {
        let mut a = item(i, seed);
        a.unit_seed = Seed::init_low(99);
        let mut b = a.clone();
        let qa = roll_quality(&t, &mut a, &rq).unwrap();
        assert_eq!(roll_quality(&m, &mut b, &rq).unwrap(), qa);
        assert_eq!(a, b);
        assert_eq!(a.unit_seed, Seed::init_low(99));
        assert_ne!(a.item_seed, Seed::init_low(seed));
    }
}

/// The ratio row: version limit 0 for format 0, 100 otherwise, highest
/// version under it; none (or no item record) → fatal.
#[test]
fn ratio_row_version_limit() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    let k = 1000;
    // v0 → unique at once; v1 → rare (after one roll); v100 → set (after
    // two rolls); v101 → unique at once.
    let v0 = ratio(0, [(0, 1); 6]);
    let v1 = ratio(1, [(k, k), (0, 1), (0, 1), (0, 1), (0, 1), (0, 1)]);
    let v100 = ratio(100, [(k, k), (k, k), (0, 1), (0, 1), (0, 1), (0, 1)]);
    let v101 = ratio(101, [(0, 1); 6]);
    t.itemratio = vec![v0, v1, v100, v101];
    let seed = find_seed(|s| s.roll(1000) != 0 && s.roll(1000) != 0);
    let go = |t: &ItemTables, format: u16, record: usize| {
        let mut it = item(record, seed);
        it.format = format;
        roll_quality(t, &mut it, &ItemRequest::default())
    };
    assert_eq!(go(&t, 0, i), Ok(q::UNIQUE));
    assert_eq!(go(&t, 101, i), Ok(q::SET));
    let mut t1 = t.clone();
    t1.itemratio.remove(2);
    assert_eq!(go(&t1, 101, i), Ok(q::RARE));
    t1.itemratio.remove(0);
    assert_eq!(go(&t1, 0, i), Err(Fatal::NoRatioRow));
    assert!(go(&t, 101, i + 1).is_err(), "no item record");
}

/// Items `quest` → normal, without a draw.
#[test]
fn quest_item_rolls_normal() {
    let mut t = tables();
    let mut r = item_rec(AXE, b"qax ");
    r.quest = 1;
    let i = push_item(&mut t, r);
    t.itemratio = vec![ratio(1, [(0, 1); 6])];
    let mut it = item(i, 4);
    assert_eq!(
        roll_quality(&t, &mut it, &ItemRequest::default()),
        Ok(q::NORMAL)
    );
    assert_eq!(it.item_seed, Seed::init_low(4));
}

// ---------------------------------------------------------------

/// Step 1: no item record → 0; auto affix := 0; clear except the file
/// index.
#[test]
fn dispatch_step_one() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    let mut it = item(i + 1, 2);
    assert_eq!(run(&t, &mut it, &mut rq(q::NORMAL)), Ok(false));
    let mut it = item(i, 2);
    (it.prefix, it.suffix) = ([1, 2, 3], [4, 5, 6]);
    (it.rare_prefix, it.rare_suffix, it.file_index, it.auto_affix) = (7, 8, 9, 10);
    assert_eq!(run(&t, &mut it, &mut rq(q::NORMAL)), Ok(true));
    assert_eq!((it.prefix, it.suffix), ([0; 3], [0; 3]));
    assert_eq!((it.rare_prefix, it.rare_suffix), (0, 0));
    assert_eq!((it.file_index, it.auto_affix), (9, 0));
}

/// Step 2: the rolled quality, unless the request has one (which wins
/// even when the roll ran, as for format 0).
#[test]
fn dispatch_request_quality_wins() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.uniques = vec![unique_row(b"rin ")];
    t.itemratio = vec![ratio(0, [(0, 1); 6]), ratio(1, [(0, 1); 6])];
    // Rolled: unique.
    let mut it = item(i, 2);
    run(&t, &mut it, &mut rq(q::NONE)).unwrap();
    assert_eq!((it.quality, it.file_index), (q::UNIQUE, 0));
    // Format 0 rolls (unique) but the request's normal wins.
    let mut it = item(i, 2);
    it.format = 0;
    run(&t, &mut it, &mut rq(q::NORMAL)).unwrap();
    assert_eq!((it.quality, it.file_index), (q::NORMAL, -1));
}

/// Step 3: the four overrides, in order.
#[test]
fn dispatch_overrides() {
    let mut t = tables();
    // Charm rows (itemtype `magic`).
    t.magic = (0..4).map(|k| affix_row(ty::CHAR, k + 1)).collect();
    t.n_suffix = 2;
    t.n_prefix = 2;
    let charm = push_item(&mut t, item_rec(ty::CHAR, b"cm1 "));
    let mut qr = item_rec(ty::CHAR, b"qcm ");
    qr.quest = 1;
    let quest_charm = push_item(&mut t, qr);
    t.uniques = vec![unique_row(b"qcm "), unique_row(b"rin ")];
    let quality = |t: &ItemTables, i: usize, want: u8| {
        let mut it = item(i, 6);
        let mut r = rq(want);
        run(t, &mut it, &mut r).unwrap();
        (it.quality, r.quality)
    };
    // 1. itemtype magic: quality outside 4–9 → 4; inside kept (a failed
    // crafted charm drops to normal, not to magic); quest → 7.
    assert_eq!(quality(&t, charm, q::NORMAL).0, q::MAGIC);
    assert_eq!(quality(&t, charm, q::LOW).0, q::MAGIC);
    assert_eq!(quality(&t, charm, q::SUPERIOR).0, q::MAGIC);
    assert_eq!(quality(&t, charm, q::CRAFTED).0, q::NORMAL);
    assert_eq!(quality(&t, quest_charm, q::NORMAL).0, q::UNIQUE);
    // 2. itemtype rare = 0 and rare → magic directly (no downgrade: the
    // request keeps 6).
    let ring = push_item(&mut t, item_rec(RING, b"rin "));
    t.itemtypes[RING as usize].rare = 0;
    for k in 0..4 {
        t.magic[k].itype[0] = RING as i16;
    }
    assert_eq!(quality(&t, ring, q::RARE), (q::MAGIC, q::RARE));
    // 3. items unique → 7.
    t.items[ring].unique = 1;
    assert_eq!(quality(&t, ring, q::MAGIC).0, q::UNIQUE);
    // 4. itemtype normal → 2, last (over items unique).
    let mut g = item_rec(ty::GEM, b"gem ");
    g.unique = 1;
    let gem = push_item(&mut t, g);
    assert_eq!(quality(&t, gem, q::UNIQUE).0, q::NORMAL);
}

/// The routine table and its downgrade chains (text, step 4).
#[test]
fn dispatch_routine_table() {
    let mut t = tables();
    let ring = push_item(&mut t, item_rec(RING, b"rin "));
    let helm = push_item(&mut t, item_rec(ty::HELM, b"cap "));
    let axe = push_item(&mut t, item_rec(AXE, b"axe "));
    t.n_lowquality = 5;
    let go = |t: &ItemTables, i: usize, quality: u8| {
        let mut it = item(i, 3);
        it.item_seed = Seed::new(0xABCD, 999);
        let r = run(t, &mut it, &mut rq(quality));
        (r, it)
    };
    // 1 low: S; ring fails → normal, replayed from {s, 666}.
    let (r, it) = go(&t, ring, q::LOW);
    assert_eq!((r, it.quality, it.file_index), (Ok(true), q::NORMAL, -1));
    assert_eq!(it.item_seed, Seed::init_low(0xABCD));
    assert_eq!(go(&t, helm, q::LOW).1.quality, q::LOW);
    // 2 normal: no save, no rewind.
    let (_, it) = go(&t, ring, q::NORMAL);
    assert_eq!(
        (it.quality, it.item_seed),
        (q::NORMAL, Seed::new(0xABCD, 999))
    );
    // 3 superior → 2.
    t.qualityitems = vec![QualityRec {
        weapon: 1,
        ..Default::default()
    }];
    assert_eq!(go(&t, axe, q::SUPERIOR).1.quality, q::SUPERIOR);
    assert_eq!(go(&t, ring, q::SUPERIOR).1.quality, q::NORMAL);
    // 4 magic → 3 → 2.
    assert_eq!(go(&t, axe, q::MAGIC).1.quality, q::SUPERIOR);
    assert_eq!(go(&t, ring, q::MAGIC).1.quality, q::NORMAL);
    // 5 set → 4 → 3 → 2.
    assert_eq!(go(&t, axe, q::SET).1.quality, q::SUPERIOR);
    let mut s = t.clone();
    s.setitems = vec![set_row(b"axe ")];
    assert_eq!(go(&s, axe, q::SET).1.quality, q::SET);
    // 6 rare → 4 → 3 → 2.
    assert_eq!(go(&t, axe, q::RARE).1.quality, q::SUPERIOR);
    // 7 unique → 6 → 4 → 3 → 2.
    assert_eq!(go(&t, axe, q::UNIQUE).1.quality, q::SUPERIOR);
    // With ring affixes: set → magic; rare without names → magic; unique
    // → rare; but crafted and tempered → normal directly.
    ring_affixes(&mut t, 0);
    assert_eq!(go(&t, ring, q::SET).1.quality, q::MAGIC);
    assert_eq!(go(&t, ring, q::RARE).1.quality, q::MAGIC);
    assert_eq!(go(&t, ring, q::CRAFTED).1.quality, q::NORMAL);
    assert_eq!(go(&t, ring, q::TEMPERED).1.quality, q::NORMAL);
    ring_affixes(&mut t, 1);
    t.rare.truncate(1); // rare suffix only: rare, crafted, tempered fail
    assert_eq!(go(&t, ring, q::UNIQUE).1.quality, q::MAGIC);
    ring_affixes(&mut t, 1);
    assert_eq!(go(&t, ring, q::UNIQUE).1.quality, q::RARE);
    assert_eq!(go(&t, ring, q::CRAFTED).1.quality, q::CRAFTED);
    // Other qualities → 0.
    assert_eq!(go(&t, ring, 10).0, Ok(false));
    // Unique "before": file index := −1 (seen when an items-`unique` base
    // has no candidate and stays unique).
    t.items[ring].unique = 1;
    let mut it = item(ring, 3);
    it.file_index = 5;
    run(&t, &mut it, &mut rq(q::UNIQUE)).unwrap();
    assert_eq!((it.quality, it.file_index), (q::UNIQUE, -1));
}

/// Finishing, in order: ethereal (format ≥ 100), sockets (1–3),
/// automagic (format ≥ 100, not 5 or 7, `auto prefix`); then 1.
#[test]
fn dispatch_finishing_order() {
    let mut t = tables();
    let mut r = item_rec(AXE, b"axe ");
    (r.durability, r.hasinv, r.gemsockets) = (30, 1, 3);
    (r.invwidth, r.invheight, r.auto_prefix) = (2, 3, 5);
    let axe = push_item(&mut t, r);
    t.itemtypes[AXE as usize].maxsock40 = 3;
    t.magic = vec![affix_row(AXE, 5)]; // the automagic part only
    t.uniques = vec![unique_row(b"axe ")];
    t.setitems = vec![set_row(b"axe ")];
    t.qualityitems = vec![QualityRec {
        weapon: 1,
        ..Default::default()
    }];
    let go = |seed: u32, quality: u8, format: u16| {
        let mut it = item(axe, seed);
        it.format = format;
        it.stats.set_base(stat::DURABILITY, 0, 10);
        it.stats.set_base(stat::MAXDURABILITY, 0, 20);
        let mut r = ItemRequest {
            quality,
            ..Default::default()
        };
        assert_eq!(run(&t, &mut it, &mut r), Ok(true));
        it
    };
    let has = |it: &Item<FakeStats>, f| it.flags & f != 0;
    // Ethereal (< 5) then sockets (≥ 33): ethereal only; then the
    // automagic coin and pick.
    let a = find_seed(|s| s.roll(100) < 5 && s.roll(100) >= 33);
    let it = go(a, q::NORMAL, 101);
    assert!(has(&it, flag::ETHEREAL) && !has(&it, flag::SOCKETED));
    assert_eq!(it.auto_affix, 1);
    assert_eq!(it.item_seed, after(a, 4));
    // Ethereal (≥ 5) then sockets (< 33).
    let b = find_seed(|s| s.roll(100) >= 5 && s.roll(100) < 33);
    let it = go(b, q::NORMAL, 101);
    assert!(!has(&it, flag::ETHEREAL) && has(&it, flag::SOCKETED));
    // Superior (roll(1) first): all three steps.
    let c = find_seed(|s| {
        s.step();
        s.roll(100) < 5 && s.roll(100) < 33
    });
    let it = go(c, q::SUPERIOR, 101);
    assert!(has(&it, flag::ETHEREAL) && has(&it, flag::SOCKETED));
    assert_eq!((it.quality, it.auto_affix), (q::SUPERIOR, 1));
    assert_eq!(it.item_seed, after(c, 5));
    // Format < 100: sockets only.
    let it = go(a, q::NORMAL, 2);
    assert!(!has(&it, flag::ETHEREAL) && has(&it, flag::SOCKETED));
    assert_eq!(it.auto_affix, 0);
    assert_eq!(it.item_seed, after(a, 1));
    // Unique and set (one roll(1) each): no sockets, no automagic; the
    // ethereal roll draws for the unique (sets skip it inside the roll).
    for (quality, steps) in [(q::UNIQUE, 2), (q::SET, 1)] {
        let it = go(a, quality, 101);
        assert_eq!(it.quality, quality);
        assert_eq!(it.auto_affix, 0);
        assert!(!has(&it, flag::SOCKETED));
        assert_eq!(it.item_seed, after(a, steps));
    }
}

// ---------------------------------------------------------------

/// D(x): clear the failed routine's leftovers (rare names, file index).
#[test]
fn downgrade_clears() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    ring_affixes(&mut t, 1);
    for r in &mut t.magic {
        r.rare = 0; // no affix on a rare: the rare routine fails after
                    // writing its names
    }
    let mut it = item(i, 3);
    run(&t, &mut it, &mut rq(q::RARE)).unwrap();
    assert_eq!(it.quality, q::MAGIC);
    assert_eq!((it.rare_prefix, it.rare_suffix), (0, 0));
    // Low quality writes a file index, then fails on a ring.
    t.n_lowquality = 5;
    let seed = find_seed(|s| s.roll(5) != 0);
    let mut it = item(i, seed);
    run(&t, &mut it, &mut rq(q::LOW)).unwrap();
    assert_eq!((it.quality, it.file_index), (q::NORMAL, -1));
}

/// D(x) replays from {s, 666} (drawing when the request quality is
/// 0), saves again and runs x's routine; D(2) is the normal routine.
#[test]
fn downgrade_replays_quality_roll() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    // Unique c = 999 (ring: L = 1), rare c < 1: one roll then rare.
    t.itemratio = vec![ratio(
        1,
        [(1000, 1), (0, 1), (0, 1), (0, 1), (0, 1), (0, 1)],
    )];
    let seed = find_seed(|s| s.roll(999) != 0);
    let mut it = item(i, seed);
    let mut r = rq(q::NONE);
    run(&t, &mut it, &mut r).unwrap();
    // Rare fails (no names, no draw) → D(4): {s1, 666}, draws again
    // (request quality still 0); s2 saved; magic fails → D(3): {s2, 666},
    // no draw (request quality 4); superior fails → D(2): {s2, 666}.
    let mut s = Seed::init_low(seed);
    s.roll(999);
    let mut s1 = Seed::init_low(s.lo);
    s1.roll(999);
    let s2 = s1.lo;
    assert_eq!((it.quality, r.quality), (q::NORMAL, q::NORMAL));
    assert_eq!(it.item_seed, Seed::init_low(s2));
    assert_eq!(it.start_seed, s2);
}

// ---------------------------------------------------------------

/// Armor: defense × 75 / 100, at least 1; success.
#[test]
fn low_quality_armor() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(ty::HELM, b"cap "));
    t.n_lowquality = 3;
    for (ac, want) in [(10, 7), (1, 1), (100, 75)] {
        let mut it = item(i, 1);
        it.stats.set_base(stat::ARMORCLASS, 0, ac);
        assert!(low_quality(&t, &mut it, &ItemRequest::default()));
        assert_eq!(it.stats.base(stat::ARMORCLASS, 0), want);
    }
}

/// Neither weapon nor armor: result 0 (after the file index roll).
#[test]
fn low_quality_other_fails() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.n_lowquality = 3;
    let mut it = item(i, 1);
    it.stats.set_base(stat::ARMORCLASS, 0, 10);
    assert!(!low_quality(&t, &mut it, &ItemRequest::default()));
    assert_eq!(it.stats.base(stat::ARMORCLASS, 0), 10);
    assert_eq!(it.item_seed, after(1, 1));
}

/// Class skill mods run last in all three cases.
#[test]
fn low_quality_class_skill_mods() {
    let rq = ItemRequest {
        ilvl: 40,
        flags2: req::STAFFMODS_ILVL,
        ..Default::default()
    };
    for itype in [AXE, ty::HELM, RING] {
        let mut t = tables();
        let i = push_item(&mut t, item_rec(itype, b"itm "));
        t.n_lowquality = 3;
        let mut off = item(i, 5);
        off.quality = q::LOW;
        let mut on = off.clone();
        let r_off = low_quality(&t, &mut off, &rq);
        staffmods_on(&mut t, itype);
        let r_on = low_quality(&t, &mut on, &rq);
        assert_eq!(r_on, r_off);
        class_skill_mods(&t, &mut off, &rq);
        assert_eq!(on, off, "type {itype}");
        assert_ne!(on.item_seed, after(5, 1), "mods drew");
    }
}

// ---------------------------------------------------------------

/// n = qualityitems count, or 4 for throwable / `nodurability` items.
#[test]
fn superior_row_count() {
    let mut t = tables();
    let axe = push_item(&mut t, item_rec(AXE, b"axe "));
    let mut nd = item_rec(AXE, b"nax ");
    nd.nodurability = 1;
    let nodur = push_item(&mut t, nd);
    let thrown = push_item(&mut t, item_rec(THROWN, b"tax "));
    // Only row 6 fits a weapon.
    t.qualityitems = (0..8)
        .map(|k| QualityRec {
            weapon: u8::from(k == 6),
            ..Default::default()
        })
        .collect();
    let mut it = item(axe, 2);
    assert!(superior(&t, &mut it, &ItemRequest::default()));
    assert_eq!(it.file_index, 6);
    for i in [nodur, thrown] {
        let mut it = item(i, 2);
        assert!(!superior(&t, &mut it, &ItemRequest::default()));
        // roll(4) until all four rows are tried.
        let mut s = Seed::init_low(2);
        let mut seen = [false; 4];
        while !seen.iter().all(|&b| b) {
            seen[s.roll(4) as usize] = true;
        }
        assert_eq!(it.item_seed, s);
    }
}

// ---------------------------------------------------------------

/// Sets have no dropped bits: the same set row twice from one game.
#[test]
fn set_items_repeat() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.setitems = vec![set_row(b"rin ")];
    let mut game = FakeGame::default();
    for seed in [1, 2] {
        let mut it = item(i, seed);
        dispatch(&t, &mut game, &mut it, &mut rq(q::SET)).unwrap();
        assert_eq!((it.quality, it.file_index), (q::SET, 0));
    }
    assert_eq!(game.uniques, UniqueBits::default());
}

/// File index := row; identified cleared; set properties; success.
#[test]
fn set_item_result() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.properties = vec![prop1(1, stat::TOBLOCK)];
    let mut a = set_row(b"amu ");
    a.lvl = 1;
    let mut b = set_row(b"rin ");
    b.props[0] = rec(0, 0, 7, 7);
    t.setitems = vec![a, b];
    let mut it = item(i, 1);
    it.flags = flag::IDENTIFIED;
    assert!(set_item(&t, &mut it, &ItemRequest::default()));
    assert_eq!(it.file_index, 1);
    assert_eq!(it.flags & flag::IDENTIFIED, 0);
    assert_eq!(it.stats.item_list(stat::TOBLOCK, 0), 7);
}

// ------------------------------------------------------- edge cases

/// Edge case 4: the unique rarity is the 32-bit value at +0x30.
#[test]
fn unique_rarity_is_32_bits() {
    let mut raw = vec![0u8; Uniqueitems::SIZE];
    raw[0x30..0x34].copy_from_slice(&[3, 0, 1, 0]);
    let r = Uniqueitems::decode(&raw);
    assert_eq!(r.rarity, 3);
    assert_eq!(UniqueRec::from_record(&r, &raw).rarity, 0x1_0003);
    raw[0x32] = 0;
    assert_eq!(UniqueRec::from_record(&r, &raw).rarity, 3);
}

//! Format-0 (legacy, version-0x47 save) branches:,
//! mode 4. Synthetic tables; draws checked through `Seed` compares.

use d2_data::tables::{Itemratio, Record};

use super::*;
use crate::items::affixes::{crafted, rare, rare_name, roll_affix};
use crate::items::create::{class_skill_mods, normal, normal_legacy};
use crate::items::props::{apply_property, mode, PropCtx};
use crate::items::props_legacy::{LegacyFn, LEGACY, LEGACY_COUNT};
use crate::items::quality::{dispatch, low_quality, roll_quality, set_item, unique};
use crate::items::tables::{RareRec, SetItemRec, SetRec, SkillRec, UniqueRec};
use crate::items::{flag, q, req, stat, Fatal, ItemRequest, RequestUnit};

/// A format-0 item of record `i`, ilvl 50, item seed `seed`.
fn f0(i: usize, seed: u32) -> Item<FakeStats> {
    let mut it = item(i, seed);
    it.format = 0;
    it
}

/// `Seed::init_low(seed)` after `n` steps.
fn after(seed: u32, n: usize) -> Seed {
    let mut s = Seed::init_low(seed);
    for _ in 0..n {
        s.step();
    }
    s
}

/// A version-0 ratio row: (base, divisor) for unique, rare, set, magic,
/// superior, normal.
fn ratio0(cols: [(u32, u32); 6]) -> Itemratio {
    let mut r = Itemratio::decode(&[0u8; Itemratio::SIZE]);
    r.version = 0;
    (r.unique, r.uniquedivisor) = cols[0];
    (r.rare, r.raredivisor) = cols[1];
    (r.set, r.setdivisor) = cols[2];
    (r.magic, r.magicdivisor) = cols[3];
    (r.hiquality, r.hiqualitydivisor) = cols[4];
    (r.normal, r.normaldivisor) = cols[5];
    r
}

/// Tables with 268 empty properties rows (the base-game count).
fn tables268() -> ItemTables {
    let mut t = tables();
    t.properties = vec![PropertyRec::default(); 268];
    t
}

/// Class skill mods for `itype`: class 1, 30 skills from `first`.
fn staffmods_on(t: &mut ItemTables, itype: u16, first: u16) {
    t.itemtypes[itype as usize].staffmods = 1;
    t.skill_lists.counts[1] = 30;
    t.skill_lists.max = 30;
    t.skill_lists.lists = vec![0; 7 * 30];
    t.skill_lists.lists[30] = first;
}

fn ap(t: &ItemTables, it: &mut Item<FakeStats>, m: u8, r: PropRec) {
    apply_property(t, it, &mut PropCtx::item(m), &r);
}

// ------------------------------------------------------------ quality

/// The roll runs with a request quality, version limit 0, L
/// unadjusted, unique / rare / set without divisor, c < 1 drawn as 1.
#[test]
fn f0_quality_roll() {
    let mut t = tables();
    let ring = push_item(&mut t, item_rec(RING, b"rin "));
    t.items[ring].level = 30;
    let rq6 = ItemRequest {
        quality: q::RARE,
        ilvl: 5,
        ..Default::default()
    };
    // Version limit 0: the fixture's version-1 row is not taken.
    let mut it = f0(ring, 1);
    assert_eq!(roll_quality(&t, &mut it, &rq6), Err(Fatal::NoRatioRow));
    // Unique 10 − 5 = 5 (divisor 0 not read; format ≥ 1 would use L = 1
    // for a misc item); rare 0 − 5 < 1 → roll(1), always a hit.
    t.itemratio = vec![ratio0([(10, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)])];
    let hit = find_seed(|s| s.roll(5) == 0);
    let miss = find_seed(|s| s.roll(5) != 0);
    let mut it = f0(ring, hit);
    assert_eq!(roll_quality(&t, &mut it, &rq6), Ok(q::UNIQUE));
    assert_eq!(it.item_seed, after(hit, 1));
    let mut it = f0(ring, miss);
    assert_eq!(roll_quality(&t, &mut it, &rq6), Ok(q::RARE));
    assert_eq!(it.item_seed, after(miss, 2));
    // The format ≥ 1 roll returns the request quality without a draw.
    let mut it = item(ring, miss);
    assert_eq!(roll_quality(&t, &mut it, &rq6), Ok(q::RARE));
    assert_eq!(it.item_seed, Seed::init_low(miss));
    // Magic, superior and normal divide: a divisor 0 reached is fatal.
    t.itemratio = vec![ratio0([
        (1000, 0),
        (1000, 0),
        (1000, 0),
        (1, 0),
        (0, 0),
        (0, 0),
    ])];
    let s3 = find_seed(|s| (0..3).all(|_| s.roll(995) != 0));
    let mut it = f0(ring, s3);
    assert_eq!(roll_quality(&t, &mut it, &rq6), Err(Fatal::DivideByZero));
    // None hit: (low, or superior with flags2 0x40).
    t.itemratio = vec![ratio0([(1000, 1); 6])];
    let s6 = find_seed(|s| (0..6).all(|_| s.roll(995) != 0));
    let mut it = f0(ring, s6);
    assert_eq!(roll_quality(&t, &mut it, &rq6), Ok(q::LOW));
    assert_eq!(it.item_seed, after(s6, 6));
    let sup = ItemRequest {
        flags2: req::SUPERIOR,
        ..rq6.clone()
    };
    let mut it = f0(ring, s6);
    assert_eq!(roll_quality(&t, &mut it, &sup), Ok(q::SUPERIOR));
    // `quest` → normal, no draw.
    t.items[ring].quest = 1;
    let mut it = f0(ring, s6);
    assert_eq!(roll_quality(&t, &mut it, &rq6), Ok(q::NORMAL));
    assert_eq!(it.item_seed, Seed::init_low(s6));
}

/// Table test, durability / 3, base damage and defense, throw
/// stats from the totals, no class skill mods.
#[test]
fn f0_low_quality() {
    let mut t = tables();
    let mut r = item_rec(THROWN, b"tax ");
    r.durability = 30;
    let tax = push_item(&mut t, r);
    staffmods_on(&mut t, THROWN, 36);
    let rq = ItemRequest {
        ilvl: 50,
        ..Default::default()
    };
    let setup = |it: &mut Item<FakeStats>| {
        it.unit_seed = Seed::init_low(77);
        it.stats.set_base(stat::MAXDAMAGE, 0, 10);
        it.stats.list_set(ListKey::ITEM, stat::MAXDAMAGE, 0, 4);
        it.stats.set_base(stat::MINDAMAGE, 0, 1);
        it.stats.set_base(stat::THROW_MINDAMAGE, 0, 4);
        it.stats
            .list_set(ListKey::ITEM, stat::THROW_MINDAMAGE, 0, 4);
        it.stats.set_base(stat::THROW_MAXDAMAGE, 0, 1);
    };
    // No lowqualityitems rows → 0, no draw.
    let mut it = f0(tax, 9);
    setup(&mut it);
    assert!(!low_quality(&t, &mut it, &rq));
    assert_eq!(it.item_seed, Seed::init_low(9));
    t.n_lowquality = 3;
    let mut it = f0(tax, 9);
    setup(&mut it);
    assert!(low_quality(&t, &mut it, &rq));
    // The file index is the only item-seed draw (no class skill
    // mods although they are on for the type).
    let mut s = Seed::init_low(9);
    assert_eq!(it.file_index, s.roll(3) as i32);
    assert_eq!(it.item_seed, s);
    assert!(it
        .stats
        .lists
        .values()
        .all(|l| l.keys().all(|&(id, _)| id != stat::ITEM_SINGLESKILL)));
    // M = 30 / 3 = 10 (format ≥ 1: 30 × 33 / 100 = 9); unit seed.
    let mut u = Seed::init_low(77);
    assert_eq!(it.stats.base(stat::DURABILITY, 0), u.roll(5) as i32 + 5);
    assert_eq!(it.stats.base(stat::MAXDURABILITY, 0), 10);
    assert_eq!(it.unit_seed, u);
    // Base 22 (10 × 75 / 100 = 7, not the total 14's 10), 21 ≥ 1,
    // 24 ≥ 2, 23 ≥ 1; throw min from the total 8 → 6 (≥ 2), max ≥ 1.
    let b = |id| it.stats.base(id, 0);
    assert_eq!(b(stat::MAXDAMAGE), 7);
    assert_eq!(b(stat::MINDAMAGE), 1);
    assert_eq!(b(stat::SECONDARY_MAXDAMAGE), 2);
    assert_eq!(b(stat::SECONDARY_MINDAMAGE), 1);
    assert_eq!(b(stat::THROW_MINDAMAGE), 6);
    assert_eq!(b(stat::THROW_MAXDAMAGE), 1);
    // Armor, base 31 (100 → 75; the total 150 would give 112).
    let helm = push_item(&mut t, item_rec(ty::HELM, b"cap "));
    let mut it = f0(helm, 9);
    it.stats.set_base(stat::ARMORCLASS, 0, 100);
    it.stats.list_set(ListKey::ITEM, stat::ARMORCLASS, 0, 50);
    assert!(low_quality(&t, &mut it, &rq));
    assert_eq!(it.stats.base(stat::ARMORCLASS, 0), 75);
    // Neither → 0.
    let ring = push_item(&mut t, item_rec(RING, b"rin "));
    let mut it = f0(ring, 9);
    assert!(!low_quality(&t, &mut it, &rq));
    // No items row → 0, no draw.
    let mut it = f0(t.items.len(), 9);
    assert!(!low_quality(&t, &mut it, &rq));
    assert_eq!(it.item_seed, Seed::init_low(9));
}

fn urow(code: &[u8; 4]) -> UniqueRec {
    UniqueRec {
        code: *code,
        enabled: true,
        rarity: 1,
        lvl: 1,
        props: [PropRec::NONE; 12],
        ..Default::default()
    }
}

/// Durability × 5 first; forced as for the unique roll; not forced: the first
/// fitting row, no `lvl` test, no draw, no marking.
#[test]
fn f0_unique() {
    let mut t = tables268();
    let mut r = item_rec(ty::HELM, b"cap ");
    r.durability = 20;
    let helm = push_item(&mut t, r);
    let mut rows: Vec<UniqueRec> = (0..6).map(|_| urow(b"cap ")).collect();
    rows[0].version = 100;
    rows[1].enabled = false;
    rows[2].ladder = true;
    rows[3].lvl = 99;
    rows[3].props[0] = rec(0, 0, 5, 5);
    rows[5].code = *b"xxx ";
    t.uniques = rows;
    let rq = ItemRequest::default();
    let fresh = || {
        let mut it = f0(helm, 4);
        it.stats.set_base(stat::DURABILITY, 0, 10);
        it.stats.list_set(ListKey::ITEM, stat::DURABILITY, 0, 2);
        it.stats.set_base(stat::MAXDURABILITY, 0, 30);
        it.flags = flag::IDENTIFIED;
        it
    };
    // Row 3 (lvl 99 > ilvl 50 is not tested); no draw, no marking.
    let mut g = FakeGame::default();
    let mut it = fresh();
    assert!(unique(&t, &mut g, &mut it, &rq));
    assert_eq!(it.file_index, 3);
    assert_eq!(it.flags & flag::IDENTIFIED, 0);
    assert_eq!(it.stats.item_list(stat::ARMORCLASS, 0), 5);
    assert_eq!(it.item_seed, Seed::init_low(4));
    assert!(!g.uniques.get(3));
    // 72 := total 12 × 5, 73 := base 30 × 5.
    assert_eq!(it.stats.base(stat::DURABILITY, 0), 60);
    assert_eq!(it.stats.base(stat::MAXDURABILITY, 0), 150);
    // A dropped row is skipped unless the item is a quest item.
    g.uniques.set(3);
    let mut it = fresh();
    assert!(unique(&t, &mut g, &mut it, &rq));
    assert_eq!(it.file_index, 4);
    t.items[helm].quest = 1;
    let mut it = fresh();
    assert!(unique(&t, &mut g, &mut it, &rq));
    assert_eq!(it.file_index, 3);
    t.items[helm].quest = 0;
    // The ladder row is taken in a ladder game.
    let mut gl = FakeGame {
        ladder: (false, true),
        ..Default::default()
    };
    let mut it = fresh();
    assert!(unique(&t, &mut gl, &mut it, &rq));
    assert_eq!(it.file_index, 2);
    // None → 0.
    g.uniques.set(4);
    let mut it = fresh();
    assert!(!unique(&t, &mut g, &mut it, &rq));
    // Forced as for the unique roll (a mismatched code: no properties).
    let forced = |index| ItemRequest {
        force: true,
        index,
        ..Default::default()
    };
    let mut it = fresh();
    assert!(unique(&t, &mut g, &mut it, &forced(3)));
    assert_eq!(it.file_index, 3);
    assert_eq!(it.stats.item_list(stat::ARMORCLASS, 0), 5);
    let mut it = fresh();
    assert!(unique(&t, &mut g, &mut it, &forced(5)));
    assert_eq!(it.file_index, 5);
    assert_eq!(it.flags & flag::IDENTIFIED, flag::IDENTIFIED);
    let mut it = fresh();
    assert!(!unique(&t, &mut g, &mut it, &forced(6)));
    // caps at 255.
    let mut it = fresh();
    it.stats.set_base(stat::MAXDURABILITY, 0, 60);
    assert!(unique(&t, &mut g, &mut it, &forced(3)));
    assert_eq!(it.stats.base(stat::MAXDURABILITY, 0), 255);
}

/// Durability × 2 without a test, k := roll(n) over the leading
/// version < 100 sets, the last matching set wins, the seed's low word
/// restored on failure; properties: `prop1`–`prop2` only (mode 4).
#[test]
fn f0_set_item() {
    let mut t = tables268();
    // Function 1 → stat 31 for the format-101 comparison.
    t.properties[0] = prop1(1, 31);
    let helm = push_item(&mut t, item_rec(ty::HELM, b"cap "));
    let other = push_item(&mut t, item_rec(ty::HELM, b"zzz "));
    let set = |version, count| SetRec {
        count,
        version,
        ..Default::default()
    };
    // Sets 0 and 1 count (n = 2); set 2 (version 100) ends the count, so
    // set 3 is never scanned.
    t.sets = vec![set(0, 2), set(0, 1), set(100, 1), set(0, 1)];
    let srow = |code: &[u8; 4], s: i16| SetItemRec {
        item: *code,
        set: s,
        props: [PropRec::NONE; 9],
        aprops: [PropRec::NONE; 10],
        add_func: 1,
        ..Default::default()
    };
    let mut r0 = srow(b"cap ", 0);
    r0.props[0] = rec(0, 0, 5, 5);
    r0.props[1] = rec(0, 0, 1, 1);
    r0.props[2] = rec(0, 0, 100, 100);
    r0.aprops[0] = rec(0, 0, 1000, 1000);
    t.setitems = vec![r0, srow(b"cap ", 0), srow(b"cap ", 1), srow(b"cap ", 3)];
    let rq = ItemRequest::default();
    let k0 = find_seed(|s| s.roll(2) == 0);
    let k1 = find_seed(|s| s.roll(2) == 1);
    // k = 0: sets 0 then 1 → row 2 (set 1 overwrites).
    let mut it = f0(helm, k0);
    assert!(set_item(&t, &mut it, &rq));
    assert_eq!(it.file_index, 2);
    assert_eq!(it.item_seed, after(k0, 1));
    // k = 1: sets 1 then 0 → row 0 (the first match of set 0 ends its
    // scan); only prop1–prop2 run: 5 + 1, no prop3, no partial record.
    let mut it = f0(helm, k1);
    it.flags = flag::IDENTIFIED;
    it.stats.set_base(stat::DURABILITY, 0, 10);
    it.stats.set_base(stat::MAXDURABILITY, 0, 200);
    assert!(set_item(&t, &mut it, &rq));
    assert_eq!(it.file_index, 0);
    assert_eq!(it.flags & flag::IDENTIFIED, 0);
    assert_eq!(it.stats.item_list(stat::ARMORCLASS, 0), 6);
    assert_eq!(it.stats.lists.len(), 1, "no itemset state list");
    // × 2 with no durability test (items `durability` 0).
    assert_eq!(it.stats.base(stat::DURABILITY, 0), 20);
    assert_eq!(it.stats.base(stat::MAXDURABILITY, 0), 255);
    // Format 101 runs all records (prop3 and the partial record too).
    let mut it101 = item(helm, k1);
    it101.file_index = 0;
    crate::items::props::apply_set_item(&t, &mut it101);
    assert_eq!(it101.stats.lists.len(), 2);
    // No match → seed {s0, 666}, 0.
    let mut it = f0(other, k1);
    assert!(!set_item(&t, &mut it, &rq));
    assert_eq!(it.item_seed, Seed::init_low(k1));
    assert_eq!(it.file_index, -1);
    // No counted set (n = 0) → no step.
    let mut t2 = t.clone();
    t2.sets[0].version = 100;
    let mut it = f0(helm, k1);
    assert!(!set_item(&t2, &mut it, &rq));
    assert_eq!(it.item_seed, Seed::init_low(k1));
    // No items row → 0.
    let mut it = f0(t.items.len(), k1);
    assert!(!set_item(&t, &mut it, &rq));
}

// ------------------------------------------------------------ affixes

/// Alvl = ilvl + 2; spawnable, version, `level` and fit only; an
/// unweighted roll(n); the preferred row replaces the pick only when
/// listed; the group argument is dropped.
#[test]
fn f0_roller() {
    let mut t = tables268();
    let mut r = item_rec(RING, b"rin ");
    r.magic_lvl = 40;
    r.level = 90;
    let ring = push_item(&mut t, r);
    let mut p = vec![affix_row(RING, 1); 6];
    // Kept: level 52 = ilvl 50 + 2, although frequency 0, maxlevel below
    // alvl, `rare` 0, another class and the same group as p5.
    (p[0].level, p[0].frequency, p[0].maxlevel, p[0].rare) = (52, 0, 1, 0);
    p[0].classspecific = 3;
    p[1].level = 53; // above alvl
    p[2].spawnable = 0;
    p[3].version = 100;
    p[4].itype = [AXE as i16, 0, 0, 0, 0, 0, 0];
    p[5].mods[0] = rec(0, 0, 7, 7);
    t.magic = vec![affix_row(RING, 1), affix_row(RING, 1)];
    t.magic.extend(p);
    t.n_suffix = 2;
    t.n_prefix = 6;
    // Combined ids: p0 → 3, p5 → 8.
    let ids = [3u16, 8];
    // The coin.
    let even = find_seed(|s| s.step() & 1 == 0);
    let mut it = f0(ring, even);
    assert_eq!(roll_affix(&t, &mut it, true, false, false, true, 0, 0), 0);
    assert_eq!(it.item_seed, after(even, 1));
    for seed in 0..40u32 {
        let mut s = Seed::init_low(seed);
        s.step();
        let pick = ids[s.roll_range(0, 2) as usize];
        // A group with prefix = false still rolls the prefixes.
        let mut it = f0(ring, seed);
        let a = roll_affix(&t, &mut it, true, true, false, false, 0, 9);
        assert_eq!((a, it.item_seed), (pick, s), "seed {seed}");
        // Preferred p5 (index 5 in part → combined 7, id 8) listed.
        let mut it = f0(ring, seed);
        assert_eq!(roll_affix(&t, &mut it, true, true, true, true, 6, 0), 8);
        assert_eq!(it.stats.item_list(stat::ARMORCLASS, 0), 7);
        // Preferred p1 not listed → 0, but candidate r's properties.
        let mut it = f0(ring, seed);
        assert_eq!(roll_affix(&t, &mut it, true, true, true, true, 2, 0), 0);
        let want = if pick == 8 { 7 } else { 0 };
        assert_eq!(it.stats.item_list(stat::ARMORCLASS, 0), want);
    }
    // Without require-spawnable p2 is listed too.
    let s = find_seed(|s| {
        s.step();
        s.roll_range(0, 3) == 1
    });
    let mut it = f0(ring, s);
    assert_eq!(roll_affix(&t, &mut it, false, true, false, true, 0, 0), 5);
    // No candidate → 0 (the suffixes fit only an axe here).
    t.magic[0].itype = [AXE as i16, 0, 0, 0, 0, 0, 0];
    t.magic[1].itype = [AXE as i16, 0, 0, 0, 0, 0, 0];
    let mut it = f0(ring, 1);
    assert_eq!(roll_affix(&t, &mut it, true, true, false, false, 0, 0), 0);
    assert_eq!(it.item_seed, after(1, 1));
}

/// Rare rows fitting `itype`: `n` per part.
fn rare_rows(t: &mut ItemTables, itype: u16, n: usize) {
    t.rare = (0..2 * n)
        .map(|_| RareRec {
            itype: [itype as i16, 0, 0, 0, 0, 0, 0],
            ..Default::default()
        })
        .collect();
    t.n_rare_suffix = n;
}

/// The same rare-name pick as for a current-format item.
#[test]
fn f0_rare_name_is_section_5() {
    let mut t = tables();
    let ring = push_item(&mut t, item_rec(RING, b"rin "));
    rare_rows(&mut t, RING, 3);
    for seed in 0..20 {
        for prefix in [true, false] {
            let mut a = f0(ring, seed);
            let mut b = item(ring, seed);
            assert_eq!(rare_name(&t, &mut a, prefix), rare_name(&t, &mut b, prefix));
            assert_eq!(a.item_seed, b.item_seed);
        }
    }
}

/// The legacy affix draws for one suffix row and one prefix row (`avail`: the
/// kind has a fitting row), in the roller's order; returns the seed
/// after, whether a prefix and a suffix were stored, and the pass count.
fn rare_mirror(seed: u32, jewel: bool, avail: [bool; 2]) -> (Seed, [bool; 2]) {
    let mut s = Seed::init_low(seed);
    s.roll_range(0, 1);
    s.roll_range(0, 1);
    let n = if jewel {
        s.roll_range(3, 2)
    } else {
        s.roll_range(4, 3)
    };
    let mut filled = [false; 2]; // prefix, suffix
    for _ in 0..n {
        let lo = s.step();
        // P and S never reach 3 with one row per kind.
        let k = usize::from(lo & 1 == 1);
        if !avail[k] {
            s.step(); // the coin; no candidate, the pass ends
        } else if filled[k] {
            for _ in 0..252 {
                s.step();
                s.roll_range(0, 1);
            }
        } else {
            s.step();
            s.roll_range(0, 1);
            filled[k] = true;
        }
    }
    (s, filled)
}

/// Names, count 4–6 (jewels 3–4), the kind step every pass, no
/// preferences, a failed roll ends the pass only, 252 tries with the rare
/// taken test, interleaved properties.
#[test]
fn f0_rare() {
    let mut t = tables268();
    let ring = push_item(&mut t, item_rec(RING, b"rin "));
    let jewel = push_item(&mut t, item_rec(ty::JEWL, b"jew "));
    let mut sfx = affix_row(ty::MISC, 1);
    sfx.mods[0] = rec(0, 0, 3, 3);
    let mut pfx = affix_row(ty::MISC, 2);
    pfx.mods[0] = rec(0, 0, 4, 4);
    t.magic = vec![sfx, pfx];
    t.n_suffix = 1;
    t.n_prefix = 1;
    rare_rows(&mut t, ty::MISC, 1);
    let prefs = ItemRequest {
        prefix: [1, 1, 1],
        suffix: [1, 1, 1],
        ..Default::default()
    };
    for seed in 0..30u32 {
        for (i, is_jewel) in [(ring, false), (jewel, true)] {
            let (s, filled) = rare_mirror(seed, is_jewel, [true, true]);
            let mut it = f0(i, seed);
            it.flags = flag::IDENTIFIED;
            assert!(rare(&t, &mut it, &prefs));
            assert_eq!(it.item_seed, s, "seed {seed}");
            assert_eq!((it.rare_prefix, it.rare_suffix), (2, 1));
            assert_eq!(it.prefix, [if filled[0] { 2 } else { 0 }, 0, 0]);
            assert_eq!(it.suffix, [if filled[1] { 1 } else { 0 }, 0, 0]);
            assert_eq!(it.flags & flag::IDENTIFIED, 0);
            let want = if filled[0] { 4 } else { 0 } + if filled[1] { 3 } else { 0 };
            assert_eq!(it.stats.item_list(stat::ARMORCLASS, 0), want);
        }
    }
    // A kind without a fitting row: its passes end after the coin, the
    // kind is not closed.
    let mut t2 = t.clone();
    t2.magic[0].itype = [AXE as i16, 0, 0, 0, 0, 0, 0];
    for seed in 0..30u32 {
        let (s, filled) = rare_mirror(seed, false, [true, false]);
        let mut it = f0(ring, seed);
        assert_eq!(rare(&t2, &mut it, &prefs), filled[0]);
        assert_eq!(it.item_seed, s, "seed {seed}");
    }
    // No affix at all → 0.
    t2.magic[1].itype = [AXE as i16, 0, 0, 0, 0, 0, 0];
    let mut it = f0(ring, 3);
    assert!(!rare(&t2, &mut it, &prefs));
    // No rare suffix name → 0 after the prefix name's draw.
    let mut t3 = t.clone();
    t3.rare[0].itype = [AXE as i16, 0, 0, 0, 0, 0, 0];
    let mut it = f0(ring, 3);
    assert!(!rare(&t3, &mut it, &prefs));
    assert_eq!(it.item_seed, after(3, 1));
}

/// Crafted with the legacy roller: frequency-0 rows (never rolled
/// by the current roller) are rolled for format 0.
#[test]
fn f0_crafted_uses_the_legacy_roller() {
    let mut t = tables268();
    let ring = push_item(&mut t, item_rec(RING, b"rin "));
    let mut a = affix_row(RING, 1);
    a.frequency = 0;
    let mut b = affix_row(RING, 2);
    b.frequency = 0;
    t.magic = vec![a, b];
    t.n_suffix = 1;
    t.n_prefix = 1;
    rare_rows(&mut t, RING, 1);
    let rq = ItemRequest {
        ilvl: 80,
        ..Default::default()
    };
    for seed in 0..10 {
        let mut it = f0(ring, seed);
        it.quality = q::CRAFTED;
        assert_eq!(crafted(&t, &mut it, &rq), Ok(true));
        assert!(it.prefix[0] != 0 || it.suffix[0] != 0, "seed {seed}");
        let mut it = item(ring, seed);
        it.quality = q::CRAFTED;
        assert_eq!(crafted(&t, &mut it, &rq), Ok(true));
        assert_eq!((it.prefix, it.suffix), ([0; 3], [0; 3]));
    }
}

// --------------------------------------------------------- generation

/// One branch by the primary type; any other type: class skill
/// mods then the socket roll.: the dispatch draws the roll with a
/// request quality, runs the socket roll again, no ethereal, no
/// automagic.
#[test]
fn f0_normal_routine() {
    let mut t = tables268();
    let play = push_item(&mut t, item_rec(ty::PLAY, b"ear "));
    let body = push_item(&mut t, item_rec(ty::BODY, b"hrt "));
    let book = push_item(&mut t, item_rec(ty::BOOK, b"tbk "));
    let scro = push_item(&mut t, item_rec(ty::SCRO, b"tsc "));
    let charm = push_item(&mut t, item_rec(ty::CHAR, b"cm1 "));
    let mut r = item_rec(RING, b"tbk ");
    r.type2 = ty::BOOK as i16;
    let ring_book = push_item(&mut t, r);
    let mut r = item_rec(AXE, b"axe ");
    (r.hasinv, r.gemsockets, r.invwidth, r.invheight) = (1, 3, 2, 3);
    r.auto_prefix = 1;
    r.durability = 10;
    let axe = push_item(&mut t, r);
    t.itemtypes[AXE as usize].maxsock1 = 3;
    t.itemtypes[AXE as usize].maxsock25 = 3;
    t.itemtypes[AXE as usize].maxsock40 = 3;
    t.books = vec![(*b"isc ", *b"ibk "), (*b"tsc ", *b"tbk ")];
    t.magic = vec![affix_row(ty::CHAR, 1), affix_row(ty::CHAR, 2)];
    t.n_suffix = 1;
    t.n_prefix = 1;
    let g = FakeGame::default();
    let unit = ItemRequest {
        unit: Some(RequestUnit {
            class: 3,
            player: None,
        }),
        index: 9,
        ..Default::default()
    };
    let index = ItemRequest {
        index: 9,
        ..Default::default()
    };
    // `play`: class or index, flag 0x10000.
    let mut it = f0(play, 1);
    normal_legacy(&t, &g, &mut it, &unit).unwrap();
    assert_eq!((it.file_index, it.flags & flag::EAR), (3, flag::EAR));
    let mut it = f0(play, 1);
    normal_legacy(&t, &g, &mut it, &index).unwrap();
    assert_eq!(it.file_index, 9);
    // `body`: the same, no flag.
    let mut it = f0(body, 1);
    normal_legacy(&t, &g, &mut it, &unit).unwrap();
    assert_eq!((it.file_index, it.flags & flag::EAR), (3, 0));
    // Books rows.
    let mut it = f0(book, 1);
    normal_legacy(&t, &g, &mut it, &index).unwrap();
    assert_eq!(it.suffix[0], 1);
    let mut it = f0(scro, 1);
    normal_legacy(&t, &g, &mut it, &index).unwrap();
    assert_eq!(it.suffix[0], 1);
    // The primary type only: a ring with `type2` book is "other" (format
    // ≥ 1 would store a books row).
    let mut it = f0(ring_book, 1);
    normal_legacy(&t, &g, &mut it, &index).unwrap();
    assert_eq!(it.suffix[0], 0);
    let mut it = item(ring_book, 1);
    normal(&t, &mut it, &index).unwrap();
    assert_eq!(it.suffix[0], 1);
    // Charm affixes.
    let mut it = f0(charm, 1);
    normal_legacy(&t, &g, &mut it, &index).unwrap();
    assert!(it.prefix[0] != 0 || it.suffix[0] != 0);
    // Other: the socket roll (class skill mods off: its one draw).
    let socket = ItemRequest {
        flags2: req::ALWAYS_SOCKETS | req::ALWAYS_ETHEREAL,
        ..Default::default()
    };
    let mut it = f0(axe, 5);
    it.quality = q::NORMAL;
    normal_legacy(&t, &g, &mut it, &socket).unwrap();
    assert_eq!(it.item_seed, after(5, 1));
    assert_eq!(it.stats.base(stat::NUMSOCKETS, 0), (5 % 3) + 1);
    // Through the dispatch: the quality roll (one draw although the
    // request has quality 2), the routine's socket roll and the finishing
    // one; no ethereal roll, no automagic.
    t.itemratio = vec![ratio0([(0, 1); 6])];
    let mut auto = affix_row(AXE, 1);
    auto.mods[0] = rec(0, 0, 1, 1);
    t.magic.push(auto);
    let mut it = f0(axe, 5);
    let mut rq = ItemRequest {
        quality: q::NORMAL,
        ..socket.clone()
    };
    assert_eq!(
        dispatch(&t, &mut FakeGame::default(), &mut it, &mut rq),
        Ok(true)
    );
    assert_eq!(it.quality, q::NORMAL);
    assert_eq!(it.item_seed, after(5, 3));
    assert_eq!((it.auto_affix, it.flags & flag::ETHEREAL), (0, 0));
}

/// The legacy skill-mod draws, in the roller's order: (seed after, mods).
fn skill_mirror(seed: u32, ilvl: i32, first: u16) -> (Seed, Vec<(u16, i32)>) {
    let mut s = Seed::init_low(seed);
    let pct = |s: &mut Seed| (s.step() % 100) as i32;
    let p = pct(&mut s);
    let n = match p {
        91.. => 3,
        71.. => 2,
        31.. => 1,
        _ => return (s, vec![]),
    };
    let tier = match ilvl {
        25.. => 4,
        19.. => 3,
        12.. => 2,
        _ => 1,
    };
    let mut out: Vec<(u16, i32)> = Vec::new();
    for _ in 0..n {
        let p = pct(&mut s);
        let tr: i32 = match p {
            81.. => tier + 1,
            31.. => tier,
            11.. => tier - 1,
            _ => tier - 2,
        }
        .max(1);
        let skill = loop {
            let k = first + 5 * (tr as u16 - 1) + (s.step() % 5) as u16;
            if k != 73 && out.iter().all(|&(c, _)| c != k) {
                break k;
            }
        };
        let v = match pct(&mut s) {
            90.. => 3,
            60.. => 2,
            _ => 1,
        };
        out.push((skill, v));
    }
    (s, out)
}

/// Counts and tiers without the request bonus, no tier 5, skill
/// 73 and repeats redrawn without a limit, no `itypea1` test.
#[test]
fn f0_class_skill_mods() {
    let mut t = tables();
    let axe = push_item(&mut t, item_rec(AXE, b"axe "));
    // Tier-3 skills are 73–77 (73 is never taken).
    staffmods_on(&mut t, AXE, 63);
    // `itypea1` = ring: format ≥ 1 rejects every skill, format 0 does not
    // test it.
    t.skills = vec![
        SkillRec {
            itypea1: RING as i16,
            ..Default::default()
        };
        120
    ];
    for ilvl in [5, 12, 19, 24, 40, 99] {
        let rq = ItemRequest {
            ilvl,
            flags2: req::STAFFMODS_ILVL,
            ..Default::default()
        };
        for seed in 0..120u32 {
            let (s, mods) = skill_mirror(seed, ilvl, 63);
            let mut it = f0(axe, seed);
            it.quality = q::LOW;
            class_skill_mods(&t, &mut it, &rq);
            assert_eq!(it.item_seed, s, "ilvl {ilvl} seed {seed}");
            let got: Vec<(u16, i32)> = it
                .stats
                .lists
                .get(&ListKey::ITEM)
                .map(|l| l.iter().map(|(&(_, k), &v)| (k, v)).collect())
                .unwrap_or_default();
            let mut want = mods.clone();
            want.sort();
            assert_eq!(got, want, "ilvl {ilvl} seed {seed}");
            assert!(got.iter().all(|&(k, _)| k != 73 && k < 63 + 25));
        }
    }
    // No request bonus: a first pct < 31 gives no mod at ilvl 99 with
    // flags2 0x20 (format ≥ 1: at least one).
    let low = find_seed(|s| s.step() % 100 < 31);
    let rq = ItemRequest {
        ilvl: 99,
        flags2: req::STAFFMODS_ILVL,
        ..Default::default()
    };
    let mut it = f0(axe, low);
    class_skill_mods(&t, &mut it, &rq);
    assert_eq!(it.item_seed, after(low, 1));
    assert!(it.stats.lists.is_empty());
}

// ------------------------------------------------------- properties

/// The legacy table, the helpers, every function kind, the crash
/// codes and the routing (every mode but 6).
#[test]
fn f0_legacy_property_table() {
    use LegacyFn::*;
    // Every code 0–243 has an entry; spot checks against the legacy table.
    assert!(LEGACY.iter().all(|&(f, _)| f != Unset));
    assert_eq!(LEGACY_COUNT, 244);
    for (code, want) in [
        (0, (D1C0, 31)),
        (4, (D1C0, 36)),
        (6, (D1C0, 35)),
        (17, (D1C0, 54)),
        (43, (D1C0, 142)),
        (50, (D1C0, 149)),
        (106, (D1C0, 134)),
        (120, (D1C0, 158)),
        (181, (D1C0, 254)),
        (5, (D2B0, 16)),
        (99, (D2B0, 122)),
        (104, (DB90, 127)),
        (57, (E450, 93)),
        (87, (E450, 105)),
        (177, (D110, 250)),
        (180, (D110, 253)),
        (230, (DD80, 303)),
        (71, (E2D0, 87)),
        (122, (E2D0, 180)),
        (138, (E070, 57)),
        (139, (DE40, 159)),
        (140, (DE40, 21)),
        (133, (D220, 194)),
        (243, (DBC0, 204)),
        (241, (E440, 0)),
    ] {
        assert_eq!(LEGACY[code], want, "code {code}");
    }

    let mut t = tables268();
    t.properties[0] = prop1(1, 50); // the §3 function 1 → stat 50
    t.skills = vec![SkillRec::default(); 10];
    let ring = push_item(&mut t, item_rec(RING, b"rin "));
    let mut r = item_rec(ty::HELM, b"cap ");
    r.maxac = 10;
    let helm = push_item(&mut t, r);
    let mut r = item_rec(AXE, b"axe ");
    (
        r.mindam,
        r.maxdam,
        r.hasinv,
        r.gemsockets,
        r.invwidth,
        r.invheight,
    ) = (3, 6, 1, 3, 2, 3);
    let axe = push_item(&mut t, r);
    t.itemtypes[AXE as usize].maxsock1 = 3;
    t.itemtypes[AXE as usize].maxsock25 = 3;
    t.itemtypes[AXE as usize].maxsock40 = 3;
    let mut r = item_rec(THROWN, b"tax ");
    (r.mindam, r.maxdam) = (1, 2);
    let tax = push_item(&mut t, r);
    let l = |it: &Item<FakeStats>, id| it.stats.item_list(id, 0);

    // Routing: code 0 is (D1C0, 31) for format 0, the dispatcher row for 101 and
    // for mode 6.
    let mut it = f0(ring, 1);
    ap(&t, &mut it, mode::AFFIX, rec(0, 0, 5, 5));
    assert_eq!((l(&it, 31), l(&it, 50)), (5, 0));
    let mut it = item(ring, 1);
    ap(&t, &mut it, mode::AFFIX, rec(0, 0, 5, 5));
    assert_eq!((l(&it, 31), l(&it, 50)), (0, 5));
    let mut it = f0(ring, 1);
    ap(&t, &mut it, mode::RUNEWORD, rec(0, 0, 5, 5));
    assert_eq!((l(&it, 31), l(&it, 50)), (0, 5));

    // R: format 0 never rolls max (n = hi − lo), one draw; swapped bounds.
    for seed in 0..10 {
        for (min, max) in [(5, 6), (6, 5)] {
            let mut it = f0(ring, seed);
            ap(&t, &mut it, 0, rec(0, 0, min, max));
            assert_eq!(l(&it, 31), 5);
            assert_eq!(it.item_seed, after(seed, 1));
        }
    }
    let mut it = f0(ring, 1);
    ap(&t, &mut it, 0, rec(0, 0, 9, 1));
    let mut s = Seed::init_low(1);
    assert_eq!(l(&it, 31), 1 + s.roll(8) as i32);
    // A: × 256 for stats 6–11 (code 11 → stat 9).
    let mut it = f0(ring, 1);
    ap(&t, &mut it, 0, rec(11, 0, 2, 2));
    assert_eq!(l(&it, 9), 512);
    // Base reset: kind 1 (code 5 → 16) in mode 0; kind 0 only in mode 1.
    let fresh_helm = || {
        let mut it = f0(helm, 1);
        it.stats.set_base(stat::ARMORCLASS, 0, 3);
        it
    };
    let mut it = fresh_helm();
    ap(&t, &mut it, 0, rec(5, 0, 20, 20));
    assert_eq!((it.stats.base(31, 0), l(&it, 16)), (11, 20));
    let mut it = fresh_helm();
    ap(&t, &mut it, 0, rec(0, 0, 1, 1));
    assert_eq!(it.stats.base(31, 0), 3);
    let mut it = fresh_helm();
    ap(&t, &mut it, mode::QUALITY, rec(0, 0, 1, 1));
    assert_eq!((it.stats.base(31, 0), l(&it, 31)), (11, 1));
    // Stat 58 (code 25) adds 1 to stat 326.
    let mut it = f0(ring, 1);
    ap(&t, &mut it, 0, rec(25, 0, 4, 4));
    assert_eq!((l(&it, 58), l(&it, 326)), (4, 1));
    // (code 56 → 93).
    let mut it = f0(ring, 1);
    ap(&t, &mut it, 0, rec(56, 0, 3, 3));
    assert_eq!(l(&it, 93), 3);
    // Param (code 141 → 214); param 0 → nothing.
    let mut it = f0(ring, 1);
    ap(&t, &mut it, 0, rec(141, 7, 1, 1));
    ap(&t, &mut it, 0, rec(142, 0, 1, 1));
    assert_eq!((l(&it, 214), l(&it, 215)), (7, 0));
    // Function 18's value, set; out of range → fatal.
    let mut it = f0(ring, 1);
    ap(&t, &mut it, 0, rec(195, 1, -50, 50));
    assert_eq!(l(&it, 268), 1254201);
    for (r, code) in [
        (rec(195, 1, 50, -50), 0x444),
        (rec(195, 4, 1, 2), 0x44C),
        // Unsigned compare: a negative param is fatal too, no stat set.
        (rec(195, -1, 1, 2), 0x44C),
        (rec(195, 1, -300, 2), 0x44D),
        (rec(195, 1, 1, 900), 0x44E),
    ] {
        let mut it = f0(ring, 1);
        ap(&t, &mut it, 0, r);
        assert_eq!(it.fatal, Some(Fatal::LegacyByTime(code)));
        assert_eq!(l(&it, 268), 0);
    }
    // param 3 still sets.
    let mut it = f0(ring, 1);
    ap(&t, &mut it, 0, rec(195, 3, 1, 2));
    assert_eq!(l(&it, 268), (((2 + 256) << 10 | (1 + 256)) * 4) + 3);
    // Stat 83 with layer 0–4, 5, 6.
    let mut it = f0(ring, 1);
    ap(&t, &mut it, 0, rec(69, 0, 4, 4));
    ap(&t, &mut it, 0, rec(121, 0, 5, 5));
    ap(&t, &mut it, 0, rec(122, 0, 6, 6));
    let ll = |it: &Item<FakeStats>, id, k| it.stats.item_list(id, k);
    assert_eq!((ll(&it, 83, 2), ll(&it, 83, 5), ll(&it, 83, 6)), (4, 5, 6));
    // (126 layer 1), (107 layer skill; outside
    // the skills → 0).
    let mut it = f0(ring, 1);
    ap(&t, &mut it, 0, rec(103, 0, 2, 2));
    ap(&t, &mut it, 0, rec(123, 5, 3, 3));
    ap(&t, &mut it, 0, rec(123, 99, 4, 4));
    assert_eq!(
        (ll(&it, 126, 1), ll(&it, 107, 5), ll(&it, 107, 0)),
        (2, 3, 4)
    );
    // S, s + 1, s + 2 := min, max, param; 57 → 326 + 1.
    let mut it = f0(ring, 1);
    ap(&t, &mut it, 0, rec(137, 3, 1, 2));
    assert_eq!((l(&it, 54), l(&it, 55), l(&it, 56)), (1, 2, 3));
    ap(&t, &mut it, 0, rec(138, 3, 1, 2));
    assert_eq!(
        (l(&it, 57), l(&it, 58), l(&it, 59), l(&it, 326)),
        (1, 2, 3, 1)
    );
    // S, s + 1 := min, max; 21: the damage pairs.
    let mut it = f0(ring, 1);
    ap(&t, &mut it, 0, rec(134, 0, 4, 9));
    assert_eq!((l(&it, 48), l(&it, 49)), (4, 9));
    let mut it = f0(tax, 1);
    ap(&t, &mut it, 0, rec(140, 0, 2, 5));
    let got: Vec<i32> = [21, 22, 23, 24, 159, 160]
        .iter()
        .map(|&s| l(&it, s))
        .collect();
    // A throwing weapon with `2handmaxdam` 0 and `mindam` ≠ 0: no 23/24.
    assert_eq!(got, [2, 5, 0, 0, 2, 5]);
    // (min damage) on a non-throwing weapon: 21 only;
    // (max) on a non-weapon: 22, 24, 160.
    let mut it = f0(axe, 1);
    ap(&t, &mut it, 0, rec(27, 0, 3, 3));
    let got: Vec<i32> = [21, 23, 159].iter().map(|&s| l(&it, s)).collect();
    assert_eq!(got, [3, 0, 0]);
    let mut it = f0(ring, 1);
    ap(&t, &mut it, 0, rec(28, 0, 3, 3));
    let got: Vec<i32> = [22, 24, 160].iter().map(|&s| l(&it, s)).collect();
    assert_eq!(got, [3, 3, 3]);
    // 17 and 18 with base reset,
    // Four resistances each.
    let mut it = f0(axe, 1);
    ap(&t, &mut it, 0, rec(29, 0, 10, 10));
    assert_eq!((l(&it, 17), l(&it, 18)), (10, 10));
    assert_eq!((it.stats.base(21, 0), it.stats.base(22, 0)), (3, 6));
    let mut it = f0(ring, 1);
    ap(&t, &mut it, 0, rec(41, 0, 5, 5));
    ap(&t, &mut it, 0, rec(42, 0, 6, 6));
    let got: Vec<i32> = (39..=46).map(|s| l(&it, s)).collect();
    assert_eq!(got, [5, 6, 5, 6, 5, 6, 5, 6]);
    // Flag 0x800 and the capped count.
    let mut it = f0(axe, 1);
    ap(&t, &mut it, 0, rec(133, 9, 0, 0));
    assert_eq!(
        (it.flags & flag::SOCKETED, it.stats.base(194, 0)),
        (flag::SOCKETED, 3)
    );
    let mut it = f0(ring, 1);
    ap(&t, &mut it, 0, rec(133, 2, 0, 0));
    assert_eq!(
        (it.flags & flag::SOCKETED, it.stats.base(194, 0)),
        (flag::SOCKETED, 0)
    );
    // Base 73 and 72 := 0.
    let mut it = f0(ring, 1);
    it.stats.set_base(72, 0, 5);
    it.stats.set_base(73, 0, 9);
    ap(&t, &mut it, 0, rec(242, 0, 0, 0));
    assert_eq!((it.stats.base(72, 0), it.stats.base(73, 0)), (0, 0));
    // Function 19's vector (min 0, level 1, {4242, 666}).
    let mut it = f0(ring, 4242);
    ap(&t, &mut it, 0, rec(243, 0, 0, 1));
    assert_eq!(ll(&it, 204, 1), 1283);
    // and the null codes 244, 257–261: nothing.
    let mut it = f0(ring, 1);
    for c in [124, 178, 244, 257, 261, 300] {
        ap(&t, &mut it, 0, rec(c, 1, 1, 1));
    }
    assert!(it.stats.lists.is_empty() && it.fatal.is_none());
    assert_eq!(it.item_seed, Seed::init_low(1));
    // Codes 245–256, 262–267: the crash; later records do nothing.
    for c in [245, 256, 262, 267] {
        let mut it = f0(ring, 1);
        ap(&t, &mut it, 0, rec(c, 0, 1, 1));
        ap(&t, &mut it, 0, rec(0, 0, 1, 1));
        assert_eq!(it.fatal, Some(Fatal::LegacyPropertyCrash(c)));
        assert!(it.stats.lists.is_empty());
    }
    // The dispatch returns it (a forced unique with a crash code).
    let mut u = UniqueRec {
        code: *b"rin ",
        enabled: true,
        props: [PropRec::NONE; 12],
        ..Default::default()
    };
    u.props[0] = rec(250, 0, 1, 1);
    t.uniques = vec![u];
    t.itemratio = vec![ratio0([(0, 1); 6])];
    let mut it = f0(ring, 1);
    let mut rq = ItemRequest {
        force: true,
        quality: q::UNIQUE,
        ..Default::default()
    };
    assert_eq!(
        dispatch(&t, &mut FakeGame::default(), &mut it, &mut rq),
        Err(Fatal::LegacyPropertyCrash(250))
    );
}

/// (code 242) acts on the extra unit when it is an item: a
/// format-0 rune's mode 5 zeroes the socketed
/// item's base 73 and 72, not the rune's; a gem (mode 2, no extra unit)
/// zeroes its own.
#[test]
fn f0_code_242_reaches_the_runes_socketed_item() {
    use crate::items::props::apply_socket_filler_into;
    use crate::items::tables::GemRec;
    let mut t = tables268();
    let mut g = item_rec(ty::RUNE, b"r01 ");
    g.gemoffset = 0;
    let rune = push_item(&mut t, g.clone());
    g.code = *b"gem ";
    g.type_ = ty::GEM as i16;
    let gem = push_item(&mut t, g);
    let block = [rec(242, 0, 0, 0), PropRec::NONE, PropRec::NONE];
    t.gems = vec![GemRec {
        mods: [block, block, block],
    }];
    let worn = |it: &mut Item<FakeStats>| {
        it.stats.set_base(72, 0, 5);
        it.stats.set_base(73, 0, 9);
    };
    let mut r = f0(rune, 1);
    worn(&mut r);
    let mut socketed = f0(rune, 2).stats;
    socketed.set_base(72, 0, 7);
    socketed.set_base(73, 0, 8);
    apply_socket_filler_into(&t, &mut r, 0, Some(&mut socketed));
    assert_eq!((socketed.base(72, 0), socketed.base(73, 0)), (0, 0));
    assert_eq!((r.stats.base(72, 0), r.stats.base(73, 0)), (5, 9));
    let mut gm = f0(gem, 1);
    worn(&mut gm);
    let mut other = f0(rune, 2).stats;
    other.set_base(72, 0, 7);
    apply_socket_filler_into(&t, &mut gm, 0, Some(&mut other));
    assert_eq!((gm.stats.base(72, 0), gm.stats.base(73, 0)), (0, 0));
    assert_eq!(other.base(72, 0), 7);
}

/// Mode 7 (craft list) goes through the wrapper, so a format-0 item
/// takes there; mode 6 calls the dispatcher directly.
#[test]
fn f0_craft_mode_goes_through_the_legacy_table() {
    let mut t = tables268();
    t.properties[0] = prop1(1, 50);
    let ring = push_item(&mut t, item_rec(RING, b"rin "));
    let l = |it: &Item<FakeStats>, id| it.stats.item_list(id, 0);
    let mut it = f0(ring, 1);
    ap(&t, &mut it, mode::CRAFT, rec(0, 0, 5, 5));
    assert_eq!((l(&it, 31), l(&it, 50)), (5, 0));
    let mut it = f0(ring, 1);
    ap(&t, &mut it, mode::RUNEWORD, rec(0, 0, 5, 5));
    assert_eq!((l(&it, 31), l(&it, 50)), (0, 5));
}

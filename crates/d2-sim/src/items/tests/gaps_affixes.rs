//! Gap tests for the affix rolls: one rule (or a few tightly linked rules)
//! per test, synthetic tables, exact draw counts through `Seed` compares.

use super::*;
use crate::items::affixes::{
    automagic, charm, crafted, magic, magic_fits, rare, rare_name, roll_affix, tempered,
    RARE_COUNTS,
};
use crate::items::create::class_skill_mods;
use crate::items::quality::dispatch;
use crate::items::tables::RareRec;
use crate::items::{flag, q, req, stat, ItemRequest};

/// `Seed::init_low(seed)` after `n` steps.
fn after(seed: u32, n: usize) -> Seed {
    let mut s = Seed::init_low(seed);
    for _ in 0..n {
        s.step();
    }
    s
}

/// Tables with one item of type `itype` and the magic array built from
/// the three parts (suffixes, prefixes, automagic).
fn parts(
    itype: u16,
    suffixes: Vec<AffixRec>,
    prefixes: Vec<AffixRec>,
    autos: Vec<AffixRec>,
) -> (ItemTables, usize) {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(itype, b"itm "));
    t.n_suffix = suffixes.len();
    t.n_prefix = prefixes.len();
    t.magic = suffixes;
    t.magic.extend(prefixes);
    t.magic.extend(autos);
    (t, i)
}

/// `n` fitting rows of type `itype` with groups `g0`, `g0 + 1`, …
fn rows(itype: u16, n: i32, g0: i32) -> Vec<AffixRec> {
    (0..n).map(|k| affix_row(itype, g0 + k)).collect()
}

/// Rare rows fitting `itype`.
fn rare_rows(itype: u16, n: usize) -> Vec<RareRec> {
    (0..n)
        .map(|_| RareRec {
            itype: [itype as i16, 0, 0, 0, 0, 0, 0],
            ..Default::default()
        })
        .collect()
}

/// Turns on class skill mods for `itype` (class 1, 30 skills from 36).
fn staffmods_on(t: &mut ItemTables, itype: u16) {
    t.itemtypes[itype as usize].staffmods = 1;
    t.skill_lists.counts[1] = 30;
    t.skill_lists.max = 30;
    t.skill_lists.lists = vec![0; 7 * 30];
    t.skill_lists.lists[30] = 36;
}

/// The roller's common call: suffix part, force, no assign.
fn pick(t: &ItemTables, it: &mut Item<FakeStats>, spawnable: bool, pref: i32, group: i32) -> u16 {
    roll_affix(t, it, spawnable, true, false, false, pref, group)
}

// ---------------------------------------------------------------

/// Three prefix slots, three suffix slots (magic ids of their parts), a
/// rare prefix and a rare suffix (rare ids, suffixes first) and the auto
/// affix (a magic id of the automagic part).
#[test]
fn slots_hold_six_affixes_rare_names_and_auto() {
    let mut auto = affix_row(RING, 50);
    auto.spawnable = 0;
    let (mut t, i) = parts(RING, rows(RING, 6, 1), rows(RING, 6, 11), vec![auto]);
    t.items[i].auto_prefix = 50;
    t.rare = rare_rows(RING, 4);
    t.n_rare_suffix = 2;
    let seed = find_seed(|s| {
        s.roll(2);
        s.roll(2);
        RARE_COUNTS[(s.step() & 7) as usize] == 6
    });
    let mut it = item(i, seed);
    it.quality = q::RARE;
    assert!(rare(&t, &mut it, &ItemRequest::default()));
    assert!(it.prefix.iter().all(|x| (7..=12).contains(x)), "{:?}", it);
    assert!(it.suffix.iter().all(|x| (1..=6).contains(x)), "{:?}", it);
    assert!((3..=4).contains(&it.rare_prefix));
    assert!((1..=2).contains(&it.rare_suffix));
    automagic(&t, &mut it);
    assert_eq!(it.auto_affix, 13);
}

/// "Index in part" and the preferred affix p: p − 1 indexes the part
/// the roller works on, not the combined array.
#[test]
fn preferred_is_index_in_part() {
    let high = |g| {
        let mut r = affix_row(RING, g);
        r.level = 99; // only a preferred row passes the level test
        r
    };
    let (t, i) = parts(
        RING,
        vec![high(1), high(2), high(3)],
        vec![high(11), high(12), high(13)],
        vec![high(21), high(22)],
    );
    let mut it = item(i, 6);
    assert_eq!(roll_affix(&t, &mut it, true, true, false, true, 2, 0), 5);
    assert_eq!(roll_affix(&t, &mut it, true, true, false, false, 2, 0), 2);
    assert_eq!(roll_affix(&t, &mut it, true, true, false, true, 3, 0), 6);
    assert_eq!(roll_affix(&t, &mut it, true, true, false, false, 2, 22), 8);
    // Each call: the coin only, no pick draw.
    assert_eq!(it.item_seed, after(6, 4));
}

// ---------------------------------------------------------------

/// The roller's arguments (text) and the part they select.
#[test]
fn roller_arguments_and_part() {
    let mut s = affix_row(RING, 1);
    s.spawnable = 0;
    let mut p = affix_row(RING, 11);
    p.mods[0] = rec(0, 0, 3, 3);
    let a = affix_row(RING, 21);
    let (mut t, i) = parts(RING, vec![s], vec![p], vec![a]);
    t.properties = vec![prop1(1, stat::TOBLOCK)];
    let odd = find_seed(|s| s.step() & 1 == 1);
    let even = find_seed(|s| s.step() & 1 == 0);
    let call = |seed, spawn, force, assign, prefix, pref, group| {
        let mut it = item(i, seed);
        let id = roll_affix(&t, &mut it, spawn, force, assign, prefix, pref, group);
        (id, it.stats.item_list(stat::TOBLOCK, 0))
    };
    // Part: g ≠ 0 → automagic even with prefix set; prefix; suffix.
    assert_eq!(call(odd, false, true, false, true, 0, 21).0, 3);
    assert_eq!(call(odd, false, true, false, true, 0, 0).0, 2);
    assert_eq!(call(odd, false, true, false, false, 0, 0).0, 1);
    // require-spawnable drops the non-spawnable suffix.
    assert_eq!(call(odd, true, true, false, false, 0, 0).0, 0);
    // force: an even coin returns 0 only without force.
    assert_eq!(call(even, false, false, false, true, 0, 0).0, 0);
    assert_eq!(call(even, false, true, false, true, 0, 0).0, 2);
    // assign: the row's properties only when asked.
    assert_eq!(call(odd, false, true, true, true, 0, 0), (2, 3));
    assert_eq!(call(odd, false, true, false, true, 0, 0), (2, 0));
    // group: the automagic row of another group is not picked.
    assert_eq!(call(odd, false, true, false, false, 0, 22).0, 0);
}

/// Each of the nine filters, both ways; a preferred row returns at once
/// (assigning if asked); the weight sum and the 511 cap.
#[test]
fn roller_nine_filters() {
    let base = || affix_row(RING, 1);
    let run = |row: AffixRec, edit: &dyn Fn(&mut ItemTables, &mut Item<FakeStats>), spawn, pref| {
        let (mut t, i) = parts(RING, vec![row], vec![affix_row(RING, 30)], vec![]);
        let mut it = item(i, 3);
        it.quality = q::MAGIC;
        edit(&mut t, &mut it);
        pick(&t, &mut it, spawn, pref, 0)
    };
    let none = |_: &mut ItemTables, _: &mut Item<FakeStats>| {};
    // 1. spawnable.
    let mut r = base();
    r.spawnable = 0;
    assert_eq!(run(r.clone(), &none, true, 0), 0);
    assert_eq!(run(r, &none, false, 0), 1);
    // 2. version < 100 or format ≥ 100.
    let mut r = base();
    r.version = 100;
    assert_eq!(run(r.clone(), &none, true, 0), 1);
    assert_eq!(run(r, &|_, it| it.format = 99, true, 0), 0);
    let mut r = base();
    r.version = 99;
    assert_eq!(run(r, &|_, it| it.format = 99, true, 0), 1);
    // 3. alvl here is 50 (ilvl 50, qlvl 0).
    for (level, maxlevel, want) in [(50, 0, 1), (51, 0, 0), (1, 50, 1), (1, 49, 0)] {
        let mut r = base();
        r.level = level;
        r.maxlevel = maxlevel;
        assert_eq!(run(r, &none, true, 0), want, "level {level} max {maxlevel}");
    }
    let mut r = base();
    r.level = 99;
    assert_eq!(run(r.clone(), &none, true, 1), 1, "preferred skips level");
    assert_eq!(run(r, &none, true, 2), 0, "another row preferred");
    // 4. `rare` = 0 excluded on qualities 6, 8, 9 only.
    for (quality, want) in [
        (q::MAGIC, 1),
        (q::UNIQUE, 1),
        (q::RARE, 0),
        (q::CRAFTED, 0),
        (q::TEMPERED, 0),
    ] {
        let mut r = base();
        r.rare = 0;
        assert_eq!(run(r, &|_, it| it.quality = quality, true, 0), want);
    }
    // 5. fits.
    let mut r = base();
    r.itype[0] = AXE as i16;
    assert_eq!(run(r, &none, true, 0), 0);
    // 6. group g (automagic part).
    let (t, i) = parts(RING, vec![], vec![], vec![affix_row(RING, 5)]);
    assert_eq!(pick(&t, &mut item(i, 3), false, 0, 5), 1);
    assert_eq!(pick(&t, &mut item(i, 3), false, 0, 6), 0);
    // 7. frequency, also for a preferred row.
    let mut r = base();
    r.frequency = 0;
    assert_eq!(run(r.clone(), &none, true, 0), 0);
    assert_eq!(run(r, &none, true, 1), 0);
    // 8. classspecific: 0xFF, item class 7 (≥ 7 as 7) or equal.
    for (cs, class, want) in [(0xFF, 3, 1), (2, 3, 0), (2, 2, 1), (2, 7, 1), (2, 9, 1)] {
        let mut r = base();
        r.classspecific = cs;
        let set = move |t: &mut ItemTables, _: &mut Item<FakeStats>| {
            t.itemtypes[RING as usize].class = class;
        };
        assert_eq!(run(r, &set, true, 0), want, "cs {cs} class {class}");
    }
    // 9. group already on the item (prefix id 2 has group 30; row 1 too).
    let mut r = base();
    r.group = 30;
    assert_eq!(run(r.clone(), &|_, it| it.prefix[0] = 2, true, 0), 0);
    assert_eq!(run(r, &none, true, 0), 1);
    // Preferred: returned at once (coin only), assigned if asked.
    let mut r = base();
    r.mods[0] = rec(0, 0, 1, 100);
    let (mut t, i) = parts(RING, vec![affix_row(RING, 9), r], vec![], vec![]);
    t.properties = vec![prop1(1, stat::TOBLOCK)];
    let mut it = item(i, 3);
    assert_eq!(roll_affix(&t, &mut it, true, true, true, false, 2, 0), 2);
    let mut s = after(3, 1);
    assert_eq!(it.stats.item_list(stat::TOBLOCK, 0), s.roll(100) as i32 + 1);
    assert_eq!(it.item_seed, s);
    // Weights: `frequency` (no magic lvl); 3 and 5 → r = 2 first, 3 second.
    let mut a = base();
    a.frequency = 3;
    let mut b = affix_row(RING, 2);
    b.frequency = 5;
    let (t, i) = parts(RING, vec![a, b], vec![], vec![]);
    for (r, want) in [(2, 1), (3, 2), (8, 2)] {
        let seed = find_seed(|s| {
            s.step();
            s.roll(9) == r
        });
        assert_eq!(pick(&t, &mut item(i, seed), true, 0, 0), want, "r {r}");
    }
    // The cap: 512 rows, 511 listed, the 512th still weighs 1: r = 511
    // (of roll(513)) and r = 512 land past the list → the 511th.
    let (t, i) = parts(RING, rows(RING, 512, 1), vec![], vec![]);
    for r in [510, 511, 512] {
        let seed = find_seed(|s| {
            s.step();
            s.roll(513) == r
        });
        assert_eq!(pick(&t, &mut item(i, seed), true, 0, 0), 511, "r {r}");
    }
}

/// No candidate after the coin → 0, and no pick draw.
#[test]
fn roller_no_candidate() {
    let mut r = affix_row(RING, 1);
    r.level = 99;
    let (t, i) = parts(RING, vec![r], vec![], vec![]);
    let mut it = item(i, 11);
    assert_eq!(pick(&t, &mut it, true, 0, 0), 0);
    assert_eq!(it.item_seed, after(11, 1));
}

/// Assign: the picked row's properties (mode 0, item seed draws after the
/// pick); the id returned is combined index + 1.
#[test]
fn roller_assigns_properties() {
    let mut p = affix_row(RING, 11);
    p.mods[0] = rec(0, 0, 1, 1000);
    let (mut t, i) = parts(RING, rows(RING, 2, 1), vec![p], vec![]);
    t.properties = vec![prop1(1, stat::TOBLOCK)];
    let seed = find_seed(|s| s.step() & 1 == 1);
    let mut it = item(i, seed);
    assert_eq!(roll_affix(&t, &mut it, true, false, true, true, 0, 0), 3);
    let mut s = after(seed, 1);
    s.roll(2);
    let v = s.roll(1000) as i32 + 1;
    assert_eq!(it.stats.item_list(stat::TOBLOCK, 0), v);
    assert_eq!(it.item_seed, s);
    let mut it = item(i, seed);
    assert_eq!(roll_affix(&t, &mut it, true, false, false, true, 0, 0), 3);
    assert_eq!(it.stats.item_list(stat::TOBLOCK, 0), 0);
    assert_eq!(it.item_seed, after(seed, 2));
}

// ---------------------------------------------------------------

/// Format < 100: stackable or throwable items fit no magic affix.
#[test]
fn fits_classic_stackable_throwable() {
    let mut t = tables();
    let mut sr = item_rec(RING, b"stk ");
    sr.stackable = 1;
    let stack = push_item(&mut t, sr);
    let thrown = push_item(&mut t, item_rec(THROWN, b"tax "));
    let ring = push_item(&mut t, item_rec(RING, b"rin "));
    let ok = |i: usize, itype: u16, format: u16| {
        let mut it = item(i, 1);
        it.format = format;
        magic_fits(&t, &it, &affix_row(itype, 1))
    };
    assert!(!ok(stack, RING, 99));
    assert!(ok(stack, RING, 100));
    assert!(!ok(thrown, THROWN, 99));
    assert!(ok(thrown, THROWN, 100));
    assert!(ok(ring, RING, 99));
}

/// The socket clause: `mod1code` out of range or a stat-194 property
/// does not fit, unless the item is socketable with max sockets ≠ 0.
#[test]
fn fits_socket_clause() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.properties = vec![prop1(1, stat::NUMSOCKETS), prop1(1, stat::TOBLOCK)];
    let row = |code| {
        let mut r = affix_row(RING, 1);
        r.mods[0] = rec(code, 0, 1, 1);
        r
    };
    let fits = |t: &ItemTables, code| magic_fits(t, &item(i, 1), &row(code));
    assert!(!fits(&t, 0), "stat 194");
    assert!(!fits(&t, 2), "out of range");
    assert!(fits(&t, 1));
    assert!(fits(&t, -1));
    // Socketable but max sockets 0: still excluded.
    t.items[i].hasinv = 1;
    assert!(!fits(&t, 0));
    // Socketable with max sockets ≠ 0 (ilvl 50 → maxsock40): no test.
    t.items[i].gemsockets = 2;
    t.itemtypes[RING as usize].maxsock40 = 2;
    assert!(fits(&t, 0));
    assert!(fits(&t, 2));
}

/// `etype1`–`etype5` stop at the first < 1; a matching one excludes.
#[test]
fn fits_etype_scan() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    let fits = |etype: [i16; 5]| {
        let mut r = affix_row(RING, 1);
        r.etype = etype;
        magic_fits(&t, &item(i, 1), &r)
    };
    let ring = RING as i16;
    let misc = ty::MISC as i16;
    assert!(!fits([ring, 0, 0, 0, 0]));
    assert!(!fits([AXE as i16, 3, 4, 5, misc]), "5th etype, via parent");
    assert!(fits([0, ring, 0, 0, 0]), "stops at the first < 1");
    assert!(fits([AXE as i16, -1, ring, 0, 0]));
}

/// `itype1`–`itype7` stop at the first < 1; a match fits, else no fit.
#[test]
fn fits_itype_scan() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    let fits = |itype: [i16; 7]| {
        let mut r = affix_row(RING, 1);
        r.itype = itype;
        magic_fits(&t, &item(i, 1), &r)
    };
    let ring = RING as i16;
    assert!(fits([AXE as i16, 3, 4, 5, 6, 8, ring]), "7th itype");
    assert!(fits([ty::MISC as i16, 0, 0, 0, 0, 0, 0]), "parent type");
    assert!(!fits([0, ring, 0, 0, 0, 0, 0]), "stops at the first < 1");
    assert!(!fits([AXE as i16, 0, 0, 0, 0, 0, 0]));
}

/// Rare affix fits (through the rare name pick, one candidate row).
#[test]
fn rare_fits_rules() {
    let mut t = tables();
    let mut sr = item_rec(RING, b"stk ");
    sr.stackable = 1;
    let stack = push_item(&mut t, sr);
    let thrown = push_item(&mut t, item_rec(THROWN, b"tax "));
    let ring = push_item(&mut t, item_rec(RING, b"rin "));
    let name = |row: RareRec, i: usize, format: u16| {
        let mut t = t.clone();
        t.rare = vec![row];
        t.n_rare_suffix = 1;
        let mut it = item(i, 1);
        it.format = format;
        rare_name(&t, &mut it, false)
    };
    let row = |itype: u16| RareRec {
        itype: [itype as i16, 0, 0, 0, 0, 0, 0],
        ..Default::default()
    };
    assert_eq!(name(row(RING), stack, 99), 0);
    assert_eq!(name(row(RING), stack, 100), 1);
    assert_eq!(name(row(THROWN), thrown, 99), 0);
    assert_eq!(name(row(THROWN), thrown, 100), 1);
    let mut v = row(RING);
    v.version = 100;
    assert_eq!(name(v.clone(), ring, 99), 0);
    assert_eq!(name(v, ring, 100), 1);
    let mut e = row(RING);
    e.etype = [AXE as i16, 0, 0, RING as i16];
    assert_eq!(name(e.clone(), ring, 101), 1, "etype scan stops at 0");
    e.etype = [AXE as i16, 3, 5, RING as i16];
    assert_eq!(name(e, ring, 101), 0, "4th etype");
    let mut i7 = row(AXE);
    i7.itype = [AXE as i16, 3, 4, 5, 6, 8, RING as i16];
    assert_eq!(name(i7.clone(), ring, 101), 1, "7th itype");
    i7.itype = [AXE as i16, 0, 4, 5, 6, 8, RING as i16];
    assert_eq!(name(i7, ring, 101), 0, "itype scan stops at 0");
}

// ---------------------------------------------------------------

/// Rare name: fitting rows of the part in order, capped at 511;
/// r := roll(count), unweighted; rare ids count suffixes first.
#[test]
fn rare_name_pick() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    let mut rows = rare_rows(RING, 2);
    rows.extend(rare_rows(AXE, 1));
    rows.extend(rare_rows(RING, 2));
    t.rare = rows;
    t.n_rare_suffix = 2;
    for (r, want) in [(0, 4), (1, 5)] {
        let seed = find_seed(|s| s.roll(2) == r);
        let mut it = item(i, seed);
        assert_eq!(rare_name(&t, &mut it, true), want);
        assert_eq!(it.item_seed, after(seed, 1));
    }
    // No candidate: 0, no draw.
    t.rare = rare_rows(AXE, 3);
    let mut it = item(i, 4);
    assert_eq!(rare_name(&t, &mut it, true), 0);
    assert_eq!(it.item_seed, Seed::init_low(4));
    // 600 fitting suffixes: only the first 511 are candidates.
    t.rare = rare_rows(RING, 600);
    t.n_rare_suffix = 600;
    let seed = find_seed(|s| s.step() % 600 >= 511);
    let mut it = item(i, seed);
    let want = Seed::init_low(seed).roll(511) as u16 + 1;
    assert_eq!(rare_name(&t, &mut it, false), want);
}

// ---------------------------------------------------------------

/// Magic item: preferences from request slot 0; p < 0 skips the prefix
/// and forces the suffix; a 0 prefix forces the suffix; a prefix with
/// s < 0 skips the suffix; s > 0 forces the (preferred) suffix.
#[test]
fn magic_item_paths() {
    let (t, i) = parts(RING, rows(RING, 2, 1), rows(RING, 2, 11), vec![]);
    let go = |seed: u32, p: [i32; 3], s: [i32; 3]| {
        let mut it = item(i, seed);
        let rq = ItemRequest {
            prefix: p,
            suffix: s,
            ..Default::default()
        };
        let ok = magic(&t, &mut it, &rq);
        (ok, it.prefix[0], it.suffix[0], it.item_seed)
    };
    let even = find_seed(|s| s.step() & 1 == 0);
    // p < 0: no prefix coin; the suffix is forced on an even coin.
    let (ok, p, s, seed) = go(even, [-1, 0, 0], [0, 0, 0]);
    assert!(ok && p == 0 && s != 0);
    assert_eq!(seed, after(even, 2));
    // p = 0, even prefix coin: prefix 0, the suffix forced.
    let e_e = find_seed(|s| s.step() & 1 == 0 && s.step() & 1 == 0);
    let (ok, p, s, seed) = go(e_e, [0, 0, 0], [0, 0, 0]);
    assert!(ok && p == 0 && s != 0);
    assert_eq!(seed, after(e_e, 3));
    // p = 0, odd coin: a prefix; s < 0 skips the suffix (coin + pick).
    let odd = find_seed(|s| s.step() & 1 == 1);
    let (ok, p, s, seed) = go(odd, [0, 0, 0], [-1, 0, 0]);
    assert!(ok && (3..=4).contains(&p) && s == 0);
    assert_eq!(seed, after(odd, 2));
    // A prefix and s = 0: the suffix is not forced (even coin → none).
    let o_e = find_seed(|s| {
        s.step() & 1 == 1 && {
            s.step();
            s.step() & 1 == 0
        }
    });
    let (ok, p, s, seed) = go(o_e, [0, 0, 0], [0, 0, 0]);
    assert!(ok && p != 0 && s == 0);
    assert_eq!(seed, after(o_e, 3));
    // s > 0: forced preferred suffix (coin only) even after a prefix.
    let (ok, p, s, seed) = go(o_e, [0, 0, 0], [2, 0, 0]);
    assert!(ok && p != 0 && s == 2);
    assert_eq!(seed, after(o_e, 3));
    // p > 0: forced preferred prefix on an even coin, no pick draw.
    let (ok, p, s, _) = go(e_e, [2, 0, 0], [-1, 0, 0]);
    assert!(ok && p == 4 && s == 0);
    // Only slot 0 of the request counts.
    assert_eq!(go(e_e, [0, 2, 2], [0, 1, 1]), go(e_e, [0, 0, 0], [0, 0, 0]));
}

// ---------------------------------------------------------------

/// itemtype `rare` = 0 → failure before any draw or rare name.
#[test]
fn rare_needs_itemtype_rare() {
    let (mut t, i) = parts(RING, rows(RING, 2, 1), rows(RING, 2, 11), vec![]);
    t.rare = rare_rows(RING, 2);
    t.n_rare_suffix = 1;
    t.itemtypes[RING as usize].rare = 0;
    let mut it = item(i, 8);
    assert!(!rare(&t, &mut it, &ItemRequest::default()));
    assert_eq!(it.item_seed, Seed::init_low(8));
    assert_eq!((it.rare_prefix, it.rare_suffix), (0, 0));
}

// ---------------------------------------------------------------

fn crafted_tables(suffixes: Vec<AffixRec>, prefixes: Vec<AffixRec>) -> (ItemTables, usize) {
    let (mut t, i) = parts(RING, suffixes, prefixes, vec![]);
    t.rare = rare_rows(RING, 2);
    t.n_rare_suffix = 1;
    (t, i)
}

/// Both rare names are drawn and both are required.
#[test]
fn crafted_rare_names() {
    let (mut t, i) = crafted_tables(vec![], vec![]);
    let mut it = item(i, 2);
    assert_eq!(crafted(&t, &mut it, &ItemRequest::default()), Ok(true));
    assert_eq!((it.rare_prefix, it.rare_suffix), (2, 1));
    // No rare prefix row: the suffix name is still drawn, then failure.
    t.rare = rare_rows(RING, 1);
    let mut it = item(i, 2);
    assert_eq!(crafted(&t, &mut it, &ItemRequest::default()), Ok(false));
    assert_eq!((it.rare_prefix, it.rare_suffix), (0, 0));
    assert_eq!(it.item_seed, after(2, 1));
    // No rare suffix row: failure after the prefix name's draw.
    t.rare = rare_rows(RING, 1);
    t.n_rare_suffix = 0;
    let mut it = item(i, 2);
    assert_eq!(crafted(&t, &mut it, &ItemRequest::default()), Ok(false));
    assert_eq!((it.rare_prefix, it.rare_suffix), (0, 0));
    assert_eq!(it.item_seed, after(2, 1));
}

/// The affix loop: a kind step per affix; 252 tries when the roll is
/// taken (same id or group in a filled slot of the kind) and then slot P
/// := 0 without advancing; the request's preference of slot P (S).
#[test]
fn crafted_affix_loop() {
    // (a) Taken 252 times, P not advanced. Prefix rows A, A2: group 1,
    // level 99 (only preferred). Request prefix [0, 1, 2]; n = 4.
    let high = || {
        let mut r = affix_row(RING, 1);
        r.level = 99;
        r
    };
    let (t, i) = crafted_tables(vec![], vec![high(), high()]);
    let seed = find_seed(|s| {
        s.roll(1);
        s.roll(1);
        if s.step() % 5 != 4 {
            return false;
        }
        for _ in 0..2 {
            if s.step() & 1 != 0 {
                return false;
            }
            s.step();
        }
        if s.step() & 1 != 0 {
            return false;
        }
        for _ in 0..252 {
            s.step();
        }
        s.step() & 1 == 0
    });
    let rq = ItemRequest {
        ilvl: 40,
        prefix: [0, 1, 2],
        ..Default::default()
    };
    let mut it = item(i, seed);
    assert_eq!(crafted(&t, &mut it, &rq), Ok(true));
    // 1st: no candidate, nothing filled → 0 stored, P = 1. 2nd: A at
    // slot 1. 3rd and 4th: A2 (A's group) taken 252 times each.
    assert_eq!(it.prefix, [0, 1, 0]);
    assert_eq!(it.suffix, [0, 0, 0]);
    assert_eq!(it.item_seed, after(seed, 2 + 1 + 2 + 2 + 253 + 253));

    // (b) An odd kind step picks the suffix, an even one the prefix.
    let (t, i) = crafted_tables(vec![affix_row(RING, 1)], vec![]);
    let rq = ItemRequest {
        ilvl: 1,
        ..Default::default()
    };
    let mut seen = [false; 2];
    for seed in 0..60 {
        let mut s = Seed::init_low(seed);
        s.roll(1);
        s.roll(1);
        if s.step() % 5 > 1 {
            continue; // n = 1 only
        }
        let odd = s.step() & 1 == 1;
        seen[usize::from(odd)] = true;
        let mut it = item(i, seed);
        assert_eq!(crafted(&t, &mut it, &rq), Ok(true));
        assert_eq!(it.suffix[0], u16::from(odd), "seed {seed}");
    }
    assert_eq!(seen, [true, true]);

    // (c) P = 3 forces the suffix whatever the step; (d) S = 3 forces
    // the prefix unless P = 3.
    for three_prefixes in [true, false] {
        let (t, i) = if three_prefixes {
            crafted_tables(vec![affix_row(RING, 1)], rows(RING, 3, 11))
        } else {
            crafted_tables(rows(RING, 3, 1), vec![affix_row(RING, 11)])
        };
        let want = u32::from(!three_prefixes);
        let seed = find_seed(|s| {
            s.roll(1);
            s.roll(1);
            s.step();
            for k in 0..3 {
                if s.step() & 1 != want {
                    return false;
                }
                s.step();
                s.roll(4 - k);
            }
            s.step() & 1 == want
        });
        let rq = ItemRequest {
            ilvl: 80,
            ..Default::default()
        };
        let mut it = item(i, seed);
        assert_eq!(crafted(&t, &mut it, &rq), Ok(true));
        if three_prefixes {
            assert!(it.prefix.iter().all(|&x| x != 0));
            assert_eq!(it.suffix, [1, 0, 0]);
        } else {
            assert!(it.suffix.iter().all(|&x| x != 0));
            assert_eq!(it.prefix, [4, 0, 0]);
        }
    }
}

/// After the loop: identified cleared, properties of P0 S0 P1 S1 P2 S2,
/// then class skill mods; success also without any affix.
#[test]
fn crafted_finish() {
    let mk = |ranged: bool| {
        let mut sfx = rows(RING, 6, 1);
        let mut pfx = rows(RING, 6, 11);
        for (k, r) in sfx.iter_mut().chain(pfx.iter_mut()).enumerate() {
            let max = if ranged { 1000 } else { 1 };
            r.mods[0] = rec(k as i32, 0, 1, max);
        }
        let (mut t, i) = crafted_tables(sfx, pfx);
        t.properties = (0..12).map(|k| prop1(1, 100 + k)).collect();
        (t, i)
    };
    let (flat, i) = mk(false);
    let (ranged, _) = mk(true);
    // flags2 0x20 adds the ilvl to the class skill chance: 3 mods.
    let rq = ItemRequest {
        ilvl: 80,
        flags2: req::STAFFMODS_ILVL,
        ..Default::default()
    };
    let run = |t: &ItemTables, seed| {
        let mut it = item(i, seed);
        it.flags = flag::IDENTIFIED;
        assert_eq!(crafted(t, &mut it, &rq), Ok(true));
        it
    };
    let seed = (0..)
        .find(|&s| {
            let it = run(&flat, s);
            it.prefix[1] != 0 && it.suffix[1] != 0
        })
        .unwrap();
    // Selection does not depend on the mods' ranges: the flat run's
    // final seed is the ranged run's seed before its properties.
    let a = run(&flat, seed);
    let b = run(&ranged, seed);
    assert_eq!((a.prefix, a.suffix), (b.prefix, b.suffix));
    assert_eq!(b.flags & flag::IDENTIFIED, 0);
    let order = |ids: Vec<u16>| {
        let mut s = a.item_seed;
        ids.into_iter()
            .filter(|&id| id != 0)
            .map(|id| (100 + id - 1, s.roll(1000) as i32 + 1))
            .collect::<Vec<_>>()
    };
    let (p, s) = (b.prefix, b.suffix);
    let want = order(vec![p[0], s[0], p[1], s[1], p[2], s[2]]);
    let other = order(vec![p[0], p[1], p[2], s[0], s[1], s[2]]);
    assert_ne!(want, other, "the test separates the orders");
    for (st, v) in want {
        assert_eq!(b.stats.item_list(st, 0), v, "stat {st}");
    }
    // Class skill mods run after the properties.
    let mut on = ranged.clone();
    staffmods_on(&mut on, RING);
    let c = run(&on, seed);
    let mut d = b.clone();
    class_skill_mods(&on, &mut d, &rq);
    assert_eq!(c, d);
    assert!(c.stats.lists[&ListKey::ITEM]
        .keys()
        .any(|&(id, _)| id == stat::ITEM_SINGLESKILL));
    // No affix rows: success with empty slots.
    let (t, i) = crafted_tables(vec![], vec![]);
    let mut it = item(i, seed);
    it.flags = flag::IDENTIFIED;
    assert_eq!(crafted(&t, &mut it, &rq), Ok(true));
    assert_eq!((it.prefix, it.suffix), ([0; 3], [0; 3]));
    assert_eq!(it.flags & flag::IDENTIFIED, 0);
}

// ---------------------------------------------------------------

/// Tempered: two rare names, no affixes, no properties; a missing name
/// downgrades to normal (dispatch case 9).
#[test]
fn tempered_names_only() {
    let mut r = affix_row(RING, 1);
    r.mods[0] = rec(0, 0, 5, 5);
    let (mut t, i) = parts(RING, vec![r.clone()], vec![r], vec![]);
    t.properties = vec![prop1(1, stat::TOBLOCK)];
    t.rare = rare_rows(RING, 2);
    t.n_rare_suffix = 1;
    let mut it = item(i, 5);
    assert!(tempered(&t, &mut it));
    assert_eq!((it.rare_prefix, it.rare_suffix), (2, 1));
    assert_eq!(it.item_seed, after(5, 2));
    let mut it = item(i, 5);
    let mut rq = ItemRequest {
        quality: q::TEMPERED,
        ..Default::default()
    };
    assert_eq!(
        dispatch(&t, &mut FakeGame::default(), &mut it, &mut rq),
        Ok(true)
    );
    assert_eq!(it.quality, q::TEMPERED);
    assert_eq!((it.rare_prefix, it.rare_suffix), (2, 1));
    assert_eq!((it.prefix, it.suffix), ([0; 3], [0; 3]));
    assert!(!it.stats.any, "no properties");
    // No rare prefix row → normal (magic rows exist but are not tried).
    t.rare = rare_rows(RING, 1);
    let mut it = item(i, 5);
    let mut rq = ItemRequest {
        quality: q::TEMPERED,
        ..Default::default()
    };
    dispatch(&t, &mut FakeGame::default(), &mut it, &mut rq).unwrap();
    assert_eq!(it.quality, q::NORMAL);
    assert_eq!((it.prefix[0], it.suffix[0]), (0, 0));
}

// ----------------------------------------------------------------

fn charm_tables() -> (ItemTables, usize) {
    parts(
        ty::CHAR,
        rows(ty::CHAR, 2, 1),
        rows(ty::CHAR, 2, 11),
        vec![],
    )
}

fn charm_run(t: &ItemTables, i: usize, seed: u32, p: i32, s: i32) -> Item<FakeStats> {
    let mut it = item(i, seed);
    it.flags = flag::IDENTIFIED;
    let rq = ItemRequest {
        prefix: [p, 0, 0],
        suffix: [s, 0, 0],
        ..Default::default()
    };
    charm(t, &mut it, &rq).expect("an affix");
    it
}

/// Charm: p, s from request slot 0; the prefix roll (forced and
/// preferred when p > 0, skipped when s > 0 ≥ p); the suffix roll
/// (forced and preferred when s > 0, forced after a 0 prefix, skipped
/// when p > 0 ≥ s); identified cleared; no class skill mods.
#[test]
fn charm_paths() {
    let (mut t, i) = charm_tables();
    staffmods_on(&mut t, ty::CHAR);
    let even = find_seed(|s| s.step() & 1 == 0);
    let o_e = find_seed(|s| {
        s.step() & 1 == 1 && {
            s.step();
            s.step() & 1 == 0
        }
    });
    let o_o = find_seed(|s| {
        s.step() & 1 == 1 && {
            s.step();
            s.step() & 1 == 1
        }
    });
    // p = s = 0, even coin: prefix 0 → suffix forced.
    let it = charm_run(&t, i, even, 0, 0);
    assert_eq!(it.prefix[0], 0);
    assert_ne!(it.suffix[0], 0);
    assert_eq!(it.item_seed, after(even, 3));
    // Odd prefix coin: a prefix; the suffix is not forced.
    let it = charm_run(&t, i, o_e, 0, 0);
    assert!(it.prefix[0] != 0 && it.suffix[0] == 0);
    assert_eq!(it.item_seed, after(o_e, 3));
    let it = charm_run(&t, i, o_o, 0, 0);
    assert!(it.prefix[0] != 0 && it.suffix[0] != 0);
    assert_eq!(it.item_seed, after(o_o, 4));
    // p > 0, s = 0: forced preferred prefix, no suffix roll.
    let it = charm_run(&t, i, even, 1, 0);
    assert_eq!((it.prefix[0], it.suffix[0]), (3, 0));
    assert_eq!(it.item_seed, after(even, 1));
    // p = 0, s > 0: no prefix roll, forced preferred suffix.
    let it = charm_run(&t, i, even, 0, 2);
    assert_eq!((it.prefix[0], it.suffix[0]), (0, 2));
    assert_eq!(it.item_seed, after(even, 1));
    // Both preferred.
    let it = charm_run(&t, i, even, 2, 1);
    assert_eq!((it.prefix[0], it.suffix[0]), (4, 1));
    assert_eq!(it.item_seed, after(even, 2));
    // Request slots 1–2 are ignored.
    let mut it = item(i, even);
    let rq = ItemRequest {
        prefix: [0, 1, 1],
        suffix: [0, 1, 1],
        ..Default::default()
    };
    charm(&t, &mut it, &rq).unwrap();
    assert_eq!(it, charm_run(&t, i, even, 0, 0));
    // Identified cleared; no class skill mods (no stat 107, no draws).
    assert_eq!(it.flags & flag::IDENTIFIED, 0);
    assert_eq!(it.stats.item_list(stat::ITEM_SINGLESKILL, 36), 0);
    assert!(it
        .stats
        .lists
        .values()
        .all(|l| l.keys().all(|&(id, _)| id != stat::ITEM_SINGLESKILL)));
}

// ------------------------------------------------------- edge cases

/// Edge case 5: the charm's prefix roll gets the suffix preference, which
/// is ≤ 0 there, so it changes nothing.
#[test]
fn charm_prefix_gets_suffix_preference() {
    let (t, i) = charm_tables();
    for seed in 0..16 {
        let base = charm_run(&t, i, seed, 0, 0);
        for (p, s) in [(0, -1), (0, -7), (-1, 0), (-3, -2)] {
            assert_eq!(charm_run(&t, i, seed, p, s), base, "seed {seed} ({p}, {s})");
        }
    }
}

//! test vectors and edge cases.

use super::*;
use crate::items::affixes::{alvl, crafted, magic, rare, roll_affix, RARE_COUNTS};
use crate::items::tables::RareRec;
use crate::items::{flag, q, Fatal, ItemRequest};

#[test]
fn affix_level_vectors() {
    for (ilvl, qlvl, m, want) in [
        (30, 20, 0, 20),
        (95, 85, 0, 91),
        (1, 5, 0, 3),
        (40, 1, 3, 43),
        (98, 60, 0, 97),
        (69, 60, 0, 39),
    ] {
        assert_eq!(alvl(ilvl, qlvl, m), want, "({ilvl}, {qlvl}, {m})");
    }
}

#[test]
fn rare_count_vectors() {
    let mut s = Seed::init_low(5);
    assert_eq!(s.step(), 367_056_499);
    for (seed, want) in [(5, 5), (6, 3), (7, 5), (8, 4)] {
        let lo = Seed::init_low(seed).step();
        assert_eq!(RARE_COUNTS[(lo & 7) as usize], want, "seed {seed}");
    }
}

/// Tables with `n` suffix rows fitting a ring (no prefixes, no automagic).
fn ring_suffixes(rows: Vec<crate::items::tables::AffixRec>) -> (ItemTables, usize) {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.n_suffix = rows.len();
    t.magic = rows;
    (t, i)
}

/// Vector: frequencies 3, 0, 5 (no magic lvl), total 8, r = 3 picks the
/// second candidate (row 3).
#[test]
fn roller_frequency_walk() {
    let rows = [3, 0, 5]
        .iter()
        .enumerate()
        .map(|(k, &f)| {
            let mut r = affix_row(RING, k as i32 + 1);
            r.frequency = f;
            r
        })
        .collect();
    let (t, i) = ring_suffixes(rows);
    let seed = find_seed(|s| {
        s.step();
        s.roll(9) == 3
    });
    let mut it = item(i, seed);
    assert_eq!(roll_affix(&t, &mut it, true, true, false, false, 0, 0), 3);
    // r = 2 → first candidate.
    let seed = find_seed(|s| {
        s.step();
        s.roll(9) == 2
    });
    let mut it = item(i, seed);
    assert_eq!(roll_affix(&t, &mut it, true, true, false, false, 0, 0), 1);
}

/// Vector: weighted mode (magic lvl ≠ 0): levels 10, 20, freq 2, 1 →
/// weights 20, 20, roll(41); r = 40 picks the last (edge case 2).
#[test]
fn roller_weighted() {
    let rows = [(10, 2), (20, 1)]
        .iter()
        .enumerate()
        .map(|(k, &(l, f))| {
            let mut r = affix_row(RING, k as i32 + 1);
            r.level = l;
            r.frequency = f;
            r
        })
        .collect();
    let (mut t, i) = ring_suffixes(rows);
    t.items[i].magic_lvl = 50;
    for (r, want) in [(19, 1), (20, 2), (40, 2)] {
        let seed = find_seed(|s| {
            s.step();
            s.roll(41) == r
        });
        let mut it = item(i, seed);
        assert_eq!(
            roll_affix(&t, &mut it, true, true, false, false, 0, 0),
            want
        );
        let mut s = Seed::init_low(seed);
        s.step();
        s.step();
        assert_eq!(it.item_seed, s, "coin + one pick");
    }
}

/// Edge case 6 and the coin: the coin step is drawn even when forced;
/// an even coin without force returns 0.
#[test]
fn roller_coin() {
    let (t, i) = ring_suffixes(vec![affix_row(RING, 1)]);
    let seed = find_seed(|s| s.step() & 1 == 0);
    let mut it = item(i, seed);
    assert_eq!(roll_affix(&t, &mut it, true, false, false, false, 0, 0), 0);
    let mut s = Seed::init_low(seed);
    s.step();
    assert_eq!(it.item_seed, s);
    let mut it = item(i, seed);
    assert_eq!(roll_affix(&t, &mut it, true, true, false, false, 0, 0), 1);
}

/// Edge case 1: rows past the 511 cap still add weight; a roll landing
/// there picks the last listed candidate.
#[test]
fn roller_candidate_cap() {
    let rows = (0..600).map(|k| affix_row(RING, k + 1)).collect();
    let (t, i) = ring_suffixes(rows);
    let seed = find_seed(|s| {
        s.step();
        s.roll(601) >= 511
    });
    let mut it = item(i, seed);
    assert_eq!(roll_affix(&t, &mut it, true, true, false, false, 0, 0), 511);
}

#[test]
fn roller_filters() {
    let mut rows: Vec<_> = (0..6).map(|k| affix_row(RING, k + 1)).collect();
    rows[0].spawnable = 0;
    rows[1].level = 90; // above alvl
    rows[2].rare = 0; // excluded on rare items
    rows[3].etype[0] = RING as i16;
    rows[4].classspecific = 2;
    let (mut t, i) = ring_suffixes(rows);
    t.itemtypes[RING as usize].class = 3;
    let mut it = item(i, 1);
    it.ilvl = 10;
    it.quality = q::RARE;
    for seed in 0..20 {
        it.item_seed = Seed::init_low(seed);
        assert_eq!(roll_affix(&t, &mut it, true, true, false, false, 0, 0), 6);
    }
    // A preferred row skips the level test and returns without a draw.
    it.item_seed = Seed::init_low(4);
    assert_eq!(roll_affix(&t, &mut it, true, true, false, false, 2, 0), 2);
    // The group of an affix already on the item is taken.
    it.suffix[0] = 6;
    assert_eq!(roll_affix(&t, &mut it, true, true, false, false, 0, 0), 0);
}

/// No affix rows → magic fails (downgrade).
#[test]
fn magic_without_rows_fails() {
    let (t, i) = ring_suffixes(vec![]);
    let mut it = item(i, 1);
    assert!(!magic(&t, &mut it, &ItemRequest::default()));
}

fn rare_rows(n: usize) -> Vec<RareRec> {
    (0..n)
        .map(|_| RareRec {
            itype: [RING as i16, 0, 0, 0, 0, 0, 0],
            ..Default::default()
        })
        .collect()
}

#[test]
fn rare_item() {
    let mut rows: Vec<_> = (0..4).map(|k| affix_row(RING, k + 1)).collect();
    rows.extend((0..4).map(|k| affix_row(RING, k + 11)));
    let (mut t, i) = ring_suffixes(rows);
    t.n_suffix = 4;
    t.n_prefix = 4;
    t.rare = rare_rows(4);
    t.n_rare_suffix = 2;
    let mut it = item(i, 77);
    it.quality = q::RARE;
    it.flags = flag::IDENTIFIED;
    assert!(rare(&t, &mut it, &ItemRequest::default()));
    assert!((3..=4).contains(&it.rare_prefix));
    assert!((1..=2).contains(&it.rare_suffix));
    let np = it.prefix.iter().filter(|&&x| x != 0).count();
    let ns = it.suffix.iter().filter(|&&x| x != 0).count();
    let mut s = Seed::init_low(77);
    s.roll(2);
    s.roll(2);
    let n = RARE_COUNTS[(s.step() & 7) as usize] as usize;
    assert_eq!(np + ns, n.min(6));
    assert!(it.prefix.iter().all(|&x| x == 0 || (5..=8).contains(&x)));
    assert_eq!(it.flags & flag::IDENTIFIED, 0);
}

/// Edge case 3: crafted with a filled slot and no candidate crashes the
/// base game (group of id 0).
#[test]
fn crafted_null_group() {
    let (mut t, i) = ring_suffixes(vec![affix_row(RING, 1), affix_row(RING, 2)]);
    t.n_suffix = 1;
    t.n_prefix = 1;
    t.rare = rare_rows(2);
    t.n_rare_suffix = 1;
    let mut it = item(i, 5);
    it.quality = q::CRAFTED;
    let rq = ItemRequest {
        ilvl: 80,
        ..Default::default()
    };
    assert_eq!(crafted(&t, &mut it, &rq), Err(Fatal::NullAffixGroup));
}

#[test]
fn crafted_minimum_count() {
    let mut rows: Vec<_> = (0..6).map(|k| affix_row(RING, k + 1)).collect();
    rows.extend((0..6).map(|k| affix_row(RING, k + 11)));
    let (mut t, i) = ring_suffixes(rows);
    t.n_suffix = 6;
    t.n_prefix = 6;
    t.rare = rare_rows(2);
    t.n_rare_suffix = 1;
    for seed in 0..30 {
        let mut it = item(i, seed);
        it.quality = q::CRAFTED;
        let rq = ItemRequest {
            ilvl: 80,
            ..Default::default()
        };
        assert_eq!(crafted(&t, &mut it, &rq), Ok(true));
        let n = it
            .prefix
            .iter()
            .chain(&it.suffix)
            .filter(|&&x| x != 0)
            .count();
        assert!(n >= 4, "seed {seed}: {n}");
    }
}

/// Edge case 4: a charm with no fitting affix is fatal.
#[test]
fn charm_without_affix() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(ty::CHAR, b"cm1 "));
    let mut it = item(i, 1);
    assert_eq!(
        crate::items::affixes::charm(&t, &mut it, &ItemRequest::default()),
        Err(Fatal::Charm)
    );
}

#[test]
fn automagic_group() {
    let mut t = tables();
    let mut r = item_rec(RING, b"rin ");
    r.auto_prefix = 7;
    let i = push_item(&mut t, r);
    let mut a = affix_row(RING, 3);
    let mut b = affix_row(RING, 7);
    a.spawnable = 0;
    b.spawnable = 0;
    t.magic = vec![a, b];
    let mut it = item(i, 1);
    crate::items::affixes::automagic(&t, &mut it);
    assert_eq!(it.auto_affix, 2);
}

// (rules the mutation run of `cargo mutants --file
// 'crates/d2-sim/src/monsters/**'` found unchecked)
//! Tests that kill mutants which survived the existing monster tests,
//! on public APIs only. Tests that need a submodule's
//! private fakes live in `mutant_tests/{ai,init,population}.rs`, mounted
//! as children of that submodule's `tests` module.

use d2_data::bin::BinTable;

use super::ai::skill_modes;
use d2_data::tables::{Levels, Monlvl, Monstats, Record};

use super::init::{
    area_level, component_counts, hp_regen, monlvl_dm, monstats_extra, pct, stats_by_level,
    GameInfo,
};
use super::population::PopState;

fn bin(record_size: usize, records: Vec<u8>) -> BinTable {
    BinTable {
        name: "test".into(),
        source: "test".into(),
        count: records.len() / record_size,
        record_size,
        records,
    }
}

#[test]
fn skill_modes_reads_bytes_0x180_to_0x187() {
    // `Sk1mode..Sk3mode` are bytes +0x180..+0x182 of each monstats record,
    // `Sk4mode` +0x183 (Vampire), `Sk5mode`..`Sk8mode` +0x184..+0x187
    // (Act II–V bodies); +0x17F and +0x188 are not read.
    let size = 0x190;
    let mut records = vec![0u8; 2 * size];
    records[0x180..0x183].copy_from_slice(&[8, 9, 4]);
    records[size + 0x17F..size + 0x189].copy_from_slice(&[7, 5, 14, 1, 7, 2, 3, 6, 11, 9]);
    assert_eq!(
        skill_modes(&bin(size, records)),
        [[8, 9, 4, 0, 0, 0, 0, 0], [5, 14, 1, 7, 2, 3, 6, 11]]
    );
}

#[test]
fn l_flag_is_game_type_or_ladder() {
    // L-flag = (game +0x6A ≠ 0) or (game +0x74 ≠ 0).
    let mut g = GameInfo::default();
    assert!(!g.l_flag());
    g.game_type = 3;
    assert!(g.l_flag());
    g.game_type = 0;
    g.ladder = true;
    assert!(g.l_flag());
}

#[test]
fn monstats_extra_reads_eight_signed_modes() {
    // `Sk1mode..Sk8mode` at record +0x180..+0x187, read signed.
    let size = 0x1A8;
    let mut records = vec![0u8; 2 * size];
    records[0x180..0x188].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 0xFF]);
    records[size + 0x17F..size + 0x189].copy_from_slice(&[9, 8, 7, 6, 5, 4, 3, 2, 1, 9]);
    let got: Vec<[i8; 8]> = monstats_extra(&bin(size, records))
        .iter()
        .map(|e| e.skill_modes)
        .collect();
    assert_eq!(got, [[1, 2, 3, 4, 5, 6, 7, -1], [8, 7, 6, 5, 4, 3, 2, 1]]);
}

#[test]
fn component_counts_read_bytes_0x15_to_0x24() {
    // Choice count i at monstats2
    // +0x15 + i, i = 0…15.
    let size = 0x30;
    let mut records = vec![0u8; 2 * size];
    for i in 0..16u8 {
        records[0x15 + usize::from(i)] = i + 1;
        records[size + 0x15 + usize::from(i)] = 2 * i + 3;
    }
    records[0x14] = 99;
    records[0x25] = 99;
    let want: [[u8; 16]; 2] = [
        std::array::from_fn(|i| i as u8 + 1),
        std::array::from_fn(|i| 2 * i as u8 + 3),
    ];
    assert_eq!(component_counts(&bin(size, records)), want);
}

#[test]
fn superunique_bit_is_su_and_7_of_byte_su_over_8() {
    // Bit su & 7 of byte su >> 3 of game
    // +0x1D30.
    let mut s = PopState::default();
    s.set_superunique_placed(9);
    assert_eq!(s.superunique_flags, [0, 0x02]);
    assert!(s.superunique_placed(9));
    for other in [1, 8, 10, 17, 72] {
        assert!(!s.superunique_placed(other), "su {other}");
    }
    s.set_superunique_placed(7);
    assert_eq!(s.superunique_flags, [0x80, 0x02]);
    assert!(s.superunique_placed(7));
    assert!(!s.superunique_placed(15));
}

#[test]
fn pct_branch_boundaries() {
    // v = 0x100000 is not > 0x100000: the 32-bit branch, (v × p) / d.
    assert_eq!(pct(0x10_0000, 3, 7), 449_389);
    // v = 0x100001 takes branch 2: (v / d) × p.
    assert_eq!(pct(0x10_0001, 3, 7), 149_796 * 3);
    // p = 0x10000 is not > 0x10000: (v × p) / d.
    assert_eq!(pct(3, 0x1_0000, 9), 21_845);
    // p = 0x20000 takes branch 3: (p / d) × v.
    assert_eq!(pct(3, 0x2_0000, 9), 14_563 * 3);
    // Small values: branch 4.
    assert_eq!(pct(7, 100, 3), 233);
}

fn zero<T: Record>() -> T {
    T::decode(&vec![0u8; T::SIZE])
}

#[test]
fn area_level_columns() {
    // `MonLvl<d+1>Ex` with
    // expansion, `MonLvl<d+1>` without.
    let mut r: Levels = zero();
    (r.monlvl1, r.monlvl2, r.monlvl3) = (11, 12, 13);
    (r.monlvl1ex, r.monlvl2ex, r.monlvl3ex) = (21, 22, 23);
    let lv = vec![zero::<Levels>(), r];
    for (d, ex, want) in [
        (0, true, 21),
        (1, true, 22),
        (2, true, 23),
        (0, false, 11),
        (1, false, 12),
        (2, false, 13),
    ] {
        assert_eq!(area_level(&lv, 1, d, ex), want, "d {d} ex {ex}");
    }
}

#[test]
fn monlvl_dm_with_two_rows_reads_row_1() {
    // umod 9: `DM` / `L-DM` at the level clamped to
    // 1..rows−1; with two rows that is row 1.
    let mut r: Monlvl = zero();
    (r.dm, r.dm_n, r.dm_h, r.l_dm, r.l_dm_n, r.l_dm_h) = (1, 2, 3, 4, 5, 6);
    let t = vec![zero::<Monlvl>(), r];
    assert_eq!(monlvl_dm(&t, false, 1, 50), 2);
    assert_eq!(monlvl_dm(&t, true, 2, 0), 6);
}

#[test]
fn hp_regen_formula_and_overflow_switch() {
    // (maxhp × r) >> 12.
    assert_eq!(hp_regen(1000, 100), Some(24));
    // maxhp = 0x7FFFFFFF / r is not above it: the product form.
    let m = i32::MAX / 3;
    assert_eq!(hp_regen(m, 3), Some((m * 3) >> 12));
    // One above: (maxhp >> 12) × r.
    assert_eq!(hp_regen(m + 1, 3), Some(((m + 1) >> 12) * 3));
    assert_ne!((m * 3) >> 12, (m >> 12) * 3);
}

#[test]
fn stats_by_level_0_reads_row_0() {
    // Only a negative level gives nothing: level 0 reads monlvl row 0.
    let mut m: Monstats = zero();
    (m.minhp, m.maxhp, m.ac, m.exp) = (10, 20, 30, 40);
    let mut r: Monlvl = zero();
    (r.hp, r.ac, r.xp) = (200, 50, 300);
    let t = vec![r, zero()];
    let s = stats_by_level(&m, &t, false, 0, 0);
    assert_eq!((s.min_hp, s.max_hp, s.ac, s.xp), (20, 40, 15, 120));
    let s = stats_by_level(&m, &t, false, 0, -1);
    assert_eq!((s.min_hp, s.max_hp, s.ac, s.xp), (0, 0, 0, 0));
}

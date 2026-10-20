//! Row lookups, the offer and the costs on synthetic rows.

use super::fake::{act1_ice_rows, row};
use crate::world::hirelings::{resurrect_cost, threshold, HirelingRow, HirelingRows};

/// Expansion Act 1 Normal: Fire (Id 0) and Ice (Id 1) at 3 / 36 / 67,
/// then Nightmare Ids 2, 3 at 36; classic rows first.
fn act1() -> HirelingRows {
    let mut rows = vec![row(0, 0, 1, 1, 3), row(0, 0, 1, 1, 25)];
    for id in 0..2 {
        for l in [3, 36, 67] {
            rows.push(row(100, id, 1, 1, l));
        }
    }
    rows.push(row(100, 2, 1, 2, 36));
    rows.push(row(100, 3, 1, 2, 36));
    HirelingRows::new(rows)
}

#[test]
fn row_at_picks_bracket_or_first() {
    let t = act1();
    // Id 1 rows are 5, 6, 7 (3 / 36 / 67).
    for (l, want) in [(2, 5), (3, 5), (35, 5), (36, 6), (99, 7)] {
        assert_eq!(t.row_at(true, 1, l), Some(want), "L = {l}");
    }
    // Classic Id 0 at L 30: the second classic bracket.
    assert_eq!(t.row_at(false, 0, 30), Some(1));
    assert_eq!(t.row_at(true, 256, 3), None);
    assert_eq!(t.row_at(false, 1, 3), None);
}

#[test]
fn candidates_are_lowest_brackets() {
    let t = act1();
    assert_eq!(t.candidates(true, 0, 0), vec![2, 5]);
    assert_eq!(t.candidates(true, 0, 1), vec![8, 9]);
    assert_eq!(t.candidates(false, 0, 0), vec![0]);
    assert!(t.candidates(true, 1, 0).is_empty());
    // Act 2 Normal with three subtypes.
    let rows = HirelingRows::new(vec![
        row(100, 6, 2, 1, 9),
        row(100, 6, 2, 1, 43),
        row(100, 7, 2, 1, 9),
        row(100, 8, 2, 1, 9),
    ]);
    assert_eq!(rows.candidates(true, 1, 0), vec![0, 2, 3]);
}

#[test]
fn act_of_name() {
    let mut a = row(100, 0, 1, 1, 3);
    (a.name_first, a.name_last) = (10, 50);
    let mut b = row(100, 6, 2, 1, 9);
    (b.name_first, b.name_last) = (51, 71);
    let mut c = row(0, 7, 3, 1, 9);
    (c.name_first, c.name_last) = (72, 80);
    let t = HirelingRows::new(vec![a, b, c]);
    assert_eq!(t.act_of_name(true, 10), 0);
    assert_eq!(t.act_of_name(true, 60), 1);
    assert_eq!(t.act_of_name(true, 5), 0);
    // Only rows of the game's version: the classic row's range is not
    // seen by an expansion game, and the other way round.
    assert_eq!(t.act_of_name(true, 75), 0);
    assert_eq!(t.act_of_name(false, 75), 2);
    assert_eq!(t.act_of_name(false, 60), 0);
}

#[test]
fn act_lookup_matches_class_before_the_name_range() {
    // The act lookup matches `Class` first; the name range is the fallback.
    let mut a = row(100, 0, 1, 1, 3);
    (a.name_first, a.name_last, a.class) = (10, 50, 271);
    let mut b = row(100, 6, 2, 1, 9);
    (b.name_first, b.name_last, b.class) = (51, 71, 338);
    let t = HirelingRows::new(vec![a, b]);
    assert_eq!(t.act_of(true, 338, 10), 1);
    assert_eq!(t.act_of(true, 999, 10), 0);
    assert_eq!(t.act_of(true, 999, 60), 1);
    // Test vector: class 0, expansion, name 0x0D68 → no `Class` 0 row;
    // the name-range row of Act 1 → 0.
    let mut k = row(100, 0, 1, 1, 3);
    (k.name_first, k.name_last, k.class) = (0x0D56, 0x0D76, 271);
    let t = HirelingRows::new(vec![k]);
    assert_eq!(t.act_of(true, 0, 0x0D68), 0);
    assert_eq!(t.act_of_name(true, 0x0D68), 0);
}

#[test]
fn offer_vector_seed_22752887() {
    // Test vectors: slot seed 22752887, 2 candidates,
    // player level 10 → candidate 1 (Id 1), L = 6, price 217. The price
    // fixes the Ice row's gold: 150·145/100 = 217.
    let mut rows = vec![row(100, 0, 1, 1, 3)];
    rows.extend(
        act1_ice_rows()
            .into_iter()
            .map(|r| HirelingRow { gold: 150, ..r }),
    );
    let t = HirelingRows::new(rows);
    let o = t.offer(true, 10, 22_752_887, 0, 0).expect("offer");
    assert_eq!((o.row, o.id, o.level, o.price), (1, 1, 6, 150 * 145 / 100));
    assert_eq!(o.price, 217);
    assert!(t.offer(true, 10, 22_752_887, 3, 0).is_none());
}

#[test]
fn offer_clamps_and_shift() {
    // Fire N, L 2 (player level 1 gives L < 2 → 2), bracket 3: d = −1.
    let r = HirelingRow {
        str_: 35,
        str_lvl: 10,
        dex: 45,
        dex_lvl: 15,
        hp: 45,
        hp_lvl: 9,
        gold: 100,
        dmg_min: 1,
        dmg_max: 3,
        dmg_lvl: 4,
        defense: 10,
        def_lvl: 3,
        exp_lvl: 100,
        share: -1,
        resist: -5,
        hire_desc: 7,
        ..row(100, 0, 1, 1, 3)
    };
    let t = HirelingRows::new(vec![r]);
    let o = t.offer(true, 1, 1, 0, 0).expect("offer");
    assert_eq!(o.level, 2);
    // `>> 3` rounds toward −∞: 35 + (−10 >> 3) = 35 − 2 = 33.
    assert_eq!(o.strength, 33);
    assert_eq!(o.dexterity, 45 + (-15 >> 3));
    assert_eq!(o.life, 40); // 45 − 9 = 36 → 40
    assert_eq!(o.price, 100); // 100·85/100 < gold → gold
    assert_eq!(o.experience, 3 * 100 * 2 * 2);
    assert_eq!(o.defense, 7);
    assert_eq!(o.min_damage, 0); // 1 + (−4 >> 3) = 0
    assert_eq!(o.max_damage, 2);
    assert_eq!((o.share, o.resist, o.hire_desc), (0, 0, 7));
}

#[test]
fn resurrect_cost_vectors() {
    assert_eq!(resurrect_cost(30), 6750);
    assert_eq!(resurrect_cost(1), 0);
    assert_eq!(resurrect_cost(81), 49200);
    assert_eq!(resurrect_cost(82), 50000);
}

#[test]
fn threshold_vectors() {
    // Exp/Lvl 105 (Act 1 Ice): threshold(6) 26460, threshold(7) 41160.
    assert_eq!(threshold(105, 6), 26460);
    assert_eq!(threshold(105, 7), 41160);
    // 32-bit wrap.
    assert_eq!(threshold(i32::MAX, 2), i32::MAX.wrapping_mul(12));
}

#[test]
fn from_table_reads_the_row_columns() {
    use d2_data::bin::BinTable;
    use d2_data::tables::{Hireling, Record};
    // Two 280-byte records: the selecting columns at their offsets
    // (Version u16 +0x00, Id +0x04, Class +0x08, Act +0x0C, Difficulty
    // +0x10, Seller +0x14, Level +0x1C) and the name ids u16 +0x114 /
    // +0x116.
    let mut records = vec![0u8; 2 * Hireling::SIZE];
    let put = |r: &mut [u8], o: usize, v: &[u8]| r[o..o + v.len()].copy_from_slice(v);
    for (i, r) in records
        .as_chunks_mut::<{ Hireling::SIZE }>()
        .0
        .iter_mut()
        .enumerate()
    {
        let i = i as u32;
        put(r, 0x00, &100u16.to_le_bytes());
        put(r, 0x04, &(3 + i).to_le_bytes());
        put(r, 0x08, &271u32.to_le_bytes());
        put(r, 0x0C, &1u32.to_le_bytes());
        put(r, 0x10, &2u32.to_le_bytes());
        put(r, 0x14, &150u32.to_le_bytes());
        put(r, 0x1C, &36u32.to_le_bytes());
        put(r, 0x114, &(1000 + i as u16).to_le_bytes());
        put(r, 0x116, &1040u16.to_le_bytes());
        // The skill pick's columns: DefaultChance +0x64,
        // Skill1 +0x78, Chance1 +0x90, ChancePerLvl1 +0xA8, Mode1 +0xC0.
        put(r, 0x64, &40u32.to_le_bytes());
        put(r, 0x78, &12u32.to_le_bytes());
        put(r, 0x90, &30u32.to_le_bytes());
        put(r, 0x94, &5u32.to_le_bytes());
        put(r, 0xA8, &4u32.to_le_bytes());
        put(r, 0xC0, &[14]);
    }
    let t = BinTable {
        name: Hireling::TABLE.into(),
        source: "patch_d2.mpq".into(),
        count: 2,
        record_size: Hireling::SIZE,
        records,
    };
    let rows = HirelingRows::from_table(&t).expect("hireling");
    let r = rows.rows[1];
    assert_eq!(
        (
            r.version,
            r.id,
            r.class,
            r.act,
            r.difficulty,
            r.seller,
            r.level
        ),
        (100, 4, 271, 1, 2, 150, 36)
    );
    assert_eq!((r.name_first, r.name_last), (1001, 1040));
    let ai = r.ai_row();
    assert_eq!((ai.level, ai.default_chance), (36, 40));
    assert_eq!((ai.skill[0], ai.chance[..2].to_vec()), (12, vec![30, 5]));
    assert_eq!((ai.chance_per_lvl[0], ai.mode[0]), (4, 14));
    // Each Id is its own row set: Id 4 at L 36 is row 1, Id 3 row 0.
    assert_eq!(rows.row_at(true, 4, 50), Some(1));
    assert_eq!(rows.row_at(true, 3, 50), Some(0));
    assert_eq!(rows.row_at(false, 3, 50), None, "version 100 only");
    // Not the hireling table: an error.
    let bad = BinTable {
        name: "levels".into(),
        ..t
    };
    assert!(HirelingRows::from_table(&bad).is_err());
}

#[test]
fn offer_level_follows_the_player_level_at_evaluation() {
    // The same slot seed evaluated at player level 10 and 11: L moves by
    // one, and the price with it (Gold·(100 + 15·d)/100).
    // The rows of the offer vector (Fire Id 0, then Ice Id 1).
    let mut rows = vec![row(100, 0, 1, 1, 3)];
    rows.extend(
        act1_ice_rows()
            .into_iter()
            .map(|r| HirelingRow { gold: 150, ..r }),
    );
    let t = HirelingRows::new(rows);
    let a = t.offer(true, 10, 22_752_887, 0, 0).expect("offer");
    let b = t.offer(true, 11, 22_752_887, 0, 0).expect("offer");
    assert_eq!((a.row, a.level, a.price), (1, 6, 217));
    assert_eq!((b.row, b.level, b.price), (1, 7, 150 * 160 / 100));
    // Re-evaluating at the same level gives the same offer.
    assert_eq!(t.offer(true, 10, 22_752_887, 0, 0), Some(a));
}

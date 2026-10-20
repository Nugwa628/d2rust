//! The elemental umod init bodies (9, 17, 18, 23, 25) and teleport (26):
//! hand-computed values from the rules and synthetic vectors.

use super::*;

/// monlvl rows `0..rows` whose six damage columns come from `dm(i)` in
/// the order `DM, DM(N), DM(H), L-DM, L-DM(N), L-DM(H)`.
fn tables_with(rows: u32, dm: impl Fn(u32) -> [u32; 6]) -> Tables {
    let mut t = Tables::new(vec![velocity_mon(0, 1)]);
    t.ids.monteleport = Some(184);
    t.monlvl = (0..rows)
        .map(|i| {
            let mut r: Monlvl = zero();
            [r.dm, r.dm_n, r.dm_h, r.l_dm, r.l_dm_n, r.l_dm_h] = dm(i);
            r
        })
        .collect();
    t
}

/// Every column of every row is `v`.
fn flat(v: u32) -> Tables {
    tables_with(111, |_| [v; 6])
}

/// A host with difficulty `d` and L-flag `l`.
fn host(t: Tables, d: u8, l: bool) -> Fake {
    let mut f = fake_with(t);
    f.info.difficulty = d;
    f.info.game_type = u8::from(l) * 3;
    f.info.ladder = false;
    f
}

/// A monster of `level`.
fn lvl(f: &mut Fake, level: i32) -> UnitId {
    let u = f.monster(0, 1);
    f.set_stat(u, stat::LEVEL, level);
    u
}

/// Every nonzero resistance / damage / length stat (ids 36..=63) of `u`.
fn stats_of(f: &Fake, u: UnitId) -> Vec<(u16, i32)> {
    f.stats
        .iter()
        .filter(|((id, s), v)| *id == u && (36..=63).contains(s) && **v != 0)
        .map(|((_, s), v)| (*s, *v))
        .collect()
}

fn run(f: &mut Fake, u: UnitId, umod: u8, unique: bool) {
    let cx = f.cx;
    run_umod_init(&cx, f, u, umod, unique);
}

#[test]
fn constants_getter_live_values_and_bounds() {
    let f = fake(vec![velocity_mon(0, 1)]);
    let k = |r: std::ops::Range<usize>| r.map(|i| f.cx.k(i)).collect::<Vec<_>>();
    assert_eq!(k(16..22), [0, 33, 33, 0, 50, 50]);
    assert_eq!(k(28..34), [66, 66, 66, 100, 100, 100]);
    // Past the row count (43 rows): 0.
    assert_eq!((f.cx.k(43), f.cx.k(1000)), (0, 0));
}

#[test]
fn level_is_the_unit_level_stat() {
    // coldlength = 5 × r + 100 with r the level (in range): the level stat
    // alone selects the row.
    let mut f = host(flat(10), 0, false);
    for level in [2, 7, 40] {
        let u = lvl(&mut f, level);
        run(&mut f, u, 18, false);
        assert_eq!(f.s(u, stat::COLDLENGTH), 5 * level + 100, "level {level}");
    }
}

#[test]
fn add_of_zero_changes_nothing() {
    // Minion on Normal: K[16] = K[19] = 0, so both damage adds are 0: an
    // existing value stays and an absent one gets no entry.
    let mut f = host(flat(50), 0, false);
    let u = lvl(&mut f, 10);
    f.set_stat(u, stat::FIREMINDAM, 7);
    run(&mut f, u, 9, false);
    assert_eq!(f.s(u, stat::FIREMINDAM), 7);
    assert!(!f.stats.contains_key(&(u, stat::FIREMAXDAM)));
}

#[test]
fn divisions_truncate_toward_zero() {
    // v = −5 (dword 0xFFFFFFFB): 66 × −5 / 100 = −3 (not −4), 100 × −5 /
    // 100 = −5; mana burn scales after: −3 × 256.
    let mut f = host(flat(0xFFFF_FFFB), 0, false);
    let u = lvl(&mut f, 10);
    run(&mut f, u, 9, true);
    assert_eq!(
        (f.s(u, stat::FIREMINDAM), f.s(u, stat::FIREMAXDAM)),
        (-3, -5)
    );
    run(&mut f, u, 25, true);
    assert_eq!(f.s(u, stat::MANADRAINMINDAM), -3 * 256);
}

#[test]
fn non_monster_and_missing_unit_do_nothing() {
    let mut f = host(flat(50), 0, false);
    let p = lvl(&mut f, 10);
    f.units.get_mut(p).unwrap().ty = UnitType::Player;
    let before = f.stats.clone();
    for umod in [9, 17, 18, 23, 25, 26] {
        run(&mut f, p, umod, true);
        run(&mut f, UnitId(9999), umod, true);
    }
    assert_eq!(f.stats, before, "no damage, length or resistance stat");
    assert!(f.ai_flags.is_empty());
    assert!(f.log.iter().all(|l| !l.starts_with("skill")));
}

#[test]
fn difficulty_and_l_flag_pick_the_dm_column_and_k() {
    // Row i: DM = i, DM(N) = i + 1000, DM(H) = i + 2000, L- columns + 3000,
    // + 4000, + 5000. Level 30.
    let t = |_| {
        tables_with(111, |i| {
            [i, i + 1000, i + 2000, i + 3000, i + 4000, i + 5000]
        })
    };
    for (l, d, v, boss) in [
        (false, 0u8, 30, (66, 100)),
        (false, 1, 1030, (66, 100)),
        (false, 2, 2030, (66, 100)),
        (false, 3, 2030, (66, 100)), // d ≥ 2 acts as 2
        (true, 0, 3030, (66, 100)),
        (true, 1, 4030, (66, 100)),
        (true, 2, 5030, (66, 100)),
        (true, 9, 5030, (66, 100)),
    ] {
        let mut f = host(t(()), d, l);
        let u = lvl(&mut f, 30);
        run(&mut f, u, 17, true);
        assert_eq!(
            (f.s(u, stat::LIGHTMINDAM), f.s(u, stat::LIGHTMAXDAM)),
            (boss.0 * v / 100, boss.1 * v / 100),
            "l {l} d {d}"
        );
    }
    // Minions use K[d' + 16] / K[d' + 19]: Normal 0 / 0, Nightmare 33 /
    // 50, Hell 33 / 50 (d = 5 acts as 2).
    for (d, v, kmin, kmax) in [
        (0u8, 30, 0, 0),
        (1, 1030, 33, 50),
        (2, 2030, 33, 50),
        (5, 2030, 33, 50),
    ] {
        let mut f = host(t(()), d, false);
        let u = lvl(&mut f, 30);
        run(&mut f, u, 17, false);
        assert_eq!(
            (f.s(u, stat::LIGHTMINDAM), f.s(u, stat::LIGHTMAXDAM)),
            (kmin * v / 100, kmax * v / 100),
            "minion d {d}"
        );
    }
    // Hell K[30] = 66 and K[33] = 100 for a unique, Nightmare K[29], K[32].
    let mut f = host(t(()), 2, false);
    let u = lvl(&mut f, 30);
    run(&mut f, u, 9, true);
    assert_eq!(f.s(u, stat::FIREMINDAM), 66 * 2030 / 100);
}

#[test]
fn level_clamps_to_the_monlvl_rows_for_both_row_and_length() {
    // 111 rows: r = 1 / 1 / 30 / 110 for level 0 / 1 / 30 / 200, and
    // coldlength = 5 r + 100 (poison 2 × (5 r + 150)) follows the row.
    let mut f = host(tables_with(111, |i| [i; 6]), 0, false);
    for (level, r) in [
        (-4, 1),
        (0, 1),
        (1, 1),
        (2, 2),
        (30, 30),
        (109, 109),
        (110, 110),
        (200, 110),
    ] {
        let u = lvl(&mut f, level);
        run(&mut f, u, 18, true);
        assert_eq!(f.s(u, stat::COLDLENGTH), 5 * r + 100, "level {level}");
        // DM = r (column 0), unique Normal: min 66 r / 100.
        assert_eq!(f.s(u, stat::COLDMINDAM), 66 * r / 100, "level {level}");
        run(&mut f, u, 23, true);
        assert_eq!(
            f.s(u, stat::POISONLENGTH),
            2 * (5 * r + 150),
            "level {level}"
        );
    }
    // Live extremes quoted by the edge case: level 0 → 105, ≥ 110 → 650.
    let u = lvl(&mut f, 0);
    run(&mut f, u, 18, true);
    assert_eq!(f.s(u, stat::COLDLENGTH), 105);
    let u = lvl(&mut f, 150);
    run(&mut f, u, 18, true);
    assert_eq!(f.s(u, stat::COLDLENGTH), 650);
}

#[test]
fn empty_table_returns_before_stats_and_tail_but_levels_are_clamped() {
    for umod in [9, 17, 18, 23, 25] {
        let mut f = host(tables_with(0, |_| [0; 6]), 0, false);
        let u = lvl(&mut f, 10);
        run(&mut f, u, umod, true);
        assert!(stats_of(&f, u).is_empty(), "umod {umod}: tail skipped too");
    }
    // One row: every level uses row 0 (r = rows − 1 = 0), never rejected.
    let mut f = host(tables_with(1, |_| [40; 6]), 0, false);
    let u = lvl(&mut f, 77);
    run(&mut f, u, 17, true);
    assert_eq!(f.s(u, stat::LIGHTMINDAM), 66 * 40 / 100);
    assert_eq!(f.s(u, stat::LIGHTRESIST), 75);
    // A level far above the table is clamped, not rejected.
    let mut f = host(flat(10), 0, false);
    let u = lvl(&mut f, 100_000);
    run(&mut f, u, 17, true);
    assert_eq!(f.s(u, stat::LIGHTMAXDAM), 10);
}

#[test]
fn per_umod_stats_scales_lengths_and_tails() {
    // v = 19 at r = 30, boss Normal (K 66 / 100): min 12, max 19.
    let mut f = host(tables_with(111, |_| [19; 6]), 0, false);
    let want: [(u8, Vec<(u16, i32)>); 5] = [
        (9, vec![(48, 12), (49, 19), (39, 75)]),
        (17, vec![(50, 12), (51, 19), (41, 75)]),
        (18, vec![(54, 12), (55, 19), (56, 250), (43, 75)]),
        (23, vec![(57, 12), (58, 19), (59, 600), (45, 75)]),
        (25, vec![(62, 12 * 256), (63, 19 * 256), (37, 20)]),
    ];
    for (umod, mut exp) in want {
        let u = lvl(&mut f, 30);
        run(&mut f, u, umod, true);
        exp.sort();
        assert_eq!(stats_of(&f, u), exp, "umod {umod}");
    }
}

#[test]
fn minions_get_length_but_no_damage_and_no_tail_on_normal() {
    // umod 18, unique 0, d 0, v 7, r 10: no damage stats, coldlength 150,
    // no resistance.
    let mut f = host(tables_with(111, |_| [7; 6]), 0, false);
    let u = lvl(&mut f, 10);
    run(&mut f, u, 18, false);
    assert_eq!(stats_of(&f, u), [(56, 150)]);
    // Poison minion on Normal likewise: poisonlength 2 × 200.
    let u = lvl(&mut f, 10);
    run(&mut f, u, 23, false);
    assert_eq!(stats_of(&f, u), [(59, 400)]);
    // Spec vector: umod 23 minion, d 1, v 25, r 30 → +8, +12, +600, and no
    // resistance change (edge 5).
    let mut f = host(tables_with(111, |_| [25; 6]), 1, false);
    let u = lvl(&mut f, 30);
    run(&mut f, u, 23, false);
    assert_eq!(stats_of(&f, u), [(57, 8), (58, 12), (59, 600)]);
    // Fire, lightning, mana minions on Nightmare get damage, never a tail.
    for (umod, lo, hi) in [(9, 48, 49), (17, 50, 51), (25, 62, 63)] {
        let u = lvl(&mut f, 30);
        run(&mut f, u, umod, false);
        let k = if umod == 25 { 256 } else { 1 };
        assert_eq!(stats_of(&f, u), [(lo, 8 * k), (hi, 12 * k)], "umod {umod}");
    }
}

#[test]
fn mana_burn_truncates_before_the_scale() {
    // Spec vector: umod 25, unique, d 0, v 19 → 12 × 256 and 19 × 256
    // (4864); never 12.54 × 256.
    let mut f = host(flat(19), 0, false);
    let u = lvl(&mut f, 5);
    run(&mut f, u, 25, true);
    assert_eq!(f.s(u, stat::MANADRAINMINDAM), 3072);
    assert_eq!(f.s(u, stat::MANADRAINMAXDAM), 4864);
    assert_eq!(f.s(u, stat::MANADRAINMINDAM) % 256, 0);
    assert_eq!(f.s(u, stat::MAGICRESIST), 20);
}

#[test]
fn teleport_sets_the_ai_may_teleport_flag_for_a_boss() {
    let mut f = host(flat(1), 0, false);
    let u = lvl(&mut f, 5);
    run(&mut f, u, 26, true);
    assert_eq!(f.ai_flags.get(&u), Some(&0x20));
    assert_eq!(crate::monsters::ai::flag::MAY_TELEPORT, 0x20);
}

#[test]
fn teleport_ignores_minions() {
    // Spec vector: umod 26, unique 0 → nothing.
    let mut f = host(flat(1), 0, false);
    let u = lvl(&mut f, 5);
    run(&mut f, u, 26, false);
    assert!(f.ai_flags.is_empty());
    assert!(f.log.iter().all(|l| !l.starts_with("skill")));
}

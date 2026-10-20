//! Tests for `kurast.rs`: Kurast border rows, fixed presets, the random preset placer,
//! Kurast Causeway and Travincal. Expectations restate the preset tables; the placer's
//! draws are recomputed from its rules on a separate `Seed`.

use super::super::grid::{cell, file_of};
use super::super::tests::Env;
use crate::rng::Seed;

const SEED: u32 = 0x1234_5678;

fn env(id: u32, gw: i32, gh: i32, j: bool) -> Env {
    let mut e = Env::new(id, gw, gh);
    e.drlg.jungle_link = j;
    e.drlg.level_mut(e.l).seed = Seed::init_low(SEED);
    e
}

fn p_at(e: &Env, x: i32, y: i32) -> u32 {
    e.info.grids[0].get(x, y)
}

fn f_at(e: &Env, x: i32, y: i32) -> i32 {
    file_of(e.info.grids[2].get(x, y))
}

fn stamped(e: &Env, x: i32, y: i32) -> bool {
    e.info.grids[2].get(x, y) & cell::PRESET != 0
}

/// Presets in first-use order (the build list inserts at the front).
fn first_use(e: &Env) -> Vec<u32> {
    e.info.build_list.iter().rev().map(|n| n.preset).collect()
}

fn stepped(n: usize) -> Seed {
    let mut s = Seed::init_low(SEED);
    for _ in 0..n {
        s.step();
    }
    s
}

/// Expected border of a 10×8 level: (cell, preset) for every stamp.
fn check_border(
    e: &Env,
    top: &[(i32, u32)],
    bottom: &[(i32, u32)],
    side: (u32, u32),
    corners: [u32; 4],
) {
    for &(i, p) in top {
        assert_eq!(p_at(e, i, 0), p, "top ({i}, 0)");
    }
    for &(i, p) in bottom {
        assert_eq!(p_at(e, i, 7), p, "bottom ({i}, 7)");
    }
    for i in 1..=6 {
        assert_eq!(p_at(e, 9, i), side.0, "side (9, {i})");
        assert_eq!(p_at(e, 0, i), side.1, "side (0, {i})");
    }
    let cells = [(0, 0), (9, 0), (0, 7), (9, 7)];
    for (k, &(x, y)) in cells.iter().enumerate() {
        assert_eq!(p_at(e, x, y), corners[k], "corner ({x}, {y})");
    }
    for y in 0..8 {
        for x in 0..10 {
            let edge = x == 0 || y == 0 || x == 9 || y == 7;
            assert_eq!(stamped(e, x, y), edge, "({x}, {y})");
            if edge {
                assert_eq!(f_at(e, x, y), 0);
            }
        }
    }
}

/// Row helper: i = 1..8 with `p`, except `(at, q)`.
fn row(p: u32, at: i32, q: u32) -> Vec<(i32, u32)> {
    (1..=8).map(|i| (i, if i == at { q } else { p })).collect()
}

/// Skip row: i = 1..8 with `p`, `q` at (gw−1)/2 = 4, i = 5 not stamped.
fn skip_row(p: u32, q: u32) -> Vec<(i32, u32)> {
    [1, 2, 3, 4, 6, 7, 8]
        .iter()
        .map(|&i| (i, if i == 4 { q } else { p }))
        .chain(std::iter::once((5, 0)))
        .collect()
}

#[test]
fn lower_kurast_border_rows() {
    for j in [false, true] {
        let mut e = env(79, 10, 8, j);
        e.od.presets[614].size_x = 16; // the wide piece covers the skipped cell
        e.gen().kurast_border().unwrap();
        let t = if j { 1 } else { 8 };
        check_border(
            &e,
            &row(605, t, 613),
            &skip_row(606, 614),
            (607, 608),
            [610, 609, 612, 611],
        );
        let top = if j { vec![613, 605] } else { vec![605, 613] };
        let rest = [606, 614, 607, 608, 610, 609, 612, 611];
        assert_eq!(first_use(&e), [top, rest.to_vec()].concat(), "J {j}");
        // One build-list roll(1) per distinct preset (Files 1).
        assert_eq!(e.seed(), stepped(10));
    }
}

#[test]
fn kurast_bazaar_border_rows() {
    for j in [false, true] {
        let mut e = env(80, 10, 8, j);
        e.gen().kurast_border().unwrap();
        let (a, b) = if j { (1, 8) } else { (8, 1) };
        check_border(
            &e,
            &row(619, b, 627),
            &row(620, a, 628),
            (621, 622),
            [624, 623, 626, 625],
        );
        // Top and bottom interleave per i: (i, 0) then (i, gh−1).
        let rows = if j {
            vec![619, 628, 620, 627]
        } else {
            vec![627, 620, 619, 628]
        };
        let rest = [621, 622, 624, 623, 626, 625];
        assert_eq!(first_use(&e), [rows, rest.to_vec()].concat(), "J {j}");
        assert_eq!(e.seed(), stepped(10));
    }
}

#[test]
fn upper_kurast_border_rows() {
    for j in [false, true] {
        let mut e = env(81, 10, 8, j);
        e.od.presets[644].size_x = 16;
        e.gen().kurast_border().unwrap();
        let t = if j { 8 } else { 1 };
        check_border(
            &e,
            &skip_row(636, 644),
            &row(637, t, 645),
            (638, 639),
            [641, 640, 643, 642],
        );
        let bottom = if j { vec![637, 645] } else { vec![645, 637] };
        let rest = [638, 639, 641, 640, 643, 642];
        assert_eq!(
            first_use(&e),
            [vec![636, 644], bottom, rest.to_vec()].concat(),
            "J {j}"
        );
        assert_eq!(e.seed(), stepped(10));
    }
}

#[test]
fn skip_row_half_truncates() {
    // gw 9: (gw−1)/2 = 4; i runs 1..7: 1, 2, 3, 4 (wide), 6, 7.
    let mut e = env(81, 9, 8, false);
    e.gen().kurast_border().unwrap();
    let got: Vec<u32> = (0..9).map(|x| p_at(&e, x, 0)).collect();
    assert_eq!(got, [641, 636, 636, 636, 644, 0, 636, 636, 640]);
}

/// Counts of each preset id in grid 0, and the file of each stamp.
fn census(e: &Env, p: u32) -> Vec<(i32, i32, i32)> {
    let mut v = Vec::new();
    for y in 0..e.info.gh() {
        for x in 0..e.info.gw() {
            if p_at(e, x, y) == p {
                v.push((x, y, f_at(e, x, y)));
            }
        }
    }
    v
}

fn fixed_presets(id: u32, wide: u32, wide_cells: [(i32, i32); 2], s: u32, rnd: [u32; 4]) {
    for j in [false, true] {
        let mut e = env(id, 10, 8, j);
        // The wide pieces (79 bottom, 81 top) cover their skipped cell.
        e.od.presets[614].size_x = 16;
        e.od.presets[644].size_x = 16;
        e.gen().kurast().unwrap();
        let w = census(&e, wide);
        assert_eq!(
            w,
            [
                (wide_cells[0].0, wide_cells[0].1, 0),
                (wide_cells[1].0, wide_cells[1].1, 1)
            ]
        );
        let sv: Vec<i32> = census(&e, s).iter().map(|c| c.2).collect();
        assert_eq!(sv.iter().filter(|&&f| f == 0).count(), 1);
        assert_eq!(sv.iter().filter(|&&f| f == 1).count(), 1);
        assert_eq!(census(&e, 631).iter().map(|c| c.2).collect::<Vec<_>>(), [0]);
        // Interior 8×6 = 48 cells; 5 fixed; R(max 4) places 4; the
        // two-preset R fills every remaining cell (1×1 fakes); the last
        // R finds none.
        assert_eq!(census(&e, rnd[0]).len(), 4);
        assert_eq!(
            census(&e, rnd[1]).len() + census(&e, rnd[2]).len(),
            48 - 5 - 4
        );
        assert!(census(&e, rnd[3]).is_empty());
        for y in 0..8 {
            for x in 0..10 {
                assert!(stamped(&e, x, y), "({x}, {y})");
            }
        }
    }
}

#[test]
fn kurast_bazaar_fixed_presets() {
    // 629 at (3, 3) F 0 and (X = 6, 3) F 1.
    fixed_presets(80, 629, [(3, 3), (6, 3)], 630, [635, 633, 634, 632]);
}

#[test]
fn upper_kurast_fixed_presets() {
    // 646 at (3, Y = 4) F 0 and (X = 6, 4) F 1.
    fixed_presets(81, 646, [(3, 4), (6, 4)], 647, [651, 649, 650, 648]);
}

#[test]
fn lower_kurast_presets() {
    let mut e = env(79, 10, 8, false);
    e.od.presets[614].size_x = 16; // the wide bottom piece
    e.gen().kurast().unwrap();
    assert_eq!(census(&e, 631).iter().map(|c| c.2).collect::<Vec<_>>(), [0]);
    assert_eq!(census(&e, 618).len(), 4);
    assert_eq!(census(&e, 616).len() + census(&e, 617).len(), 48 - 1 - 4);
    assert!(census(&e, 615).is_empty());
}

#[test]
fn causeway_one_fixed_stamp() {
    // Level 82 is 48×16: 6×2 cells.
    let mut e = env(82, 6, 2, false);
    e.od.presets[652].size_x = 48;
    e.od.presets[652].size_y = 16;
    e.gen().kurast().unwrap();
    e.gen().travincal().unwrap();
    assert_eq!(p_at(&e, 0, 0), 652);
    for y in 0..2 {
        for x in 0..6 {
            assert!(stamped(&e, x, y));
            assert_eq!(f_at(&e, x, y), 0);
            if (x, y) != (0, 0) {
                assert_eq!(p_at(&e, x, y), 0);
            }
        }
    }
    // F 0: no build-list draw.
    assert!(e.info.build_list.is_empty());
    assert_eq!(e.seed(), Seed::init_low(SEED));
}

#[test]
fn travincal_six_stamps() {
    // Level 83 is 64×64: 8×8 cells; lvlprest Files 1.
    let mut e = env(83, 8, 8, false);
    for (p, w) in [(653, 2), (654, 4), (655, 2), (656, 2), (657, 4), (658, 2)] {
        e.od.presets[p].size_x = 8 * w;
        e.od.presets[p].size_y = 32;
    }
    e.gen().kurast().unwrap();
    e.gen().travincal().unwrap();
    let cells = [
        (0, 0, 653),
        (2, 0, 654),
        (6, 0, 655),
        (0, 4, 656),
        (2, 4, 657),
        (6, 4, 658),
    ];
    for (x, y, p) in cells {
        assert_eq!(p_at(&e, x, y), p);
    }
    for y in 0..8 {
        for x in 0..8 {
            assert!(stamped(&e, x, y), "({x}, {y})");
            assert_eq!(f_at(&e, x, y), 0);
        }
    }
    assert_eq!(first_use(&e), [653, 654, 655, 656, 657, 658]);
    // Six build-list roll(1), one per stamp.
    assert_eq!(e.seed(), stepped(6));
}

#[test]
fn other_levels_untouched() {
    for id in [76, 77, 78, 83] {
        let mut e = env(id, 8, 8, true);
        e.gen().kurast().unwrap();
        assert!(
            e.info.grids.iter().all(|g| g.cells.iter().all(|&c| c == 0)),
            "{id}"
        );
        assert_eq!(e.seed(), Seed::init_low(SEED));
    }
    for id in [76, 79, 80, 81, 82] {
        let mut e = env(id, 10, 8, true);
        e.gen().travincal().unwrap();
        assert!(
            e.info.grids.iter().all(|g| g.cells.iter().all(|&c| c == 0)),
            "{id}"
        );
        assert_eq!(e.seed(), Seed::init_low(SEED));
    }
}

/// The placer's rules on their own seed: full-grid entries
/// (k mod gw, k div gw), A pairs of roll(A) swaps, then per tried entry
/// P := lo + roll(n); a free cell takes P (first use of P draws the
/// build list's roll(Files = 1)); stop at `max` when max > 0.
fn model(
    seed: &mut Seed,
    gw: i32,
    gh: i32,
    busy: &[(i32, i32)],
    lo: u32,
    n: i32,
    max: i32,
) -> Vec<(i32, i32, u32)> {
    let a = gw * gh;
    let mut e: Vec<(i32, i32)> = (0..a).map(|k| (k % gw, k / gw)).collect();
    for _ in 0..a {
        let i = seed.roll(a) as usize;
        let j = seed.roll(a) as usize;
        e.swap(i, j);
    }
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for (x, y) in e {
        let p = lo + seed.roll(n);
        if busy.contains(&(x, y)) {
            continue;
        }
        if !seen.contains(&p) {
            seed.roll(1);
            seen.push(p);
        }
        out.push((x, y, p));
        if max > 0 && out.len() as i32 >= max {
            break;
        }
    }
    out
}

fn run_placer(lo: u32, hi: u32, max: i32) {
    let busy = [(1, 0), (2, 2), (3, 1)];
    let mut e = env(79, 4, 3, false);
    for &(x, y) in &busy {
        e.info.grids[2].op(x, y, super::super::grid::Op::Or, cell::PRESET);
    }
    e.gen().random_presets(lo, hi, max).unwrap();
    let mut s = Seed::init_low(SEED);
    let want = model(&mut s, 4, 3, &busy, lo, (hi - lo + 1) as i32, max);
    let mut got = Vec::new();
    for y in 0..3 {
        for x in 0..4 {
            let p = p_at(&e, x, y);
            if p != 0 {
                got.push((x, y, p));
            }
        }
    }
    let mut want_sorted = want.clone();
    want_sorted.sort_by_key(|&(x, y, _)| (y, x));
    assert_eq!(got, want_sorted, "R({lo}, {hi}, {max})");
    assert_eq!(e.seed(), s, "R({lo}, {hi}, {max}) draws");
    // Stamp F −1: every placed cell takes file 0 (Files 1).
    for (x, y, _) in want {
        assert_eq!(f_at(&e, x, y), 0);
    }
}

#[test]
fn random_placer_draw_order() {
    run_placer(616, 617, 0); // no limit: fills the 9 free cells
    run_placer(618, 618, 4); // n = 1 still draws; stops at 4
    run_placer(649, 650, 2);
}

#[test]
fn random_placer_n1_steps_per_entry() {
    // 2×1 grid, nothing busy, max 0: A = 2 → 2 pairs of swaps (4 steps),
    // then one roll(1) per entry (2) and one build-list roll(1): 7 steps.
    let mut e = env(79, 2, 1, false);
    e.gen().random_presets(615, 615, 0).unwrap();
    assert_eq!(e.seed(), stepped(7));
    assert_eq!((p_at(&e, 0, 0), p_at(&e, 1, 0)), (615, 615));
    // max 1 stops after the first stamp: 4 + 1 + 1 steps.
    let mut e = env(79, 2, 1, false);
    e.gen().random_presets(615, 615, 1).unwrap();
    assert_eq!(e.seed(), stepped(6));
}

//! Tests for outdoor rules the other test files do not cover: arithmetic and roll rules,
//! act-wide placement, placement checks, borders, and the Act I, II and IV builds.

#[allow(unused_imports)]
use super::*;

use super::acts::desert_cliff_row;
use super::grid::{cell, Op};
use super::place::{self, Check, Driver, LinkRow, Linker};
use super::tests::{act1_data, data, od, one_cell_file, stepped, Env, Presets, Rec};
use crate::drlg::{Drlg, DrlgData, NoLevelTypes, TileRect};
use crate::rng::Seed;

// ---- helpers ------------------------------------------------------------------

fn set(d: &mut DrlgData, id: usize, ty: u32, size: (i32, i32), off: (i32, i32)) {
    d.levels[id].drlg_type = ty;
    d.levels[id].size = [size; 3];
    d.levels[id].offset = off;
}

/// DRLG creation of `act` through the outdoor adapter.
fn create(act: u8, seed: u32, data: &DrlgData) -> (Drlg, Outdoor) {
    let od = od();
    let mut outdoor = Outdoor::default();
    let mut rec = Rec::default();
    let mut presets = Presets::default();
    let subs = SubFileMap::default();
    let mut types = OutdoorTypes {
        outdoor: &mut outdoor,
        od: &od,
        subs: &subs,
        presets: &mut presets,
        others: &mut rec,
        last_error: None,
    };
    let drlg = Drlg::create(act, seed, 0, 0, false, data, &mut types).unwrap();
    assert_eq!(types.last_error, None);
    (drlg, outdoor)
}

fn rect_of(drlg: &Drlg, id: u32) -> TileRect {
    drlg.level(drlg.find_level(id).unwrap()).rect
}

fn orth_of(drlg: &Drlg, o: &Outdoor, id: u32) -> Vec<(u32, i32, bool)> {
    o.level(drlg.find_level(id).unwrap())
        .unwrap()
        .orth
        .iter()
        .map(|e| (e.level_id, e.direction, e.preset))
        .collect()
}

/// Lvlsub rows of types `ts`, each with a file whose single pattern cell
/// never matches (wall style 199: grid 0 would need base + 199).
fn inert_subs(e: &mut Env, ts: &[i32]) {
    for &t in ts {
        let name = format!("inert{t}").into_bytes();
        e.od.subs.push(SubRow {
            type_: t,
            file: name.clone(),
            bord_type: 1,
            grid_size: 1,
            ..SubRow::default()
        });
        e.subs.0.insert(name, one_cell_file(0, (200 << 8) | 1, 1));
    }
}

/// Presets in first-stamp order (the build list is head-inserted).
fn first_stamps(info: &OutdoorLevel, keep: impl Fn(u32) -> bool) -> Vec<u32> {
    info.build_list
        .iter()
        .rev()
        .map(|n| n.preset)
        .filter(|&p| keep(p))
        .collect()
}

fn is_rotation(seg: &[u32], list: &[u32]) -> bool {
    let n = list.len();
    seg.len() == n && (0..n).any(|r| (0..n).all(|k| seg[k] == list[(r + k) % n]))
}

fn count(e: &Env, p: u32) -> usize {
    e.info.grids[0].cells.iter().filter(|&&c| c == p).count()
}

fn file_at(e: &Env, x: i32, y: i32) -> u32 {
    (e.info.grids[2].get(x, y) >> 16) & 0xF
}

// ---- arithmetic and roll rules ------------------------------------------------------

#[test]
fn division_truncates_toward_zero_and_roll_semantics() {
    // `/8` truncates toward zero: q = −3 gives 8·0 + 11 (a floor would
    // give 8·(−1) + 11 = 3).
    let mut e = Env::new(2, 10, 10);
    let r = e.drlg.level(e.l).rect;
    let g = e.gen();
    let at = |x, y, direction| g.adjust(PathPoint { x, y, direction });
    assert_eq!(at(r.x - 3, r.y, 0).x - r.x, 11);
    assert_eq!(at(r.x - 3, r.y, 2).x - r.x, -5);
    assert_eq!(at(r.x, r.y - 9, 3).y - r.y, -13);
    assert_eq!(at(r.x, r.y - 9, 1).y - r.y, 3);
    // roll(n ≤ 0): no step; roll(1) steps (river bridge roll(gh − 2)).
    let mut e = Env::new(4, 6, 2);
    e.info.flags = 0x10;
    let s0 = e.seed();
    e.gen().river(2).unwrap();
    assert_eq!(e.seed(), s0);
    let mut e = Env::new(4, 6, 3);
    e.info.flags = 0x10;
    let s0 = e.seed();
    e.gen().river(2).unwrap();
    assert_eq!(e.seed(), stepped(s0, 1));
}

// ---- act-wide placement -------------------------------------------------------------

#[test]
fn act_wide_placement_per_act() {
    // Act I: A1W then A1M (allocation order of their rows, Outer Cloister
    // with Black Marsh), then neighbour entries.
    let (drlg, o) = create(0, 644409375, &act1_data());
    let mut order: Vec<u32> = drlg
        .level_list()
        .into_iter()
        .map(|l| drlg.level(l).id)
        .collect();
    order.reverse();
    // The placer rows allocate first; the neighbour-entry walk over 1..17 then allocates
    // 8..16.
    assert_eq!(
        order,
        [4, 3, 2, 1, 17, 39, 26, 7, 6, 27, 5, 8, 9, 10, 11, 12, 13, 14, 15, 16]
    );
    assert!(!orth_of(&drlg, &o, 2).is_empty());

    // Act II: A2 (Def 40, RW 41, …), A2C (Def 46), neighbours 40..46.
    let mut d = data();
    set(&mut d, 40, 2, (56, 40), (5000, 5000));
    for id in 41..=45 {
        set(&mut d, id, 3, (80, 80), (0, 0));
    }
    set(&mut d, 46, 3, (40, 40), (9000, 9000));
    d.levels[41].vis = [40, 42, 0, 0, 0, 0, 0, 0];
    let (drlg, o) = create(1, 99, &d);
    // The driver works on a copy: the DRLG seed is where creation left it.
    let mut copy = drlg.seed;
    let r0 = (copy.step() & 1) as i32 + 1;
    assert_eq!(rect_of(&drlg, 40), TileRect::new(5000, 5000, 56, 40));
    let r41 = rect_of(&drlg, 41);
    let want = if r0 == 1 { (4920, 5000) } else { (5000, 4920) };
    assert_eq!((r41.x, r41.y), want);
    for id in 42..=45 {
        assert!(drlg.find_level(id).is_some(), "level {id}");
    }
    assert_eq!(rect_of(&drlg, 46), TileRect::new(9000, 9000, 40, 40));
    assert_eq!(o.preset_direction.get(&40), Some(&r0));
    let dir = if r0 == 1 { 2 } else { 3 };
    assert!(orth_of(&drlg, &o, 41).contains(&(40, dir, true)));

    // Act III: docks at leveldefs offset and size; jungles; Kurast chain anchored on
    // level 78; adjacency warps and neighbour entries 75..83.
    let mut d = data();
    set(&mut d, 75, 2, (400, 100), (2000, 5000));
    for id in 76..=78 {
        set(&mut d, id, 3, (64, 192), (0, 0));
    }
    let sizes = [(80, 40), (80, 48), (80, 56), (40, 40), (120, 80)];
    for (k, id) in (79..=83).enumerate() {
        set(&mut d, id, 3, sizes[k], (0, 0));
    }
    let (drlg, o) = create(2, 99, &d);
    assert_eq!(rect_of(&drlg, 75), TileRect::new(2000, 5000, 400, 100));
    for id in 76..=78 {
        assert!(drlg.find_level(id).is_some(), "level {id}");
    }
    let r78 = rect_of(&drlg, 78);
    let mut y = 0;
    for (k, id) in (79..=83).enumerate() {
        let (w, h) = sizes[k];
        y -= h;
        assert_eq!(
            rect_of(&drlg, id),
            TileRect::new(r78.x + 32 - w / 2, r78.y + y, w, h),
            "level {id}"
        );
    }
    // Vis came only from the adjacency warps (data vis empty).
    let n79 = orth_of(&drlg, &o, 79);
    assert!(n79.contains(&(78, 3, false)), "{n79:?}");
    assert!(n79.contains(&(80, 1, false)), "{n79:?}");

    // Act IV: A4 (OS draws on the copy), A4C, Outer Steppes takes the OS
    // transition flag, neighbours 103..106.
    let mut d = data();
    set(&mut d, 103, 2, (40, 40), (1000, 1000));
    for id in 104..=106 {
        set(&mut d, id, 3, (80, 80), (0, 0));
    }
    set(&mut d, 108, 3, (120, 120), (5000, 5000));
    d.levels[104].vis = [103, 105, 0, 0, 0, 0, 0, 0];
    let (drlg, o) = create(3, 99, &d);
    let mut copy = drlg.seed;
    let (pos, flag) = if copy.step() & 1 == 0 {
        ((1040, 992), 0x40_0000)
    } else {
        ((1040, 968), 0x80_0000)
    };
    let r104 = rect_of(&drlg, 104);
    assert_eq!((r104.x, r104.y), pos);
    let l104 = drlg.find_level(104).unwrap();
    assert_eq!(o.level(l104).unwrap().flags & 0xC0_0000, flag);
    assert_eq!(rect_of(&drlg, 108), TileRect::new(5000, 5000, 120, 120));
    assert!(orth_of(&drlg, &o, 104).contains(&(103, 0, true)));

    // Act V: A5, A5T, adjacency 111..112, neighbours 111..112, adjacency
    // 110..111, 109..110, A5U; no link-column warps.
    let mut d = data();
    set(&mut d, 109, 2, (40, 40), (1000, 1000));
    set(&mut d, 110, 3, (40, 200), (3000, 1000));
    set(&mut d, 111, 3, (64, 160), (0, 0));
    set(&mut d, 112, 3, (64, 160), (0, 0));
    set(&mut d, 117, 3, (64, 160), (6000, 6000));
    set(&mut d, 134, 3, (40, 40), (8000, 8000));
    set(&mut d, 136, 2, (40, 40), (9000, 9000));
    let (drlg, o) = create(4, 99, &d);
    let mut copy = drlg.seed;
    let d1 = (copy.step() & 1) as usize;
    let d2 = (copy.step() & 1) as usize;
    let size = |v| if v == 0 { (64, 160) } else { (160, 64) };
    let (w1, h1) = size(d1);
    let r111 = TileRect::new(3000 - w1, 1000 + 200 - h1 - 16, w1, h1);
    assert_eq!(rect_of(&drlg, 111), r111);
    let off = [(0, -160), (-96, -64), (-64, -96), (-160, 0)][d2 + 2 * d1];
    let (w2, h2) = size(d2);
    assert_eq!(
        rect_of(&drlg, 112),
        TileRect::new(r111.x + off.0, r111.y + off.1, w2, h2)
    );
    // A5T's own copy: BD's draw is the copy's first step again.
    assert_eq!(rect_of(&drlg, 117), TileRect::new(6000, 6000, w1, h1));
    assert_eq!(rect_of(&drlg, 134), TileRect::new(8000, 8000, 40, 40));
    assert_eq!(rect_of(&drlg, 136), TileRect::new(9000, 9000, 40, 40));
    // 110 links to 109 in the table, but Act V sets no link warps (and the
    // rects do not touch).
    assert!(!drlg.vis_array(&d, 110).unwrap().contains(&109));
    // Adjacency 111..112 ran before the neighbour entries; 110..111 after.
    let vis111 = drlg.vis_array(&d, 111).unwrap();
    assert!(vis111.contains(&112) && vis111.contains(&110));
    assert_eq!(orth_of(&drlg, &o, 111).len(), 1);
    assert_eq!(orth_of(&drlg, &o, 111)[0].0, 112);
}

#[test]
fn link_tables_rows() {
    use Linker::*;
    let t = |rows: &[LinkRow]| -> Vec<(Linker, u32, i32)> {
        rows.iter().map(|r| (r.linker, r.level, r.link)).collect()
    };
    assert_eq!(
        t(&place::A1W),
        [
            (Def, 4, -1),
            (R4, 3, 0),
            (Bm, 2, 1),
            (Re, 1, 2),
            (R4, 17, 1)
        ]
    );
    assert_eq!(
        t(&place::A1M),
        [
            (Def, 39, -1),
            (Def, 26, -1),
            (Fix, 7, 1),
            (R4, 6, 2),
            (R4, 5, 3)
        ]
    );
    assert_eq!(
        t(&place::A2),
        [
            (Def, 40, -1),
            (Rw, 41, 0),
            (R8, 42, 1),
            (R8, 43, 2),
            (R8, 44, 3),
            (Vs, 45, 4)
        ]
    );
    assert_eq!(t(&place::A2C), [(Def, 46, -1)]);
    assert_eq!(t(&place::A5U), [(Def, 134, -1), (Def, 136, -1)]);
    assert_eq!(
        t(&place::A4),
        [(Def, 103, -1), (Os, 104, 0), (R4, 105, 1), (R4, 106, 2)]
    );
    assert_eq!(t(&place::A4C), [(Def, 108, -1)]);
    assert_eq!(
        t(&place::A5),
        [(Def, 109, -1), (Def, 110, 0), (B1, 111, 1), (B2, 112, 2)]
    );
    assert_eq!(t(&place::A5T), [(Bd, 117, -1)]);
    for rows in [
        &place::A1W[..],
        &place::A1M,
        &place::A2,
        &place::A2C,
        &place::A5U,
        &place::A4,
        &place::A4C,
        &place::A5,
        &place::A5T,
    ] {
        assert!(rows.len() <= place::TABLE_ROWS);
    }
}

// ---- placement checks and Act I border flags ----------------------------------------

fn drive_rows(seed: u32, rows: &[LinkRow], chk: Check, data: &DrlgData) -> (Driver, Seed) {
    let mut drlg = Drlg::create(0, seed, 0, 0, false, data, &mut NoLevelTypes).unwrap();
    let s0 = drlg.seed;
    let mut o = Outdoor::default();
    let d = o
        .drive(&mut drlg, data, &mut NoLevelTypes, rows, chk, false)
        .unwrap();
    (d, s0)
}

/// Expected link-flag rows, written out independently: (filter, excl, r, rNext, flags).
fn spec_flags(id: u32, r: i32, rn: i32) -> u32 {
    let any_23: [(i32, i32, u32); 2] = [(1, 0, 0x4), (2, 3, 0x4)];
    let any_317: [(i32, i32, u32); 4] = [(2, 1, 0x8), (3, 0, 0x8), (1, 1, 0x10), (3, 3, 0x10)];
    let lvl2: [(i32, i32, u32); 9] = [
        (0, 0, 0x8),
        (2, 2, 0x8),
        (3, 0, 0x8),
        (3, 2, 0x8),
        (0, 1, 0x400),
        (1, 1, 0x400),
        (2, 1, 0x200),
        (2, 2, 0x80),
        (3, 2, 0x100),
    ];
    let mut f = 0;
    let hit = |rows: &[(i32, i32, u32)]| {
        rows.iter()
            .filter(|&&(a, b, _)| a == r && b == rn)
            .fold(0, |acc, &(_, _, x)| acc | x)
    };
    if id != 2 && id != 3 {
        f |= hit(&any_23);
    }
    if id != 3 && id != 17 {
        f |= hit(&any_317);
    }
    if id == 2 {
        f |= hit(&lvl2);
    }
    f
}

#[test]
fn gap_test_checks_and_act1_flags() {
    // Gap test, margin 0: touching is not overlapping.
    let a = TileRect::new(0, 0, 10, 10);
    assert!(!place::overlaps(&a, &TileRect::new(10, 0, 5, 5)));
    assert!(place::overlaps(&a, &TileRect::new(9, 0, 5, 5)));
    assert!(!place::overlaps(&a, &TileRect::new(-5, -20, 5, 40)));
    assert!(place::overlaps(&a, &TileRect::new(-4, -4, 5, 5)));
    assert!(!place::overlaps(&a, &TileRect::new(3, 10, 2, 2)));
    assert!(place::overlaps(&a, &TileRect::new(2, 2, 2, 2)));

    // Simple / A1W / A1M on a synthetic table: A (39) at (0, 300), O (4)
    // blocks R4 case 3 of B (5); A1M also rejects case 2 (inside A's
    // rect extended 200 upward).
    let mut d = data();
    set(&mut d, 39, 3, (64, 64), (0, 300));
    set(&mut d, 4, 3, (40, 40), (84, 300));
    set(&mut d, 5, 3, (80, 80), (0, 0));
    let rows = [
        LinkRow {
            linker: Linker::Def,
            level: 39,
            link: -1,
        },
        LinkRow {
            linker: Linker::Def,
            level: 4,
            link: -1,
        },
        LinkRow {
            linker: Linker::R4,
            level: 5,
            link: 0,
        },
    ];
    let mut seen = [false; 4];
    for seed in 1..=40 {
        let (dv, s0) = drive_rows(seed, &rows, Check::None, &d);
        let mut c = s0;
        let first = (c.step() & 3) as i32;
        seen[first as usize] = true;
        assert_eq!(dv.r[0][2], first);
        for chk in [Check::Simple, Check::A1W] {
            let (dv, _) = drive_rows(seed, &rows, chk, &d);
            assert_eq!(dv.r[0][2], if first == 3 { 0 } else { first });
        }
        let (dv, _) = drive_rows(seed, &rows, Check::A1M, &d);
        assert_eq!(dv.r[0][2], if first >= 2 { 0 } else { first });
    }
    assert_eq!(seen, [true; 4]);

    // A1W on the real table: Rogue Encampment's T entry and Burial
    // Grounds' distinct R0 hold whenever the check is on; without it some
    // seed breaks one of them.
    let t_ones = [0, 1, 9, 19, 21, 24, 30, 31, 33, 39, 41, 53, 54, 56, 62, 63];
    let rows = place::A1W;
    let ok = |dv: &Driver| {
        let clear = (0..rows.len()).all(|i| {
            (0..i).all(|j| j as i32 == rows[i].link || !place::overlaps(&dv.c[j], &dv.c[i]))
        });
        let (i, l) = (3, 2);
        let idx = dv.r[0][i] + 4 * (dv.r[2][i] + 2 * (dv.r[0][l] + 4 * dv.r[2][l]));
        let burial = (0..rows.len())
            .all(|j| j == 4 || rows[j].link != rows[4].link || dv.r[0][j] != dv.r[0][4]);
        clear && t_ones.contains(&idx) && burial
    };
    let data1 = act1_data();
    let mut broken = 0;
    for seed in 1..=30 {
        let (dv, _) = drive_rows(seed, &rows, Check::A1W, &data1);
        assert!(ok(&dv), "seed {seed}");
        let (dv, _) = drive_rows(seed, &rows, Check::None, &data1);
        broken += usize::from(!ok(&dv));
    }
    assert!(broken > 0);

    // Flags function: every row of the table, by level filter and
    // exclusions.
    for id in [2u32, 3, 4, 17] {
        for r in -1..4 {
            for rn in -1..4 {
                let mut dv = Driver::new(Seed::init());
                dv.r[0][0] = r;
                dv.r[0][1] = rn;
                let mut o = Outdoor::default();
                let l = LevelIdx(0);
                o.act1_flags(l, id, &dv, 0);
                assert_eq!(
                    o.levels[&l].flags,
                    spec_flags(id, r, rn),
                    "level {id}, ({r}, {rn})"
                );
            }
        }
    }
    // Outdoor levels only: the preset Rogue Encampment gets no outdoor
    // info from the driver.
    let mut drlg = Drlg::create(0, 7, 0, 0, false, &data1, &mut NoLevelTypes).unwrap();
    let mut o = Outdoor::default();
    o.drive(
        &mut drlg,
        &data1,
        &mut NoLevelTypes,
        &place::A1W,
        Check::A1W,
        true,
    )
    .unwrap();
    assert!(o.level(drlg.find_level(1).unwrap()).is_none());
    assert!(o.level(drlg.find_level(2).unwrap()).is_some());
}

// ---- level generation ---------------------------------------------------------------

#[test]
fn act_dispatch_by_level_id() {
    for (id, act) in [
        (2, 0),
        (39, 0),
        (40, 1),
        (46, 1),
        (75, 2),
        (102, 2),
        (103, 3),
        (108, 3),
        (109, 4),
        (117, 4),
        (133, 4),
        (134, 1),
        (135, 4),
    ] {
        assert_eq!(dispatch_act(id), act, "level {id}");
    }
}

// ---- borders ------------------------------------------------------------------------

#[test]
fn straight_pieces_follow_each_edge_direction() {
    // Plain wild rect: W edge walked S→N (0, −1), N W→E (1, 0), E N→S
    // (0, 1), S E→W (−1, 0). Border(dx, dy, 1) = P[N[dx + 3dy + 4] + 1][1].
    let mut e = Env::new(4, 6, 6);
    let mut g = e.gen();
    g.polygon().unwrap();
    g.borders().unwrap();
    for k in 1..5 {
        assert_eq!(g.g(0, 0, k), 5, "W (0, {k})");
        assert_eq!(g.g(0, k, 0), 6, "N ({k}, 0)");
        assert_eq!(g.g(0, 5, k), 7, "E (5, {k})");
        assert_eq!(g.g(0, k, 5), 4, "S ({k}, 5)");
    }
}

#[test]
fn corner_pieces_at_the_edge_end() {
    // Corner of edge V0 (0, 5) → V1 (0, 0), next edge east: Corner(0, −2,
    // 2, 0, s') stamped at (0, 0).
    let run = |d0: u8, d1: u8, f0: u32, f1: u32| {
        let mut e = Env::new(4, 6, 6);
        let mut g = e.gen();
        g.polygon().unwrap();
        g.info.vertices[0].direction = d0;
        g.info.vertices[0].flags = f0;
        g.info.vertices[1].direction = d1;
        g.info.vertices[1].flags = f1;
        g.borders().unwrap();
        (g.g(0, 0, 0), g.g(2, 0, 0) & cell::DIRECTION)
    };
    // d = 0: wild style, N[84] = 6 → 9; bit 0x2 clear.
    assert_eq!(run(0, 0, 0, 0), (9, 0));
    // d ≠ 0: cliff style, piece 19 → 20 / 21 / 19 by the directions; bit
    // 0x2 set (from the corner when only n has a direction).
    assert_eq!(run(1, 0, 0, 0), (20, cell::DIRECTION));
    assert_eq!(run(0, 1, 0, 0), (21, cell::DIRECTION));
    assert_eq!(run(1, 1, 0, 0), (19, cell::DIRECTION));
    // A preset-link vertex keeps its arguments undoubled (doubled: 9):
    // v → Corner(0, −1, 2, 0) = N[85] = 3 → P[3][1] = 6; n → Corner(0, −2,
    // 1, 0) = N[75] = 2 → P[2][1] = 5.
    let pl = VERTEX_LINK | VERTEX_PRESET_LINK;
    assert_eq!(run(0, 0, pl, 0).0, 6);
    assert_eq!(run(0, 0, 0, pl).0, 5);
}

// ---- Act I --------------------------------------------------------------------------

#[test]
fn moo_moo_farm_substitution_types_in_order() {
    let run = |ts: &[i32]| {
        let mut e = Env::new(39, 12, 12);
        inert_subs(&mut e, ts);
        let mut g = e.gen();
        g.polygon().unwrap();
        g.act1()
    };
    assert_eq!(run(&[0, 1, 2, 3]), Ok(()));
    // A missing type is reached; of two missing types the lower one first.
    assert_eq!(run(&[2, 3]), Err(OutdoorError::NoSubRows(0)));
    assert_eq!(run(&[0, 3]), Err(OutdoorError::NoSubRows(1)));
    assert_eq!(run(&[0, 1]), Err(OutdoorError::NoSubRows(2)));
    assert_eq!(run(&[0, 1, 2]), Err(OutdoorError::NoSubRows(3)));
}

#[test]
fn special_presets_per_level() {
    let special = |p: u32| matches!(p, 29..=50 | 160..=163);
    for (id, want) in [
        (2u32, &[46u32, 47, 29, 30][..]),
        (3, &[48, 49, 44, 29, 30]),
        (4, &[160, 45, 162, 47, 49, 42, 31]),
        (5, &[161, 41, 40, 48, 49, 43, 29, 30]),
        (6, &[163, 38, 39, 47, 49, 42, 29, 30]),
        (7, &[48, 49, 43, 31]),
        (39, &[50, 46, 31, 38, 39, 29, 30]),
    ] {
        let mut e = Env::new(id, 16, 16);
        inert_subs(&mut e, &[0, 1, 2, 3]);
        let town = e
            .drlg
            .get_or_alloc_level(&e.data, &mut NoLevelTypes, 1)
            .unwrap();
        e.drlg.level_mut(town).rect = TileRect::new(0, 0, 56, 40);
        {
            let mut g = e.gen();
            g.polygon().unwrap();
            g.act1().unwrap();
        }
        let got = first_stamps(&e.info, special);
        // Cottage 49 depends on a draw: compare without it, then check it
        // only follows a "yes" cottage.
        let strip = |v: &[u32]| v.iter().copied().filter(|&p| p != 49).collect::<Vec<_>>();
        assert_eq!(strip(&got), strip(want), "level {id}: {got:?}");
        if let Some(i) = got.iter().position(|&p| p == 49) {
            assert!(
                i > 0 && matches!(got[i - 1], 47 | 48),
                "level {id}: {got:?}"
            );
        }
    }
    // Burial Grounds: 108 at (1, 1).
    let mut e = Env::new(17, 8, 8);
    {
        let mut g = e.gen();
        g.polygon().unwrap();
        g.act1().unwrap();
    }
    assert_eq!(e.info.grids[0].get(1, 1), 108);
    // Cottage(P, extra): `&3`; nonzero → R(P) and, if extra, `&1` → R(49);
    // zero → R(P) twice. R on a 6×6 grid without paths: two shuffles of
    // 16 entries (64 steps) plus the build-list roll of a new preset.
    let mut seen = [false; 3];
    for s in 1..=40u32 {
        for extra in [false, true] {
            let mut e = Env::new(4, 6, 6);
            e.drlg.level_mut(e.l).seed = Seed::init_low(s);
            let mut c = e.seed();
            let k = c.mask(4);
            let mut after = stepped(e.seed(), 1 + 64 + 1);
            let bit = after.mask(2);
            e.gen().cottage(47, extra).unwrap();
            let (n47, n49) = if k != 0 {
                (1, usize::from(extra && bit != 0))
            } else {
                (2, 0)
            };
            assert_eq!((count(&e, 47), count(&e, 49)), (n47, n49), "seed {s}");
            seen[if k == 0 { 0 } else { 1 + bit as usize }] = true;
        }
    }
    assert_eq!(seen, [true; 3]);
}

#[test]
fn river_at_gw_minus_2_with_flags_c() {
    for (flags, dir_bit, river) in [
        (0x8, false, true),
        (0x4, false, true),
        (0x8, true, false),
        (0x10, false, false),
    ] {
        let mut e = Env::new(4, 10, 10);
        e.info.flags = flags;
        if dir_bit {
            e.gen().op(2, 9, 3, Op::Or, cell::DIRECTION);
        }
        e.gen().river_caves().unwrap();
        for y in 0..10 {
            let (a, b) = (e.info.grids[0].get(8, y), e.info.grids[0].get(9, y));
            if river {
                assert!(
                    matches!(a, 26 | 28) && matches!(b, 27 | 28),
                    "{flags:#x} y {y}"
                );
            } else {
                assert!(a != 26 && b != 27, "{flags:#x} y {y}");
            }
        }
    }
}

#[test]
fn river_at_mid_column_with_flag_10() {
    for (flags, dir_bit, river) in [
        (0x50, false, true),
        (0x50, true, false),
        (0x40, false, false),
    ] {
        let mut e = Env::new(4, 10, 10);
        e.info.flags = flags;
        if dir_bit {
            e.gen().op(2, 5, 7, Op::Or, cell::DIRECTION);
        }
        e.gen().transitions().unwrap();
        // x = gw/2 − 1 = 4.
        for y in 0..10 {
            let (a, b) = (e.info.grids[0].get(4, y), e.info.grids[0].get(5, y));
            if river {
                assert!(
                    matches!(a, 26 | 28) && matches!(b, 27 | 28),
                    "{flags:#x} y {y}"
                );
            } else {
                assert!(a != 26 && b != 27, "{flags:#x} y {y}");
            }
        }
    }
}

/// A path point as (x, y, direction).
type Pt = (i32, i32, i32);

#[test]
fn dirt_path_join_points() {
    let joins = |e: &Env| -> Vec<(Pt, Pt)> {
        e.info
            .path_ends
            .iter()
            .map(|p| {
                (
                    (p.join.x, p.join.y, p.join.direction),
                    (
                        p.join_adjusted.x,
                        p.join_adjusted.y,
                        p.join_adjusted.direction,
                    ),
                )
            })
            .collect()
    };
    // Bridge at (gw/2 − 1, 3) = (4, 3), file 1: starts west of it join at
    // (bx, by) facing 2, east at (bx + 8, by) facing 0.
    let mut e = Env::new(4, 10, 10);
    let r = e.drlg.level(e.l).rect;
    e.info.flags = 0x10;
    e.gen().op(0, 4, 3, Op::Set, 28);
    e.gen().op(2, 4, 3, Op::Or, 1 << 16);
    e.gen().op(0, 1, 5, Op::Set, 24);
    e.gen().op(0, 8, 6, Op::Set, 25);
    e.gen().dirt_paths().unwrap();
    let (bx, by) = (r.x + 35, r.y + 27);
    assert_eq!(
        joins(&e),
        [
            ((bx, by, 2), (r.x + 27, by, 2)),
            ((bx + 8, by, 0), (r.x + 51, by, 0))
        ]
    );
    // One start: centre (gw/2, gh/2); (5, 5) is not spawn valid, the spiral
    // takes (4, 5) (r 1, k 0); direction 4 is not adjusted.
    let mut e = Env::new(4, 10, 10);
    e.gen().op(0, 1, 5, Op::Set, 24);
    e.gen().op(2, 5, 5, Op::Or, cell::BORDER);
    e.gen().dirt_paths().unwrap();
    let j = (r.x + 35, r.y + 43, 4);
    assert_eq!(joins(&e), [(j, j)]);
    // Two starts: centre = mean cell (70/16, 54/16) = (4, 3); every join
    // copies join[0].
    let mut e = Env::new(4, 10, 10);
    e.gen().op(0, 1, 1, Op::Set, 24);
    e.gen().op(0, 7, 5, Op::Set, 25);
    e.gen().dirt_paths().unwrap();
    let j = (r.x + 35, r.y + 27, 4);
    assert_eq!(joins(&e), [(j, j), (j, j)]);
    // No valid cell: the last tried one, (5 + 7, 5).
    let mut e = Env::new(4, 10, 10);
    for c in e.info.grids[2].cells.iter_mut() {
        *c = cell::BORDER;
    }
    e.gen().op(0, 1, 5, Op::Set, 24);
    e.gen().dirt_paths().unwrap();
    let j = (r.x + 99, r.y + 43, 4);
    assert_eq!(joins(&e), [(j, j)]);
}

// ---- Act II -------------------------------------------------------------------------

/// A desert level of 20 × 20 cells run through the Act II build.
fn act2_env(id: u32, gw: i32, town: Option<i32>) -> (Env, Result<(), OutdoorError>) {
    let mut e = Env::new(id, gw, gw);
    e.drlg.level_mut(e.l).level_type = 16;
    e.od.presets[392].files = 2;
    inert_subs(&mut e, &[1, 2, 3]);
    let r = {
        let mut g = e.gen();
        g.polygon().unwrap();
        if let Some(dir) = town {
            g.info.orth.push(Orth {
                level_id: 40,
                direction: dir,
                init: false,
                rect: TileRect::default(),
                preset: true,
            });
        }
        g.act2()
    };
    (e, r)
}

#[test]
fn act2_level_sequences() {
    let keep = |p: u32| matches!(p, 362 | 363) || p >= 376;
    let cliff = |p: u32| (376..=382).contains(&p);
    let marks = |e: &Env, bit: u32| {
        e.info.grids[2]
            .cells
            .iter()
            .filter(|&&c| c & bit != 0)
            .count()
    };
    // Cliffs: one of the 8 rows stamped (P, x, y), F as given.
    let one_cliff_row = |e: &Env| {
        (0..8)
            .filter(|&r| {
                desert_cliff_row(r).iter().all(|&(p, f, x, y)| {
                    e.info.grids[0].get(x, y) == p && (f < 0 || file_at(e, x, y) == f as u32)
                })
            })
            .count()
            == 1
    };
    // 41: town transition S (direction 3) at (0, gh − 1); exit 388; 5
    // shrines; variants.
    let (e, r) = act2_env(41, 20, Some(3));
    r.unwrap();
    let got = first_stamps(&e.info, keep);
    assert_eq!(&got[..2], &[363, 388]);
    assert!(
        is_rotation(&got[2..], &[395, 411, 401, 402, 399, 398, 403]),
        "{got:?}"
    );
    assert_eq!(e.info.grids[0].get(0, 19), 363);
    assert_eq!((marks(&e, cell::SHRINE), marks(&e, cell::WAYPOINT)), (5, 0));
    // Other directions: 362 at (gw − 1, 0); no level-40 entry: nothing.
    let (e, _) = act2_env(41, 20, Some(2));
    assert_eq!(e.info.grids[0].get(19, 0), 362);
    let (e, _) = act2_env(41, 20, None);
    assert!(!first_stamps(&e.info, keep)
        .iter()
        .any(|&p| p == 362 || p == 363));
    // 42..44: cliffs, exit, waypoint, shrines, variant lists.
    for (id, exit, lists, iter_ids) in [
        (
            42u32,
            388u32,
            vec![&[395u32, 411, 400, 398][..]],
            vec![(404u32, 1usize), (405, 1), (406, 1), (407, 1)],
        ),
        (
            43,
            390,
            vec![&[396, 397][..], &[411, 399, 398, 403][..]],
            vec![(395, 2)],
        ),
        (
            44,
            412,
            vec![&[413, 408, 409, 410][..], &[395, 400, 398, 404, 405][..]],
            vec![(411, 2)],
        ),
    ] {
        let (e, r) = act2_env(id, 20, None);
        r.unwrap();
        assert!(one_cliff_row(&e), "level {id}");
        let got = first_stamps(&e.info, keep);
        let x = got.iter().position(|&p| p == exit).unwrap();
        assert!(
            x > 0 && got[..x].iter().all(|&p| cliff(p)),
            "level {id}: {got:?}"
        );
        let mut rest = &got[x + 1..];
        for l in lists {
            assert!(is_rotation(&rest[..l.len()], l), "level {id}: {got:?}");
            rest = &rest[l.len()..];
        }
        assert!(rest.is_empty(), "level {id}: {got:?}");
        for (p, n) in iter_ids {
            assert_eq!(count(&e, p), n, "level {id} preset {p}");
        }
        assert_eq!((marks(&e, cell::SHRINE), marks(&e, cell::WAYPOINT)), (5, 1));
    }
    // 45: the exit only (after link flags and desert borders).
    let (e, r) = act2_env(45, 20, None);
    r.unwrap();
    assert_eq!(first_stamps(&e.info, keep), [389]);
    assert_eq!(marks(&e, cell::SHRINE), 0);
    assert_eq!(e.info.grids[0].get(3, 0), 366);
    // Exit not placed: fatal.
    let (_, r) = act2_env(45, 2, None);
    assert_eq!(r, Err(OutdoorError::ExitNotPlaced(389)));
    // 46: tomb row and 394 at (4, 4); 134: 394 at (4, 4). Then
    // variants; 392 (Files 2) once per file, 393 once.
    for id in [46u32, 134] {
        let (e, r) = act2_env(id, 20, None);
        r.unwrap();
        let got = first_stamps(&e.info, keep);
        assert_eq!(got[0], 394);
        assert!(
            is_rotation(&got[1..], &[401, 402, 406, 407, 403]),
            "{got:?}"
        );
        assert_eq!(e.info.grids[0].get(4, 4), 394);
        assert_eq!((count(&e, 392), count(&e, 393)), (2, 1));
        let mut files: Vec<u32> = (0..20)
            .flat_map(|y| (0..20).map(move |x| (x, y)))
            .filter(|&(x, y)| e.info.grids[0].get(x, y) == 392)
            .map(|(x, y)| file_at(&e, x, y))
            .collect();
        files.sort();
        assert_eq!(files, [0, 1]);
        assert_eq!(marks(&e, cell::SHRINE), 5);
        if id == 46 {
            for (p, f, x, y) in [
                (384, 0, 8, 0),
                (383, 2, 6, 0),
                (383, 1, 4, 0),
                (383, 0, 2, 0),
                (387, 0, 0, 0),
                (385, 0, 0, 2),
                (385, 1, 0, 4),
                (385, 2, 0, 6),
                (386, 0, 0, 8),
            ] {
                assert_eq!(
                    (e.info.grids[0].get(x, y), file_at(&e, x, y)),
                    (p, f),
                    "({x}, {y})"
                );
            }
        }
    }
    // Variants: roll(n) then n spawns from r (no iteration: the
    // build list); with iteration: S(P, f) for every file.
    let mut e = Env::new(42, 20, 20);
    let s0 = e.seed();
    let r = {
        let mut c = s0;
        c.roll(3) as usize
    };
    e.gen().variants(&[392, 393, 395], false).unwrap();
    let list = [392, 393, 395];
    let got = first_stamps(&e.info, |_| true);
    assert_eq!(got, [list[r], list[(r + 1) % 3], list[(r + 2) % 3]]);
    // The eight cliff rows as (P, x, y) sets.
    let set_of = |r: u32| {
        let mut v: Vec<(u32, i32, i32)> = desert_cliff_row(r)
            .iter()
            .map(|&(p, _, x, y)| (p, x, y))
            .collect();
        v.sort();
        v
    };
    for r in 0..3 {
        let px = 2 + 2 * r as i32;
        let mut want = vec![(376, 0, 4), (376, 8, 4)];
        for x in [2, 4, 6] {
            want.push((if x == px { 378 } else { 377 }, x, 4));
        }
        want.sort();
        assert_eq!(set_of(r), want, "row {r}");
    }
    for (r, a, b) in [(3, 377, 381), (4, 378, 380)] {
        let mut want = vec![(376, 8, 4), (a, 6, 4), (382, 4, 4), (b, 4, 6), (379, 4, 8)];
        want.sort();
        assert_eq!(set_of(r), want, "row {r}");
    }
    for r in 5..8 {
        let py = 2 + 2 * (r as i32 - 5);
        let mut want = vec![(379, 4, 0), (379, 4, 8)];
        for y in [2, 4, 6] {
            want.push((if y == py { 381 } else { 380 }, 4, y));
        }
        want.sort();
        assert_eq!(set_of(r), want, "row {r}");
    }
}

// ---- Act IV -------------------------------------------------------------------------

#[test]
fn act4_mesas_pits_and_sanctum() {
    // Chaos Sanctum: 25 stamps on a 3-cell lattice, no borders.
    let mut e = Env::new(108, 15, 15);
    {
        let mut g = e.gen();
        g.polygon().unwrap();
        g.act4().unwrap();
    }
    for i in 0..25 {
        let want = match i {
            22 => 857,
            11 => 858,
            13 => 859,
            17 => 860,
            7 => 861,
            12 => 862,
            _ => 836,
        };
        assert_eq!(e.info.grids[0].get(3 * (i % 5), 3 * (i / 5)), want, "i {i}");
    }
    assert_eq!(e.info.grids[0].get(1, 0), 0);
    // Outer Steppes / Plains of Despair / City of the Damned.
    let keep = |p: u32| p == 798 || (811..=835).contains(&p);
    for (id, flags, want, counts) in [
        (
            104u32,
            0x40_0000u32,
            vec![798u32, 812, 813, 814, 815, 816, 828, 829, 830, 831],
            vec![
                (812u32, 1usize),
                (813, 2),
                (814, 2),
                (815, 2),
                (816, 4),
                (828, 1),
                (829, 2),
                (830, 2),
                (831, 4),
            ],
        ),
        (
            105,
            0x80_0000,
            vec![798, 817, 818, 819, 820, 822, 821, 832, 833, 834, 835],
            vec![
                (817, 1),
                (818, 2),
                (819, 2),
                (820, 2),
                (822, 1),
                (821, 4),
                (832, 1),
                (833, 2),
                (834, 2),
                (835, 4),
            ],
        ),
        (
            106,
            0,
            vec![811, 823, 824, 825, 826, 827, 832, 833, 834, 835],
            vec![
                (811, 1),
                (823, 1),
                (824, 2),
                (825, 2),
                (826, 2),
                (827, 4),
                (832, 1),
                (833, 2),
                (834, 2),
                (835, 4),
            ],
        ),
    ] {
        let mut e = Env::new(id, 20, 20);
        e.drlg.level_mut(e.l).level_type = 27;
        e.info.flags = flags;
        inert_subs(&mut e, &[1, 2, 3]);
        {
            let mut g = e.gen();
            g.polygon().unwrap();
            g.act4().unwrap();
        }
        assert_eq!(first_stamps(&e.info, keep), want, "level {id}");
        for (p, n) in counts {
            assert_eq!(count(&e, p), n, "level {id} preset {p}");
        }
        // Mesa borders (style 3): N edge Border(1, 0, 3) = 801.
        assert_eq!(e.info.grids[0].get(10, 0), 801);
        match flags {
            0x40_0000 => assert_eq!(e.info.grids[0].get(0, 1), 798),
            0x80_0000 => assert_eq!(e.info.grids[0].get(0, 4), 798),
            _ => assert_eq!(count(&e, 798), 0),
        }
    }
}

#[test]
fn link_driver_draws_on_a_copy_of_the_drlg_seed() {
    let mut d = data();
    set(&mut d, 39, 3, (64, 64), (0, 300));
    set(&mut d, 4, 3, (40, 40), (84, 300));
    set(&mut d, 5, 3, (80, 80), (0, 0));
    let rows = [
        LinkRow {
            linker: Linker::Def,
            level: 39,
            link: -1,
        },
        LinkRow {
            linker: Linker::Def,
            level: 4,
            link: -1,
        },
        LinkRow {
            linker: Linker::R4,
            level: 5,
            link: 0,
        },
    ];
    let mut drlg = Drlg::create(0, 7, 0, 0, false, &d, &mut NoLevelTypes).unwrap();
    let s0 = drlg.seed;
    let mut o = Outdoor::default();
    let dv = o
        .drive(&mut drlg, &d, &mut NoLevelTypes, &rows, Check::None, false)
        .unwrap();
    // The copy restarts from the DRLG seed and advances on its own.
    assert_ne!(dv.seed, s0);
    assert_eq!(drlg.seed, s0);
}

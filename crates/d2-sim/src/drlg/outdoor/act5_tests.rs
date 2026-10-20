//! Tests for the Act V build: act placement and the level 110 siege strip with fixed
//! seeds, and synthetic checks of each build step (barricade borders, ravine, entrances,
//! caves, siege connection, prisons, specials) on small fake levels.

use super::{CRASH_RAVINE_WALK, FATAL_PRISONS, FATAL_SPECIAL};
use crate::drlg::outdoor::grid::{cell, file_of, Op};
use crate::drlg::outdoor::tests::{data, od, one_cell_file, stepped, Env, Presets, Rec};
use crate::drlg::outdoor::*;
use crate::drlg::{Drlg, DrlgData, TileRect};

// ---- helpers ------------------------------------------------------------------

fn set(d: &mut DrlgData, id: usize, ty: u32, size: (i32, i32), off: (i32, i32)) {
    d.levels[id].drlg_type = ty;
    d.levels[id].size = [size; 3];
    d.levels[id].offset = off;
}

fn g0(e: &Env, x: i32, y: i32) -> u32 {
    e.info.grids[0].get(x, y)
}

fn g2(e: &Env, x: i32, y: i32) -> u32 {
    e.info.grids[2].get(x, y)
}

fn count(e: &Env, p: u32) -> usize {
    e.info.grids[0].cells.iter().filter(|&&c| c == p).count()
}

/// Presets in first-stamp order (the build list is head-inserted).
fn first_stamps(e: &Env) -> Vec<u32> {
    e.info.build_list.iter().rev().map(|n| n.preset).collect()
}

/// A type-12 lvlsub row whose only group is larger than any test grid:
/// border substitution skips it without a draw.
fn inert_barricade_sub(e: &mut Env) {
    e.od.subs.push(SubRow {
        type_: 12,
        file: b"inert12".to_vec(),
        bord_type: 1,
        grid_size: 1,
        ..SubRow::default()
    });
    let mut f = one_cell_file(0, (48 << 20) | (2 << 8) | 1, 1);
    f.groups[0].w = 100;
    f.groups[0].h = 100;
    e.subs.0.insert(b"inert12".to_vec(), f);
}

fn v(x: i32, y: i32, flags: u32) -> Vertex {
    Vertex {
        x,
        y,
        direction: 0,
        flags,
    }
}

// ---- placement and siege strip ---------------------------------------------------

#[test]
fn act5_placement_vector() {
    // Leveldefs of the Act V levels: 111 and 112 have size −1 (the linker sets their
    // sizes), and 117's 128×80 is overridden.
    let mut d = data();
    set(&mut d, 109, 2, (40, 40), (1000, 1000));
    set(&mut d, 110, 3, (240, 48), (760, 1000));
    set(&mut d, 111, 3, (-1, -1), (-1, -1));
    set(&mut d, 112, 3, (-1, -1), (-1, -1));
    set(&mut d, 117, 3, (128, 80), (2000, 1896));
    set(&mut d, 134, 3, (40, 40), (8000, 8000));
    set(&mut d, 136, 2, (40, 40), (9000, 9000));
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
    let drlg = Drlg::create(4, 644409375, 0, 0, false, &d, &mut types).unwrap();
    assert_eq!(types.last_error, None);
    // The first two draws of the Act V copy of the seed: 1406222081 and 3154683627.
    let mut copy = drlg.seed;
    assert_eq!(copy.step(), 1406222081);
    assert_eq!(copy.step(), 3154683627);
    let rect = |id| drlg.level(drlg.find_level(id).unwrap()).rect;
    assert_eq!(rect(110), TileRect::new(760, 1000, 240, 48));
    assert_eq!(rect(111), TileRect::new(600, 968, 160, 64));
    assert_eq!(rect(112), TileRect::new(440, 968, 160, 64));
    assert_eq!(rect(117), TileRect::new(2000, 1896, 160, 64));
    let grid = |id| {
        let r = rect(id);
        (r.w / 8, r.h / 8)
    };
    assert_eq!(grid(110), (30, 6));
    assert_eq!(grid(111), (20, 8));
    assert_eq!(grid(112), (20, 8));
    assert_eq!(grid(117), (20, 8));
    // Neighbour entries: 111 opens only toward 112, 112 only toward 111.
    let orth = |id| -> Vec<u32> {
        outdoor
            .level(drlg.find_level(id).unwrap())
            .map(|o| o.orth.iter().map(|e| e.level_id).collect())
            .unwrap_or_default()
    };
    assert_eq!(orth(111), [112]);
    assert_eq!(orth(112), [111]);
}

#[test]
fn bloody_foothills_siege_strip_vector() {
    // 865 is 16 tiles wide (2 cells): 865 + i at (28 − 2i, 0), F 0.
    let mut e = Env::new(110, 30, 6);
    for p in 865..=879 {
        e.od.presets[p] = PresetDef {
            size_x: 16,
            size_y: 48,
            files: 1,
        };
    }
    let s0 = e.seed();
    e.gen().act5().unwrap();
    for i in 0..15 {
        let x = 28 - 2 * i;
        assert_eq!(g0(&e, x, 0), 865 + i as u32, "piece {i}");
        assert_eq!(g0(&e, x + 1, 0), 0);
        for y in 0..6 {
            for xx in [x, x + 1] {
                assert_eq!(g2(&e, xx, y), cell::PRESET, "({xx}, {y})");
            }
        }
    }
    assert_eq!(g0(&e, 0, 0), 879);
    // No draw, no build list, no link flags.
    assert_eq!(e.seed(), s0);
    assert!(e.info.build_list.is_empty());
    assert!(e.info.grids[1].cells.iter().all(|&c| c == 0));
}

// ---- build steps -------------------------------------------------------------------

#[test]
fn barricade_border_walk_plain_rect() {
    // 8×6 cells: vertices (0,5) (0,0) (7,0) (7,5), cleared (0,4) (0,0)
    // (6,0) (6,4). Straight Q[N[dx + 3dy + 4]]: W 882, N 883, E 884,
    // S 881. Corners Q[N[b + a' + 9(e + c') + 50] − 1] with doubled
    // directions grown in a, c: NW (0,−2,2,0) → N[84] 6 → 886; NE
    // (2,0,0,2) → N[72] 7 → 887; SE (0,2,−2,0) → N[16] 8 → 888; SW
    // (−2,0,0,−2) → N[28] 5 → 885.
    let mut e = Env::new(112, 8, 6);
    let mut g = e.gen();
    g.polygon().unwrap();
    g.barricade_borders().unwrap();
    let want = [
        ((0, 0), 886),
        ((2, 0), 883),
        ((4, 0), 883),
        ((6, 0), 887),
        ((6, 2), 884),
        ((6, 4), 888),
        ((4, 4), 881),
        ((2, 4), 881),
        ((0, 4), 885),
        ((0, 2), 882),
    ];
    for ((x, y), p) in want {
        assert_eq!(g0(&e, x, y), p, "({x}, {y})");
        assert_eq!(g2(&e, x, y), cell::PRESET | cell::BORDER, "({x}, {y})");
    }
    // Odd cells and the inside are untouched.
    assert_eq!(g2(&e, 1, 0), 0);
    assert_eq!(g2(&e, 7, 0), 0);
    assert_eq!(g2(&e, 2, 2), 0);
    // First stamps: walk then corner, edge by edge from the head.
    assert_eq!(first_stamps(&e), [882, 886, 883, 887, 884, 888, 881, 885]);
    // Level 117 uses the snow column (957 + k).
    let mut e = Env::new(117, 8, 6);
    let mut g = e.gen();
    g.polygon().unwrap();
    g.barricade_borders().unwrap();
    assert_eq!(g0(&e, 2, 0), 959);
    assert_eq!(g0(&e, 6, 0), 963);
    assert_eq!(g0(&e, 0, 2), 958);
}

#[test]
fn barricade_border_walk_links_and_preset_links() {
    // Head (0,5) link: W edge (0,5)→(0,0), dy −1: x := 0, y := (5 − 4)
    // & ~1 = 0 → 0x400 at (0,0) and (0,2). (0,0) preset link: no N walk,
    // and as a link: x := (7 − 4) & ~1 = 2 → 0x400 at (2,0), (4,0).
    let mut e = Env::new(112, 8, 6);
    e.info.vertices = vec![
        v(0, 5, VERTEX_LINK),
        v(0, 0, VERTEX_LINK | VERTEX_PRESET_LINK),
        v(7, 0, 0),
        v(7, 5, 0),
    ];
    e.gen().barricade_borders().unwrap();
    assert_ne!(g2(&e, 0, 0) & cell::LINK, 0);
    assert_ne!(g2(&e, 0, 2) & cell::LINK, 0);
    assert_eq!(g2(&e, 2, 0), cell::LINK);
    assert_eq!(g2(&e, 4, 0), cell::LINK);
    assert_eq!(g0(&e, 2, 0), 0);
    assert_eq!(g0(&e, 4, 0), 0);
    // The W walk still runs; the corners are stamped either way.
    assert_eq!(g0(&e, 0, 2), 882);
    assert_eq!(g0(&e, 0, 0), 886);
    assert_eq!(g0(&e, 6, 0), 887);
    assert_eq!(g2(&e, 2, 2) & cell::LINK, 0);
    // Level 111 marks (gw−2, gh−4) and (gw−2, gh−3) after the walk.
    let mut e = Env::new(111, 8, 6);
    e.gen().polygon().unwrap();
    e.gen().barricade_borders().unwrap();
    assert_eq!(g2(&e, 6, 2), cell::PRESET | cell::BORDER | cell::LINK);
    assert_eq!(g2(&e, 6, 3), cell::LINK);
    assert_eq!(g2(&e, 6, 1) & cell::LINK, 0);
}

#[test]
fn ravine_walk_follows_the_table() {
    // Hand-built grid 0 on 8×6: from (6,0) 887 (k 6, south), (6,2) 884
    // (k 3, south), (6,4) 888 (k 7, west), (4,4) 881, (2,4) 881 (k 0,
    // west), stop at (0,4) = (0, gh − 2).
    let mut e = Env::new(112, 8, 6);
    for ((x, y), p) in [
        ((6, 0), 887),
        ((6, 2), 884),
        ((6, 4), 888),
        ((4, 4), 881),
        ((2, 4), 881),
        ((0, 4), 885),
        ((0, 2), 882),
    ] {
        e.gen().op(0, x, y, Op::Set, p);
    }
    e.gen().ravine().unwrap();
    assert_eq!(g0(&e, 6, 2), 893 + 3);
    assert_eq!(g0(&e, 6, 4), 893 + 7);
    assert_eq!(g0(&e, 4, 4), 893);
    assert_eq!(g0(&e, 2, 4), 893);
    // Ends: 906 at (gw−2, 0) over 899, 905 at (0, gh−2).
    assert_eq!(g0(&e, 6, 0), 906);
    assert_eq!(g0(&e, 0, 4), 905);
    // (0,2) is off the path.
    assert_eq!(g0(&e, 0, 2), 882);
    assert_eq!(first_stamps(&e), [899, 896, 900, 893, 906, 905]);

    // Level 117: B 957, B' 969, ends 982 / 981: k 3 south, k 8 west,
    // k 3 south, k 0 west.
    let mut e = Env::new(117, 6, 6);
    for ((x, y), p) in [
        ((4, 0), 957 + 3),
        ((4, 2), 957 + 8),
        ((2, 2), 957 + 3),
        ((2, 4), 957),
    ] {
        e.gen().op(0, x, y, Op::Set, p);
    }
    e.gen().ravine().unwrap();
    assert_eq!(first_stamps(&e), [972, 977, 969, 982, 981]);
    assert_eq!(g0(&e, 4, 0), 982);
    assert_eq!(g0(&e, 4, 2), 977);
    assert_eq!(g0(&e, 2, 2), 972);
    assert_eq!(g0(&e, 2, 4), 969);
    assert_eq!(g0(&e, 0, 4), 981);
}

#[test]
fn ravine_walk_off_the_pieces_is_reported() {
    // (gw − 2, 0) holds no barricade piece: k = −881 has no D row; stamping
    // that non-row id would crash, so D2RUST reports it before any stamp.
    let mut e = Env::new(112, 8, 6);
    assert_eq!(
        e.gen().ravine(),
        Err(OutdoorError::Crash(CRASH_RAVINE_WALK))
    );
    assert!(e.info.grids[0].cells.iter().all(|&c| c == 0));
    // A closed loop of pieces that never reaches (0, gh − 2): k 3 south
    // then k 1 north, which never ends.
    let mut e = Env::new(112, 8, 6);
    e.gen().op(0, 6, 0, Op::Set, 881 + 3);
    e.gen().op(0, 6, 2, Op::Set, 881 + 1);
    assert_eq!(
        e.gen().ravine(),
        Err(OutdoorError::Crash(CRASH_RAVINE_WALK))
    );
}

#[test]
fn entrances_take_the_first_link_cell() {
    let mut e = Env::new(112, 10, 8);
    for (x, y) in [
        (7, 0),
        (3, 0),
        (5, 6),
        (1, 6),
        (0, 4),
        (0, 2),
        (8, 5),
        (9, 1),
        (2, 7),
    ] {
        e.gen().op(2, x, y, Op::Or, cell::LINK);
    }
    e.gen().entrances().unwrap();
    assert_eq!(g0(&e, 3, 0), 909);
    assert_eq!(g0(&e, 7, 0), 0);
    assert_eq!(g0(&e, 1, 6), 908);
    assert_eq!(g0(&e, 5, 6), 0);
    assert_eq!(g0(&e, 0, 2), 910);
    assert_eq!(g0(&e, 0, 4), 0);
    // East column gw − 2 = 8: (8,5); column 9 is not scanned.
    assert_eq!(g0(&e, 8, 5), 907);
    assert_eq!(g0(&e, 9, 1), 0);
    assert_eq!(first_stamps(&e), [909, 908, 910, 907]);
    // No link cell: nothing, no draw.
    let mut e = Env::new(112, 10, 8);
    let s0 = e.seed();
    e.gen().entrances().unwrap();
    assert_eq!(e.seed(), s0);
    assert!(e.info.grids[0].cells.iter().all(|&c| c == 0));
}

#[test]
fn caves_tall_and_wide() {
    type Case = (u32, i32, i32, &'static [((i32, i32), u32)]);
    let cases: [Case; 4] = [
        (112, 20, 8, &[((0, 2), 914)]),
        (112, 8, 20, &[((2, 0), 913)]),
        (117, 20, 8, &[((18, 2), 984), ((0, 2), 986)]),
        (117, 8, 20, &[((2, 18), 983), ((2, 0), 985)]),
    ];
    for (id, gw, gh, want) in cases {
        let mut e = Env::new(id, gw, gh);
        let s0 = e.seed();
        e.gen().caves().unwrap();
        for &((x, y), p) in want {
            assert_eq!(g0(&e, x, y), p, "level {id} {gw}×{gh}");
            // F 0 from the row: file 0, no build-list draw.
            assert_eq!(g2(&e, x, y), cell::PRESET);
        }
        let stamped = e.info.grids[0].cells.iter().filter(|&&c| c != 0).count();
        assert_eq!(stamped, want.len());
        assert_eq!(e.seed(), s0);
        assert!(e.info.build_list.is_empty());
    }
    // Level 111 has no cave row.
    let mut e = Env::new(111, 20, 8);
    e.gen().caves().unwrap();
    assert!(e.info.grids[0].cells.iter().all(|&c| c == 0));
    // w = h (level tile rect): the tall id.
    let mut e = Env::new(112, 10, 10);
    e.gen().caves().unwrap();
    assert_eq!(g0(&e, 2, 0), 913);
}

#[test]
fn connect_to_siege_positions() {
    // 880 is 32×16 tiles (4×2 cells): x = 20 − 4, y = 8 − 2; 896 two
    // cells above.
    let mut e = Env::new(111, 20, 8);
    e.od.presets[880] = PresetDef {
        size_x: 32,
        size_y: 16,
        files: 1,
    };
    e.gen().connect_to_siege().unwrap();
    assert_eq!(g0(&e, 16, 6), 880);
    assert_eq!(g0(&e, 16, 4), 896);
    assert_eq!(g2(&e, 19, 7), cell::PRESET);
    assert_eq!(first_stamps(&e), [880, 896]);
    // A missing lvlprest 880 is fatal.
    let mut e = Env::new(111, 20, 8);
    e.od.presets.truncate(880);
    assert_eq!(
        e.gen().connect_to_siege(),
        Err(OutdoorError::UnknownPreset(880))
    );
}

/// Grid of `gw × gh` with preset cells `v` at `cells` (grid 2 0x200).
fn prison_env(gw: i32, gh: i32, cells: &[((i32, i32), u32)]) -> Env {
    let mut e = Env::new(111, gw, gh);
    for &((x, y), p) in cells {
        e.gen().op(0, x, y, Op::Set, p);
        e.gen().op(2, x, y, Op::Or, cell::PRESET);
    }
    e
}

#[test]
fn prisons_draw_order_and_fatal() {
    // Nothing to place: 90 tries of two rolls, then roll(gw/2),
    // roll(gh/2), then fatal 0x259. Values out of 915..922 or without
    // 0x200 never count.
    let mut e = prison_env(8, 6, &[((2, 2), 914), ((4, 2), 923)]);
    e.gen().op(0, 2, 4, Op::Set, 915);
    let s0 = e.seed();
    assert_eq!(e.gen().prisons(), Err(OutdoorError::Fatal(FATAL_PRISONS)));
    assert_eq!(FATAL_PRISONS, 0x259);
    assert_eq!(e.seed(), stepped(s0, 182));
    assert_eq!(g0(&e, 2, 4), 915);

    // Every even cell holds 915: the random tries place three (a repeat
    // cell holds 931 and fails); one build-list roll at the first stamp.
    let cells: Vec<_> = (0..3)
        .flat_map(|y| (0..4).map(move |x| ((2 * x, 2 * y), 915)))
        .collect();
    let mut e = prison_env(8, 6, &cells);
    let mut s = e.seed();
    let mut want = Vec::new();
    let mut steps = 0;
    while want.len() < 3 {
        let x = 2 * s.roll(4) as i32;
        let y = 2 * s.roll(3) as i32;
        steps += 2;
        if !want.contains(&(x, y)) {
            want.push((x, y));
            if want.len() == 1 {
                s.step();
                steps += 1;
            }
        }
    }
    let s0 = e.seed();
    e.gen().prisons().unwrap();
    assert_eq!(e.seed(), stepped(s0, steps + 2));
    for &(x, y) in &want {
        assert_eq!(g0(&e, x, y), 931);
    }
    assert_eq!(count(&e, 931), 3);

    // Odd cells are never drawn: the tries fail, then the scan from
    // (2sx, 2sy) (row-major, wrapping) places the first three of four.
    let cand = [((1, 1), 915), ((3, 1), 917), ((5, 3), 920), ((7, 5), 922)];
    let mut e = prison_env(8, 6, &cand);
    let s0 = e.seed();
    let mut s = stepped(s0, 180);
    let sx = s.roll(4) as i32;
    let sy = s.roll(3) as i32;
    let mut order = Vec::new();
    for i in 0..6 {
        for j in 0..8 {
            let c = ((j + 2 * sx) % 8, (i + 2 * sy) % 6);
            if let Some(&(_, p)) = cand.iter().find(|(xy, _)| *xy == c) {
                order.push((c, p));
            }
        }
    }
    e.gen().prisons().unwrap();
    for &((x, y), p) in &order[..3] {
        assert_eq!(g0(&e, x, y), p + 16);
    }
    let ((x, y), p) = order[3];
    assert_eq!(g0(&e, x, y), p);
    // 180 + 2 rolls, then three new presets: three build-list rolls.
    assert_eq!(e.seed(), stepped(s0, 185));
    let placed: Vec<u32> = order[..3].iter().map(|&(_, p)| p + 16).collect();
    assert_eq!(first_stamps(&e), placed);
}

#[test]
fn special_presets_rows_and_tall_wide() {
    // 111 wide (20×8): 956 F 0, 947, 945 ×4, 946 ×4; each call one
    // shuffle of 2·18·6 draws, plus a build-list roll for each new
    // F −1 preset (947, 945, 946).
    let mut e = Env::new(111, 20, 8);
    let s0 = e.seed();
    e.gen().act5_specials().unwrap();
    assert_eq!([956, 947, 945, 946].map(|p| count(&e, p)), [1, 1, 4, 4]);
    assert_eq!([955, 944, 942, 943].map(|p| count(&e, p)), [0; 4]);
    assert_eq!(e.seed(), stepped(s0, 10 * 216 + 3));
    assert_eq!(first_stamps(&e), [947, 945, 946]);
    // 111 tall (8×20): the tall ids.
    let mut e = Env::new(111, 8, 20);
    e.gen().act5_specials().unwrap();
    assert_eq!([955, 944, 942, 943].map(|p| count(&e, p)), [1, 1, 4, 4]);
    // w = h (level tile rect): the wide ids (the opposite tie rule to the caves).
    let mut e = Env::new(111, 12, 12);
    e.gen().act5_specials().unwrap();
    assert_eq!([956, 947, 945, 946].map(|p| count(&e, p)), [1, 1, 4, 4]);
    assert_eq!([955, 944, 942, 943].map(|p| count(&e, p)), [0; 4]);
    // 112: 956 F 0, 953, 941, 939, 940 ×5.
    let mut e = Env::new(112, 20, 8);
    e.gen().act5_specials().unwrap();
    assert_eq!(
        [956, 953, 941, 939, 940].map(|p| count(&e, p)),
        [1, 1, 1, 1, 5]
    );
    assert_eq!(first_stamps(&e), [953, 941, 939, 940]);
    // 117: 956 with F 1, 954, 948 ×4, 949 ×4, 950 ×4, 951 ×3.
    let mut e = Env::new(117, 20, 8);
    e.gen().act5_specials().unwrap();
    assert_eq!(
        [956, 954, 948, 949, 950, 951].map(|p| count(&e, p)),
        [1, 1, 4, 4, 4, 3]
    );
    let at = e.info.grids[0]
        .cells
        .iter()
        .position(|&c| c == 956)
        .unwrap() as i32;
    assert_eq!(file_of(g2(&e, at % 20, at / 20)), 1);
}

#[test]
fn special_presets_fatal_only_on_fatal_rows() {
    // Nothing fits: the first row (fatal) places nothing → 0x219 after
    // one shuffle.
    let mut e = Env::new(112, 20, 8);
    e.info.grids[2].cells.fill(cell::BLANK);
    let s0 = e.seed();
    assert_eq!(
        e.gen().act5_specials(),
        Err(OutdoorError::Fatal(FATAL_SPECIAL))
    );
    assert_eq!(FATAL_SPECIAL, 0x219);
    assert_eq!(e.seed(), stepped(s0, 216));
    // A non-fatal row that places nothing (947 too large) is passed.
    let mut e = Env::new(111, 20, 8);
    e.od.presets[947] = PresetDef {
        size_x: 800,
        size_y: 800,
        files: 1,
    };
    e.gen().act5_specials().unwrap();
    assert_eq!(count(&e, 947), 0);
    assert_eq!(count(&e, 945), 4);
    // A fatal row other than the first: 953 does not fit.
    let mut e = Env::new(112, 20, 8);
    e.od.presets[953] = PresetDef {
        size_x: 800,
        size_y: 800,
        files: 1,
    };
    assert_eq!(
        e.gen().act5_specials(),
        Err(OutdoorError::Fatal(FATAL_SPECIAL))
    );
    assert_eq!(count(&e, 956), 1);
    assert_eq!(count(&e, 941), 0);
}

#[test]
fn act5_build_order() {
    // Level 112, 8×6, no neighbours: border walk, ravine, (no entrance),
    // cave 914 F 0, border substitution (inert row), specials.
    let mut e = Env::new(112, 8, 6);
    inert_barricade_sub(&mut e);
    e.gen().polygon().unwrap();
    e.gen().act5().unwrap();
    assert_eq!(
        first_stamps(&e),
        [
            882, 886, 883, 887, 884, 888, 881, 885, 899, 896, 900, 893, 906, 905, 953, 941, 939,
            940
        ]
    );
    assert_eq!(g0(&e, 0, 2), 914);
    assert_eq!(count(&e, 956), 1);
    assert_eq!(count(&e, 940), 5);
    // Border substitution runs after the ravine with the barricade callbacks: a
    // pattern of style (48, 8) (style map → 896) matches the ravine's
    // 893 + 3 at (6, 2) and its variant (48, 5) stamps 895 there.
    let mut e = Env::new(112, 8, 6);
    e.od.subs.push(SubRow {
        type_: 12,
        file: b"b12".to_vec(),
        bord_type: 1,
        grid_size: 1,
        ..SubRow::default()
    });
    let wall = |v: u32| (48 << 20) | (v << 8) | 1;
    let mut f = one_cell_file(0, wall(8), 1);
    f.walls[0].set(2, 0, wall(5));
    e.subs.0.insert(b"b12".to_vec(), f);
    e.gen().polygon().unwrap();
    e.gen().act5().unwrap();
    assert_eq!(g0(&e, 6, 2), 895);
    assert_eq!(count(&e, 896), 0);
    // No type-12 lvlsub rows: the substitution step is fatal.
    let mut e = Env::new(112, 8, 6);
    e.gen().polygon().unwrap();
    assert_eq!(e.gen().act5(), Err(OutdoorError::NoSubRows(12)));
    // Level 111 runs connect-to-siege and prisons: with no prison cells
    // the build stops with 0x259 (after the substitution step).
    let mut e = Env::new(111, 20, 8);
    inert_barricade_sub(&mut e);
    e.gen().polygon().unwrap();
    assert_eq!(e.gen().act5(), Err(OutdoorError::Fatal(FATAL_PRISONS)));
    assert_eq!(g0(&e, 19, 7), 880);
    assert_eq!(g0(&e, 19, 5), 896);
    // Link flags run first: a link vertex marks grid 2 bits 0x1
    // on its edge (and is a barricade link).
    let mut e = Env::new(112, 8, 6);
    inert_barricade_sub(&mut e);
    e.info.vertices = vec![v(0, 5, 0), v(0, 0, VERTEX_LINK), v(7, 0, 0), v(7, 5, 0)];
    e.gen().act5().unwrap();
    assert_ne!(g2(&e, 1, 0) & cell::BORDER, 0);
    assert_eq!(g0(&e, 2, 0), 909);
}

#[test]
fn ravine_table_matches_spec() {
    assert_eq!(
        super::RAVINE_STEPS,
        [
            (-1, 0),
            (0, -1),
            (1, 0),
            (0, 1),
            (0, -1),
            (1, 0),
            (0, 1),
            (-1, 0),
            (-1, 0),
            (0, -1),
            (1, 0),
            (0, 1)
        ]
    );
}

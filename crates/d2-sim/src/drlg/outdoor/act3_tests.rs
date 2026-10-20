//! Tests for the Act III build: act creation with fixed seeds (jungle placer, hand-off to
//! levels 76..=78, Kurast chain), jungle stamping with fixed ids, and rule checks of the
//! block grids and the block code table.

use super::*;
use crate::drlg::outdoor::grid::file_of;
use crate::drlg::outdoor::tests::{data, od, Env, Presets, Rec};
use crate::drlg::outdoor::{Outdoor, OutdoorData, OutdoorTypes, PresetDef, SubFileMap};
use crate::drlg::{Drlg, DrlgData};

// ---- fixtures ------------------------------------------------------------------

/// Leveldefs for the Act III tests (DrlgType 2 for level 75, 3 for 76..=83).
fn act3_data() -> DrlgData {
    let mut d = data();
    let mut set = |id: usize, ty: u32, size: (i32, i32), off: (i32, i32)| {
        d.levels[id].drlg_type = ty;
        d.levels[id].size = [size; 3];
        d.levels[id].offset = off;
    };
    set(75, 2, (64, 48), (1000, 1000));
    for id in 76..=78 {
        set(id, 3, (64, 192), (-1, -1));
    }
    for id in 79..=81 {
        set(id, 3, (80, 64), (0, 0));
    }
    set(82, 3, (48, 16), (0, 0));
    set(83, 3, (64, 64), (0, 0));
    d
}

/// Lvlprest rows for the Act III tests: 530–544 Files 3 (541: 5), 545–572 Files 1,
/// 573/574 Files 0 (64×32), 575–604 Files 1; 32×32 otherwise.
fn act3_od() -> OutdoorData {
    let mut od = od();
    let mut set = |p: usize, size_x: i32, size_y: i32, files: i32| {
        od.presets[p] = PresetDef {
            size_x,
            size_y,
            files,
        };
    };
    for p in 530..=544 {
        set(p, 32, 32, 3);
    }
    set(541, 32, 32, 5);
    for p in 545..=572 {
        set(p, 32, 32, 1);
    }
    set(573, 64, 32, 0);
    set(574, 64, 32, 0);
    for p in 575..=604 {
        set(p, 32, 32, 1);
    }
    od
}

/// Act III creation from init seed 644409375, difficulty 0.
fn create_act3() -> (Drlg, Outdoor, DrlgData) {
    let data = act3_data();
    let od = act3_od();
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
    let drlg = Drlg::create(2, 644_409_375, 0, 0, false, &data, &mut types).unwrap();
    assert_eq!(types.last_error, None);
    (drlg, outdoor, data)
}

/// Steps from `from` to `to` (None past 10 000).
fn steps_between(from: Seed, to: Seed) -> Option<usize> {
    let mut s = from;
    for n in 0..10_000 {
        if s == to {
            return Some(n);
        }
        s.step();
    }
    None
}

const DRLG_AT_PLACER: Seed = Seed::new(1_406_222_081, 1_674_353_446);
const DOCKS: TileRect = TileRect::new(1000, 1000, 64, 48);

const IDS_76: [u32; 12] = [541, 533, 543, 565, 570, 582, 571, 575, 570, 575, 537, 0];
const IDS_77: [u32; 12] = [554, 577, 541, 0, 539, 534, 0, 541, 576, 569, 576, 566];
const IDS_78: [u32; 12] = [0, 533, 535, 565, 570, 582, 539, 534, 558, 565, 541, 581];

// ---- act creation -------------------------------------------------------------

#[test]
fn jungle_placer_derived_vector() {
    // The draws for the second jungle (k = 1).
    let mut s = DRLG_AT_PLACER;
    assert_eq!(s.step(), 3_154_683_627);
    assert_eq!(s.step(), 457_460_266);
    assert_eq!(457_460_266 % 5, 1);

    let mut seed = DRLG_AT_PLACER;
    let j = place_jungles(&mut seed, DOCKS, 64, 192).unwrap();
    // Placement order: docks jungle, case 1 on 0, case 0 on 0.
    let rects: Vec<TileRect> = j.recs.iter().map(|r| r.rect).collect();
    assert_eq!(
        rects,
        [
            TileRect::new(1000, 808, 64, 192),
            TileRect::new(936, 744, 64, 192),
            TileRect::new(1000, 616, 64, 192),
        ]
    );
    assert_eq!((j.recs[1].case, j.recs[1].base), (1, 0));
    assert_eq!((j.recs[2].case, j.recs[2].base), (0, 0));
    assert_eq!(j.recs[0].branches, [1, 2]);
    // Grid W 6, H 14; blocks (bx, by).
    assert_eq!((j.grid.w, j.grid.h), (6, 14));
    let blocks: Vec<(i32, i32)> = j.recs.iter().map(|r| (r.bx, r.by)).collect();
    assert_eq!(blocks, [(3, 7), (1, 5), (3, 1)]);
    // 4 + 4 + 1 + 12 + 2 + 84 + 8 + 8 = 123 steps (no attach restart).
    assert_eq!(seed, Seed::new(4_015_082_244, 577_631_236));
    assert_eq!(steps_between(DRLG_AT_PLACER, seed), Some(123));
    // Ids and clearing counts per record; sorted order 0, 1, 2 (y 808,
    // 744, 616).
    assert_eq!(j.order, [0, 1, 2]);
    assert_eq!(j.recs[0].ids, IDS_76);
    assert_eq!(j.recs[1].ids, IDS_77);
    assert_eq!(j.recs[2].ids, IDS_78);
    let clearings: Vec<i32> = j.recs.iter().map(|r| r.clearings).collect();
    assert_eq!(clearings, [3, 3, 2]);
    // River orders stay in 100(k+1) .. 100(k+1) + 2·SYb − 1.
    for (i, &b) in j.grid.b.iter().enumerate() {
        if b != 0 {
            let k = j.grid.a[i];
            assert!((100 * k..100 * k + 12).contains(&b), "B {b} owner {k}");
        }
    }
}

#[test]
fn act3_creation_derived_vector() {
    let (drlg, o, _) = create_act3();
    assert_eq!(drlg.start_seed, 4_014_346_869);
    assert_eq!(drlg.seed, Seed::new(4_015_082_244, 577_631_236));
    let rect = |id| drlg.level(drlg.find_level(id).unwrap()).rect;
    let info = |id| o.level(drlg.find_level(id).unwrap()).unwrap();
    assert_eq!(rect(75), DOCKS);
    assert_eq!(rect(76), TileRect::new(1000, 808, 64, 192));
    assert_eq!(rect(77), TileRect::new(936, 744, 64, 192));
    assert_eq!(rect(78), TileRect::new(1000, 616, 64, 192));
    assert_eq!(info(76).jungle_ids.as_deref(), Some(&IDS_76[..]));
    assert_eq!(info(77).jungle_ids.as_deref(), Some(&IDS_77[..]));
    assert_eq!(info(78).jungle_ids.as_deref(), Some(&IDS_78[..]));
    assert_eq!(
        (
            info(76).jungle_clearings,
            info(77).jungle_clearings,
            info(78).jungle_clearings
        ),
        (3, 3, 2)
    );
    // Allocation order: 75, then 76, 77, 78, then the chain 79..83.
    let mut order: Vec<u32> = drlg
        .level_list()
        .into_iter()
        .map(|l| drlg.level(l).id)
        .collect();
    order.reverse();
    assert_eq!(order, [75, 76, 77, 78, 79, 80, 81, 82, 83]);
}

#[test]
fn kurast_chain_anchors_on_level_78() {
    let (drlg, _, _) = create_act3();
    let rect = |id| drlg.level(drlg.find_level(id).unwrap()).rect;
    assert_eq!(rect(79), TileRect::new(992, 552, 80, 64));
    assert_eq!(rect(80), TileRect::new(992, 488, 80, 64));
    assert_eq!(rect(81), TileRect::new(992, 424, 80, 64));
    assert_eq!(rect(82), TileRect::new(1008, 408, 48, 16));
    assert_eq!(rect(83), TileRect::new(1000, 344, 64, 64));
}

// ---- jungle stamping -----------------------------------------------------

/// Level `id` (8 × 24 cells) with the given ids and clearing count,
/// level seed {dwStartSeed + id, 666}.
fn stamp_env(id: u32, ids: &[u32], clearings: i32) -> Env {
    let mut e = Env::new(id, 8, 24);
    e.data.levels[76].size = [(64, 192); 3];
    e.od = act3_od();
    e.info.jungle_ids = Some(ids.to_vec());
    e.info.jungle_clearings = clearings;
    e.drlg.level_mut(e.l).seed = Seed::new(4_014_346_869u32.wrapping_add(id), 666);
    e
}

/// Stamps as (cell x, cell y, P, F): grid 0 in row-major order (the
/// stamping order), F from grid 2.
fn stamps(e: &Env) -> Vec<(i32, i32, u32, i32)> {
    let g = &e.info.grids;
    let mut out = Vec::new();
    for y in 0..g[0].h {
        for x in 0..g[0].w {
            let p = g[0].get(x, y);
            if p != 0 {
                out.push((x, y, p, file_of(g[2].get(x, y))));
            }
        }
    }
    out
}

fn run_stamping(id: u32, ids: &[u32], clearings: i32) -> (Env, Seed) {
    let mut e = stamp_env(id, ids, clearings);
    let s0 = e.seed();
    e.gen().jungle_stamping().unwrap();
    (e, s0)
}

#[test]
fn jungle_stamping_derived_vectors() {
    // 76: r = 1 from the low seed word 4139712799 (roll(6)).
    let mut s = Seed::new(4_014_346_869 + 76, 666);
    let lo = s.step();
    assert_eq!(lo, 4_139_712_799);
    assert_eq!(lo % 6, 1);
    let (e, s0) = run_stamping(76, &IDS_76, 3);
    assert_eq!(
        stamps(&e),
        [
            (0, 0, 541, 4),
            (4, 0, 533, 2),
            (0, 4, 543, 0),
            (4, 4, 565, 0),
            (0, 8, 570, 0),
            (4, 8, 582, 1),
            (0, 12, 571, 0),
            (4, 12, 575, 0),
            (0, 16, 570, 0),
            (4, 16, 575, 2),
            (0, 20, 573, 1),
        ]
    );
    assert_eq!(e.seed(), Seed::new(2_661_750_284, 470_510_827));
    assert_eq!(steps_between(s0, e.seed()), Some(7));

    // 77: r = 0; Boggy clearings (+10).
    let (e, s0) = run_stamping(77, &IDS_77, 3);
    assert_eq!(
        stamps(&e),
        [
            (0, 0, 554, 0),
            (4, 0, 587, 0),
            (0, 4, 541, 4),
            (0, 8, 539, 1),
            (4, 8, 534, 1),
            (4, 12, 541, 0),
            (0, 16, 586, 1),
            (4, 16, 569, 0),
            (0, 20, 586, 2),
            (4, 20, 566, 0),
        ]
    );
    assert_eq!(e.seed(), Seed::new(2_370_309_536, 189_899_886));
    assert_eq!(steps_between(s0, e.seed()), Some(7));

    // 78: r = 1 of roll(2); Pygmy clearings (+20); tail row.
    let (e, s0) = run_stamping(78, &IDS_78, 2);
    assert_eq!(
        stamps(&e),
        [
            (0, 0, 574, 0),
            (0, 4, 535, 2),
            (4, 4, 565, 0),
            (0, 8, 570, 0),
            (4, 8, 602, 1),
            (0, 12, 539, 2),
            (4, 12, 534, 2),
            (0, 16, 558, 0),
            (4, 16, 565, 0),
            (0, 20, 541, 4),
            (4, 20, 601, 0),
        ]
    );
    assert_eq!(e.seed(), Seed::new(4_128_120_053, 867_080_307));
    assert_eq!(steps_between(s0, e.seed()), Some(8));
}

#[test]
fn jungle_rooms_after_stamping() {
    for (id, ids, clearings, preset_rooms, outdoor_rooms) in [
        (76, &IDS_76, 3, 11, 0),
        (77, &IDS_77, 3, 10, 32),
        (78, &IDS_78, 2, 11, 0),
    ] {
        let (mut e, _) = run_stamping(id, ids, clearings);
        let mut o = Outdoor::default();
        let mut presets = Presets::default();
        o.cells_to_rooms(&mut e.gen(), &mut presets).unwrap();
        assert_eq!(presets.calls.len(), preset_rooms, "level {id}");
        assert_eq!(e.drlg.level_rooms(e.l).len(), outdoor_rooms, "level {id}");
    }
}

#[test]
fn jungle_stamping_without_ids_is_fatal_after_the_roll() {
    let mut e = stamp_env(77, &[], 0);
    e.info.jungle_ids = None;
    let s0 = e.seed();
    assert_eq!(e.gen().jungle_stamping(), Err(OutdoorError::Fatal(0x27)));
    // r was drawn first (roll(2)).
    let mut s = s0;
    s.roll(2);
    assert_eq!(e.seed(), s);
}

#[test]
fn jungle_stamping_rejects_a_short_id_array() {
    // Creation and build read the same leveldefs 76 row, so the array always holds
    // SXb·SYb = 12 ids; a shorter one is an error, reported after the file-order roll and
    // before any stamp.
    let mut e = stamp_env(77, &[0; 11], 0);
    let s0 = e.seed();
    assert_eq!(
        e.gen().jungle_stamping(),
        Err(OutdoorError::JungleIdsShort(11, 12))
    );
    let mut s = s0;
    s.roll(2);
    assert_eq!(e.seed(), s);
    assert!(stamps(&e).is_empty());
}

#[test]
fn jungle_stamping_fourth_clearing_is_fatal() {
    let ids = [575, 576, 577, 578, 0, 0, 0, 0, 0, 0, 0, 0];
    let mut e = stamp_env(77, &ids, 4);
    assert_eq!(e.gen().jungle_stamping(), Err(OutdoorError::Fatal(0x47)));
    // Three clearings were stamped before the fatal one, with the first three files of the
    // chosen order (clearing count ≠ 3: roll(2)).
    assert_eq!(stamps(&e).len(), 3);
}

// ---- placer fatals ------------------------------------------------------------

#[test]
fn jungle_spans_not_multiple_of_32_are_fatal() {
    // SX 40: x spans 40, 80, 120.
    let mut s = DRLG_AT_PLACER;
    assert_eq!(
        place_jungles(&mut s, DOCKS, 40, 192),
        Err(OutdoorError::Fatal(0x64D))
    );
    // SY 200: every y span is 200 plus sums of 200, 66 and 133.
    let mut s = DRLG_AT_PLACER;
    assert_eq!(
        place_jungles(&mut s, DOCKS, 64, 200),
        Err(OutdoorError::Fatal(0x64E))
    );
}

// ---- code table ----------------------------------------------------------------

#[test]
fn code_to_id_table() {
    assert_eq!(code_to_id(0), Ok(0));
    // River pieces 530..544.
    for l in 1..16 {
        assert_eq!(code_to_id(l), Ok(529 + l as u32));
    }
    // Clearings S[D >> 4].
    let s = [
        575, 576, 577, 578, 579, 580, 0, 581, 582, 583, 0, 584, 0, 0, 0,
    ];
    for (i, &want) in s.iter().enumerate() {
        let d = ((i + 1) << 4) as i32;
        if want == 0 {
            assert_eq!(code_to_id(d), Err(OutdoorError::Fatal(0x799)), "D {d:#x}");
        } else {
            assert_eq!(code_to_id(d), Ok(want), "D {d:#x}");
        }
    }
    // One attach bit: T[L][slot] (slot 0 W 0x10, 1 E 0x20, 2 S 0x40, 3 N 0x80).
    assert_eq!(code_to_id(0x21), Ok(545));
    assert_eq!(code_to_id(0x41), Ok(546));
    assert_eq!(code_to_id(0x81), Ok(547));
    assert_eq!(code_to_id(0x12), Ok(548));
    assert_eq!(code_to_id(0x1E), Ok(572));
    assert_eq!(code_to_id(0x2D), Ok(571));
    // A T entry 0: fatal 0x78C.
    assert_eq!(code_to_id(0x11), Err(OutdoorError::Fatal(0x78C)));
    // The second bit indexes the exit table with the first result (548). Rows 545..572
    // read 0 in slots 0 and 2 (fatal 0x78C) and a non-id in slots 1 and 3 (a crash
    // error); row 0 reads 0 in every slot.
    assert_eq!(code_to_id(0x52), Err(OutdoorError::Fatal(0x78C)));
    assert_eq!(code_to_id(0x32), Err(OutdoorError::Crash(CRASH_EXIT_ID)));
    assert_eq!(code_to_id(0x92), Err(OutdoorError::Crash(CRASH_EXIT_ID)));
    assert_eq!(code_to_id(0xF1), Err(OutdoorError::Fatal(0x78C)));
    // Row 15 (low nibble 15 with any attach bit) holds only zeros: fatal 0x78C.
    for d in [0x1F, 0x2F, 0x4F, 0x8F, 0xFF] {
        assert_eq!(code_to_id(d), Err(OutdoorError::Fatal(0x78C)), "D {d:#x}");
    }
    // Clearing entry 0 (256) is never produced.
    for d in 1..256 {
        assert_ne!(code_to_id(d), Ok(256));
    }
}

// ---- rivers and attach points on small grids ---------------------------------------

/// One jungle (SXb 2, SYb 6) at blocks (1, 1) in a 4 × 8 grid.
fn one_jungle() -> (JungleGrid, Vec<JungleRec>) {
    let recs = vec![JungleRec {
        bx: 1,
        by: 1,
        ..JungleRec::default()
    }];
    (JungleGrid::new(4, 8), recs)
}

#[test]
fn river_pass_rules() {
    let mut seen_row2 = false;
    for k in 0..300u32 {
        let (mut g, recs) = one_jungle();
        let mut s = Seed::init_low(k.wrapping_mul(2_654_435_761));
        river_and_attach(&mut g, &mut s, &recs, 0, 2, 6);
        // The river starts in the bottom row (B = 100) in one column.
        let starts = [g.b_at(1, 6), g.b_at(2, 6)];
        assert_eq!(starts.iter().filter(|&&b| b == 100).count(), 1, "seed {k}");
        // At least 2 and at most 3 attach points survive.
        let n = g.c.iter().filter(|&&c| c == 2).count();
        assert!((2..=3).contains(&n), "seed {k}: {n}");
        // No mark in absolute row 1 (the jungle's top row), marks possible in row 2.
        assert_ne!(g.c_at(1, 1), 2);
        assert_ne!(g.c_at(2, 1), 2);
        seen_row2 |= g.c_at(1, 2) == 2 || g.c_at(2, 2) == 2;
        // Every row of the jungle holds one or two river blocks.
        for row in 1..=6 {
            let r = (g.b_at(1, row) != 0) as i32 + (g.b_at(2, row) != 0) as i32;
            assert!((1..=2).contains(&r), "seed {k} row {row}");
        }
    }
    assert!(seen_row2);
}

#[test]
fn river_connections_rules() {
    // Two owners side by side; a link block of owner 1 next to its river
    // (B 101 N, B 100 S: the smaller wins) and a link block of owner 2.
    let mut g = JungleGrid::new(5, 5);
    let i = |c: i32, r: i32| (r * 5 + c) as usize;
    for r in 1..4 {
        g.a[i(1, r)] = 1;
        g.a[i(2, r)] = 1;
        g.a[i(3, r)] = 2;
    }
    g.b[i(2, 1)] = 101;
    g.b[i(2, 3)] = 100;
    g.c[i(2, 2)] = 1;
    g.c[i(3, 2)] = 1;
    g.connect_rivers();
    // Link: S (4) toward B 100, E (2) toward the other owner's link.
    assert_eq!(g.d[i(2, 2)], 4 | 2);
    // The chosen neighbour got the opposite bit (N).
    assert_eq!(g.d[i(2, 3)] & 8, 8);
    assert_eq!(g.d[i(2, 1)] & 4, 0);
    // The other owner's link block: W toward owner 1's link.
    assert_eq!(g.d[i(3, 2)], 1);
    // B 101 and 100 are not adjacent; |ΔB| = 1 joins only neighbours.
    let mut g = JungleGrid::new(4, 4);
    g.b[5] = 100;
    g.b[6] = 101;
    g.b[9] = 103;
    g.connect_rivers();
    assert_eq!((g.d[5], g.d[6], g.d[9]), (2, 1, 0));
}

#[test]
fn attach_point_without_river_restarts() {
    // An attach point with no river neighbour: the pass stops after its own draw (blocks
    // before it drew one each).
    let mut g = JungleGrid::new(3, 3);
    g.a[4] = 1;
    g.c[4] = 2;
    let mut s = Seed::init_low(5);
    let s0 = s;
    assert!(!g.attach(&mut s));
    assert_eq!(steps_between(s0, s), Some(5));
    // With a river block (D 4) of the same owner north of it: one bit
    // (0x80), the river gets 0x40; D was 0, so two more draws.
    let mut g = JungleGrid::new(3, 3);
    g.a[1] = 1;
    g.a[4] = 1;
    g.d[1] = 4;
    g.c[4] = 2;
    let mut s = Seed::init_low(5);
    assert!(g.attach(&mut s));
    assert_eq!(steps_between(s0, s), Some(9 + 2));
    assert_eq!(g.d[4] & 0x80, 0x80);
    assert_eq!(g.d[1], 4 | 0x40);
    assert_eq!(g.c[4], 0);
}

#[test]
fn act3_build_stamps_the_jungle_from_creation() {
    let (mut drlg, mut o, data) = create_act3();
    let od = act3_od();
    let l = drlg.find_level(76).unwrap();
    drlg.level_mut(l).seed = Seed::new(4_014_346_869 + 76, 666);
    let mut presets = Presets::default();
    o.generate(
        &mut drlg,
        &data,
        &od,
        &SubFileMap::default(),
        &mut presets,
        l,
    )
    .unwrap();
    let info = o.level(l).unwrap();
    // The jungle stamps, then the head; no blank cells.
    let p: Vec<u32> = info.grids[0]
        .cells
        .iter()
        .copied()
        .filter(|&p| p != 0)
        .collect();
    assert_eq!(p, [541, 533, 543, 565, 570, 582, 571, 575, 570, 575, 573]);
    assert!(info.grids[2].cells.iter().all(|&c| c & 0x100 == 0));
    // One preset room per stamp.
    assert_eq!(presets.calls.len(), 11);
}

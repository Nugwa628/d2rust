//! Tests for outdoor level generation: Act I placement with fixed seeds, the Blood Moor
//! room's sub-theme pick, and synthetic rule checks on small fake levels and rooms.
//!
//! The fakes at the top (level data, outdoor data, preset builder, `Env`) are shared with
//! the other test modules in this crate.

use std::collections::BTreeMap;

use crate::drlg::outdoor::grid::{cell, shuffle_cells, Gen, Op};
use crate::drlg::outdoor::place::{self, Driver};
use crate::drlg::outdoor::tilesub::{self, pick_sub_themes, room_substitution, BorderCtx, RoomSub};
use crate::drlg::outdoor::vertex::{border_piece, build_polygon, corner_piece, to_cells};
use crate::drlg::outdoor::wild::{dir, grid_path};
use crate::drlg::outdoor::*;
use crate::drlg::tiles::CellGrid;
use crate::drlg::{
    Drlg, DrlgData, DrlgError, LevelDef, LevelIdx, LevelTypes, NoLevelTypes, PresetUnit, TileRect,
    TileRef,
};
use crate::rng::Seed;

// ---- fakes ------------------------------------------------------------------

pub(super) fn data() -> DrlgData {
    let mut d = DrlgData {
        levels: vec![LevelDef::default(); 140],
        ..DrlgData::default()
    };
    for l in &mut d.levels {
        l.warp = [-1; 8];
    }
    d
}

pub(super) fn od() -> OutdoorData {
    OutdoorData {
        levels: vec![
            SubDefs {
                sub_type: -1,
                sub_theme: -1,
                sub_waypoint: -1,
                sub_shrine: -1,
            };
            140
        ],
        presets: vec![
            PresetDef {
                size_x: 8,
                size_y: 8,
                files: 1,
            };
            1100
        ],
        subs: Vec::new(),
    }
}

/// Records non-outdoor inits (allocation order).
#[derive(Default)]
pub(super) struct Rec {
    pub(super) inits: Vec<u32>,
}

impl LevelTypes for Rec {
    fn init_level(&mut self, drlg: &mut Drlg, _: &DrlgData, l: LevelIdx) -> Result<(), DrlgError> {
        self.inits.push(drlg.level(l).id);
        Ok(())
    }
}

/// Preset cell builder that records its calls and draws `roll(Files)` on the level seed,
/// as the real preset builder does.
#[derive(Default)]
pub(super) struct Presets {
    pub(super) calls: Vec<(u32, i32, i32, u32, u32)>,
    pub(super) files: BTreeMap<u32, i32>,
}

impl OutdoorPresets for Presets {
    fn build_preset_cell(
        &mut self,
        drlg: &mut Drlg,
        _: &DrlgData,
        level: LevelIdx,
        def: u32,
        x: i32,
        y: i32,
        file: u32,
        flags: u32,
    ) -> Result<(), DrlgError> {
        let n = self.files.get(&def).copied().unwrap_or(1);
        drlg.level_mut(level).seed.roll(n);
        self.calls.push((def, x, y, file, flags));
        Ok(())
    }
}

/// A level of `gw × gh` cells at (800, 800) with zeroed grids; the
/// level is DrlgType 0 so allocation runs no type init.
pub(super) struct Env {
    pub(super) drlg: Drlg,
    pub(super) data: DrlgData,
    pub(super) od: OutdoorData,
    pub(super) subs: SubFileMap,
    pub(super) info: OutdoorLevel,
    pub(super) l: LevelIdx,
    pub(super) id: u32,
}

impl Env {
    pub(super) fn new(id: u32, gw: i32, gh: i32) -> Self {
        let mut data = data();
        data.levels[id as usize].level_type = 2;
        let mut drlg = Drlg::create(0, 1, 0, 0, false, &data, &mut NoLevelTypes).unwrap();
        let l = drlg
            .get_or_alloc_level(&data, &mut NoLevelTypes, id)
            .unwrap();
        drlg.level_mut(l).rect = TileRect::new(800, 800, 8 * gw, 8 * gh);
        let info = OutdoorLevel {
            grids: [
                Grid::new(gw, gh),
                Grid::new(gw, gh),
                Grid::new(gw, gh),
                Grid::new(gw, gh),
            ],
            ..OutdoorLevel::default()
        };
        Self {
            drlg,
            data,
            od: od(),
            subs: SubFileMap::default(),
            info,
            l,
            id,
        }
    }

    pub(super) fn gen(&mut self) -> Gen<'_> {
        let rect = self.drlg.level(self.l).rect;
        Gen {
            drlg: &mut self.drlg,
            data: &self.data,
            od: &self.od,
            subs: &self.subs,
            level: self.l,
            id: self.id,
            rect,
            info: &mut self.info,
        }
    }

    pub(super) fn seed(&self) -> Seed {
        self.drlg.level(self.l).seed
    }
}

/// `seed` advanced by `n` steps.
pub(super) fn stepped(mut seed: Seed, n: usize) -> Seed {
    for _ in 0..n {
        seed.step();
    }
    seed
}

// ---- act-wide placement -------------------------------------------------------

/// Act I level data for the placement test: sizes and offsets of the derived rects.
/// Blood Moor's size comes from its linker.
pub(super) fn act1_data() -> DrlgData {
    let mut d = data();
    let set = |d: &mut DrlgData, id: usize, ty: u32, size: (i32, i32), off: (i32, i32)| {
        d.levels[id].drlg_type = ty;
        d.levels[id].size = [size; 3];
        d.levels[id].offset = off;
    };
    set(&mut d, 1, 2, (56, 40), (0, 0));
    set(&mut d, 2, 3, (56, 96), (0, 0));
    set(&mut d, 3, 3, (80, 80), (0, 0));
    set(&mut d, 4, 3, (80, 80), (1000, 1000));
    set(&mut d, 5, 3, (80, 80), (0, 0));
    set(&mut d, 6, 3, (80, 80), (0, 0));
    set(&mut d, 7, 3, (80, 80), (0, 0));
    set(&mut d, 17, 3, (40, 48), (0, 0));
    set(&mut d, 26, 2, (40, 18), (3000, 1000));
    set(&mut d, 27, 2, (40, 40), (0, 0));
    set(&mut d, 39, 3, (64, 64), (5000, 1148));
    // Vis: Blood Moor ↔ Rogue, Cold Plains; Cold Plains ↔ Stony, Burial.
    d.levels[2].vis = [1, 3, 0, 0, 0, 0, 0, 0];
    d.levels[3].vis = [2, 4, 17, 0, 0, 0, 0, 0];
    d.levels[1].vis = [2, 0, 0, 0, 0, 0, 0, 0];
    d.levels[4].vis = [3, 0, 0, 0, 0, 0, 0, 0];
    d.levels[17].vis = [3, 0, 0, 0, 0, 0, 0, 0];
    d
}

#[test]
fn act1_placement_matches_recording() {
    let data = act1_data();
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
    let drlg = Drlg::create(0, 644409375, 0, 0, false, &data, &mut types).unwrap();
    assert_eq!(types.last_error, None);
    assert_eq!(drlg.start_seed, 4014346869);
    // Only the Black Marsh draw advanced the DRLG seed.
    assert_eq!(drlg.seed, Seed::new(1406222081, 1674353446));
    let mut order: Vec<u32> = drlg
        .level_list()
        .into_iter()
        .map(|l| drlg.level(l).id)
        .collect();
    order.reverse();
    // The placer rows come first, then the neighbour-entry walk over 1..17 allocates
    // 8..16.
    assert_eq!(
        order,
        [4, 3, 2, 1, 17, 39, 26, 7, 6, 27, 5, 8, 9, 10, 11, 12, 13, 14, 15, 16]
    );
    let rect = |id| drlg.level(drlg.find_level(id).unwrap()).rect;
    assert_eq!(rect(4), TileRect::new(1000, 1000, 80, 80));
    assert_eq!(rect(3), TileRect::new(920, 984, 80, 80));
    assert_eq!(rect(2), TileRect::new(904, 1064, 56, 96));
    assert_eq!(rect(1), TileRect::new(960, 1112, 56, 40));
    assert_eq!(rect(17), TileRect::new(880, 968, 40, 48));
    assert_eq!(rect(39).x, 5000);
    assert_eq!(rect(39).y, 1148);
    assert_eq!(rect(26), TileRect::new(3000, 1000, 40, 18));
    assert_eq!(rect(7).y, 1018);
    assert_eq!((rect(6).x, rect(6).y), (2920, 1002));
    assert_eq!((rect(5).x, rect(5).y), (2904, 1082));
    assert_eq!(outdoor.preset_direction.get(&1), Some(&3));
    assert_eq!(outdoor.preset_direction.get(&27), Some(&1));
    // Act I flags of Blood Moor and Cold Plains: 0.
    let flags = |id| outdoor.level(drlg.find_level(id).unwrap()).unwrap().flags;
    assert_eq!(flags(2), 0);
    assert_eq!(flags(3), 0);
}

#[test]
fn copy_one_draws_are_the_recorded_values() {
    // Six draws on the driver's copy: R0 of Cold Plains, Blood Moor (R0, R3), Rogue
    // (R0, R3), Burial Grounds.
    let mut s = Seed::new(4014346869, 268778232);
    let v: Vec<u32> = (0..6).map(|_| s.step()).collect();
    assert_eq!(
        v,
        [1406222081, 3154683627, 457460266, 1949180022, 1550108608, 4175359377]
    );
    assert_eq!(
        (v[0] & 3, v[1] & 3, v[2] & 1, v[3] & 3, v[4] & 1, v[5] & 3),
        (1, 3, 0, 2, 0, 1)
    );
}

#[test]
fn act1_neighbours_and_warps() {
    let data = act1_data();
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
    let drlg = Drlg::create(0, 644409375, 0, 0, false, &data, &mut types).unwrap();
    // Blood Moor's neighbours: Rogue (E, preset) and Cold Plains (N),
    // head insertion in vis order.
    let bm = outdoor.level(drlg.find_level(2).unwrap()).unwrap();
    let got: Vec<(u32, i32, bool)> = bm
        .orth
        .iter()
        .map(|e| (e.level_id, e.direction, e.preset))
        .collect();
    assert_eq!(got, [(3, 1, false), (1, 2, true)]);
    assert_eq!(bm.orth[1].rect, TileRect::new(960, 1112, 56, 40));
    // The driver set warps both ways (warp −1): vis kept.
    assert_eq!(drlg.warp_array(&data, 2).unwrap()[0], -1);
}

#[test]
fn act1_flags_table() {
    let mut d = Driver::new(Seed::init());
    let mut o = Outdoor::default();
    let l = LevelIdx(0);
    // Blood Moor (filter 2): R0 2, next 2 → 0x8 | 0x80.
    d.r[0][0] = 2;
    d.r[0][1] = 2;
    o.act1_flags(l, 2, &d, 0);
    assert_eq!(o.levels[&l].flags, 0x88);
    // Stony Field: (1, 1) → 0x10 (excl 3, 17 only).
    let l = LevelIdx(1);
    d.r[0][0] = 1;
    d.r[0][1] = 1;
    o.act1_flags(l, 4, &d, 0);
    assert_eq!(o.levels[&l].flags, 0x10);
    // Cold Plains is excluded from every "any" row.
    let l = LevelIdx(2);
    o.act1_flags(l, 3, &d, 0);
    assert_eq!(o.levels[&l].flags, 0);
}

#[test]
fn placement_cases() {
    let p = TileRect::new(100, 200, 40, 30);
    let mut c = TileRect::new(0, 0, 10, 20);
    place::place_a(p, &mut c, 3, 3);
    assert_eq!((c.x, c.y), (140, 218));
    place::place_b(p, &mut c, 0, 1);
    assert_eq!((c.x, c.y), (146, 230));
    place::place_c(p, &mut c, 6, 1);
    assert_eq!((c.x, c.y), (140, 182));
    place::place_c(p, &mut c, 6, 0);
    assert_eq!((c.x, c.y), (140, 200));
    assert_eq!(place::direction(&p, &TileRect::new(60, 0, 40, 10)), 0);
    assert_eq!(place::direction(&p, &TileRect::new(140, 0, 4, 4)), 2);
    assert_eq!(place::direction(&p, &TileRect::new(0, 170, 4, 30)), 1);
    assert_eq!(place::direction(&p, &TileRect::new(0, 230, 4, 4)), 3);
    assert_eq!(place::direction(&p, &TileRect::new(0, 0, 4, 4)), -1);
}

#[test]
fn jungle_offsets_for_192() {
    assert_eq!(place::jungle_offsets(192), (-64, -128));
}

// ---- primitives -----------------------------------------------------------------

#[test]
fn stamp_and_build_list() {
    let mut e = Env::new(50, 6, 6);
    e.od.presets[5] = PresetDef {
        size_x: 16,
        size_y: 8,
        files: 3,
    };
    let s0 = e.seed();
    let mut first = s0;
    let r = first.roll(3) as i32;
    let mut g = e.gen();
    g.stamp(1, 1, 5, -1, true).unwrap();
    g.stamp(3, 3, 5, -1, false).unwrap();
    assert_eq!(g.info.build_list.len(), 1);
    assert_eq!(g.g(0, 1, 1), 5);
    assert_eq!(g.g(0, 2, 1), 0);
    assert_eq!(g.g(2, 1, 1), 0x200 | 0x1 | ((((r + 1) % 3) as u32) << 16));
    assert_eq!(g.g(2, 2, 1), g.g(2, 1, 1));
    assert_eq!(g.g(2, 3, 3), 0x200 | ((((r + 2) % 3) as u32) << 16));
    // One draw for both stamps.
    assert_eq!(e.seed(), stepped(s0, 1));
    // roll(1) still steps; Files 0 divides by zero.
    let mut e = Env::new(50, 4, 4);
    e.od.presets[7].files = 0;
    let s0 = e.seed();
    e.gen().stamp(0, 0, 9, -1, false).unwrap();
    assert_eq!(e.seed(), stepped(s0, 1));
    assert_eq!(
        e.gen().stamp(0, 0, 7, -1, false),
        Err(OutdoorError::NoFiles(7))
    );
    // An explicit file draws nothing.
    let s0 = e.seed();
    e.gen().stamp(1, 1, 7, 2, false).unwrap();
    assert_eq!(e.seed(), s0);
}

#[test]
fn fit_test_margins() {
    let mut e = Env::new(50, 6, 6);
    e.od.presets[5] = PresetDef {
        size_x: 16,
        size_y: 16,
        files: 1,
    };
    let mut g = e.gen();
    assert!(g.fits(2, 2, 5, 0, 15).unwrap());
    assert!(!g.fits(5, 5, 5, 0, 15).unwrap());
    g.op(2, 2, 1, Op::Or, cell::PATH);
    assert!(g.fits(2, 2, 5, 0, 15).unwrap());
    assert!(!g.fits(2, 2, 5, 1, 1).unwrap());
    g.op(2, 4, 2, Op::Or, cell::LINK);
    // Link bit 0x400 is not in 0x1B81: still spawn valid.
    assert!(g.fits(2, 2, 5, 1, 2).unwrap());
    g.op(2, 4, 3, Op::Or, cell::SHRINE);
    assert!(!g.fits(2, 2, 5, 1, 2).unwrap());
    assert!(g.fits(0, 0, 0, 0, 0).unwrap());
}

#[test]
fn shuffle_draws_two_per_entry() {
    let s0 = Seed::init_low(77);
    let mut s = s0;
    let e = shuffle_cells(&mut s, 3, 2);
    assert_eq!(s, stepped(s0, 12));
    let mut sorted = e.clone();
    sorted.sort();
    assert_eq!(sorted, [(0, 0), (0, 1), (1, 0), (1, 1), (2, 0), (2, 1)]);
    let mut s = s0;
    assert!(shuffle_cells(&mut s, 0, 5).is_empty());
    assert_eq!(s, s0);
}

#[test]
fn placers_draw_counts() {
    // Shrines: `&3` even when the grid has no candidates.
    let mut e = Env::new(50, 2, 2);
    let s0 = e.seed();
    e.gen().shrines(5);
    assert_eq!(e.seed(), stepped(s0, 1));
    // Shrines on a 6×6 grid: 1 + 2·16 draws, five cells marked.
    let mut e = Env::new(50, 6, 6);
    let s0 = e.seed();
    e.gen().shrines(5);
    assert_eq!(e.seed(), stepped(s0, 33));
    let marked = e.info.grids[2]
        .cells
        .iter()
        .filter(|&&c| c & cell::SHRINE != 0)
        .count();
    assert_eq!(marked, 5);
    // FarAway: rx, ry, then the stamp's build-list roll.
    let mut e = Env::new(50, 6, 6);
    let s0 = e.seed();
    assert!(e
        .gen()
        .far_away(TileRect::new(0, 0, 8, 8), 52, -1, 1, 15)
        .unwrap());
    assert_eq!(e.seed(), stepped(s0, 3));
    // The far-away cell is the one farthest from the rect centre among
    // cells fitting with margin 1 (rows 1..4 minus the top row): (4, 4).
    assert_eq!(e.info.grids[0].get(4, 4), 52);
    // SpawnRandomDS1 without path cells: two shuffles.
    let mut e = Env::new(50, 6, 6);
    let s0 = e.seed();
    e.gen().r(46).unwrap();
    assert_eq!(e.seed(), stepped(s0, 32 + 32 + 1));
    // With a path cell: one shuffle, a neighbour stamped.
    let mut e = Env::new(50, 6, 6);
    e.gen().op(2, 3, 3, Op::Or, cell::PATH);
    let s0 = e.seed();
    e.gen().r(46).unwrap();
    assert_eq!(e.seed(), stepped(s0, 32 + 1));
    assert_eq!(e.info.grids[0].get(2, 3), 46);
}

#[test]
fn waypoint_cold_plains_link() {
    let mut e = Env::new(3, 10, 10);
    e.data.levels[3].vis = [4, 2, 0, 0, 0, 0, 0, 0];
    let mask = 1 << (1 + 4);
    e.gen().op(1, 9, 5, Op::Or, mask);
    e.gen().op(2, 9, 5, Op::Or, cell::LINK);
    let s0 = e.seed();
    e.gen().waypoint().unwrap();
    assert_eq!(e.seed(), s0);
    assert_eq!(e.info.grids[1].get(8, 5), 0x20000);
    assert_eq!(e.info.grids[2].get(8, 5) & cell::WAYPOINT, cell::WAYPOINT);
    // Other levels: a shuffle, room flag 0x10000.
    let mut e = Env::new(4, 4, 4);
    let s0 = e.seed();
    e.gen().waypoint().unwrap();
    assert_eq!(e.seed(), stepped(s0, 8));
    let n = e.info.grids[1]
        .cells
        .iter()
        .filter(|&&c| c == 0x10000)
        .count();
    assert_eq!(n, 1);
}

// ---- polygon and borders -----------------------------------------------------

pub(super) fn orth(dir: i32, rect: TileRect) -> Orth {
    Orth {
        level_id: 9,
        direction: dir,
        init: false,
        rect,
        preset: false,
    }
}

#[test]
fn polygon_insertion_order() {
    let rect = TileRect::new(0, 0, 80, 80);
    let es = [
        orth(0, TileRect::new(-80, 16, 80, 32)),
        orth(0, TileRect::new(-80, 48, 80, 31)),
    ];
    let vs = build_polygon(rect, &es).unwrap();
    let got: Vec<(i32, i32, u32)> = vs.iter().map(|v| (v.x, v.y, v.flags)).collect();
    assert_eq!(
        got,
        [
            (0, 79, 0),
            (0, 78, 1),
            (0, 48, 0),
            (0, 47, 1),
            (0, 16, 0),
            (0, 0, 0),
            (79, 0, 0),
            (79, 79, 0)
        ]
    );
    let mut cells = vs.clone();
    to_cells(&mut cells);
    let got: Vec<(i32, i32, u32)> = cells.iter().map(|v| (v.x, v.y, v.flags)).collect();
    // (0, 79) and (0, 78) merge: the earlier takes the later's flags.
    assert_eq!(
        got,
        [
            (0, 9, 1),
            (0, 6, 0),
            (0, 5, 1),
            (0, 2, 0),
            (0, 0, 0),
            (9, 0, 0),
            (9, 9, 0)
        ]
    );
    // A box starting at the corner flags the corner itself.
    let vs = build_polygon(rect, &[orth(1, TileRect::new(0, -40, 40, 40))]).unwrap();
    assert_eq!(vs[1].flags, 1);
    assert_eq!((vs[2].x, vs[2].y), (39, 0));
    assert_eq!(
        build_polygon(rect, &[orth(-1, rect)]),
        Err(OutdoorError::UnknownDirection(-1))
    );
}

#[test]
fn link_flags_mark_the_edge() {
    let mut e = Env::new(50, 10, 10);
    e.data.levels[50].vis = [0, 0, 7, 0, 0, 0, 0, 0];
    let rect = e.drlg.level(e.l).rect;
    e.info.orth = vec![Orth {
        level_id: 7,
        direction: 1,
        init: false,
        rect: TileRect::new(rect.x + 16, rect.y - 40, 40, 40),
        preset: false,
    }];
    let mut g = e.gen();
    g.polygon().unwrap();
    g.link_flags().unwrap();
    // Link edge (2, 0)..(6, 0): vis slot 2 → 0x40.
    for x in 0..10 {
        let expect = if (2..=6).contains(&x) { 0x40 } else { 0 };
        assert_eq!(g.g(1, x, 0), expect, "x {x}");
        assert_eq!(g.g(2, x, 0), if expect != 0 { 1 } else { 0 });
    }
}

#[test]
fn border_lookups() {
    // Straight: N[dx + 3dy + 4] + 1 into P.
    assert_eq!(border_piece(1, 0, 1), 6);
    assert_eq!(border_piece(0, -1, 0), 16);
    assert_eq!(border_piece(-1, 0, 0), 0);
    assert_eq!(border_piece(0, 1, 2), 367);
    assert_eq!(border_piece(1, 0, 4), 883);
    assert_eq!(border_piece(1, 0, 5), 959);
    // Corners: E then S doubled → N[72] = 7.
    assert_eq!(corner_piece(2, 0, 0, 2, 1), 10);
    assert_eq!(corner_piece(0, -2, 2, 0, 0), 19);
    assert_eq!(corner_piece(2, 0, 0, 2, 5), 963);
    // k = −1 → piece 0.
    assert_eq!(corner_piece(0, 0, 0, 0, 1), 0);
}

#[test]
fn borders_on_a_plain_wild_level() {
    let mut e = Env::new(4, 6, 6);
    let mut g = e.gen();
    g.polygon().unwrap();
    g.borders().unwrap();
    // Every edge cell is a border, the inside is untouched.
    for y in 0..6 {
        for x in 0..6 {
            let edge = x == 0 || y == 0 || x == 5 || y == 5;
            assert_eq!(g.g(2, x, y) & cell::BORDER != 0, edge, "({x}, {y})");
            assert_eq!(g.g(2, x, y) & cell::BLANK, 0);
        }
    }
    // Wild style 1, N edge walked W→E: Border(1, 0, 1) = 6 at (1..5, 0)
    // except the corner (5, 0), which gets Corner(2, 0, 0, 2, 1) = 10.
    assert_eq!(g.g(0, 3, 0), 6);
    assert_eq!(g.g(0, 5, 0), 10);
}

#[test]
fn blank_corners_fill_to_the_border() {
    let mut e = Env::new(50, 5, 5);
    let mut g = e.gen();
    for i in 0..5 {
        g.op(2, 2, i, Op::Or, cell::BORDER);
        g.op(2, i, 2, Op::Or, cell::BORDER);
    }
    g.blank_corners();
    for (x, y) in [(0, 0), (1, 1), (4, 4), (3, 0), (0, 3)] {
        assert_ne!(g.g(2, x, y) & cell::BLANK, 0, "({x}, {y})");
    }
    assert_eq!(g.g(2, 2, 2) & cell::BLANK, 0);
}

// ---- Act I ---------------------------------------------------------------------

#[test]
fn cliff_marking_plain_rect() {
    let mut e = Env::new(4, 10, 10);
    let mut g = e.gen();
    g.polygon().unwrap();
    g.cliff_marking();
    let dirs: Vec<u8> = g.info.vertices.iter().map(|v| v.direction).collect();
    assert_eq!(dirs, [1, 1, 0, 0]);
    assert_eq!(g.info.flags, 0x20);
}

#[test]
fn caves_draws() {
    // Cliff cave: one `&1`, then the first grid-0 16 or 17 cell.
    let mut e = Env::new(4, 10, 10);
    e.info.flags = 0x20;
    e.gen().op(0, 4, 2, Op::Set, 17);
    let s0 = e.seed();
    e.gen().river_caves().unwrap();
    assert_eq!(e.seed(), stepped(s0, 2));
    assert_eq!(e.info.grids[0].get(4, 2), 24);
    assert_eq!(e.info.flags, 0x60);
    // Side cave: `&3`; r odd → x = 3, r ≥ 2 → y = 3.
    let mut e = Env::new(2, 10, 10);
    e.info.flags = 0x10;
    // Keep the river out: a direction bit in column gw − 2.
    let s0 = e.seed();
    let r = {
        let mut c = s0;
        c.mask(4) as i32
    };
    e.gen().river_caves().unwrap();
    let x = if r & 1 != 0 { 3 } else { 6 };
    let y = if r >= 2 { 3 } else { 6 };
    assert_eq!(e.info.grids[0].get(x, y), 52);
    assert_eq!(e.info.flags, 0x50);
}

#[test]
fn river_and_bridge() {
    let mut e = Env::new(4, 6, 6);
    e.info.flags = 0x10;
    e.od.presets[28].files = 4;
    e.gen().op(2, 2, 0, Op::Or, cell::BLANK);
    e.gen().op(0, 3, 5, Op::Set, 4);
    let s0 = e.seed();
    let r = {
        let mut c = s0;
        c.roll(4) as i32
    };
    e.gen().river(2).unwrap();
    // Upper (2, 0) on a blank cell: file 0; lower (3, 5) on P 4: L[4] = 2.
    let f = |x, y| ((e.info.grids[2].get(x, y) >> 16) & 0xF) as i32;
    assert_eq!(e.info.grids[0].get(2, 0), 26);
    assert_eq!(f(2, 0), 0);
    assert_eq!(f(3, 5), 2);
    // Bridge: roll(gh − 2), first row from r with both files 3.
    let y = r % 4 + 1;
    assert_eq!(e.info.grids[0].get(2, y), 28);
    assert_eq!((f(2, y), f(3, y)), (1, 2));
    assert_eq!(e.seed(), stepped(s0, 1));
}

#[test]
fn cottage_draws() {
    let mut e = Env::new(2, 6, 6);
    let s0 = e.seed();
    let k = {
        let mut c = s0;
        c.mask(4)
    };
    e.gen().cottage(47, false).unwrap();
    // Two shuffles for each RandomDS1 (no paths) + a build-list roll.
    let per = 32 + 32;
    let expect = if k != 0 { 1 + per + 1 } else { 1 + 2 * per + 1 };
    assert_eq!(e.seed(), stepped(s0, expect));
}

#[test]
fn path_starts_and_adjust() {
    let mut e = Env::new(2, 10, 10);
    e.info.orth = vec![Orth {
        level_id: 1,
        direction: 3,
        init: false,
        rect: TileRect::new(800, 880, 56, 40),
        preset: true,
    }];
    let mut g = e.gen();
    g.stamp(2, 0, 51, 1, false).unwrap();
    let s = g.path_starts();
    assert_eq!(s.len(), 2);
    assert_eq!((s[0].x, s[0].y, s[0].direction), (829, 883, 3));
    assert_eq!((s[1].x, s[1].y, s[1].direction), (819, 803, 1));
    let a = g.adjust(s[0]);
    assert_eq!((a.x, a.y), (829, 875));
    let a = g.adjust(s[1]);
    assert_eq!((a.x, a.y), (819, 811));
}

#[test]
fn dir_table_and_parity_quirk() {
    assert_eq!(dir((0, 0), (5, 0)), 0);
    assert_eq!(dir((0, 0), (0, 3)), 2);
    assert_eq!(dir((0, 0), (-3, 0)), 4);
    assert_eq!(dir((0, 0), (0, -3)), 6);
    // dy = 1 survives `& 1`, dy = 2 does not.
    assert_eq!(dir((0, 0), (4, 1)), 0);
    assert_eq!(dir((0, 0), (6, 2)), 0);
    assert_eq!(dir((0, 0), (2, 2)), 1);
}

#[test]
fn grid_path_search() {
    assert_eq!(
        grid_path((1, 1), (2, 1), 8, 8, |_| false),
        Some(vec![(1, 1), (2, 1)])
    );
    let p = grid_path((1, 1), (5, 1), 8, 8, |_| false).unwrap();
    assert_eq!(p, [(5, 1), (4, 1), (3, 1), (2, 1), (1, 1)]);
    // Around a wall at x = 3, rows 0..5.
    let p = grid_path((1, 2), (5, 2), 8, 8, |c| c.0 == 3 && c.1 < 6).unwrap();
    assert_eq!(p.first(), Some(&(5, 2)));
    assert_eq!(p.last(), Some(&(1, 2)));
    for w in p.windows(2) {
        assert_eq!((w[0].0 - w[1].0).abs() + (w[0].1 - w[1].1).abs(), 1);
    }
    assert!(p.iter().all(|c| !(c.0 == 3 && c.1 < 6)));
    // Walled off completely: no path.
    assert_eq!(grid_path((1, 2), (5, 2), 8, 8, |c| c.0 == 3), None);
}

#[test]
fn jitter_draws() {
    let mut e = Env::new(2, 10, 10);
    let ends = PathEnds {
        start: PathPoint {
            x: 1,
            y: 2,
            direction: 3,
        },
        start_adjusted: PathPoint {
            x: 3,
            y: 4,
            direction: 3,
        },
        join_adjusted: PathPoint {
            x: 5,
            y: 6,
            direction: 1,
        },
        join: PathPoint {
            x: 7,
            y: 8,
            direction: 1,
        },
    };
    let s0 = e.seed();
    assert!(e.gen().jitter(&ends, Vec::new()).is_empty());
    assert_eq!(e.seed(), stepped(s0, 1));
    let s0 = e.seed();
    let p = e.gen().jitter(&ends, vec![(5, 5), (4, 5), (3, 5), (2, 5)]);
    assert_eq!(e.seed(), stepped(s0, 1 + 2 * 2));
    assert_eq!(p.len(), 6);
    assert_eq!(p[0], (7, 8));
    assert_eq!(p[1], (5, 6));
    assert_eq!(p[4], (3, 4));
    assert_eq!(p[5], (1, 2));
}

// ---- Acts II–V ------------------------------------------------------------------

#[test]
fn desert_rows_and_variants() {
    use crate::drlg::outdoor::acts::{desert_cliff_row, TOMB_ROW};
    assert_eq!(desert_cliff_row(1)[2], (378, -1, 4, 4));
    assert_eq!(desert_cliff_row(1)[1], (377, -1, 2, 4));
    assert_eq!(desert_cliff_row(4)[3], (380, -1, 4, 6));
    assert_eq!(desert_cliff_row(7)[3], (381, -1, 4, 6));
    assert_eq!(TOMB_ROW[4], (387, 0, 0, 0));
    // Variants: roll(n), then each S in list order from r.
    let mut e = Env::new(42, 6, 6);
    e.od.presets[392].files = 2;
    let s0 = e.seed();
    e.gen().variants(&[392, 393], true).unwrap();
    // roll(2) + 3 shuffles (Files 2 + Files 1), explicit files.
    assert_eq!(e.seed(), stepped(s0, 1 + 3 * 32));
}

#[test]
fn sanctum_and_siege_strip() {
    let mut e = Env::new(108, 15, 15);
    e.gen().act4().unwrap();
    assert_eq!(e.info.grids[0].get(12, 12), 836);
    assert_eq!(e.info.grids[0].get(6, 6), 862);
    assert_eq!(e.info.grids[0].get(6, 12), 857);
    assert_eq!(e.info.grids[0].get(3, 6), 858);
    let mut e = Env::new(110, 16, 2);
    e.gen().act5().unwrap();
    assert_eq!(e.info.grids[0].get(15, 0), 865);
    assert_eq!(e.info.grids[0].get(1, 0), 879);
    let mut e = Env::new(110, 10, 2);
    assert_eq!(e.gen().act5(), Err(OutdoorError::SiegeStrip(875)));
}

// ---- rooms ------------------------------------------------------------------------

fn wild_subs(e: &mut Env) {
    // lvlsub type 6: six rows with their Prob0 values and DT1 masks (rows 2 and 4:
    // 0x40001, 0x400001).
    let prob = [30, 50, 90, 0, 50, 20];
    let mask = [0x1, 0x2, 0x40001, 0x8, 0x400001, 0x10];
    for k in 0..6 {
        e.od.subs.push(SubRow {
            type_: 6,
            file: format!("sub{k}").into_bytes(),
            prob: [prob[k], 0, 0, 0, 0],
            dt1_mask: mask[k],
            ..SubRow::default()
        });
    }
}

#[test]
fn blood_moor_room_sub_theme_pick() {
    let mut e = Env::new(2, 2, 2);
    wild_subs(&mut e);
    let mut room = Seed::init_low(223305360);
    assert_eq!(room.step(), 2795816810);
    let (mask, dt1) = pick_sub_themes(&e.od, &mut room, 6, 0).unwrap();
    assert_eq!(mask, 0b010100);
    assert_eq!(0x44103 | dt1, 0x44103 | 0x40001 | 0x400001);
    // No draws for type or theme −1.
    let mut s = Seed::init_low(5);
    assert_eq!(pick_sub_themes(&e.od, &mut s, -1, 0).unwrap(), (0, 0));
    assert_eq!(s, Seed::init_low(5));
}

#[test]
fn sub_theme_lo_values() {
    let mut s = Seed::init_low(223305360);
    s.step();
    let v: Vec<u32> = (0..6).map(|_| s.step()).collect();
    assert_eq!(
        v,
        [310527139, 1584266672, 2804909076, 4172721383, 3426375614, 3941659325]
    );
    let r: Vec<u32> = v.iter().map(|x| x % 100).collect();
    assert_eq!(r, [39, 72, 76, 83, 14, 25]);
}

#[test]
fn cells_to_rooms() {
    let mut e = Env::new(2, 3, 2);
    wild_subs(&mut e);
    e.od.levels[2] = SubDefs {
        sub_type: 6,
        sub_theme: 0,
        sub_waypoint: 4,
        sub_shrine: 5,
    };
    {
        let mut g = e.gen();
        g.stamp(0, 0, 40, 2, false).unwrap();
        g.op(2, 1, 0, Op::Or, cell::BLANK);
        g.op(1, 2, 0, Op::Or, 0x10);
        g.op(2, 2, 0, Op::Or, cell::BORDER);
    }
    let mut presets = Presets::default();
    presets.files.insert(40, 3);
    let mut o = Outdoor::default();
    let s0 = e.seed();
    let rect = e.drlg.level(e.l).rect;
    {
        let mut g = e.gen();
        o.cells_to_rooms(&mut g, &mut presets).unwrap();
    }
    assert_eq!(presets.calls, [(40, rect.x, rect.y, 2, 0)]);
    // Preset roll + 4 room allocations (cells (2,0), (0,1), (1,1), (2,1)).
    assert_eq!(e.seed(), stepped(s0, 1 + 4));
    let rooms = e.drlg.level_rooms(e.l);
    assert_eq!(rooms.len(), 4);
    // Head insertion: the last cell's room first.
    let first = e.drlg.room(*rooms.last().unwrap());
    assert_eq!(first.rect, TileRect::new(rect.x + 16, rect.y, 8, 8));
    assert_eq!(first.flags, 0x10 | 0x80000);
    let od = &o.rooms[rooms.last().unwrap()];
    assert_eq!(od.flags, cell::BORDER);
    assert_eq!((od.sub_type, od.sub_theme), (6, 0));
    assert_eq!(first.dt1_mask & 0x44103, 0x44103);
}

/// A sub file with one 1×1 group at (0, 0), variants `n`.
pub(super) fn one_cell_file(floor: u32, wall: u32, n: i32) -> SubFile {
    let mut f = CellGrid::new(8, 2);
    f.set(0, 0, floor);
    let mut w = CellGrid::new(8, 2);
    w.set(0, 0, wall);
    SubFile {
        method: 2,
        groups: vec![SubGroup {
            x: 0,
            y: 0,
            w: 1,
            h: 1,
            variants: n,
        }],
        floor: Some(f),
        walls: vec![w],
        tile_types: vec![CellGrid::new(8, 2)],
        shadow: None,
        units: Vec::new(),
    }
}

fn room() -> OutdoorRoom {
    OutdoorRoom {
        tile_type: CellGrid::new(9, 9),
        wall: CellGrid::new(9, 9),
        floor: CellGrid::new(9, 9),
        ..OutdoorRoom::default()
    }
}

#[test]
fn scattered_draw_counts() {
    let mut od = od();
    od.subs.push(SubRow {
        type_: 6,
        file: b"a".to_vec(),
        max: [2, 0, 0, 0, 0],
        trials: [3, 0, 0, 0, 0],
        ..SubRow::default()
    });
    let mut subs = SubFileMap::default();
    subs.0.insert(b"a".to_vec(), one_cell_file(2, 0, 1));
    // Room floor without bit 2: every trial fails.
    let mut r = room();
    let s0 = Seed::init_low(9);
    let mut s = s0;
    let mut rs = RoomSub {
        w: 8,
        h: 8,
        tile_x: 0,
        tile_y: 0,
        room: &mut r,
        pick: None,
    };
    room_substitution(&od, &subs, &mut s, &mut rs, 6, 0, 1).unwrap();
    assert_eq!(s, stepped(s0, 2 * (1 + 2 * 3)));
    // Trials −1: one shuffle of 7·7 entries per repetition.
    od.subs[0].trials[0] = -1;
    let mut s = s0;
    room_substitution(&od, &subs, &mut s, &mut rs, 6, 0, 1).unwrap();
    assert_eq!(s, stepped(s0, 2 * (1 + 2 * 49)));
    // A group as large as the room: only the group roll.
    subs.0.get_mut(&b"a"[..]).unwrap().groups[0].w = 8;
    let mut s = s0;
    room_substitution(&od, &subs, &mut s, &mut rs, 6, 0, 1).unwrap();
    assert_eq!(s, stepped(s0, 2));
    // Mask 0: nothing.
    let mut s = s0;
    room_substitution(&od, &subs, &mut s, &mut rs, 6, 0, 0).unwrap();
    assert_eq!(s, s0);
}

#[test]
fn fixed_test_and_apply() {
    let mut file = one_cell_file(0x0010_0002, 0, 1);
    file.groups[0].w = 2;
    let mut sh = CellGrid::new(8, 2);
    sh.set(3, 0, 0x800_0000);
    file.shadow = Some(sh);
    file.units = vec![
        PresetUnit {
            unit_type: 2,
            class: 7,
            x: 3,
            y: 2,
        },
        PresetUnit {
            unit_type: 2,
            class: 8,
            x: 0,
            y: 2,
        },
    ];
    let mut r = room();
    for y in 0..8 {
        for x in 0..8 {
            r.floor.set(x, y, 0x40002);
        }
    }
    r.wall.set(4, 4, 1);
    let mut rs = RoomSub {
        w: 8,
        h: 8,
        tile_x: 100,
        tile_y: 200,
        room: &mut r,
        pick: None,
    };
    let g = file.groups[0];
    assert!(tilesub::fixed_test(&rs, &file, g, 1, 1));
    // The pattern cell (0, 0) on a room wall fails.
    assert!(!tilesub::fixed_test(&rs, &file, g, 4, 4));
    // Apply variant 1 (offset w + 1 = 3).
    tilesub::apply(&mut Seed::default(), &mut rs, &file, g, 2, 3, 3).unwrap();
    assert_eq!(rs.room.roof_count, 1);
    assert_eq!(rs.room.shadows, [(102, 203, 0x800_0000)]);
    // No picker: no tile chosen.
    assert!(rs.room.shadow_tiles.is_empty());
    // Unit (3, 2) inside the match box (0, 10) × (0, 5), not (0, 2).
    assert_eq!(rs.room.units.len(), 1);
    assert_eq!((rs.room.units[0].x, rs.room.units[0].y), (13, 17));
    // Variant floor at x 3 is 0: room floor untouched.
    assert_eq!(rs.room.floor.get(2, 3), 0x40002);
    tilesub::apply(&mut Seed::default(), &mut rs, &file, g, 2, 3, 0).unwrap();
    assert_eq!(rs.room.floor.get(2, 3), 0x0010_0082);
    // With bit 20 set there: the fixed test fails.
    assert!(!tilesub::fixed_test(&rs, &file, g, 2, 3));
}

#[test]
fn apply_picks_shadow_tiles_from_the_seed() {
    let mut file = one_cell_file(0x0010_0002, 0, 1);
    let mut sh = CellGrid::new(2, 1);
    sh.set(0, 0, 0x800_0000 | (3 << 20) | (5 << 8));
    file.shadow = Some(sh);
    let mut r = room();
    let pick = |s: &mut Seed, v: u32| {
        Ok(TileRef {
            file: (v >> 20) & 0x3F,
            index: s.roll(10),
        })
    };
    let mut rs = RoomSub {
        w: 8,
        h: 8,
        tile_x: 100,
        tile_y: 200,
        room: &mut r,
        pick: Some(&pick),
    };
    let g = file.groups[0];
    let mut seed = Seed::default();
    let mut want = seed;
    tilesub::apply(&mut seed, &mut rs, &file, g, 2, 3, 0).unwrap();
    // One draw on the seed passed in, in step 2.
    let index = want.roll(10);
    assert_eq!(seed, want);
    assert_eq!(rs.room.shadows.len(), 1);
    assert_eq!(rs.room.shadow_tiles, [TileRef { file: 3, index }]);
}

#[test]
fn check_all_random_pastes_when_prob_below() {
    let mut od = od();
    od.subs.push(SubRow {
        type_: 6,
        file: b"a".to_vec(),
        check_all: 1,
        prob: [-1, 0, 0, 0, 0],
        ..SubRow::default()
    });
    let mut subs = SubFileMap::default();
    // Variants 0: roll(0) draws nothing.
    subs.0.insert(b"a".to_vec(), one_cell_file(0, 0, 0));
    let mut r = room();
    let s0 = Seed::init_low(3);
    let mut s = s0;
    let mut rs = RoomSub {
        w: 8,
        h: 8,
        tile_x: 0,
        tile_y: 0,
        room: &mut r,
        pick: None,
    };
    room_substitution(&od, &subs, &mut s, &mut rs, 6, 0, 1).unwrap();
    // Random test passes on all 8×8 cells: one step each, prob −1 < r
    // pastes with roll(0) (no draw).
    assert_eq!(s, stepped(s0, 64));
    // Method 1: fixed test from (1, 1), no draws.
    subs.0.get_mut(&b"a"[..]).unwrap().method = 1;
    let mut s = s0;
    room_substitution(&od, &subs, &mut s, &mut rs, 6, 0, 1).unwrap();
    assert_eq!(s, s0);
}

#[test]
fn border_substitution_bord_types() {
    let setup = |bord: i32| {
        let mut e = Env::new(4, 6, 6);
        e.od.subs.push(SubRow {
            type_: 2,
            file: b"b".to_vec(),
            bord_type: bord,
            grid_size: 1,
            ..SubRow::default()
        });
        let mut f = one_cell_file(0, 0, 2);
        f.groups.push(f.groups[0]);
        e.subs.0.insert(b"b".to_vec(), f);
        e
    };
    // W = H = 6 per group: 36 pairs; every cell passes (no pattern bits)
    // and blanks.
    let mut e = setup(2);
    let s0 = e.seed();
    e.gen().border_sub(BorderCtx::wild(2, 4)).unwrap();
    assert_eq!(e.seed(), stepped(s0, 2 * (72 + 36)));
    assert!(e.info.grids[2].cells.iter().all(|&c| c == cell::BLANK));
    let mut e = setup(1);
    let s0 = e.seed();
    e.gen().border_sub(BorderCtx::wild(2, 4)).unwrap();
    assert_eq!(e.seed(), stepped(s0, 2 * (72 + 1)));
    let mut e = setup(0);
    let s0 = e.seed();
    e.gen().border_sub(BorderCtx::wild(2, 4)).unwrap();
    assert_eq!(e.seed(), stepped(s0, 1 + 72 + 1));
    // Missing file and missing type.
    let mut e = setup(0);
    e.subs.0.clear();
    assert_eq!(
        e.gen().border_sub(BorderCtx::wild(2, 4)),
        Err(OutdoorError::MissingSubFile(b"b".to_vec()))
    );
    assert_eq!(
        e.gen().border_sub(BorderCtx::wild(9, 4)),
        Err(OutdoorError::NoSubRows(9))
    );
}

#[test]
fn border_substitution_stamps_draw_no_build_list_roll() {
    // A row that replaces the first shuffled cell with preset 12 (style
    // 8, base 4) through its one variant: the stamp draws nothing and
    // leaves the build list as it was; the stamp uses file 0. The preset here has 3
    // files.
    let mut e = Env::new(4, 6, 6);
    e.od.presets[12].files = 3;
    e.od.subs.push(SubRow {
        type_: 1,
        file: b"b".to_vec(),
        bord_type: 0,
        grid_size: 1,
        ..SubRow::default()
    });
    let mut f = one_cell_file(0, 0, 1);
    // Variant 0 at x offset (0 + 1)·(w + 1) = 2.
    f.walls[0].set(2, 0, (9 << 8) | 1);
    e.subs.0.insert(b"b".to_vec(), f);
    let s0 = e.seed();
    e.gen().border_sub(BorderCtx::wild(1, 4)).unwrap();
    // roll(1 group), 36 shuffle pairs, roll(1) variant: no fourth kind.
    assert_eq!(e.seed(), stepped(s0, 1 + 72 + 1));
    assert!(e.info.build_list.is_empty());
    let hit = e.info.sub_hits[0];
    let (x, y) = (hit.x, hit.y);
    assert_eq!(e.info.grids[0].get(x, y), 12);
    assert_eq!(
        e.info.grids[2].get(x, y) & cell::FILE_MASK,
        (tilesub::SUB_STAMP_FILE as u32) << 16
    );
}

#[test]
fn act5_style_map() {
    assert_eq!(tilesub::style_map(49, 5, false), Ok(919));
    assert_eq!(tilesub::style_map(49, 31, true), Ok(987));
    assert_eq!(tilesub::style_map(48, 3, false), Ok(882));
    assert_eq!(tilesub::style_map(48, 31, false), Ok(-5));
    assert_eq!(
        tilesub::style_map(48, 9, false),
        Err(OutdoorError::StyleMap(48, 9))
    );
    assert_eq!(
        tilesub::style_map(50, 1, false),
        Err(OutdoorError::StyleMap(50, 1))
    );
}

#[test]
fn outdoor_room_grids() {
    let mut e = Env::new(16, 1, 1);
    e.data.levels[16].level_type = 16;
    e.drlg.level_mut(e.l).level_type = 16;
    let mut o = Outdoor::default();
    let mut presets = Presets::default();
    {
        let mut g = e.gen();
        o.cells_to_rooms(&mut g, &mut presets).unwrap();
    }
    let room = e.drlg.level_rooms(e.l)[0];
    assert_eq!(e.drlg.room(room).dt1_mask, 0x1);
    let grids = o.room_grids(&mut e.drlg, &e.od, &e.subs, room).unwrap();
    assert_eq!(grids.passes.len(), 2);
    let floor = &grids.passes[1].cells;
    assert_eq!(floor.get(3, 3), 0x40002 | 0x100);
    assert_eq!(floor.get(0, 3), 0x40002 | 0x100 | 0x4);
    assert_eq!(floor.get(8, 8), 0x100 | 0x4);
    assert_eq!(grids.passes[0].cells.get(4, 0), 0x4);
    assert_eq!(grids.passes[0].cells.get(4, 4), 0);
}

#[test]
fn blood_moor_generates_end_to_end() {
    let mut data = act1_data();
    data.levels[2].level_type = 2;
    let mut od = od();
    od.levels[2] = SubDefs {
        sub_type: 6,
        sub_theme: 0,
        sub_waypoint: 4,
        sub_shrine: 5,
    };
    let mut subs = SubFileMap::default();
    for t in 0..7 {
        let name = format!("t{t}").into_bytes();
        od.subs.push(SubRow {
            type_: t,
            file: name.clone(),
            bord_type: 1,
            grid_size: 1,
            prob: [50; 5],
            max: [2; 5],
            trials: [3; 5],
            ..SubRow::default()
        });
        // Pattern: a style-0 wall (base 4) replaced by style 1.
        let mut f = one_cell_file(0, 0x101, 1);
        f.walls[0].set(2, 0, 0x201);
        subs.0.insert(name, f);
    }
    let mut outdoor = Outdoor::default();
    let mut rec = Rec::default();
    let mut presets = Presets::default();
    let mut types = OutdoorTypes {
        outdoor: &mut outdoor,
        od: &od,
        subs: &subs,
        presets: &mut presets,
        others: &mut rec,
        last_error: None,
    };
    let mut drlg = Drlg::create(0, 644409375, 0, 0, false, &data, &mut types).unwrap();
    let l = drlg.find_level(2).unwrap();
    drlg.generate_level(&data, &mut types, l).unwrap();
    assert_eq!(types.last_error, None);
    let rooms = drlg.level_rooms(l);
    for &r in &rooms {
        types.room_grids(&mut drlg, &data, r).unwrap();
    }
    let info = outdoor.level(l).unwrap();
    assert_eq!((info.gw(), info.gh()), (7, 12));
    let outdoor_cells = info.grids[2]
        .cells
        .iter()
        .filter(|&&c| c & (cell::PRESET | cell::BLANK) == 0)
        .count();
    assert_eq!(rooms.len(), outdoor_cells);
    assert_eq!(outdoor.rooms.len(), outdoor_cells);
    let preset_cells = (0..12)
        .flat_map(|y| (0..7).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            info.grids[2].get(x, y) & cell::PRESET != 0 && info.grids[0].get(x, y) != 0
        })
        .count();
    assert_eq!(presets.calls.len(), preset_cells);
    // The Rogue Encampment edge is a preset link; shrines were placed.
    assert!(info.vertices.iter().any(|v| v.is_preset_link()));
    assert!(info.grids[2].cells.iter().any(|&c| c & cell::SHRINE != 0));
}

#[test]
fn border_substitution_tests_and_style_maps() {
    // Act V: the style map decides the test and the stamp.
    let mut e = Env::new(111, 4, 4);
    e.od.subs.push(SubRow {
        type_: 12,
        file: b"d".to_vec(),
        bord_type: 2,
        grid_size: 1,
        ..SubRow::default()
    });
    let wall = |v: u32| (48 << 20) | (v << 8) | 1;
    let mut f = one_cell_file(0, wall(2), 1);
    f.walls[0].set(2, 0, wall(1));
    e.subs.0.insert(b"d".to_vec(), f);
    e.gen().op(0, 1, 1, Op::Set, 881);
    e.gen().op(0, 2, 1, Op::Set, 881);
    e.gen().op(2, 2, 1, Op::Or, cell::LINK);
    e.gen().border_sub(BorderCtx::barricade()).unwrap();
    assert_eq!(e.info.grids[0].get(1, 1), 883);
    // The border flag sets bit 0x1 only for presets 4..15 and 364..375.
    assert_eq!(e.info.grids[2].get(1, 1), cell::PRESET);
    assert_eq!(e.info.grids[0].get(2, 1), 881);
    assert_eq!(e.info.grids[0].get(0, 0), 0);
    // Wild: a style wall needs grid 0 = base + style unless style = S
    // (62); the replacement stamps base + style with the border flag.
    let mut e = Env::new(4, 4, 4);
    e.od.subs.push(SubRow {
        type_: 0,
        file: b"c".to_vec(),
        bord_type: 2,
        grid_size: 1,
        ..SubRow::default()
    });
    let mut f = one_cell_file(0, 0x201, 1);
    f.walls[0].set(2, 0, 0x301);
    e.subs.0.insert(b"c".to_vec(), f);
    e.gen().op(0, 1, 1, Op::Set, 5);
    e.gen().op(0, 2, 2, Op::Set, 5);
    e.gen().op(2, 2, 2, Op::Or, cell::LINK);
    e.gen().border_sub(BorderCtx::wild(0, 4)).unwrap();
    assert_eq!(e.info.grids[0].get(1, 1), 6);
    assert_ne!(e.info.grids[2].get(1, 1) & cell::BORDER, 0);
    assert_eq!(e.info.grids[0].get(2, 2), 5);
    // Floor bit 2: the cell must fit 1×1; the variant keeps it.
    let mut e = Env::new(4, 3, 3);
    e.od.subs.push(SubRow {
        type_: 0,
        file: b"k".to_vec(),
        bord_type: 2,
        grid_size: 1,
        ..SubRow::default()
    });
    let mut f = one_cell_file(2, 0, 1);
    f.floor.as_mut().unwrap().set(2, 0, 2);
    e.subs.0.insert(b"k".to_vec(), f);
    e.gen().op(2, 1, 1, Op::Or, cell::SHRINE | 0x40);
    e.gen().border_sub(BorderCtx::wild(0, 4)).unwrap();
    assert_eq!(e.info.grids[2].get(1, 1), cell::SHRINE | 0x40);
    assert_eq!(e.info.grids[2].get(0, 0), 0);
    // A file without groups is fatal.
    e.subs.0.get_mut(&b"k"[..]).unwrap().groups.clear();
    assert_eq!(
        e.gen().border_sub(BorderCtx::wild(0, 4)),
        Err(OutdoorError::SubFileNoGroups(b"k".to_vec()))
    );
}

#[test]
fn border_styles() {
    use crate::drlg::outdoor::vertex::style;
    assert_eq!(style(2, 4, 0), 1);
    assert_eq!(style(2, 4, 1), 0);
    assert_eq!(style(16, 41, 0), 2);
    assert_eq!(style(27, 104, 1), 3);
    assert_eq!(style(31, 111, 0), 4);
    assert_eq!(style(31, 117, 0), 5);
    assert_eq!(style(21, 76, 0), -1);
}

/// A level of 6 × 6 cells with a W neighbour box giving a link edge
/// (0, 4) → (0, 1).
fn west_link(id: u32, level_type: u32) -> Env {
    let mut e = Env::new(id, 6, 6);
    e.drlg.level_mut(e.l).level_type = level_type;
    e.info.orth = vec![orth(0, TileRect::new(752, 808, 48, 32))];
    e
}

#[test]
fn link_midpoints() {
    let mut e = west_link(4, 2);
    let mut g = e.gen();
    g.polygon().unwrap();
    assert!(g.info.vertices[1].is_link());
    assert_eq!((g.info.vertices[1].x, g.info.vertices[1].y), (0, 4));
    g.borders().unwrap();
    assert_eq!(g.g(2, 0, 2) & 0xF0400, 0x30400);
    let mut e = west_link(17, 2);
    let mut g = e.gen();
    g.polygon().unwrap();
    g.borders().unwrap();
    assert_eq!(g.g(2, 0, 2) & 0xF0400, 0x40400);
    // Act II: the desert pair (373, 372) at the midpoint and the next
    // cell; the corner at (0, 1) overwrites the second.
    let mut e = west_link(41, 16);
    let mut g = e.gen();
    g.polygon().unwrap();
    g.borders().unwrap();
    assert_eq!(g.g(0, 0, 2), 373);
    // Act III: nothing at the midpoint.
    let mut e = west_link(76, 16);
    let mut g = e.gen();
    g.polygon().unwrap();
    g.borders().unwrap();
    assert_eq!(g.g(2, 0, 2) & 0xF0400, 0);
}

#[test]
fn town_transition_stamps() {
    let mut e = Env::new(2, 10, 10);
    e.info.flags = 0x80 | 0x100 | 0x200 | 0x400 | 0x40;
    let s0 = e.seed();
    e.gen().transitions().unwrap();
    assert_eq!(e.seed(), s0);
    let f = |e: &Env, x, y| {
        (
            e.info.grids[0].get(x, y),
            (e.info.grids[2].get(x, y) >> 16) & 0xF,
        )
    };
    assert_eq!(f(&e, 0, 0), (3, 1));
    assert_eq!(f(&e, 3, 0), (3, 2));
    assert_eq!(f(&e, 0, 1), (2, 1));
    assert_eq!(f(&e, 0, 4), (2, 1));
}

#[test]
fn all_placement_cases() {
    let p = TileRect::new(100, 200, 40, 30);
    let c = TileRect::new(0, 0, 10, 20);
    let at = |f: fn(TileRect, &mut TileRect, i32, i32), case, v| {
        let mut r = c;
        f(p, &mut r, case, v);
        (r.x, r.y)
    };
    // Place A.
    assert_eq!(at(place::place_a, 0, 0), (100, 230));
    assert_eq!(at(place::place_a, 0, 1), (84, 230));
    assert_eq!(at(place::place_a, 1, 0), (90, 200));
    assert_eq!(at(place::place_a, 1, 1), (90, 184));
    assert_eq!(at(place::place_a, 1, 2), (90, 208));
    assert_eq!(at(place::place_a, 2, 0), (130, 180));
    assert_eq!(at(place::place_a, 2, 1), (146, 180));
    assert_eq!(at(place::place_a, 3, 0), (140, 210));
    assert_eq!(at(place::place_a, 3, 1), (140, 226));
    assert_eq!(at(place::place_a, 3, 2), (140, 202));
    assert_eq!(at(place::place_a, 3, 3), (140, 218));
    // Place B.
    assert_eq!(at(place::place_b, 0, 0), (130, 230));
    assert_eq!(at(place::place_b, 0, 1), (146, 230));
    assert_eq!(at(place::place_b, 1, 0), (90, 210));
    assert_eq!(at(place::place_b, 1, 1), (90, 226));
    assert_eq!(at(place::place_b, 1, 2), (90, 202));
    assert_eq!(at(place::place_b, 2, 0), (100, 180));
    assert_eq!(at(place::place_b, 2, 1), (84, 180));
    assert_eq!(at(place::place_b, 3, 0), (140, 200));
    assert_eq!(at(place::place_b, 3, 1), (140, 184));
    assert_eq!(at(place::place_b, 3, 2), (140, 208));
    assert_eq!(at(place::place_b, 3, 3), (140, 192));
    // Place C, variant 1 (half = 5 + 8 = 13 in x, 10 + 8 = 18 in y).
    let cs: Vec<(i32, i32)> = (0..8).map(|k| at(place::place_c, k, 1)).collect();
    assert_eq!(
        cs,
        [
            (87, 230),
            (113, 230),
            (90, 182),
            (90, 218),
            (87, 180),
            (113, 180),
            (140, 182),
            (140, 218)
        ]
    );
    assert_eq!(at(place::place_c, 1, 0), (100, 230));
}

/// Units are prepended, so the room's list is the reverse of the pattern file's list
/// (the waypoint ends up last).
#[test]
fn apply_prepends_preset_units() {
    let mut file = one_cell_file(0x0010_0002, 0, 1);
    let u = |class, x| PresetUnit {
        unit_type: 2,
        class,
        x,
        y: 2,
    };
    file.units = vec![u(1, 1), u(2, 2), u(3, 3)];
    let mut r = room();
    let mut rs = RoomSub {
        w: 8,
        h: 8,
        tile_x: 0,
        tile_y: 0,
        room: &mut r,
        pick: None,
    };
    let g = file.groups[0];
    tilesub::apply(&mut Seed::default(), &mut rs, &file, g, 0, 0, 0).unwrap();
    let classes: Vec<u32> = rs.room.units.iter().map(|p| p.class).collect();
    assert_eq!(classes, [3, 2, 1]);
}

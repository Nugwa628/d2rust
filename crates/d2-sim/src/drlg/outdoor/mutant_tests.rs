//! Tests written to kill mutants that `cargo mutants` left alive in `drlg::outdoor`.
//!
//! Each test states a generation rule in its doc comment and checks the code against it,
//! often through a small independent model of the rule.

use d2_data::tables::{Leveldefs, Lvlprest, Lvlsub, Record};

use super::tests::{act1_data, data, od, Presets, Rec};
use super::*;
use crate::drlg::room::LinkAt;
use crate::drlg::tiles::RoomGrids;
use crate::drlg::{DrlgError, DrlgRoomId, LevelIdx, NoLevelTypes, PresetUnit, RoomKind, TileRect};
use crate::rng::Seed;

fn u32s(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// The outdoor table view reads leveldefs `SubType`, `SubTheme`, `SubWaypoint` and
/// `SubShrine`, lvlprest `SizeX/Y` and `Files`, and every lvlsub column in file order.
#[test]
fn table_view_from_records() {
    let mut l = vec![0u8; Leveldefs::SIZE];
    u32s(&mut l, 56, 6);
    u32s(&mut l, 60, u32::MAX);
    u32s(&mut l, 64, 2);
    u32s(&mut l, 68, 3);
    let mut p = vec![0u8; Lvlprest::SIZE];
    u32s(&mut p, 40, 16);
    u32s(&mut p, 44, 24);
    u32s(&mut p, 64, 5);
    let mut s = vec![0u8; Lvlsub::SIZE];
    u32s(&mut s, 0, 4);
    s[4..9].copy_from_slice(b"x.ds1");
    u32s(&mut s, 64, 1);
    u32s(&mut s, 68, 2);
    u32s(&mut s, 72, 0x30);
    u32s(&mut s, 76, 7);
    for k in 0..5u32 {
        u32s(&mut s, 284 + 4 * k as usize, 10 + k);
        u32s(&mut s, 304 + 4 * k as usize, 20 + k);
        u32s(&mut s, 324 + 4 * k as usize, 30 + k);
    }
    let d = OutdoorData::from_tables(
        &[Leveldefs::decode(&l)],
        &[Lvlprest::decode(&p)],
        &[Lvlsub::decode(&s)],
    );
    assert_eq!(
        d.sub_defs(0),
        SubDefs {
            sub_type: 6,
            sub_theme: -1,
            sub_waypoint: 2,
            sub_shrine: 3,
        }
    );
    assert_eq!(
        d.preset(0),
        Ok(&PresetDef {
            size_x: 16,
            size_y: 24,
            files: 5,
        })
    );
    assert_eq!(
        d.subs,
        [tilesub::SubRow {
            type_: 4,
            file: b"x.ds1".to_vec(),
            check_all: 1,
            bord_type: 2,
            dt1_mask: 0x30,
            grid_size: 7,
            prob: [10, 11, 12, 13, 14],
            trials: [20, 21, 22, 23, 24],
            max: [30, 31, 32, 33, 34],
        }]
    );
}

/// Wall grid k ≥ 1 reads ORed with k << 18
/// (layer 0 as stored); tile-type grid k is the layer's own grid; a
/// missing layer reads 0.
#[test]
fn sub_file_layer_reads() {
    let grid = |v: u32| {
        let mut g = crate::drlg::tiles::CellGrid::new(2, 2);
        g.set(1, 1, v);
        g
    };
    let f = SubFile {
        walls: vec![grid(5), grid(5), grid(5), grid(5)],
        tile_types: vec![grid(9), grid(11)],
        ..SubFile::default()
    };
    assert_eq!(f.wall_at(0, 1, 1), 5);
    assert_eq!(f.wall_at(1, 1, 1), 5 | 1 << 18);
    assert_eq!(f.wall_at(3, 1, 1), 5 | 3 << 18);
    assert_eq!(f.wall_at(3, 0, 0), 3 << 18);
    assert_eq!(f.wall_at(4, 1, 1), 0);
    assert_eq!(f.tile_type_at(0, 1, 1), 9);
    assert_eq!(f.tile_type_at(1, 1, 1), 11);
    assert_eq!(f.tile_type_at(2, 1, 1), 0);
}

// ---- the LevelTypes adapter (dispatch by level type) ------------------------

/// Records every hook the adapter forwards to the other level types.
#[derive(Default)]
struct Hooks {
    inits: Vec<u32>,
    resets: Vec<u32>,
    added: Vec<DrlgRoomId>,
    grids: Vec<DrlgRoomId>,
    freed: Vec<DrlgRoomId>,
    doors: Vec<(i32, i32, u32, u32)>,
    warps: Vec<(i32, i32, u32, u32)>,
}

fn marker_unit() -> PresetUnit {
    PresetUnit {
        unit_type: 2,
        class: 7,
        x: 1,
        y: 2,
    }
}

fn marker_grids() -> RoomGrids {
    RoomGrids {
        kill_edge_x: true,
        anim_speed: 9,
        ..RoomGrids::default()
    }
}

impl LevelTypes for Hooks {
    fn init_level(&mut self, drlg: &mut Drlg, _: &DrlgData, l: LevelIdx) -> Result<(), DrlgError> {
        self.inits.push(drlg.level(l).id);
        Ok(())
    }

    fn generate(&mut self, drlg: &mut Drlg, _: &DrlgData, l: LevelIdx) -> Result<(), DrlgError> {
        let r = drlg.alloc_room(l, RoomKind::Preset, TileRect::new(0, 0, 8, 8));
        drlg.link_room(r, LinkAt::Tail);
        Ok(())
    }

    fn reset_level(&mut self, drlg: &mut Drlg, l: LevelIdx) {
        self.resets.push(drlg.level(l).id);
    }

    fn add_preset_units(&mut self, _: &mut Drlg, room: DrlgRoomId) -> Result<(), DrlgError> {
        self.added.push(room);
        Ok(())
    }

    fn preset_units(&self, _: &Drlg, _: DrlgRoomId) -> Vec<PresetUnit> {
        vec![marker_unit()]
    }

    fn room_grids(
        &mut self,
        _: &mut Drlg,
        _: &DrlgData,
        room: DrlgRoomId,
    ) -> Result<RoomGrids, DrlgError> {
        self.grids.push(room);
        Ok(marker_grids())
    }

    fn free_room_tiles(&mut self, _: &mut Drlg, room: DrlgRoomId) {
        self.freed.push(room);
    }

    #[allow(clippy::too_many_arguments)]
    fn door_unit(
        &mut self,
        _: &mut Drlg,
        _: &DrlgData,
        _: DrlgRoomId,
        wx: i32,
        wy: i32,
        cell: u32,
        orientation: u32,
    ) -> bool {
        self.doors.push((wx, wy, cell, orientation));
        false
    }

    #[allow(clippy::too_many_arguments)]
    fn warp_unit(
        &mut self,
        _: &mut Drlg,
        _: &DrlgData,
        _: DrlgRoomId,
        wx: i32,
        wy: i32,
        cell: u32,
        orientation: u32,
    ) -> Result<bool, DrlgError> {
        self.warps.push((wx, wy, cell, orientation));
        Ok(true)
    }
}

/// The type hooks dispatch by DrlgType. A non-outdoor level and its rooms reach the
/// other level types for every hook; an outdoor level's init only zeroes its outdoor
/// info.
#[test]
fn adapter_dispatches_by_drlg_type() {
    let mut data = data();
    data.levels[5].drlg_type = 2;
    data.levels[5].size = [(8, 8); 3];
    data.levels[2].drlg_type = 3;
    let od = od();
    let subs = SubFileMap::default();
    let mut outdoor = Outdoor::default();
    let mut presets = Presets::default();
    let mut hooks = Hooks::default();
    let mut drlg = Drlg::create(0, 1, 0, 0, false, &data, &mut NoLevelTypes).unwrap();
    let mut types = OutdoorTypes {
        outdoor: &mut outdoor,
        od: &od,
        subs: &subs,
        presets: &mut presets,
        others: &mut hooks,
        last_error: None,
    };
    let l5 = drlg.get_or_alloc_level(&data, &mut types, 5).unwrap();
    let l2 = drlg.get_or_alloc_level(&data, &mut types, 2).unwrap();
    assert_eq!(types.outdoor.level(l2), Some(&OutdoorLevel::default()));
    assert_eq!(types.outdoor.level(l5), None);
    drlg.generate_level(&data, &mut types, l5).unwrap();
    let r = drlg.level_rooms(l5)[0];
    types.add_preset_units(&mut drlg, r).unwrap();
    assert_eq!(types.preset_units(&drlg, r), [marker_unit()]);
    assert_eq!(types.room_grids(&mut drlg, &data, r), Ok(marker_grids()));
    types.free_room_tiles(&mut drlg, r);
    types.door_unit(&mut drlg, &data, r, 3, 4, 5, 9);
    assert_eq!(types.warp_unit(&mut drlg, &data, r, 6, 7, 8, 11), Ok(true));
    types.reset_level(&mut drlg, l5);
    assert_eq!(types.last_error, None);
    assert_eq!(hooks.inits, [5]);
    assert_eq!(hooks.added, [r]);
    assert_eq!(hooks.grids, [r]);
    assert_eq!(hooks.freed, [r]);
    assert_eq!(hooks.doors, [(3, 4, 5, 9)]);
    assert_eq!(hooks.warps, [(6, 7, 8, 11)]);
    assert_eq!(hooks.resets, [5]);
}

/// Through the act placer, the levels it allocates get their type init, except outdoor
/// ones (zeroed info only).
#[test]
fn act_placer_inits_only_non_outdoor_levels() {
    let data = act1_data();
    let od = od();
    let subs = SubFileMap::default();
    let mut outdoor = Outdoor::default();
    let mut presets = Presets::default();
    let mut rec = Rec::default();
    let mut types = OutdoorTypes {
        outdoor: &mut outdoor,
        od: &od,
        subs: &subs,
        presets: &mut presets,
        others: &mut rec,
        last_error: None,
    };
    let drlg = Drlg::create(0, 644_409_375, 0, 0, false, &data, &mut types).unwrap();
    let mut levels = drlg.level_list();
    levels.sort();
    let want: Vec<u32> = levels
        .iter()
        .map(|&l| drlg.level(l))
        // Types 1 and 2 get a type init; outdoor (3) and 0 (e.g. the neighbour-entry
        // allocations 8..16, left at 0 in this data) none.
        .filter(|l| matches!(l.drlg_type, 1 | 2))
        .map(|l| l.id)
        .collect();
    assert!(!want.is_empty());
    assert_eq!(rec.inits, want);
}

/// Freeing a room's tiles frees its outdoor grids; the room's other outdoor data stays.
#[test]
fn outdoor_room_tiles_freed() {
    let grid = || {
        let mut g = crate::drlg::tiles::CellGrid::new(9, 9);
        g.set(1, 1, 3);
        g
    };
    let room = OutdoorRoom {
        flags: 4,
        tile_type: grid(),
        wall: grid(),
        floor: grid(),
        ..OutdoorRoom::default()
    };
    let mut o = Outdoor::default();
    o.rooms.insert(DrlgRoomId(3), room.clone());
    assert_eq!(o.room(DrlgRoomId(3)), Some(&room));
    assert_eq!(o.room(DrlgRoomId(4)), None);
    o.free_room_tiles(DrlgRoomId(3));
    assert_eq!(
        o.room(DrlgRoomId(3)),
        Some(&OutdoorRoom {
            flags: 4,
            ..OutdoorRoom::default()
        })
    );
}

// ---- acts.rs ------------------------------------------------------------------

/// The five (P, F, x, y) entries of each cliff row, compared as a set: the order of the
/// middle entries is not fixed.
#[test]
fn desert_cliff_rows() {
    use super::acts::desert_cliff_row;
    let sorted = |mut v: Vec<(u32, i32, i32, i32)>| {
        v.sort();
        v
    };
    for r in 0..8u32 {
        let want = match r {
            0..=2 => {
                let mut v = vec![(376, 1, 0, 4), (376, 2, 8, 4)];
                for x in [2, 4, 6] {
                    let p = if x == 2 + 2 * r as i32 { 378 } else { 377 };
                    v.push((p, -1, x, 4));
                }
                v
            }
            3 => vec![
                (376, 2, 8, 4),
                (377, -1, 6, 4),
                (382, -1, 4, 4),
                (381, -1, 4, 6),
                (379, 2, 4, 8),
            ],
            4 => vec![
                (376, 2, 8, 4),
                (378, -1, 6, 4),
                (382, -1, 4, 4),
                (380, -1, 4, 6),
                (379, 2, 4, 8),
            ],
            _ => {
                let mut v = vec![(379, 1, 4, 0), (379, 2, 4, 8)];
                for y in [2, 4, 6] {
                    let p = if y == 2 + 2 * (r as i32 - 5) {
                        381
                    } else {
                        380
                    };
                    v.push((p, -1, 4, y));
                }
                v
            }
        };
        assert_eq!(
            sorted(desert_cliff_row(r).to_vec()),
            sorted(want),
            "row {r}"
        );
    }
}

// ---- grid.rs: grids and preset primitives -----------------------------------

use super::grid::{cell, shuffle_cells, Op};
use super::tests::Env;

/// The grid ops on an in-grid cell; outside the grid nothing happens.
#[test]
fn grid_ops() {
    let mut g = Grid::new(2, 2);
    let apply = |g: &mut Grid, start: u32, op: Op, v: u32| {
        g.op(1, 1, Op::Set, start);
        g.op(1, 1, op, v);
        g.get(1, 1)
    };
    assert_eq!(apply(&mut g, 0b1100, Op::Or, 0b1010), 0b1110);
    assert_eq!(apply(&mut g, 0b1100, Op::And, 0b1010), 0b1000);
    assert_eq!(apply(&mut g, 0b1100, Op::Xor, 0b1010), 0b0110);
    assert_eq!(apply(&mut g, 0b1100, Op::Set, 0b1010), 0b1010);
    assert_eq!(apply(&mut g, 0b1100, Op::SetIfZero, 0b1010), 0b1100);
    assert_eq!(apply(&mut g, 0, Op::SetIfZero, 0b1010), 0b1010);
    assert_eq!(apply(&mut g, 0b1100, Op::AndNot, 0b1010), 0b0100);
    g.op(2, 0, Op::Set, 7);
    g.op(-1, 0, Op::Set, 7);
    assert_eq!(g.cells, [0, 0, 0, 0b0100]);
}

/// Every covered cell of a multi-cell preset
/// gets the preset bit, the file in bits 16..19 and (border presets with
/// the flag) bit 0x1; grid 0 is cleared and (x, y) := P.
#[test]
fn stamp_covers_every_cell() {
    let mut e = Env::new(2, 8, 8);
    e.od.presets[5] = PresetDef {
        size_x: 16,
        size_y: 24,
        files: 1,
    };
    e.info.grids[2].op(3, 4, Op::Set, 0x7_0000);
    e.info.grids[0].op(3, 3, Op::Set, 9);
    let mut g = e.gen();
    g.stamp(2, 3, 5, 6, true).unwrap();
    for y in 0..8 {
        for x in 0..8 {
            let inside = (2..4).contains(&x) && (3..6).contains(&y);
            let want = if inside {
                cell::PRESET | 6 << 16 | cell::BORDER
            } else {
                0
            };
            assert_eq!(g.g(2, x, y), want, "({x}, {y})");
            let p = if (x, y) == (2, 3) { 5 } else { 0 };
            assert_eq!(g.g(0, x, y), p, "({x}, {y})");
        }
    }
}

/// Margin m with flag bits 1 (y − m, h + m), 2 (w + m),
/// 4 (h + m), 8 (x − m, w + m); every cell must be spawn valid.
#[test]
fn fits_margins_by_flag() {
    for (flags, blocked, free) in [
        (1, (5, 3), (5, 7)),
        (2, (7, 5), (3, 5)),
        (4, (5, 7), (5, 3)),
        (8, (3, 5), (7, 5)),
    ] {
        for (at, want) in [(blocked, false), (free, true)] {
            let mut e = Env::new(2, 10, 10);
            e.info.grids[2].op(at.0, at.1, Op::Set, cell::NOT_SPAWN);
            let g = e.gen();
            assert_eq!(g.fits(5, 5, 0, 2, flags), Ok(want), "flags {flags} {at:?}");
            // Without a margin the flags do nothing.
            assert_eq!(g.fits(5, 5, 0, 0, flags), Ok(true));
        }
    }
}

/// The candidate cell of entry k is (x + 1, y + 1).
#[test]
fn shuffle_candidates_are_offset_by_one() {
    let mut e = Env::new(2, 6, 5);
    let mut s = e.seed();
    let want: Vec<_> = shuffle_cells(&mut s, 4, 3)
        .into_iter()
        .map(|(x, y)| (x + 1, y + 1))
        .collect();
    let mut g = e.gen();
    assert_eq!(g.shuffle(), want);
}

/// SpawnRandomDS1: around the (only) path cell, the
/// neighbours are tried in the order dx = [−1, 0, 0, 1, −1, 1, 1, −1],
/// dy = [0, −1, 1, 0, −1, 1, −1, 1]; the first that fits is stamped.
#[test]
fn random_ds1_neighbour_order() {
    const DX: [i32; 8] = [-1, 0, 0, 1, -1, 1, 1, -1];
    const DY: [i32; 8] = [0, -1, 1, 0, -1, 1, -1, 1];
    for k in 0..8 {
        let mut e = Env::new(2, 9, 9);
        for y in 0..9 {
            for x in 0..9 {
                e.info.grids[2].op(x, y, Op::Set, cell::BLANK);
            }
        }
        e.info.grids[2].op(4, 4, Op::Set, cell::PATH);
        // Neighbours k.. are free; the earlier ones blocked.
        for (j, (dx, dy)) in DX.iter().zip(DY).enumerate().skip(k) {
            let _ = j;
            e.info.grids[2].op(4 + dx, 4 + dy, Op::Set, 0);
        }
        // One shuffle only: no fallback to SpawnOutdoorLevelPreset.
        let mut s = e.seed();
        shuffle_cells(&mut s, 7, 7);
        let mut g = e.gen();
        g.random_ds1(7, 0).unwrap();
        assert_eq!(g.g(0, 4 + DX[k], 4 + DY[k]), 7, "k {k}");
        assert_eq!(*g.seed(), s, "k {k}");
    }
}

/// FarAway, computed from the rules: rx := roll(gw
/// − 2), ry := roll(gh − 2); W, H := gw − 2, gh − 2; centre (rect.x +
/// rect.w/2, rect.y + rect.h/2); i in 0..=H, j in 0..=W: cell ((j + rx)
/// mod W + 1, (i + ry) mod H + 1); ax := |8x − cx + level.x + 4|, ay
/// likewise; d := (ax > ay ? ay + 2ax : ax + 2ay) / 2; the first strictly
/// larger d wins.
#[test]
fn far_away_by_the_rule() {
    // Rects around the level and a symmetric one (the level's centre:
    // corner ties decided by the scan start).
    let mut rects: Vec<TileRect> = (0..300)
        .map(|k| {
            TileRect::new(
                780 + 7 * k % 90,
                790 + 13 * k % 80,
                1 + 5 * k % 23,
                1 + 3 * k % 17,
            )
        })
        .collect();
    rects.extend([TileRect::new(800, 800, 72, 64); 40]);
    for (k, rect) in rects.into_iter().enumerate() {
        let blocked: Vec<(i32, i32)> = (0..k as i32 % 4)
            .map(|b| (1 + (3 * b + k as i32) % 7, 1 + (5 * b + k as i32) % 6))
            .collect();
        far_away_case(rect, 1000 + 7919 * k as u32, &blocked);
    }
    // Only (1, 2) and (2, 1) fit, with raw values 80 and 81 (d 40 both):
    // the first visited wins.
    let mut blocked = Vec::new();
    for y in 1..7 {
        for x in 1..8 {
            if (x, y) != (1, 2) && (x, y) != (2, 1) {
                blocked.push((x, y));
            }
        }
    }
    for k in 0..20 {
        far_away_case(TileRect::new(790, 791, 1, 1), 7919 * k, &blocked);
    }
}

/// FarAway on a 9 × 8 grid at (800, 800) against the rules.
fn far_away_case(rect: TileRect, seed: u32, blocked: &[(i32, i32)]) {
    let (gw, gh) = (9, 8);
    let mut e = Env::new(2, gw, gh);
    e.drlg.level_mut(e.l).seed = Seed::init_low(seed);
    for &(x, y) in blocked {
        e.info.grids[2].op(x, y, Op::Set, cell::NOT_SPAWN);
    }
    let level = e.drlg.level(e.l).rect;
    let mut s = e.seed();
    let rx = s.roll(gw - 2) as i32;
    let ry = s.roll(gh - 2) as i32;
    let (w, h) = (gw - 2, gh - 2);
    let (cx, cy) = (rect.x + rect.w / 2, rect.y + rect.h / 2);
    let mut best: Option<(i32, i32, i32)> = None;
    for i in 0..=h {
        for j in 0..=w {
            let x = (j + rx) % w + 1;
            let y = (i + ry) % h + 1;
            if blocked.contains(&(x, y)) {
                continue;
            }
            let ax = (8 * x - cx + level.x + 4).abs();
            let ay = (8 * y - cy + level.y + 4).abs();
            let d = if ax > ay { ay + 2 * ax } else { ax + 2 * ay } / 2;
            if best.is_none_or(|b| d > b.2) {
                best = Some((x, y, d));
            }
        }
    }
    let (bx, by, _) = best.unwrap();
    let mut g = e.gen();
    assert_eq!(g.far_away(rect, 9, 0, 0, 15), Ok(true));
    assert_eq!(g.g(0, bx, by), 9, "{rect:?} seed {seed}");
    assert_eq!(*g.seed(), s, "two draws");
}

/// Waypoint, level 3: i := first vis slot of level 3
/// holding 2, mask := 1 << (i + 4); the first cell (rows, then columns)
/// with grid-1 & mask **and** grid-2 & 0x400, clamped to 1..gw−2,
/// 1..gh−2, gets grid 1 |= 0x20000, grid 2 |= 0x800.
#[test]
fn waypoint_on_the_cold_plains_link() {
    for (at, clamped) in [((0, 3), (1, 3)), ((7, 2), (6, 2)), ((4, 7), (4, 6))] {
        let mut e = Env::new(3, 8, 8);
        e.data.levels[3].vis = [17, 2, 0, 0, 0, 0, 0, 0];
        let mask = 1 << (1 + 4);
        // Decoys earlier in scan order: mask only, link only.
        e.info.grids[1].op(1, 0, Op::Set, mask);
        e.info.grids[2].op(2, 0, Op::Set, cell::LINK);
        e.info.grids[1].op(at.0, at.1, Op::Set, mask);
        e.info.grids[2].op(at.0, at.1, Op::Set, cell::LINK);
        let mut g = e.gen();
        g.waypoint().unwrap();
        let (x, y) = clamped;
        assert_eq!(g.g(1, x, y) & 0x20000, 0x20000, "{at:?}");
        assert_eq!(g.g(2, x, y) & cell::WAYPOINT, cell::WAYPOINT, "{at:?}");
    }
}

/// Shrines(n): k := lo' & 3, then each placed shrine
/// takes bit [0x1000, 0x2000, 0x4000, 0x8000][k] and k := (k + 1) mod 4.
#[test]
fn shrines_cycle_their_bits() {
    let mut e = Env::new(2, 8, 8);
    let mut g = e.gen();
    g.shrines(5);
    let mut bits: Vec<u32> = Vec::new();
    for y in 0..8 {
        for x in 0..8 {
            if g.g(2, x, y) & cell::SHRINE != 0 {
                bits.push(g.g(1, x, y));
            }
        }
    }
    bits.sort();
    // Five shrines: one bit twice, the other three once.
    assert_eq!(bits.len(), 5);
    for b in [0x1000, 0x2000, 0x4000, 0x8000] {
        assert!(bits.contains(&b), "{b:#x}");
    }
}

// ---- place.rs: checks --------------------------------------------------------

/// Shares an edge (margin −1) when one gap is 0 and the
/// other ≤ −1.
#[test]
fn shares_edge_needs_one_zero_gap_and_one_overlap() {
    use super::place::shares_edge;
    let a = TileRect::new(0, 0, 8, 8);
    // East, overlapping in y.
    assert!(shares_edge(&a, &TileRect::new(8, 4, 8, 8)));
    // South, overlapping in x.
    assert!(shares_edge(&a, &TileRect::new(4, 8, 8, 8)));
    // Corner only: both gaps 0.
    assert!(!shares_edge(&a, &TileRect::new(8, 8, 8, 8)));
    // Gap 0 on x, gap 1 on y: apart.
    assert!(!shares_edge(&a, &TileRect::new(8, 9, 8, 8)));
    assert!(!shares_edge(&a, &TileRect::new(9, 8, 8, 8)));
}

/// The direction needs both
/// conditions (W: B.x < A.x and A.x = B.x + B.w).
#[test]
fn place_direction_needs_both_conditions() {
    use super::place::direction;
    let a = TileRect::new(8, 8, 8, 8);
    assert_eq!(direction(&a, &TileRect::new(4, 0, 8, 8)), 1);
    assert_eq!(direction(&a, &TileRect::new(0, 4, 8, 8)), 0);
    assert_eq!(direction(&a, &TileRect::new(0, 20, 4, 4)), -1);
    assert_eq!(direction(&a, &TileRect::new(20, 0, 4, 4)), -1);
    assert_eq!(direction(&a, &TileRect::new(20, 4, 4, 4)), 1);
    assert_eq!(direction(&a, &TileRect::new(16, 4, 4, 4)), 2);
    assert_eq!(direction(&a, &TileRect::new(8, 16, 4, 4)), 3);
}

// ---- place.rs: act-wide placement --------------------------------------------

/// DRLG creation of act index `act` through the outdoor adapter.
fn create_act(act: u8, seed: u32, data: &DrlgData) -> Result<(Drlg, Outdoor), DrlgError> {
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
    let drlg = Drlg::create(act, seed, 0, 0, false, data, &mut types)?;
    Ok((drlg, outdoor))
}

fn rect_of(drlg: &Drlg, id: u32) -> TileRect {
    drlg.level(drlg.find_level(id).unwrap()).rect
}

/// Driver: each linked row pair gets a warp slot toward the other level with warp id
/// −1 (slot −1, warp −1).
#[test]
fn act1_linked_levels_get_warp_minus_one() {
    let data = act1_data();
    let (drlg, _) = create_act(0, 644_409_375, &data).unwrap();
    for (a, b) in [(3, 4), (2, 3), (1, 2), (17, 3), (7, 26), (6, 7), (5, 6)] {
        for (x, y) in [(a, b), (b, a)] {
            let vis = drlg.vis_array(&data, x).unwrap();
            let warp = drlg.warp_array(&data, x).unwrap();
            let slot = vis.iter().position(|&v| v == y);
            assert!(slot.is_some(), "{x} → {y}");
            assert_eq!(warp[slot.unwrap()], -1, "{x} → {y}");
        }
    }
}

/// A1M check: for i > 0 the row-0 rect extended 200 upward
/// (y − 200, h + 200) must not overlap row i; a level far above that
/// band (and above row 0) is placed.
#[test]
fn a1m_row_zero_extension_is_200_tiles() {
    let mut data = act1_data();
    // Row 0: 39 at (5000, 1148, 64, 64) → band y 948..1212. Row 1: 26 at
    // y 100..118, same columns.
    data.levels[26].offset = (5000, 100);
    let (drlg, _) = create_act(0, 644_409_375, &data).unwrap();
    assert_eq!(rect_of(&drlg, 26), TileRect::new(5000, 100, 40, 18));
}

/// Driver: after level 6, R0 = 1 → preset direction of
/// 27 := 2 − (lo' & 1), R0 = 3 → 1 − (lo' & 1), one step of the DRLG
/// seed itself (the only DRLG-seed draw of Act I placement here).
#[test]
fn level_27_preset_direction_from_level_6() {
    let data = act1_data();
    let mut seen_r3 = false;
    for k in 0..40u32 {
        let init = 644_409_375u32.wrapping_add(k.wrapping_mul(7919));
        let (_, o) = create_act(0, init, &data).unwrap();
        let mut s = Seed::init_low(init);
        s.step();
        let bit = (s.step() & 1) as i32;
        match o.preset_direction.get(&27) {
            None => {}
            Some(&v) => {
                // R0 = 1 gives 2 − bit, R0 = 3 gives 1 − bit.
                assert!(v == 2 - bit || v == 1 - bit, "init {init}: {v}");
                if v == 0 || (bit == 0 && v == 1) {
                    seen_r3 = true;
                }
            }
        }
    }
    assert!(seen_r3, "some seed has R0 = 3 for level 6");
}

/// Jungle placer: block 0 at Kurast Docks (x, y − SY); for k = 1, 2:
/// base := roll(k), case := lo' mod 5; offsets (0, −SY), (−SX, y1),
/// (SX, y1), (−SX, y3), (SX, y3); an overlapping block redoes k; levels
/// 76..78 by y descending. The DRLG seed itself, after the start seed
/// and the jungle-link step.
#[test]
fn jungle_blocks_by_the_rule() {
    let mut data = data();
    let set = |d: &mut DrlgData, id: usize, ty: u32, size: (i32, i32), off: (i32, i32)| {
        d.levels[id].drlg_type = ty;
        d.levels[id].size = [size; 3];
        d.levels[id].offset = off;
    };
    set(&mut data, 75, 2, (400, 100), (2000, 5000));
    for id in 76..=78 {
        set(&mut data, id, 3, (64, 192), (0, 0));
    }
    let sizes = [(80, 40), (80, 48), (80, 56), (40, 40), (120, 80)];
    for (k, id) in (79..=83).enumerate() {
        set(&mut data, id, 3, sizes[k], (0, 0));
    }
    let (sx, sy) = (64, 192);
    let (y1, y3) = super::place::jungle_offsets(sy);
    for init in [99u32, 7, 12345, 4_000_000_000, 31337, 2024] {
        let (drlg, _) = create_act(2, init, &data).unwrap();
        let mut s = Seed::init_low(init);
        s.step();
        s.step();
        let mut blocks = vec![TileRect::new(2000, 5000 - sy, sx, sy)];
        for k in 1..=2i32 {
            loop {
                let base = s.roll(k) as usize;
                let (ox, oy) = match s.step() % 5 {
                    0 => (0, -sy),
                    1 => (-sx, y1),
                    2 => (sx, y1),
                    3 => (-sx, y3),
                    _ => (sx, y3),
                };
                let b = blocks[base];
                let nb = TileRect::new(b.x + ox, b.y + oy, sx, sy);
                if blocks.iter().any(|e| super::place::overlaps(e, &nb)) {
                    continue;
                }
                blocks.push(nb);
                break;
            }
        }
        blocks.sort_by_key(|b| std::cmp::Reverse(b.y));
        for (n, b) in blocks.iter().enumerate() {
            assert_eq!(
                rect_of(&drlg, 76 + n as u32),
                *b,
                "init {init} level {}",
                76 + n
            );
        }
    }
}

/// Neighbour entries: only vis slots with warp id −1
/// make entries; a slot with a warp id does not.
#[test]
fn neighbour_entries_skip_warp_slots() {
    let mut data = act1_data();
    data.levels[2].vis = [1, 3, 17, 0, 0, 0, 0, 0];
    data.levels[2].warp = [-1, -1, 9, -1, -1, -1, -1, -1];
    let (drlg, o) = create_act(0, 644_409_375, &data).unwrap();
    let l2 = drlg.find_level(2).unwrap();
    let ids: Vec<u32> = o
        .level(l2)
        .unwrap()
        .orth
        .iter()
        .map(|e| e.level_id)
        .collect();
    assert!(ids.contains(&1) && ids.contains(&3), "{ids:?}");
    assert!(!ids.contains(&17), "{ids:?}");
}

/// The driver's set
/// warp passes slot −1, so a level without the vis entry gets it in its
/// first free slot (vis 0 and warp −1).
#[test]
fn driver_warps_take_the_first_free_slot() {
    let mut data = act1_data();
    data.levels[4].vis = [0; 8];
    data.levels[17].vis = [0; 8];
    let (drlg, _) = create_act(0, 644_409_375, &data).unwrap();
    assert_eq!(drlg.vis_array(&data, 4).unwrap()[0], 3);
    assert_eq!(drlg.vis_array(&data, 17).unwrap()[0], 3);
    // Cold Plains (vis [2, 4, 17, ...]) keeps its slots.
    assert_eq!(drlg.vis_array(&data, 3).unwrap()[..3], [2, 4, 17]);
}

// ---- rooms.rs: generation dispatch and cells to rooms ------------------------

/// The DT1 mask by level type.
#[test]
fn dt1_mask_by_level_type() {
    use super::rooms::dt1_mask;
    for t in 0..40 {
        let want = match t {
            2 => 0x44103,
            16 | 22 | 27 | 28 => 0x1,
            21 => 0x4,
            30 | 31 => 0x11,
            _ => 0,
        };
        assert_eq!(dt1_mask(t), want, "type {t}");
    }
}

/// Floor flags by level type (31 only for level 117).
#[test]
fn floor_flags_by_level_type() {
    use super::rooms::floor_flags;
    for t in 0..40 {
        let want = match t {
            16 => 0x100,
            21 => 0x12_0000,
            22 => 0x10_0000,
            27 => 0xA0_0000,
            28 => 0x160_0000,
            _ => 0,
        };
        assert_eq!(floor_flags(t, 1), want, "type {t}");
    }
    assert_eq!(floor_flags(31, 117), 0x60_0000);
    assert_eq!(floor_flags(31, 118), 0);
}

/// One outdoor level `id` at (800, 800) of `w` × `h` tiles, generated
/// directly; returns the DRLG, the level, the level seed before
/// generation and the preset calls.
#[allow(clippy::type_complexity)]
fn generate_one(
    id: u32,
    w: i32,
    h: i32,
    od: &OutdoorData,
    subs: &SubFileMap,
) -> Result<(Drlg, LevelIdx, Seed, Vec<(u32, i32, i32, u32, u32)>), OutdoorError> {
    let mut data = data();
    data.levels[id as usize].drlg_type = 3;
    let mut drlg = Drlg::create(0, 1, 0, 0, false, &data, &mut NoLevelTypes).unwrap();
    let l = drlg
        .get_or_alloc_level(&data, &mut NoLevelTypes, id)
        .unwrap();
    drlg.level_mut(l).rect = TileRect::new(800, 800, w, h);
    let before = drlg.level(l).seed;
    let mut o = Outdoor::default();
    let mut presets = Presets::default();
    o.generate(&mut drlg, &data, od, subs, &mut presets, l)?;
    Ok((drlg, l, before, presets.calls))
}

/// Generator dispatch by act: Act IV (Chaos Sanctum's 25 stamps), Act III (levels
/// 76..78 reach jungle stamping) and Act II (level 134 stamps 394 at (4, 4)).
#[test]
fn generation_dispatches_by_act() {
    let subs = SubFileMap::default();
    // Act IV, 108: 25 preset cells, the heart once.
    let (_, _, _, calls) = generate_one(108, 120, 120, &od(), &subs).unwrap();
    assert_eq!(calls.len(), 25);
    assert_eq!(calls.iter().filter(|c| c.0 == 862).count(), 1);
    assert_eq!(calls.iter().filter(|c| c.0 == 836).count(), 19);
    // Act III, 76 (one cell): jungle stamping runs; this level has no block ids (no act
    // creation): fatal 0x27.
    assert_eq!(
        generate_one(76, 8, 8, &od(), &subs).err(),
        Some(OutdoorError::Fatal(0x27))
    );
    // Act II, 134: with inert lvlsub rows for PB.
    let mut od = od();
    let mut subs = SubFileMap::default();
    for t in [1, 2, 3] {
        let name = format!("inert{t}").into_bytes();
        od.subs.push(SubRow {
            type_: t,
            file: name.clone(),
            bord_type: 1,
            grid_size: 1,
            ..SubRow::default()
        });
        subs.0
            .insert(name, super::tests::one_cell_file(0, (200 << 8) | 1, 1));
    }
    let (_, _, _, calls) = generate_one(134, 80, 80, &od, &subs).unwrap();
    assert!(
        calls.iter().any(|c| (c.0, c.1, c.2) == (394, 832, 832)),
        "{calls:?}"
    );
}

/// One-cell outdoor level `id` of level type `lt` (one outdoor room),
/// generated directly; returns the DRLG, the outdoor state and the room.
/// The callers use level 107, whose act build stamps nothing; Kurast 79 stamps its
/// border rows, so it is not used here.
fn one_room_level(id: u32, lt: u32, od: &OutdoorData) -> (Drlg, Outdoor, DrlgRoomId) {
    let mut data = data();
    data.levels[id as usize].drlg_type = 3;
    data.levels[id as usize].level_type = lt;
    let mut drlg = Drlg::create(0, 1, 0, 0, false, &data, &mut NoLevelTypes).unwrap();
    let l = drlg
        .get_or_alloc_level(&data, &mut NoLevelTypes, id)
        .unwrap();
    drlg.level_mut(l).rect = TileRect::new(800, 800, 8, 8);
    let mut o = Outdoor::default();
    let mut presets = Presets::default();
    o.generate(
        &mut drlg,
        &data,
        od,
        &SubFileMap::default(),
        &mut presets,
        l,
    )
    .unwrap();
    let r = drlg.level_rooms(l)[0];
    (drlg, o, r)
}

/// The room's outdoor data takes leveldefs `SubType`, `SubTheme` and the sub-theme pick
/// (bit k when lo' mod 100 < Prob[theme] of row k).
#[test]
fn outdoor_room_takes_sub_type_theme_and_pick() {
    let mut od = od();
    od.levels[107].sub_type = 5;
    od.levels[107].sub_theme = 1;
    for p in [100, 0, 100] {
        od.subs.push(SubRow {
            type_: 5,
            prob: [p; 5],
            ..SubRow::default()
        });
    }
    let (_, o, r) = one_room_level(107, 0, &od);
    let room = o.room(r).unwrap();
    assert_eq!((room.sub_type, room.sub_theme, room.picked), (5, 1, 0b101));
}

/// Room grids: floor cells (0..7) := 0x40002; floor flags
/// by level type OR into floor cells without bits 0x3F0FF80; wall and
/// floor grid edges |= 0x4.
#[test]
fn outdoor_room_grids_floor_and_edges() {
    let od = od();
    let (mut drlg, mut o, r) = one_room_level(107, 16, &od);
    let g = o
        .room_grids(&mut drlg, &od, &SubFileMap::default(), r)
        .unwrap();
    let (wall, floor) = (&g.passes[0].cells, &g.passes[1].cells);
    for y in 0..9 {
        for x in 0..9 {
            let edge = if x == 0 || y == 0 || x == 8 || y == 8 {
                0x4
            } else {
                0
            };
            let base = if x < 8 && y < 8 { 0x40002 } else { 0 };
            assert_eq!(floor.get(x, y), base | 0x100 | edge, "floor ({x}, {y})");
            assert_eq!(wall.get(x, y), edge, "wall ({x}, {y})");
        }
    }
}

/// The waypoint rows run when room flags bits 16..17
/// are set, the shrine rows when bits 12..15 are (picked := those bits;
/// bit 0 = row 0 here); a scattered row draws `Max[0]` group rolls on the
/// room seed.
#[test]
fn outdoor_room_waypoint_and_shrine_rows() {
    let mut od = od();
    od.levels[107].sub_waypoint = 7;
    od.levels[107].sub_shrine = 8;
    let mut subs = SubFileMap::default();
    for (t, max) in [(7, 3), (8, 5)] {
        let name = format!("s{t}").into_bytes();
        od.subs.push(SubRow {
            type_: t,
            file: name.clone(),
            max: [max; 5],
            ..SubRow::default()
        });
        subs.0.insert(name, super::tests::one_cell_file(0, 0, 1));
    }
    for (flags, steps) in [(0, 0), (0x1_0000, 3), (0x1000, 5), (0x1_1000, 8)] {
        let (mut drlg, mut o, r) = one_room_level(107, 0, &od);
        drlg.room_mut(r).flags |= flags;
        let mut s = drlg.room(r).seed;
        o.room_grids(&mut drlg, &od, &subs, r).unwrap();
        for _ in 0..steps {
            s.step();
        }
        assert_eq!(drlg.room(r).seed, s, "flags {flags:#x}");
    }
}

// ---- tilesub.rs: sub-theme pick and room substitution -----------------------

use super::tilesub::{apply, fixed_test, pick_sub_themes, random_test, room_substitution, RoomSub};
use crate::drlg::tiles::CellGrid;

/// Sub-theme pick: one room-seed step per row; bit k when
/// lo' mod 100 < Prob[theme] (strict) of row k, by the theme's column;
/// t = −1 or h = −1: nothing.
#[test]
fn sub_theme_pick_by_theme_column() {
    let seed = Seed::init_low(4242);
    let r: Vec<i32> = {
        let mut s = seed;
        (0..3).map(|_| (s.step() % 100) as i32).collect()
    };
    let mut od = od();
    // Row 0: theme 1 column above r0; row 1: exactly r1 (not picked);
    // row 2: theme 1 column 0 but theme 0 column 100.
    for (k, p1) in [(0, r[0] + 1), (1, r[1]), (2, 0)] {
        let mut prob = [100; 5];
        prob[1] = p1;
        od.subs.push(SubRow {
            type_: 6,
            prob,
            dt1_mask: 1 << (4 * k),
            ..SubRow::default()
        });
    }
    let mut s = seed;
    assert_eq!(pick_sub_themes(&od, &mut s, 6, 1), Ok((0b001, 0x1)));
    let mut s = seed;
    assert_eq!(pick_sub_themes(&od, &mut s, 6, 0), Ok((0b111, 0x111)));
    for (t, h) in [(-1, 1), (6, -1)] {
        let mut s = seed;
        assert_eq!(pick_sub_themes(&od, &mut s, t, h), Ok((0, 0)));
        assert_eq!(s, seed);
    }
}

/// An 8 × 8 room side: 9 × 9 grids, floor 0x2 on cells (0..7, 0..7).
fn room_side() -> OutdoorRoom {
    let mut floor = CellGrid::new(9, 9);
    for y in 0..8 {
        for x in 0..8 {
            floor.set(x, y, 0x2);
        }
    }
    OutdoorRoom {
        tile_type: CellGrid::new(9, 9),
        wall: CellGrid::new(9, 9),
        floor,
        ..OutdoorRoom::default()
    }
}

fn rsub(room: &mut OutdoorRoom) -> RoomSub<'_> {
    RoomSub {
        w: 8,
        h: 8,
        tile_x: 800,
        tile_y: 900,
        room,
        pick: None,
    }
}

/// A file with one group (x 0, y 0, w × h, N variants), method `m`, and a
/// floor pattern: match cells 0x2, variant v (at x offset (v + 1)·(w +
/// 1)) cells 0x2 | (v + 1) << 4.
fn pattern(w: i32, h: i32, n: i32, m: u32) -> SubFile {
    let width = ((n + 1) * (w + 1)) as usize;
    let mut f = CellGrid::new(width, h as usize);
    for v in -1..n {
        let o = (v + 1) * (w + 1);
        for j in 0..h {
            for i in 0..w {
                f.set((o + i) as usize, j as usize, 0x2 | ((v + 1) as u32) << 4);
            }
        }
    }
    SubFile {
        method: m,
        groups: vec![SubGroup {
            x: 0,
            y: 0,
            w,
            h,
            variants: n,
        }],
        floor: Some(f),
        ..SubFile::default()
    }
}

/// Room substitution: rows of type t while mask bits remain, bit k
/// = row k (the mask shifts right per row); a scattered row draws Max[h]
/// group rolls.
#[test]
fn room_substitution_rows_by_mask() {
    let mut od = od();
    let mut subs = SubFileMap::default();
    for (k, max) in [(0, 2), (1, 3)] {
        let name = format!("m{k}").into_bytes();
        od.subs.push(SubRow {
            type_: 9,
            file: name.clone(),
            max: [max; 5],
            ..SubRow::default()
        });
        subs.0.insert(name, pattern(1, 1, 1, 2));
    }
    for (mask, steps) in [(0, 0), (0b01, 2), (0b10, 3), (0b11, 5)] {
        let mut room = room_side();
        let mut rs = rsub(&mut room);
        let start = Seed::init_low(77);
        let mut s = start;
        room_substitution(&od, &subs, &mut s, &mut rs, 9, 0, mask).unwrap();
        let mut want = start;
        for _ in 0..steps {
            want.step();
        }
        assert_eq!(s, want, "mask {mask:#b}");
    }
}

/// The pasted floor cells (bit 0x80) of a room side.
fn pasted(room: &OutdoorRoom) -> Vec<(i32, i32, u32)> {
    let mut out = Vec::new();
    for y in 0..9 {
        for x in 0..9 {
            let v = room.floor.get(x, y);
            if v & 0x80 != 0 {
                out.push((x, y, v));
            }
        }
    }
    out
}

/// Scattered rows: per repetition G := group[roll(count)];
/// aw, ah := w − G.w, h − G.h (≤ 0: next, the roll stays drawn);
/// Trials > 0: x := roll(aw) + 1, y := roll(ah) + 1, apply on a passing fixed
/// test; Trials −1: shuffle aw·ah entries, the first passing at (x + 1,
/// y + 1).
#[test]
fn scattered_positions() {
    let file = pattern(1, 1, 1, 2);
    let row = |trials: i32| SubRow {
        type_: 9,
        file: b"p".to_vec(),
        max: [1; 5],
        trials: [trials; 5],
        ..SubRow::default()
    };
    let mut subs = SubFileMap::default();
    subs.0.insert(b"p".to_vec(), file.clone());
    for k in 0..6u32 {
        let start = Seed::init_low(500 + 7919 * k);
        // Trials 1.
        let mut odt = od();
        odt.subs.push(row(1));
        let mut room = room_side();
        let mut s = start;
        room_substitution(&odt, &subs, &mut s, &mut rsub(&mut room), 9, 0, 1).unwrap();
        let mut e = start;
        e.roll(1);
        let x = e.roll(7) as i32 + 1;
        let y = e.roll(7) as i32 + 1;
        assert_eq!(pasted(&room), [(x, y, 0x82)], "trials 1, k {k}");
        assert_eq!(s, e);
        // Trials −1. The cell left of the first entry's candidate fails
        // the fixed test (the candidate is (x + 1, y + 1)).
        let (x0, y0) = {
            let mut e = start;
            e.roll(1);
            shuffle_cells(&mut e, 7, 7)[0]
        };
        let mut odt = od();
        odt.subs.push(row(-1));
        let mut room = room_side();
        room.floor.set(x0 as usize, y0 as usize + 1, 0);
        let mut s = start;
        room_substitution(&odt, &subs, &mut s, &mut rsub(&mut room), 9, 0, 1).unwrap();
        let mut e = start;
        e.roll(1);
        let (x, y) = shuffle_cells(&mut e, 7, 7)[0];
        assert_eq!(pasted(&room), [(x + 1, y + 1, 0x82)], "trials -1, k {k}");
        assert_eq!(s, e);
    }
    // A group as wide as the room (aw = 0): only the group roll.
    let mut subs = SubFileMap::default();
    subs.0.insert(b"p".to_vec(), pattern(8, 1, 1, 2));
    let mut odt = od();
    odt.subs.push(row(1));
    let mut room = room_side();
    let start = Seed::init_low(9);
    let mut s = start;
    room_substitution(&odt, &subs, &mut s, &mut rsub(&mut room), 9, 0, 1).unwrap();
    let mut e = start;
    e.step();
    assert_eq!(s, e);
    assert!(pasted(&room).is_empty());
}

/// CheckAll rows: method 1 applies variant 0 at every
/// passing (x, y) with y in 1..H−1, x in 1..W−1, no draws; method 2 scans
/// 0..H−1 × 0..W−1, one room-seed step r := lo' mod 100 per passing cell,
/// and if Prob[h] < r: v := roll(N), apply at x offset (v + 1)·(G.w + 1).
#[test]
fn check_all_methods() {
    let row = |prob: i32| SubRow {
        type_: 9,
        file: b"c".to_vec(),
        check_all: 1,
        prob: [prob; 5],
        ..SubRow::default()
    };
    // Method 1.
    let mut subs = SubFileMap::default();
    subs.0.insert(b"c".to_vec(), pattern(1, 1, 1, 1));
    let mut odt = od();
    odt.subs.push(row(0));
    let mut room = room_side();
    let start = Seed::init_low(3);
    let mut s = start;
    room_substitution(&odt, &subs, &mut s, &mut rsub(&mut room), 9, 0, 1).unwrap();
    assert_eq!(s, start);
    let want: Vec<_> = (1..8)
        .flat_map(|y| (1..8).map(move |x| (x, y, 0x82)))
        .collect();
    assert_eq!(pasted(&room), want);
    // Method 2, two variants: the pasted value tells v.
    let mut subs = SubFileMap::default();
    subs.0.insert(b"c".to_vec(), pattern(1, 1, 2, 2));
    for prob in [30, 70] {
        let mut odt = od();
        odt.subs.push(row(prob));
        let mut room = room_side();
        let start = Seed::init_low(11 + prob as u32);
        let mut s = start;
        room_substitution(&odt, &subs, &mut s, &mut rsub(&mut room), 9, 0, 1).unwrap();
        let mut e = start;
        let mut want = Vec::new();
        for y in 0..8 {
            for x in 0..8 {
                let r = (e.step() % 100) as i32;
                if prob < r {
                    let v = e.roll(2);
                    want.push((x, y, (0x2 | (v + 1) << 4) | 0x80));
                }
            }
        }
        assert_eq!(pasted(&room), want, "prob {prob}");
        assert_eq!(s, e);
    }
}

/// Fixed test: every group cell whose pattern
/// floor has bit 2 needs room floor bit 2 and no bit of 0x3F0FF00, and no
/// room wall bit 1.
#[test]
fn fixed_test_covers_the_group_box() {
    let file = pattern(2, 2, 1, 1);
    let g = file.groups[0];
    for (block, x, y, want) in [
        ((5, 3), 3, 2, true),
        ((5, 3), 4, 2, false),
        ((5, 3), 4, 3, false),
        ((5, 3), 5, 2, false),
        ((5, 3), 4, 1, true),
        ((5, 3), 3, 3, true),
    ] {
        for kind in 0..3 {
            let mut room = room_side();
            match kind {
                0 => room.floor.set(block.0, block.1, 0),
                1 => room.floor.set(block.0, block.1, 0x2 | 0x100),
                _ => room.wall.set(block.0, block.1, 0x1),
            }
            let rs = rsub(&mut room);
            assert_eq!(
                fixed_test(&rs, &file, g, x, y),
                want,
                "{kind} at ({x}, {y})"
            );
        }
    }
}

/// Random test: pattern tile type equals the
/// room's; floor bit 2 needs room bit 2 and equal 0x3F0FF00 bits; wall
/// bit 1 likewise.
#[test]
fn random_test_rules() {
    let mut file = pattern(2, 1, 1, 2);
    let mut tt = CellGrid::new(6, 1);
    tt.set(1, 0, 5);
    file.tile_types = vec![tt];
    let mut w = CellGrid::new(6, 1);
    w.set(0, 0, 0x1 | 0x100);
    file.walls = vec![w];
    let g = file.groups[0];
    let base = || {
        let mut room = room_side();
        room.tile_type.set(4, 2, 5);
        room.wall.set(3, 2, 0x1 | 0x100);
        room
    };
    let mut room = base();
    assert!(random_test(&rsub(&mut room), &file, g, 3, 2));
    // Shifted: tile types and wall no longer line up.
    assert!(!random_test(&rsub(&mut room), &file, g, 2, 2));
    let mut room = base();
    room.tile_type.set(4, 2, 6);
    assert!(!random_test(&rsub(&mut room), &file, g, 3, 2));
    let mut room = base();
    room.floor.set(4, 2, 0x2 | 0x200);
    assert!(!random_test(&rsub(&mut room), &file, g, 3, 2));
    let mut room = base();
    room.floor.set(4, 2, 0);
    assert!(!random_test(&rsub(&mut room), &file, g, 3, 2));
    let mut room = base();
    room.wall.set(3, 2, 0x1 | 0x200);
    assert!(!random_test(&rsub(&mut room), &file, g, 3, 2));
    let mut room = base();
    room.wall.set(3, 2, 0x100);
    assert!(!random_test(&rsub(&mut room), &file, g, 3, 2));
}

/// Apply: roof growth by pattern shadow cells with
/// 0x8000000 at offset o; floor bit 2 → value | 0x80; wall bit 1 and tile
/// type ≠ 0 overwrite; shadows at (tile x + x + i, tile y + y + j); units
/// strictly inside the match box move to (5x + ux − 5G.x, 5y + uy −
/// 5G.y).
#[test]
fn apply_writes_the_variant() {
    let mut file = pattern(2, 2, 1, 1);
    // Variant 0 at x offset 3.
    let mut w = CellGrid::new(6, 2);
    w.set(4, 1, 0x1 | 0x300);
    w.set(3, 1, 0x100);
    file.walls = vec![w];
    let mut tt = CellGrid::new(6, 2);
    tt.set(3, 0, 7);
    file.tile_types = vec![tt];
    let mut sh = CellGrid::new(6, 2);
    sh.set(4, 0, 0x800_0000 | 9);
    sh.set(1, 1, 0x800_0000);
    file.shadow = Some(sh);
    file.units = vec![
        PresetUnit {
            unit_type: 2,
            class: 1,
            x: 4,
            y: 6,
        },
        PresetUnit {
            unit_type: 2,
            class: 2,
            x: 0,
            y: 6,
        },
        PresetUnit {
            unit_type: 2,
            class: 3,
            x: 12,
            y: 3,
        },
    ];
    let g = file.groups[0];
    let mut room = room_side();
    apply(
        &mut Seed::default(),
        &mut rsub(&mut room),
        &file,
        g,
        2,
        5,
        3,
    )
    .unwrap();
    assert_eq!(room.roof_count, 1);
    assert_eq!(room.floor.get(2, 5), 0x12 | 0x80);
    assert_eq!(room.floor.get(3, 6), 0x12 | 0x80);
    assert_eq!(room.floor.get(4, 5), 0x2);
    assert_eq!(room.wall.get(3, 6), 0x1 | 0x300);
    assert_eq!(room.wall.get(2, 6), 0);
    assert_eq!(room.tile_type.get(2, 5), 7);
    assert_eq!(room.shadows, [(803, 905, 0x800_0000 | 9)]);
    assert_eq!(room.units.len(), 1);
    assert_eq!(
        (room.units[0].class, room.units[0].x, room.units[0].y),
        (1, 14, 31)
    );
}

// ---- tilesub.rs: border substitution (Acts I/II/IV callbacks) ---------------

use super::tilesub::{style_map, BorderCtx, SKIP_STYLE, STYLE_ANY};

/// A wall pattern value of style s (`(s + 1) << 8 | 1`).
fn wall_style(s: i32) -> u32 {
    ((s + 1) as u32) << 8 | 1
}

/// A border file: group (0, 0, w, h, N) whose match cells are `cells`
/// (wall styles, or `None` for a floor-bit-2 cell, or `Some(-1)` for an
/// empty cell); variant v's cells are `variant(v, i, j)`.
fn border_file(
    w: i32,
    h: i32,
    n: i32,
    cells: &[Option<i32>],
    variant: impl Fn(i32, i32, i32) -> Option<i32>,
) -> SubFile {
    let width = ((n + 1) * (w + 1)) as usize;
    let mut floor = CellGrid::new(width, h as usize);
    let mut wall = CellGrid::new(width, h as usize);
    let mut put = |x: i32, y: i32, c: Option<i32>| match c {
        Some(-1) => {}
        Some(s) => wall.set(x as usize, y as usize, wall_style(s)),
        None => floor.set(x as usize, y as usize, 0x2),
    };
    for j in 0..h {
        for i in 0..w {
            put(i, j, cells[(j * w + i) as usize]);
            for v in 0..n {
                put((v + 1) * (w + 1) + i, j, variant(v, i, j));
            }
        }
    }
    SubFile {
        method: 2,
        groups: vec![SubGroup {
            x: 0,
            y: 0,
            w,
            h,
            variants: n,
        }],
        floor: Some(floor),
        walls: vec![wall],
        ..SubFile::default()
    }
}

/// The border-substitution rules (Wild callbacks), applied to copies of grids 0 and 2
/// with the level seed; stamps use 1 × 1 presets, file 0, no build-list roll.
#[allow(clippy::too_many_arguments)]
fn border_model(
    seed: &mut Seed,
    g0: &mut Grid,
    g2: &mut Grid,
    id: u32,
    flags: u32,
    ctx: BorderCtx,
    rows: &[(SubRow, SubFile)],
) {
    let (gw, gh) = (g0.w, g0.h);
    for (row, file) in rows {
        let skip = SKIP_STYLE;
        let count = file.groups.len() as i32;
        let first = if row.bord_type == 0 {
            seed.roll(count) as i32
        } else {
            0
        };
        'groups: for j in 0..count {
            let g = file.groups[((first + j) % count) as usize];
            let off = if ctx.t == 1 && flags & 0xC != 0 {
                -1
            } else {
                1
            };
            let w = gw - row.grid_size * g.w + off;
            let h = gh - row.grid_size * g.h + 1;
            if w * h <= 0 {
                continue;
            }
            let small = ctx.t == 1 && (2..=7).contains(&id) && w < 6 && h < 6;
            for (x, y) in shuffle_cells(seed, w, h) {
                if small && (x, y) == (2, 2) {
                    continue;
                }
                let gs = row.grid_size;
                let (sx, sy) = (x - x % gs, y - y % gs);
                let cell_at = |i: i32, jj: i32| (sx + i * gs, sy + jj * gs);
                let mut pass = true;
                for jj in 0..g.h {
                    for i in 0..g.w {
                        let (cx, cy) = cell_at(i, jj);
                        let f = file.floor_at(g.x + i, g.y + jj);
                        let wv = file.wall_at(0, g.x + i, g.y + jj);
                        let c = g0.get(cx, cy) as i32;
                        let not_link = g2.get(cx, cy) & cell::LINK == 0;
                        let ok = if wv & 1 != 0 {
                            let s = (wv >> 8 & 0xFF) as i32 - 1;
                            (s == skip || c == ctx.base as i32 + s) && not_link
                        } else if f & 2 != 0 {
                            g2.contains(cx, cy) && g2.get(cx, cy) & cell::NOT_SPAWN == 0
                        } else {
                            true
                        };
                        pass &= ok;
                    }
                }
                if !pass {
                    continue;
                }
                let v = seed.roll(g.variants) as i32;
                let xoff = (v + 1) * (g.w + 1);
                for jj in 0..g.h {
                    for i in 0..g.w {
                        let (cx, cy) = cell_at(i, jj);
                        let f = file.floor_at(g.x + i + xoff, g.y + jj);
                        let wv = file.wall_at(0, g.x + i + xoff, g.y + jj);
                        if wv & 1 != 0 {
                            let s = (wv >> 8 & 0xFF) as i32 - 1;
                            let p = (ctx.base as i32 + s) as u32;
                            // No build-list roll; file 0.
                            if s != skip {
                                let border = matches!(p, 4..=15 | 364..=375);
                                g2.op(cx, cy, Op::AndNot, cell::FILE_MASK);
                                g2.op(
                                    cx,
                                    cy,
                                    Op::Or,
                                    cell::PRESET | if border { cell::BORDER } else { 0 },
                                );
                                g0.op(cx, cy, Op::Set, p);
                            }
                        } else if f & 2 != 0 {
                            g0.op(cx, cy, Op::Set, 0);
                            g2.op(cx, cy, Op::Set, 0);
                        } else {
                            g0.op(cx, cy, Op::Set, 0);
                            g2.op(cx, cy, Op::Set, cell::BLANK);
                        }
                    }
                }
                match row.bord_type {
                    0 => break 'groups,
                    1 => continue 'groups,
                    _ => {}
                }
            }
        }
    }
}

/// Border substitution against the rule
/// model, over seeds, BordType 0/1/2, GridSize 1/2, type 1 with outdoor
/// flags 0xC (Off −1) and the small-area skip in levels 2..7, the skip
/// style S = 62, keep and blank cells, links.
#[test]
fn border_substitution_by_the_rules() {
    struct Case {
        id: u32,
        t: i32,
        flags: u32,
        bord: i32,
        gs: i32,
        gw: i32,
        gh: i32,
    }
    let cases = [
        Case {
            id: 10,
            t: 2,
            flags: 0,
            bord: 2,
            gs: 1,
            gw: 7,
            gh: 6,
        },
        Case {
            id: 10,
            t: 3,
            flags: 0,
            bord: 1,
            gs: 2,
            gw: 9,
            gh: 8,
        },
        Case {
            id: 3,
            t: 1,
            flags: 0xC,
            bord: 0,
            gs: 1,
            gw: 6,
            gh: 6,
        },
        Case {
            id: 3,
            t: 1,
            flags: 0,
            bord: 2,
            gs: 1,
            gw: 6,
            gh: 5,
        },
        Case {
            id: 30,
            t: 1,
            flags: 0x4,
            bord: 2,
            gs: 1,
            gw: 6,
            gh: 6,
        },
    ];
    let base = 4;
    let mut stamped = [0usize; 5];
    for (ci, c) in cases.iter().enumerate() {
        // Two-cell group: a style-3 wall then a floor (fit) cell; variant
        // 0: style 5 wall, empty; variant 1: style 61 (= S, no stamp),
        // style 6 wall. A second file with one 1 × 1 cell of style S
        // (matches anything) replaced by a floor (keep) cell.
        let f1 = border_file(2, 1, 2, &[Some(3), None], |v, i, _| match (v, i) {
            (0, 0) => Some(5),
            (0, _) => Some(-1),
            (1, 0) => Some(SKIP_STYLE),
            _ => Some(6),
        });
        let f2 = border_file(1, 1, 1, &[Some(SKIP_STYLE)], |_, _, _| None);
        // A 2 × 2 group (row 0 style S, row 1 style 3) replaced by styles
        // 7 + 4v + i + 2j, and a second 1 × 1 group whose replacement column is empty
        // (blank cell).
        let s = Some(SKIP_STYLE);
        let mut f3 = border_file(2, 2, 2, &[s, s, Some(3), Some(3)], |v, i, j| {
            Some(7 + 4 * v + i + 2 * j)
        });
        f3.groups.push(SubGroup {
            x: 0,
            y: 0,
            w: 1,
            h: 1,
            variants: 1,
        });
        let row = |file: &str| SubRow {
            type_: c.t,
            file: file.as_bytes().to_vec(),
            bord_type: c.bord,
            grid_size: c.gs,
            ..SubRow::default()
        };
        for k in 0..6u32 {
            let mut e = Env::new(c.id, c.gw, c.gh);
            e.od.subs = vec![row("f1"), row("f2"), row("f3")];
            e.subs.0.insert(b"f1".to_vec(), f1.clone());
            e.subs.0.insert(b"f2".to_vec(), f2.clone());
            e.subs.0.insert(b"f3".to_vec(), f3.clone());
            e.info.flags = c.flags;
            e.drlg.level_mut(e.l).seed = Seed::init_low(31 + 7919 * (k + 8 * ci as u32));
            // Grid 0: style-3 borders (base + 3) on a diagonal pattern;
            // a link cell and a not-spawn cell.
            for y in 0..c.gh {
                for x in 0..c.gw {
                    if (x + 2 * y + k as i32) % 3 == 0 {
                        e.info.grids[0].op(x, y, Op::Set, base + 3);
                    }
                }
            }
            e.info.grids[2].op(1, 1, Op::Set, cell::LINK);
            e.info.grids[2].op(4, 2, Op::Set, cell::LINK);
            e.info.grids[2].op(2, 3, Op::Set, cell::BLANK);
            let mut s = e.seed();
            let (mut m0, mut m2) = (e.info.grids[0].clone(), e.info.grids[2].clone());
            let before = m0.clone();
            let rows = [
                (row("f1"), f1.clone()),
                (row("f2"), f2.clone()),
                (row("f3"), f3.clone()),
            ];
            let ctx = BorderCtx::wild(c.t, base);
            border_model(&mut s, &mut m0, &mut m2, c.id, c.flags, ctx, &rows);
            let mut g = e.gen();
            g.border_sub(ctx).unwrap();
            let (r0, r2) = (&g.info.grids[0], &g.info.grids[2]);
            assert_eq!(r0.cells, m0.cells, "case {ci} k {k}: grid 0");
            // Grid 2 without the file bits (build-list files).
            let strip = |g: &Grid| {
                g.cells
                    .iter()
                    .map(|v| v & !cell::FILE_MASK)
                    .collect::<Vec<_>>()
            };
            assert_eq!(strip(r2), strip(&m2), "case {ci} k {k}: grid 2");
            assert_eq!(*g.seed(), s, "case {ci} k {k}: seed");
            let changed = before
                .cells
                .iter()
                .zip(&m0.cells)
                .filter(|(a, b)| a != b)
                .count();
            stamped[ci] += changed;
        }
    }
    // Every case replaced something.
    assert!(stamped.iter().all(|&n| n > 0), "{stamped:?}");
}

/// The Wild and Barricade contexts start with
/// skip style S = −1 (then 62); the Act V context is type 12, base 0.
#[test]
fn border_contexts() {
    let w = BorderCtx::wild(3, 364);
    assert_eq!((w.t, w.base, w.skip), (3, 364, -1));
    let b = BorderCtx::barricade();
    assert_eq!((b.t, b.base, b.skip), (12, 0, -1));
    assert_eq!(STYLE_ANY, -5);
}

/// The Act V style map gives P + v − lo (P
/// snow for level 117); P ≤ 0 rows return P itself.
#[test]
fn style_map_rows() {
    assert_eq!(style_map(49, 1, false), Ok(915));
    assert_eq!(style_map(49, 16, false), Ok(930));
    assert_eq!(style_map(49, 31, true), Ok(987));
    assert_eq!(style_map(48, 3, false), Ok(882));
    assert_eq!(style_map(48, 7, true), Ok(970));
    assert_eq!(style_map(48, 30, false), Ok(0));
    assert_eq!(style_map(48, 31, false), Ok(-5));
    assert!(style_map(48, 9, false).is_err());
    assert!(style_map(47, 1, false).is_err());
}

/// A 7 × 4 file whose one group is (1, 1, 2, 2) with N = 1 (variant 0 at
/// x offset 3: pattern columns 4..5).
fn offset_file() -> SubFile {
    SubFile {
        method: 2,
        groups: vec![SubGroup {
            x: 1,
            y: 1,
            w: 2,
            h: 2,
            variants: 1,
        }],
        floor: Some(CellGrid::new(7, 4)),
        walls: vec![CellGrid::new(7, 4)],
        tile_types: vec![CellGrid::new(7, 4)],
        shadow: Some(CellGrid::new(7, 4)),
        ..SubFile::default()
    }
}

/// Fixed and random tests with the group away from the file origin:
/// group cell (i, j) reads pattern (G.x + i, G.y + j) and room (x + i,
/// y + j); only cells with pattern floor bit 2 or wall bit 1 are checked.
#[test]
fn tests_read_the_group_box() {
    // Fixed test: only group cell (1, 0) (pattern (2, 1)) is checked.
    let mut file = offset_file();
    file.floor.as_mut().unwrap().set(2, 1, 0x2);
    let g = file.groups[0];
    let mut room = room_side();
    room.floor.set(4, 3, 0);
    // (3, 3): cell (1, 0) is room (4, 3): fails; (4, 3) itself is cell
    // (0, 0) of candidate (4, 3): unchecked.
    assert!(!fixed_test(&rsub(&mut room), &file, g, 3, 3));
    assert!(fixed_test(&rsub(&mut room), &file, g, 4, 3));
    assert!(fixed_test(&rsub(&mut room), &file, g, 3, 2));
    // Wall bit 1 is checked the same way.
    let mut file = offset_file();
    file.walls[0].set(1, 2, 0x1);
    let mut room = room_side();
    room.wall.set(3, 5, 0x1);
    assert!(!fixed_test(&rsub(&mut room), &file, g, 3, 4));
    assert!(fixed_test(&rsub(&mut room), &file, g, 2, 4));
    // Random test: tile type of group cell (1, 1) (pattern (2, 2)).
    let mut file = offset_file();
    file.tile_types[0].set(2, 2, 5);
    let mut room = room_side();
    room.tile_type.set(4, 4, 5);
    assert!(random_test(&rsub(&mut room), &file, g, 3, 3));
    assert!(!random_test(&rsub(&mut room), &file, g, 3, 2));
    assert!(!random_test(&rsub(&mut room), &file, g, 2, 3));
    // Random test, floor and wall agreement at group cell (0, 1)
    // (pattern (1, 2)).
    let mut file = offset_file();
    file.floor.as_mut().unwrap().set(1, 2, 0x2 | 0x400);
    file.walls[0].set(1, 2, 0x1 | 0x800);
    let mut room = room_side();
    room.floor.set(3, 4, 0x2 | 0x400);
    room.wall.set(3, 4, 0x1 | 0x800);
    assert!(random_test(&rsub(&mut room), &file, g, 3, 3));
    assert!(!random_test(&rsub(&mut room), &file, g, 3, 2));
    assert!(!random_test(&rsub(&mut room), &file, g, 4, 3));
}

/// Apply with the group away from the file origin:
/// values come from pattern (G.x + i + o, G.y + j); roofs counted in the
/// group box at offset o; units strictly inside the match box (5G.x,
/// 5(G.x + G.w)) × (5G.y, 5(G.y + G.h)) move to (5x + ux − 5G.x, 5y +
/// uy − 5G.y).
#[test]
fn apply_reads_the_variant_box() {
    let mut file = offset_file();
    // Variant 0 cells: pattern columns 4..5, rows 1..2.
    let f = file.floor.as_mut().unwrap();
    f.set(4, 1, 0x2 | 0x10);
    f.set(5, 2, 0x2 | 0x20 | 0x80);
    f.set(2, 1, 0x2 | 0x40); // match box: not pasted
    file.walls[0].set(5, 1, 0x1 | 0x500);
    file.walls[0].set(4, 2, 0x100); // no bit 1
    file.tile_types[0].set(4, 2, 9);
    let sh = file.shadow.as_mut().unwrap();
    sh.set(5, 2, 0x800_0000 | 3);
    sh.set(4, 1, 0x800_0000);
    sh.set(6, 1, 0x800_0000); // outside the box
    sh.set(1, 1, 0x800_0000); // the match box: not counted at o = 3
    let unit = |class, x, y| PresetUnit {
        unit_type: 2,
        class,
        x,
        y,
    };
    file.units = vec![
        unit(1, 7, 12),
        unit(2, 5, 7),
        unit(3, 14, 14),
        unit(4, 15, 7),
        unit(5, 7, 15),
        unit(6, 7, 5),
    ];
    let g = file.groups[0];
    let mut room = room_side();
    apply(
        &mut Seed::default(),
        &mut rsub(&mut room),
        &file,
        g,
        3,
        4,
        3,
    )
    .unwrap();
    assert_eq!(room.roof_count, 2);
    assert_eq!(room.floor.get(3, 4), 0x12 | 0x80);
    assert_eq!(room.floor.get(4, 5), 0x22 | 0x80);
    assert_eq!(room.floor.get(4, 4), 0x2);
    assert_eq!(room.floor.get(3, 5), 0x2);
    assert_eq!(room.wall.get(4, 4), 0x1 | 0x500);
    assert_eq!(room.wall.get(3, 5), 0);
    assert_eq!(room.tile_type.get(3, 5), 9);
    assert_eq!(room.tile_type.get(3, 4), 0);
    let mut shadows = room.shadows.clone();
    shadows.sort();
    assert_eq!(
        shadows,
        [(803, 904, 0x800_0000), (804, 905, 0x800_0000 | 3)]
    );
    let units: Vec<_> = room.units.iter().map(|u| (u.class, u.x, u.y)).collect();
    // Prepended: reverse of the file list.
    assert_eq!(units, [(3, 24, 29), (1, 17, 27)]);
}

// ---- vertex.rs: the vertex polygon ------------------------------------------

/// The polygon rules on a plain vertex list (corner tags 0..3, inserted −1).
fn polygon_model(rect: TileRect, orth: &[Orth]) -> Vec<(i32, i32, u32)> {
    let (x, y, w, h) = (rect.x, rect.y, rect.w - 1, rect.h - 1);
    let corners = [(x, y + h), (x, y), (x + w, y), (x + w, y + h)];
    // (tag, x, y, flags)
    let mut vs: Vec<(i32, i32, i32, u32)> = corners
        .iter()
        .enumerate()
        .map(|(k, &(cx, cy))| (k as i32, cx, cy, 0))
        .collect();
    for e in orth {
        let b = TileRect::new(e.rect.x, e.rect.y, e.rect.w - 1, e.rect.h - 1);
        let d = e.direction as usize;
        let (c, nc) = (corners[d], corners[(d + 1) % 4]);
        let (on_x, s, p, q) = match d {
            0 => (false, -1, b.y + b.h, b.y),
            1 => (true, 1, b.x, b.x + b.w),
            2 => (false, 1, b.y, b.y + b.h),
            _ => (true, -1, b.x + b.w, b.x),
        };
        let (a, bb) = if on_x { (c.0, nc.0) } else { (c.1, nc.1) };
        let at = |t: i32| if on_x { (t, c.1) } else { (c.0, t) };
        let lf = 1 | if e.preset { 2 } else { 0 };
        let pos = vs.iter().position(|v| v.0 == d as i32).unwrap();
        if s * p > s * a {
            if s * p <= s * bb {
                let (ux, uy) = at(p);
                vs.insert(pos + 1, (-1, ux, uy, lf));
                if s * q < s * bb {
                    let (qx, qy) = at(q);
                    vs.insert(pos + 2, (-1, qx, qy, 0));
                }
            }
        } else if s * q >= s * a {
            vs[pos].3 |= lf;
            if s * q < s * bb {
                let (qx, qy) = at(q);
                vs.insert(pos + 1, (-1, qx, qy, 0));
            }
        }
    }
    vs.iter()
        .map(|v| (v.1 - rect.x, v.2 - rect.y, v.3))
        .collect()
}

/// The polygon against the rules, on deterministic
/// pseudo-random level rects and neighbour boxes (every direction, inside,
/// overlapping and outside the edge, preset or not).
#[test]
fn vertex_polygon_by_the_rules() {
    use super::vertex::build_polygon;
    let mut seed = Seed::init_low(2718);
    let mut r = |n: i32| seed.roll(n) as i32;
    for case in 0..400 {
        let rect = TileRect::new(
            800 + 8 * r(10),
            600 + 8 * r(10),
            8 * (4 + r(12)),
            8 * (4 + r(12)),
        );
        let n = 1 + r(4);
        let orth: Vec<Orth> = (0..n)
            .map(|_| {
                let d = r(4);
                let span = |base: i32, len: i32, r: &mut dyn FnMut(i32) -> i32| {
                    base - 40 + 8 * r((len + 80) / 8)
                };
                let (bw, bh) = (8 * (1 + r(8)), 8 * (1 + r(8)));
                let bx = span(rect.x, rect.w, &mut r);
                let by = span(rect.y, rect.h, &mut r);
                Orth {
                    level_id: 1,
                    direction: d,
                    init: false,
                    rect: TileRect::new(bx, by, bw, bh),
                    preset: r(2) == 1,
                }
            })
            .collect();
        let got: Vec<(i32, i32, u32)> = build_polygon(rect, &orth)
            .unwrap()
            .iter()
            .map(|v| (v.x, v.y, v.flags))
            .collect();
        assert_eq!(got, polygon_model(rect, &orth), "case {case}");
    }
}

/// Consecutive equal cell vertices merge, the
/// earlier keeping its place, OR-ing the later's flags and taking its
/// direction.
#[test]
fn to_cells_merges_into_the_earlier_vertex() {
    use super::vertex::to_cells;
    let v = |x, y, direction, flags| Vertex {
        x,
        y,
        direction,
        flags,
    };
    let mut vs = vec![
        v(0, 0, 0, 0),
        v(16, 8, 0, 1),
        v(17, 15, 3, 2),
        v(40, 8, 1, 0),
    ];
    to_cells(&mut vs);
    assert_eq!(vs, [v(0, 0, 0, 0), v(2, 1, 3, 3), v(5, 1, 1, 0)]);
}

/// Border table N (index: value, −1 elsewhere).
fn spec_n(i: i32) -> i32 {
    const N: [(i32, i32); 37] = [
        (1, 1),
        (3, 0),
        (5, 2),
        (7, 3),
        (9, 0),
        (10, 1),
        (11, 9),
        (12, 9),
        (15, 1),
        (16, 8),
        (19, 12),
        (24, 12),
        (25, 4),
        (28, 5),
        (29, 2),
        (30, 2),
        (31, 10),
        (36, 10),
        (37, 1),
        (38, 9),
        (39, 9),
        (61, 11),
        (62, 11),
        (63, 3),
        (64, 12),
        (69, 12),
        (70, 4),
        (71, 4),
        (72, 7),
        (75, 2),
        (76, 10),
        (81, 10),
        (84, 6),
        (85, 3),
        (88, 11),
        (89, 11),
        (90, 3),
    ];
    N.iter().find(|e| e.0 == i).map_or(-1, |e| e.1)
}

/// Border table P rows 1..12.
fn spec_p(k: i32, s: i32) -> u32 {
    const P: [[u32; 4]; 12] = [
        [0, 4, 364, 799],
        [16, 5, 365, 800],
        [17, 6, 366, 801],
        [0, 7, 367, 802],
        [18, 8, 368, 803],
        [19, 9, 369, 804],
        [22, 10, 370, 805],
        [0, 11, 371, 806],
        [0, 12, 372, 807],
        [23, 13, 373, 808],
        [0, 14, 374, 809],
        [0, 15, 375, 810],
    ];
    P[(k - 1) as usize][s as usize]
}

/// Border lookups: Corner(a, b, c, e, s): a and c grow by 2 in
/// magnitude; k := N[b + a + 9(e + c) + 50]; −1 → 0; s < 4: P[k][s], else
/// Q[k − 1][s − 4] = (881, 957)[s − 4] + k − 1. Border(dx, dy, s): k :=
/// N[dx + 3dy + 4]; s < 4: P[k + 1][s], else Q[k][s − 4].
#[test]
fn corner_and_border_pieces_by_the_tables() {
    use super::vertex::{border_piece, corner_piece};
    let grow = |v: i32| v + 2 * v.signum();
    let mut checked = 0;
    for a in -1..=1 {
        for b in -1..=1 {
            for c in -1..=1 {
                for e in -1..=1 {
                    let k = spec_n(b + grow(a) + 9 * (e + grow(c)) + 50);
                    for s in 0..6 {
                        let want = if k == -1 {
                            0
                        } else if s < 4 {
                            spec_p(k, s)
                        } else if k >= 1 {
                            [881, 957][(s - 4) as usize] + (k - 1) as u32
                        } else {
                            continue;
                        };
                        assert_eq!(
                            corner_piece(a, b, c, e, s),
                            want,
                            "({a}, {b}, {c}, {e}, {s})"
                        );
                        checked += 1;
                    }
                }
            }
        }
    }
    assert!(checked > 100);
    for dx in -1..=1 {
        for dy in -1..=1 {
            let k = spec_n(dx + 3 * dy + 4);
            if k < 0 {
                continue;
            }
            for s in 0..6 {
                let want = if s < 4 {
                    spec_p(k + 1, s)
                } else {
                    [881, 957][(s - 4) as usize] + k as u32
                };
                assert_eq!(border_piece(dx, dy, s), want, "({dx}, {dy}, {s})");
            }
        }
    }
}

/// Link vis flag: side s from the vertex position, probe
/// point (level.x + 8vx + dx[s], level.y + 8vy + dy[s]) with (dx, dy) =
/// (−4, 4), (4, −4), (12, 4), (4, 12); the first entry of direction s
/// whose box contains it: init 0 → 1 << (j + 4) for the vis slot j
/// holding it, else 0.
#[test]
fn link_vis_side_and_probe() {
    let (gw, gh) = (6, 5);
    let d = [(-4, 4), (4, -4), (12, 4), (4, 12)];
    let cases = [
        ((0, 0), 1),
        ((0, 3), 0),
        ((5, 0), 2),
        ((2, 0), 1),
        ((5, 4), 3),
        ((5, 2), 2),
        ((2, 4), 3),
    ];
    for ((vx, vy), s) in cases {
        for (dx_off, init, want_hit) in [(0, false, true), (1, false, false), (0, true, false)] {
            let mut e = Env::new(2, gw, gh);
            e.data.levels[2].vis = [0, 8, 9, 0, 0, 0, 0, 0];
            let lr = e.drlg.level(e.l).rect;
            let (px, py) = (lr.x + 8 * vx + d[s].0, lr.y + 8 * vy + d[s].1);
            e.info.orth = vec![
                // A box of another direction holding the probe comes first.
                Orth {
                    level_id: 8,
                    direction: (s as i32 + 1) % 4,
                    init: false,
                    rect: TileRect::new(px, py, 1, 1),
                    preset: false,
                },
                Orth {
                    level_id: 9,
                    direction: s as i32,
                    init,
                    rect: TileRect::new(px + dx_off, py, 1, 1),
                    preset: false,
                },
            ];
            let g = e.gen();
            let v = Vertex {
                x: vx,
                y: vy,
                direction: 0,
                flags: VERTEX_LINK,
            };
            let want = if want_hit { 1 << (2 + 4) } else { 0 };
            assert_eq!(
                g.link_vis(v),
                Ok(want),
                "({vx}, {vy}) off {dx_off} init {init}"
            );
        }
    }
    // An interior vertex has no side.
    let e = &mut Env::new(2, gw, gh);
    let g = e.gen();
    let v = Vertex {
        x: 2,
        y: 2,
        direction: 0,
        flags: VERTEX_LINK,
    };
    assert_eq!(g.link_vis(v), Ok(0));
}

/// Each link vertex marks the polygon edge to the next
/// vertex, both ends included: grid 1 |= its link vis flag, grid 2 |=
/// 0x1 (| 0x2 if its direction ≠ 0).
#[test]
fn link_flags_mark_the_edge() {
    let mut e = Env::new(2, 6, 5);
    e.data.levels[2].vis = [9, 0, 0, 0, 0, 0, 0, 0];
    let lr = e.drlg.level(e.l).rect;
    // The east edge (5, 0) → (5, 4): side 2 at (5, 0), probe (12, 4).
    e.info.orth = vec![Orth {
        level_id: 9,
        direction: 2,
        init: false,
        rect: TileRect::new(lr.x + 8 * 5 + 12, lr.y + 4, 1, 1),
        preset: false,
    }];
    let v = |x, y, direction, flags| Vertex {
        x,
        y,
        direction,
        flags,
    };
    e.info.vertices = vec![
        v(0, 4, 0, 0),
        v(0, 0, 0, 0),
        v(5, 0, 1, VERTEX_LINK),
        v(5, 4, 0, 0),
    ];
    let mut g = e.gen();
    g.link_flags().unwrap();
    for y in 0..5 {
        for x in 0..6 {
            let on = x == 5;
            assert_eq!(g.g(1, x, y), if on { 1 << 4 } else { 0 }, "({x}, {y})");
            assert_eq!(g.g(2, x, y), if on { 0x3 } else { 0 }, "({x}, {y})");
        }
    }
    // The walk from (5, 4) back to (0, 4) (leftward, the last edge).
    let mut e = Env::new(2, 6, 5);
    e.info.vertices = vec![
        v(0, 4, 0, 0),
        v(0, 0, 0, 0),
        v(5, 0, 0, 0),
        v(5, 4, 0, VERTEX_LINK),
    ];
    let mut g = e.gen();
    g.link_flags().unwrap();
    for x in 0..6 {
        assert_eq!(g.g(2, x, 4), 0x1, "({x}, 4)");
        assert_eq!(g.g(2, x, 3), 0, "({x}, 3)");
    }
    // Upward: (0, 4) → (0, 0).
    let mut e = Env::new(2, 6, 5);
    e.info.vertices = vec![
        v(0, 4, 0, VERTEX_LINK),
        v(0, 0, 0, 0),
        v(5, 0, 0, 0),
        v(5, 4, 0, 0),
    ];
    let mut g = e.gen();
    g.link_flags().unwrap();
    for y in 0..5 {
        assert_eq!(g.g(2, 0, y), 0x1, "(0, {y})");
        assert_eq!(g.g(2, 1, y), 0, "(1, {y})");
    }
}

// ---- vertex.rs: borders ------------------------------------------------------

/// Border table Q (rows 0..11 × [barricade, snow]).
fn spec_q(k: i32, col: i32) -> u32 {
    [881, 957][col as usize] + k as u32
}

fn spec_border(dx: i32, dy: i32, s: i32) -> u32 {
    let k = spec_n(dx + 3 * dy + 4);
    if s < 4 {
        spec_p(k + 1, s)
    } else {
        spec_q(k, s - 4)
    }
}

fn spec_corner(a: i32, b: i32, c: i32, e: i32, s: i32) -> u32 {
    let grow = |v: i32| v + 2 * v.signum();
    let k = spec_n(b + grow(a) + 9 * (e + grow(c)) + 50);
    if k == -1 {
        0
    } else if s < 4 {
        spec_p(k, s)
    } else {
        spec_q(k - 1, s - 4)
    }
}

fn spec_style(lt: u32, id: u32, d: u8) -> i32 {
    match lt {
        2 => i32::from(d == 0),
        16 => 2,
        27 => 3,
        31 => 4 + i32::from(id == 117),
        _ => -1,
    }
}

/// The border rules on copies of grids 0 and 2 (stamps of 1 × 1 presets;
/// grid-2 file bits are not modelled).
#[allow(clippy::too_many_arguments)]
fn borders_model(
    seed: &mut Seed,
    g0: &mut Grid,
    g2: &mut Grid,
    vs: &[Vertex],
    lt: u32,
    id: u32,
    act: u8,
) {
    // Stamps with F = −1: the build list draws roll(Files = 1) on a
    // preset's first use; the file is then (r + 1) mod 1 = 0.
    let mut built: Vec<u32> = Vec::new();
    let mut stamp = |g0: &mut Grid, g2: &mut Grid, x: i32, y: i32, p: u32| {
        if !built.contains(&p) {
            seed.roll(1);
            built.push(p);
        }
        g2.op(x, y, Op::AndNot, cell::FILE_MASK);
        g2.op(x, y, Op::Or, cell::PRESET);
        g0.op(x, y, Op::Set, p);
    };
    let n_of = |i: usize| vs[(i + 1) % vs.len()];
    let sgn = |a: &Vertex, b: &Vertex| ((b.x - a.x).signum(), (b.y - a.y).signum());
    for (i, &v) in vs.iter().enumerate() {
        let (n, nn) = (n_of(i), n_of(i + 1));
        let ((dx, dy), (ndx, ndy)) = (sgn(&v, &n), sgn(&n, &nn));
        let straight = spec_border(dx, dy, spec_style(lt, id, v.direction));
        let bits = 0x1 | if v.direction != 0 { 0x2 } else { 0 };
        let preset = |x: &Vertex| x.flags & VERTEX_PRESET_LINK != 0;
        if !preset(&v) {
            let (mut x, mut y) = (v.x, v.y);
            while (x, y) != (n.x, n.y) {
                x += dx;
                y += dy;
                if straight != 0 {
                    stamp(g0, g2, x, y, straight);
                }
                g2.op(x, y, Op::Or, bits);
            }
        }
        if v.flags & VERTEX_LINK != 0 && !preset(&v) {
            let l = (n.x - v.x).abs() + (n.y - v.y).abs();
            let (mx, my) = (
                v.x.min(n.x) + dx.abs() * l / 2,
                v.y.min(n.y) + dy.abs() * l / 2,
            );
            match act {
                0 | 3 | 4 => {
                    let f = if act != 3 && id == 17 {
                        0x40400
                    } else {
                        0x30400
                    };
                    g2.op(mx, my, Op::AndNot, cell::FILE_MASK);
                    g2.op(mx, my, Op::Or, f);
                }
                1 => {
                    let pairs = [(373, 372), (372, 375), (0, 0), (373, 374), (374, 375)];
                    let (a, b) = pairs[(dx + 2 * dy + 2) as usize];
                    if a != 0 {
                        stamp(g0, g2, mx, my, a);
                    }
                    if b != 0 {
                        stamp(g0, g2, mx + dx, my + dy, b);
                    }
                }
                _ => {}
            }
        }
        let d = if v.direction != 0 {
            v.direction
        } else {
            n.direction
        };
        let k2 = |x: i32, keep: bool| if keep { x } else { 2 * x };
        let mut piece = spec_corner(
            k2(dx, preset(&v)),
            k2(dy, preset(&v)),
            k2(ndx, preset(&n)),
            k2(ndy, preset(&n)),
            spec_style(lt, id, d),
        );
        if piece == 19 {
            if v.direction == 1 && n.direction != 1 {
                piece = 20;
            } else if v.direction != 1 {
                piece = 21;
            }
        }
        if piece != 0 {
            stamp(g0, g2, n.x, n.y, piece);
            g2.op(n.x, n.y, Op::Or, 0x1 | if d != 0 { 0x2 } else { 0 });
        }
    }
    // Blank corners.
    let (gw, gh) = (g2.w, g2.h);
    for (cx, cy, sx, sy) in [
        (0, 0, 1, 1),
        (gw - 1, 0, -1, 1),
        (0, gh - 1, 1, -1),
        (gw - 1, gh - 1, -1, -1),
    ] {
        let mut y = cy;
        while g2.contains(cx, y) && g2.get(cx, y) & 1 == 0 {
            let mut x = cx;
            while g2.contains(x, y) && g2.get(x, y) & 1 == 0 {
                g2.op(x, y, Op::Or, cell::BLANK);
                x += sx;
            }
            y += sy;
        }
        let mut x = cx;
        while g2.contains(x, cy) && g2.get(x, cy) & 1 == 0 {
            let mut y = cy;
            while g2.contains(x, y) && g2.get(x, y) & 1 == 0 {
                g2.op(x, y, Op::Or, cell::BLANK);
                y += sy;
            }
            x += sx;
        }
    }
}

/// Borders against the rule model: straight pieces along the
/// edges, link midpoints by act (file 3 / level 17 file 4 / the desert
/// pairs / nothing), corner pieces with doubled directions unless preset
/// links, the cliff 19 → 20 / 21 swap, and the blank corners.
#[test]
fn borders_by_the_rules() {
    let v = |x, y, direction, flags| Vertex {
        x,
        y,
        direction,
        flags,
    };
    let link = VERTEX_LINK;
    let plink = VERTEX_LINK | VERTEX_PRESET_LINK;
    // (polygon, inset) shapes on a 10 × 8 grid.
    let shapes: [Vec<Vertex>; 3] = [
        vec![
            v(0, 7, 0, 0),
            v(0, 4, 1, link),
            v(0, 0, 0, 0),
            v(4, 0, 0, link),
            v(6, 0, 0, plink),
            v(9, 0, 1, 0),
            v(9, 7, 0, 0),
            v(5, 7, 0, link),
        ],
        vec![
            v(1, 6, 1, 0),
            v(1, 1, 1, link),
            v(8, 1, 0, 0),
            v(8, 6, 0, link),
        ],
        vec![
            v(0, 7, 1, plink),
            v(0, 0, 0, link),
            v(9, 0, 1, link),
            v(9, 3, 0, plink),
            v(9, 7, 1, 0),
        ],
    ];
    // (level id, level type, act).
    let levels = [
        (2, 2, 0),
        (17, 2, 0),
        (41, 16, 1),
        (104, 27, 3),
        (112, 31, 4),
        (117, 31, 4),
        (79, 2, 2),
    ];
    for (si, shape) in shapes.iter().enumerate() {
        for &(id, lt, act) in &levels {
            let mut e = Env::new(id, 10, 8);
            e.drlg.level_mut(e.l).level_type = lt;
            e.info.vertices = shape.clone();
            let (mut m0, mut m2) = (e.info.grids[0].clone(), e.info.grids[2].clone());
            let mut s = e.seed();
            borders_model(&mut s, &mut m0, &mut m2, shape, lt, id, act);
            let mut g = e.gen();
            g.borders().unwrap();
            assert_eq!(
                g.info.grids[2].cells, m2.cells,
                "shape {si} level {id}: grid 2 (files)"
            );
            assert_eq!(*g.seed(), s, "shape {si} level {id}: seed");
            assert_eq!(
                g.info.grids[0].cells, m0.cells,
                "shape {si} level {id}: grid 0"
            );
            let strip = |g: &Grid| {
                g.cells
                    .iter()
                    .map(|v| v & !cell::FILE_MASK)
                    .collect::<Vec<_>>()
            };
            // Midpoint file bits are part of the rule: compare them where
            // no stamp wrote a file.
            assert_eq!(
                strip(&g.info.grids[2]),
                strip(&m2),
                "shape {si} level {id}: grid 2"
            );
            for (k, (&a, &b)) in g.info.grids[2].cells.iter().zip(&m2.cells).enumerate() {
                if b & cell::PRESET == 0 {
                    assert_eq!(a, b, "shape {si} level {id}: grid 2 cell {k}");
                }
            }
        }
    }
}

// ---- wild.rs: Act I ----------------------------------------------------------

/// Dir(p, q) from the rules: d := q − p; if |dx|
/// ≥ 2|dy|: dy := −1 if dy < 0 else dy & 1; else if |dy| ≥ 2|dx|: dx :=
/// −1 if dx < 0 else dx & 1; clamp to −2..2; T[5dx + dy + 12].
#[test]
fn dir_by_the_rule() {
    use super::wild::dir;
    const T: [i32; 25] = [
        5, 4, 4, 4, 3, 6, 5, 4, 3, 2, 6, 6, 6, 2, 2, 6, 7, 0, 1, 2, 7, 0, 0, 0, 1,
    ];
    for dx0 in -7i32..=7 {
        for dy0 in -7i32..=7 {
            let (mut dx, mut dy) = (dx0, dy0);
            if dx.abs() >= 2 * dy.abs() {
                dy = if dy < 0 { -1 } else { dy & 1 };
            } else if dy.abs() >= 2 * dx.abs() {
                dx = if dx < 0 { -1 } else { dx & 1 };
            }
            let (dx, dy) = (dx.clamp(-2, 2), dy.clamp(-2, 2));
            let want = T[(5 * dx + dy + 12) as usize];
            assert_eq!(dir((3, 4), (3 + dx0, 4 + dy0)), want, "d ({dx0}, {dy0})");
        }
    }
}

/// Order rows and steps, and river files.
#[test]
fn act1_tables() {
    use super::wild::{ORDER, RIVER_FILES, STEP_X, STEP_Y};
    assert_eq!(
        ORDER,
        [[0, 1, 2, 3], [0, 1, 1, 1], [3, 2, 1, 2], [0, 3, 2, 1]]
    );
    assert_eq!((STEP_X, STEP_Y), ([1, 0, -1, 0], [0, 1, 0, -1]));
    assert_eq!(
        RIVER_FILES,
        [
            (2, 2),
            (0, 3),
            (1, 1),
            (3, 0),
            (0, 2),
            (0, 1),
            (1, 0),
            (2, 0),
            (2, 3),
            (1, 3),
            (3, 1),
            (3, 2)
        ]
    );
}

/// Grid path, from the rules (the model's choices where the rules leave a point open:
/// the root counts toward the 900 nodes; a climb to the root advances it, and its tries
/// reaching 3 fail the round). Returns cells B … A.
fn grid_path_model(
    a: (i32, i32),
    b: (i32, i32),
    gw: i32,
    gh: i32,
    blocked: &[(i32, i32)],
) -> Option<Vec<(i32, i32)>> {
    use super::wild::dir;
    const ROWS: [[i32; 4]; 4] = [[0, 1, 2, 3], [0, 1, 1, 1], [3, 2, 1, 2], [0, 3, 2, 1]];
    const X: [i32; 4] = [1, 0, -1, 0];
    const Y: [i32; 4] = [0, 1, 0, -1];
    let h = |p: (i32, i32)| {
        let (dx, dy) = ((p.0 - b.0).abs(), (p.1 - b.1).abs());
        dx.min(dy) + 2 * dx.max(dy)
    };
    if (a.0 - b.0).abs() + (a.1 - b.1).abs() < 2 {
        return Some(vec![a, b]);
    }
    // (cell, g, tries, row, pos, facing, parent, child)
    type N = (
        (i32, i32),
        i32,
        i32,
        usize,
        usize,
        i32,
        Option<usize>,
        Option<usize>,
    );
    let start = h(a) + h(a) / 2;
    let mut budget = start;
    loop {
        let mut nodes: Vec<N> = vec![(a, 0, -1, 0, 0, (dir(a, b) / 2) & 3, None, None)];
        let mut cur = 0;
        let mut failed = false;
        while nodes[cur].0 != b {
            let (cell, g, _, _, _, facing, _, _) = nodes[cur];
            let c = (cell.0 + X[facing as usize], cell.1 + Y[facing as usize]);
            let mut chain = false;
            let mut k = Some(cur);
            while let Some(i) = k {
                chain |= nodes[i].0 == c;
                k = nodes[i].6;
            }
            let inside = c.0 >= 0 && c.1 >= 0 && c.0 < gw && c.1 < gh;
            if (c == b || (inside && !blocked.contains(&c) && !chain)) && g + 2 + h(c) <= budget {
                let d = dir(c, b) / 2;
                let row = ((facing - d) & 3) as usize;
                let child: N = (c, g + 2, 0, row, 0, (d + ROWS[row][0]) & 3, Some(cur), None);
                cur = match nodes[cur].7 {
                    Some(id) => {
                        nodes[id] = child;
                        id
                    }
                    None => {
                        if nodes.len() >= 900 {
                            return None;
                        }
                        nodes.push(child);
                        let id = nodes.len() - 1;
                        nodes[cur].7 = Some(id);
                        id
                    }
                };
                continue;
            }
            loop {
                let n = &mut nodes[cur];
                if n.2 < 4 {
                    n.4 += 1;
                    if n.4 < 4 {
                        n.5 = (n.5 + ROWS[n.3][n.4]) & 3;
                    }
                }
                n.2 += 1;
                if n.2 < 3 {
                    break;
                }
                match n.6 {
                    Some(p) => cur = p,
                    None => {
                        failed = true;
                        break;
                    }
                }
            }
            if failed {
                break;
            }
        }
        if !failed {
            let mut out = Vec::new();
            let mut k = Some(cur);
            while let Some(i) = k {
                out.push(nodes[i].0);
                k = nodes[i].6;
            }
            return Some(out);
        }
        budget += 5;
        if budget >= start + 35 {
            return None;
        }
    }
}

/// Grid path against the rule model on pseudo-random grids
/// with blocked (0x200) cells: found paths, give-ups and the adjacent
/// shortcut.
#[test]
fn grid_path_by_the_rules() {
    use super::wild::grid_path;
    let mut seed = Seed::init_low(1618);
    let mut r = |n: i32| seed.roll(n) as i32;
    let (mut found, mut none) = (0, 0);
    for case in 0..300 {
        let (gw, gh) = (4 + r(10), 4 + r(10));
        let nb = r(gw * gh / 3 + 1);
        let blocked: Vec<(i32, i32)> = (0..nb).map(|_| (r(gw), r(gh))).collect();
        let a = (r(gw), r(gh));
        let b = (r(gw), r(gh));
        let want = grid_path_model(a, b, gw, gh, &blocked);
        let got = grid_path(a, b, gw, gh, |c| blocked.contains(&c));
        assert_eq!(got, want, "case {case}: {a:?} → {b:?} on {gw} × {gh}");
        if want.is_some() {
            found += 1;
        } else {
            none += 1;
        }
    }
    assert!(found > 50 && none > 5, "{found} found, {none} none");
    // A detour longer than the last round's budget (h + h/2 + 30): a wall
    // at x = 2 open only from y = 10; the path costs 2·24 = 48 > 12 + 30.
    let wall: Vec<(i32, i32)> = (0..10).map(|y| (2, y)).collect();
    assert_eq!(
        grid_path((0, 0), (4, 0), 20, 15, |c| wall.contains(&c)),
        None
    );
    assert_eq!(grid_path_model((0, 0), (4, 0), 20, 15, &wall), None);
    // Open at y = 3: found within the rounds.
    let wall: Vec<(i32, i32)> = (0..3).map(|y| (2, y)).collect();
    let p = grid_path((0, 0), (4, 0), 20, 15, |c| wall.contains(&c));
    assert_eq!(p, grid_path_model((0, 0), (4, 0), 20, 15, &wall));
    assert!(p.is_some());
}

/// Cliff marking from the rules: (directions,
/// whether flag 0x20 was set).
fn cliff_model(vs: &[Vertex]) -> (Vec<u8>, bool) {
    let n = vs.len();
    let at = |i: usize| vs[i % n];
    let link = |i: usize| at(i).flags & VERTEX_LINK != 0;
    let stop = |w: usize| {
        let (a, b) = (at(w), at(w + 1));
        a.y < b.y || a.x > b.x || link(w) || link(w + 1)
    };
    let turn = |w: usize| {
        let (a, b, c) = (at(w), at(w + 1), at(w + 2));
        !link(w) && !link(w + 1) && ((a.x < b.x && b.y < c.y) || (a.y > b.y && b.x < c.x))
    };
    let mut dirs: Vec<u8> = vs.iter().map(|v| v.direction).collect();
    let mut flag = false;
    let (mut v, mut p) = (0usize, n - 1);
    let mut head_passed = false;
    loop {
        let (cv, cp, cn) = (at(v), at(p), at(v + 1));
        let starts =
            !link(v) && !link(p) && ((cv.x < cn.x && cp.y > cv.y) || (cv.y > cn.y && cp.x > cv.x));
        let mut end = v;
        if starts {
            let mut w = v;
            let mut u = None;
            loop {
                if w == 0 {
                    head_passed = true;
                }
                if stop(w) {
                    end = w;
                    break;
                }
                if turn(w) {
                    u = Some(w);
                }
                w = (w + 1) % n;
                if w == v {
                    end = v;
                    break;
                }
            }
            if let Some(u) = u {
                let mut k = v;
                loop {
                    dirs[k] = 1;
                    if k == u {
                        break;
                    }
                    k = (k + 1) % n;
                }
                flag = true;
            }
        }
        p = end;
        v = (p + 1) % n;
        if head_passed || v == 0 {
            break;
        }
    }
    (dirs, flag)
}

/// Cliff marking against the rule model on pseudo-random vertex
/// lists (coordinates and link flags).
#[test]
fn cliff_marking_by_the_rules() {
    let mut seed = Seed::init_low(1414);
    let mut r = |n: i32| seed.roll(n) as i32;
    let mut marked = 0;
    for case in 0..500 {
        let n = 3 + r(8);
        let vs: Vec<Vertex> = (0..n)
            .map(|_| Vertex {
                x: r(6),
                y: r(6),
                direction: 0,
                flags: if r(5) == 0 { VERTEX_LINK } else { 0 },
            })
            .collect();
        let (dirs, flag) = cliff_model(&vs);
        let mut e = Env::new(4, 8, 8);
        e.info.vertices = vs.clone();
        let mut g = e.gen();
        g.cliff_marking();
        let got: Vec<u8> = g.info.vertices.iter().map(|v| v.direction).collect();
        assert_eq!(got, dirs, "case {case}: {vs:?}");
        assert_eq!(g.info.flags & 0x20 != 0, flag, "case {case}");
        marked += usize::from(flag);
    }
    assert!(marked > 30, "{marked}");
    // A link vertex next on the run stops it at once: no turn is reached
    // past the link (which would mark v0..v2).
    let v = |x, y, flags| Vertex {
        x,
        y,
        direction: 0,
        flags,
    };
    let vs = vec![
        v(0, 0, 0),
        v(2, 0, VERTEX_LINK),
        v(4, 0, 0),
        v(6, 0, 0),
        v(6, 5, 0),
        v(0, 5, 0),
    ];
    let mut e = Env::new(4, 8, 8);
    e.info.vertices = vs.clone();
    let mut g = e.gen();
    g.cliff_marking();
    assert!(g.info.vertices.iter().all(|v| v.direction == 0));
    assert_eq!(g.info.flags & 0x20, 0);
    assert_eq!(cliff_model(&vs), (vec![0; 6], false));
}

/// Cave and river steps: with flags 0x20 (no 0x40) one step, bit
/// := lo' & 1; bit 0 scans rows then columns, bit 1 uses (outer over gh,
/// inner over gw) as (x, y); the first grid-0 cell 16 → stamp 25, 17 →
/// 24, flags |= 0x40. With flags & 0x1C (no 0x40): y := gh − 4, x := gw −
/// 4 if 0x10 else gw − 5; r := lo' & 3: odd → x := 3, ≥ 2 → y := 3;
/// stamp 52 in level 2 else 51; flags |= 0x40.
#[test]
fn river_caves_cliff_and_side_cave() {
    let mut seen = [false; 2];
    for k in 0..12u32 {
        let mut e = Env::new(4, 7, 9);
        e.drlg.level_mut(e.l).seed = Seed::init_low(100 + 7919 * k);
        e.info.flags = 0x20;
        e.info.grids[0].op(4, 1, Op::Set, 16);
        e.info.grids[0].op(1, 3, Op::Set, 17);
        let mut s = e.seed();
        let bit = s.step() & 1;
        seen[bit as usize] = true;
        let mut g = e.gen();
        g.river_caves().unwrap();
        let (x, y, p) = if bit == 0 { (4, 1, 25) } else { (1, 3, 24) };
        assert_eq!(g.g(0, x, y), p, "k {k}");
        assert_eq!(g.info.flags & 0x40, 0x40);
        // F = −1: the build list's first roll(Files = 1), file 0.
        s.step();
        assert_eq!(*g.seed(), s);
        assert_eq!(super::grid::file_of(g.g(2, x, y)), 0);
    }
    assert_eq!(seen, [true, true]);
    let mut seen = [false; 4];
    for (id, flags) in [(2, 0x10), (4, 0x10), (4, 0x4), (4, 0x8)] {
        for k in 0..10u32 {
            let (gw, gh) = (9, 8);
            let mut e = Env::new(id, gw, gh);
            e.drlg.level_mut(e.l).seed = Seed::init_low(300 + 7919 * k);
            e.info.flags = flags;
            // A direction bit in column gw − 2: no river.
            e.info.grids[2].op(gw - 2, 0, Op::Set, cell::DIRECTION);
            let mut s = e.seed();
            let r = s.step() & 3;
            seen[r as usize] = true;
            let mut x = if flags & 0x10 != 0 { gw - 4 } else { gw - 5 };
            let mut y = gh - 4;
            if r & 1 != 0 {
                x = 3;
            }
            if r >= 2 {
                y = 3;
            }
            let mut g = e.gen();
            g.river_caves().unwrap();
            let p = if id == 2 { 52 } else { 51 };
            assert_eq!(g.g(0, x, y), p, "level {id} flags {flags:#x} k {k}");
            assert_eq!(g.info.flags & 0x40, 0x40);
            s.step();
            assert_eq!(*g.seed(), s);
            assert_eq!(super::grid::file_of(g.g(2, x, y)), 0);
        }
    }
    assert_eq!(seen, [true; 4]);
    // A row with the direction bit in both columns gw − 2 and
    // gw − 1 also blocks the river (OR, not XOR).
    let mut e = Env::new(4, 9, 8);
    e.info.flags = 0x4 | 0x40;
    e.info.grids[2].op(7, 2, Op::Set, cell::DIRECTION);
    e.info.grids[2].op(8, 2, Op::Set, cell::DIRECTION);
    let s = e.seed();
    let mut g = e.gen();
    g.river_caves().unwrap();
    assert!(g.info.grids[0].cells.iter().all(|&c| c != 26 && c != 27));
    assert_eq!(*g.seed(), s);
}

/// Flags 0x80 → 3 at (0, 0) F 1; 0x100 → 3 at
/// (gw − 7, 0) F 2; 0x200 → 2 at (0, 1) F 1; 0x400 → 2 at (0, gh − 6)
/// F 1. Without 0x40: level 2 FarAway(level 1 rect, 52, −1, 1, 15),
/// others SpawnOutdoorLevelPreset(51, −1, 1, 15); flags |= 0x40.
#[test]
fn act1_transitions() {
    use super::grid::file_of;
    let (gw, gh) = (10, 9);
    for (flag, (x, y, p, f)) in [
        (0x80, (0, 0, 3, 1)),
        (0x100, (gw - 7, 0, 3, 2)),
        (0x200, (0, 1, 2, 1)),
        (0x400, (0, gh - 6, 2, 1)),
    ] {
        let mut e = Env::new(4, gw, gh);
        e.info.flags = flag | 0x40;
        let mut g = e.gen();
        g.transitions().unwrap();
        let stamped: Vec<_> = (0..gh)
            .flat_map(|yy| (0..gw).map(move |xx| (xx, yy)))
            .filter(|&(xx, yy)| g.g(0, xx, yy) != 0)
            .map(|(xx, yy)| (xx, yy, g.g(0, xx, yy), file_of(g.g(2, xx, yy))))
            .collect();
        assert_eq!(stamped, [(x, y, p, f)], "flag {flag:#x}");
    }
    // Without 0x40, level 4: one 51 (file 0 from the build list), flag 0x40.
    let mut e = Env::new(4, gw, gh);
    let mut g = e.gen();
    g.transitions().unwrap();
    let cells: Vec<_> = g.info.grids[0].cells.iter().filter(|&&c| c != 0).collect();
    assert_eq!(cells, [&51]);
    assert_eq!(g.info.flags & 0x40, 0x40);
    let at = g.info.grids[0].cells.iter().position(|&c| c == 51).unwrap() as i32;
    assert_eq!(file_of(g.g(2, at % gw, at / gw)), 0);
    // Level 2: FarAway from the Rogue Encampment's rect places 52.
    let mut e = Env::new(2, gw, gh);
    let l1 = e
        .drlg
        .get_or_alloc_level(&e.data, &mut NoLevelTypes, 1)
        .unwrap();
    e.drlg.level_mut(l1).rect = TileRect::new(700, 700, 40, 40);
    let mut g = e.gen();
    g.transitions().unwrap();
    let cells: Vec<_> = g.info.grids[0].cells.iter().filter(|&&c| c != 0).collect();
    assert_eq!(cells, [&52]);
    assert_eq!(g.info.flags & 0x40, 0x40);
    let at = g.info.grids[0].cells.iter().position(|&c| c == 52).unwrap() as i32;
    assert_eq!(file_of(g.g(2, at % gw, at / gw)), 0);
}

/// Rivers from the rules: river halves 26 / 27 with files
/// U[P] / L[P] (P = 7 with file 3 → 3; P = 0: 0 if blank else 3); with
/// flags & 0x14 a bridge: R := gh − 2, r := roll(R); first y = (r + i)
/// mod R + 1 with spawn valid at (x − 1, y) and (unless 0x4) (x + 2, y)
/// and files 3 at (x, y), (x + 1, y): 28 at (x, y) F 1 and (x + 1, y) F 3
/// if 0x4 else 2.
#[test]
fn river_and_bridge_by_the_rules() {
    use super::grid::file_of;
    use super::wild::RIVER_FILES;
    let (gw, gh, x) = (8, 9, 3);
    let mut bridges = [0; 2];
    for flags in [0, 0x4, 0x10, 0x14] {
        for k in 0..24u32 {
            let mut e = Env::new(4, gw, gh);
            e.drlg.level_mut(e.l).seed = Seed::init_low(77 + 7919 * k);
            e.info.flags = flags;
            // Column contents (P, file, blank) for x and x + 1, by row.
            let col = |y: i32, side: i32| -> (u32, u32, bool) {
                match (y + side + k as i32) % 5 {
                    0 => (0, 0, false),
                    1 => (0, 0, true),
                    2 => (7, 3, false),
                    3 => (4 + ((y + 3 * side) % 12) as u32, 0, false),
                    _ => (0, 0, false),
                }
            };
            for y in 0..gh {
                for side in 0..2 {
                    let (p, f, blank) = col(y, side);
                    e.info.grids[0].op(x + side, y, Op::Set, p);
                    let v = f << 16 | if blank { cell::BLANK } else { 0 };
                    e.info.grids[2].op(x + side, y, Op::Set, v);
                }
                // Neighbours: (x − 1, y) not spawn valid on every third row.
                if (y + k as i32) % 3 == 0 {
                    e.info.grids[2].op(x - 1, y, Op::Set, cell::NOT_SPAWN);
                }
                if (y + (k / 2) as i32) % 2 == 0 {
                    e.info.grids[2].op(x + 2, y, Op::Set, cell::NOT_SPAWN);
                }
            }
            let files: Vec<(i32, i32)> = (0..gh)
                .map(|y| {
                    let half = |side: i32| {
                        let (p, f, blank) = col(y, side);
                        match p {
                            0 if blank => 0,
                            0 => 3,
                            7 if f == 3 => 3,
                            _ => {
                                let (u, l) = RIVER_FILES[(p - 4) as usize];
                                if side == 0 {
                                    u
                                } else {
                                    l
                                }
                            }
                        }
                    };
                    (half(0), half(1))
                })
                .collect();
            let mut s = e.seed();
            let mut bridge = None;
            if flags & 0x14 != 0 {
                let rn = gh - 2;
                let r = s.roll(rn) as i32;
                for i in 0..rn {
                    let y = (r + i) % rn + 1;
                    let ok = e.info.grids[2].get(x - 1, y) & cell::NOT_SPAWN == 0
                        && (flags & 0x4 != 0
                            || e.info.grids[2].get(x + 2, y) & cell::NOT_SPAWN == 0)
                        && files[y as usize] == (3, 3);
                    if ok {
                        bridge = Some(y);
                        break;
                    }
                }
            }
            let mut g = e.gen();
            g.river(x).unwrap();
            for y in 0..gh {
                let got = (
                    g.g(0, x, y),
                    file_of(g.g(2, x, y)),
                    g.g(0, x + 1, y),
                    file_of(g.g(2, x + 1, y)),
                );
                let want = if bridge == Some(y) {
                    (28, 1, 28, if flags & 0x4 != 0 { 3 } else { 2 })
                } else {
                    (26, files[y as usize].0, 27, files[y as usize].1)
                };
                assert_eq!(got, want, "flags {flags:#x} k {k} y {y}");
            }
            assert_eq!(*g.seed(), s, "flags {flags:#x} k {k}");
            if bridge.is_some() {
                bridges[usize::from(flags & 0x4 != 0)] += 1;
            }
        }
    }
    assert!(bridges[0] > 3 && bridges[1] > 3, "{bridges:?}");
}

/// Paths from the rules on a prepared level: starts
/// (neighbour entries of levels 1 and 26, then grid cells x outer, y
/// inner), adjusted points, the join point (bridge or the centre search),
/// grid paths (bit 0x80 in grid 2) and the jitter.
#[test]
fn dirt_paths_by_the_rules() {
    use super::wild::grid_path;
    use super::{PathEnds, PathPoint};
    let pt = |x, y, direction| PathPoint { x, y, direction };
    for (case, flags) in [(0u32, 0u32), (1, 0x10), (2, 0), (3, 0x10)] {
        for k in 0..4u32 {
            let (gw, gh) = (10, 8);
            let mut e = Env::new(4, gw, gh);
            e.drlg.level_mut(e.l).seed = Seed::init_low(900 + 7919 * (k + 4 * case));
            e.info.flags = flags;
            let lr = e.drlg.level(e.l).rect;
            // Neighbour entries: the town on side `case`, level 26, another.
            let town = TileRect::new(700 + 3 * k as i32, 650 + 5 * case as i32, 56, 40);
            let orth = |id, d, r| Orth {
                level_id: id,
                direction: d,
                init: false,
                rect: r,
                preset: true,
            };
            e.info.orth = vec![
                orth(1, case as i32, town),
                orth(26, 1, TileRect::new(900, 600, 40, 18)),
                orth(3, 2, TileRect::new(1000, 800, 80, 80)),
            ];
            // Grid cells: border pieces with file 3, caves, river pieces.
            let put = |e: &mut Env, x: i32, y: i32, p: u32, f: u32| {
                e.info.grids[0].op(x, y, Op::Set, p);
                e.info.grids[2].op(x, y, Op::Set, cell::PRESET | f << 16);
            };
            put(&mut e, 0, 2 + k as i32 % 3, 5, 3);
            put(&mut e, 4, 0, 6, 3);
            put(&mut e, 9, 5, 7, 3);
            put(&mut e, 3, 7, 4, 3);
            put(&mut e, 2, 0, 4, 2); // file ≠ 3: no start
            put(&mut e, 8, 6, if case < 2 { 51 } else { 52 }, k % 2);
            put(&mut e, 6, 3, 24, 0);
            put(&mut e, gw - 2, 1, 28, 1);
            put(&mut e, gw / 2 - 1, 2 + k as i32 % 2, 28, 1);
            // File 1 without the bridge preset, above it: not the bridge.
            put(&mut e, gw / 2 - 1, 1, 9, 1);
            // The model.
            let mut starts: Vec<PathPoint> = Vec::new();
            let d = case as i32;
            let (tx, ty) = [(59, 19), (29, 35), (4, 22), (29, 3)][case as usize];
            starts.push(pt(town.x + tx, town.y + ty, d));
            starts.push(pt(927, 613, 1));
            for x in 0..gw {
                for y in 0..gh {
                    let p = e.info.grids[0].get(x, y);
                    let f = (e.info.grids[2].get(x, y) >> 16) & 0xF;
                    let dir = match (p, f) {
                        (4, 3) => 3,
                        (5, 3) => 0,
                        (6, 3) => 1,
                        (7, 3) => 2,
                        (24, _) => 1,
                        (25, _) => 0,
                        (28, 1) if x == gw - 2 => 2,
                        (51 | 52, f) => i32::from(f != 0),
                        _ => continue,
                    };
                    starts.push(pt(lr.x + 8 * x + 3, lr.y + 8 * y + 3, dir));
                }
            }
            let adjust = |p: PathPoint| {
                let (mut qx, mut qy) = (p.x - lr.x, p.y - lr.y);
                match p.direction {
                    0 => qx = 8 * (qx / 8) + 11,
                    1 => qy = 8 * (qy / 8) + 11,
                    2 => qx = 8 * (qx / 8) - 5,
                    3 => qy = 8 * (qy / 8) - 5,
                    _ => {}
                }
                pt(qx + lr.x, qy + lr.y, p.direction)
            };
            let bridge = if flags & 0x10 != 0 {
                let x = gw / 2 - 1;
                (1..gw - 1)
                    .find(|&y| {
                        e.info.grids[0].get(x, y) == 28
                            && (e.info.grids[2].get(x, y) >> 16) & 0xF == 1
                    })
                    .map(|y| (x, y))
            } else {
                None
            };
            let n = starts.len() as i32;
            let joins: Vec<PathPoint> = match bridge {
                Some((x, y)) => {
                    let (bx, by) = (lr.x + 8 * x + 3, lr.y + 8 * y + 3);
                    starts
                        .iter()
                        .map(|s| {
                            if s.x <= bx {
                                pt(bx, by, 2)
                            } else {
                                pt(bx + 8, by, 0)
                            }
                        })
                        .collect()
                }
                None => {
                    let cx = starts.iter().map(|s| s.x - lr.x).sum::<i32>() / (8 * n);
                    let cy = starts.iter().map(|s| s.y - lr.y).sum::<i32>() / (8 * n);
                    let mut last = (cx, cy);
                    'f: for r in 0..8 {
                        for (dx, dy) in [(-1, 0), (0, 1), (0, -1), (1, 0)] {
                            last = (cx + r * dx, cy + r * dy);
                            let c = e.info.grids[2].get(last.0, last.1);
                            if e.info.grids[2].contains(last.0, last.1) && c & cell::NOT_SPAWN == 0
                            {
                                break 'f;
                            }
                        }
                    }
                    vec![pt(lr.x + 8 * last.0 + 3, lr.y + 8 * last.1 + 3, 4); starts.len()]
                }
            };
            let ends: Vec<PathEnds> = starts
                .iter()
                .zip(&joins)
                .map(|(&s, &j)| PathEnds {
                    start: s,
                    start_adjusted: adjust(s),
                    join_adjusted: adjust(j),
                    join: j,
                })
                .collect();
            let mut m2 = e.info.grids[2].clone();
            let mut s = e.seed();
            let mut paths = Vec::new();
            for en in &ends {
                let a = (
                    (en.start_adjusted.x - lr.x) / 8,
                    (en.start_adjusted.y - lr.y) / 8,
                );
                let b = (
                    (en.join_adjusted.x - lr.x) / 8,
                    (en.join_adjusted.y - lr.y) / 8,
                );
                let snapshot = m2.clone();
                let path = grid_path(a, b, gw, gh, |c| snapshot.get(c.0, c.1) & cell::PRESET != 0);
                let mut kk = (s.step() & 3) as usize;
                let mut out = Vec::new();
                if let Some(p) = &path {
                    for &(x, y) in p {
                        m2.op(x, y, Op::Or, cell::PATH);
                    }
                    if en.join.direction != 4 {
                        out.push((en.join.x, en.join.y));
                    }
                    for (i, &(x, y)) in p.iter().enumerate() {
                        if i == 0 {
                            out.push((en.join_adjusted.x, en.join_adjusted.y));
                        } else if i + 1 < p.len() {
                            let ox = ((s.step() & 1) as i32 + 2) * [1, 0, -1, 0][kk];
                            let oy = ((s.step() & 1) as i32 + 2) * [0, 1, 0, -1][kk];
                            kk = (kk + 1) % 4;
                            out.push((8 * x + lr.x + ox + 3, 8 * y + lr.y + oy + 3));
                        } else {
                            out.push((en.start_adjusted.x, en.start_adjusted.y));
                        }
                    }
                    out.push((en.start.x, en.start.y));
                }
                paths.push(out);
            }
            let mut g = e.gen();
            g.dirt_paths().unwrap();
            assert_eq!(g.info.path_ends, ends, "case {case} k {k}: ends");
            assert_eq!(g.info.paths, paths, "case {case} k {k}: paths");
            assert_eq!(g.info.grids[2].cells, m2.cells, "case {case} k {k}: grid 2");
            assert_eq!(*g.seed(), s, "case {case} k {k}: seed");
            assert!(
                paths.iter().any(|p| p.len() > 3),
                "case {case} k {k}: a real path"
            );
        }
    }
}

/// With one start the join search starts at
/// the grid centre (gw/2, gh/2); the first in-grid spawn-valid cell of
/// r = 0.. and the four directions (−1, 0), (0, 1), (0, −1), (1, 0).
#[test]
fn dirt_path_single_start_centre() {
    use super::PathPoint;
    let (gw, gh) = (9, 7);
    for blocked in [
        vec![],
        vec![(4, 3)],
        vec![(4, 3), (3, 3)],
        vec![(4, 3), (3, 3), (4, 4)],
    ] {
        let mut e = Env::new(4, gw, gh);
        e.info.orth = vec![Orth {
            level_id: 26,
            direction: 1,
            init: false,
            rect: TileRect::new(700, 600, 40, 18),
            preset: true,
        }];
        for &(x, y) in &blocked {
            e.info.grids[2].op(x, y, Op::Set, cell::WAYPOINT);
        }
        let lr = e.drlg.level(e.l).rect;
        let want = match blocked.len() {
            0 => (4, 3),
            1 => (3, 3),
            2 => (4, 4),
            _ => (4, 2),
        };
        let mut g = e.gen();
        g.dirt_paths().unwrap();
        assert_eq!(
            g.info.path_ends[0].join,
            PathPoint {
                x: lr.x + 8 * want.0 + 3,
                y: lr.y + 8 * want.1 + 3,
                direction: 4
            },
            "{blocked:?}"
        );
    }
}

/// Burial Grounds stamps 108 at (1, 1) with F −1 (the
/// build list: file 0 with Files 1); Moo Moo Farm spawns 50, 46, 31, 38,
/// 39, 29, 30 once each.
#[test]
fn act1_special_presets_17_and_39() {
    use super::grid::file_of;
    let mut e = Env::new(17, 8, 8);
    let mut g = e.gen();
    g.act1().unwrap();
    assert_eq!(g.g(0, 1, 1), 108);
    assert_eq!(file_of(g.g(2, 1, 1)), 0);
    let mut e = Env::new(39, 10, 10);
    for t in 0..4 {
        let name = format!("inert{t}").into_bytes();
        e.od.subs.push(SubRow {
            type_: t,
            file: name.clone(),
            bord_type: 1,
            grid_size: 1,
            ..SubRow::default()
        });
        e.subs
            .0
            .insert(name, super::tests::one_cell_file(0, (200 << 8) | 1, 1));
    }
    // The level outline (borders keep the inside spawnable).
    let v = |x, y| Vertex {
        x,
        y,
        direction: 0,
        flags: 0,
    };
    e.info.vertices = vec![v(0, 9), v(0, 0), v(9, 0), v(9, 9)];
    let mut g = e.gen();
    g.act1().unwrap();
    let specials = [29, 30, 31, 38, 39, 46, 50];
    let mut placed: Vec<u32> = g.info.grids[0]
        .cells
        .iter()
        .copied()
        .filter(|c| specials.contains(c))
        .collect();
    placed.sort();
    assert_eq!(placed, specials);
}

/// Cottage(P, extra): one step lo' & 3; non-zero:
/// RandomDS1(P), and with extra one step lo' & 1, non-zero →
/// RandomDS1(49); zero: RandomDS1(P) twice.
#[test]
fn cottage_counts() {
    let mut seen = [false; 3];
    for k in 0..40u32 {
        for extra in [false, true] {
            let mut e = Env::new(4, 10, 10);
            e.drlg.level_mut(e.l).seed = Seed::init_low(5 + 7919 * k);
            let mut s = e.seed();
            let first = s.step() & 3;
            let mut g = e.gen();
            g.cottage(47, extra).unwrap();
            let count = |p: u32| g.info.grids[0].cells.iter().filter(|&&c| c == p).count();
            let (n47, n49) = (count(47), count(49));
            if first == 0 {
                assert_eq!((n47, n49), (2, 0), "k {k}");
                seen[0] = true;
            } else {
                assert_eq!(n47, 1, "k {k}");
                if extra {
                    // The second draw follows the first RandomDS1's draws:
                    // only its effect is checked.
                    assert!(n49 <= 1);
                    seen[1 + n49] = true;
                } else {
                    assert_eq!(n49, 0);
                }
            }
        }
    }
    assert_eq!(seen, [true; 3]);
}

/// The bridge cell is searched for y in
/// 1..gw − 2 (bounded by gw, not gh), so a bridge at y = gw − 1 of a taller grid is
/// not found; with no start there is no join point at all.
#[test]
fn dirt_path_bridge_rows_and_no_start() {
    let (gw, gh) = (6, 9);
    let mut e = Env::new(4, gw, gh);
    e.info.flags = 0x10;
    e.info.orth = vec![Orth {
        level_id: 26,
        direction: 1,
        init: false,
        rect: TileRect::new(700, 600, 40, 18),
        preset: true,
    }];
    e.info.grids[0].op(gw / 2 - 1, gw - 1, Op::Set, 28);
    e.info.grids[2].op(gw / 2 - 1, gw - 1, Op::Set, cell::PRESET | 1 << 16);
    let mut g = e.gen();
    g.dirt_paths().unwrap();
    assert_eq!(g.info.path_ends[0].join.direction, 4);
    // No start: nothing, no draws.
    let mut e = Env::new(4, gw, gh);
    let s = e.seed();
    let mut g = e.gen();
    g.dirt_paths().unwrap();
    assert!(g.info.path_ends.is_empty());
    assert_eq!(*g.seed(), s);
}

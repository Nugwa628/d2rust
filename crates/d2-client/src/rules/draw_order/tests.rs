//! Tests of the draw order: worked vectors and rules.

use std::collections::BTreeMap;

use super::sky::SkyFrame;
use super::source::{
    ordered_source, placement_list, resolve, resolve_drawn, TileArt, WeatherFrame,
};
use super::weather::{EnvPool, FloorContext, LevelWeather, PoolRecord, SkyDraw, Weather};
use super::*;
use crate::bridge::world::ClientWorld;
use crate::bridge::ClientUnit;
use crate::composite::ComponentFrame;
use crate::frames::{FramePart, FrameSetKey};
use crate::rules::camera::{FrameSize, TileList, UnitPosition};
use crate::rules::view::{MapTile, ViewSource};
use crate::scene::{BlendOp, ShadeChain};
use crate::world_view::{RunningShake, UnitPose, ViewAssets, ViewError, ViewFeed};
use d2_formats::palette::{Palette, Rgb};
use d2_sim::rng::Seed;

const CLOCK: FadeClock = FadeClock {
    now: 10_000,
    instant: false,
};

fn pos(x: i32, y: i32) -> ClientPos {
    ClientPos { x, y }
}

/// The 800 × 600 mode 0 grid at tile origin (600, 1720).
fn grid() -> DrawGrid {
    DrawGrid::new(800, 560, pos(600, 1720))
}

fn layered(layer: u32) -> Layered {
    Layered {
        room: 0,
        record: layer as usize,
        layer,
    }
}

fn layers(list: &[Layered]) -> Vec<u32> {
    list.iter().map(|l| l.layer).collect()
}

fn record(tile: (i32, i32), layer: u32, ty: u32) -> TileRecord {
    TileRecord {
        tile,
        flags: layer << 14,
        ty,
        dt1: Dt1Facts::default(),
        fade: Fade::OPAQUE,
        logical: None,
    }
}

/// One room at tile (20, 10), 20 × 20 tiles: inside the grid of [`grid`].
fn room() -> Room {
    Room {
        tiles: TileRect {
            x: 20,
            y: 10,
            w: 20,
            h: 20,
        },
        subtile_origin: (100, 50),
        ..Room::default()
    }
}

fn near(rooms: Vec<Room>) -> NearRooms {
    NearRooms {
        rooms,
        player_tile: (0, 0),
        player_logical: 0,
        player_subtile: (0, 0),
        edge: None,
        level: LevelFacts::default(),
    }
}

fn unit(guid: u32, unit_type: u8, mode: u32) -> RoomUnit {
    RoomUnit {
        key: UnitKey { unit_type, guid },
        facts: UnitFacts {
            unit_type,
            mode,
            sight_hidden: Some(false),
            ..UnitFacts::default()
        },
    }
}

#[test]
fn grid_size_and_origin() {
    let g = grid();
    assert_eq!((g.a, g.side), (16, 34));
    let low = DrawGrid::new(640, 440, pos(0, 0));
    assert_eq!((low.a, low.side), (15, 31));
    assert_eq!(g.origin, (22, 1));
    // Player at client (1000, 2000) → cell (9, 17), index 587.
    assert_eq!(tile_of(1000, 2000), (31, 18));
    assert_eq!(g.cell(tile_of(1000, 2000)), Some(587));
    assert_eq!(g.cell((21, 1)), None);
    assert_eq!(g.cell((22 + 34, 1)), None);
    assert_eq!(g.cell((22 + 33, 1 + 33)), Some(34 * 34 - 1));
}

#[test]
fn grid_of_camera_uses_the_view_rectangle() {
    // Mode 1 shifts the view; Wv stays W.
    let cam = Camera::new(
        FrameSize::D2RUST,
        OpenMode::new(1).unwrap(),
        pos(1000, 2000),
        (0, 0),
    );
    let g = DrawGrid::of_camera(&cam);
    assert_eq!((g.a, g.side), (16, 34));
    assert_eq!(g.view.left, cam.tile.x);
    assert_eq!(g.view.right - g.view.left, 800);
    assert_eq!(g.view.bottom - g.view.top, 560);
}

#[test]
fn tile_of_pixel() {
    assert_eq!(tile_of(600, 1720), (25, 17));
    assert_eq!(q(-160), -2);
    assert_eq!(q(-1), -1);
    assert_eq!(q(159), 0);
    // DO1, answered: ty = q(2·0 + 160) = 1.
    assert_eq!(tile_of(-160, 0), (-2, 1));
    assert_eq!(tile_of(0, -80), (-2, -2));
    assert_eq!(tile_of(-1, 0), (-1, 0));
}

#[test]
fn layer_sorted_insertion() {
    let cases: [(&[u32], u32, &[u32]); 4] = [
        (&[2], 1, &[2, 1]),
        (&[2, 3], 1, &[1, 2, 3]),
        (&[1, 3], 2, &[1, 3, 2]),
        (&[], 3, &[3]),
    ];
    for (start, new, want) in cases {
        let mut list: Vec<Layered> = start.iter().map(|&l| layered(l)).collect();
        insert_layered(&mut list, layered(new), |l| l.layer);
        assert_eq!(layers(&list), want, "{start:?} + {new}");
    }
}

#[test]
fn wall_record_files_into_its_own_cell() {
    assert_eq!(tile_entry(30, 20), (720, 2080));
    assert_eq!(tile_of(720, 2080), (30, 21));
    let mut r = room();
    // Tile (30, 20) = room origin (20, 10) + (10, 10).
    r.walls.push(record((10, 10), 1, 1));
    let mut n = near(vec![r]);
    let lists = fill(&grid(), &mut n, &BTreeMap::new(), CLOCK, false).unwrap();
    let ci = grid().cell((30, 20)).unwrap();
    assert_eq!(lists.cells[ci].wall.len(), 1);
    assert_eq!(lists.count, 1);
}

#[test]
fn wall_array_records_go_by_layer_bits_and_type() {
    let mut r = room();
    r.walls.push(record((10, 10), 0, 1)); // no layer bits → shadow list
    r.walls.push(record((10, 10), 1, 15)); // roof
    r.walls.push(record((10, 10), 1, 16)); // lower wall
    r.walls.push(record((10, 10), 2, 3)); // wall
    let mut n = near(vec![r]);
    let lists = fill(&grid(), &mut n, &BTreeMap::new(), CLOCK, false).unwrap();
    let cell = &lists.cells[grid().cell((30, 20)).unwrap()];
    assert_eq!(
        cell.shadow,
        vec![Entry::Tile {
            room: 0,
            array: TileArray::Wall,
            record: 0
        }]
    );
    assert_eq!(cell.roof.iter().map(|l| l.record).collect::<Vec<_>>(), [1]);
    assert_eq!(cell.lower.iter().map(|l| l.record).collect::<Vec<_>>(), [2]);
    assert_eq!(cell.wall.iter().map(|l| l.record).collect::<Vec<_>>(), [3]);
    assert_eq!(
        lists.flags,
        GridFlags {
            lower_walls: true,
            roofs: true,
            shadows: true
        }
    );
    // Cell flag 4: the last wall-array record filed resets the word, then
    // sets 4 for a wall of type ∉ {0, 13} without record flag 0x4.
    assert_eq!(cell.flags, 4);
}

#[test]
fn cell_flag_word_follows_the_last_record() {
    let mut r = room();
    r.walls.push(record((10, 10), 1, 1));
    let mut last = record((10, 10), 1, 13);
    last.flags |= REC_NO_FADE;
    r.walls.push(last);
    let mut n = near(vec![r]);
    let lists = fill(&grid(), &mut n, &BTreeMap::new(), CLOCK, false).unwrap();
    assert_eq!(lists.cells[grid().cell((30, 20)).unwrap()].flags, 0);
}

#[test]
fn record_rectangle_test() {
    let v = grid().view;
    let e = (v.right, v.top + 100);
    assert!(record_visible(e, 0, 0, &v));
    assert!(!record_visible((v.right + 1, e.1), 0, 0, &v));
    assert!(record_visible((v.left - 160, e.1), 0, 0, &v));
    assert!(!record_visible((v.left - 161, e.1), 0, 0, &v));
    // e1 − rh + h ≤ bottom and e1 − rh − h ≥ top.
    assert!(record_visible((v.left, v.bottom + 50), 50, 0, &v));
    assert!(!record_visible((v.left, v.bottom + 51), 50, 0, &v));
    assert!(!record_visible((v.left, v.top + 10), 0, 11, &v));
}

#[test]
fn room_test_and_corners() {
    let t = TileRect {
        x: 20,
        y: 10,
        w: 20,
        h: 20,
    };
    let r = room_rect(0, &t).unwrap();
    assert_eq!(room_corner(20, 10), (720, 1280));
    assert_eq!(
        r,
        PixelRect {
            left: room_corner(20, 30).0,
            top: 1280,
            right: room_corner(40, 10).0,
            bottom: room_corner(40, 30).1
        }
    );
    let inverted = TileRect { w: 0, h: -10, ..t };
    assert!(matches!(
        room_rect(3, &inverted),
        Err(OrderError::RoomRect { room: 3, .. })
    ));
    let v = PixelRect {
        left: 0,
        top: 0,
        right: 100,
        bottom: 100,
    };
    let at = |left, top, right, bottom| PixelRect {
        left,
        top,
        right,
        bottom,
    };
    assert!(room_visible(&at(100, 500, 200, 600), &v));
    assert!(!room_visible(&at(101, 0, 200, 600), &v));
    assert!(!room_visible(&at(0, 501, 200, 600), &v));
    assert!(room_visible(&at(-300, 0, -200, 10), &v));
    assert!(!room_visible(&at(-300, 0, -201, 10), &v));
    assert!(!room_visible(&at(0, -20, 10, -1), &v));
    // A skipped room files nothing.
    let mut far = room();
    far.tiles.x = 500;
    far.walls.push(record((0, 0), 1, 1));
    let mut n = near(vec![far]);
    let lists = fill(&grid(), &mut n, &BTreeMap::new(), CLOCK, false).unwrap();
    assert_eq!(lists.count, 0);
}

#[test]
fn shadow_array_appends() {
    let mut r = room();
    r.shadows.push(record((10, 10), 0, 0));
    r.shadows.push(record((10, 10), 0, 0));
    let mut n = near(vec![r]);
    let lists = fill(&grid(), &mut n, &BTreeMap::new(), CLOCK, false).unwrap();
    let cell = &lists.cells[grid().cell((30, 20)).unwrap()];
    let records: Vec<_> = cell
        .shadow
        .iter()
        .map(|e| match e {
            Entry::Tile {
                array: TileArray::Shadow,
                record,
                ..
            } => *record,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(records, [0, 1]);
}

#[test]
fn the_fill_runs_one_y_sort_pass_per_room() {
    let mut r = room();
    for g in 1..=4 {
        r.units.push(unit(g, MONSTER, 1));
    }
    let mut n = near(vec![r]);
    let ys = [30, 10, 30, 10];
    let positions: BTreeMap<_, _> = n.rooms[0]
        .units
        .iter()
        .zip(ys)
        .map(|(u, y)| (u.key, pos(1000, 2000 + y)))
        .collect();
    // "Skip units": no sort.
    fill(&grid(), &mut n, &positions, CLOCK, true).unwrap();
    assert!(!n.rooms[0].units_sorted);
    fill(&grid(), &mut n, &positions, CLOCK, false).unwrap();
    let guids: Vec<u32> = n.rooms[0].units.iter().map(|u| u.key.guid).collect();
    // One pass (the sort never restarts from the head): the greater y
    // moves down one swap at a time, equal y keep their list order, and
    // 4 (y 10) stays behind 1 (y 30).
    assert_eq!(guids, [2, 1, 4, 3]);
    assert!(n.rooms[0].units_sorted);
    // The next frame's pass starts from that order.
    fill(&grid(), &mut n, &positions, CLOCK, false).unwrap();
    let guids: Vec<u32> = n.rooms[0].units.iter().map(|u| u.key.guid).collect();
    assert_eq!(guids, [2, 4, 1, 3]);
    // A room failing the room test is not sorted.
    let mut far = room();
    far.tiles.x = 100_000;
    far.units.push(unit(9, MONSTER, 1));
    far.units.push(unit(8, MONSTER, 1));
    let mut n = near(vec![far]);
    let positions: BTreeMap<_, _> = [(9, 5), (8, 1)]
        .map(|(g, y)| (n.rooms[0].units[if g == 9 { 0 } else { 1 }].key, pos(0, y)))
        .into_iter()
        .collect();
    fill(&grid(), &mut n, &positions, CLOCK, false).unwrap();
    assert!(!n.rooms[0].units_sorted);
    assert_eq!(n.rooms[0].units[0].key.guid, 9);
}

#[test]
fn units_file_by_flatness() {
    let mut r = room();
    let mut standing = unit(1, MONSTER, 1);
    standing.facts.flag_ex = UNIT_EX_VISIBLE;
    r.units.push(standing);
    r.units.push(unit(2, MONSTER, 12)); // dead, unflatDead 0: flat
    r.units.push(unit(3, ITEM, 3)); // on the ground: flat
    r.units.push(unit(4, PLAYER, 1)); // outside the grid
    let mut n = near(vec![r]);
    let mut positions = BTreeMap::new();
    for g in 1..=3 {
        positions.insert(n.rooms[0].units[g - 1].key, pos(1000, 2000));
    }
    // Same y as the others, so the Y sort keeps the list order.
    positions.insert(n.rooms[0].units[3].key, pos(-100_000, 2000));
    let lists = fill(&grid(), &mut n, &positions, CLOCK, false).unwrap();
    let cell = &lists.cells[587];
    assert_eq!(cell.unit, vec![Entry::Unit { room: 0, unit: 0 }]);
    assert_eq!(
        cell.shadow,
        vec![
            Entry::UnitShadow { room: 0, unit: 0 },
            Entry::Unit { room: 0, unit: 1 },
            Entry::Unit { room: 0, unit: 2 },
        ]
    );
    assert_eq!(lists.count, 4);
    // "skip units" files none.
    let lists = fill(&grid(), &mut n, &positions, CLOCK, true).unwrap();
    assert_eq!(lists.count, 0);
    // A unit without a position is an error.
    let err = fill(&grid(), &mut n, &BTreeMap::new(), CLOCK, false).unwrap_err();
    assert!(matches!(err, OrderError::NoPosition { room: 0, .. }));
}

#[test]
fn flat_units() {
    let f = |unit_type, mode| UnitFacts {
        unit_type,
        mode,
        ..UnitFacts::default()
    };
    assert!(is_flat(&f(PLAYER, 17)));
    assert!(!is_flat(&f(PLAYER, 12)));
    assert!(is_flat(&f(MONSTER, 12)));
    assert!(!is_flat(&UnitFacts {
        unflat_dead: true,
        ..f(MONSTER, 12)
    }));
    assert!(is_flat(&UnitFacts {
        draw_under: 2,
        ..f(OBJECT, 0)
    }));
    assert!(!is_flat(&UnitFacts {
        draw_under: 1,
        ..f(OBJECT, 1)
    }));
    assert!(is_flat(&UnitFacts {
        draw_under: 1,
        ..f(OBJECT, 2)
    }));
    assert!(!is_flat(&f(MISSILE, 0)));
    assert!(is_flat(&UnitFacts {
        flags: UNIT_MISSILE_FLAT,
        ..f(MISSILE, 0)
    }));
    assert!(is_flat(&f(ITEM, 3)));
    assert!(!is_flat(&f(ITEM, 4)));
    assert!(is_flat(&UnitFacts {
        flags: UNIT_FLAT,
        ..f(MONSTER, 1)
    }));
}

#[test]
fn pool_drops_past_3000() {
    let mut r = room();
    for i in 0..POOL + 1 {
        let mut rec = record((10, 10), 0, 0);
        rec.ty = i as u32;
        r.shadows.push(rec);
    }
    let mut n = near(vec![r]);
    let lists = fill(&grid(), &mut n, &BTreeMap::new(), CLOCK, false).unwrap();
    assert_eq!(lists.count, POOL + 1);
    let filed: usize = lists.cells.iter().map(|c| c.shadow.len()).sum();
    assert_eq!(filed, POOL);
    // The 3,001st (record index 3000) is the one dropped.
    let last = lists.cells.iter().flat_map(|c| &c.shadow).last().unwrap();
    assert_eq!(
        *last,
        Entry::Tile {
            room: 0,
            array: TileArray::Shadow,
            record: POOL - 1
        }
    );
}

#[test]
fn skipped_units() {
    let f = |unit_type, mode| UnitFacts {
        unit_type,
        mode,
        sight_hidden: Some(false),
        ..UnitFacts::default()
    };
    assert!(unit_skipped(&UnitFacts {
        flag_ex: UNIT_EX_SKIP,
        ..f(OBJECT, 0)
    }));
    assert!(unit_skipped(&f(PLAYER, 17)));
    assert!(!unit_skipped(&UnitFacts {
        playerbody: true,
        ..f(PLAYER, 17)
    }));
    assert!(unit_skipped(&UnitFacts {
        attached: true,
        ..f(MONSTER, 1)
    }));
    assert!(unit_skipped(&UnitFacts {
        invis: true,
        ..f(OBJECT, 1)
    }));
    assert!(!unit_skipped(&UnitFacts {
        invis: true,
        ..f(MISSILE, 1)
    }));
    // A skipped unit keeps its flags.
    let mut u = UnitFacts {
        flag_ex: UNIT_EX_SKIP | UNIT_EX_VISIBLE,
        ..f(MONSTER, 1)
    };
    assert!(!unit_draws(&mut u).unwrap());
    assert_eq!(u.flag_ex, UNIT_EX_SKIP | UNIT_EX_VISIBLE);
    assert_eq!(u.flags & UNIT_DRAWN, 0);
}

#[test]
fn sight_test() {
    let f = |unit_type, mode, local| UnitFacts {
        unit_type,
        mode,
        local,
        ..UnitFacts::default()
    };
    assert!(sight_tested(&f(PLAYER, 1, false)));
    assert!(!sight_tested(&f(PLAYER, 1, true)));
    assert!(!sight_tested(&f(PLAYER, 0, false)));
    assert!(!sight_tested(&f(PLAYER, 17, false)));
    assert!(sight_tested(&f(MONSTER, 1, false)));
    assert!(!sight_tested(&f(MONSTER, 12, false)));
    assert!(sight_tested(&f(MISSILE, 0, false)));
    assert!(sight_tested(&f(ITEM, 3, false)));
    assert!(!sight_tested(&f(OBJECT, 1, false)));

    // Hidden: flag-ex 0x80 cleared, not drawn.
    let mut u = UnitFacts {
        flag_ex: UNIT_EX_VISIBLE,
        sight_hidden: Some(true),
        ..f(MONSTER, 1, false)
    };
    assert!(!unit_draws(&mut u).unwrap());
    assert_eq!(u.flag_ex & UNIT_EX_VISIBLE, 0);
    // Seen: flag-ex 0x80 set, drawn (flag 0x10000000).
    u.sight_hidden = Some(false);
    assert!(unit_draws(&mut u).unwrap());
    assert_eq!(u.flag_ex & UNIT_EX_VISIBLE, UNIT_EX_VISIBLE);
    assert_eq!(u.flags & UNIT_DRAWN, UNIT_DRAWN);
    // Untested units pass without an answer; tested ones need one.
    let mut obj = f(OBJECT, 1, false);
    assert!(unit_draws(&mut obj).unwrap());
    let mut m = f(MONSTER, 1, false);
    assert!(matches!(
        unit_draws(&mut m),
        Err(OrderError::Open { question: 9, .. })
    ));
}

#[test]
fn fade_near_test() {
    let player = (10, 20);
    // x: 50 < lx < 70 with type ∈ {1, 4, 5, 7, 8, 10, 12}.
    assert!(fade_near((51, 0), player, 1));
    assert!(fade_near((69, 0), player, 12));
    assert!(!fade_near((50, 0), player, 1));
    assert!(!fade_near((70, 0), player, 1));
    assert!(!fade_near((60, 0), player, 2));
    // y: 100 < ly < 120 with type ∈ {2, 3, 6, 7, 9, 11, 12}.
    assert!(fade_near((0, 101), player, 2));
    assert!(fade_near((0, 119), player, 7));
    assert!(!fade_near((0, 119), player, 1));
    assert!(!fade_near((0, 120), player, 2));
}

#[test]
fn fade_targets_and_ramp() {
    let mut f = Fade::OPAQUE;
    retarget(&mut f, true, 1000);
    assert_eq!((f.from, f.to, f.end, f.state), (0xFF, 0x80, 1500, 3));
    // Near again: nothing changes.
    retarget(&mut f, true, 2000);
    assert_eq!(f.end, 1500);

    let mut rec = record((0, 0), 1, 1);
    rec.fade = f;
    // Halfway (now − end + 500 = 250): 255 + (−127) × 250 / 500 = 192.
    advance(
        &mut rec,
        FadeClock {
            now: 1250,
            instant: false,
        },
    );
    assert_eq!(rec.fade.alpha, 192);
    assert_eq!(rec.fade.state & 2, 2);
    // Not near, bit 0 set: back to 0xFF from alpha 192:
    // end = t + 500 × (128 − 192) / 127 = 1750 − 251.
    retarget(&mut rec.fade, false, 1250);
    assert_eq!(
        (rec.fade.from, rec.fade.to, rec.fade.end, rec.fade.state),
        (0x80, 0xFF, 1750 - 251, 2)
    );
    // End reached: alpha := to, bit 1 cleared.
    advance(
        &mut rec,
        FadeClock {
            now: 1750,
            instant: false,
        },
    );
    assert_eq!((rec.fade.alpha, rec.fade.state), (0xFF, 0));
    // A fade to 0 sets 0x400; bit 2 hides the record at its end.
    rec.fade = Fade {
        state: 2 | 4,
        alpha: 10,
        from: 10,
        to: 0,
        end: 500,
    };
    advance(
        &mut rec,
        FadeClock {
            now: 0,
            instant: true,
        },
    );
    assert_eq!(rec.fade.alpha, 0);
    assert_eq!(rec.flags & REC_FADED_OUT, REC_FADED_OUT);
    assert_eq!(rec.flags & REC_HIDDEN, 0);
    rec.fade.state = 2 | 4;
    advance(
        &mut rec,
        FadeClock {
            now: 500,
            instant: false,
        },
    );
    assert_eq!(rec.flags & REC_HIDDEN, REC_HIDDEN);
}

fn tiles(order: &FrameOrder) -> Vec<OrderedTile> {
    order
        .items
        .iter()
        .filter_map(|o| match o {
            Ordered::Tile(t) => Some(*t),
            _ => None,
        })
        .collect()
}

fn order(n: &mut NearRooms, positions: &BTreeMap<UnitKey, ClientPos>) -> FrameOrder {
    order_grid(&grid(), n, positions, CLOCK).unwrap()
}

#[test]
fn roofs_draw_by_layer_mask() {
    let mut r = room();
    for layer in 1..=4 {
        r.walls.push(record((10, 10), layer, 15));
    }
    let mut n = near(vec![r]);
    let o = order(&mut n, &BTreeMap::new());
    let mut passes: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for t in tiles(&o) {
        let TileKind::Roof { pass } = t.kind else {
            panic!("{t:?}")
        };
        let layer = n.rooms[0].walls[t.record].layer();
        passes.entry(layer).or_default().push(pass);
        assert_eq!(t.key.pass, pass::ROOFS);
        assert_eq!(
            t.key.major as usize,
            (pass as usize - 1) * 34 * 34 + grid().cell((30, 20)).unwrap()
        );
    }
    assert_eq!(passes[&1], [1]);
    assert_eq!(passes[&2], [2]);
    assert_eq!(passes[&3], [1, 2, 3]);
    assert_eq!(passes[&4], [4]);
    // Flag 0x20000 comes after the draw, not from the order.
    assert!(n.rooms[0].walls.iter().all(|w| w.flags & REC_DRAWN == 0));
}

#[test]
fn floors_by_room_and_layer() {
    let mut a = room();
    a.floors.push(record((0, 0), 2, 0));
    a.floors.push(record((1, 0), 1, 0));
    a.floors.push(record((2, 0), 3, 0)); // ℓ 3: never
    let mut hidden = record((3, 0), 1, 0);
    hidden.flags |= REC_HIDDEN;
    a.floors.push(hidden);
    let mut turned = record((4, 0), 1, 0);
    turned.dt1.orientation = 1;
    a.floors.push(turned);
    let mut b = room();
    b.floors.push(record((0, 0), 1, 0));
    let mut n = near(vec![a, b]);
    let o = order(&mut n, &BTreeMap::new());
    let got: Vec<_> = tiles(&o)
        .iter()
        .map(|t| (t.room, t.record, t.key.major, t.key.minor))
        .collect();
    assert_eq!(got, [(0, 1, 0, 1), (0, 0, 1, 0), (1, 0, 2, 0)]);
    assert!(tiles(&o).iter().all(|t| t.key.pass == pass::FLOORS));
}

#[test]
fn lower_walls_and_skips() {
    let mut r = room();
    r.walls.push(record((10, 10), 2, 16));
    let mut faded = record((10, 10), 1, 17);
    faded.flags |= REC_FADED_OUT;
    r.walls.push(faded);
    r.walls.push(record((10, 10), 1, 18));
    let mut n = near(vec![r]);
    let o = order(&mut n, &BTreeMap::new());
    let got: Vec<_> = tiles(&o)
        .iter()
        .map(|t| (t.record, t.kind, t.key.pass, t.key.minor))
        .collect();
    // [ℓ2 (0)] + ℓ1 (1) appends; ℓ1 (2) goes before ℓ2: list
    // [2, 0, 1], and record 1 is skipped (0x400).
    assert_eq!(
        got,
        [
            (2, TileKind::LowerWall, pass::LOWER_WALLS, 0),
            (0, TileKind::LowerWall, pass::LOWER_WALLS, 1)
        ]
    );
}

#[test]
fn shadow_pass_then_walls_and_units() {
    let mut r = room();
    r.walls.push(record((11, 7), 1, 1)); // wall in cell 587
    r.walls.push(record((11, 7), 1, 2)); // second wall in cell 587
    r.shadows.push(record((11, 7), 0, 0)); // shadow tile in cell 587
    r.units.push(unit(1, MONSTER, 1));
    r.units.push(unit(2, MONSTER, 12)); // flat
    r.units.push(unit(3, OBJECT, 1));
    let mut n = near(vec![r]);
    let ci = grid().cell((31, 17)).unwrap();
    let positions: BTreeMap<_, _> = n.rooms[0]
        .units
        .iter()
        .map(|u| (u.key, pos(1120, 1960)))
        .collect();
    assert_eq!(grid().cell(tile_of(1120, 1960)), Some(ci));
    let o = order(&mut n, &positions);
    let at = |pass, minor| OrderKey {
        pass,
        major: ci as u32,
        minor,
    };
    let got: Vec<_> = o
        .items
        .iter()
        .map(|i| match i {
            Ordered::Tile(t) => (format!("tile {:?} {}", t.kind, t.record), t.key),
            Ordered::Unit { key, at } => (format!("unit {}", key.guid), *at),
            Ordered::UnitShadow { key, at } => (format!("shadow {}", key.guid), *at),
        })
        .collect();
    let want = vec![
        ("tile ShadowTile 0".to_string(), at(pass::SHADOWS, 0)),
        ("unit 2".to_string(), at(pass::SHADOWS, 1)),
        ("tile Wall 0".to_string(), at(pass::WALLS_UNITS, 0)),
        ("tile Wall 1".to_string(), at(pass::WALLS_UNITS, 1)),
        ("unit 1".to_string(), at(pass::WALLS_UNITS, 2)),
        ("unit 3".to_string(), at(pass::WALLS_UNITS, 3)),
    ];
    assert_eq!(got, want);
    // Drawn: flag-ex 0x80 set, so the next frame files the shadows.
    assert!(n.rooms[0].units[0].facts.flag_ex & UNIT_EX_VISIBLE != 0);
    let o = order(&mut n, &positions);
    let shadows: Vec<_> = o
        .items
        .iter()
        .filter_map(|i| match i {
            Ordered::UnitShadow { key, at } => Some((key.guid, at.minor)),
            _ => None,
        })
        .collect();
    assert_eq!(shadows, [(1, 1), (3, 3)]);
    assert_eq!(
        o.unit_slot(UnitKey {
            unit_type: MONSTER,
            guid: 1
        }),
        UnitSlot::Drawn(at(pass::WALLS_UNITS, 2))
    );
    assert_eq!(
        o.unit_slot(UnitKey {
            unit_type: MONSTER,
            guid: 9
        }),
        UnitSlot::NotDrawn
    );
}

#[test]
fn type_15_never_reaches_the_wall_list() {
    // A type-15 record goes to the roof list, so the fatal check
    // is reached only through a hand-built list.
    let mut r = room();
    r.walls.push(record((10, 10), 1, 15));
    let mut n = near(vec![r]);
    let o = order(&mut n, &BTreeMap::new());
    assert!(tiles(&o)
        .iter()
        .all(|t| matches!(t.kind, TileKind::Roof { .. })));
}

#[test]
fn wall_fade_runs_through_the_frame() {
    let mut r = room();
    // Absolute subtile (100 + 50, 50 + 50) = (150, 100): near a player at
    // tile (29, 0) on x (145 < 150 < 165), type 1.
    r.walls.push(record((10, 10), 1, 1));
    let mut n = near(vec![r]);
    n.player_tile = (29, 0);
    // The unused geometric fade branch.
    n.level.fade_geometric = true;
    let o = order(&mut n, &BTreeMap::new());
    // t = 10 500: end = t (alpha 0xFF); the ramp starts at now = end − 500.
    let w = &n.rooms[0].walls[0];
    assert_eq!((w.fade.state, w.fade.end), (3, 10_500));
    assert_eq!(w.fade.alpha, 0xFF);
    assert_eq!(tiles(&o)[0].alpha, 0xFF);
    // The draw gets the state the filing left: the faded light points from
    // the first drawn frame on.
    assert_eq!(tiles(&o)[0].fade_state, 3);
    // Record flag 0x4: no target update.
    n.rooms[0].walls[0] = record((10, 10), 1, 1);
    n.rooms[0].walls[0].flags |= REC_NO_FADE;
    order(&mut n, &BTreeMap::new());
    assert_eq!(n.rooms[0].walls[0].fade.state, 0);
}

#[test]
fn open_mode_3_and_level_backgrounds() {
    let cam = Camera::new(
        FrameSize::D2RUST,
        OpenMode::new(3).unwrap(),
        pos(1000, 2000),
        (0, 0),
    );
    let mut r = room();
    r.walls.push(record((10, 10), 1, 1));
    let mut n = near(vec![r]);
    let o = order_frame(
        &cam,
        OpenMode::new(3).unwrap(),
        &mut n,
        &BTreeMap::new(),
        CLOCK,
    )
    .unwrap();
    assert!(o.items.is_empty());
    // Levels 74 and 120: the backgrounds are the world view's pass-1 layer
    // (`world_view::background_view`): the rooms order as usual.
    n.level.id = 74;
    assert!(order_grid(&grid(), &mut n, &BTreeMap::new(), CLOCK).is_ok());
    n.level.id = 120;
    assert!(order_grid(&grid(), &mut n, &BTreeMap::new(), CLOCK).is_ok());
    n.level.id = 1;
    n.level.draw_edges = true;
    // Edge floors are the camera's part
    // (`order_frame`): the grid order draws the rooms alone.
    assert!(order_grid(&grid(), &mut n, &BTreeMap::new(), CLOCK).is_ok());
}

/// A `DrawEdges` level: one drawn floor at the camera's tile (31, 18),
/// sub-tile extents (155, 90)–(155, 90), the player at sub-tile (160, 95).
fn edge_level() -> NearRooms {
    let mut r = room();
    r.floors.push(record((11, 8), 1, 0));
    let mut n = near(vec![r]);
    n.level.draw_edges = true;
    n.player_subtile = (160, 95);
    n.edge = Some(record((0, 0), 0, 0));
    n
}

fn edge_items(o: &FrameOrder) -> Vec<OrderedTile> {
    tiles(o)
        .into_iter()
        .filter(|t| t.array == TileArray::Edge)
        .collect()
}

#[test]
fn edge_floors_follow_the_last_room_at_open_mode_0() {
    let cam = camera();
    let mut n = edge_level();
    let o = order_frame(&cam, OpenMode::NONE, &mut n, &BTreeMap::new(), CLOCK).unwrap();
    let e = edge_items(&o);
    // px − min x = 5 < 30: the first strip starts at (155 − 5, 95), step
    // (0, −5): sub-tiles (150, 95), (150, 90), (150, 85).
    let cells: Vec<_> = e.iter().take(3).map(|t| t.cell).collect();
    assert_eq!(cells, vec![(30, 19), (30, 18), (30, 17)]);
    for (k, t) in e.iter().enumerate() {
        assert_eq!(t.key, edges::edge_key(1, k));
        assert_eq!(t.kind, TileKind::Floor { layer: 1 });
    }
    // Floor items keep pass order: the room's floor, then the edges.
    let floors: Vec<_> = tiles(&o)
        .into_iter()
        .filter(|t| t.key.pass == pass::FLOORS)
        .map(|t| t.array)
        .collect();
    assert_eq!(floors[0], TileArray::Floor);
    assert!(floors[1..].iter().all(|a| *a == TileArray::Edge));
    assert_eq!(n.edge.unwrap().flags & REC_DRAWN, REC_DRAWN);
    // Another open mode: no edge floors.
    let mut n = edge_level();
    let o = order_frame(
        &cam,
        OpenMode::new(1).unwrap(),
        &mut n,
        &BTreeMap::new(),
        CLOCK,
    )
    .unwrap();
    assert!(edge_items(&o).is_empty());
    // Resolution mode 0 (640 × 480): none either.
    let low = Camera::new(FrameSize::LOW, OpenMode::NONE, pos(1000, 2000), (0, 0));
    let mut n = edge_level();
    let o = order_frame(&low, OpenMode::NONE, &mut n, &BTreeMap::new(), CLOCK).unwrap();
    assert!(edge_items(&o).is_empty());
    // Acts IV and V: the record holds no tile.
    let mut n = edge_level();
    n.edge = None;
    let e = order_frame(&cam, OpenMode::NONE, &mut n, &BTreeMap::new(), CLOCK).unwrap_err();
    assert_eq!(e, OrderError::Edge(edges::EdgeError::NoEdgeRecord));
}

// ------------------------------------------------------------- wiring

fn tile_key(name: &str) -> FrameSetKey {
    FrameSetKey::new(format!("data/global/tiles/{name}.dt1"), FramePart::Tile(0)).unwrap()
}

fn art(t: &OrderedTile) -> Result<TileArt, ViewError> {
    Ok(TileArt {
        frame: ComponentFrame {
            set: tile_key("t"),
            index: t.record,
        },
        blocks: Vec::new(),
        shade: ShadeChain::EMPTY,
        blend: BlendOp::Opaque,
    })
}

fn camera() -> Camera {
    Camera::new(FrameSize::D2RUST, OpenMode::NONE, pos(1000, 2000), (0, 0))
}

#[test]
fn map_tiles_carry_kind_list_and_key() {
    let mut r = room();
    r.walls.push(record((11, 7), 1, 1));
    r.walls.push(record((11, 7), 1, 15));
    r.walls.push(record((11, 7), 1, 16));
    r.walls[1].dt1.roof_height = 40;
    r.shadows.push(record((11, 7), 0, 0));
    r.floors.push(record((11, 7), 1, 0));
    let mut n = near(vec![r]);
    let o = order_frame(&camera(), OpenMode::NONE, &mut n, &BTreeMap::new(), CLOCK).unwrap();
    let (tiles, units) = resolve(&camera(), &o, art).unwrap();
    assert!(units.is_empty());
    let got: Vec<_> = tiles.iter().map(|t| (t.list, t.key.pass())).collect();
    assert_eq!(
        got,
        [
            (TileList::Wall, pass::LOWER_WALLS),
            (TileList::Floor, pass::FLOORS),
            (TileList::Wall, pass::SHADOWS),
            (TileList::Wall, pass::WALLS_UNITS),
            (TileList::Roof { roof_height: 40 }, pass::ROOFS),
        ]
    );
    let keys: Vec<_> = tiles.iter().map(|t| t.key).collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "build order is key order");
    let lists: Vec<_> = self::tiles(&o).iter().map(placement_list).collect();
    assert_eq!(lists, got.iter().map(|g| g.0).collect::<Vec<_>>());
}

#[test]
fn unresolved_items_fail_the_frame() {
    // A drawn water floor is reported with its handed position
    // (the water floor runs in the feed's weather state).
    let mut r = room();
    let mut water = record((11, 7), 1, 0);
    water.dt1.material = 0x2;
    r.floors.push(water);
    let mut n = near(vec![r]);
    let o = order_frame(&camera(), OpenMode::NONE, &mut n, &BTreeMap::new(), CLOCK).unwrap();
    let (_, _, fx) = resolve_drawn(&camera(), &o, art).unwrap();
    let handed = camera().tile_handed(TileList::Floor, 31, 17);
    assert_eq!(fx.water, [handed]);
    assert_eq!(fx.drawn, [(0, TileArray::Floor, 0)]);
    // A culled one is not drawn, so no effect starts.
    n.rooms[0].floors[0].tile = (-500, 0);
    let o = order_frame(&camera(), OpenMode::NONE, &mut n, &BTreeMap::new(), CLOCK).unwrap();
    let (tiles, _, fx) = resolve_drawn(&camera(), &o, art).unwrap();
    assert_eq!(tiles.len(), 1);
    assert!(fx.water.is_empty() && fx.drawn.is_empty());
    // A unit shadow: its shadow pass slot is recorded.
    let slot = OrderKey {
        pass: 5,
        major: 0,
        minor: 0,
    };
    let key = UnitKey {
        unit_type: 1,
        guid: 1,
    };
    let o = FrameOrder {
        items: vec![Ordered::UnitShadow { key, at: slot }],
        count: 1,
    };
    let (_, units, fx) = resolve_drawn(&camera(), &o, art).unwrap();
    assert!(units.is_empty());
    assert_eq!(fx.shadows.get(&key), Some(&slot));
}

/// A feed with near rooms; every unit stands at subtile (187, 62).
struct MapFeed {
    near: NearRooms,
    seed: Seed,
    weather: Option<(Weather, FloorContext)>,
    sky: Option<SkyFrame>,
}

impl ViewSource for MapFeed {
    fn unit_position(&self, _: &ClientUnit) -> Result<UnitPosition, String> {
        Ok(UnitPosition::Static { sx: 187, sy: 62 })
    }

    fn unit_offset(&self, _: &ClientUnit, _: &UnitPose) -> Result<(i32, i32), String> {
        Ok((0, 0))
    }

    fn map_tiles(&self, _: &ClientWorld, _: &ViewAssets) -> Result<Vec<MapTile>, ViewError> {
        Err(ViewError::unresolved(
            "map tiles",
            "not consulted with near rooms",
        ))
    }
}

impl ViewFeed for MapFeed {
    fn player(&self, _: &ClientWorld) -> Result<Option<UnitPosition>, ViewError> {
        Ok(Some(UnitPosition::Static { sx: 187, sy: 62 }))
    }

    fn open_mode(&self, _: &ClientWorld) -> Result<OpenMode, ViewError> {
        Ok(OpenMode::NONE)
    }

    fn shake(&self, _: &ClientWorld) -> Result<Option<RunningShake>, ViewError> {
        Ok(None)
    }

    fn player_seed(&mut self, _: &ClientWorld) -> Result<&mut Seed, ViewError> {
        Ok(&mut self.seed)
    }

    /// Every live `Levels.txt` row has BlankScreen 1; the order tests do
    /// not depend on it.
    fn blank_screen(&self, _: &ClientWorld) -> Result<bool, ViewError> {
        Ok(true)
    }

    fn near_rooms(&mut self, _: &ClientWorld) -> Result<Option<&mut NearRooms>, ViewError> {
        Ok(Some(&mut self.near))
    }

    fn fade_clock(&self, _: &ClientWorld) -> Result<FadeClock, ViewError> {
        Ok(CLOCK)
    }

    fn tile_art(&self, t: &OrderedTile, _: &ViewAssets) -> Result<TileArt, ViewError> {
        art(t)
    }

    fn weather_frame(&mut self, _: &ClientWorld) -> Result<Option<WeatherFrame<'_>>, ViewError> {
        Ok(self.weather.as_mut().map(|(weather, floors)| WeatherFrame {
            weather,
            floors,
            seed: &mut self.seed,
            update_count: 100,
            mud: false,
            sky: self.sky,
            thunder: None,
            commit: None,
        }))
    }
}

fn palette() -> Palette {
    Palette {
        colors: [Rgb::default(); 256],
    }
}

#[test]
fn ordered_source_wraps_the_feed() {
    // Static (187, 62) → client ((187 − 62) × 16, (187 + 62) × 8) = (2000, 1992).
    let at = UnitPosition::Static { sx: 187, sy: 62 }.client();
    assert_eq!((at.x, at.y), (2000, 1992));
    let mut world = ClientWorld::default();
    let player = UnitKey {
        unit_type: PLAYER,
        guid: 1,
    };
    let stray = UnitKey {
        unit_type: MONSTER,
        guid: 2,
    };
    for key in [player, stray] {
        world.units.insert(key, ClientUnit::new(key));
    }
    // The wall at the player's tile (37, 12) = room origin + (17, 2).
    assert_eq!(tile_of(2000, 1992), (37, 12));
    let mut r = room();
    r.walls.push(record((17, 2), 1, 1));
    let mut p = unit(1, PLAYER, 1);
    p.facts.local = true;
    r.units.push(p);
    let mut feed = MapFeed {
        near: near(vec![r]),
        seed: Seed::new(1, 0),
        weather: None,
        sky: None,
    };
    let cam = Camera::new(FrameSize::D2RUST, OpenMode::NONE, at, (0, 0));
    let assets = ViewAssets::new(palette());
    let source = ordered_source(&world, &cam, OpenMode::NONE, &mut feed, &assets)
        .unwrap()
        .unwrap();
    let tiles = source.map_tiles(&world, &assets).unwrap();
    assert_eq!(tiles.len(), 1);
    assert_eq!(tiles[0].key.pass(), pass::WALLS_UNITS);
    assert_eq!(tiles[0].cell, (37, 12));
    let slot = source.unit_slot(&ClientUnit::new(player));
    let UnitSlot::Drawn(k) = slot else {
        panic!("{slot:?}")
    };
    assert_eq!(k.pass, pass::WALLS_UNITS);
    assert_eq!(
        source.unit_slot(&ClientUnit::new(stray)),
        UnitSlot::NotDrawn
    );
    // The frame's writes land in the feed.
    assert!(feed.near.rooms[0].units[0].facts.flags & UNIT_DRAWN != 0);

    // A room unit the world does not hold is an error.
    feed.near.rooms[0].units.push(unit(7, MONSTER, 1));
    assert!(ordered_source(&world, &cam, OpenMode::NONE, &mut feed, &assets).is_err());
}

// ------------------------------------------------- implementation answers

/// A room whose wall array fills the pool with 3,000 walls of tile
/// (10, 10) (no fade: record flag 0x4).
fn full_pool_room() -> Room {
    let mut r = room();
    for _ in 0..POOL {
        let mut w = record((10, 10), 1, 1);
        w.flags |= REC_NO_FADE;
        r.walls.push(w);
    }
    r
}

#[test]
fn pool_overflow_sets_no_grid_flag() {
    // Pool full, then one roof (type 15) and one shadow-array record:
    // count 3,002, the roof and shadow grid flags stay clear, nothing filed.
    let mut r = full_pool_room();
    r.walls.push(record((10, 10), 1, 15));
    r.shadows.push(record((10, 10), 0, 0));
    let mut n = near(vec![r]);
    let lists = fill(&grid(), &mut n, &BTreeMap::new(), CLOCK, false).unwrap();
    assert_eq!(lists.count, POOL + 2);
    assert_eq!(lists.flags, GridFlags::default());
    let cell = &lists.cells[grid().cell((30, 20)).unwrap()];
    assert!(cell.roof.is_empty() && cell.shadow.is_empty());
    assert_eq!(cell.wall.len(), POOL);
}

#[test]
fn pool_dropped_wall_sets_flag_4_but_no_fade_target() {
    // Pool full, then a wall (type 1, record flag 0x4 clear) of another
    // logical room in front of the player: cell flag 4 set, its fade
    // bytes unchanged, not filed.
    let mut r = full_pool_room();
    let mut w = record((10, 10), 1, 1);
    w.logical = Some(Logical {
        x0: 40,
        y0: 40,
        index: 2,
    });
    r.walls.push(w);
    let mut n = near(vec![r]);
    n.player_logical = 1;
    let lists = fill(&grid(), &mut n, &BTreeMap::new(), CLOCK, false).unwrap();
    let cell = &lists.cells[grid().cell((30, 20)).unwrap()];
    assert_eq!(cell.flags, 4);
    assert_eq!(cell.wall.len(), POOL);
    assert_eq!(n.rooms[0].walls[POOL].fade, Fade::OPAQUE);
    // With room in the pool the same wall gets its target.
    let mut r = room();
    r.walls.push(w);
    let mut n = near(vec![r]);
    n.player_logical = 1;
    fill(&grid(), &mut n, &BTreeMap::new(), CLOCK, false).unwrap();
    assert_eq!(n.rooms[0].walls[0].fade.state, 3);
}

#[test]
fn group_mode_near() {
    let g = Some(Logical {
        x0: 30,
        y0: 20,
        index: 5,
    });
    // Player tile before the corner on x or y, other index: near.
    assert!(fade_near_group(g, 4, (29, 25)));
    assert!(fade_near_group(g, 4, (35, 19)));
    // Inside or past the corner on both axes: not near.
    assert!(!fade_near_group(g, 4, (30, 20)));
    assert!(!fade_near_group(g, 4, (40, 30)));
    // The player's own logical room never fades.
    assert!(!fade_near_group(g, 5, (0, 0)));
    // No coordinate record (outdoor levels, presets without Logicals,
    // the towns): never near.
    assert!(!fade_near_group(None, 4, (0, 0)));
    assert!(!fade_near_group(None, 0, (0, 0)));
}

#[test]
fn group_mode_is_the_default_and_only_logicals_walls_fade() {
    // The game runs in group mode, so LevelFacts::default() is group mode.
    assert!(!LevelFacts::default().fade_geometric);
    let mut r = room();
    // A wall the geometric branch would call near (see
    // `wall_fade_runs_through_the_frame`) but with no coordinate record.
    r.walls.push(record((10, 10), 1, 1));
    let mut logical = record((11, 10), 1, 1);
    logical.logical = Some(Logical {
        x0: 31,
        y0: 0,
        index: 3,
    });
    r.walls.push(logical);
    let mut n = near(vec![r]);
    n.player_tile = (29, 0);
    n.player_logical = 2;
    order(&mut n, &BTreeMap::new());
    assert_eq!(n.rooms[0].walls[0].fade.state, 0);
    assert_eq!(n.rooms[0].walls[1].fade.state, 3);
    // The player walks into that logical room: the fade reverses.
    n.player_logical = 3;
    order(&mut n, &BTreeMap::new());
    assert_eq!(n.rooms[0].walls[1].fade.state & 1, 0);
    assert_eq!(n.rooms[0].walls[1].fade.to, 0xFF);
}

#[test]
fn fade_clock_wraps_unsigned() {
    let mut rec = record((0, 0), 1, 1);
    let running = Fade {
        state: 3,
        alpha: 0xFF,
        from: 0xFF,
        to: 0x80,
        end: 0xFFFF_FF00,
    };
    rec.fade = running;
    advance(
        &mut rec,
        FadeClock {
            now: 0x10,
            instant: false,
        },
    );
    // end ≤ now is false unsigned; (−127 × 772) / 500 = −196 → 0x3C.
    assert_eq!(rec.fade.alpha, 0x3B);
    assert_eq!(rec.fade.state & 2, 2);
    // Render kind < 4: the ramp completes at once.
    rec.fade = running;
    advance(
        &mut rec,
        FadeClock {
            now: 0x10,
            instant: true,
        },
    );
    assert_eq!((rec.fade.alpha, rec.fade.state & 2), (0x80, 0));
}

fn ordered(kind: TileKind, cell: (i32, i32)) -> OrderedTile {
    OrderedTile {
        room: 0,
        array: TileArray::Wall,
        record: 0,
        kind,
        cell,
        dt1: Dt1Facts::default(),
        alpha: 0xFF,
        fade_state: 0,
        key: OrderKey {
            pass: 0,
            major: 0,
            minor: 0,
        },
    }
}

#[test]
fn drawn_flag_after_the_drawer() {
    let cam = camera();
    // A tile cell whose wall position lands on screen.
    let (tx, ty) = tile_of(1000, 2000);
    let wall = ordered(TileKind::Wall, (tx, ty));
    let handed = cam.tile_handed(TileList::Wall, tx, ty);
    let on = BlockRect {
        x: 0,
        y: 0,
        width: 32,
        height: 32,
    };
    let off = BlockRect {
        x: -handed.0 - 5_000,
        ..on
    };
    // Lit wall, every block culled by the block test: not set.
    assert!(!sets_drawn_flag(&cam, &wall, &[off]));
    assert!(!sets_drawn_flag(&cam, &wall, &[]));
    assert!(sets_drawn_flag(&cam, &wall, &[off, on]));
    let lower = ordered(TileKind::LowerWall, (tx, ty));
    assert!(sets_drawn_flag(&cam, &lower, &[on]));
    // Roof passing the whole-tile test, all blocks off screen: set.
    let roof = ordered(TileKind::Roof { pass: 1 }, (tx, ty));
    assert!(sets_drawn_flag(&cam, &roof, &[off]));
    // Floor ℓ 1 passing the whole-tile test: set; far away: not.
    assert!(sets_drawn_flag(
        &cam,
        &ordered(TileKind::Floor { layer: 1 }, (tx, ty)),
        &[]
    ));
    assert!(!sets_drawn_flag(
        &cam,
        &ordered(TileKind::Floor { layer: 1 }, (tx - 50, ty)),
        &[]
    ));
    // Shadow tiles never set it.
    assert!(!sets_drawn_flag(
        &cam,
        &ordered(TileKind::ShadowTile, (tx, ty)),
        &[on]
    ));

    let mut r = room();
    r.walls.push(record((0, 0), 1, 1));
    r.floors.push(record((0, 0), 1, 0));
    let mut n = near(vec![r]);
    mark_drawn(&mut n, &[(0, TileArray::Floor, 0)]);
    assert_eq!(n.rooms[0].floors[0].flags & REC_DRAWN, REC_DRAWN);
    assert_eq!(n.rooms[0].walls[0].flags & REC_DRAWN, 0);
}

#[test]
fn automap_reveal_distance_and_records() {
    let mut r = room();
    r.level = 1;
    let mut drawn = record((0, 0), 1, 1);
    drawn.flags |= REC_DRAWN;
    let mut hidden = drawn;
    hidden.flags |= REC_HIDDEN;
    r.walls.push(drawn);
    r.walls.push(hidden);
    r.walls.push(record((0, 0), 1, 1));
    r.floors.push(drawn);
    let mut other = r.clone();
    other.level = 2;
    let n = near(vec![r, other]);
    // Levels 1 and 2 on distinct leveldefs `Layer`s.
    let own = |level: u32| level;

    let mut a = AutomapReveal {
        countdown: 0,
        last: (1000, 2000),
    };
    // d = 45 / 60: no reveal; the last position stays.
    assert!(a.frame((1040, 2010), 1, &n, own).is_empty());
    assert!(a.frame((1060, 2000), 1, &n, own).is_empty());
    assert_eq!(a.last, (1000, 2000));
    // d = 80 = 0x50: floors then walls of the player level's rooms,
    // drawn and not hidden.
    assert_eq!(
        a.frame((1080, 2000), 1, &n, own),
        [(0, TileArray::Floor, 0), (0, TileArray::Wall, 0)]
    );
    assert_eq!(a.last, (1080, 2000));
    // A running countdown is decremented instead.
    let mut c = AutomapReveal {
        countdown: 2,
        last: (0, 0),
    };
    assert!(c.frame((5000, 0), 1, &n, own).is_empty());
    assert_eq!((c.countdown, c.last), (1, (0, 0)));
    // The AutoMap preset path reveals every record.
    let mut all = Vec::new();
    reveal_room(0, &n.rooms[0], true, &mut all);
    assert_eq!(all.len(), 4);
}

#[test]
fn automap_reveal_takes_rooms_of_every_level_on_the_player_layer() {
    // Rooms whose level has the same leveldefs `Layer` as the
    // player's level are revealed, not only the player level's rooms.
    let mut r = room();
    r.level = 1;
    let mut drawn = record((0, 0), 1, 1);
    drawn.flags |= REC_DRAWN;
    r.walls.push(drawn);
    let mut same = r.clone();
    same.level = 2;
    let mut apart = r.clone();
    apart.level = 3;
    let n = near(vec![r, same, apart]);
    let layer = |level: u32| if level == 3 { 9 } else { 4 };
    let mut a = AutomapReveal {
        countdown: 0,
        last: (0, 0),
    };
    assert_eq!(
        a.frame((80, 0), 1, &n, layer),
        [(0, TileArray::Wall, 0), (1, TileArray::Wall, 0)]
    );
}

#[test]
fn water_floor_draws_the_player_seed_through_the_feed() {
    let world = ClientWorld::default();
    let at = UnitPosition::Static { sx: 187, sy: 62 }.client();
    let (tx, ty) = tile_of(at.x, at.y);
    let mut r = room();
    let mut water = record((tx - 20, ty - 10), 1, 0);
    water.dt1.material = 0x2;
    r.floors.push(water);
    let cam = Camera::new(FrameSize::D2RUST, OpenMode::NONE, at, (0, 0));
    let assets = ViewAssets::new(palette());
    // No weather state: the frame fails, never skips the draw.
    let mut feed = MapFeed {
        near: near(vec![r.clone()]),
        seed: Seed::new(1, 0),
        weather: None,
        sky: None,
    };
    let e = ordered_source(&world, &cam, OpenMode::NONE, &mut feed, &assets)
        .err()
        .unwrap();
    assert!(e.to_string().contains("water floor"), "{e}");
    // Rain off (intensity 0, no mud): one roll_range(0, 1000), no spawn.
    let mut feed = MapFeed {
        near: near(vec![r]),
        seed: Seed::new(1, 0),
        weather: Some((Weather::new(), FloorContext::default())),
        sky: None,
    };
    assert!(
        ordered_source(&world, &cam, OpenMode::NONE, &mut feed, &assets)
            .unwrap()
            .is_some()
    );
    let mut want = Seed::new(1, 0);
    want.roll_range(0, 1_000);
    assert_eq!(feed.seed, want);
    let (w, _) = feed.weather.as_ref().unwrap();
    assert_eq!((w.splashes().live(), w.bubbles().live()), (0, 0));
    // The drawn floor got flag 0x20000 after its draw.
    assert_eq!(feed.near.rooms[0].floors[0].flags & REC_DRAWN, REC_DRAWN);
}

#[test]
fn ordered_source_runs_passes_4_and_9() {
    let world = ClientWorld::default();
    let at = UnitPosition::Static { sx: 187, sy: 62 }.client();
    let cam = Camera::new(FrameSize::D2RUST, OpenMode::NONE, at, (0, 0));
    let assets = ViewAssets::new(palette());
    let level = LevelWeather {
        level_id: 2,
        act: 0,
        rain: true,
        mud: false,
    };
    let sky = SkyFrame {
        frame: FrameSize::LOW,
        mode: OpenMode::new(1).unwrap(),
        shift_x: 0,
        level,
        frame_rate: 25,
        low_quality: false,
    };
    let mut weather = Weather::new();
    // A splash at (100, 200) frame 1 kind 2, and a lightning strike.
    weather.pools_mut().1.alloc(PoolRecord {
        x: 100,
        y: 200,
        kind: 2,
        frame: 1,
        countdown: 2,
    });
    weather.start_lightning(false);
    let mut feed = MapFeed {
        near: near(vec![room()]),
        seed: Seed::new(1, 0),
        weather: Some((weather, FloorContext::default())),
        sky: Some(sky),
    };
    let source = ordered_source(&world, &cam, OpenMode::NONE, &mut feed, &assets)
        .unwrap()
        .unwrap();
    // The camera's frame and mode replace the feed's: pass 4 keeps the
    // record, pass 9 flashes over the camera's span (mode 0: 0 … 800).
    assert_eq!(source.sky.pools.len(), 1);
    let d = source.sky.pools[0];
    assert_eq!(
        (d.pool, d.kind, d.frame, d.x, d.y),
        (EnvPool::Splashes, 2, 1, 100, 200)
    );
    assert_eq!(
        source.sky.sky,
        [SkyDraw::Flash {
            x0: 0,
            y0: 0,
            x1: 800,
            y1: 553
        }]
    );
    // Without a sky frame the live pools fail the frame, never dropped.
    let (w, _) = feed.weather.as_mut().unwrap();
    w.start_lightning(true);
    feed.sky = None;
    assert!(ordered_source(&world, &cam, OpenMode::NONE, &mut feed, &assets).is_err());
}

/// A synthetic room: enter behind a multi-piece wall, stand, walk along
/// it at 60 Hz, then leave. Every piece changes target once per crossing.
#[test]
fn room_wall_fade_is_stable_standing_and_walking() {
    for instant in [false, true] {
        let mut r = room();
        for x in 9..12 {
            let mut wall = record((x, 10), 1, 1);
            wall.logical = Some(Logical {
                x0: 30,
                y0: 20,
                index: 5,
            });
            r.walls.push(wall);
        }
        r.walls.push(record((10, 10), 1, 15));
        let mut n = near(vec![r]);
        let mut changes = [0; 3];
        let mut last_target = [false; 3];
        let mut last_alpha = [255; 3];
        for frame in 0..180 {
            // The player is behind the wall for frames 1..120; the first
            // half is standing, the second walking parallel to the wall.
            let behind = (1..120).contains(&frame);
            let sx = if frame < 60 {
                127
            } else {
                127 + (frame - 60).min(59) / 3
            };
            n.player_tile = (sx / 5, 25);
            n.player_subtile = (sx, 127);
            n.player_logical = if behind { 4 } else { 5 };
            let clock = FadeClock {
                now: 10_000 + frame as u32 * 1000 / 60,
                instant,
            };
            order_grid(&grid(), &mut n, &BTreeMap::new(), clock).unwrap();
            for (i, wall) in n.rooms[0].walls[..3].iter().enumerate() {
                let target = wall.fade.state & 1 != 0;
                changes[i] += usize::from(target != last_target[i]);
                assert_eq!(target, behind);
                if behind {
                    assert!(wall.fade.alpha <= last_alpha[i]);
                } else {
                    assert!(wall.fade.alpha >= last_alpha[i]);
                }
                last_target[i] = target;
                last_alpha[i] = wall.fade.alpha;
            }
            assert_eq!(n.rooms[0].walls[3].fade, Fade::OPAQUE, "roof stays solid");
        }
        assert_eq!(changes, [2; 3]);
        assert_eq!(last_alpha, [255; 3]);
    }
}

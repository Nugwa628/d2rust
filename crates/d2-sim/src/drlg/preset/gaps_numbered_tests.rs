//! Gap tests: the numbered edge cases and bugs of the preset rules
//! not yet claimed by other tests, one test per rule.

#[allow(unused_imports)]
use super::*;

use super::tests::{cell, ds1, preset_data, World};
use crate::drlg::{DrlgRoomId, SpawnTile};

/// A DS1 object record (flags 0).
fn obj(kind: u32, id: u32, x: u32, y: u32) -> Ds1ObjectInput {
    Ds1ObjectInput {
        kind,
        id,
        x,
        y,
        flags: 0,
    }
}

/// Orientation `o` and wall value `v` at DS1 tile (x, y), wall layer 0.
fn set(f: &mut Ds1Input, x: u32, y: u32, o: u32, v: u32) {
    let i = (y * (f.width + 1) + x) as usize;
    f.orientations[0][i] = o;
    f.walls[0][i] = v;
}

/// Level 33 (Def 210) of the DS1's size at map origin (40, 24),
/// generated with the given `Scan` and `Pops`.
fn scanned(f: Ds1Input, scan: u32, pops: u32) -> (World, LevelIdx) {
    let mut w = World::new();
    let l = w.level(33, 210, f.width as i32, f.height as i32);
    w.drlg.level_mut(l).rect = TileRect::new(40, 24, f.width as i32, f.height as i32);
    w.pd.defs[210].scan = scan;
    w.pd.defs[210].pops = pops;
    w.file(210, b"s.ds1", f);
    w.init(l);
    w.generate(l).unwrap();
    (w, l)
}

fn room_at(w: &World, l: LevelIdx, x: i32, y: i32) -> DrlgRoomId {
    *w.drlg
        .level_rooms(l)
        .iter()
        .find(|&&r| w.drlg.room(r).rect.contains(x, y))
        .unwrap()
}

#[test]
fn pop_style_seen_once_spans_from_ds1_origin() {
    let mut f = ds1(16, 16);
    set(&mut f, 5, 6, 10, cell(9, 3));
    let (w, l) = scanned(f, 0, 1);
    let m = w.p.level_maps(l)[0];
    let pops = &w.p.map(m).unwrap().pops;
    assert_eq!(pops.len(), 1);
    // Corner 2 stays (0, 0): the rectangle runs from the DS1 origin (map
    // origin 40, 24) to the marker, not a 1×1 rectangle at (5, 6).
    assert_eq!(
        (pops[0].group, pops[0].sub, pops[0].rect),
        (1, 3, TileRect::new(40, 24, 6, 7))
    );
}

#[test]
fn tile_info_and_pops_have_no_capacity_check() {
    let mut f = ds1(32, 16);
    // Five distinct pop styles with `Pops` = 1: every one gets an entry.
    for (k, style) in [8u32, 12, 16, 20, 24].into_iter().enumerate() {
        set(&mut f, 2 + 2 * k as u32, 2, 10, cell(style, 0));
    }
    let (w, l) = scanned(f, 0, 1);
    let m = w.p.level_maps(l)[0];
    let groups: Vec<_> = w.p.map(m).unwrap().pops.iter().map(|p| p.group).collect();
    assert_eq!(groups, [1, 2, 3, 4, 5]);

    // Tile info: every marker of every row is appended.
    let mut f = ds1(32, 16);
    for y in 0..16 {
        for x in 0..32 {
            set(&mut f, x, y, 10, cell(32, 0));
        }
    }
    let (w, l) = scanned(f, 1, 0);
    let tiles = &w.drlg.level(l).spawn_tiles;
    assert_eq!(tiles.len(), 32 * 16);
    assert_eq!(
        tiles[tiles.len() - 1],
        SpawnTile {
            x: 40 + 31,
            y: 24 + 15,
            index: 10
        }
    );
}

#[test]
fn units_outside_every_room_are_never_transferred() {
    let mut w = World::new();
    let l = w.level(20, 202, 8, 8);
    w.pd.defs[202].scan = 1;
    let mut f = ds1(8, 8);
    // Inside (sub-tile 10, 10) and outside (x = 5·W and far away).
    f.objects = vec![obj(2, 151, 10, 10), obj(2, 152, 40, 0), obj(2, 153, 99, 99)];
    w.file(202, b"t.ds1", f);
    w.init(l);
    w.generate(l).unwrap();
    for r in w.drlg.level_rooms(l) {
        w.run(|p, d, c| p.room_grids(d, c, r)).unwrap();
    }
    let in_rooms: Vec<i32> = w
        .drlg
        .level_rooms(l)
        .iter()
        .flat_map(|&r| w.p.room_units(r).iter().map(|u| u.class))
        .collect();
    assert_eq!(in_rooms, [1]);
    let m = w.p.level_maps(l)[0];
    let left: Vec<_> = w.p.map(m).unwrap().units.iter().map(|u| u.class).collect();
    assert_eq!(left, [2, 3]);
}

#[test]
fn path_points_stay_absolute_unit_becomes_room_relative() {
    let mut w = World::new();
    let l = w.level(20, 202, 16, 8);
    w.drlg.level_mut(l).rect = TileRect::new(2, 3, 16, 8);
    w.pd.defs[202].scan = 1;
    let mut f = ds1(16, 8);
    // In the second room (DS1 tiles 8..16 → sub-tiles 40..80).
    f.objects = vec![obj(2, 151, 45, 5)];
    f.paths = vec![Ds1PathInput {
        x: 45,
        y: 5,
        points: vec![(50, 6, 1)],
    }];
    w.file(202, b"t.ds1", f);
    w.init(l);
    w.generate(l).unwrap();
    // Map: unit and path both level-absolute (+ map origin · 5).
    let r = room_at(&w, l, 10, 3);
    assert_eq!(w.drlg.room(r).rect, TileRect::new(10, 3, 8, 8));
    w.run(|p, d, c| p.room_grids(d, c, r)).unwrap();
    let u = &w.p.room_units(r)[0];
    // Unit: (45 + 10, 5 + 15) − room origin (50, 15) = (5, 5).
    assert_eq!((u.x, u.y), (5, 5));
    // Path: (50 + 10, 6 + 15), not shifted by the room origin.
    assert_eq!(
        u.path.as_deref(),
        Some(
            &[PathPoint {
                action: 1,
                x: 60,
                y: 21
            }][..]
        )
    );
}

#[test]
fn v3_ds1_loses_its_floor_layer() {
    let pd = preset_data();
    let mut f = ds1(4, 4);
    f.floors[0].fill(cell(1, 2));
    for (v, floors) in [(3u32, 0usize), (4, 1)] {
        f.version = v;
        let p = Ds1File::from_input(&f, &pd).unwrap();
        assert_eq!(p.floors.len(), floors, "v{v}");
        assert_eq!(p.walls.len(), 1, "v{v}");
    }
}

#[test]
fn item_unit_id_beyond_code_table_is_not_mapped() {
    let pd = preset_data();
    let mut f = ds1(4, 4);
    f.version = 12;
    // Entry 0 (`hdm `) is the only code table entry.
    f.objects = vec![obj(4, 0, 1, 1)];
    let p = Ds1File::from_input(&f, &pd).unwrap();
    assert_eq!((p.units[0].class, p.units[0].mode), (pd.hdm_item, 3));
    // Id 1 would read past the one-entry table: D2RUST reports it
    // instead of inventing a class.
    f.objects = vec![obj(4, 1, 1, 1)];
    assert_eq!(
        Ds1File::from_input(&f, &pd),
        Err(PresetError::ItemCodeBeyondTable(1))
    );
    // v ≤ 4 keeps the id as stored (no table read).
    f.version = 4;
    f.objects = vec![obj(4, 1, 1, 1)];
    assert_eq!(Ds1File::from_input(&f, &pd).unwrap().units[0].class, 1);
}

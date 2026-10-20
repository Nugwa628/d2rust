//! Gap tests: rules of the two specs not yet claimed by other tests.

use std::cell::RefCell;

use d2_data::tables::{Leveldefs, Record};

use super::fakes::*;
use crate::drlg::collision::bits;
use crate::drlg::room::sort_near;
use crate::drlg::tiles::{cell, rarity_walk, rec_flags, record_flags, RecordKind};
use crate::drlg::*;
use crate::rng::Seed;
use crate::units::ClientId;

const INIT: u32 = 644_409_375;
const START: u32 = 4_014_346_869;

/// Steps from `a` to `b`, if within 400.
fn steps(a: Seed, b: Seed) -> Option<usize> {
    let mut s = a;
    for n in 0..400 {
        if s == b {
            return Some(n);
        }
        s.step();
    }
    None
}

fn key(main: u32, sub: u32) -> u32 {
    (main << 20) | (sub << 8)
}

fn ids(d: &Drlg) -> Vec<u32> {
    d.level_list().iter().map(|&l| d.level(l).id).collect()
}

/// Level 2 (preset) with the given rooms, generated.
fn level_world(rooms: Vec<RoomSpec>) -> (World, Drlg, LevelIdx, Vec<DrlgRoomId>) {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    dat.levels[2].size = [(48, 24); 3];
    let mut types = FakeTypes::default();
    types.rooms.insert(2, rooms);
    types.default_grid = Some(floor_grid);
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let l = d.get_or_alloc_level(&w.data, &mut w.types, 2).unwrap();
    d.generate_level(&w.data, &mut w.types, l).unwrap();
    let r = d.level_rooms(l);
    (w, d, l, r)
}

/// A row of `n` 8×8 rooms in level 2.
fn row(n: i32) -> (World, Drlg, LevelIdx, Vec<DrlgRoomId>) {
    level_world((0..n).map(|i| preset(8 * i, 0, 8, 8)).collect())
}

/// One 2×2 preset room at (0, 0) in `level` with the given grids
/// (3×3 cells per pass).
fn one_room(grids: RoomGrids, level: u32) -> (World, Drlg, DrlgRoomId) {
    let mut dat = data();
    gen_level(&mut dat, level, 2);
    let mut types = FakeTypes::default();
    types.rooms.insert(level, vec![preset(0, 0, 2, 2)]);
    types.grids.insert((level, 0), grids);
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let l = d.get_or_alloc_level(&w.data, &mut w.types, level).unwrap();
    d.generate_level(&w.data, &mut w.types, l).unwrap();
    let r = d.level_rooms(l)[0];
    (w, d, r)
}

fn pass(cells: &[u32], orientation: Option<&[u32]>, fill_blanks: bool) -> GridPass {
    let g = |c: &[u32]| CellGrid {
        width: 3,
        height: 3,
        cells: c.to_vec(),
    };
    GridPass {
        cells: g(cells),
        orientation: orientation.map(g),
        fill_blanks,
    }
}

fn grids(passes: Vec<GridPass>) -> RoomGrids {
    RoomGrids {
        passes,
        ..RoomGrids::default()
    }
}

// ---- ----------------------------------------------------------

#[test]
fn acts_created_on_first_need_with_their_town() {
    let mut dat = data();
    for t in TOWN_LEVELS {
        gen_level(&mut dat, t, 2);
    }
    gen_level(&mut dat, 5, 2);
    let mut w = World::new(dat, FakeTypes::default());
    let mut g = Dungeon::default();
    assert!(g.acts.iter().all(Option::is_none), "never eager");
    let d = g
        .get_or_create(1, INIT, 2, None, &w.data, &mut w.types)
        .unwrap();
    // r2/r3: init seed, difficulty, server flags 0, town 40 generated.
    assert_eq!(
        (d.act, d.init_seed, d.difficulty, d.on_client),
        (1, INIT, 2, false)
    );
    assert_eq!(ids(d), [40]);
    assert_eq!(w.types.generated, [40]);
    assert!(g.acts[0].is_none());
    // A second need returns the stored act: no re-creation.
    g.get_or_create(1, 7, 0, None, &w.data, &mut w.types)
        .unwrap();
    assert_eq!(g.acts[1].as_ref().unwrap().init_seed, INIT);
    assert_eq!(w.types.generated, [40]);
    for act in [0, 2, 3, 4] {
        g.get_or_create(act, INIT, 0, None, &w.data, &mut w.types)
            .unwrap();
    }
    assert_eq!(w.types.generated, [40, 1, 75, 103, 109]);
    // Arena game: the arena's level replaces the town.
    let mut g = Dungeon::default();
    let d = g
        .get_or_create(0, INIT, 0, Some(5), &w.data, &mut w.types)
        .unwrap();
    assert_eq!(ids(d), [5]);
    // Client copy: town id 0, flags 1: no town level.
    let c = Drlg::create(0, INIT, 0, 0, true, &w.data, &mut w.types).unwrap();
    assert!(c.on_client);
    assert!(c.level_list().is_empty());
}

#[test]
fn drlg_creation_stores_its_fields_and_empty_status_lists() {
    let mut w = World::new(data(), FakeTypes::default());
    let d = Drlg::create(3, INIT, 2, 0, true, &w.data, &mut w.types).unwrap();
    assert_eq!(
        (d.act, d.init_seed, d.difficulty, d.on_client),
        (3, INIT, 2, true)
    );
    for s in 0..4 {
        assert!(d.status_list(s).is_empty());
    }
    assert_eq!(
        (d.rooms_built, d.builds_since_update, d.freed_rooms),
        (0, 0, 0)
    );
}

#[test]
fn get_or_allocate_finds_or_allocates_never_generates() {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    gen_level(&mut dat, 7, 2);
    dat.levels[2].vis = [7, 0, 0, 0, 0, 0, 0, 0];
    let mut types = FakeTypes::default();
    types.rooms.insert(2, vec![preset(0, 0, 8, 8)]);
    types.rooms.insert(7, vec![preset(50, 0, 8, 8)]);
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let l = d.get_or_alloc_level(&w.data, &mut w.types, 2).unwrap();
    assert_eq!(d.get_or_alloc_level(&w.data, &mut w.types, 2), Ok(l));
    assert_eq!(ids(&d), [2]);
    assert!(w.types.generated.is_empty());
    assert_eq!(d.room_count(l), 0);
    // A lookup by id (vis levels of the activity count) allocates level 7
    // without generating it.
    d.generate_level(&w.data, &mut w.types, l).unwrap();
    let r = d.level_rooms(l)[0];
    d.update_level_activity(&w.data, &mut w.types, None, Some(r))
        .unwrap();
    let l7 = d.find_level(7).unwrap();
    assert_eq!(ids(&d), [7, 2]);
    assert_eq!(w.types.generated, [2]);
    assert_eq!(d.room_count(l7), 0);
}

#[test]
fn generation_dispatches_drlg_types_1_to_3() {
    let mut dat = data();
    for ty in 1..=4 {
        gen_level(&mut dat, 10 + ty, ty);
    }
    let mut w = World::new(dat, FakeTypes::default());
    let mut d = w.drlg(INIT);
    for id in 11..=14 {
        let l = d.get_or_alloc_level(&w.data, &mut w.types, id).unwrap();
        assert_eq!(d.level(l).drlg_type, id - 10);
        d.generate_level(&w.data, &mut w.types, l).unwrap();
    }
    // Maze, preset and outdoor reach the type generator; type 4 does not.
    assert_eq!(w.types.generated, [11, 12, 13]);
}

#[test]
fn vis_and_warp_slots_from_leveldefs_columns() {
    let mut b = vec![0u8; 156];
    for i in 0..8 {
        b[72 + 4 * i..76 + 4 * i].copy_from_slice(&(i as u32 + 1).to_le_bytes());
        let w: u32 = if i == 2 { 9 } else { u32::MAX };
        b[104 + 4 * i..108 + 4 * i].copy_from_slice(&w.to_le_bytes());
    }
    let def = LevelDef::from_record(&Leveldefs::decode(&b));
    assert_eq!(def.vis, [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(def.warp, [-1, -1, 9, -1, -1, -1, -1, -1]);
    // Without a DRLG record, the readers return these arrays.
    let mut dat = data();
    dat.levels[2] = def.clone();
    let mut w = World::new(dat, FakeTypes::default());
    let d = w.drlg(INIT);
    assert_eq!(d.vis_array(&w.data, 2), Ok(def.vis));
    assert_eq!(d.warp_array(&w.data, 2), Ok(def.warp));
}

#[test]
fn warp_flag_bit_i_is_vis_slot_i() {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    // Only slot 3 has a warp id.
    dat.levels[2].warp = [-1, -1, -1, 6, -1, -1, -1, -1];
    let mut types = FakeTypes::default();
    let mut a = preset(0, 0, 8, 8);
    a.flags = room_flags::WARP_0 << 3;
    let mut b = preset(8, 0, 8, 8);
    b.flags = room_flags::WARP_0 << 2;
    types.rooms.insert(2, vec![b, a]);
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let l = d.get_or_alloc_level(&w.data, &mut w.types, 2).unwrap();
    d.generate_level(&w.data, &mut w.types, l).unwrap();
    // Bit 0x10 << 3 reads slot 3 (warp 6): room a qualifies; room b's bit
    // reads slot 2 (−1): it does not.
    assert_eq!(d.level(l).warp_centres, [(20, 20)]);
}

#[test]
fn freeing_draws_nothing_and_regeneration_keeps_identity() {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    dat.levels[2].size = [(24, 24); 3];
    dat.levels[2].vis = [3, 0, 0, 0, 0, 0, 0, 0];
    let mut types = FakeTypes::default();
    grid3x3(&mut types, 2);
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let l = d.get_or_alloc_level(&w.data, &mut w.types, 2).unwrap();
    d.generate_level(&w.data, &mut w.types, l).unwrap();
    let r4 = d.level_rooms(l)[4];
    d.room_mut(r4).other_flags |= 1;
    let seeds: Vec<_> = d
        .level_rooms(l)
        .iter()
        .map(|&r| d.room(r).init_seed)
        .collect();
    let (drlg_seed, level_seed) = (d.seed, d.level(l).seed);
    assert_eq!(ids(&d), [2]);
    // Count 0, frames 0: the free test runs (allocating vis level 3) and
    // passes.
    d.free_inactive_levels(&w.data, &mut w.types).unwrap();
    assert_eq!(d.room_count(l), 0);
    assert_eq!((d.seed, d.level(l).seed), (drlg_seed, level_seed));
    // Edge case 5: same id and seed, populated bit restored; level 3,
    // allocated since, stays in the list.
    d.generate_level(&w.data, &mut w.types, l).unwrap();
    assert_eq!(ids(&d), [3, 2]);
    assert_eq!(d.level(l).id, 2);
    assert_eq!(d.level(l).seed, level_seed);
    let rooms = d.level_rooms(l);
    let again: Vec<_> = rooms.iter().map(|&r| d.room(r).init_seed).collect();
    assert_eq!(again, seeds);
    assert_eq!(d.room(rooms[4]).other_flags & 1, 1);
}

fn spawn_world(position: u32) -> (World, Drlg) {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    dat.levels[2].position = position;
    dat.levels[2].size = [(24, 24); 3];
    let mut types = FakeTypes::default();
    grid3x3(&mut types, 2);
    types.default_grid = Some(floor_grid);
    let mut w = World::new(dat, types);
    let d = w.drlg(INIT);
    (w, d)
}

#[test]
fn spawn_room_allocates_and_generates_the_level_once() {
    let (mut w, mut d) = spawn_world(0);
    assert!(d.find_level(2).is_none());
    let mut svc = w.svc();
    d.spawn_room(&mut svc, 2, 0).unwrap();
    assert!(d.find_level(2).is_some());
    d.spawn_room(&mut svc, 2, 0).unwrap();
    assert_eq!(w.types.generated, [2]);
}

// The "loop runs out" half of the rule cannot occur (step 2:
// step 2: k <= n matches), so the record-0 read is the whole behaviour.
#[test]
fn spawn_tile_without_match_reads_record_0() {
    let (mut w, mut d) = spawn_world(1);
    w.types.spawn_tiles.insert(
        2,
        vec![
            SpawnTile {
                x: 9,
                y: 9,
                index: 6,
            },
            SpawnTile {
                x: 17,
                y: 17,
                index: 7,
            },
        ],
    );
    let mut svc = w.svc();
    // t = 12: class (0, 4), no record with index 12: record 0, no draw.
    let p = d.spawn_room(&mut svc, 2, 12).unwrap();
    let l = d.find_level(2).unwrap();
    let mut s = Seed::init_low(START + 2);
    for _ in 0..9 {
        s.step();
    }
    assert_eq!((p.x, p.y), (9, 9));
    assert_eq!(d.level(l).seed, s);
    // No records at all: record 0 of the zeroed level struct.
    let (mut w, mut d) = spawn_world(1);
    let mut svc = w.svc();
    let p = d.spawn_room(&mut svc, 2, 12).unwrap();
    assert_eq!((p.x, p.y), (0, 0));
    assert_eq!(d.room(p.room).rect, TileRect::new(0, 0, 8, 8));
}

// ---- -----------------------------------------------------------

#[test]
fn room_creation_fields_and_seed_by_index() {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    let mut w = World::new(dat, FakeTypes::default());
    let mut d = w.drlg(INIT);
    let l = d.get_or_alloc_level(&w.data, &mut w.types, 2).unwrap();
    let r0 = d.alloc_room(l, RoomKind::Outdoor, TileRect::new(0, 0, 8, 8));
    let room = d.room(r0);
    assert_eq!(room.status, room::STATUS_NONE);
    assert_eq!((room.kind, room.level), (RoomKind::Outdoor, l));
    assert_eq!((room.flags, room.other_flags, room.counts), (0, 0, [0; 4]));
    assert!(room.near().is_none() && room.active().is_none());
    assert!(room.warp_links.is_empty());
    // The caller links it: not in the level yet.
    assert!(d.level_rooms(l).is_empty());
    // Room k's seed is fixed by the level seed and k.
    let r1 = d.alloc_room(l, RoomKind::Preset, TileRect::new(8, 0, 8, 8));
    let mut ls = Seed::init_low(START + 2);
    for r in [r0, r1] {
        let mut s = ls.derive();
        let init = s.step();
        assert_eq!((d.room(r).seed, d.room(r).init_seed), (s, init));
    }
    assert_eq!(d.level(l).seed, ls);
}

#[test]
fn near_array_mixes_levels_sorted_by_global_coordinates() {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    gen_level(&mut dat, 3, 2);
    dat.levels[2].vis = [0, 3, 0, 0, 0, 0, 0, 0];
    dat.levels[2].warp = [-1, 5, -1, -1, -1, -1, -1, -1];
    dat.levels[3].vis = [2, 0, 0, 0, 0, 0, 0, 0];
    dat.warps = vec![WarpDef {
        id: 5,
        direction: b'b',
        ..WarpDef::default()
    }];
    let mut types = FakeTypes::default();
    let mut a = preset(0, 0, 8, 8);
    a.flags = room_flags::WARP_0 << 1;
    types.rooms.insert(2, vec![a, preset(8, 0, 8, 8)]);
    let mut t = preset(-20, 0, 8, 8);
    t.flags = room_flags::WARP_0;
    types.rooms.insert(3, vec![t]);
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let l = d.get_or_alloc_level(&w.data, &mut w.types, 2).unwrap();
    d.generate_level(&w.data, &mut w.types, l).unwrap();
    let r = d.level_rooms(l);
    d.build_near(&w.data, &mut w.types, r[0]).unwrap();
    let t = d.level_rooms(d.find_level(3).unwrap())[0];
    // [a, b] + t re-sorted on tile coordinates: t (x −20) sorts first.
    assert_eq!(d.room(r[0]).near(), Some(&[t, r[0], r[1]][..]));
}

#[test]
fn build_sequence_and_stop_when_built() {
    let (mut w, mut d, l, r) = row(3);
    let level_seed = d.level(l).seed;
    assert!(d.room(r[0]).near().is_none());
    let mut svc = w.svc();
    let a = d.stream_room(&mut svc, r[0]).unwrap().unwrap();
    // Near array built first.
    assert_eq!(d.room(r[0]).near(), Some(&[r[0], r[1]][..]));
    // Tile draws on the room seed after the reset, none on the level seed.
    let mut s = Seed::init_low(d.room(r[0]).init_seed);
    for _ in 0..81 {
        s.roll(4);
    }
    let act = s.derive();
    assert_eq!(d.room(r[0]).seed, s);
    assert_eq!(d.level(l).seed, level_seed);
    // The active room, in the act list.
    assert_eq!(d.active_room(r[0]).unwrap().seed, act);
    assert_eq!(w.lists.active_rooms(0), [a]);
    assert_ne!(d.room(r[0]).flags & room_flags::HAS_ROOM, 0);
    // Flag 0x100000 set: a second bring-up stops.
    let mut svc = w.svc();
    assert_eq!(d.stream_room(&mut svc, r[0]), Ok(Some(a)));
    assert_eq!(d.room(r[0]).seed, s);
    assert_eq!(d.rooms_built, 1);

    // Other room types: no grid init, no fill, but the flag is set.
    let (mut w, mut d, _, r) = level_world(vec![RoomSpec {
        rect: TileRect::new(0, 0, 8, 8),
        kind: RoomKind::Other(7),
        flags: 0,
    }]);
    let mut svc = w.svc();
    assert_eq!(d.stream_room(&mut svc, r[0]), Ok(None));
    assert!(d.room(r[0]).tiles().is_none());
    assert_ne!(d.room(r[0]).flags & room_flags::HAS_ROOM, 0);
    assert_eq!(d.room(r[0]).seed, Seed::init_low(d.room(r[0]).init_seed));
}

#[test]
fn library_and_preset_units_loaded_once() {
    let (mut w, mut d, _, r) = level_world(vec![
        preset(0, 0, 8, 8),
        RoomSpec {
            rect: TileRect::new(8, 0, 8, 8),
            kind: RoomKind::Outdoor,
            flags: 0,
        },
    ]);
    let mut svc = w.svc();
    let a = d.stream_room(&mut svc, r[0]).unwrap().unwrap();
    let id = |p: &[u8]| d.dt1_by_path[p];
    // Mask files first, then Blank, InvisWal, Warp.
    let want = [
        id(&b"floor.dt1"[..]),
        id(tiles::FIXED_LIBRARY[0]),
        id(tiles::FIXED_LIBRARY[1]),
        id(tiles::FIXED_LIBRARY[2]),
    ];
    assert_eq!(d.room(r[0]).library, want);
    let f = d.room(r[0]).flags;
    assert_ne!(f & room_flags::TILE_LIB_LOADED, 0);
    assert_ne!(f & room_flags::PRESET_UNITS_ADDED, 0);
    // Tiles freed and rebuilt: library and preset units are not redone.
    d.remove_active_room(&mut svc, a).unwrap();
    assert_ne!(d.room(r[0]).flags & room_flags::TILE_LIB_LOADED, 0);
    d.stream_room(&mut svc, r[0]).unwrap();
    assert_eq!(d.room(r[0]).library, want);
    // The outdoor room loads its library but adds no preset units.
    d.stream_room(&mut svc, r[1]).unwrap();
    assert_eq!(d.room(r[1]).library.len(), 4);
    assert_eq!(w.types.preset_units_added, [r[0]]);
}

/// FakeTypes noting, at each preset-unit read, whether the room was
/// already built (flag 0x100000).
#[derive(Default)]
struct StreamCheck {
    inner: FakeTypes,
    reads: RefCell<Vec<(DrlgRoomId, bool)>>,
}

impl LevelTypes for StreamCheck {
    fn generate(&mut self, d: &mut Drlg, data: &DrlgData, l: LevelIdx) -> Result<(), DrlgError> {
        self.inner.generate(d, data, l)
    }

    fn add_preset_units(&mut self, d: &mut Drlg, room: DrlgRoomId) -> Result<(), DrlgError> {
        self.inner.add_preset_units(d, room)
    }

    fn preset_units(&self, d: &Drlg, room: DrlgRoomId) -> Vec<PresetUnit> {
        let built = d.room(room).flags & room_flags::HAS_ROOM != 0;
        self.reads.borrow_mut().push((room, built));
        self.inner.preset_units(d, room)
    }

    fn room_grids(
        &mut self,
        d: &mut Drlg,
        data: &DrlgData,
        room: DrlgRoomId,
    ) -> Result<RoomGrids, DrlgError> {
        self.inner.room_grids(d, data, room)
    }
}

#[test]
fn waypoint_room_streamed_before_its_units_are_read() {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    dat.levels[2].size = [(24, 24); 3];
    dat.object_subclass = vec![0; 600];
    dat.object_subclass[119] = 0x40;
    let mut st = StreamCheck::default();
    grid3x3(&mut st.inner, 2);
    st.inner.default_grid = Some(floor_grid);
    st.inner.rooms.get_mut(&2).unwrap()[5].flags = room_flags::WAYPOINT;
    st.inner.preset_units.insert(
        2,
        vec![PresetUnit {
            unit_type: 2,
            class: 119,
            x: 10,
            y: 10,
        }],
    );
    let tl = tiles();
    let mut lists = crate::units::UnitLists::new();
    let mut d = Drlg::create(0, INIT, 0, 0, false, &dat, &mut st).unwrap();
    let mut svc = Services {
        data: &dat,
        tiles: &tl,
        types: &mut st,
        rooms: &mut lists,
    };
    let p = d.spawn_room(&mut svc, 2, 0).unwrap();
    let wp = d.level_rooms(d.find_level(2).unwrap())[5];
    assert_eq!(p.room, wp);
    assert!(p.active.is_some());
    assert_eq!(st.reads.borrow().as_slice(), &[(wp, true)]);
}

#[test]
fn active_room_coordinates_collision_and_populated_restart() {
    let (mut w, mut d, _, r) = row(3);
    let mut svc = w.svc();
    let a = d.stream_room(&mut svc, r[1]).unwrap().unwrap();
    let act = d.active_room(r[1]).unwrap();
    assert_eq!(act.subtiles, TileRect::new(40, 0, 40, 40));
    assert_eq!(d.room(r[1]).rect, TileRect::new(8, 0, 8, 8));
    // Collision grid built over the sub-tile rect from the floor tiles.
    assert_eq!(act.collision.rect, act.subtiles);
    assert_eq!(act.collision.get(40, 4), Some(bits::WALL));
    assert_eq!(act.collision.get(41, 4), Some(0));
    // Populated, removed, built again: starts with active flag bit 0.
    w.lists.room_mut(a).unwrap().populated = true;
    let mut svc = w.svc();
    d.remove_active_room(&mut svc, a).unwrap();
    let b = d.stream_room(&mut svc, r[1]).unwrap().unwrap();
    assert_eq!(d.active_room(r[1]).unwrap().flags, 1);
    assert!(w.lists.room(b).unwrap().populated);
}

#[test]
fn grids_become_floor_wall_and_shadow_records() {
    // Floor pass: key (0, 0). Wall pass: walls of orientation 1 with
    // shadows, key (1, 0).
    let f = pass(&[cell::FLOOR; 9], None, false);
    let w_ = pass(
        &[cell::WALL | cell::SHADOW | key(1, 0); 9],
        Some(&[1; 9]),
        false,
    );
    let (mut w, mut d, r) = one_room(grids(vec![f, w_]), 2);
    let l = d.room(r).level;
    let level_seed = d.level(l).seed;
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    let t = d.room(r).tiles().unwrap();
    assert_eq!((t.floors.len(), t.walls.len(), t.shadows.len()), (9, 9, 9));
    // Type: 0 for floors, the orientation grid for walls, 13 for shadows.
    assert!(t.floors.iter().all(|x| x.kind == 0));
    assert!(t.walls.iter().all(|x| x.kind == 1));
    assert!(t.shadows.iter().all(|x| x.kind == 13));
    // One room-seed draw per choice (floors roll(4), walls and shadows
    // roll(2)), then the active-room step; the level seed is untouched.
    let mut s = Seed::init_low(d.room(r).init_seed);
    for n in [4; 9].into_iter().chain([2; 18]) {
        s.roll(n);
    }
    s.step();
    assert_eq!(d.room(r).seed, s);
    assert_eq!(d.level(l).seed, level_seed);
    // Packed cell: main bits 20–25, sub 8–15, layer 18–19; 0 is key (0, 0).
    let v = (0x7F << 20) | (0x1AB << 8) | (2 << 18);
    assert_eq!(
        (cell::main(v), cell::sub(v), cell::layer(v)),
        (0x3F, 0xAB, 2)
    );
    assert_eq!((cell::main(0), cell::sub(0)), (0, 0));
    // Bits → record flags (table column "Used for").
    for (bit, flag) in [
        (cell::ENCLOSED, 0x4),
        (cell::LAYER_ABOVE, 0x1),
        (cell::FILL_LOS, 0x80),
        (cell::UNWALKABLE, 0x40),
        (cell::REVEAL_HIDDEN, 0x20C),
        (cell::LINKAGE, 0x102),
        (cell::OBJECT_WALL, 0x800),
        (cell::HIDDEN, 0x8),
    ] {
        assert_eq!(record_flags(0, 13, bit, 0, true), flag, "bit {bit:#x}");
    }
}

#[test]
fn choice_total_is_the_sum_of_rarities() {
    let (mut w, mut d, r) = one_room(RoomGrids::default(), 2);
    // Key (0, 5, 0): rarities 1, 2 in file order → list [2, 1].
    let f = w.tiles.0.get_mut(&b"floor.dt1"[..]).unwrap();
    f.push(tile(0, 5, 0, 1));
    f.push(tile(0, 5, 0, 2));
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    for _ in 0..6 {
        let mut want = d.room(r).seed;
        let rr = want.roll(3);
        let t = d.choose_tile(r, 0, 5, 0).unwrap();
        assert_eq!(d.room(r).seed, want);
        let e = d.lookup_tiles(r, 0, 5, 0);
        assert_eq!(t, e[rarity_walk(&[2, 1], rr)]);
    }
}

#[test]
fn linked_cell_takes_at_most_one_linked_tile() {
    let mut c = [0u32; 9];
    let mut o = [0u32; 9];
    // (0,0) floor + wall + shadow: the linked floor only.
    c[0] = cell::LINKED | cell::FLOOR | cell::WALL | cell::SHADOW;
    o[0] = 1;
    // (1,0) wall + shadow: the linked wall only.
    c[1] = cell::LINKED | cell::WALL | cell::SHADOW | key(1, 0);
    o[1] = 1;
    // (2,0) shadow: the linked shadow.
    c[2] = cell::LINKED | cell::SHADOW | key(1, 0);
    // (0,1) hidden shadow: not linked, falls through to a plain shadow.
    c[3] = cell::LINKED | cell::SHADOW | cell::HIDDEN | key(1, 0);
    // (1,1) linked bit alone: falls through, nothing.
    c[4] = cell::LINKED;
    // (2,1) blank floor (30, 0) with bit 7: bit 7 cleared, linked floor.
    c[5] = cell::LINKED | cell::FLOOR | cell::LAYER_ABOVE | key(30, 0);
    let (mut w, mut d, r) = one_room(grids(vec![pass(&c, Some(&o), false)]), 2);
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    let t = d.room(r).tiles().unwrap();
    let pos = |v: &[TileRecord]| v.iter().map(|x| (x.x, x.y)).collect::<Vec<_>>();
    assert_eq!(pos(&t.floors), [(0, 0), (2, 1)]);
    assert_eq!(t.floor_links, [0, 1]);
    assert_eq!(pos(&t.walls), [(1, 0)]);
    assert_eq!(pos(&t.shadows), [(2, 0), (0, 1)]);
    assert_eq!(
        t.other_links,
        [(RecordKind::Wall, 0), (RecordKind::Shadow, 0)]
    );
    assert_eq!(t.floors[1].cell & cell::LAYER_ABOVE, 0);
    assert_eq!(t.floors[1].flags & rec_flags::LAYER_ABOVE, 0);
}

/// Two 8×8 rooms side by side with linked edges, A built first.
fn pair() -> (World, Drlg, DrlgRoomId, DrlgRoomId) {
    let (w, d, _, r) = row(2);
    (w, d, r[0], r[1])
}

#[test]
fn shared_cells_cost_no_draw_once_built() {
    let (mut w, mut d, a, b) = pair();
    let mut svc = w.svc();
    d.stream_room(&mut svc, a).unwrap();
    let seed_a = d.room(a).seed;
    d.stream_room(&mut svc, b).unwrap();
    // B: 81 cells − the 9 of the shared column already built by A; the
    // found records keep their type: no re-choice on A's seed either.
    assert_eq!(
        steps(Seed::init_low(d.room(b).init_seed), d.room(b).seed),
        Some(72 + 1)
    );
    assert_eq!(d.room(a).seed, seed_a);
}

#[test]
fn tile_edge_cases() {
    let mut fc = [0u32; 9];
    let mut wc = [0u32; 9];
    let mut wo = [0u32; 9];
    // (1,0): hidden floor: still a record (hidden), still collision.
    fc[1] = cell::FLOOR | cell::HIDDEN;
    // (0,0): the TownN1 cell, orientation 1, key (5, 0).
    wc[0] = 0x0050_0081;
    wo[0] = 1;
    // (2,0): exit with main ≥ 8: dropped.
    wc[2] = cell::WALL | key(8, 0);
    wo[2] = 10;
    let (mut w, mut d, r) = one_room(
        grids(vec![pass(&fc, None, false), pass(&wc, Some(&wo), false)]),
        2,
    );
    let f = w.tiles.0.get_mut(&b"floor.dt1"[..]).unwrap();
    f.push(tile(1, 5, 0, 0));
    f.extend((0..45).map(|_| tile(0, 7, 0, 1)));
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    let t = d.room(r).tiles().unwrap();
    assert_eq!(t.walls.len(), 1);
    assert_eq!((t.walls[0].x, t.walls[0].y, t.walls[0].kind), (0, 0, 1));
    assert_eq!(t.walls[0].flags & rec_flags::HIDDEN, 0, "visible-flagged");
    assert_eq!(t.floors.len(), 1);
    assert_ne!(t.floors[0].flags & rec_flags::HIDDEN, 0);
    // Draws: the hidden floor's roll(4) and the active-room step only.
    assert_eq!(
        steps(Seed::init_low(d.room(r).init_seed), d.room(r).seed),
        Some(2)
    );
    // The hidden floor's DT1 byte is in the collision grid.
    let g = &d.active_room(r).unwrap().collision;
    assert_eq!(g.get(5, 4), Some(bits::WALL));
    // 40-entry cap per key.
    assert_eq!(d.lookup_tiles(r, 0, 7, 0).len(), 40);
}

#[test]
fn collision_grid_per_active_room_lifetime() {
    let (mut w, mut d, a, b) = pair();
    let mut svc = w.svc();
    let aa = d.stream_room(&mut svc, a).unwrap().unwrap();
    d.stream_room(&mut svc, b).unwrap();
    // B's grid was built after its adjacency array: it holds A's linked
    // column records (x 8) that lie in B.
    let gb = &d.active_room(b).unwrap().collision;
    assert_eq!(gb.get(40, 4), Some(bits::WALL));
    // Units set higher bits at run time.
    *d.collision_at_mut(41, 0).unwrap() |= bits::MONSTER;
    assert_eq!(d.collision_at(41, 0), Some(bits::MONSTER));
    // Removal frees A's grid; B's stays.
    d.remove_active_room(&mut svc, aa).unwrap();
    assert_eq!(d.collision_at(0, 4), None);
    assert_eq!(d.collision_at(40, 4), Some(bits::WALL));
}

#[test]
fn collision_bits() {
    assert_eq!(
        [
            bits::WALL,
            bits::VISIBLE,
            bits::MISSILE_BARRIER,
            bits::NOPLAYER,
            bits::PRESET,
            bits::BLANK,
            bits::MISSILE,
            bits::PLAYER,
            bits::MONSTER,
            bits::ITEM,
            bits::OBJECT,
            bits::DOOR,
            bits::NO_PATH,
            bits::PET,
            bits::BIT_4000,
            bits::CORPSE,
        ],
        core::array::from_fn::<u16, 16, _>(|i| 1 << i)
    );
    // Blank floors (DT1 bytes 0x20) and a door wall (record flag 0x2) at
    // (0, 0); unwalkable + fill-LOS cell at (1, 0).
    let floors = pass(
        &[
            0,
            cell::FLOOR | cell::UNWALKABLE | cell::FILL_LOS,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ],
        None,
        true,
    );
    let mut wc = [0u32; 9];
    let mut wo = [0u32; 9];
    wc[0] = cell::WALL | key(1, 0);
    wo[0] = 8;
    let (mut w, mut d, r) = one_room(grids(vec![floors, pass(&wc, Some(&wo), false)]), 2);
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    let g = &d.active_room(r).unwrap().collision;
    assert_eq!(g.get(2, 2), Some(bits::BLANK | bits::PRESET));
    assert_eq!(g.get(7, 1), Some(bits::WALL | bits::MISSILE_BARRIER));
    assert_eq!(g.get(5, 9), Some(bits::BLANK));
}

#[test]
fn near_sort_depends_on_input_order() {
    let a = TileRect::new(0, 0, 8, 8);
    let c = TileRect::new(8, 0, 8, 8);
    let d = TileRect::new(0, 8, 8, 8);
    let f = |mut v: Vec<TileRect>| {
        sort_near(&mut v, |r| r);
        v
    };
    // The same three rooms, C and D in opposite relative orders.
    assert_eq!(f(vec![a, c, d]), [a, c, d]);
    assert_eq!(f(vec![a, d, c]), [a, d, c]);
}

#[test]
fn removal_perturbs_until_next_activation_nearby() {
    let (mut w, mut d, _, r) = row(4);
    let mut svc = w.svc();
    for &x in &r[..3] {
        d.stream_room(&mut svc, x).unwrap();
    }
    let a0 = d.active_room(r[0]).unwrap().id;
    d.remove_active_room(&mut svc, a0).unwrap();
    // Near order restricted to active rooms would be [r1, r2].
    assert_eq!(d.active_room(r[1]).unwrap().adjacency, [r[2], r[1]]);
    // An activation not next to r1 leaves it perturbed.
    d.stream_room(&mut svc, r[3]).unwrap();
    assert_eq!(d.active_room(r[1]).unwrap().adjacency, [r[2], r[1]]);
    // An activation next to it refills it.
    d.stream_room(&mut svc, r[0]).unwrap();
    assert_eq!(d.active_room(r[1]).unwrap().adjacency, [r[0], r[1], r[2]]);
}

#[test]
fn inactivity_counter_keeps_growing_while_removal_is_blocked() {
    let (mut w, mut d, _, r) = row(6);
    let mut svc = w.svc();
    let a5 = d.stream_room(&mut svc, r[5]).unwrap().unwrap();
    d.room_mut(r[5]).flags |= room_flags::PORTAL;
    for n in 1..=15 {
        assert_eq!(d.room_inactivity(a5), Ok(n));
        assert_eq!(d.allows_removal(a5), Ok(false));
    }
    d.room_mut(r[5]).flags &= !room_flags::PORTAL;
    // The removal test does not advance or reset it.
    assert_eq!(d.allows_removal(a5), Ok(true));
    assert_eq!(d.active_room(r[5]).unwrap().inactivity, 15);
    assert_eq!(d.room_inactivity(a5), Ok(16));

    // Town: blocked while any town room has status ≤ 1.
    let mut dat = data();
    gen_level(&mut dat, 1, 2);
    let mut types = FakeTypes::default();
    types
        .rooms
        .insert(1, (0..6).map(|i| preset(8 * i, 0, 8, 8)).collect());
    types.default_grid = Some(floor_grid);
    let mut w = World::new(dat, types);
    let mut d = Drlg::create(0, INIT, 0, 1, false, &w.data, &mut w.types).unwrap();
    let r = d.level_rooms(d.find_level(1).unwrap());
    let mut svc = w.svc();
    let a5 = d.stream_room(&mut svc, r[5]).unwrap().unwrap();
    d.client_changes_room(&mut svc, ClientId(0), None, Some(r[0]))
        .unwrap();
    for n in 1..=12 {
        assert_eq!(d.room_inactivity(a5), Ok(n));
        assert_eq!(d.allows_removal(a5), Ok(false));
    }
}

#[test]
fn warp_level_generated_inside_the_linking_build() {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    gen_level(&mut dat, 3, 2);
    dat.levels[2].vis = [0, 3, 0, 0, 0, 0, 0, 0];
    dat.levels[2].warp = [-1, 5, -1, -1, -1, -1, -1, -1];
    dat.levels[3].vis = [2, 0, 0, 0, 0, 0, 0, 0];
    dat.warps = vec![WarpDef {
        id: 5,
        direction: b'b',
        ..WarpDef::default()
    }];
    let mut types = FakeTypes::default();
    let mut a = preset(0, 0, 8, 8);
    a.flags = room_flags::WARP_0 << 1;
    types.rooms.insert(2, vec![a]);
    let mut t = preset(0, 30, 8, 8);
    t.flags = room_flags::WARP_0;
    types.rooms.insert(3, vec![preset(50, 50, 8, 8), t]);
    types.default_grid = Some(floor_grid);
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let l = d.get_or_alloc_level(&w.data, &mut w.types, 2).unwrap();
    d.generate_level(&w.data, &mut w.types, l).unwrap();
    let r = d.level_rooms(l)[0];
    assert_eq!(w.types.generated, [2]);
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    // Level 3 was allocated and generated by the build (two room
    // allocations on its level seed), and the build completed.
    assert_eq!(w.types.generated, [2, 3]);
    let l3 = d.find_level(3).unwrap();
    let mut s = Seed::init_low(START + 3);
    s.step();
    s.step();
    assert_eq!(d.level(l3).seed, s);
    assert!(d.active_room(r).is_some());
}

#[test]
fn spawn_room_position_path_crashes_and_unset_position() {
    // Tile 13 with no waypoint room: a fatal error.
    let (mut w, mut d) = spawn_world(1);
    let mut svc = w.svc();
    assert_eq!(
        d.spawn_room(&mut svc, 2, 13),
        Err(DrlgError::NoWaypointRoom)
    );
    // A waypoint room without a waypoint object: that room, (x, y) left
    // at (−1, −1) (no centre default on the `Position` ≠ 0 path).
    let (mut w, mut d) = spawn_world(1);
    w.types.rooms.get_mut(&2).unwrap()[5].flags = room_flags::WAYPOINT;
    let mut svc = w.svc();
    let p = d.spawn_room(&mut svc, 2, 13).unwrap();
    let l = d.find_level(2).unwrap();
    assert_eq!(p.room, d.level_rooms(l)[5]);
    assert_eq!((p.x, p.y), (-1, -1));
    // A spawn-tile record whose position is in no room: fatal as well.
    let (mut w, mut d) = spawn_world(1);
    w.types.spawn_tiles.insert(
        2,
        vec![SpawnTile {
            x: 500,
            y: 500,
            index: 0,
        }],
    );
    let mut svc = w.svc();
    assert_eq!(d.spawn_room(&mut svc, 2, 0), Err(DrlgError::NoSpawnRoom));
}

#[test]
fn spawn_room_with_every_fallback_failing_finds_nothing() {
    let (mut w, mut d) = spawn_world(0);
    w.types.rooms.insert(2, Vec::new());
    let mut svc = w.svc();
    assert_eq!(d.spawn_room(&mut svc, 2, 0), Err(DrlgError::NoSpawnRoom));
    let l = d.find_level(2).unwrap();
    let seed = d.level(l).seed;
    // The kind-11 query reports it as (−1, −1); no draw on the level seed
    // (a level with no rooms has nothing to roll over).
    assert_eq!(d.kind11_location(&mut svc, 2), Ok((-1, -1)));
    assert_eq!(d.level(l).seed, seed);
}

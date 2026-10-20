//! Tests written to kill mutants `cargo mutants` left alive in the
//! act-level DRLG modules (`level`, `room`, `active`, `tiles`,
//! `collision`, `data`, `mod`). Each asserts what the DRLG rules say; the
//! survivors that no test can observe are listed in the mutation report.

use d2_data::tables::{Leveldefs, Lvltypes, Lvlwarp, Objects, Record};

use super::fakes::*;
use crate::drlg::seams::ActRooms;
use crate::drlg::tiles::{self, cell, rec_flags};
use crate::drlg::*;
use crate::rng::Seed;
use crate::units::{ClientId, UnitLists};

const INIT: u32 = 644_409_375;

/// A row of six 8×8 preset rooms in level `id`: near(i) = {i−1, i, i+1}.
fn row(id: u32, town: u32) -> (World, Drlg, Vec<DrlgRoomId>) {
    let mut dat = data();
    gen_level(&mut dat, id, 2);
    dat.levels[id as usize].size = [(48, 8); 3];
    let mut types = FakeTypes::default();
    types
        .rooms
        .insert(id, (0..6).map(|i| preset(8 * i, 0, 8, 8)).collect());
    types.default_grid = Some(floor_grid);
    let mut w = World::new(dat, types);
    let mut d = Drlg::create(0, INIT, 0, town, false, &w.data, &mut w.types).unwrap();
    let l = if town != 0 {
        d.find_level(town).unwrap()
    } else {
        let l = d.get_or_alloc_level(&w.data, &mut w.types, id).unwrap();
        d.generate_level(&w.data, &mut w.types, l).unwrap();
        l
    };
    let rooms = d.level_rooms(l);
    (w, d, rooms)
}

// ---- mod.rs ---------------------------------------------------------------

/// The closed variant of the
/// half-open containment: the far border is inside.
#[test]
fn rect_closed_containment_includes_the_far_border() {
    let r = TileRect::new(2, 3, 4, 5);
    // Corners of the closed rect [2, 6] × [3, 8].
    for (x, y) in [(2, 3), (6, 3), (2, 8), (6, 8), (4, 5)] {
        assert!(r.contains_closed(x, y), "({x}, {y})");
    }
    for (x, y) in [(1, 3), (7, 3), (2, 2), (2, 9), (7, 9), (1, 2)] {
        assert!(!r.contains_closed(x, y), "({x}, {y})");
    }
    // Half-open: the far border is outside.
    assert!(!r.contains(6, 3));
    assert!(!r.contains(2, 8));
}

// ---- active.rs --------------------------------------------------------------

/// The act-list record keeps the active-room
/// flags (+0x34: bit 0 populated, bit 1 units active, bit 2 no update)
/// and removal hands them back as they were.
#[test]
fn act_room_flags_round_trip() {
    for f in [0u32, 1, 2, 4, 5, 7] {
        let mut lists = UnitLists::new();
        let id = lists.create_active_room(0, f);
        let r = lists.room(id).unwrap();
        assert_eq!(r.populated, f & 1 != 0, "flags {f}");
        assert_eq!(r.units_active, f & 2 != 0, "flags {f}");
        assert_eq!(r.no_update, f & 4 != 0, "flags {f}");
        assert_eq!(lists.remove_active_room(id), f);
        assert!(lists.room(id).is_none());
    }
}

/// The adjacency array as act-list ids, in rooms-near
/// order restricted to active rooms.
#[test]
fn adjacent_rooms_are_the_act_list_ids() {
    let (mut w, mut d, r) = row(2, 0);
    let mut svc = w.svc();
    let a0 = d.stream_room(&mut svc, r[0]).unwrap().unwrap();
    let a1 = d.stream_room(&mut svc, r[1]).unwrap().unwrap();
    assert_eq!(d.adjacent_rooms(r[0]), [a0, a1]);
    assert_eq!(d.adjacent_rooms(r[1]), [a0, a1]);
    assert!(d.adjacent_rooms(r[3]).is_empty());
}

/// Outside a town (and level 120) only this room's
/// status counts; a seen room elsewhere in the level does not block.
#[test]
fn removal_outside_town_ignores_other_rooms() {
    let (mut w, mut d, r) = row(2, 0);
    let mut svc = w.svc();
    let a5 = d.stream_room(&mut svc, r[5]).unwrap().unwrap();
    d.client_changes_room(&mut svc, ClientId(0), None, Some(r[0]))
        .unwrap();
    assert_eq!(d.room(r[0]).status, 0);
    assert_eq!(d.room(r[5]).status, 4);
    assert_eq!(d.allows_removal(a5), Ok(true));
}

/// A town room with no room of the town at status ≤ 1
/// may be removed.
#[test]
fn town_room_removable_when_no_room_is_seen() {
    let (mut w, mut d, r) = row(1, 1);
    let mut svc = w.svc();
    let a5 = d.stream_room(&mut svc, r[5]).unwrap().unwrap();
    assert!(r.iter().all(|&x| d.room(x).status > 1));
    assert_eq!(d.allows_removal(a5), Ok(true));
}

/// A room change adds the client to rooms of the new
/// adjacency array missing from the old and removes it only from rooms of
/// the old array missing from the new; rooms in both keep it once.
#[test]
fn room_change_keeps_clients_of_shared_rooms() {
    let (mut w, mut d, r) = row(2, 0);
    let c = ClientId(7);
    let clients = |d: &Drlg, x: DrlgRoomId| d.active_room(x).map(|a| a.clients.clone());
    let mut svc = w.svc();
    d.client_changes_room(&mut svc, c, None, Some(r[0]))
        .unwrap();
    d.client_changes_room(&mut svc, c, Some(r[0]), Some(r[1]))
        .unwrap();
    for &x in &r[..3] {
        assert_eq!(clients(&d, x), Some(vec![c]));
    }
    d.client_changes_room(&mut svc, c, Some(r[1]), Some(r[2]))
        .unwrap();
    assert_eq!(clients(&d, r[0]), Some(vec![]));
    for &x in &r[1..4] {
        assert_eq!(clients(&d, x), Some(vec![c]));
    }
}

// ---- data.rs --------------------------------------------------------------

/// The table view keeps the
/// lvlwarp rows in file order (`Id`, `Direction`, `LitVersion`, `Tiles`,
/// signed `OffsetX` / `OffsetY` at +0x1C / +0x20, the warp offset
/// rule), `lvltypes` `File 1..32` NUL-trimmed, and the objects
/// subclass per class.
#[test]
fn table_view_from_records() {
    let mut w = vec![0u8; Lvlwarp::SIZE];
    w[0..4].copy_from_slice(&7u32.to_le_bytes());
    // SelectX (signed), SelectY (signed), SelectDX, SelectDY at +4…+0x10:
    // the warp tile's hover box.
    w[4..8].copy_from_slice(&(-30i32).to_le_bytes());
    w[8..12].copy_from_slice(&(-60i32).to_le_bytes());
    w[12..16].copy_from_slice(&60u32.to_le_bytes());
    w[16..20].copy_from_slice(&40u32.to_le_bytes());
    w[28..32].copy_from_slice(&(-4i32).to_le_bytes());
    w[32..36].copy_from_slice(&6i32.to_le_bytes());
    w[36..40].copy_from_slice(&3u32.to_le_bytes());
    w[40..44].copy_from_slice(&5u32.to_le_bytes());
    w[44] = b'r';
    let mut t = vec![0u8; Lvltypes::SIZE];
    t[0..5].copy_from_slice(b"a.dt1");
    t[60 * 31..60 * 31 + 2].copy_from_slice(b"zz");
    let mut o = vec![0u8; Objects::SIZE];
    o[359] = 0x41;
    let lw = Lvlwarp::decode(&w);
    let d = DrlgData::from_tables(
        &[Leveldefs::decode(&[0u8; Leveldefs::SIZE])],
        &[lw.clone(), lw],
        &[Lvltypes::decode(&t)],
        &[Objects::decode(&o)],
    );
    assert_eq!(d.levels.len(), 1);
    let warp = WarpDef {
        id: 7,
        direction: b'r',
        lit_version: 3,
        tiles: 5,
        offset_x: -4,
        offset_y: 6,
        select: (-30, -60, 60, 40),
    };
    assert_eq!(d.warps, [warp.clone(), warp]);
    assert_eq!(d.lvltypes.len(), 1);
    assert_eq!(d.lvltypes[0].len(), 32);
    assert_eq!(d.lvltype_file(0, 0), Some(&b"a.dt1"[..]));
    assert_eq!(d.lvltype_file(0, 31), Some(&b"zz"[..]));
    assert_eq!(d.lvltype_file(0, 1), None);
    assert_eq!(d.object_subclass, [0x41]);
    assert!(d.is_waypoint_object(0));
}

// ---- level.rs ---------------------------------------------------------------

/// Act III draws one step after the start seed and
/// keeps its low bit as the jungle-link bit.
#[test]
fn act3_jungle_link_bit_is_the_low_bit() {
    let mut w = World::new(data(), FakeTypes::default());
    let mut seen = [false; 2];
    for init in 0..16u32 {
        let d = Drlg::create(2, init, 0, 0, false, &w.data, &mut w.types).unwrap();
        let mut s = Seed::init_low(init);
        assert_eq!(d.start_seed, s.step());
        let bit = s.step() & 1 != 0;
        assert_eq!(d.jungle_link, bit, "init {init}");
        seen[usize::from(bit)] = true;
    }
    assert_eq!(seen, [true, true]);
}

/// The warp array reader also fails on a record of
/// level id 0.
#[test]
fn warp_array_of_level_zero_record_is_fatal() {
    let mut w = World::new(data(), FakeTypes::default());
    let mut d = w.drlg(INIT);
    d.warp_record_mut(&w.data, 0).unwrap();
    assert_eq!(
        d.warp_array(&w.data, 0),
        Err(DrlgError::WarpRecordLevelZero)
    );
}

/// The lvlwarp row of a slot is the first row (file
/// order) with the slot's warp id whose direction matches.
#[test]
fn lvlwarp_row_of_a_slot() {
    let mut dat = data();
    dat.levels[2].warp = [4, 9, -1, -1, -1, -1, -1, -1];
    let row = |id, direction| WarpDef {
        id,
        direction,
        ..WarpDef::default()
    };
    dat.warps = vec![row(4, b'l'), row(9, b'l'), row(4, b'r'), row(9, b'b')];
    let mut w = World::new(dat, FakeTypes::default());
    let d = w.drlg(INIT);
    assert_eq!(d.lvlwarp_row(&w.data, 2, 0, b'b'), Ok(0));
    assert_eq!(d.lvlwarp_row(&w.data, 2, 0, b'r'), Ok(2));
    assert_eq!(d.lvlwarp_row(&w.data, 2, 1, b'r'), Ok(3));
    assert_eq!(
        d.lvlwarp_row(&w.data, 2, 2, b'b'),
        Err(DrlgError::NoLvlWarp(-1))
    );
}

/// A room of the hint's rooms-near array wins over
/// the given level.
#[test]
fn room_at_prefers_the_hints_near_rooms_over_the_level() {
    let (mut w, mut d, r) = row(2, 0);
    gen_level(&mut w.data, 3, 2);
    let mut svc = w.svc();
    d.stream_room(&mut svc, r[0]).unwrap();
    assert!(d.room(r[0]).near().is_some_and(|n| n.contains(&r[1])));
    let other = d.get_or_alloc_level(&w.data, &mut w.types, 3).unwrap();
    // (9, 1) is in r[1] = (8, 0, 8, 8).
    assert_eq!(
        d.room_at(&w.data, &mut w.types, 9, 1, Some(r[0]), Some(other)),
        Ok(Some(r[1]))
    );
}

/// The level's room count (+0x08) counts its rooms.
#[test]
fn level_room_count() {
    let (_w, d, r) = row(2, 0);
    let l = d.room(r[0]).level;
    assert_eq!(d.room_count(l), 6);
}

// ---- collision.rs -----------------------------------------------------------

fn key(main: u32, sub: u32) -> u32 {
    (main << 20) | (sub << 8)
}

/// One 9×9 floor pass with one linked cell `(lx, ly)` of key `k`.
fn corner_grid(lx: usize, ly: usize, k: u32) -> RoomGrids {
    let mut g = CellGrid::new(9, 9);
    for y in 0..9 {
        for x in 0..9 {
            g.set(x, y, cell::FLOOR);
        }
    }
    g.set(lx, ly, cell::FLOOR | cell::LINKED | k);
    RoomGrids {
        passes: vec![GridPass {
            cells: g,
            orientation: None,
            fill_blanks: false,
        }],
        ..RoomGrids::default()
    }
}

/// A re-chosen tile updates the grid of the active
/// room that contains the record's sub-tile origin, found among the
/// record's room and its list; here a neighbour's grid, not the owner's.
///
/// A = (0, 0) owns the shared corner record at (8, 8) (blank key 30, 0),
/// whose origin lies in C = (8, 8); C (also blank there) is active when
/// B = (8, 0) re-chooses the record with its key (0, 0).
#[test]
fn collision_update_reaches_the_neighbour_holding_the_origin() {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    let mut types = FakeTypes::default();
    types.rooms.insert(
        2,
        vec![preset(0, 0, 8, 8), preset(8, 8, 8, 8), preset(8, 0, 8, 8)],
    );
    types.grids.insert((2, 0), corner_grid(8, 8, key(30, 0)));
    types.grids.insert((2, 1), corner_grid(0, 0, key(30, 0)));
    types.grids.insert((2, 2), corner_grid(0, 8, 0));
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let l = d.get_or_alloc_level(&w.data, &mut w.types, 2).unwrap();
    d.generate_level(&w.data, &mut w.types, l).unwrap();
    let r = d.level_rooms(l);
    let (a, c, b) = (r[0], r[1], r[2]);
    let mut svc = w.svc();
    d.stream_room(&mut svc, a).unwrap();
    d.stream_room(&mut svc, c).unwrap();
    // A's blank record (flags 0x20 everywhere) is in C's grid.
    let g = &d.active_room(c).unwrap().collision;
    assert_eq!(g.get(40, 40), Some(0x20));
    assert_eq!(g.get(40, 44), Some(0x20));
    d.stream_room(&mut svc, b).unwrap();
    // Re-chosen as a (0, 0) floor: 0x20 cleared, flag byte 0 set at the
    // block's bottom-left sub-tile, in C's grid.
    let g = &d.active_room(c).unwrap().collision;
    assert_eq!(g.get(40, 40), Some(0));
    assert_eq!(g.get(40, 44), Some(collision::bits::WALL));
}

// ---- level.rs: spawn room --------------------------------------------

fn spawn_world(position: u32, tiles: &[(i32, i32, u32)]) -> (World, Drlg) {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    dat.levels[2].position = position;
    dat.levels[2].size = [(24, 24); 3];
    dat.object_subclass = vec![0; 600];
    dat.object_subclass[119] = 0x40;
    dat.object_subclass[573] = 0x40;
    let mut types = FakeTypes::default();
    grid3x3(&mut types, 2);
    types.default_grid = Some(floor_grid);
    types.spawn_tiles.insert(
        2,
        tiles
            .iter()
            .map(|&(x, y, index)| SpawnTile { x, y, index })
            .collect(),
    );
    let mut w = World::new(dat, types);
    let d = w.drlg(INIT);
    (w, d)
}

/// T = 0 (class (1, 0)) matches records of indexes
/// 1..4 by class even with no record of index 0: two matches, one
/// `roll(2)` on the level seed.
#[test]
fn spawn_tile_matches_by_class_pair() {
    let (mut w, mut d) = spawn_world(1, &[(1, 1, 3), (17, 17, 4), (2, 20, 6)]);
    let mut svc = w.svc();
    let l = d.get_or_alloc_level(svc.data, svc.types, 2).unwrap();
    d.generate_level(svc.data, svc.types, l).unwrap();
    let mut s = d.level(l).seed;
    let p = d.spawn_room(&mut svc, 2, 0).unwrap();
    let r = s.roll(2) as usize;
    assert_eq!((p.x, p.y), [(1, 1), (17, 17)][r]);
    assert_eq!(d.level(l).seed, s);
    // t = 5 (class (1, 1)) matches index 6 by class: one match, a draw.
    let mut s = d.level(l).seed;
    let p = d.spawn_room(&mut svc, 2, 5).unwrap();
    s.roll(1);
    assert_eq!((p.x, p.y), (2, 20));
    assert_eq!(d.level(l).seed, s);
}

/// A class with a = 0 (t = 1) matches only its own
/// index: records 2 and 3 share b = 0 but do not match; record 0, no draw.
#[test]
fn spawn_tile_class_a_zero_matches_only_its_index() {
    let (mut w, mut d) = spawn_world(1, &[(1, 1, 2), (17, 17, 3)]);
    let mut svc = w.svc();
    let l = d.get_or_alloc_level(svc.data, svc.types, 2).unwrap();
    d.generate_level(svc.data, svc.types, l).unwrap();
    let s = d.level(l).seed;
    let p = d.spawn_room(&mut svc, 2, 1).unwrap();
    assert_eq!((p.x, p.y), (1, 1));
    assert_eq!(d.level(l).seed, s);
}

/// The waypoint object is the first preset object
/// with class < 573 whose subclass has bit 0x40; position
/// (room x + px / 5, room y + py / 5).
#[test]
fn waypoint_position_divides_by_five_and_skips_class_573() {
    let (mut w, mut d) = spawn_world(0, &[]);
    w.types.rooms.get_mut(&2).unwrap()[5].flags = room_flags::WAYPOINT;
    let unit = |class, x, y| PresetUnit {
        unit_type: 2,
        class,
        x,
        y,
    };
    w.types
        .preset_units
        .insert(2, vec![unit(573, 0, 0), unit(119, 17, 39)]);
    let mut svc = w.svc();
    let p = d.spawn_room(&mut svc, 2, 0).unwrap();
    assert_eq!(d.room(p.room).rect, TileRect::new(16, 8, 8, 8));
    assert_eq!((p.x, p.y), (16 + 3, 8 + 7));
}

/// Without a waypoint room, the first room (list
/// order) with a warp flag bit i whose warp slot i is not −1; the
/// position is its centre.
#[test]
fn spawn_in_first_room_with_a_used_warp_flag() {
    let (mut w, mut d) = spawn_world(0, &[]);
    w.data.levels[2].warp = [-1, 5, -1, -1, -1, -1, -1, -1];
    let rooms = w.types.rooms.get_mut(&2).unwrap();
    // Room 0: flag for slot 3, whose warp id is −1.
    rooms[0].flags = room_flags::WARP_0 << 3;
    // Room 2 = (16, 0): flag for slot 1 (warp id 5).
    rooms[2].flags = room_flags::WARP_0 << 1;
    let mut svc = w.svc();
    let p = d.spawn_room(&mut svc, 2, 0).unwrap();
    assert_eq!(d.room(p.room).rect, TileRect::new(16, 0, 8, 8));
    assert_eq!((p.x, p.y), (20, 4));
}

// ---- room.rs ----------------------------------------------------------------

/// The gap uses `R − A.w − A.x` only when `A.x < R.x`
/// (else `A.x − R.w − R.x`), the same for y; near needs both gaps < 6.
#[test]
fn near_gaps_at_equal_origins_and_the_y_bound() {
    use crate::drlg::room::{is_near, near_gaps};
    let a = TileRect::new(0, 0, 8, 8);
    // Same x and y: the else branches use the other room's size.
    assert_eq!(near_gaps(&a, &TileRect::new(0, 0, 4, 2)), (-4, -2));
    // gap_y = 14 − 8 − 0 = 6: not near; 5: near.
    assert_eq!(near_gaps(&a, &TileRect::new(0, 14, 8, 8)), (-8, 6));
    assert!(!is_near(&a, &TileRect::new(0, 14, 8, 8)));
    assert!(is_near(&a, &TileRect::new(0, 13, 8, 8)));
}

/// A freed room leaves its status
/// list and its slot is released.
#[test]
fn freed_rooms_leave_their_status_lists() {
    let (mut w, mut d, r) = row(2, 0);
    let mut svc = w.svc();
    d.client_changes_room(&mut svc, ClientId(0), None, Some(r[0]))
        .unwrap();
    assert_eq!(d.status_list(2), &[r[2]]);
    assert_eq!(d.status_list(3), &[r[3]]);
    let l = d.room(r[0]).level;
    d.free_level_rooms(&mut w.types, l);
    for s in 0..4 {
        assert!(d.status_list(s).is_empty(), "status list {s}");
    }
    assert!(r.iter().all(|&x| d.try_room(x).is_none()));
}

/// Level 2 (vis [3, 3, 0, ...], warp [−1, `warp`, −1, ...]) with room A
/// flagged for slot 1 only; level 3 (vis [2, 2, 0, ...]) with t0 flagged
/// for slot 0 and t1 for slot 1, both far from A. lvlwarp row 1 is
/// `Id` 1.
fn two_slot_world(warp: i32, level_id: u32) -> (World, Drlg, DrlgRoomId, Vec<DrlgRoomId>) {
    let mut dat = data();
    gen_level(&mut dat, level_id, 2);
    gen_level(&mut dat, 3, 2);
    dat.levels[level_id as usize].vis = [3, 3, 0, 0, 0, 0, 0, 0];
    dat.levels[level_id as usize].warp = [-1, warp, -1, -1, -1, -1, -1, -1];
    dat.levels[3].vis = [level_id, level_id, 0, 0, 0, 0, 0, 0];
    dat.warps = vec![
        WarpDef {
            id: 4,
            direction: b'b',
            ..WarpDef::default()
        },
        WarpDef {
            id: 1,
            direction: b'b',
            ..WarpDef::default()
        },
    ];
    let mut types = FakeTypes::default();
    let mut a = preset(0, 0, 8, 8);
    a.flags = room_flags::WARP_0 << 1;
    types.rooms.insert(level_id, vec![a]);
    let mut t0 = preset(100, 0, 8, 8);
    t0.flags = room_flags::WARP_0;
    let mut t1 = preset(200, 0, 8, 8);
    t1.flags = room_flags::WARP_0 << 1;
    types.rooms.insert(3, vec![t0, t1]);
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let l = d
        .get_or_alloc_level(&w.data, &mut w.types, level_id)
        .unwrap();
    d.generate_level(&w.data, &mut w.types, l).unwrap();
    let room = d.level_rooms(l)[0];
    d.build_near(&w.data, &mut w.types, room).unwrap();
    let targets = d
        .find_level(3)
        .map(|l3| d.level_rooms(l3))
        .unwrap_or_default();
    (w, d, room, targets)
}

/// Only flagged slots are walked (no level is allocated
/// for the unflagged vis slots); with W ≠ −1 the link is tried first at
/// the (c+1)-th slot of L whose vis is this level, c = earlier slots of
/// this level with the same vis (here c = 1, so L's slot 1 and its room
/// flagged `0x10 << 1`).
#[test]
fn warp_link_at_the_matching_slot_count() {
    let (_w, d, room, t) = two_slot_world(1, 2);
    assert_eq!(
        d.room(room).warp_links,
        [WarpLink {
            target: t[1],
            enabled: true,
            lvlwarp_row: 1,
        }]
    );
    assert_eq!(d.room(room).near(), Some(&[room, t[1]][..]));
    assert!(d.find_level(0).is_none(), "no Null level allocated");
}

/// No warp links in level 133 (its vis level is not
/// even allocated).
#[test]
fn no_warp_links_in_level_133() {
    let (_w, d, room, t) = two_slot_world(1, 133);
    assert!(t.is_empty());
    assert!(d.find_level(3).is_none());
    assert!(d.room(room).warp_links.is_empty());
    assert_eq!(d.room(room).near(), Some(&[room][..]));
}

/// Unset handler 3: on a client copy, a room whose status
/// becomes 4 has its tiles freed; a room still at a lower status keeps
/// them.
#[test]
fn client_copy_frees_tiles_when_status_becomes_none() {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    let mut types = FakeTypes::default();
    types
        .rooms
        .insert(2, (0..6).map(|i| preset(8 * i, 0, 8, 8)).collect());
    types.default_grid = Some(floor_grid);
    let mut w = World::new(dat, types);
    let mut d = Drlg::create(0, INIT, 0, 0, true, &w.data, &mut w.types).unwrap();
    let l = d.get_or_alloc_level(&w.data, &mut w.types, 2).unwrap();
    d.generate_level(&w.data, &mut w.types, l).unwrap();
    let r = d.level_rooms(l);
    let c = ClientId(0);
    let mut svc = w.svc();
    d.client_changes_room(&mut svc, c, None, Some(r[0]))
        .unwrap();
    d.client_changes_room(&mut svc, c, Some(r[0]), Some(r[3]))
        .unwrap();
    // Every room is within three steps of r3: none is at status 4.
    assert!(r.iter().all(|&x| d.room(x).status < 4));
    assert!(d.room(r[0]).tiles().is_some());
    d.client_changes_room(&mut svc, c, Some(r[3]), None)
        .unwrap();
    for &x in &r {
        assert_eq!(d.room(x).status, 4);
        assert!(d.room(x).tiles().is_none());
        assert!(d.active_room(x).is_none());
        assert_eq!(d.room(x).flags & room_flags::HAS_ROOM, 0);
    }
}

// ---- tiles.rs ---------------------------------------------------------------

/// One room at `rect` in level `level` with `grids`.
fn room_with(rect: TileRect, grids: RoomGrids, level: u32) -> (World, Drlg, DrlgRoomId) {
    let mut dat = data();
    gen_level(&mut dat, level, 2);
    let mut types = FakeTypes::default();
    types.rooms.insert(
        level,
        vec![RoomSpec {
            rect,
            kind: RoomKind::Preset,
            flags: 0,
        }],
    );
    types.grids.insert((level, 0), grids);
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let l = d.get_or_alloc_level(&w.data, &mut w.types, level).unwrap();
    d.generate_level(&w.data, &mut w.types, l).unwrap();
    let r = d.level_rooms(l)[0];
    (w, d, r)
}

/// One 3×3 pass (a 2×2 room) with an optional orientation grid.
fn pass3(cells: [u32; 9], orientation: Option<[u32; 9]>, fill_blanks: bool) -> RoomGrids {
    let g = |c: [u32; 9]| CellGrid {
        width: 3,
        height: 3,
        cells: c.to_vec(),
    };
    RoomGrids {
        passes: vec![GridPass {
            cells: g(cells),
            orientation: orientation.map(g),
            fill_blanks,
        }],
        ..RoomGrids::default()
    }
}

/// Only the set bits of the DT1 mask load
/// `File(i+1)`, up to the highest set bit (bit 20 here), then the three
/// fixed files.
#[test]
fn library_loads_exactly_the_mask_bits() {
    let (mut w, mut d, r) = room_with(TileRect::new(0, 0, 2, 2), RoomGrids::default(), 2);
    w.data.lvltypes[1][1] = b"floor.dt1".to_vec();
    w.data.lvltypes[1][20] = b"far.dt1".to_vec();
    w.tiles
        .0
        .insert(b"far.dt1".to_vec(), vec![tile(0, 5, 5, 1)]);
    d.room_mut(r).dt1_mask = 1 << 20;
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    assert_eq!(d.room(r).library.len(), 4);
    assert_eq!(d.lookup_tiles(r, 0, 5, 5).len(), 1);
    assert!(
        d.lookup_tiles(r, 0, 0, 0).is_empty(),
        "floor.dt1 not loaded"
    );
}

/// Room tiles, step 6 and door records: a (not hidden) door wall
/// adds its preset unit at the cell's world tile.
#[test]
fn door_unit_at_the_cells_world_tile() {
    let mut c = [0; 9];
    let mut o = [0; 9];
    c[7] = cell::WALL | key(1, 0);
    o[7] = 8;
    let (mut w, mut d, r) = room_with(TileRect::new(10, 20, 2, 2), pass3(c, Some(o), false), 2);
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    // Cell 7 = (1, 2).
    assert_eq!(w.types.door_units, [(11, 22)]);
}

/// Door records: the flag rules call the door unit
/// first and the record gets flag 0x20 on the outcomes that set it
/// (unit added or `roll(3)` = 0), and only then.
#[test]
fn door_record_flag_0x20_follows_the_door_unit() {
    for flag in [false, true] {
        let mut c = [0; 9];
        let mut o = [0; 9];
        c[7] = cell::WALL | key(1, 0);
        o[7] = 8;
        let (mut w, mut d, r) = room_with(TileRect::new(10, 20, 2, 2), pass3(c, Some(o), false), 2);
        w.types.door_flag = flag;
        let mut svc = w.svc();
        d.stream_room(&mut svc, r).unwrap();
        let t = d.room(r).tiles().unwrap();
        let door = t.walls.iter().find(|x| x.kind == 8).unwrap();
        assert_eq!(door.flags & rec_flags::DOOR_UNIT != 0, flag);
    }
}

/// Only keys (30, 0) and (30, 1) are blank;
/// a (30, 2) floor stays visible.
#[test]
fn only_sub_0_and_1_of_main_30_are_blank() {
    let mut c = [0; 9];
    c[0] = cell::FLOOR | key(30, 1);
    c[1] = cell::FLOOR | key(30, 2);
    let (mut w, mut d, r) = room_with(TileRect::new(0, 0, 2, 2), pass3(c, None, false), 2);
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    let f = &d.room(r).tiles().unwrap().floors;
    assert_eq!(f.len(), 2);
    assert_ne!(f[0].flags & rec_flags::HIDDEN, 0);
    assert_eq!(f[1].flags & rec_flags::HIDDEN, 0);
}

/// A FillBlanks floor's packed value is `v`
/// with bit 7 cleared and the hidden bit set; its record is hidden.
#[test]
fn fill_blank_floor_is_hidden() {
    // Already hidden or not, the blank is hidden.
    for extra in [0, cell::HIDDEN] {
        let c = [cell::LAYER_ABOVE | extra; 9];
        let (mut w, mut d, r) = room_with(TileRect::new(0, 0, 2, 2), pass3(c, None, true), 2);
        let mut svc = w.svc();
        d.stream_room(&mut svc, r).unwrap();
        let f = &d.room(r).tiles().unwrap().floors;
        assert_eq!(f.len(), 4);
        for rec in f {
            assert_eq!(rec.cell, cell::HIDDEN);
            assert_ne!(rec.flags & rec_flags::HIDDEN, 0);
        }
    }
}

/// Room tiles, step 6 and warp tiles: an exit wall
/// (10/11) outside level 133 adds a warp unit only for sub index 0 or 4.
#[test]
fn wall_warp_unit_for_sub_0_and_4() {
    let mut c = [0; 9];
    let mut o = [0; 9];
    c[0] = cell::WALL | key(1, 0);
    o[0] = 10;
    c[1] = cell::WALL | key(1, 4);
    o[1] = 11;
    c[3] = cell::WALL | key(1, 1);
    o[3] = 10;
    let grids = pass3(c, Some(o), false);
    let (mut w, mut d, r) = room_with(TileRect::new(0, 0, 2, 2), grids.clone(), 2);
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    assert_eq!(w.types.warp_units, [(0, 0), (1, 0)]);
    let (mut w, mut d, r) = room_with(TileRect::new(0, 0, 2, 2), grids, 133);
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    assert!(w.types.warp_units.is_empty());
}

/// Record flags: the layer and material rules OR onto
/// the existing flags.
#[test]
fn layer_rule_ors_onto_existing_flags() {
    assert_eq!(tiles::record_flags(0x10, 1, 0, 0, false), 0x10 | 1 << 14);
    assert_eq!(
        tiles::record_flags(0x2000, 5, 2 << 18, 0, false),
        0x2000 | 3 << 14
    );
    // DT1 material bit 0x1 ORs 0x4 onto what is there.
    assert_eq!(tiles::record_flags(0x10, 13, 0, 0x1, true), 0x14);
}

/// The border two 8×8 rooms share, as cells of each grid.
#[derive(Clone, Copy)]
enum Shared {
    /// A = (0, 0), B = (8, 0): column wx = 8 is B's left edge.
    BLeft,
    /// A = (8, 0), B = (0, 0): column wx = 8 is A's left edge.
    ALeft,
    /// A = (0, 8), B = (0, 0): row wy = 8 is A's top edge.
    ATop,
}

/// A builds first with linked cells `va` of type `ta` on the shared
/// border (cells 1..7 of it), then B with linked cells `vb` of type `tb`
/// on the same tiles. Returns the DRLG and the two rooms.
fn link_pair(
    shared: Shared,
    (va, ta): (u32, u32),
    (vb, tb): (u32, u32),
    table: [[u32; 7]; 6],
) -> (Drlg, DrlgRoomId, DrlgRoomId) {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    dat.wall_remap = WallRemap::with_rows(table);
    // (A origin, B origin, A cell of step k, B cell of step k).
    type Cells = fn(usize) -> (usize, usize);
    let (ao, bo, ac, bc): ((i32, i32), (i32, i32), Cells, Cells) = match shared {
        Shared::BLeft => ((0, 0), (8, 0), |k| (8, k), |k| (0, k)),
        Shared::ALeft => ((8, 0), (0, 0), |k| (0, k), |k| (8, k)),
        Shared::ATop => ((0, 8), (0, 0), |k| (k, 0), |k| (k, 8)),
    };
    let mut types = FakeTypes::default();
    types
        .rooms
        .insert(2, vec![preset(ao.0, ao.1, 8, 8), preset(bo.0, bo.1, 8, 8)]);
    let g = |at: Cells, v: u32, t: u32| {
        let mut c = CellGrid::new(9, 9);
        let mut o = CellGrid::new(9, 9);
        for k in 1..8 {
            let (x, y) = at(k);
            c.set(x, y, v | cell::LINKED);
            o.set(x, y, t);
        }
        RoomGrids {
            passes: vec![GridPass {
                cells: c,
                orientation: Some(o),
                fill_blanks: false,
            }],
            ..RoomGrids::default()
        }
    };
    types.grids.insert((2, 0), g(ac, va, ta));
    types.grids.insert((2, 1), g(bc, vb, tb));
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let l = d.get_or_alloc_level(&w.data, &mut w.types, 2).unwrap();
    d.generate_level(&w.data, &mut w.types, l).unwrap();
    let r = d.level_rooms(l);
    let mut svc = w.svc();
    d.stream_room(&mut svc, r[0]).unwrap();
    d.stream_room(&mut svc, r[1]).unwrap();
    (d, r[0], r[1])
}

const WALL: u32 = cell::WALL | (1 << 20);

fn merge_pair(
    ta: u32,
    tb: u32,
    vb: u32,
    a_right: bool,
    table: [[u32; 7]; 6],
) -> (Drlg, DrlgRoomId) {
    let shared = if a_right {
        Shared::BLeft
    } else {
        Shared::ALeft
    };
    let (d, a, _) = link_pair(shared, (WALL, ta), (WALL | vb, tb), table);
    (d, a)
}

fn linked_kinds(d: &Drlg, a: DrlgRoomId) -> Vec<(u32, u32)> {
    let t = d.room(a).tiles().unwrap();
    t.other_links
        .iter()
        .map(|&(k, i)| (t.records(k)[i].kind, t.records(k)[i].flags))
        .collect()
}

/// The index table maps new types 1, 2, 3, 5, 6, 7
/// to remap rows 0..5; the merged type is row[R.type − 1] (here 2 over
/// R type 1, re-chosen on A's seed).
#[test]
fn wall_remap_rows_by_new_type() {
    for (tb, row) in [(1, 0), (2, 1), (3, 2), (5, 3), (6, 4), (7, 5)] {
        let mut table = [[1u32; 7]; 6];
        table[row][0] = 2;
        let (d, a) = merge_pair(1, tb, 0, true, table);
        let kinds = linked_kinds(&d, a);
        assert_eq!(kinds.len(), 7);
        assert!(kinds.iter().all(|&(k, _)| k == 2), "new type {tb}");
    }
}

/// Wall remap: new types 8 (a door off this
/// room's top/left edge) and 13 "stop": R is left as it was, no flag
/// rules. A "keep" type (12) takes the new type over R and re-runs the
/// flag rules (here the new cell's unwalkable bit).
#[test]
fn wall_remap_keep_and_stop() {
    for tb in [8, 13] {
        let (d, a) = merge_pair(1, tb, cell::UNWALKABLE, false, [[2; 7]; 6]);
        let kinds = linked_kinds(&d, a);
        assert_eq!(kinds.len(), 7);
        for (k, f) in kinds {
            assert_eq!(k, 1, "new type {tb}");
            assert_eq!(f & rec_flags::UNWALKABLE, 0, "new type {tb}");
        }
    }
    let (d, a) = merge_pair(1, 12, cell::UNWALKABLE, false, [[2; 7]; 6]);
    let kinds = linked_kinds(&d, a);
    assert_eq!(kinds.len(), 7);
    for (k, f) in kinds {
        assert_eq!(k, 12);
        assert_ne!(f & rec_flags::UNWALKABLE, 0);
    }
}

fn link_count(d: &Drlg, r: DrlgRoomId) -> usize {
    d.room(r).tiles().map_or(0, |t| t.other_links.len())
}

/// The record's layer must match (record layer − 1 =
/// cell bits 18–19); a wall record does not match a cell with the shadow
/// bit; a shadow record matches a shadow cell.
#[test]
fn linked_find_layer_and_shadow_rules() {
    let t = [[1; 7]; 6];
    // Same layer: B finds A's seven records.
    let (d, _, b) = link_pair(Shared::BLeft, (WALL, 1), (WALL, 1), t);
    assert_eq!(link_count(&d, b), 0);
    // B's cells on layer 1: A's records (layer 0 + 1) do not match.
    let (d, _, b) = link_pair(Shared::BLeft, (WALL, 1), (WALL | 1 << 18, 1), t);
    assert_eq!(link_count(&d, b), 7);
    // B's wall cells also carry the shadow bit: A's walls do not match.
    let (d, _, b) = link_pair(Shared::BLeft, (WALL, 1), (WALL | cell::SHADOW, 1), t);
    assert_eq!(link_count(&d, b), 7);
    // Shadow cells on both sides: B finds A's shadow records.
    let sh = cell::SHADOW | (1 << 20);
    let (d, a, b) = link_pair(Shared::BLeft, (sh, 0), (sh, 0), t);
    assert_eq!(link_count(&d, a), 7);
    assert_eq!(link_count(&d, b), 0);
}

/// A linked exit (10/11) outside the room is skipped;
/// one inside gets its record and, outside level 133, the wall warp
/// tiles; a plain linked wall gets no warp tiles.
#[test]
fn linked_exits_and_warp_tiles() {
    let mut c = [0; 9];
    let mut o = [0; 9];
    // (0, 0) inside exit, (2, 0) exit on the extra column, (1, 0) wall.
    c[0] = WALL | cell::LINKED;
    o[0] = 10;
    c[2] = WALL | cell::LINKED;
    o[2] = 10;
    c[1] = WALL | cell::LINKED;
    o[1] = 1;
    let (mut w, mut d, r) = room_with(TileRect::new(0, 0, 2, 2), pass3(c, Some(o), false), 2);
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    let t = d.room(r).tiles().unwrap();
    let pos: Vec<_> = t.walls.iter().map(|x| (x.x, x.y, x.kind)).collect();
    assert_eq!(pos, [(0, 0, 10), (1, 0, 1)]);
    assert_eq!(w.types.warp_units, [(0, 0)]);
}

/// Wall remap door cases: a new door on this room's top or left
/// edge keeps its type (here over a wall, whatever the table says).
#[test]
fn new_door_on_own_top_left_edge_keeps_its_type() {
    let (d, a, _) = link_pair(Shared::BLeft, (WALL, 1), (WALL, 8), [[2; 7]; 6]);
    let kinds = linked_kinds(&d, a);
    assert_eq!(kinds.len(), 7);
    for (k, f) in kinds {
        assert_eq!(k, 8);
        // Re-run flag rules on the new door type add 0x2.
        assert_ne!(f & rec_flags::DOOR_OR_EXIT, 0);
    }
}

/// Wall remap door cases: a new non-door cell over an existing
/// door stops when the tile is on the neighbour's top or left edge (left
/// column and top row here); elsewhere the class of the new type decides
/// (a "keep" type 12 replaces the door and re-runs the flag rules; a
/// "table" type 1 over R type 8 > 7 stops).
#[test]
fn non_door_over_door_on_neighbours_edge_stops() {
    let door = (WALL, 8);
    let new = (WALL | cell::UNWALKABLE, 12);
    for shared in [Shared::ALeft, Shared::ATop] {
        let (d, a, _) = link_pair(shared, door, new, [[2; 7]; 6]);
        let kinds = linked_kinds(&d, a);
        assert_eq!(kinds.len(), 7);
        for (k, f) in kinds {
            assert_eq!(k, 8);
            assert_eq!(f & rec_flags::UNWALKABLE, 0);
        }
    }
    // B's left edge is not A's: no edge stop; "keep" → type 12.
    let (d, a, _) = link_pair(Shared::BLeft, door, new, [[2; 7]; 6]);
    let kinds = linked_kinds(&d, a);
    assert_eq!(kinds.len(), 7);
    for (k, f) in kinds {
        assert_eq!(k, 12);
        assert_ne!(f & rec_flags::UNWALKABLE, 0);
    }
    // "table" type 1 over the door (type 8 > 7): stop.
    let (d, a, _) = link_pair(Shared::BLeft, door, (new.0, 1), [[2; 7]; 6]);
    for (k, f) in linked_kinds(&d, a) {
        assert_eq!(k, 8);
        assert_eq!(f & rec_flags::UNWALKABLE, 0);
    }
}

/// Two 8×8 rooms N = (0, 0) and B = (8, 0) sharing column wx = 8. N
/// fills the linked wall cells `n_cells` (y, type) of that column in
/// order, then B fills (0, y) with type t for `b` = (y, t) (nothing for
/// `None`); both use cell bits `WALL` (layer 0, no bits 7 / 26 / 31).
/// Returns N, B and B's result.
fn corner_vector(
    n_cells: &[(usize, u32)],
    b: Option<(usize, u32)>,
) -> (Drlg, DrlgRoomId, DrlgRoomId, Result<(), DrlgError>) {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    let mut types = FakeTypes::default();
    types
        .rooms
        .insert(2, vec![preset(0, 0, 8, 8), preset(8, 0, 8, 8)]);
    let g = |x: usize, cells: &[(usize, u32)]| {
        let mut c = CellGrid::new(9, 9);
        let mut o = CellGrid::new(9, 9);
        for &(y, t) in cells {
            c.set(x, y, WALL | cell::LINKED);
            o.set(x, y, t);
        }
        RoomGrids {
            passes: vec![GridPass {
                cells: c,
                orientation: Some(o),
                fill_blanks: false,
            }],
            ..RoomGrids::default()
        }
    };
    types.grids.insert((2, 0), g(8, n_cells));
    types.grids.insert((2, 1), g(0, b.as_slice()));
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let l = d.get_or_alloc_level(&w.data, &mut w.types, 2).unwrap();
    d.generate_level(&w.data, &mut w.types, l).unwrap();
    let r = d.level_rooms(l);
    let mut svc = w.svc();
    d.stream_room(&mut svc, r[0]).unwrap();
    let out = d.stream_room(&mut svc, r[1]);
    (d, r[0], r[1], out.map(|_| ()))
}

/// The kinds of a room's non-floor chain, head first.
fn chain_head_first(d: &Drlg, r: DrlgRoomId) -> Vec<u32> {
    let t = d.room(r).tiles().unwrap();
    t.other_links
        .iter()
        .rev()
        .map(|&(k, i)| t.records(k)[i].kind)
        .collect()
}

/// Chain case C3: N's chain H → R → W; B fills R's
/// cell with t = 12 (`keep`): W (the record after R, at another cell)
/// is hidden, R becomes type 12 on N's seed, H is untouched, B draws
/// nothing and adds no record.
#[test]
fn corner_c3_hides_the_record_after_r_not_its_half() {
    let (d, n, b, out) = corner_vector(&[(1, 1), (2, 3)], Some((2, 12)));
    out.unwrap();
    let t = d.room(n).tiles().unwrap();
    assert_eq!(chain_head_first(&d, n), [4, 12, 1]);
    let rec = |e: usize| {
        let (k, i) = t.other_links[e];
        t.records(k)[i]
    };
    let (w_, r, h) = (rec(0), rec(1), rec(2));
    assert_eq!((w_.y, r.y, h.y), (1, 2, 2));
    assert_ne!(w_.flags & rec_flags::HIDDEN, 0, "W hidden (C3)");
    assert_eq!(r.kind, 12);
    assert_eq!(r.flags & rec_flags::HIDDEN, 0, "R's 0x8 cleared (C7)");
    assert_eq!(h.kind, 4);
    assert_eq!(h.flags & rec_flags::HIDDEN, 0, "H never reached");
    let tb = d.room(b).tiles().unwrap();
    assert!(tb.walls.is_empty() && tb.other_links.is_empty());
    // B's seed made no draw for the found cell: equal to a B with no
    // cells at all.
    let (d0, _, b0, _) = corner_vector(&[(1, 1), (2, 3)], None);
    assert_eq!(d.room(b).seed, d0.room(b0).seed);
}

/// Chain case C5: N's chain W; B fills W's cell with
/// t = 3 (`table` r1 = 3): W gets 0xC008, B gets S3 then S4 (chain S4 →
/// S3, two draws on B's seed), W is re-chosen as type 3 on N's seed with
/// no half; the closing re-run clears W's 0x8 and keeps 0xC000.
#[test]
fn corner_c5_adds_a_pair_to_this_room() {
    let (d, n, b, out) = corner_vector(&[(2, 1)], Some((2, 3)));
    out.unwrap();
    let t = d.room(n).tiles().unwrap();
    assert_eq!(chain_head_first(&d, n), [3]);
    let (k, i) = t.other_links[0];
    let w_ = t.records(k)[i];
    assert_eq!((w_.kind, w_.half), (3, None));
    assert_eq!(w_.flags & 0xC008, 0xC000);
    assert_eq!(chain_head_first(&d, b), [4, 3]);
    let tb = d.room(b).tiles().unwrap();
    assert_eq!(tb.walls.iter().map(|r| r.kind).collect::<Vec<_>>(), [3, 4]);
    // B's draws: the same as a B whose linked cell finds nothing (no N
    // record), which chooses (3, v) and (4, v) on B's seed for its pair.
    let (d1, _, b1, out) = corner_vector(&[], Some((2, 3)));
    out.unwrap();
    assert_eq!(chain_head_first(&d1, b1), [4, 3]);
    assert_eq!(d.room(b).seed, d1.room(b1).seed);
    // And N's seed made the type change's draw: one more than an N whose
    // W is never revisited.
    let (d0, n0, _, _) = corner_vector(&[(2, 1)], None);
    let (mut s, want) = (d0.room(n0).seed, d.room(n).seed);
    s.step();
    assert_eq!(s, want);
}

/// Chain case C4: R first in its chain (R +0x20
/// null) with m ≠ 3: D2RUST fails the fill.
#[test]
fn corner_c4_without_a_next_record_is_fatal() {
    let (_, _, _, out) = corner_vector(&[(2, 3)], Some((2, 12)));
    assert_eq!(out, Err(DrlgError::LinkedCornerNoNext));
}

/// Chain case C3 over a column of corners: each R's next record is
/// the previous corner's half, so merging R_k away hides H_(k−1); R_1 is
/// first in its chain. Merged to 3 no record is hidden.
#[test]
fn corner_merged_away_hides_the_previous_half() {
    // R_1 first in A's chain: fatal at B's first cell (C4).
    let (_, _, _, out) = corner_vector(&[(1, 3), (2, 3)], Some((1, 12)));
    assert_eq!(out, Err(DrlgError::LinkedCornerNoNext));
    // With a wall ahead of the corners: B over R_2 (keep 12) hides H_1.
    let (d, a, _, out) = corner_vector(&[(1, 1), (2, 3), (3, 3)], Some((3, 12)));
    out.unwrap();
    let t = d.room(a).tiles().unwrap();
    let flags: Vec<(u32, i32, bool)> = t
        .other_links
        .iter()
        .map(|&(k, i)| {
            let r = t.records(k)[i];
            (r.kind, r.y, r.flags & rec_flags::HIDDEN != 0)
        })
        .collect();
    assert_eq!(
        flags,
        [
            (1, 1, false),
            (3, 2, false),
            (4, 2, true),
            (12, 3, false),
            (4, 3, false)
        ]
    );
    // Merged to 3 (C6): nothing hidden.
    let (d, a, _) = link_pair(Shared::BLeft, (WALL, 3), (WALL, 1), [[3; 7]; 6]);
    let t = d.room(a).tiles().unwrap();
    assert!(t.walls.iter().all(|h| h.flags & rec_flags::HIDDEN == 0));
}

/// Only records whose tile has material 0x100 get an
/// animation entry; frame records k ≥ 1 carry flag 0x8 (hidden), also
/// when the base record is hidden.
#[test]
fn animation_only_for_animated_tiles_and_frames_hidden() {
    let anim = cell::FLOOR | key(2, 0);
    let mut c = [cell::FLOOR; 9];
    c[0] = anim | cell::HIDDEN;
    c[4] = anim;
    let mut g = pass3(c, None, false);
    g.animate = true;
    let (mut w, mut d, r) = room_with(TileRect::new(0, 0, 2, 2), g, 2);
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    let t = d.room(r).tiles().unwrap();
    assert_eq!(t.anims.len(), 2);
    // 9 cells + 2 extra frames for each of the two animated cells.
    assert_eq!(t.floors.len(), 13);
    for a in &t.anims {
        assert_eq!(a.frames.len(), 3);
        for &f in &a.frames[1..] {
            assert_ne!(t.floors[f].flags & rec_flags::HIDDEN, 0);
        }
    }
}

/// A new cell with v bit 7 merges to its
/// own type, without the table (which would give 3) or R's type (1).
#[test]
fn bit_7_merges_to_the_new_type() {
    let (d, a, _) = link_pair(
        Shared::BLeft,
        (WALL, 1),
        (WALL | cell::LAYER_ABOVE, 2),
        [[3; 7]; 6],
    );
    let kinds = linked_kinds(&d, a);
    assert_eq!(kinds.len(), 7);
    assert!(kinds.iter().all(|&(k, _)| k == 2), "{kinds:?}");
}

/// Column `r4` is never read: the find step skips type-4 records, so a
/// linked cell over a type-4 record finds nothing and builds its own.
#[test]
fn wall_remap_type4_records_are_never_found() {
    let (d, _, b) = link_pair(Shared::BLeft, (WALL, 4), (WALL, 1), [[2; 7]; 6]);
    assert_eq!(link_count(&d, b), 7);
}

/// A door (8, 9) is `stop` unless it is on its own room's top or left
/// edge (where it keeps its type before the table is consulted).
#[test]
fn wall_remap_doors_stop_off_the_top_left_edge() {
    for tb in [8, 9] {
        let (d, a) = merge_pair(1, tb, 0, false, [[2; 7]; 6]);
        assert!(linked_kinds(&d, a).iter().all(|&(k, _)| k == 1), "{tb}");
        let (d, a) = merge_pair(1, tb, 0, true, [[2; 7]; 6]);
        assert!(linked_kinds(&d, a).iter().all(|&(k, _)| k == tb), "{tb}");
    }
}

/// A `table` type over R of type > 7 stops; a `keep` type over the same
/// R replaces R's type.
#[test]
fn wall_remap_table_stops_over_high_r_and_keep_replaces() {
    let (d, a) = merge_pair(14, 1, 0, true, [[2; 7]; 6]);
    assert!(linked_kinds(&d, a).iter().all(|&(k, _)| k == 14));
    let (d, a) = merge_pair(14, 15, 0, true, [[2; 7]; 6]);
    assert!(linked_kinds(&d, a).iter().all(|&(k, _)| k == 15));
}

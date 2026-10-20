use super::fakes::*;
use crate::drlg::logic::{INFO_GRID, INFO_ONE};
use crate::drlg::tiles::RecordKind;
use crate::drlg::*;

const INIT: u32 = 644_409_375;

/// A DRLG with levels 2 and 3 allocated (no generation).
fn world() -> (Drlg, LevelIdx, LevelIdx) {
    let mut w = World::new(data(), FakeTypes::default());
    let mut d = w.drlg(INIT);
    let l2 = d.get_or_alloc_level(&w.data, &mut w.types, 2).unwrap();
    let l3 = d.get_or_alloc_level(&w.data, &mut w.types, 3).unwrap();
    (d, l2, l3)
}

fn room(d: &mut Drlg, l: LevelIdx, x: i32, y: i32, w: i32, h: i32) -> DrlgRoomId {
    let id = d.alloc_room(l, RoomKind::Preset, TileRect::new(x, y, w, h));
    d.room_mut(id).tiles = Some(RoomTiles::default());
    id
}

fn rec(x: i32, y: i32, kind: u32, flags: u32) -> TileRecord {
    TileRecord {
        x,
        y,
        kind,
        flags,
        tile: TileRef { file: 0, index: 0 },
        cell: 0,
        half: None,
    }
}

/// A layer-0 wall record (flags exactly 0x4000).
fn wall(x: i32, y: i32) -> TileRecord {
    rec(x, y, 1, 0x4000)
}

fn grid(w: usize, h: usize, set: &[(usize, usize, u32)]) -> CellGrid {
    let mut g = CellGrid::new(w, h);
    for &(x, y, v) in set {
        g.set(x, y, v);
    }
    g
}

fn cr(index: u32, node: bool, boxr: [i32; 4], clipped: [i32; 4]) -> CoordRec {
    CoordRec {
        boxr,
        clipped,
        node,
        index,
    }
}

fn indexes(d: &Drlg, id: DrlgRoomId) -> Vec<u32> {
    d.coord_first(id).unwrap().iter().map(|r| r.index).collect()
}

#[test]
fn one_record_room() {
    let (mut d, l, _) = world();
    let id = room(&mut d, l, 10, 20, 8, 8);
    d.level_mut(l).coord_counter = 7;
    d.build_logic_one(id);
    assert_eq!(d.level(l).coord_counter, 1);
    let info = d.room(id).logic().unwrap();
    assert_eq!((info.flags, info.array_len), (INFO_ONE, 1));
    let want = cr(1, false, [10, 20, 18, 28], [10, 20, 18, 28]);
    assert_eq!(d.coord_first(id).unwrap(), &[want]);
    assert_eq!(d.coord_at(id, 52, 103), Some(want));
    // One-record room: any point gives its record.
    assert_eq!(d.coord_at(id, 0, 0), Some(want));
}

#[test]
fn lookups_without_info() {
    let (mut d, l, _) = world();
    let id = room(&mut d, l, 0, 0, 2, 2);
    assert_eq!(d.coord_first(id), None);
    assert_eq!(d.coord_at(id, 0, 0), None);
}

#[test]
fn grid_build_open_room() {
    let (mut d, l, _) = world();
    let id = room(&mut d, l, 10, 20, 2, 1);
    d.build_logic_grid(id, &LogicGrids::default());
    assert_eq!(d.level(l).coord_counter, 4);
    let info = d.room(id).logic().unwrap();
    assert_eq!((info.flags, info.array_len), (INFO_GRID, 2));
    assert_eq!(info.index_grid, vec![0x1000_0002; 6]);
    assert_eq!(info.record_grid, vec![0; 6]);
    assert_eq!(
        info.list,
        vec![
            cr(2, false, [10, 20, 13, 22], [10, 20, 12, 21]),
            CoordRec::default()
        ]
    );
}

#[test]
fn grid_build_blockers_orientation_1() {
    let (mut d, l, _) = world();
    let id = room(&mut d, l, 10, 20, 2, 1);
    d.room_mut(id).tiles.as_mut().unwrap().walls = vec![wall(1, 0), wall(1, 1)];
    d.level_mut(l).coord_counter = 3;
    let g = LogicGrids {
        orientation: grid(3, 2, &[(1, 0, 1), (1, 1, 1)]),
        floor: grid(3, 2, &[(2, 0, 0x01E0_0002)]),
        wall: CellGrid::default(),
    };
    d.build_logic_grid(id, &g);
    assert_eq!(d.level(l).coord_counter, 8);
    let info = d.room(id).logic().unwrap();
    assert_eq!(info.array_len, 3);
    assert_eq!(
        info.index_grid,
        [4, 5, 5, 4, 5, 5].map(|i| 0x1000_0000 | i).to_vec()
    );
    assert_eq!(
        info.list,
        vec![
            cr(5, false, [11, 20, 13, 22], [11, 20, 12, 21]),
            cr(4, false, [10, 20, 11, 22], [10, 20, 11, 21]),
            CoordRec::default()
        ]
    );
    // Record grid by sub-tile point (C division).
    assert_eq!(d.coord_at(id, 50, 100).map(|r| r.index), Some(4));
    assert_eq!(d.coord_at(id, 55, 105).map(|r| r.index), Some(5));
    assert_eq!(d.coord_at(id, 64, 109).map(|r| r.index), Some(5));
    // Table, outside the (W+1) × (H+1) cells: with 0 ≤ cy ≤ H a
    // cell index inside the block reads that cell, so a column past W or
    // before 0 wraps into the next / previous row (cell (3, 0) is cell
    // (0, 1), cell (−1, 1) is cell (2, 0)); a negative index or a row
    // outside 0..H is not reproducible: no record.
    assert_eq!(d.coord_at(id, 65, 100).map(|r| r.index), Some(4));
    assert_eq!(d.coord_at(id, 45, 105).map(|r| r.index), Some(5));
    assert_eq!(d.coord_at(id, 45, 100), None);
    assert_eq!(d.coord_at(id, 65, 105), None);
    assert_eq!(d.coord_at(id, 50, 110), None);
}

#[test]
fn grid_build_orientation_0_reads_before_t2() {
    let (mut d, l, _) = world();
    let id = room(&mut d, l, 0, 0, 1, 1);
    d.room_mut(id).tiles.as_mut().unwrap().walls = vec![wall(0, 0)];
    d.build_logic_grid(id, &LogicGrids::default());
    assert_eq!(d.level(l).coord_counter, 4);
    assert_eq!(
        d.coord_first(id).unwrap(),
        &[
            cr(2, false, [0, 0, 2, 2], [0, 0, 1, 1]),
            CoordRec::default()
        ]
    );
}

#[test]
fn grid_build_unreached_blocker_and_zero_clip() {
    // (1, 0) blocked, orientation 0: entered from d 0 and d 3 the rule is
    // 0 (not marked), so it starts its own region; boxes starting at
    // x0 ≥ X + W are clipped to zero.
    let (mut d, l, _) = world();
    let id = room(&mut d, l, 0, 0, 1, 1);
    d.room_mut(id).tiles.as_mut().unwrap().walls = vec![wall(1, 0)];
    d.build_logic_grid(id, &LogicGrids::default());
    assert_eq!(d.level(l).coord_counter, 6);
    assert_eq!(
        d.coord_first(id).unwrap(),
        &[
            cr(2, false, [1, 1, 2, 2], [0; 4]),
            cr(3, false, [1, 0, 2, 1], [0; 4]),
            cr(2, false, [0, 0, 1, 2], [0, 0, 1, 1]),
            CoordRec::default()
        ]
    );
    assert_eq!(d.room(id).logic().unwrap().record_grid, vec![2, 1, 2, 0]);
    assert_eq!(d.coord_at(id, 5, 0).map(|r| r.index), Some(3));
    // C division toward zero: sub-tile −4 is tile 0.
    assert_eq!(d.coord_at(id, -4, 0).map(|r| r.index), Some(2));
    assert_eq!(d.coord_at(id, -5, 0), None);
}

/// Region count of a 2×1-cell room (rect (0, 0, 1, 0)) whose cell (1, 0)
/// has the given records (own walls, or a neighbour's link records).
fn regions_with(own: Vec<TileRecord>, neighbour: Option<(TileRect, TileRecord)>) -> usize {
    let (mut d, l, _) = world();
    let id = room(&mut d, l, 0, 0, 1, 0);
    d.room_mut(id).tiles.as_mut().unwrap().walls = own;
    let mut near = vec![id];
    if let Some((r, w)) = neighbour {
        let n = room(&mut d, l, r.x, r.y, r.w, r.h);
        let t = d.room_mut(n).tiles.as_mut().unwrap();
        t.walls = vec![rec(0, 0, 1, 0x4000), w];
        t.other_links = vec![(RecordKind::Wall, 1)];
        near.push(n);
    }
    d.room_mut(id).near = Some(near);
    d.build_logic_grid(id, &LogicGrids::default());
    d.coord_first(id).unwrap().len() - 1
}

#[test]
fn blocker_test() {
    assert_eq!(regions_with(vec![], None), 1);
    assert_eq!(regions_with(vec![wall(1, 0)], None), 2);
    // Layer bits must be exactly 0x4000; roofs and object walls don't block.
    assert_eq!(regions_with(vec![rec(1, 0, 1, 0x8000)], None), 1);
    assert_eq!(regions_with(vec![rec(1, 0, 1, 0xC000)], None), 1);
    assert_eq!(regions_with(vec![rec(1, 0, 1, 0)], None), 1);
    assert_eq!(regions_with(vec![rec(1, 0, 15, 0x4000)], None), 1);
    assert_eq!(regions_with(vec![rec(1, 0, 1, 0x4800)], None), 1);
    assert_eq!(regions_with(vec![rec(1, 0, 1, 0x4001)], None), 2);
    // A neighbour's non-floor link records, at N.x + rec x, block when on
    // this room's rect or its border; its unlinked records don't.
    let n = TileRect::new(1, -3, 4, 4);
    assert_eq!(regions_with(vec![], Some((n, wall(0, 3)))), 2);
    assert_eq!(regions_with(vec![], Some((n, rec(0, 3, 15, 0x4000)))), 1);
    assert_eq!(regions_with(vec![], Some((n, wall(1, 3)))), 1);
    // The unlinked record at N's (0, 0) = (1, −3) is outside anyway; put
    // the neighbour so its unlinked record lands on (1, 0).
    let n2 = TileRect::new(1, 0, 4, 4);
    assert_eq!(regions_with(vec![], Some((n2, wall(2, 0)))), 1);
}

#[test]
fn node_flag_from_start_cell() {
    let node = |v: u32| {
        let (mut d, l, _) = world();
        let id = room(&mut d, l, 0, 0, 0, 0);
        let g = LogicGrids {
            floor: grid(1, 1, &[(0, 0, v)]),
            ..LogicGrids::default()
        };
        d.build_logic_grid(id, &g);
        let info = d.room(id).logic().unwrap();
        assert_eq!(info.index_grid[0] & NODE_BIT != 0, info.list[0].node);
        info.list[0].node
    };
    const NODE_BIT: u32 = crate::drlg::logic::NODE;
    // Keys (30, 0), (31, 0), (62, 0), (63, 0); bits 20 and 25 ignored.
    assert!(node(30 << 20));
    assert!(node(31 << 20));
    assert!(node(62 << 20));
    assert!(node(63 << 20));
    assert!(node((30 << 20) | (1 << 25)));
    assert!(node((30 << 20) | 0xFF));
    // Sub index ≠ 0, main index bits 21–24 not all set.
    assert!(!node((30 << 20) | (1 << 8)));
    assert!(!node(28 << 20));
    assert!(!node(0));
    // Hidden bit.
    assert!(node(0x8000_0000));
}

#[test]
fn level_counter_rules() {
    let (mut d, l, _) = world();
    let a = room(&mut d, l, 0, 0, 2, 2);
    let b = room(&mut d, l, 10, 0, 2, 2);
    let c = room(&mut d, l, 20, 0, 2, 2);
    d.build_logic_grid(a, &LogicGrids::default());
    assert_eq!((indexes(&d, a), d.level(l).coord_counter), (vec![2, 0], 4));
    d.build_logic_grid(b, &LogicGrids::default());
    assert_eq!((indexes(&d, b), d.level(l).coord_counter), (vec![5, 0], 7));
    // A one-record room resets the counter to 1.
    d.build_logic_one(c);
    assert_eq!((indexes(&d, c), d.level(l).coord_counter), (vec![1], 1));
    d.room_mut(a).logic = None;
    d.build_logic_grid(a, &LogicGrids::default());
    assert_eq!((indexes(&d, a), d.level(l).coord_counter), (vec![2, 0], 4));
}

/// Rooms of `levels` (level, x) as 2×2 rooms on row 0, all near each
/// other (array order = argument order).
fn merge_world(rooms: &[(bool, i32)]) -> (Drlg, Vec<DrlgRoomId>) {
    let (mut d, l2, l3) = world();
    let ids: Vec<DrlgRoomId> = rooms
        .iter()
        .map(|&(other, x)| room(&mut d, if other { l3 } else { l2 }, x, 0, 2, 2))
        .collect();
    for &id in &ids {
        d.room_mut(id).near = Some(ids.clone());
    }
    (d, ids)
}

#[test]
fn merge_renames_to_touching_neighbour() {
    let (mut d, r) = merge_world(&[(false, 0), (false, 2)]);
    d.build_logic_grid(r[0], &LogicGrids::default());
    d.build_logic_grid(r[1], &LogicGrids::default());
    assert_eq!(indexes(&d, r[0]), [2, 0]);
    assert_eq!(indexes(&d, r[1]), [2, 0]);
    // The counter still counts the consumed indexes.
    assert_eq!(d.level(d.room(r[1]).level).coord_counter, 7);
}

#[test]
fn merge_conditions() {
    // Gap of 1 tile: no merge.
    let (mut d, r) = merge_world(&[(false, 0), (false, 3)]);
    d.build_logic_grid(r[0], &LogicGrids::default());
    d.build_logic_grid(r[1], &LogicGrids::default());
    assert_eq!(indexes(&d, r[1]), [5, 0]);
    // Different level ids: no merge.
    let (mut d, r) = merge_world(&[(false, 0), (true, 2)]);
    d.build_logic_grid(r[0], &LogicGrids::default());
    d.build_logic_grid(r[1], &LogicGrids::default());
    assert_eq!(indexes(&d, r[1]), [2, 0]);
    // Node flags differ: no merge.
    let (mut d, r) = merge_world(&[(false, 0), (false, 2)]);
    let node = LogicGrids {
        floor: grid(3, 3, &[(0, 0, 0x8000_0000)]),
        ..LogicGrids::default()
    };
    d.build_logic_grid(r[0], &node);
    d.build_logic_grid(r[1], &LogicGrids::default());
    assert_eq!(indexes(&d, r[0]), [2, 0]);
    assert_eq!(indexes(&d, r[1]), [5, 0]);
    // A neighbour without info: nothing.
    let (mut d, r) = merge_world(&[(false, 0), (false, 2)]);
    d.build_logic_grid(r[1], &LogicGrids::default());
    assert_eq!(indexes(&d, r[1]), [2, 0]);
    // A one-record neighbour: its only record decides; this room takes
    // index 1.
    let (mut d, r) = merge_world(&[(false, 0), (false, 2)]);
    d.build_logic_one(r[0]);
    d.build_logic_grid(r[1], &LogicGrids::default());
    assert_eq!(indexes(&d, r[1]), [1, 0]);
}

#[test]
fn rename_recurses_through_rooms_near() {
    // A (0) and C (4) are built apart; B (2) touches both: B takes A's
    // index, then C's, and the rename spreads back into A.
    let (mut d, r) = merge_world(&[(false, 0), (false, 2), (false, 4)]);
    d.room_mut(r[0]).near = Some(vec![r[0], r[1]]);
    d.room_mut(r[2]).near = Some(vec![r[1], r[2]]);
    d.build_logic_grid(r[0], &LogicGrids::default());
    d.build_logic_grid(r[2], &LogicGrids::default());
    assert_eq!(indexes(&d, r[2]), [5, 0]);
    d.build_logic_grid(r[1], &LogicGrids::default());
    for &id in &r {
        assert_eq!(indexes(&d, id), [5, 0]);
    }
}

#[test]
fn rename_rules() {
    let (mut d, r) = merge_world(&[(false, 0), (false, 20), (true, 40), (false, 60)]);
    d.build_logic_one(r[0]);
    for &id in &r[1..] {
        d.level_mut(d.room(id).level).coord_counter = 8;
        d.build_logic_grid(id, &LogicGrids::default());
    }
    assert_eq!(indexes(&d, r[1]), [9, 0]);
    // One-record rooms are never renamed.
    d.rename_logic(r[0], 1, 30);
    assert_eq!(indexes(&d, r[0]), [1]);
    // Renamed: recursion into rooms near of the same level id only.
    d.rename_logic(r[1], 9, 30);
    assert_eq!(indexes(&d, r[1]), [30, 0]);
    assert_eq!(indexes(&d, r[2]), [9, 0]);
    assert_eq!(indexes(&d, r[3]), [30, 0]);
}

#[test]
fn wall_records_point_at_the_record_grid() {
    let (mut d, l, _) = world();
    let id = room(&mut d, l, 10, 20, 2, 1);
    d.room_mut(id).tiles.as_mut().unwrap().walls = vec![wall(1, 0), wall(1, 1)];
    d.level_mut(l).coord_counter = 3;
    let g = LogicGrids {
        orientation: grid(3, 2, &[(1, 0, 1), (1, 1, 1)]),
        floor: grid(3, 2, &[(2, 0, 0x01E0_0002)]),
        wall: CellGrid::default(),
    };
    d.build_logic_grid(id, &g);
    // Room-relative cells: (0, 0) is in record 4, (1, 0) in record 5.
    assert_eq!(d.wall_coord(id, 0, 0).map(|r| r.index), Some(4));
    assert_eq!(
        d.wall_coord(id, 1, 0),
        Some(cr(5, false, [11, 20, 13, 22], [11, 20, 12, 21]))
    );
    // A one-record room: 0 (no record pointer).
    let one = room(&mut d, l, 30, 20, 2, 2);
    d.build_logic_one(one);
    assert_eq!(d.wall_coord(one, 0, 0), None);
    // No info: none.
    let bare = room(&mut d, l, 50, 20, 2, 2);
    assert_eq!(d.wall_coord(bare, 0, 0), None);
}

#[test]
fn tree_marks_change_nothing() {
    // A wall record with flag 0x4 and no layer bits (0x1C000 clear) is
    // the tree-mark case; its mark lands in a grid nothing reads again,
    // and it is no blocker (blockers need layer bits exactly 0x4000).
    let (mut d, l, _) = world();
    let plain = room(&mut d, l, 10, 20, 2, 2);
    let tree = room(&mut d, l, 30, 20, 2, 2);
    d.room_mut(tree).tiles.as_mut().unwrap().walls = vec![rec(1, 1, 1, 0x4)];
    for id in [plain, tree] {
        d.build_logic_grid(id, &LogicGrids::default());
    }
    let (a, b) = (
        d.room(plain).logic().unwrap(),
        d.room(tree).logic().unwrap(),
    );
    assert_eq!(a.index_grid.len(), b.index_grid.len());
    // The counter runs on through the second room; compare the shapes.
    let strip = |g: &[u32]| g.iter().map(|&v| v & !0x00FF_FFFF).collect::<Vec<_>>();
    assert_eq!(strip(&a.index_grid), strip(&b.index_grid));
    assert_eq!(a.list.len(), b.list.len());
}

#[test]
fn coordinate_indexes_follow_the_build_order() {
    // The same two rooms, built in the other order, get other indexes.
    let build = |first: usize| {
        let (mut d, l, _) = world();
        let rooms = [room(&mut d, l, 0, 0, 2, 2), room(&mut d, l, 10, 0, 2, 2)];
        d.build_logic_grid(rooms[first], &LogicGrids::default());
        d.build_logic_grid(rooms[1 - first], &LogicGrids::default());
        (indexes(&d, rooms[0]), indexes(&d, rooms[1]))
    };
    assert_eq!(build(0), (vec![2, 0], vec![5, 0]));
    assert_eq!(build(1), (vec![5, 0], vec![2, 0]));
}

#[test]
fn coordinate_lists_by_room_kind_when_the_tiles_fill() {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    let mut types = FakeTypes::default();
    types.rooms.insert(
        2,
        vec![
            preset(0, 0, 8, 8),
            preset(8, 0, 8, 8),
            RoomSpec {
                kind: RoomKind::Other(5),
                ..preset(16, 0, 8, 8)
            },
        ],
    );
    types.default_grid = Some(floor_grid);
    // Room 0: a preset room whose lvlprest has `Logicals` (the grid build).
    let mut g = floor_grid(TileRect::new(0, 0, 8, 8));
    g.logicals = Some(LogicGrids::default());
    types.grids.insert((2, 0), g);
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let l = d.get_or_alloc_level(&w.data, &mut w.types, 2).unwrap();
    d.generate_level(&w.data, &mut w.types, l).unwrap();
    let r = d.level_rooms(l);
    let mut svc = w.svc();
    for &id in &r {
        d.stream_room(&mut svc, id).unwrap();
    }
    // r1: a `Logicals` preset room gets the grid lists; every other
    // preset room one record.
    assert_eq!(d.room(r[0]).logic().unwrap().flags, INFO_GRID);
    assert_eq!(d.room(r[1]).logic().unwrap().flags, INFO_ONE);
    // r3: rooms of other types get no info.
    assert!(d.room(r[2]).logic().is_none());
}

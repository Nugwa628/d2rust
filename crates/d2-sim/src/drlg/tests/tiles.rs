use super::fakes::*;
use crate::drlg::collision::bits;
use crate::drlg::tiles::{cell, rarity_walk, rec_flags, record_flags, RecordKind};
use crate::drlg::*;
use crate::rng::Seed;

const INIT: u32 = 644_409_375;

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

#[test]
fn rarity_walk_rules() {
    assert_eq!(
        (0..4)
            .map(|r| rarity_walk(&[1, 1, 1, 1], r))
            .collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    // A rarity-0 entry is never chosen when another has rarity > 0.
    assert_eq!(
        (0..3).map(|r| rarity_walk(&[0, 3], r)).collect::<Vec<_>>(),
        [1, 1, 1]
    );
    assert_eq!(
        (0..5)
            .map(|r| rarity_walk(&[2, 0, 3], r))
            .collect::<Vec<_>>(),
        [0, 0, 2, 2, 2]
    );
    // One entry: always it.
    assert_eq!(rarity_walk(&[5], 4), 0);
    assert_eq!(rarity_walk(&[0, 0], 0), 0);
}

#[test]
fn record_flag_rules() {
    assert_eq!(record_flags(0, 0, 0, 0, false), 1 << 14);
    let v = cell::LAYER_ABOVE | (2 << 18) | cell::UNWALKABLE | cell::FILL_LOS;
    assert_eq!(
        record_flags(0, 1, v, 0, false),
        (3 << 14) | 0x1 | 0x40 | 0x80
    );
    assert_eq!(record_flags(0, 14, 0, 0, false) & 0x4, 0x4);
    assert_eq!(record_flags(0, 9, 0, 0, false) & 0x2, 0x2);
    assert_eq!(record_flags(0, 13, cell::LINKAGE, 0, true), 0x102);
    assert_eq!(record_flags(0, 13, cell::REVEAL_HIDDEN, 0, true), 0x20C);
    assert_eq!(
        record_flags(
            0,
            13,
            cell::HIDDEN | cell::OBJECT_WALL | cell::LINKED,
            0,
            true
        ),
        0x2808
    );
    assert_eq!(record_flags(0, 13, cell::ENCLOSED, 0b101, true), 0x804);
    // Re-run: OR, and bit 0x8 cleared when the cell is not hidden.
    assert_eq!(record_flags(0x1008, 13, 0, 0, true), 0x1000);
}

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

fn key(main: u32, sub: u32) -> u32 {
    (main << 20) | (sub << 8)
}

#[test]
fn library_order_and_lookup() {
    let (mut w, mut d, r) = one_room(RoomGrids::default(), 2);
    w.data.lvltypes[1][3] = b"second.dt1".to_vec();
    w.tiles.0.insert(
        b"second.dt1".to_vec(),
        vec![tile(0, 0, 0, 7), tile(0, 0, 0, 8)],
    );
    d.room_mut(r).dt1_mask = 0b1001;
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    let e = d.lookup_tiles(r, 0, 0, 0);
    let rar: Vec<u32> = e.iter().map(|&t| d.tile_info(t).rarity).collect();
    // Slot 0 (floor.dt1, 4 tiles in reverse file order), slot 1 (second).
    assert_eq!(e.len(), 6);
    assert_eq!(e[0].index, 3);
    assert_eq!(e[3].index, 0);
    assert_eq!(rar[4..], [8, 7]);
    // 4 library slots: the two files and the fixed three.
    assert_eq!(
        d.room(r).flags & room_flags::TILE_LIB_LOADED,
        room_flags::TILE_LIB_LOADED
    );
}

#[test]
fn lookup_cap_library_full_and_missing_file() {
    let (mut w, mut d, r) = one_room(RoomGrids::default(), 2);
    w.tiles.0.insert(
        b"many.dt1".to_vec(),
        (0..50).map(|_| tile(0, 0, 0, 1)).collect(),
    );
    w.data.lvltypes[1] = vec![b"many.dt1".to_vec(); 32];
    d.room_mut(r).dt1_mask = 0x1FFF_FFFF; // 29 files + 3 fixed = 32
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    assert_eq!(d.lookup_tiles(r, 0, 0, 0).len(), 40);

    let (mut w, mut d, r) = one_room(RoomGrids::default(), 2);
    w.data.lvltypes[1] = vec![b"floor.dt1".to_vec(); 32];
    d.room_mut(r).dt1_mask = 0x3FFF_FFFF; // 30 + 3
    let mut svc = w.svc();
    assert_eq!(d.stream_room(&mut svc, r), Err(DrlgError::LibraryFull));

    let (mut w, mut d, r) = one_room(RoomGrids::default(), 2);
    w.data.lvltypes[1][1] = b"gone.dt1".to_vec();
    d.room_mut(r).dt1_mask = 2;
    let mut svc = w.svc();
    assert_eq!(
        d.stream_room(&mut svc, r),
        Err(DrlgError::MissingDt1("gone.dt1".into()))
    );
}

#[test]
fn choice_fallback_and_no_tile() {
    // Unknown key: falls back to (10, 0, 0) (Warp.dt1), rarity 0: no draw.
    let (mut w, mut d, r) = one_room(
        grids(vec![pass(&[cell::FLOOR | key(9, 9); 9], None, false)]),
        2,
    );
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    let t = d.room(r).tiles().unwrap().floors[0].tile;
    assert_eq!(d.tile_info(t).orientation, 10);
    assert_eq!(
        steps(Seed::init_low(d.room(r).init_seed), d.room(r).seed),
        Some(1)
    );
    let (mut w, mut d, r) = one_room(
        grids(vec![pass(&[cell::FLOOR | key(9, 9); 9], None, false)]),
        2,
    );
    w.tiles.0.insert(tiles::FIXED_LIBRARY[2].to_vec(), vec![]);
    let mut svc = w.svc();
    assert_eq!(d.stream_room(&mut svc, r), Err(DrlgError::NoTile));
    // The fallback's first entry is taken without a draw even when the
    // fallback tiles have rarity.
    let (mut w, mut d, r) = one_room(
        grids(vec![pass(&[cell::FLOOR | key(9, 9); 9], None, false)]),
        2,
    );
    for t in w.tiles.0.get_mut(tiles::FIXED_LIBRARY[2]).unwrap() {
        t.rarity = 3;
    }
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    assert_eq!(
        steps(Seed::init_low(d.room(r).init_seed), d.room(r).seed),
        Some(1)
    );
}

#[test]
fn cell_rules_and_draw_order() {
    // Cell 0: floor + wall (type 3 corner) + shadow: draws floor, 3, 4, 13.
    // Cell 1: floor (30, 0) → hidden, Blank (no draw).
    // Cell 2: exit orientation 10 with main ≥ 8 → nothing.
    // Cell 3: hidden exit (10) → warp unit, room flag 0x800000.
    // Cell 4: hidden door (8) → door unit.
    // Fill blanks: inside cells only; cells 3 and 4 stop at step 3.
    let mut c = [0u32; 9];
    let mut o = [0u32; 9];
    c[0] = cell::FLOOR | cell::WALL | cell::SHADOW | key(1, 0);
    o[0] = 3;
    c[1] = cell::FLOOR | key(30, 0);
    c[2] = cell::WALL | key(8, 0);
    o[2] = 10;
    c[3] = cell::WALL | cell::HIDDEN | key(1, 0);
    o[3] = 10;
    c[4] = cell::WALL | cell::HIDDEN | key(1, 0);
    o[4] = 8;
    let (mut w, mut d, r) = one_room(grids(vec![pass(&c, Some(&o), true)]), 2);
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    let t = d.room(r).tiles().unwrap();
    // Floors: cell 0 (key (1,0) has no type-0 tile → fallback (10,0,0)),
    // cell 1 blank (hidden).
    assert_eq!(
        t.floors.iter().map(|f| (f.x, f.y)).collect::<Vec<_>>(),
        [(0, 0), (1, 0)]
    );
    assert_eq!(d.tile_info(t.floors[0].tile).orientation, 10);
    assert_ne!(t.floors[1].flags & rec_flags::HIDDEN, 0);
    assert_eq!(t.walls.iter().map(|x| x.kind).collect::<Vec<_>>(), [3, 4]);
    assert_eq!(t.walls[0].half, Some(1));
    assert_eq!(t.shadows.len(), 1);
    // Draws: wall 3, wall 4, shadow (2 tiles of rarity 1 each) = 3; the
    // floors are rarity 0 → no draws; plus the active-room step.
    assert_eq!(
        steps(Seed::init_low(d.room(r).init_seed), d.room(r).seed),
        Some(4)
    );
    assert_eq!(w.types.warp_units, [(0, 1)]);
    assert_eq!(w.types.door_units, [(1, 1)]);
    assert_ne!(d.room(r).flags & room_flags::NO_POPULATION, 0);
}

#[test]
fn fill_blank_key_in_arcane_sanctuary() {
    let (mut w, mut d, r) = one_room(grids(vec![pass(&[0; 9], None, true)]), 74);
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    let t = d.room(r).tiles().unwrap();
    assert_eq!(t.floors.len(), 4);
    assert_eq!(d.tile_info(t.floors[0].tile).sub, 1);
}

#[test]
fn kill_edges() {
    let mut g = grids(vec![pass(&[cell::FLOOR; 9], None, false)]);
    g.kill_edge_x = true;
    g.kill_edge_y = true;
    let (mut w, mut d, r) = one_room(g, 2);
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    assert_eq!(d.room(r).tiles().unwrap().floors.len(), 4);
}

/// Two 8×8 rooms side by side (A at x 0, B at x 8) with linked edges.
fn pair(b_first: bool) -> (World, Drlg, DrlgRoomId, DrlgRoomId) {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    let mut types = FakeTypes::default();
    types
        .rooms
        .insert(2, vec![preset(0, 0, 8, 8), preset(8, 0, 8, 8)]);
    types.default_grid = Some(floor_grid);
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let l = d.get_or_alloc_level(&w.data, &mut w.types, 2).unwrap();
    d.generate_level(&w.data, &mut w.types, l).unwrap();
    let r = d.level_rooms(l);
    let mut svc = w.svc();
    let order = if b_first { [r[1], r[0]] } else { [r[0], r[1]] };
    for x in order {
        d.stream_room(&mut svc, x).unwrap();
    }
    (w, d, r[0], r[1])
}

#[test]
fn linked_column_is_shared() {
    // Standalone: 81 draws. Built after its neighbour: the shared 9-cell
    // column costs nothing (9 lower than standalone).
    let (_w, d, a, b) = pair(false);
    let n = |r: DrlgRoomId| steps(Seed::init_low(d.room(r).init_seed), d.room(r).seed).unwrap() - 1;
    assert_eq!(n(a), 81);
    assert_eq!(n(b), 72);
    assert_eq!(d.room(a).tiles().unwrap().floor_links.len(), 32);
    assert_eq!(d.room(b).tiles().unwrap().floors.len(), 72);
    let (_w, d, a, b) = pair(true);
    let n = |r: DrlgRoomId| steps(Seed::init_low(d.room(r).init_seed), d.room(r).seed).unwrap() - 1;
    assert_eq!((n(a), n(b)), (72, 81));
}

/// `floor_grid` with every cell on layer 3 (cell bits 18–19) in the room
/// at x = 0, layer 0 elsewhere.
fn layer3_left(r: TileRect) -> RoomGrids {
    let mut g = floor_grid(r);
    if r.x == 0 {
        for c in &mut g.passes[0].cells.cells {
            *c |= 3 << 18;
        }
    }
    g
}

fn layer3_both(r: TileRect) -> RoomGrids {
    let mut g = floor_grid(r);
    for c in &mut g.passes[0].cells.cells {
        *c |= 3 << 18;
    }
    g
}

#[test]
fn linked_layer_field_is_three_bits() {
    // A layer-3 cell's record holds 4 in bits 14-16 (`0x1C000`): a layer-0
    // neighbour cell does not match it and chooses its own
    // tile, a layer-3 neighbour cell does.
    let build = |grid: fn(TileRect) -> RoomGrids| {
        let mut dat = data();
        gen_level(&mut dat, 2, 2);
        let mut types = FakeTypes::default();
        types
            .rooms
            .insert(2, vec![preset(0, 0, 8, 8), preset(8, 0, 8, 8)]);
        types.default_grid = Some(grid);
        let mut w = World::new(dat, types);
        let mut d = w.drlg(INIT);
        let l = d.get_or_alloc_level(&w.data, &mut w.types, 2).unwrap();
        d.generate_level(&w.data, &mut w.types, l).unwrap();
        let r = d.level_rooms(l);
        let mut svc = w.svc();
        for &x in &r {
            d.stream_room(&mut svc, x).unwrap();
        }
        let a = &d.room(r[0]).tiles().unwrap().floors[0];
        assert_eq!(a.flags & 0x1C000, 0x10000);
        steps(Seed::init_low(d.room(r[1]).init_seed), d.room(r[1]).seed).unwrap() - 1
    };
    assert_eq!(build(layer3_left), 81);
    assert_eq!(build(layer3_both), 72);
}

#[test]
fn collision_from_own_and_neighbour_records() {
    let (_w, d, a, b) = pair(false);
    let ga = &d.active_room(a).unwrap().collision;
    let gb = &d.active_room(b).unwrap().collision;
    assert_eq!(ga.rect, TileRect::new(0, 0, 40, 40));
    // Floor tiles flag byte 0 → bottom-left sub-tile of each tile, plus
    // the floor record flags (none of 0x2/0x40/0x80).
    assert_eq!(ga.get(0, 4), Some(bits::WALL));
    assert_eq!(ga.get(0, 0), Some(0));
    assert_eq!(ga.get(4, 4), Some(0));
    // B's column x = 8 records live in A's link list; B's grid gets them
    // through its adjacency list.
    assert_eq!(gb.get(40, 4), Some(bits::WALL));
    assert_eq!(gb.get(75, 39), Some(bits::WALL));
    assert_eq!(d.collision_at(40, 4), Some(bits::WALL));
    assert_eq!(d.collision_at(80, 4), None);
}

#[test]
fn collision_record_flag_bits() {
    let v = cell::FLOOR | cell::UNWALKABLE | cell::FILL_LOS;
    let (mut w, mut d, r) = one_room(grids(vec![pass(&[v; 9], None, false)]), 2);
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    let g = &d.active_room(r).unwrap().collision;
    assert_eq!(g.get(7, 7), Some(bits::WALL | bits::MISSILE_BARRIER));
    assert_eq!(g.masks.len(), 100);
}

#[test]
fn blank_floor_rechosen_on_neighbour_seed_with_collision_update() {
    // A = (0, 8) built first; its top row (world y 8) is linked blank
    // floor (30, 0): records hidden, Blank tile, no draws. B = (0, 0) built
    // second: its bottom row (y 8) finds A's records; R shows (30, 0), so
    // each is re-chosen on A's seed with B's key (0, 0) and A's collision
    // grid updated (0x20 cleared, 1 set at the bottom-left sub-tile).
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    let mut types = FakeTypes::default();
    types
        .rooms
        .insert(2, vec![preset(0, 8, 8, 8), preset(0, 0, 8, 8)]);
    let mut ga = CellGrid::new(9, 9);
    let mut gb = CellGrid::new(9, 9);
    for x in 0..9 {
        ga.set(x, 0, cell::FLOOR | cell::LINKED | key(30, 0));
        gb.set(x, 8, cell::FLOOR | cell::LINKED);
    }
    let g = |c: CellGrid| RoomGrids {
        passes: vec![GridPass {
            cells: c,
            orientation: None,
            fill_blanks: false,
        }],
        ..RoomGrids::default()
    };
    types.grids.insert((2, 0), g(ga));
    types.grids.insert((2, 1), g(gb));
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let l = d.get_or_alloc_level(&w.data, &mut w.types, 2).unwrap();
    d.generate_level(&w.data, &mut w.types, l).unwrap();
    let r = d.level_rooms(l);
    let (a, b) = (r[0], r[1]);
    let mut svc = w.svc();
    d.stream_room(&mut svc, a).unwrap();
    let seed_a = d.room(a).seed;
    assert_eq!(steps(Seed::init_low(d.room(a).init_seed), seed_a), Some(1));
    assert_eq!(d.active_room(a).unwrap().collision.get(0, 40), Some(0x20));
    d.stream_room(&mut svc, b).unwrap();
    // 9 draws on A's seed, none on B's (B has no other cells).
    assert_eq!(steps(seed_a, d.room(a).seed), Some(9));
    // B made no record of its own: no active room, no draw.
    assert_eq!(
        steps(Seed::init_low(d.room(b).init_seed), d.room(b).seed),
        Some(0)
    );
    assert!(d.active_room(b).is_none());
    let rec = d.room(a).tiles().unwrap().floors[0];
    assert_eq!(d.tile_info(rec.tile).main, 0);
    let g = &d.active_room(a).unwrap().collision;
    assert_eq!(g.get(0, 44), Some(bits::WALL));
    assert_eq!(g.get(0, 40), Some(0));
    // The re-run flag rules clear hidden (B's cell is not hidden).
    assert_eq!(rec.flags & rec_flags::HIDDEN, 0);
}

fn wall_pair(remap: WallRemap) -> Result<(World, Drlg, DrlgRoomId, DrlgRoomId), DrlgError> {
    // A = (0, 0) with a linked type-1 wall column at x 8; B = (8, 0) with a
    // linked type-2 wall column at x 0.
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    dat.wall_remap = remap;
    let mut types = FakeTypes::default();
    types
        .rooms
        .insert(2, vec![preset(0, 0, 8, 8), preset(8, 0, 8, 8)]);
    let mut ca = CellGrid::new(9, 9);
    let mut oa = CellGrid::new(9, 9);
    let mut cb = CellGrid::new(9, 9);
    let mut ob = CellGrid::new(9, 9);
    for y in 1..8 {
        ca.set(8, y, cell::WALL | cell::LINKED | key(1, 0));
        oa.set(8, y, 1);
        cb.set(0, y, cell::WALL | cell::LINKED | key(1, 0));
        ob.set(0, y, 2);
    }
    let g = |c: CellGrid, o: CellGrid| RoomGrids {
        passes: vec![
            GridPass {
                cells: CellGrid {
                    width: 9,
                    height: 9,
                    cells: vec![cell::FLOOR; 81],
                },
                orientation: None,
                fill_blanks: false,
            },
            GridPass {
                cells: c,
                orientation: Some(o),
                fill_blanks: false,
            },
        ],
        ..RoomGrids::default()
    };
    types.grids.insert((2, 0), g(ca, oa));
    types.grids.insert((2, 1), g(cb, ob));
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let l = d.get_or_alloc_level(&w.data, &mut w.types, 2).unwrap();
    d.generate_level(&w.data, &mut w.types, l).unwrap();
    let r = d.level_rooms(l);
    let mut svc = w.svc();
    d.stream_room(&mut svc, r[0])?;
    d.stream_room(&mut svc, r[1])?;
    Ok((w, d, r[0], r[1]))
}

#[test]
fn wall_remap_is_the_transcribed_table() {
    use crate::drlg::WallClass::{Keep, Stop, Table};
    let m = WallRemap::original();
    assert_eq!(DrlgData::default().wall_remap, m);
    // Wall-remap test vectors.
    let at = |t: u32, r: usize| match m.class(t) {
        Some(Table(row)) => row[r],
        c => panic!("type {t}: {c:?}"),
    };
    assert_eq!(at(1, 5), 1);
    assert_eq!(at(5, 6), 6);
    for r in [1, 2, 3, 5, 6, 7] {
        assert_eq!(at(3, r), 3);
    }
    assert_eq!(at(2, 0), 1, "r0 = row 0's r7");
    assert_eq!(at(1, 0), 0, "r0 of wall-remap row 0 is zero");
    for t in [0, 4, 10, 11, 12, 14, 15, 16, 17, 18, 19] {
        assert_eq!(m.class(t), Some(Keep), "type {t}");
    }
    for t in [8, 9, 13] {
        assert_eq!(m.class(t), Some(Stop), "type {t}");
    }
    assert_eq!(m.class(20), None);
    // Rows (values equal D2MOO's `nWallTileTypeRemap`).
    let rows: Vec<[u32; 7]> = [1, 2, 3, 5, 6, 7]
        .into_iter()
        .map(|t| core::array::from_fn(|c| at(t, c + 1)))
        .collect();
    assert_eq!(
        rows,
        [
            [1, 3, 3, 4, 1, 3, 1],
            [1, 2, 3, 4, 3, 2, 2],
            [3, 3, 3, 4, 3, 3, 3],
            [1, 3, 3, 4, 5, 6, 1],
            [3, 2, 3, 4, 3, 6, 2],
            [1, 2, 3, 4, 1, 2, 7],
        ]
    );
    assert_eq!(WallRemap::with_rows(rows.clone().try_into().unwrap()), m);
}

#[test]
fn wall_remap_parse_is_strict() {
    use crate::drlg::data::WALL_REMAP_TSV;
    assert!(WallRemap::parse(WALL_REMAP_TSV).is_ok());
    let bad = [
        WALL_REMAP_TSV.replacen("new_type", "type", 1),
        WALL_REMAP_TSV.replacen("\n4\tkeep", "\n5\tkeep", 1),
        WALL_REMAP_TSV.replacen("0\tkeep", "0\tsame", 1),
        WALL_REMAP_TSV.replacen("1\ttable\t0", "1\ttable\tx", 1),
        WALL_REMAP_TSV.replacen("0\tkeep\t", "0\tkeep\t1", 1),
        WALL_REMAP_TSV
            .trim_end()
            .rsplit_once('\n')
            .unwrap()
            .0
            .to_string(),
        format!("{WALL_REMAP_TSV}20\tkeep\t\t\t\t\t\t\t\t\n"),
    ];
    for (i, b) in bad.iter().enumerate() {
        assert_ne!(b, WALL_REMAP_TSV, "perturbation {i} changed nothing");
        assert!(WallRemap::parse(b).is_err(), "perturbation {i}");
    }
}

#[test]
fn wall_merge_to_corner() {
    // Remap row for new type 2 (index 1), column R.type 1 → 3.
    let mut table = [[0u32; 7]; 6];
    table[1][0] = 3;
    let (_w, d, a, b) = wall_pair(WallRemap::with_rows(table)).unwrap();
    let ta = d.room(a).tiles().unwrap();
    let tb = d.room(b).tiles().unwrap();
    // A's 7 linked walls became type 3, flagged layer 3 + hidden.
    let linked: Vec<_> = ta
        .other_links
        .iter()
        .map(|&(k, i)| ta.records(k)[i])
        .collect();
    assert_eq!(linked.len(), 7);
    assert!(linked
        .iter()
        .all(|r| r.kind == 3 && r.flags & 0xC008 == 0xC000));
    // B got a new type-3 record with its type-4 half for each cell.
    assert_eq!(tb.walls.iter().filter(|r| r.kind == 3).count(), 7);
    assert_eq!(tb.walls.iter().filter(|r| r.kind == 4).count(), 7);
    // Each pair is in the chain, the half after its record (chain cases C1, C5).
    assert_eq!(tb.other_links.len(), 14);
    assert!(tb.other_links.iter().all(|&(k, _)| k == RecordKind::Wall));
}

#[test]
fn animated_tiles() {
    let mut g = grids(vec![pass(&[cell::FLOOR | key(2, 0); 9], None, false)]);
    g.animate = true;
    let (mut w, mut d, r) = one_room(g, 2);
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    d.build_near(svc.data, svc.types, r).unwrap();
    // Each cell drew roll(3) (frames 0 + 1 + 2), result overwritten.
    assert_eq!(
        steps(Seed::init_low(d.room(r).init_seed), d.room(r).seed),
        Some(10)
    );
    let t = d.room(r).tiles().unwrap();
    assert_eq!(t.floors.len(), 27);
    assert_eq!(t.anims.len(), 9);
    // Head insertion: the last cell's entry first.
    assert_eq!(t.anims[0].frames, [8, 25, 26]);
    assert_eq!(t.anims[0].speed, 80);
    assert_eq!(d.tile_info(t.floors[8].tile).rarity, 0);
    assert_eq!(d.tile_info(t.floors[26].tile).rarity, 2);
    assert_ne!(t.floors[26].flags & rec_flags::HIDDEN, 0);
    assert_ne!(d.room(r).flags & room_flags::ANIMATED, 0);
    for _ in 0..4 {
        d.animate_tiles(r);
    }
    let t = d.room(r).tiles().unwrap();
    assert_eq!(t.anims[0].pos, 320);
    assert_ne!(t.floors[8].flags & rec_flags::HIDDEN, 0);
    assert_eq!(t.floors[25].flags & rec_flags::HIDDEN, 0);
    for _ in 0..6 {
        d.animate_tiles(r);
    }
    // 800 mod 768 = 32: frame 0 again.
    let t = d.room(r).tiles().unwrap();
    assert_eq!(t.anims[0].pos, 32);
    assert_eq!(t.floors[8].flags & rec_flags::HIDDEN, 0);
    // Phase copy: from no room, every entry restarts at 0;
    // the shown frame (0, from pos 32) keeps its flags.
    d.animate_tiles(r);
    d.animate_tiles(r);
    d.animate_tiles(r);
    let shown = |d: &Drlg| {
        let t = d.room(r).tiles().unwrap();
        [8, 25, 26].map(|i| t.floors[i].flags & rec_flags::HIDDEN == 0)
    };
    assert_eq!(d.room(r).tiles().unwrap().anims[0].pos, 272);
    assert_eq!(shown(&d), [false, true, false]);
    d.copy_anim_phase(None, r);
    let t = d.room(r).tiles().unwrap();
    assert!(t.anims.iter().all(|a| a.pos == 0));
    assert_eq!(shown(&d), [false, true, false]);
    // From a room with entries: the head entry's position.
    d.animate_tiles(r);
    d.copy_anim_phase(Some(r), r);
    assert!(d.room(r).tiles().unwrap().anims.iter().all(|a| a.pos == 80));
}

#[test]
fn missing_animation_frame_is_fatal() {
    let mut g = grids(vec![pass(&[cell::FLOOR | key(2, 0); 9], None, false)]);
    g.animate = true;
    let (mut w, mut d, r) = one_room(g, 2);
    let f = w.tiles.0.get_mut(&b"floor.dt1"[..]).unwrap();
    let last = f.len() - 1;
    f[last].rarity = 5;
    let mut svc = w.svc();
    assert_eq!(d.stream_room(&mut svc, r), Err(DrlgError::MissingFrame(2)));
}

#[test]
fn client_copy_frees_tiles_on_status_4() {
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
    let mut svc = w.svc();
    let c = crate::units::ClientId(0);
    d.client_changes_room(&mut svc, c, None, Some(r[0]))
        .unwrap();
    assert!(d.room(r[1]).tiles().is_some());
    d.client_changes_room(&mut svc, c, Some(r[0]), None)
        .unwrap();
    // Client copy: status 4 frees tiles (and the active room).
    assert!(d.room(r[1]).tiles().is_none());
    assert!(d.room(r[1]).active().is_none());
    assert_eq!(d.freed_rooms, 2);
}

#[test]
fn entry_identity_carries_roof_height_and_height() {
    // OQ 17 vector: a DT1 with 3 tiles whose tile 2 has key (1, 0, 0),
    // roof height 0, height −80, loaded into slot 0 → the lookup for
    // (1, 0, 0) returns (slot 0's path, index 2), and the record reads
    // roof height 0, height −80 from it.
    let (mut w, mut d, r) = one_room(RoomGrids::default(), 2);
    w.data.lvltypes[1][0] = b"three.dt1".to_vec();
    let mut t2 = tile(1, 0, 0, 1);
    t2.roof_height = 0;
    t2.height = -80;
    let mut t0 = tile(0, 5, 0, 1);
    t0.roof_height = 0x30;
    w.tiles
        .0
        .insert(b"three.dt1".to_vec(), vec![t0, tile(0, 6, 0, 1), t2]);
    d.room_mut(r).dt1_mask = 0b1;
    let mut svc = w.svc();
    d.stream_room(&mut svc, r).unwrap();
    let e = d.lookup_tiles(r, 1, 0, 0);
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].index, 2);
    assert_eq!(d.dt1_path(e[0]), b"three.dt1");
    let info = d.tile_info(e[0]);
    assert_eq!((info.roof_height, info.height), (0, -80));
    let first = d.lookup_tiles(r, 0, 5, 0);
    assert_eq!(d.tile_info(first[0]).roof_height, 0x30);
}

//! Unit tests from the path-core rules and the edge cases they own, on
//! synthetic rooms (the P/F/D vectors belong to the searches).

use crate::drlg::collision::bits;
use crate::drlg::{CollisionGrid, TileRect};
use crate::units::{RoomId, UnitId, UnitType};

use super::collision::*;
use super::coords::*;
use super::footprint::*;
use super::record::*;
use super::tables::{PathTables, PATH_TABLES_TSV};
use super::PathError;

/// Rooms for tests: index = `RoomId.0`; `None` rect = not active.
#[derive(Default)]
struct Rooms {
    rooms: Vec<(Option<TileRect>, Vec<RoomId>, Option<CollisionGrid>)>,
}

impl Rooms {
    /// Add an active room with a zeroed grid; returns its id.
    fn add(&mut self, x: i32, y: i32, w: i32, h: i32) -> RoomId {
        let rect = TileRect::new(x, y, w, h);
        self.rooms
            .push((Some(rect), Vec::new(), Some(CollisionGrid::new(rect))));
        RoomId(self.rooms.len() as u32 - 1)
    }
    fn adj(&mut self, r: RoomId, list: &[RoomId]) {
        self.rooms[r.0 as usize].1 = list.to_vec();
    }
    /// The mask of a cell, wherever it lives.
    fn at(&self, x: i32, y: i32) -> u16 {
        self.rooms
            .iter()
            .find_map(|r| r.2.as_ref().and_then(|g| g.get(x, y)))
            .expect("cell in a room")
    }
    fn set(&mut self, x: i32, y: i32, v: u16) {
        for r in &mut self.rooms {
            if let Some(m) = r.2.as_mut().and_then(|g| g.get_mut(x, y)) {
                *m = v;
                return;
            }
        }
        panic!("cell ({x}, {y}) in no room");
    }
}

impl CollisionRooms for Rooms {
    fn subtile_rect(&self, room: RoomId) -> Option<TileRect> {
        self.rooms.get(room.0 as usize)?.0
    }
    fn adjacent_count(&self, room: RoomId) -> usize {
        self.rooms.get(room.0 as usize).map_or(0, |r| r.1.len())
    }
    fn adjacent(&self, room: RoomId, i: usize) -> Option<RoomId> {
        self.rooms.get(room.0 as usize)?.1.get(i).copied()
    }
    fn grid(&self, room: RoomId) -> Option<&CollisionGrid> {
        self.rooms.get(room.0 as usize)?.2.as_ref()
    }
    fn grid_mut(&mut self, room: RoomId) -> Option<&mut CollisionGrid> {
        self.rooms.get_mut(room.0 as usize)?.2.as_mut()
    }
}

/// One room [0, 20) × [0, 20), no neighbours (the spec's synthetic room).
fn one_room() -> (Rooms, RoomId) {
    let mut w = Rooms::default();
    let a = w.add(0, 0, 20, 20);
    w.adj(a, &[a]);
    (w, a)
}

/// Four rooms of 10 × 10: A (0,0), B (10,0), C (0,10), D (10,10); each
/// adjacent to all four, itself first.
fn four_rooms() -> (Rooms, [RoomId; 4]) {
    let mut w = Rooms::default();
    let a = w.add(0, 0, 10, 10);
    let b = w.add(10, 0, 10, 10);
    let c = w.add(0, 10, 10, 10);
    let d = w.add(10, 10, 10, 10);
    w.adj(a, &[a, b, c, d]);
    w.adj(b, &[b, a, c, d]);
    w.adj(c, &[c, a, b, d]);
    w.adj(d, &[d, a, b, c]);
    (w, [a, b, c, d])
}

fn tables() -> PathTables {
    PathTables::spec().expect("embedded tables parse")
}

/// Cells of the grid that are non-zero, sorted (x, y, value).
fn nonzero(w: &Rooms) -> Vec<(i32, i32, u16)> {
    let mut out = Vec::new();
    for r in &w.rooms {
        if let Some(g) = &r.2 {
            for y in g.rect.y..g.rect.y + g.rect.h {
                for x in g.rect.x..g.rect.x + g.rect.w {
                    let v = g.get(x, y).unwrap();
                    if v != 0 {
                        out.push((x, y, v));
                    }
                }
            }
        }
    }
    out.sort();
    out
}

fn plus_at(x: i32, y: i32) -> Vec<(i32, i32)> {
    let mut v = vec![(x, y), (x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)];
    v.sort();
    v
}

fn box_at(x: i32, y: i32) -> Vec<(i32, i32)> {
    let mut v = Vec::new();
    for dx in -1..=1 {
        for dy in -1..=1 {
            v.push((x + dx, y + dy));
        }
    }
    v.sort();
    v
}

// ---- tables ----------------------------------------------------------------

#[test]
fn tables_parse_with_shapes() {
    let t = tables();
    assert_eq!(PATH_TABLES_TSV.lines().count(), 710);
    assert_eq!(t.pattern_of_size, vec![0, 1, 1, 2]);
    assert_eq!(t.pathtype_flags[7], 0x21900);
    assert_eq!(t.pathtype_flags[4], 0x60000);
    assert_eq!(t.pathtype_diroff[5], 2);
    assert_eq!(t.pathtype_diroff[6], -2);
    assert_eq!(t.field_dx.len(), 9);
    assert_eq!(t.tan.len(), 128);
    assert_eq!(t.tan[0], [0, 4096, 0]);
    assert_eq!(t.velmod_player.len(), 20);
    assert_eq!(t.animstat[0], [1, 120, 93]);
}

// M08: the strict parser reports each kind of perturbation.
#[test]
fn tables_tsv_check_catches_perturbations() {
    let good = PATH_TABLES_TSV;
    let bad = [
        good.replacen("table\t", "tabel\t", 1),
        // Index out of order.
        good.replacen("pattern_of_size\t1\t1", "pattern_of_size\t2\t1", 1),
        // Non-integer value.
        good.replacen("pattern_of_size\t1\t1", "pattern_of_size\t1\tx", 1),
        // A column that must be empty.
        good.replacen("pattern_of_size\t1\t1\t", "pattern_of_size\t1\t1\t5", 1),
        // Address off the stride.
        good.replacen("0x006EB3E0", "0x006EB3E1", 1),
        // Unknown table, extra row, missing row.
        format!("{good}other\t0\t0\t\t\t\t\t0x0\n"),
        format!("{good}field_dx\t9\t0\t\t\t\t\t0x007497A4\n"),
        good.replacen("pattern_of_size\t3\t2\t\t\t\t\t0x006EB3E8\n", "", 1),
        // Negative flags.
        good.replacen("pathtype_flags\t2\t0\t", "pathtype_flags\t2\t-1\t", 1),
    ];
    for (i, b) in bad.iter().enumerate() {
        assert_ne!(b, good, "perturbation {i} must change the text");
        assert!(
            matches!(PathTables::from_tsv(b), Err(PathError::Tsv(_))),
            "perturbation {i}"
        );
    }
    // A changed value parses and changes exactly that entry.
    let t =
        PathTables::from_tsv(&good.replacen("pattern_of_size\t3\t2", "pattern_of_size\t3\t1", 1))
            .unwrap();
    let mut want = tables();
    want.pattern_of_size[3] = 1;
    assert_eq!(t, want);
}

// Ported from the walk tables (709 rows; the
// type table and the values the walk code reads).
#[test]
fn tables_parse_and_match_spec_values() {
    let t = tables();
    assert_eq!(PATH_TABLES_TSV.lines().count() - 1, 709);
    // Type table: flags of types 0, 1, 7, 8, 11 and offsets of 5, 6, 12.
    assert_eq!(t.pathtype_flags[0], 0x21900);
    assert_eq!(t.pathtype_flags[1], 0x1900);
    assert_eq!(t.pathtype_flags[7], 0x21900);
    assert_eq!(t.pathtype_flags[8], 0x1E600);
    assert_eq!(t.pathtype_flags[11], 0x1E604);
    assert_eq!(t.pathtype_diroff[5], 2);
    assert_eq!(t.pathtype_diroff[6], -2);
    assert_eq!(t.pathtype_diroff[12], -4);
    // Steps.
    assert_eq!(
        t.dir8_toward,
        vec![
            [1, 0],
            [1, 1],
            [0, 1],
            [-1, 1],
            [-1, 0],
            [-1, -1],
            [0, -1],
            [1, -1]
        ]
    );
    assert_eq!(t.dir8_target, t.dir8_toward);
    assert_eq!(t.dist8_unit, t.dist8_path);
    // tan (x, y, angle); animstat (has base, base, stat).
    assert_eq!(t.tan[127][0], 2896);
    assert_eq!(t.animstat[4][1], 150);
    assert_eq!(t.animstat[4][2], 96);
}

// Ported from the walk tables: M08, one changed value is seen exactly
// there; a dropped row, a repeated row, an unknown table and a bad header
// are errors.
#[test]
fn tables_parse_rejects_walk_perturbations() {
    let changed = PATH_TABLES_TSV.replacen("snap9\t40\t0\t", "snap9\t40\t7\t", 1);
    assert_ne!(changed, PATH_TABLES_TSV);
    let t2 = PathTables::from_tsv(&changed).unwrap();
    let t = tables();
    assert_eq!(t2.snap9[40], 7);
    let mut back = t2.clone();
    back.snap9[40] = t.snap9[40];
    assert_eq!(back, t);
    let dropped: String = PATH_TABLES_TSV
        .lines()
        .filter(|l| !l.starts_with("tan\t5\t"))
        .map(|l| format!("{l}\n"))
        .collect();
    let dup = format!("{PATH_TABLES_TSV}tan\t5\t1\t2\t3\t\t\t0x0\n");
    let unknown = format!("{PATH_TABLES_TSV}nope\t0\t1\t\t\t\t\t0x0\n");
    for (name, bad) in [
        ("dropped", dropped.as_str()),
        ("repeated", dup.as_str()),
        ("unknown", unknown.as_str()),
        ("header", "x\n"),
    ] {
        assert!(
            matches!(PathTables::from_tsv(bad), Err(PathError::Tsv(_))),
            "{name}"
        );
    }
}

// ---- coordinates --------------------------------------------------------

#[test]
fn subtiles_and_precise() {
    assert_eq!(tile_to_subtile(3), 15);
    assert_eq!(tile_to_subtile(-2), -10);
    assert_eq!(to_fp16_center(10), 0x000A_8000);
    assert_eq!(subtile_of(0x000A_8000), 10);
    assert_eq!(subtile_of(0x000A_FFFF), 10);
    assert_eq!(subtile_of(0x000B_0000), 11);
    assert_eq!(dist_sq(3, -4), 25);
    assert_eq!(dist_sq(0, 0), 0);
}

#[test]
fn client_coordinates() {
    // a = 100·32 + 16 = 3216, b = 50·32 + 16 = 1616.
    let (px, py) = (to_fp16_center(100), to_fp16_center(50));
    assert_eq!(client_from_precise(px, py), (800, 1208));
    // a − b < 0: arithmetic shifts.
    assert_eq!(client_from_precise(py, px), (-800, 1208));
    // Fractions below 1/32 sub-tile vanish: >> 11.
    assert_eq!(client_from_precise(0x7FF, 0), (0, 0));
    assert_eq!(client_from_precise(0x800, 0), (0, 0));
    assert_eq!(client_from_precise(0x1000, 0), (1, 0));
    assert_eq!(client_from_subtile(100, 50), (800, 1200));
    assert_eq!(client_from_subtile(50, 100), (-800, 1200));
}

// ---- path records -------------------------------------------------------

#[test]
fn path_kind_position_and_room() {
    for (t, k) in [
        (UnitType::Player, PathKind::Dynamic),
        (UnitType::Monster, PathKind::Dynamic),
        (UnitType::Missile, PathKind::Dynamic),
        (UnitType::Object, PathKind::Static),
        (UnitType::Item, PathKind::Static),
        (UnitType::Tile, PathKind::Static),
    ] {
        assert_eq!(PathKind::of(t), k, "{t:?}");
    }
    assert_eq!(unit_position(None), (0, 0));
    let mut s = StaticPath::default();
    s.set(Some(RoomId(3)), 7, 9);
    let sp = UnitPath::Static(s);
    assert_eq!((sp.position(), sp.room()), ((7, 9), Some(RoomId(3))));
    let d = DynamicPath {
        precise_x: 0x0012_8000,
        precise_y: 0x0034_1234,
        room: Some(RoomId(5)),
        ..DynamicPath::default()
    };
    let dp = UnitPath::Dynamic(Box::new(d));
    assert_eq!(unit_position(Some(&dp)), (0x12, 0x34));
    assert_eq!(dp.room(), Some(RoomId(5)));
}

#[test]
fn static_path_set() {
    let mut s = StaticPath {
        room_changed: 1,
        direction: 4,
        ..StaticPath::default()
    };
    s.set(Some(RoomId(1)), 100, 50);
    assert_eq!(s.room, Some(RoomId(1)));
    assert_eq!((s.x, s.y), (100, 50));
    assert_eq!((s.client_x, s.client_y), (800, 1200));
    assert_eq!(s.room_changed, 0);
    assert_eq!(s.direction, 4);
}

#[test]
fn dynamic_path_fields() {
    let mut p = DynamicPath::default();
    assert_eq!(p.points.len(), 78);
    assert_eq!(p.saved_steps.len(), 10);
    p.target_unit = Some(TargetUnit {
        unit: UnitId(4),
        ty: UnitType::Monster,
        guid: 4,
    });
    p.set_target_point(12, 34);
    assert_eq!((p.target_x, p.target_y, p.target_unit), (12, 34, None));
    p.precise_x = to_fp16_center(100);
    p.precise_y = to_fp16_center(50);
    p.update_client();
    assert_eq!((p.client_x, p.client_y), (800, 1208));
    assert_eq!((p.x(), p.y()), (100, 50));
}

#[test]
fn set_path_type_rules() {
    let t = tables();
    let mut p = DynamicPath {
        path_type: 3,
        velocity: 0x900,
        flags: 0x20 | 0x100,
        ..DynamicPath::default()
    };
    // Type 8 flags 0x1E600: has 0x2000 and 0x8000, path lacks 0x4000 /
    // 0x10000 → previous type and saved velocity stored.
    p.set_path_type(&t, false, 8).unwrap();
    assert_eq!(
        (p.path_type, p.prev_path_type, p.saved_velocity),
        (8, 3, 0x900)
    );
    assert_eq!(p.flags, 0x20 | 0x1E600);
    // Now previous type 3 → fine; type 9 (0x1E800): path has 0x4000 and
    // 0x10000 → neither is stored again.
    p.velocity = 0x400;
    p.set_path_type(&t, false, 9).unwrap();
    assert_eq!(
        (p.path_type, p.prev_path_type, p.saved_velocity),
        (9, 3, 0x900)
    );
    assert_eq!(p.flags, 0x20 | 0x1E800);
    // Direction offset from the table.
    p.set_path_type(&t, false, 6).unwrap();
    assert_eq!(p.dir_offset, -2);
    // Fatal asserts.
    let mut q = DynamicPath::default();
    assert_eq!(q.set_path_type(&t, true, 2), Err(PathError::PathType(2)));
    q.prev_path_type = 11;
    assert_eq!(q.set_path_type(&t, false, 1), Err(PathError::PathType(1)));
    let mut m = DynamicPath {
        max_distance: 78,
        ..DynamicPath::default()
    };
    assert_eq!(m.set_path_type(&t, false, 4), Err(PathError::PathType(4)));
    assert_eq!(q.set_path_type(&t, false, 18), Err(PathError::PathType(18)));
}

#[test]
fn alloc_player_path() {
    let t = tables();
    let (mut w, a) = one_room();
    let p = alloc_dynamic_path(
        &t,
        &mut w,
        DynamicKind::Player,
        UnitId(7),
        Some(a),
        5,
        6,
        true,
    )
    .unwrap();
    assert_eq!(p.owner, Some(UnitId(7)));
    assert_eq!((p.unit_size, p.pattern), (2, 1));
    assert_eq!((p.precise_x, p.precise_y), (0x0005_8000, 0x0006_8000));
    assert_eq!((p.velocity, p.room), (0x800, Some(a)));
    assert_eq!(p.saved_count, 1);
    assert_eq!(p.saved_steps[0], PathPoint { x: 5, y: 6 });
    assert_eq!((p.foot_mask, p.move_mask), (0x80, 0x1C09));
    assert_eq!((p.path_type, p.dir_offset), (7, 0));
    assert_eq!(p.flags, 0x21900 | 0x10);
    assert_eq!((p.max_distance, p.ida_score), (73, 70));
    assert_eq!(
        (p.client_x, p.client_y),
        client_from_precise(p.precise_x, p.precise_y)
    );
    // Footprint: plus of 0x80, NO_PATH on the centre.
    let mut want: Vec<_> = plus_at(5, 6)
        .into_iter()
        .map(|(x, y)| (x, y, 0x80))
        .collect();
    want.iter_mut().find(|c| (c.0, c.1) == (5, 6)).unwrap().2 |= bits::NO_PATH;
    assert_eq!(nonzero(&w), want);
    // No room: nothing stamped, flag 0x10 only when asked.
    let (mut w2, _) = one_room();
    let p = alloc_dynamic_path(
        &t,
        &mut w2,
        DynamicKind::Player,
        UnitId(1),
        None,
        5,
        6,
        false,
    )
    .unwrap();
    assert_eq!((p.room, p.flags), (None, 0x21900));
    assert!(nonzero(&w2).is_empty());
}

#[test]
fn alloc_monster_and_missile_paths() {
    let t = tables();
    let (mut w, a) = one_room();
    let plain = MonsterShape {
        size_x: 2,
        base_id: 0,
        ..MonsterShape::default()
    };
    let p = alloc_dynamic_path(
        &t,
        &mut w,
        DynamicKind::Monster(plain),
        UnitId(1),
        Some(a),
        10,
        10,
        false,
    )
    .unwrap();
    assert_eq!((p.foot_mask, p.move_mask, p.pattern), (0x100, 0x3C01, 1));
    assert_eq!((p.path_type, p.dir_offset, p.flags), (2, 0, 0));
    assert_eq!((p.max_distance, p.ida_score), (14, 0));
    for (shape, mask) in [
        (
            MonsterShape {
                flying: true,
                open_doors: true,
                ..plain
            },
            0x1804,
        ),
        (
            MonsterShape {
                open_doors: true,
                ..plain
            },
            0x3401,
        ),
    ] {
        let (mut w, a) = one_room();
        let p = alloc_dynamic_path(
            &t,
            &mut w,
            DynamicKind::Monster(shape),
            UnitId(1),
            Some(a),
            10,
            10,
            false,
        )
        .unwrap();
        assert_eq!(p.move_mask, mask);
    }
    // Wraith1 (BaseId 38): pattern 5 (no marker), move mask 0x804.
    let (mut w, a) = one_room();
    let wraith = MonsterShape {
        base_id: 38,
        flying: true,
        ..plain
    };
    let p = alloc_dynamic_path(
        &t,
        &mut w,
        DynamicKind::Monster(wraith),
        UnitId(1),
        Some(a),
        10,
        10,
        false,
    )
    .unwrap();
    assert_eq!((p.pattern, p.move_mask), (5, 0x804));
    let want: Vec<_> = plus_at(10, 10)
        .into_iter()
        .map(|(x, y)| (x, y, 0x100))
        .collect();
    assert_eq!(nonzero(&w), want);
    // Missile: masks 0, type 4; a size-3 missile stamps nothing (mask 0).
    let (mut w, a) = one_room();
    let p = alloc_dynamic_path(
        &t,
        &mut w,
        DynamicKind::Missile { size: 3 },
        UnitId(1),
        Some(a),
        10,
        10,
        false,
    )
    .unwrap();
    assert_eq!((p.foot_mask, p.move_mask, p.path_type), (0, 0, 4));
    assert_eq!((p.unit_size, p.pattern), (3, 2));
    assert!(nonzero(&w).is_empty());
}

// ---- size, pattern, mask ------------------------------------------------

#[test]
fn sizes_and_footprint_masks() {
    let obj = ObjectShape {
        size_x: 4,
        size_y: 2,
        ..ObjectShape::default()
    };
    let mons = MonsterShape {
        size_x: -1,
        ..MonsterShape::default()
    };
    let path = DynamicPath {
        foot_mask: 0x1234,
        ..DynamicPath::default()
    };
    for (s, size, mask) in [
        (UnitShape::Player, 2, 0x1234),
        (UnitShape::Monster(mons), -1, 0x1234),
        (UnitShape::Object(obj), 4, 0x400),
        (UnitShape::Missile { size: 5 }, 5, 0x1234),
        (UnitShape::Item, 1, 0x200),
        (UnitShape::Tile, 0, 0x1),
    ] {
        assert_eq!(s.size(), size, "{s:?}");
        assert_eq!(s.foot_mask(Some(&path)), mask, "{s:?}");
    }
    assert_eq!(UnitShape::Player.foot_mask(None), 0);
    // Object masks.
    let m = |is_door, blocks_vis, block_missile, sub_class| {
        ObjectShape {
            is_door,
            blocks_vis,
            block_missile,
            sub_class,
            ..obj
        }
        .foot_mask()
    };
    assert_eq!(m(false, false, false, 0), 0x400);
    assert_eq!(m(false, false, true, 0), 0x404);
    assert_eq!(m(false, true, false, 4), 0x8000);
    // `SubClass` bit 4 returns 0x8000 at once:
    // no `BlockMissile` bit with it.
    assert_eq!(m(false, false, true, 0xC), 0x8000);
    assert_eq!(m(true, true, true, 4), 0x806);
    assert_eq!(m(true, false, true, 4), 0x804);
    assert_eq!(m(true, false, false, 0), 0x400);
    // Pattern of size: table for 0..3, anything else 1.
    let t = tables();
    for (size, pat) in [(0, 0), (1, 1), (2, 1), (3, 2), (4, 1), (100, 1), (-1, 1)] {
        assert_eq!(
            pattern_of_size(&t, size, &UnitShape::Player),
            pat,
            "size {size}"
        );
    }
}

#[test]
fn town_monster_pet_patterns() {
    let t = tables();
    let base = MonsterShape::default();
    let pat = |m: MonsterShape, size| pattern_of_size(&t, size, &UnitShape::Monster(m));
    for town in [
        MonsterShape { npc: true, ..base },
        MonsterShape {
            in_town: true,
            ..base
        },
        MonsterShape {
            unit_flag_31: true,
            ..base
        },
    ] {
        assert!(town.can_be_in_town());
        assert_eq!(
            [pat(town, 0), pat(town, 1), pat(town, 2), pat(town, 3)],
            [0, 3, 3, 4]
        );
        let interact = MonsterShape {
            interact: true,
            ..town
        };
        assert_eq!(pat(interact, 1), 1);
        assert_eq!(pat(interact, 3), 2);
    }
    assert!(!base.can_be_in_town());
    assert_eq!(pat(base, 2), 1);
    // A size outside 0..3 gives 1, then 3 for a town pet.
    assert_eq!(pat(MonsterShape { npc: true, ..base }, 7), 3);
}

// ---- collision queries --------------------------------------------------

#[test]
fn cell_lookup_through_adjacency() {
    let (mut w, [a, b, c, d]) = four_rooms();
    assert_eq!(find_room(&w, Some(a), 5, 5), Some(a));
    assert_eq!(find_room(&w, Some(a), 15, 5), Some(b));
    assert_eq!(find_room(&w, Some(a), 15, 15), Some(d));
    assert_eq!(find_room(&w, Some(b), 5, 15), Some(c));
    assert_eq!(find_room(&w, Some(a), 25, 5), None);
    assert_eq!(find_room(&w, Some(a), -1, 5), None);
    assert_eq!(find_room(&w, None, 5, 5), None);
    // Only the adjacency array is searched: B not adjacent to A.
    w.adj(a, &[a, c]);
    assert_eq!(find_room(&w, Some(a), 15, 5), None);
    // An inactive entry (no rect) is skipped; order decides between two.
    let e = RoomId(w.rooms.len() as u32);
    w.rooms.push((None, Vec::new(), None));
    w.adj(a, &[e, a, b]);
    assert_eq!(find_room(&w, Some(a), 15, 5), Some(b));
    // An inactive room finds nothing, even itself.
    assert_eq!(find_room(&w, Some(e), 5, 5), None);
}

#[test]
fn cell_value_and_missing_room() {
    let (mut w, [a, ..]) = four_rooms();
    w.set(3, 4, 0x0405);
    assert_eq!(point_value(&w, Some(a), 3, 4, 0xFFFF), 0x0405);
    assert_eq!(point_value(&w, Some(a), 3, 4, 0x0001), 0x0001);
    assert_eq!(point_value(&w, Some(a), 3, 4, 0x1000), 0);
    // No room: 0x27 unmasked, even for mask 0.
    assert_eq!(point_value(&w, Some(a), 30, 4, 0x1C09), 0x27);
    assert_eq!(point_value(&w, Some(a), 30, 4, 0), 0x27);
    assert_eq!(point_value(&w, None, 3, 4, 0), 0x27);
    assert!(pattern_collides(&w, Some(a), 30, 4, 0, 0));
    assert!(!pattern_collides(&w, Some(a), 3, 4, 0, 0));
    // A room without a grid also reads 0x27.
    w.rooms[a.0 as usize].2 = None;
    assert_eq!(point_value(&w, Some(a), 3, 4, 0), 0x27);
}

#[test]
fn plus_query_crosses_rooms() {
    let (mut w, [a, b, _, _]) = four_rooms();
    w.set(10, 5, bits::WALL);
    w.set(8, 5, bits::DOOR);
    // Plus at (9, 5) from A: neighbour (10, 5) in B.
    assert_eq!(plus_value(&w, Some(a), 9, 5, 0x1C09), 0x801);
    assert_eq!(plus_value(&w, Some(a), 9, 5, 0x0800), 0x800);
    // A missing neighbour adds 0x27; a missing centre gives 0x27.
    assert_eq!(plus_value(&w, Some(a), 0, 5, 0), 0x27);
    assert_eq!(plus_value(&w, Some(a), 0, 5, 0xFFFF), 0x27);
    assert_eq!(plus_value(&w, Some(a), -1, 5, 0), 0x27);
    // Neighbours are looked up from the centre's room: E right of B is
    // adjacent to B only.
    let e = w.add(20, 0, 10, 10);
    w.adj(b, &[b, a, e]);
    w.adj(e, &[e, b]);
    w.set(20, 5, bits::WALL);
    assert_eq!(plus_value(&w, Some(a), 19, 5, 0xFFFF), 0x1);
    assert_eq!(find_room(&w, Some(a), 20, 5), None);
    // The pattern / size queries use the same plus.
    assert_eq!(pattern_value(&w, Some(a), 19, 5, 1, 0xFFFF), 0x1);
    assert_eq!(size_value(&w, Some(a), 19, 5, 2, 0xFFFF), 0x1);
}

#[test]
fn box_query_clips_at_room_edges() {
    let (mut w, [a, ..]) = four_rooms();
    // One bit per cell of the 3×3 box at (10, 10), corners of all four
    // rooms; the ring around it gets 0x8000 (must not be read).
    for x in 8..=12 {
        for y in 8..=12 {
            w.set(x, y, bits::CORPSE);
        }
    }
    let mut want = 0u16;
    for (i, (x, y)) in box_at(10, 10).into_iter().enumerate() {
        w.set(x, y, 1 << i);
        want |= 1 << i;
    }
    assert_eq!(want, 0x1FF);
    assert_eq!(box_value(&w, Some(a), 10, 10, (3, 3), 0xFFFF), 0x1FF);
    assert_eq!(size_value(&w, Some(a), 10, 10, 3, 0xFFFF), 0x1FF);
    assert_eq!(pattern_value(&w, Some(a), 10, 10, 2, 0xFFFF), 0x1FF);
    assert_eq!(pattern_value(&w, Some(a), 10, 10, 4, 0x00F0), 0xF0);
    // Corners: unsigned halving; 4 × 2 at (10, 10) → x 8..11, y 9..10.
    assert_eq!(super::collision::box_corners(10, 10, 4, 2), (8, 9, 11, 10));
    assert_eq!(
        super::collision::box_corners(10, 10, 1, 1),
        (10, 10, 10, 10)
    );
    let v = box_value(&w, Some(a), 10, 10, (4, 2), 0xFFFF);
    let mut want = 0u16;
    for x in 8..=11 {
        for y in 9..=10 {
            want |= w.at(x, y);
        }
    }
    assert_eq!(v, want);
    // Lower-left without a room → 0x27; a strip without one adds 0x27.
    assert_eq!(box_value(&w, Some(a), 0, 5, (3, 3), 0), 0x27);
    assert_eq!(box_value(&w, Some(a), 19, 5, (3, 3), 0), 0x27);
    assert_eq!(box_value(&w, Some(a), 18, 5, (3, 3), 0), 0);
}

#[test]
fn query_functions_by_shape() {
    let (mut w, a) = one_room();
    // Diagonal neighbour only: seen by the box, not the plus or point.
    w.set(11, 11, bits::WALL);
    for (size, v) in [(0, 0), (1, 0), (2, 0), (3, 1), (4, 0xFFFF), (-1, 0xFFFF)] {
        assert_eq!(
            size_value(&w, Some(a), 10, 10, size, 0xFFFF),
            v,
            "size {size}"
        );
    }
    for (pat, v) in [(0, 0), (1, 0), (2, 1), (3, 0), (4, 1), (5, 0), (6, 0xFFFF)] {
        assert_eq!(
            pattern_value(&w, Some(a), 10, 10, pat, 0xFFFF),
            v,
            "pattern {pat}"
        );
    }
    for (pat, c) in [
        (0, false),
        (1, false),
        (2, true),
        (5, false),
        (6, true),
        (99, true),
    ] {
        assert_eq!(
            pattern_collides(&w, Some(a), 10, 10, pat, 0xFFFF),
            c,
            "pattern {pat}"
        );
    }
    // Object boxes: set and clear bits.
    box_apply(&mut w, Some(a), 10, 10, (2, 3), 0x400, true);
    let mut cells: Vec<_> = nonzero(&w).into_iter().map(|c| (c.0, c.1)).collect();
    cells.sort();
    assert_eq!(
        cells,
        vec![
            (9, 9),
            (9, 10),
            (9, 11),
            (10, 9),
            (10, 10),
            (10, 11),
            (11, 11)
        ]
    );
    assert_eq!(w.at(11, 11), 0x1);
    box_apply(&mut w, Some(a), 10, 10, (2, 3), 0x400, false);
    assert_eq!(nonzero(&w), vec![(11, 11, 0x1)]);
}

#[test]
fn box_set_clear_across_rooms() {
    let (mut w, [a, ..]) = four_rooms();
    box_apply(&mut w, Some(a), 10, 10, (3, 3), 0x8000, true);
    let want: Vec<_> = box_at(10, 10)
        .into_iter()
        .map(|(x, y)| (x, y, 0x8000))
        .collect();
    assert_eq!(nonzero(&w), want);
    // A box whose lower-left has no room is skipped.
    box_apply(&mut w, Some(a), 0, 0, (3, 3), 0x400, true);
    assert_eq!(nonzero(&w), want);
    box_apply(&mut w, Some(a), 10, 10, (3, 3), 0x8000, false);
    assert!(nonzero(&w).is_empty());
}

// ---- footprints ---------------------------------------------------------

#[test]
fn pattern_and_size_stamps() {
    type Cells = Vec<(i32, i32)>;
    let cases: [(u32, Cells, u16, Cells); 6] = [
        // Pattern 0 stamps and clears nothing (empty jump-table
        // entry).
        (0, vec![], 0, vec![]),
        (1, plus_at(10, 10), bits::NO_PATH, vec![(10, 10)]),
        (2, box_at(10, 10), bits::NO_PATH, plus_at(10, 10)),
        (3, plus_at(10, 10), bits::PET, vec![(10, 10)]),
        (4, box_at(10, 10), bits::PET, plus_at(10, 10)),
        (5, plus_at(10, 10), 0, vec![]),
    ];
    for (pat, cells, marker, marked) in cases {
        let (mut w, a) = one_room();
        stamp_pattern(&mut w, Some(a), 10, 10, pat, 0x100);
        let mut want: Vec<_> = cells
            .iter()
            .map(|&(x, y)| {
                (
                    x,
                    y,
                    0x100 | if marked.contains(&(x, y)) { marker } else { 0 },
                )
            })
            .collect();
        want.sort();
        assert_eq!(nonzero(&w), want, "pattern {pat}");
        clear_pattern(&mut w, Some(a), 10, 10, pat, 0x100);
        assert!(nonzero(&w).is_empty(), "pattern {pat} clear");
        // Mask 0: no marker.
        stamp_pattern(&mut w, Some(a), 10, 10, pat, 0);
        assert!(nonzero(&w).is_empty(), "pattern {pat} mask 0");
    }
    // Clear keeps other bits; marker cleared only with a non-zero mask.
    let (mut w, a) = one_room();
    stamp_pattern(&mut w, Some(a), 10, 10, 1, 0x80);
    w.set(11, 10, 0x81);
    clear_pattern(&mut w, Some(a), 10, 10, 1, 0);
    assert_eq!(w.at(10, 10), 0x1080);
    clear_pattern(&mut w, None, 10, 10, 1, 0x80);
    assert_eq!(w.at(10, 10), 0x1080);
    clear_pattern(&mut w, Some(a), 10, 10, 1, 0x80);
    assert_eq!(nonzero(&w), vec![(11, 10, 0x1)]);
    // Cells without a room are skipped.
    let (mut w, a) = one_room();
    stamp_pattern(&mut w, Some(a), 0, 0, 2, 0x100);
    assert_eq!(
        nonzero(&w),
        vec![
            (0, 0, 0x1100),
            (0, 1, 0x1100),
            (1, 0, 0x1100),
            (1, 1, 0x100)
        ]
    );
    // Size stamps: 1 cell, 2 plus, 3 box, others nothing; no marker.
    for (size, cells) in [
        (0, vec![]),
        (1, vec![(10, 10)]),
        (2, plus_at(10, 10)),
        (3, box_at(10, 10)),
        (4, vec![]),
        (-1, vec![]),
    ] {
        let (mut w, a) = one_room();
        stamp_size(&mut w, Some(a), 10, 10, size, 0x200);
        let want: Vec<_> = cells.iter().map(|&(x, y)| (x, y, 0x200)).collect();
        assert_eq!(nonzero(&w), want, "size {size}");
        clear_size(&mut w, Some(a), 10, 10, size, 0x200);
        assert!(nonzero(&w).is_empty());
    }
}

#[test]
fn add_and_remove_per_kind() {
    let (mut w, a) = one_room();
    let fp = |shape| Footprint {
        room: Some(a),
        x: 10,
        y: 10,
        shape,
        mask: 0x400,
    };
    // Object box 2 × 3.
    let obj = fp(FootShape::Box {
        size_x: 2,
        size_y: 3,
    });
    add_footprint(&mut w, &obj);
    assert_eq!(nonzero(&w).len(), 6);
    let shape = ObjectShape {
        has_collision: [false, true, false, false, false, false, false, false],
        ..ObjectShape::default()
    };
    assert!(!remove_footprint(
        &mut w,
        &obj,
        RemoveRule::Object { mode: 0, shape },
        false
    ));
    assert_eq!(nonzero(&w).len(), 6);
    assert!(remove_footprint(
        &mut w,
        &obj,
        RemoveRule::Object { mode: 1, shape },
        false
    ));
    assert!(nonzero(&w).is_empty());
    add_footprint(&mut w, &obj);
    assert!(remove_footprint(
        &mut w,
        &obj,
        RemoveRule::Object { mode: 0, shape },
        true
    ));
    assert!(nonzero(&w).is_empty());
    // Players: modes 0 and 17 keep the footprint unless forced.
    let pl = Footprint {
        mask: 0x80,
        ..fp(FootShape::Pattern(1))
    };
    for (mode, cleared) in [(0, false), (17, false), (1, true), (12, true)] {
        let (mut w, _) = one_room();
        add_footprint(&mut w, &pl);
        assert_eq!(
            remove_footprint(&mut w, &pl, RemoveRule::Player { mode }, false),
            cleared
        );
        assert_eq!(nonzero(&w).is_empty(), cleared, "player mode {mode}");
    }
    // Monsters: modes 0 and 12.
    for (mode, cleared) in [(0, false), (12, false), (17, true), (1, true)] {
        let (mut w, _) = one_room();
        add_footprint(&mut w, &pl);
        assert_eq!(
            remove_footprint(&mut w, &pl, RemoveRule::Monster { mode }, false),
            cleared
        );
    }
    let (mut w, _) = one_room();
    add_footprint(&mut w, &pl);
    assert!(remove_footprint(
        &mut w,
        &pl,
        RemoveRule::Player { mode: 0 },
        true
    ));
    assert!(nonzero(&w).is_empty());
    // Others (size): always.
    let item = Footprint {
        mask: 0x200,
        ..fp(FootShape::Size(1))
    };
    add_footprint(&mut w, &item);
    assert_eq!(nonzero(&w), vec![(10, 10, 0x200)]);
    assert!(remove_footprint(&mut w, &item, RemoveRule::Other, false));
    assert!(nonzero(&w).is_empty());
}

fn player_path(w: &mut Rooms, room: RoomId, x: i32, y: i32) -> DynamicPath {
    alloc_dynamic_path(
        &tables(),
        w,
        DynamicKind::Player,
        UnitId(1),
        Some(room),
        x,
        y,
        false,
    )
    .unwrap()
}

#[test]
fn foot_mask_change_restamps() {
    let (mut w, a) = one_room();
    let mut p = player_path(&mut w, a, 10, 10);
    set_foot_mask(&mut w, &mut p, false, 0x100);
    assert_eq!(p.foot_mask, 0x100);
    let mut want: Vec<_> = plus_at(10, 10)
        .into_iter()
        .map(|(x, y)| (x, y, 0x100))
        .collect();
    want.iter_mut().find(|c| (c.0, c.1) == (10, 10)).unwrap().2 |= bits::NO_PATH;
    assert_eq!(nonzero(&w), want);
    // Pattern set does not restamp.
    set_pattern(&mut p, 2);
    assert_eq!(p.pattern, 2);
    assert_eq!(nonzero(&w), want);
    // Missiles use the size shape.
    let (mut w, a) = one_room();
    let mut m = alloc_dynamic_path(
        &tables(),
        &mut w,
        DynamicKind::Missile { size: 1 },
        UnitId(2),
        Some(a),
        4,
        4,
        false,
    )
    .unwrap();
    set_foot_mask(&mut w, &mut m, true, 0x40);
    assert_eq!(nonzero(&w), vec![(4, 4, 0x40)]);
    set_foot_mask(&mut w, &mut m, true, 0);
    assert!(nonzero(&w).is_empty());
}

#[test]
fn mask_change_after_pattern_change_clears_the_new_shape() {
    // A big monster (pattern 2, box) switched to pattern 1: the mask
    // change clears only the plus and its centre marker; the box corners
    // keep the old mask and the plus sides the old marker.
    let (mut w, a) = one_room();
    let mut p = DynamicPath {
        precise_x: to_fp16_center(10),
        precise_y: to_fp16_center(10),
        room: Some(a),
        pattern: 2,
        foot_mask: 0x100,
        ..DynamicPath::default()
    };
    stamp_pattern(&mut w, Some(a), 10, 10, 2, 0x100);
    set_pattern(&mut p, 1);
    set_foot_mask(&mut w, &mut p, false, 0x200);
    for (x, y) in box_at(10, 10) {
        let v = w.at(x, y);
        if (x, y) == (10, 10) {
            assert_eq!(v, 0x200 | bits::NO_PATH);
        } else if plus_at(10, 10).contains(&(x, y)) {
            // The old pattern's NO_PATH marker stays too.
            assert_eq!(v, 0x200 | bits::NO_PATH, "({x}, {y})");
        } else {
            assert_eq!(v, 0x100, "corner ({x}, {y})");
        }
    }
}

#[test]
fn path_reset_gives_a_wraith_its_size_pattern() {
    let t = tables();
    let (mut w, a) = one_room();
    let wraith = MonsterShape {
        size_x: 2,
        base_id: 38,
        ..MonsterShape::default()
    };
    let mut p = alloc_dynamic_path(
        &t,
        &mut w,
        DynamicKind::Monster(wraith),
        UnitId(1),
        Some(a),
        10,
        10,
        false,
    )
    .unwrap();
    assert_eq!(p.pattern, 5);
    reset_pattern(&mut w, &t, &mut p, &UnitShape::Monster(wraith));
    // Size 2 → pattern 1: the plus with mask 0x100 and the NO_PATH marker.
    assert_eq!(p.pattern, 1);
    for (x, y) in plus_at(10, 10) {
        let marker = if (x, y) == (10, 10) { bits::NO_PATH } else { 0 };
        assert_eq!(w.at(x, y), 0x100 | marker, "({x}, {y})");
    }
    // Without a room only the pattern changes.
    let mut q = DynamicPath {
        pattern: 5,
        unit_size: 2,
        ..DynamicPath::default()
    };
    reset_pattern(&mut w, &t, &mut q, &UnitShape::Monster(wraith));
    assert_eq!(q.pattern, 1);
}

#[test]
fn dead_body_footprint() {
    let (mut w, a) = one_room();
    let mut p = player_path(&mut w, a, 10, 10);
    make_corpse_footprint(&mut w, &mut p);
    assert_eq!((p.pattern, p.foot_mask), (5, 0x8000));
    let want: Vec<_> = plus_at(10, 10)
        .into_iter()
        .map(|(x, y)| (x, y, 0x8000))
        .collect();
    assert_eq!(nonzero(&w), want);
    // Unit removal: forced clear of the corpse.
    let fp = Footprint {
        room: p.room,
        x: p.x(),
        y: p.y(),
        shape: FootShape::Pattern(p.pattern),
        mask: p.foot_mask,
    };
    assert!(remove_footprint(
        &mut w,
        &fp,
        RemoveRule::Player { mode: 0 },
        true
    ));
    assert!(nonzero(&w).is_empty());
}

// ---- moving a footprint -------------------------------------------------

#[test]
fn try_move_tests_then_stamps() {
    let (mut w, a) = one_room();
    stamp_pattern(&mut w, Some(a), 10, 10, 1, 0x80);
    // Move one cell right: the plus overlaps the old one, which is
    // cleared first, so the player's own bit does not block.
    assert_eq!(
        try_move(&mut w, Some(a), (10, 10), (11, 10), 1, 0x80, 0x1C89),
        0
    );
    let mut want: Vec<_> = plus_at(11, 10)
        .into_iter()
        .map(|(x, y)| (x, y, 0x80))
        .collect();
    want.iter_mut().find(|c| (c.0, c.1) == (11, 10)).unwrap().2 |= bits::NO_PATH;
    assert_eq!(nonzero(&w), want);
    // Blocked: result returned, footprint back at old.
    w.set(13, 10, bits::WALL);
    assert_eq!(
        try_move(&mut w, Some(a), (11, 10), (12, 10), 1, 0x80, 0x1C09),
        0x1
    );
    assert_eq!(w.at(11, 10), 0x80 | bits::NO_PATH);
    assert_eq!(w.at(12, 10), 0x80);
    assert_eq!(w.at(13, 10), 0x1);
    // Leaving every room collides (0x27).
    assert_eq!(
        try_move(&mut w, Some(a), (11, 10), (25, 10), 1, 0x80, 0),
        0x27
    );
    assert_eq!(w.at(11, 10), 0x80 | bits::NO_PATH);
}

#[test]
fn forced_and_missile_moves() {
    let (mut w, a) = one_room();
    w.set(5, 5, bits::WALL);
    stamp_pattern(&mut w, Some(a), 3, 5, 1, 0x80);
    forced_move(&mut w, Some(a), (3, 5), (5, 5), 1, 0x80);
    assert_eq!(w.at(5, 5), 0x1 | 0x80 | bits::NO_PATH);
    assert_eq!(w.at(3, 5), 0);
    forced_move(&mut w, None, (5, 5), (8, 8), 1, 0x80);
    assert_eq!(w.at(5, 5), 0x1 | 0x80 | bits::NO_PATH);
    // Missile: 0x1 or 0x4 keeps it at old; other bits move it.
    let (mut w, a) = one_room();
    w.set(5, 5, bits::MISSILE_BARRIER);
    w.set(6, 6, bits::MONSTER);
    stamp_size(&mut w, Some(a), 4, 5, 1, 0x40);
    assert_eq!(
        missile_move(&mut w, Some(a), (4, 5), (5, 5), 1, 0x40, 0x105),
        0x4
    );
    assert_eq!((w.at(4, 5), w.at(5, 5)), (0x40, 0x4));
    assert_eq!(
        missile_move(&mut w, Some(a), (4, 5), (6, 6), 1, 0x40, 0x105),
        0x100
    );
    assert_eq!((w.at(4, 5), w.at(6, 6)), (0, 0x140));
}

/// Records the motion calls and sets the centre position.
#[derive(Default)]
struct Motion {
    calls: Vec<String>,
}

impl PathMotion for Motion {
    fn set_position(&mut self, path: &mut DynamicPath, x: i32, y: i32, hint: Option<RoomId>) {
        self.calls
            .push(format!("set {x} {y} {hint:?} {:#x}", path.flags));
        path.precise_x = to_fp16_center(x);
        path.precise_y = to_fp16_center(y);
        if hint.is_some() {
            path.room = hint;
        }
    }
    fn reset(&mut self, _: &mut DynamicPath) {
        self.calls.push("reset".into());
    }
}

/// The rooms and a motion recorder as one teleport context.
struct Both<'a>(&'a mut Rooms, &'a mut Motion);

impl CollisionRooms for Both<'_> {
    fn subtile_rect(&self, room: RoomId) -> Option<TileRect> {
        self.0.subtile_rect(room)
    }
    fn adjacent_count(&self, room: RoomId) -> usize {
        self.0.adjacent_count(room)
    }
    fn adjacent(&self, room: RoomId, i: usize) -> Option<RoomId> {
        self.0.adjacent(room, i)
    }
    fn grid(&self, room: RoomId) -> Option<&CollisionGrid> {
        self.0.grid(room)
    }
    fn grid_mut(&mut self, room: RoomId) -> Option<&mut CollisionGrid> {
        self.0.grid_mut(room)
    }
}

impl PathMotion for Both<'_> {
    fn set_position(&mut self, path: &mut DynamicPath, x: i32, y: i32, hint: Option<RoomId>) {
        self.1.set_position(path, x, y, hint);
    }
    fn reset(&mut self, path: &mut DynamicPath) {
        self.1.reset(path);
    }
}

#[test]
fn teleport_player_and_missile() {
    let (mut w, [a, b, ..]) = four_rooms();
    let mut p = player_path(&mut w, a, 5, 5);
    w.set(15, 5, bits::WALL);
    let mut m = Motion::default();
    teleport(&mut Both(&mut w, &mut m), &mut p, false, Some(b), 15, 5).unwrap();
    // Forced: lands on the wall; flag 0x1 for the other room.
    assert_eq!(w.at(15, 5), 0x1 | 0x80 | bits::NO_PATH);
    assert_eq!(w.at(5, 5), 0);
    assert_eq!(
        m.calls,
        vec![
            format!("set 15 5 Some({b:?}) {:#x}", 0x21901),
            "reset".into()
        ]
    );
    // Same room: no 0x1 added.
    let mut p = player_path(&mut w, a, 2, 2);
    let mut m = Motion::default();
    teleport_and_clear(&mut Both(&mut w, &mut m), &mut p, false, Some(a), 3, 3).unwrap();
    assert_eq!(p.flags & 0x1, 0);
    assert_eq!(p.point_count, 0);
    assert_eq!(w.at(3, 3), 0x80 | bits::NO_PATH);
    // (0, 0): clear only.
    let mut m = Motion::default();
    teleport(&mut Both(&mut w, &mut m), &mut p, false, Some(a), 0, 0).unwrap();
    assert_eq!(w.at(3, 3), 0);
    assert_eq!(m.calls.len(), 2);
    // A non-zero point without a room: fatal, nothing changed.
    let mut p = player_path(&mut w, a, 6, 6);
    let before = (p.clone(), nonzero(&w));
    assert_eq!(
        teleport(
            &mut Both(&mut w, &mut Motion::default()),
            &mut p,
            false,
            None,
            7,
            7
        ),
        Err(PathError::TeleportNoRoom { x: 7, y: 7 })
    );
    assert_eq!((p, nonzero(&w)), before);
    // Missile: collided mask from the size query, moved flag, saved steps.
    let (mut w, [a, ..]) = four_rooms();
    let mut ms = alloc_dynamic_path(
        &tables(),
        &mut w,
        DynamicKind::Missile { size: 1 },
        UnitId(3),
        Some(a),
        2,
        2,
        false,
    )
    .unwrap();
    ms.foot_mask = 0x40;
    ms.move_mask = 0x5;
    stamp_size(&mut w, Some(a), 2, 2, 1, 0x40);
    w.set(4, 4, bits::WALL);
    teleport(
        &mut Both(&mut w, &mut Motion::default()),
        &mut ms,
        true,
        Some(a),
        4,
        4,
    )
    .unwrap();
    assert_eq!(ms.collided_mask, 0x1);
    assert_eq!(ms.flags & flags::MOVED, flags::MOVED);
    assert_eq!(
        (ms.saved_count, ms.saved_steps[0]),
        (1, PathPoint { x: 4, y: 4 })
    );
    assert_eq!((w.at(2, 2), w.at(4, 4)), (0, 0x41));
}

// ---- answered handoff questions (PC1–PC5, WP4, W5, W6) ---------------------

/// Three rooms in a row, A [0,10), B [10,20), C [20,30) × [0,10): A sees
/// B, B sees A and C, C sees B (A does not see C).
fn three_in_a_row() -> (Rooms, [RoomId; 3]) {
    let mut w = Rooms::default();
    let a = w.add(0, 0, 10, 10);
    let b = w.add(10, 0, 10, 10);
    let c = w.add(20, 0, 10, 10);
    w.adj(a, &[a, b]);
    w.adj(b, &[b, a, c]);
    w.adj(c, &[c, b]);
    (w, [a, b, c])
}

// PC2: every stamped cell is looked up from the room argument, not from
// the centre's room.
#[test]
fn stamp_cells_are_looked_up_from_the_room_argument() {
    let (mut w, [a, b, _]) = three_in_a_row();
    // Centre (19, 5) lies in B; given room A, (20, 5) (in C) is not
    // found from A: skipped.
    stamp_pattern(&mut w, Some(a), 19, 5, 1, 0x80);
    assert_eq!(w.at(20, 5), 0);
    assert_eq!(w.at(19, 5), 0x80 | bits::NO_PATH);
    assert_eq!((w.at(18, 5), w.at(19, 4), w.at(19, 6)), (0x80, 0x80, 0x80));
    // Given room B, C is found.
    let (mut w, [_, b2, _]) = three_in_a_row();
    let _ = b;
    stamp_size(&mut w, Some(b2), 19, 5, 2, 0x40);
    assert_eq!(w.at(20, 5), 0x40);
    // Pattern 0 stamps nothing, the pattern query still tests the point.
    let (mut w, a) = one_room();
    stamp_pattern(&mut w, Some(a), 5, 5, 0, 0x80);
    assert!(nonzero(&w).is_empty());
    w.set(5, 5, 0x1);
    assert_eq!(pattern_value(&w, Some(a), 5, 5, 0, 0x1), 0x1);
}

// PC1: an empty box reads 0x27; the right strip takes the corner beyond
// both edges and the top strip the clipped width.
#[test]
fn box_split_empty_box_and_corner_strip() {
    let (w, [a, ..]) = four_rooms();
    assert_eq!(box_value(&w, Some(a), 5, 5, (0, 3), 0xFFFF), MISSING_ROOM);
    assert_eq!(box_value(&w, Some(a), 5, 5, (3, 0), 0xFFFF), MISSING_ROOM);
    // The set / clear of an empty box does nothing.
    let (mut w, [a, ..]) = four_rooms();
    box_apply(&mut w, Some(a), 5, 5, (0, 0), 0x400, true);
    assert!(nonzero(&w).is_empty());
    // A box straddling A's corner: D's corner cell (10, 10) belongs to the
    // right strip (queried from A, found through A's adjacency).
    let (mut w, [a, ..]) = four_rooms();
    w.set(10, 10, 0x8);
    assert_eq!(box_value(&w, Some(a), 9, 9, (3, 3), 0x8), 0x8);
    box_apply(&mut w, Some(a), 9, 9, (3, 3), 0x400, true);
    for (x, y) in [(8, 8), (10, 8), (8, 10), (10, 10)] {
        assert_ne!(w.at(x, y) & 0x400, 0, "({x}, {y})");
    }
}

// PC3: a missile path gets type 4 through set type: flags 0x60000.
#[test]
fn missile_allocation_sets_type_4_through_set_type() {
    let (mut w, a) = one_room();
    let p = alloc_dynamic_path(
        &tables(),
        &mut w,
        DynamicKind::Missile { size: 1 },
        UnitId(4),
        Some(a),
        3,
        3,
        false,
    )
    .unwrap();
    assert_eq!(p.path_type, 4);
    assert_eq!(p.flags & 0x7FF00, 0x60000);
    assert_eq!(p.dir_offset, 0);
}

// PC4 / WP4: teleport stamps (and a missile queries) from the destination
// room; the zero point clears the missile's collided mask; the forced move
// with room1 null does nothing.
#[test]
fn teleport_uses_the_destination_room() {
    // A player from A to C (not adjacent to A).
    let (mut w, [a, _, c]) = three_in_a_row();
    let mut p = player_path(&mut w, a, 5, 5);
    teleport(
        &mut Both(&mut w, &mut Motion::default()),
        &mut p,
        false,
        Some(c),
        25,
        5,
    )
    .unwrap();
    assert_eq!(w.at(5, 5), 0);
    assert_eq!(w.at(25, 5), 0x80 | bits::NO_PATH);
    // Forced move: room1 null → nothing, not even the stamp.
    let (mut w, [a, _, c]) = three_in_a_row();
    forced_move_rooms(&mut w, None, (5, 5), Some(c), (25, 5), 1, 0x80);
    assert!(nonzero(&w).is_empty());
    forced_move_rooms(&mut w, Some(a), (5, 5), Some(c), (25, 5), 1, 0x80);
    assert_eq!(w.at(25, 5), 0x80 | bits::NO_PATH);
    // Missile from A to C: the query and the stamp from C; the clear at
    // old happens first, so the missile's own footprint never collides.
    let (mut w, [a, _, c]) = three_in_a_row();
    let mut ms = alloc_dynamic_path(
        &tables(),
        &mut w,
        DynamicKind::Missile { size: 2 },
        UnitId(3),
        Some(a),
        5,
        5,
        false,
    )
    .unwrap();
    ms.foot_mask = 0x40;
    ms.move_mask = 0x41;
    stamp_size(&mut w, Some(a), 5, 5, 2, 0x40);
    w.set(26, 5, bits::WALL);
    teleport(
        &mut Both(&mut w, &mut Motion::default()),
        &mut ms,
        true,
        Some(c),
        25,
        5,
    )
    .unwrap();
    assert_eq!(ms.collided_mask, 0x1);
    assert_eq!(w.at(25, 5), 0x40);
    assert_eq!(w.at(5, 5), 0);
    // Overlapping move inside one room: old cells cleared before the
    // query, so the own footprint (0x40) is not reported.
    let (mut w, a) = one_room();
    let mut ms = alloc_dynamic_path(
        &tables(),
        &mut w,
        DynamicKind::Missile { size: 2 },
        UnitId(3),
        Some(a),
        5,
        5,
        false,
    )
    .unwrap();
    ms.foot_mask = 0x40;
    ms.move_mask = 0x40;
    stamp_size(&mut w, Some(a), 5, 5, 2, 0x40);
    teleport(
        &mut Both(&mut w, &mut Motion::default()),
        &mut ms,
        true,
        Some(a),
        6,
        5,
    )
    .unwrap();
    assert_eq!(ms.collided_mask, 0);
    // Same cell: flag 0x8 cleared. Zero point: collided mask := 0.
    ms.collided_mask = 0x55;
    teleport(
        &mut Both(&mut w, &mut Motion::default()),
        &mut ms,
        true,
        Some(a),
        6,
        5,
    )
    .unwrap();
    assert_eq!(ms.flags & flags::MOVED, 0);
    ms.collided_mask = 0x55;
    teleport(
        &mut Both(&mut w, &mut Motion::default()),
        &mut ms,
        true,
        Some(a),
        0,
        0,
    )
    .unwrap();
    assert_eq!(ms.collided_mask, 0);
}

// PC5: HasCollision has no bound check; D2RUST reads modes above
// 7 (absent from ObjMode.txt) as 0.
#[test]
fn object_modes_above_7_do_not_collide() {
    let shape = ObjectShape {
        has_collision: [true; 8],
        ..ObjectShape::default()
    };
    assert!(shape.collides_in(7));
    assert!(!shape.collides_in(8));
}

/// Units for the unit search: (room, type, mode, size, x, y).
struct UnitWorld {
    rooms: Rooms,
    units: Vec<(RoomId, UnitType, u32, i32, i32, i32)>,
}

impl CollisionRooms for UnitWorld {
    fn subtile_rect(&self, room: RoomId) -> Option<TileRect> {
        self.rooms.subtile_rect(room)
    }
    fn adjacent_count(&self, room: RoomId) -> usize {
        self.rooms.adjacent_count(room)
    }
    fn adjacent(&self, room: RoomId, i: usize) -> Option<RoomId> {
        self.rooms.adjacent(room, i)
    }
    fn grid(&self, room: RoomId) -> Option<&CollisionGrid> {
        self.rooms.grid(room)
    }
    fn grid_mut(&mut self, room: RoomId) -> Option<&mut CollisionGrid> {
        self.rooms.grid_mut(room)
    }
}

impl UnitsAtPoint for UnitWorld {
    type Unit = usize;
    fn room_units(&self, room: RoomId) -> Option<Vec<usize>> {
        let v: Vec<usize> = (0..self.units.len())
            .filter(|&i| self.units[i].0 == room)
            .collect();
        (!v.is_empty()).then_some(v)
    }
    fn unit_type(&self, u: usize) -> UnitType {
        self.units[u].1
    }
    fn unit_mode(&self, u: usize) -> u32 {
        self.units[u].2
    }
    fn unit_size(&self, u: usize) -> i32 {
        self.units[u].3
    }
    fn unit_point(&self, u: usize) -> (i32, i32) {
        (self.units[u].4, self.units[u].5)
    }
}

// W6: rooms in adjacency order, units in list order, the type / mode
// filter, the size table and the accept callback.
#[test]
fn unit_at_point_search_order_and_hit_table() {
    let (rooms, [a, b, ..]) = four_rooms();
    let mut w = UnitWorld {
        rooms,
        units: vec![
            // 0: player in mode 17 (DD): skipped.
            (a, UnitType::Player, 17, 2, 5, 5),
            // 1: object: skipped.
            (a, UnitType::Object, 1, 3, 5, 5),
            // 2: monster at distance (1, 1) of size 3.
            (b, UnitType::Monster, 1, 3, 6, 6),
            // 3: missile on the point, size 1.
            (b, UnitType::Missile, 1, 1, 5, 5),
        ],
    };
    // Room A is searched first (no candidate), then B in list order.
    assert_eq!(unit_at_point(&w, Some(a), 5, 5, 1, |_| true), Some(2));
    // accept refuses the monster: the missile next.
    assert_eq!(unit_at_point(&w, Some(a), 5, 5, 1, |u| u != 2), Some(3));
    // r outside 1..3 or no room: none.
    assert_eq!(unit_at_point(&w, Some(a), 5, 5, 0, |_| true), None);
    assert_eq!(unit_at_point(&w, Some(a), 5, 5, 4, |_| true), None);
    assert_eq!(unit_at_point(&w, None, 5, 5, 1, |_| true), None);
    // Size above 3 reads as 3; size 0 is skipped.
    w.units = vec![
        (a, UnitType::Monster, 1, 0, 5, 5),
        (a, UnitType::Monster, 1, 7, 7, 7),
    ];
    assert_eq!(unit_at_point(&w, Some(a), 5, 5, 3, |_| true), Some(1));
    assert_eq!(unit_at_point(&w, Some(a), 5, 5, 2, |_| true), None);
    // The table (r \ s).
    for (r, s, d, hit) in [
        (1, 1, (0, 0), true),
        (1, 1, (1, 0), false),
        (1, 2, (1, 0), true),
        (2, 1, (1, 1), false),
        (1, 3, (1, 1), true),
        (2, 2, (1, 1), true),
        (2, 2, (2, 1), false),
        (2, 3, (2, 1), true),
        (3, 2, (1, 2), true),
        (3, 2, (2, 2), false),
        (3, 3, (2, 2), true),
        (3, 3, (3, 0), false),
    ] {
        assert_eq!(shapes_overlap(r, s, d.0, d.1), hit, "r {r} s {s} d {d:?}");
    }
}

// W5: the size query reads the size's cells (0, 1 point; 2 plus; 3 box),
// and the size stamp / clear skip size 0.
#[test]
fn size_query_reads_the_shape_of_the_size() {
    let (mut w, a) = one_room();
    w.set(6, 5, 0x1);
    w.set(6, 6, 0x2);
    assert_eq!(size_value(&w, Some(a), 5, 5, 0, 0xFF), 0);
    assert_eq!(size_value(&w, Some(a), 5, 5, 1, 0xFF), 0);
    assert_eq!(size_value(&w, Some(a), 5, 5, 2, 0xFF), 0x1);
    assert_eq!(size_value(&w, Some(a), 5, 5, 3, 0xFF), 0x3);
    let (mut w, a) = one_room();
    stamp_size(&mut w, Some(a), 5, 5, 0, 0x40);
    assert!(nonzero(&w).is_empty());
    stamp_size(&mut w, Some(a), 5, 5, 2, 0x40);
    clear_size(&mut w, Some(a), 5, 5, 0, 0x40);
    assert_eq!(nonzero(&w).len(), 5);
}

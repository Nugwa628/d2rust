//! Tests written to kill surviving mutants of `cargo mutants` on
//! `crates/d2-sim/src/path/`. Each asserts what the path rules state;
//! the mutant it kills is named in the test's comment.

use crate::drlg::collision::bits;
use crate::drlg::{CollisionGrid, TileRect};
use crate::units::{RoomId, UnitId};

use super::collision::*;
use super::coords::Point;
use super::footprint::*;
use super::place_seams::mask;
use super::record::*;
use super::search::tests::Grid;
use super::search::{free_point_step, ExpField};
use super::tables::PathTables;

/// Rooms for these tests: index = `RoomId.0`, adjacency = the room itself.
#[derive(Default)]
struct Rooms {
    rooms: Vec<(TileRect, CollisionGrid)>,
}

impl Rooms {
    /// One room [0, w) × [0, h), all masks 0.
    fn one(w: i32, h: i32) -> (Rooms, RoomId) {
        let rect = TileRect::new(0, 0, w, h);
        let rooms = Rooms {
            rooms: vec![(rect, CollisionGrid::new(rect))],
        };
        (rooms, RoomId(0))
    }
    fn set(&mut self, x: i32, y: i32, v: u16) {
        *self.rooms[0].1.get_mut(x, y).expect("cell in the room") = v;
    }
    fn at(&self, x: i32, y: i32) -> u16 {
        self.rooms[0].1.get(x, y).expect("cell in the room")
    }
}

impl CollisionRooms for Rooms {
    fn subtile_rect(&self, room: RoomId) -> Option<TileRect> {
        self.rooms.get(room.0 as usize).map(|r| r.0)
    }
    fn adjacent_count(&self, room: RoomId) -> usize {
        usize::from(self.rooms.get(room.0 as usize).is_some())
    }
    fn adjacent(&self, room: RoomId, i: usize) -> Option<RoomId> {
        (i == 0).then_some(room)
    }
    fn grid(&self, room: RoomId) -> Option<&CollisionGrid> {
        self.rooms.get(room.0 as usize).map(|r| &r.1)
    }
    fn grid_mut(&mut self, room: RoomId) -> Option<&mut CollisionGrid> {
        self.rooms.get_mut(room.0 as usize).map(|r| &mut r.1)
    }
}

fn tables() -> PathTables {
    PathTables::spec().expect("embedded tables parse")
}

/// Records nothing; sets the centre position (pathing).
impl PathMotion for Rooms {
    fn set_position(&mut self, path: &mut DynamicPath, x: i32, y: i32, _: Option<RoomId>) {
        path.precise_x = super::coords::to_fp16_center(x);
        path.precise_y = super::coords::to_fp16_center(y);
    }
    fn reset(&mut self, _: &mut DynamicPath) {}
}

// ---- queries -------------------------------------------------------

// Kills `| → ^` in `plus_value`: two plus cells with the same bit.
#[test]
fn plus_query_ors_equal_bits() {
    let (mut w, a) = Rooms::one(20, 20);
    w.set(9, 10, bits::WALL);
    w.set(11, 10, bits::WALL);
    w.set(10, 11, 0x3);
    assert_eq!(plus_value(&w, Some(a), 10, 10, 0xFFFF), 0x3);
    assert_eq!(plus_value(&w, Some(a), 10, 10, bits::WALL), bits::WALL);
}

// ---- records -------------------------------------------------------

// Kills `& → |` / `& → ^` on `flags & 0x10000` and `&& → ||` in the
// type-4 assert of `set_path_type`.
#[test]
fn set_type_saves_velocity_and_allows_short_missile() {
    let t = tables();
    // Type 8 has 0x8000 (table 0x1E600); the path lacks 0x10000.
    let mut p = DynamicPath {
        velocity: 0x700,
        saved_velocity: 0,
        path_type: 7,
        ..DynamicPath::default()
    };
    p.set_path_type(&t, true, 8).unwrap();
    assert_eq!(p.saved_velocity, 0x700);
    // The path has 0x10000: the saved velocity stays.
    p.velocity = 0x900;
    p.prev_path_type = 7;
    assert_ne!(p.flags & 0x10000, 0);
    p.set_path_type(&t, true, 8).unwrap();
    assert_eq!(p.saved_velocity, 0x700);
    // Type 4 with max distance < 78 is no assert.
    let mut m = DynamicPath {
        max_distance: 14,
        ..DynamicPath::default()
    };
    assert_eq!(m.set_path_type(&t, false, 4), Ok(()));
    assert_eq!(m.path_type, 4);
    // Max distance ≥ 78 with another type is no assert either.
    let mut q = DynamicPath {
        max_distance: 78,
        ..DynamicPath::default()
    };
    assert_eq!(q.set_path_type(&t, true, 7), Ok(()));
}

// ---- teleport ------------------------------------------------------

// Kills the mutants of `flags 0x8 := moved` in the missile teleport: the
// other flag bits stay.
#[test]
fn missile_teleport_sets_only_the_moved_flag() {
    let (mut w, a) = Rooms::one(20, 20);
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
    ms.flags = flags::ACTIVE | flags::KEEP_TARGET;
    teleport(&mut w, &mut ms, true, Some(a), 5, 5).unwrap();
    assert_eq!(ms.flags, flags::ACTIVE | flags::KEEP_TARGET | flags::MOVED);
    assert_eq!(w.at(5, 5), 0);
}

// ---- field -------------------------------------------------------

// Kills `< → <=` in `ExpField::from_bytes`: a 10-byte file is a whole
// header (u16 + two u32) with 0 × 0 cells.
#[test]
fn field_header_only_file_parses() {
    let mut b = vec![0x0A, 0x01];
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    let f = ExpField::from_bytes(&b).unwrap();
    assert_eq!((f.version, f.height, f.width), (0x010A, 0, 0));
    assert!(ExpField::from_bytes(&b[..9]).is_err());
}

// ---- ring search -------------------------------------------------

/// The search tests' grid (`search::tests::Grid`): rooms of `TileRect`s.
fn grid(w: i32, h: i32) -> Grid {
    Grid::with_rooms(&[TileRect::new(0, 0, w, h)])
}

// Kills `(r − 1)·k → /`, `X0 + (r−1)k → −` and `x ≤ R → >` in
// `nearest_free_point`. k = 2: ring 1 tests (9, 9), (11, 9), (9, 11),
// (11, 11), (10, 9), (10, 11) (walled, with the centre); (11, 10) is
// skipped. Ring 2 (L 8, R 12, T 7, B 13, s 6): columns x = 7, 13 for
// y = 7, 9, 11, 13 keep (7, 9) (d 4); rows x = 8, 10, 12 for y = 7, 13
// replace it with (10, 7) (d 3); (10, 13) ties and loses.
#[test]
fn ring_step_two_rows_and_offsets() {
    let mut g = grid(20, 20);
    for (x, y) in [
        (10, 10),
        (9, 9),
        (11, 9),
        (9, 11),
        (11, 11),
        (10, 9),
        (10, 11),
    ] {
        g.set(x, y, 0x1);
    }
    let mut p = Point::new(10, 10);
    let r = free_point_step(&g, Some(0), &mut p, 1, mask::PLAYER_PLACE, 2).unwrap();
    assert_eq!((r, p), (Some(0), Point::new(10, 7)));
}

// Kills `r·k + 1 ≥ D → r / k` in `nearest_free_point`: with k = 2 and
// D = 50 the rings run while r·2 + 1 < 50, so ring 25 (offset 48) is the
// last; a free cell first reached by ring 26 (offset 50: column
// x = X0 − 51, top row y = Y0 − 51) is not found.
#[test]
fn ring_limit_with_step_two() {
    let mut g = grid(120, 120);
    for y in 0..120 {
        for x in 0..120 {
            g.set(x, y, 0x1);
        }
    }
    // Ring 25's first cell (60 − 49, 60 − 49) free: found.
    let mut g25 = g.clone();
    g25.rooms[0].1[11 * 120 + 11] = 0;
    let mut p = Point::new(60, 60);
    let r = free_point_step(&g25, Some(0), &mut p, 1, mask::PLAYER_PLACE, 2).unwrap();
    assert_eq!((r, p), (Some(0), Point::new(11, 11)));
    // Ring 26's first cell (9, 9) free: not reached.
    let mut g26 = g.clone();
    g26.rooms[0].1[9 * 120 + 9] = 0;
    let mut p = Point::new(60, 60);
    let r = free_point_step(&g26, Some(0), &mut p, 1, mask::PLAYER_PLACE, 2).unwrap();
    assert_eq!((r, p), (None, Point::new(60, 60)));
}

// ---- path-tables.tsv --------------------------------------------------

// Kills `|| → &&` in the index check of `PathTables::from_tsv`: two rows
// swapped (each with its own address) would otherwise land at each
// other's index. The `index` column names the entry, so a parse either
// fails or yields the unswapped tables; the strict parser fails.
// (Constants table: no rule id to claim.)
#[test]
fn tables_rows_out_of_index_order_are_an_error() {
    let good = super::tables::PATH_TABLES_TSV;
    let r2 = "pattern_of_size\t2\t1\t\t\t\t\t0x006EB3E4\n";
    let r3 = "pattern_of_size\t3\t2\t\t\t\t\t0x006EB3E8\n";
    let swapped = good.replacen(&format!("{r2}{r3}"), &format!("{r3}{r2}"), 1);
    assert_ne!(swapped, good);
    assert!(matches!(
        PathTables::from_tsv(&swapped),
        Err(super::PathError::Tsv(_))
    ));
}

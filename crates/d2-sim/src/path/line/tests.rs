//! Synthetic vectors of the line test and the unit collision line.

use super::*;
use crate::drlg::{CollisionGrid, TileRect};

/// [`line_test`] on tuples.
fn lt(w: &Rooms, room: Option<RoomId>, from: (i32, i32), to: (i32, i32), m: u16) -> LineTest {
    line_test(
        w,
        room,
        Point::new(from.0, from.1),
        Point::new(to.0, to.1),
        m,
    )
}

fn blocked_at(x: i32, y: i32) -> LineTest {
    LineTest::Blocked(Point::new(x, y))
}

/// Two rooms side by side: A [0, 10) × [0, 10), B [10, 20) × [0, 10),
/// adjacent to each other (each itself first).
struct Rooms(Vec<(TileRect, Vec<RoomId>, CollisionGrid)>);

const A: RoomId = RoomId(0);
const B: RoomId = RoomId(1);

fn rooms() -> Rooms {
    let ra = TileRect::new(0, 0, 10, 10);
    let rb = TileRect::new(10, 0, 10, 10);
    Rooms(vec![
        (ra, vec![A, B], CollisionGrid::new(ra)),
        (rb, vec![B, A], CollisionGrid::new(rb)),
    ])
}

impl Rooms {
    fn set(&mut self, x: i32, y: i32, v: u16) {
        for r in &mut self.0 {
            if let Some(m) = r.2.get_mut(x, y) {
                *m = v;
            }
        }
    }
}

impl CollisionRooms for Rooms {
    fn subtile_rect(&self, room: RoomId) -> Option<TileRect> {
        Some(self.0.get(room.0 as usize)?.0)
    }
    fn adjacent_count(&self, room: RoomId) -> usize {
        self.0.get(room.0 as usize).map_or(0, |r| r.1.len())
    }
    fn adjacent(&self, room: RoomId, i: usize) -> Option<RoomId> {
        self.0.get(room.0 as usize)?.1.get(i).copied()
    }
    fn grid(&self, room: RoomId) -> Option<&CollisionGrid> {
        Some(&self.0.get(room.0 as usize)?.2)
    }
    fn grid_mut(&mut self, room: RoomId) -> Option<&mut CollisionGrid> {
        Some(&mut self.0.get_mut(room.0 as usize)?.2)
    }
}

/// The cells a line visits: block every cell but one at a time and
/// see which blocks change the result.
fn visited(from: (i32, i32), to: (i32, i32)) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for y in 0..10 {
        for x in 0..20 {
            let mut w = rooms();
            w.set(x, y, 1);
            if lt(&w, Some(A), from, to, 1).blocked() {
                out.push((x, y));
            }
        }
    }
    out.sort_by_key(|&(x, y)| (x - from.0).abs() + (y - from.1).abs());
    out
}

#[test]
fn line_cells_follow_the_late_error_term() {
    assert_eq!(
        visited((0, 0), (4, 1)),
        [(0, 0), (1, 0), (2, 0), (3, 0), (4, 1)]
    );
    assert_eq!(
        visited((0, 0), (1, 4)),
        [(0, 0), (0, 1), (0, 2), (0, 3), (1, 4)]
    );
    // One cell, a vertical and a horizontal line.
    assert_eq!(visited((3, 3), (3, 3)), [(3, 3)]);
    assert_eq!(visited((2, 5), (2, 3)).len(), 3);
    assert_eq!(visited((5, 2), (2, 2)).len(), 4);
}

#[test]
fn line_crosses_rooms_and_reports_the_stop_cell() {
    let mut w = rooms();
    assert_eq!(lt(&w, Some(A), (8, 2), (12, 2), 1), LineTest::Clear);
    w.set(11, 2, 3);
    assert_eq!(lt(&w, Some(A), (8, 2), (12, 2), 1), blocked_at(11, 2));
    // Masked out: clear.
    assert_eq!(lt(&w, Some(A), (8, 2), (12, 2), 4), LineTest::Clear);
    // Null room, start in no room, leaving every room: blocked (rule 1:
    // the stop cell is `from`).
    assert_eq!(lt(&w, None, (1, 1), (2, 2), 1), blocked_at(1, 1));
    assert_eq!(lt(&w, Some(A), (30, 1), (2, 2), 1), blocked_at(30, 1));
    assert_eq!(lt(&w, Some(A), (18, 2), (21, 2), 1), blocked_at(20, 2));
}

fn unit(x: i32, y: i32, size: i32) -> LineUnit {
    LineUnit {
        room: Some(A),
        x,
        y,
        size,
    }
}

#[test]
fn touching_units_and_roomless_units_are_clear() {
    let mut w = rooms();
    for y in 0..10 {
        for x in 0..20 {
            w.set(x, y, 2);
        }
    }
    // Vector: a (10, 10) size 2, b (11, 11) size 1 → dx + dy = 2 <
    // 3: no line test. Here at (2, 2) / (3, 3).
    assert!(!units_line_blocked(&w, &unit(2, 2, 2), &unit(3, 3, 1), 2));
    // Sizes above 2 count 2: 5 and 1 → 3; dx + dy = 3 is not < 3.
    assert!(units_line_blocked(&w, &unit(2, 2, 5), &unit(4, 3, 1), 2));
    assert!(!units_line_blocked(&w, &unit(2, 2, 5), &unit(3, 3, 2), 2));
    // a without a room: clear.
    let a = LineUnit {
        room: None,
        ..unit(2, 2, 0)
    };
    assert!(!units_line_blocked(&w, &a, &unit(8, 8, 0), 2));
}

#[test]
fn ends_are_pulled_toward_each_other() {
    // Vector: a (10, 10) size 2, b (12, 11) size 1: dy < dx → ax
    // 12, bx 11; line (12, 10) → (11, 11). Shifted to (2, 2) / (4, 3):
    // line (4, 2) → (3, 3); a block at (2, 2) (the unpulled start) does
    // not matter, one at (4, 2) does.
    let mut w = rooms();
    w.set(2, 2, 2);
    assert!(!units_line_blocked(&w, &unit(2, 2, 2), &unit(4, 3, 1), 2));
    w.set(4, 2, 2);
    assert!(units_line_blocked(&w, &unit(2, 2, 2), &unit(4, 3, 1), 2));
    // dx = dy moves both axes: a (2, 2) size 1, b (5, 5) size 1 → line
    // (3, 3) → (4, 4); the corners are not tested.
    let mut w = rooms();
    w.set(2, 2, 2);
    w.set(5, 5, 2);
    assert!(!units_line_blocked(&w, &unit(2, 2, 1), &unit(5, 5, 1), 2));
    w.set(4, 4, 2);
    assert!(units_line_blocked(&w, &unit(2, 2, 1), &unit(5, 5, 1), 2));
    // Sizes 0: no pull, the ends themselves are tested; reverse
    // direction pulls the other way.
    let mut w = rooms();
    w.set(8, 2, 2);
    assert!(units_line_blocked(&w, &unit(8, 2, 0), &unit(2, 2, 0), 2));
    assert!(!units_line_blocked(&w, &unit(8, 2, 1), &unit(2, 2, 0), 2));
}

#[test]
fn the_blocking_cell_is_reported_for_the_object_footprint_test() {
    // The same diagonal-tie vector as above: the pulled line (4, 2) → (3, 3)
    // reports the cell; a clear line reports none.
    let mut w = rooms();
    w.set(4, 2, 2);
    assert_eq!(
        units_line_block_point(&w, &unit(2, 2, 2), &unit(4, 3, 1), 2),
        Some(Point::new(4, 2))
    );
    w.set(4, 2, 0);
    assert_eq!(
        units_line_block_point(&w, &unit(2, 2, 2), &unit(4, 3, 1), 2),
        None
    );
    // Masked out: no blocker.
    w.set(4, 2, 2);
    assert_eq!(
        units_line_block_point(&w, &unit(2, 2, 2), &unit(4, 3, 1), 4),
        None
    );
}

#[test]
fn reach_offset_table() {
    let want = [
        2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    ];
    for (d, &k) in want.iter().enumerate() {
        assert_eq!(reach_offset(d as i32), k, "d {d}");
    }
    assert_eq!(reach_offset(25), 3);
    assert_eq!(reach_offset(400), 3);
}

#[test]
fn can_reach_directly_probes_target_then_both_sides() {
    // a (2, 5) size 0, b (14, 5): d 12 → k 4; sx = −4, sy = 0; probes
    // (14, 5), (14, 1), (14, 9); the point end is size 2, so each line
    // stops 2 short on the major axis (x 12).
    let a = unit(2, 5, 0);
    let w = rooms();
    assert!(can_reach_directly(&w, &a, (14, 5), 12));
    // Cell values outside mask 0x805 do not block.
    let mut w = rooms();
    for y in 0..10 {
        w.set(6, y, 2);
    }
    assert!(can_reach_directly(&w, &a, (14, 5), 12));
    // One blocked cell on the direct line: the side probe to (14, 1)
    // passes above it.
    let mut w = rooms();
    w.set(6, 5, 1);
    assert!(point_line_blocked(&w, &a, 14, 5, REACH_MASK));
    assert!(!point_line_blocked(&w, &a, 14, 1, REACH_MASK));
    assert!(can_reach_directly(&w, &a, (14, 5), 12));
    // A wide wall blocks all three probes.
    let mut w = rooms();
    for y in 2..=8 {
        w.set(6, y, 0x800);
    }
    assert!(!can_reach_directly(&w, &a, (14, 5), 12));
    // A unit without a room: every probe is clear.
    let roomless = LineUnit { room: None, ..a };
    assert!(can_reach_directly(&w, &roomless, (14, 5), 12));
}

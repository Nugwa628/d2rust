//! Cell line test `(room R, &from, &to, mask)`:
//! the sub-tile cells from `from` to `to`
//! (both included), in Bresenham order on the major axis, each tested
//! `value & mask` ≠ 0 on the collision grid of the room the walk is in;
//! and the collision line between two units
//! (the unit line-of-sight rule) over it. The server's monster AI, skill
//! bodies, missiles and melee range read them. No draws.
use crate::units::RoomId;

use super::collision::{find_room, CollisionRooms, MISSING_ROOM};
use super::coords::Point;

/// The result of [`line_test`]: `Clear` (result 0, `to` unchanged) or
/// `Blocked(cell)` (result 1, `to` := the cell).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineTest {
    Clear,
    Blocked(Point),
}

impl LineTest {
    /// Result 1.
    pub fn blocked(self) -> bool {
        matches!(self, Self::Blocked(_))
    }
}

/// Whether R's rect holds the cell (room +0x4C … +0x58, half-open).
fn in_rect<R: CollisionRooms + ?Sized>(rooms: &R, room: RoomId, p: Point) -> bool {
    rooms
        .subtile_rect(room)
        .is_some_and(|r| r.contains(p.x, p.y))
}

/// `(room, &from, &to, mask)`.
///
/// The cell value is R's collision grid word; a room
/// without a grid reads as [`MISSING_ROOM`] (the collision queries'
/// value for a missing grid).
pub fn line_test<R: CollisionRooms + ?Sized>(
    rooms: &R,
    room: Option<RoomId>,
    from: Point,
    to: Point,
    mask: u16,
) -> LineTest {
    // Rule 1.
    let Some(mut r) = room else {
        return LineTest::Blocked(from);
    };
    if !in_rect(rooms, r, from) {
        match find_room(rooms, Some(r), from.x, from.y) {
            Some(f) if in_rect(rooms, f, from) => r = f,
            _ => return LineTest::Blocked(from),
        }
    }
    // Rule 2.
    let (dx, dy) = (to.x.wrapping_sub(from.x), to.y.wrapping_sub(from.y));
    let sx = if dx < 0 { -1 } else { 1 };
    let sy = if dy < 0 { -1 } else { 1 };
    let (adx, ady) = (dx.wrapping_abs(), dy.wrapping_abs());
    // Rule 3: the major axis (x on ties) and the error term.
    let x_major = adx >= ady;
    let (major, minor) = if x_major { (adx, ady) } else { (ady, adx) };
    let value = |r: RoomId, p: Point| {
        rooms
            .grid(r)
            .and_then(|g| g.get(p.x, p.y))
            .unwrap_or(MISSING_ROOM)
    };
    let mut p = from;
    let mut e = 0i32;
    let mut k = 0i32;
    loop {
        if value(r, p) & mask != 0 {
            return LineTest::Blocked(p);
        }
        if k == major {
            return LineTest::Clear;
        }
        k += 1;
        e += minor;
        let step_minor = minor != 0 && e >= major;
        if step_minor {
            e -= major;
        }
        p = if x_major {
            Point::new(p.x + sx, if step_minor { p.y + sy } else { p.y })
        } else {
            Point::new(if step_minor { p.x + sx } else { p.x }, p.y + sy)
        };
        // Rule 4: the next cell left R's rect → R := its lookup from R.
        if !in_rect(rooms, r, p) {
            match find_room(rooms, Some(r), p.x, p.y) {
                Some(f) if in_rect(rooms, f, p) => r = f,
                _ => return LineTest::Blocked(p),
            }
        }
    }
}

/// Sizes above this read as it.
pub const SIZE_CAP: i32 = 2;

/// One unit of the collision line: its room
/// sub-tile position ((0, 0)
/// without a path) and size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineUnit {
    pub room: Option<RoomId>,
    pub x: i32,
    pub y: i32,
    pub size: i32,
}

/// The end pull.
fn pull(a: &mut i32, b: &mut i32, sa: i32, sb: i32) {
    if *a < *b {
        *a += sa;
        *b -= sb;
    } else {
        *a -= sa;
        *b += sb;
    }
}

/// Blocked (true) or clear. The
/// null-unit fatal of rule 1 is the caller's (it has no unit to pass).
pub fn units_line_blocked<R: CollisionRooms + ?Sized>(
    rooms: &R,
    a: &LineUnit,
    b: &LineUnit,
    mask: u16,
) -> bool {
    units_line_block_point(rooms, a, b, mask).is_some()
}

/// [`units_line_blocked`] with the cell that blocks the line: `None` when
/// the line is clear.
pub fn units_line_block_point<R: CollisionRooms + ?Sized>(
    rooms: &R,
    a: &LineUnit,
    b: &LineUnit,
    mask: u16,
) -> Option<Point> {
    // Rule 1.
    a.room?;
    // Rules 2–3.
    let (mut ax, mut ay, mut bx, mut by) = (a.x, a.y, b.x, b.y);
    let sa = a.size.min(SIZE_CAP);
    let sb = b.size.min(SIZE_CAP);
    // Rule 4.
    let dx = bx.wrapping_sub(ax).wrapping_abs();
    let dy = by.wrapping_sub(ay).wrapping_abs();
    if dx.wrapping_add(dy) < sa + sb {
        return None;
    }
    // Rule 5.
    if !(sa == 0 && sb == 0) {
        if dy <= dx {
            pull(&mut ax, &mut bx, sa, sb);
        }
        if dy >= dx {
            pull(&mut ay, &mut by, sa, sb);
        }
    }
    // Rule 6 (the stop cell is discarded).
    match line_test(rooms, a.room, Point::new(ax, ay), Point::new(bx, by), mask) {
        LineTest::Blocked(p) => Some(p),
        LineTest::Clear => None,
    }
}

/// The collision line
/// from unit `a` to the point (x, y), the point read as an end of size 2
/// (the unit size with the second size 2, capped as any size).
pub fn point_line_blocked<R: CollisionRooms + ?Sized>(
    rooms: &R,
    a: &LineUnit,
    x: i32,
    y: i32,
    mask: u16,
) -> bool {
    let b = LineUnit {
        room: a.room,
        x,
        y,
        size: 2,
    };
    units_line_blocked(rooms, a, &b, mask)
}

/// The probe mask of [`can_reach_directly`]: 0x805.
pub const REACH_MASK: u16 = 0x805;

/// The probe offset of the monster AI by the full-size distance d:
/// 2 for d < 3, 3 for d < 11, 4 for d < 25,
/// else 3 (negative d never occurs: the distance is clamped at 0).
pub fn reach_offset(d: i32) -> i32 {
    match d {
        i32::MIN..=2 => 2,
        3..=10 => 3,
        11..=24 => 4,
        _ => 3,
    }
}

/// "can reach directly": with
/// k = [`reach_offset`] of the full-size distance `d` from a to b,
/// sx = sign(a.x − b.x)·k and sy = sign(a.y − b.y)·k, probe the line
/// from a ([`point_line_blocked`], mask 0x805) to b's point, then to
/// (b.x − sy, b.y + sx), then to (b.x + sy, b.y − sx), stopping at the
/// first clear one. True when one is clear, false when all three are
/// blocked.
pub fn can_reach_directly<R: CollisionRooms + ?Sized>(
    rooms: &R,
    a: &LineUnit,
    b: (i32, i32),
    d: i32,
) -> bool {
    let k = reach_offset(d);
    let sx = a.x.wrapping_sub(b.0).signum() * k;
    let sy = a.y.wrapping_sub(b.1).signum() * k;
    let probes = [
        (b.0, b.1),
        (b.0.wrapping_sub(sy), b.1.wrapping_add(sx)),
        (b.0.wrapping_add(sy), b.1.wrapping_sub(sx)),
    ];
    probes
        .iter()
        .any(|&(x, y)| !point_line_blocked(rooms, a, x, y, REACH_MASK))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_pathing {
    use super::*;
    use crate::drlg::{CollisionGrid, TileRect};

    #[derive(Default)]
    struct Rooms(Vec<(TileRect, Vec<RoomId>, CollisionGrid)>);

    impl Rooms {
        fn add(&mut self, rect: TileRect) -> RoomId {
            self.0.push((rect, Vec::new(), CollisionGrid::new(rect)));
            RoomId(self.0.len() as u32 - 1)
        }
        fn set(&mut self, x: i32, y: i32, v: u16) {
            for r in &mut self.0 {
                if let Some(c) = r.2.get_mut(x, y) {
                    *c = v;
                    return;
                }
            }
            panic!("no room at ({x}, {y})");
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

    fn vector_room() -> (Rooms, RoomId) {
        let mut w = Rooms::default();
        let a = w.add(TileRect::new(0, 0, 20, 20));
        w.set(3, 1, 1);
        (w, a)
    }

    fn pt(x: i32, y: i32) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn line_test_vectors() {
        let (w, a) = vector_room();
        // x major: (0,0) (1,0) (2,0) (3,1) → blocked at (3, 1).
        assert_eq!(
            line_test(&w, Some(a), pt(0, 0), pt(6, 2), 1),
            LineTest::Blocked(pt(3, 1))
        );
        // y major: (0,0) (0,1) (0,2) (1,3) (1,4) (1,5) (2,6) → clear.
        assert_eq!(
            line_test(&w, Some(a), pt(0, 0), pt(2, 6), 1),
            LineTest::Clear
        );
        // One cell, tested.
        assert_eq!(
            line_test(&w, Some(a), pt(3, 1), pt(3, 1), 1),
            LineTest::Blocked(pt(3, 1))
        );
        // M08: the mask decides (bit 1 not in mask 2).
        assert_eq!(
            line_test(&w, Some(a), pt(0, 0), pt(6, 2), 2),
            LineTest::Clear
        );
        // Negative directions, axis lines: (3, 4) → (3, 0) passes (3, 1).
        assert_eq!(
            line_test(&w, Some(a), pt(3, 4), pt(3, 0), 1),
            LineTest::Blocked(pt(3, 1))
        );
        assert_eq!(
            line_test(&w, Some(a), pt(6, 1), pt(0, 1), 1),
            LineTest::Blocked(pt(3, 1))
        );
        // The x-major tie: (0, 0) → (4, 4) is the diagonal, misses (3, 1).
        assert_eq!(
            line_test(&w, Some(a), pt(0, 0), pt(4, 4), 1),
            LineTest::Clear
        );
    }

    #[test]
    fn line_test_rooms() {
        let mut w = Rooms::default();
        let a = w.add(TileRect::new(0, 0, 10, 10));
        let b = w.add(TileRect::new(10, 0, 10, 10));
        w.0[a.0 as usize].1 = vec![a, b];
        w.0[b.0 as usize].1 = vec![b, a];
        w.set(14, 2, 1);
        // Rule 1: no room → blocked at from.
        assert_eq!(
            line_test(&w, None, pt(1, 1), pt(5, 1), 1),
            LineTest::Blocked(pt(1, 1))
        );
        // Rule 1: from outside R → looked up (B), then walked.
        assert_eq!(
            line_test(&w, Some(a), pt(12, 2), pt(16, 2), 1),
            LineTest::Blocked(pt(14, 2))
        );
        // Rule 4: crossing from A into B, blocked in B.
        assert_eq!(
            line_test(&w, Some(a), pt(8, 2), pt(16, 2), 1),
            LineTest::Blocked(pt(14, 2))
        );
        // Rule 4: a cell in no room → blocked there, untested.
        assert_eq!(
            line_test(&w, Some(a), pt(5, 8), pt(5, 12), 1),
            LineTest::Blocked(pt(5, 10))
        );
        // Rule 1: from in no room.
        assert_eq!(
            line_test(&w, Some(a), pt(25, 2), pt(16, 2), 1),
            LineTest::Blocked(pt(25, 2))
        );
        // M08: without the neighbour link, B is not found.
        w.0[a.0 as usize].1 = vec![a];
        assert_eq!(
            line_test(&w, Some(a), pt(8, 2), pt(12, 2), 1),
            LineTest::Blocked(pt(10, 2))
        );
    }
}

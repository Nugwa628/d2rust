//! Tests of the sight and line test: worked vectors and rules.

use d2_sim::drlg::collision::bits;
use d2_sim::drlg::{CollisionGrid, TileRect};
use d2_sim::path::CollisionRooms;
use d2_sim::units::RoomId;

use super::*;

/// Rooms by index: sub-tile rect, adjacency array, collision grid.
struct Fake {
    rooms: Vec<(TileRect, Vec<u32>, Option<CollisionGrid>)>,
}

impl Fake {
    fn rect(x: i32, y: i32, w: i32, h: i32) -> TileRect {
        TileRect { x, y, w, h }
    }

    /// One room over (-20, -20)–(19, 19) with an empty grid.
    fn one() -> Self {
        let r = Self::rect(-20, -20, 40, 40);
        Self {
            rooms: vec![(r, vec![], Some(CollisionGrid::new(r)))],
        }
    }

    /// Room 0 over x 0..10, room 1 over x 10..20 (adjacent to 0), room 2
    /// over x 30..40 (adjacent to none), all y 0..10.
    fn three() -> Self {
        let r0 = Self::rect(0, 0, 10, 10);
        let r1 = Self::rect(10, 0, 10, 10);
        let r2 = Self::rect(30, 0, 10, 10);
        Self {
            rooms: vec![
                (r0, vec![1], Some(CollisionGrid::new(r0))),
                (r1, vec![0], Some(CollisionGrid::new(r1))),
                (r2, vec![], Some(CollisionGrid::new(r2))),
            ],
        }
    }

    fn set(&mut self, x: i32, y: i32, v: u16) {
        for (rect, _, grid) in &mut self.rooms {
            if rect.contains(x, y) {
                *grid.as_mut().unwrap().get_mut(x, y).unwrap() = v;
            }
        }
    }
}

impl CollisionRooms for Fake {
    fn subtile_rect(&self, room: RoomId) -> Option<TileRect> {
        self.rooms.get(room.0 as usize).map(|r| r.0)
    }
    fn adjacent_count(&self, room: RoomId) -> usize {
        self.rooms.get(room.0 as usize).map_or(0, |r| r.1.len())
    }
    fn adjacent(&self, room: RoomId, i: usize) -> Option<RoomId> {
        self.rooms
            .get(room.0 as usize)?
            .1
            .get(i)
            .map(|&r| RoomId(r))
    }
    fn grid(&self, room: RoomId) -> Option<&CollisionGrid> {
        self.rooms.get(room.0 as usize)?.2.as_ref()
    }
    fn grid_mut(&mut self, room: RoomId) -> Option<&mut CollisionGrid> {
        self.rooms.get_mut(room.0 as usize)?.2.as_mut()
    }
}

const R0: Option<RoomId> = Some(RoomId(0));

/// The cells a line tests, in order: with a wall on cell `c` alone, the
/// line stops at `c` exactly when it tests `c`; ordered by the walk
/// (each later cell is reached only past the earlier ones).
fn tested(from: (i32, i32), to: (i32, i32)) -> Vec<(i32, i32)> {
    let mut hits = vec![];
    for y in -10..=10 {
        for x in -10..=10 {
            let mut f = Fake::one();
            f.set(x, y, bits::VISIBLE);
            let r = line_test(&f, R0, from, to, SIGHT_MASK).unwrap();
            if r == (LineResult::Blocked { at: Some((x, y)) }) {
                hits.push((x, y));
            } else {
                assert_eq!(r, LineResult::Clear);
            }
        }
    }
    // Order along the walk: by distance along the major axis.
    let y_major = (to.0 - from.0).abs() < (to.1 - from.1).abs();
    hits.sort_by_key(|&(x, y)| {
        if y_major {
            (y - from.1).abs()
        } else {
            (x - from.0).abs()
        }
    });
    hits
}

#[test]
fn line_vectors_error_term_starts_at_zero() {
    assert_eq!(
        tested((0, 0), (4, 1)),
        [(0, 0), (1, 0), (2, 0), (3, 0), (4, 1)]
    );
    assert_eq!(
        tested((0, 0), (1, 4)),
        [(0, 0), (0, 1), (0, 2), (0, 3), (1, 4)]
    );
    // Negative steps: the same late minor step.
    assert_eq!(
        tested((4, 1), (0, 0)),
        [(4, 1), (3, 1), (2, 1), (1, 1), (0, 0)]
    );
    // Empty grid: clear.
    let f = Fake::one();
    assert_eq!(
        line_test(&f, R0, (0, 0), (4, 1), SIGHT_MASK),
        Ok(LineResult::Clear)
    );
}

#[test]
fn line_straight_and_single_cell() {
    assert_eq!(tested((2, 3), (2, 3)), [(2, 3)]);
    assert_eq!(
        tested((1, 2), (1, -2)),
        [(1, 2), (1, 1), (1, 0), (1, -1), (1, -2)]
    );
    assert_eq!(tested((-1, 5), (2, 5)), [(-1, 5), (0, 5), (1, 5), (2, 5)]);
    // dx = dy: the diagonal.
    assert_eq!(tested((0, 0), (3, -3)), [(0, 0), (1, -1), (2, -2), (3, -3)]);
}

#[test]
fn line_cell_test_uses_the_mask() {
    let mut f = Fake::one();
    f.set(2, 0, bits::WALL);
    assert_eq!(
        line_test(&f, R0, (0, 0), (4, 0), SIGHT_MASK),
        Ok(LineResult::Clear)
    );
    assert_eq!(
        line_test(&f, R0, (0, 0), (4, 0), bits::WALL),
        Ok(LineResult::Blocked { at: Some((2, 0)) })
    );
    // A room without a grid: no value is known for it.
    f.rooms[0].2 = None;
    assert_eq!(
        line_test(&f, R0, (0, 0), (4, 0), SIGHT_MASK),
        Err(SightError::NoGridCell {
            room: RoomId(0),
            x: 0,
            y: 0
        })
    );
}

#[test]
fn line_room_null_or_start_outside() {
    let f = Fake::three();
    assert_eq!(
        line_test(&f, None, (0, 0), (1, 0), SIGHT_MASK),
        Ok(LineResult::Blocked { at: None })
    );
    // Start in room 1, given room 0: switched through the adjacency.
    let mut g = Fake::three();
    g.set(15, 5, bits::VISIBLE);
    assert_eq!(
        line_test(&g, R0, (12, 5), (18, 5), SIGHT_MASK),
        Ok(LineResult::Blocked { at: Some((15, 5)) })
    );
    // Start in room 2, not adjacent to room 0: blocked.
    assert_eq!(
        line_test(&f, R0, (32, 5), (35, 5), SIGHT_MASK),
        Ok(LineResult::Blocked { at: None })
    );
}

#[test]
fn line_switches_rooms() {
    // Across rooms 0 and 1: room 1's grid is read past x 10.
    let mut f = Fake::three();
    assert_eq!(
        line_test(&f, R0, (5, 5), (15, 6), SIGHT_MASK),
        Ok(LineResult::Clear)
    );
    f.set(12, 5, bits::VISIBLE);
    assert_eq!(
        line_test(&f, R0, (5, 5), (15, 6), SIGHT_MASK),
        Ok(LineResult::Blocked { at: Some((12, 5)) })
    );
    // Leaving into no room: blocked at that cell.
    let f = Fake::three();
    assert_eq!(
        line_test(&f, R0, (5, 5), (5, 12), SIGHT_MASK),
        Ok(LineResult::Blocked { at: Some((5, 10)) })
    );
    // Room 2 is not adjacent to room 1: blocked at the first cell past it.
    assert_eq!(
        line_test(&f, Some(RoomId(1)), (15, 5), (35, 5), SIGHT_MASK),
        Ok(LineResult::Blocked { at: Some((20, 5)) })
    );
}

fn unit(room: Option<RoomId>, x: i32, y: i32, size: i32) -> SightUnit {
    SightUnit { room, x, y, size }
}

#[test]
fn sight_vectors() {
    let a = unit(R0, 10, 10, 2);
    assert_eq!(
        sight_line(&a, &unit(None, 12, 11, 1)),
        Some(((12, 10), (11, 11)))
    );
    assert_eq!(sight_line(&a, &unit(None, 11, 11, 1)), None);
}

#[test]
fn sight_sizes_and_pulling() {
    // Sizes ≥ 3 read as 2: 3 + 1 → 3, so dx + dy = 3 is not < 3.
    let a = unit(R0, 0, 0, 5);
    assert_eq!(sight_line(&a, &unit(None, 3, 0, 1)), Some(((2, 0), (2, 0))));
    // Both sizes 0: no pulling.
    assert_eq!(
        sight_line(&unit(R0, 0, 0, 0), &unit(None, -3, 1, 0)),
        Some(((0, 0), (-3, 1)))
    );
    // dy > dx: y pulled; dx = dy: both pulled.
    assert_eq!(
        sight_line(&unit(R0, 0, 0, 1), &unit(None, 1, -5, 2)),
        Some(((0, -1), (1, -3)))
    );
    assert_eq!(
        sight_line(&unit(R0, 0, 0, 1), &unit(None, -4, 4, 1)),
        Some(((-1, 1), (-3, 3)))
    );
}

#[test]
fn sight_hidden_gates_and_line() {
    let mut f = Fake::one();
    f.set(3, 0, bits::VISIBLE);
    let a = unit(R0, 0, 0, 1);
    let b = unit(None, 6, 0, 1);
    // Line (1, 0) → (5, 0) crosses the bit-2 cell: hidden.
    assert_eq!(sight_hidden(true, &a, &b, &f), Ok(true));
    // LOSDraw 0: every unit passes.
    assert_eq!(sight_hidden(false, &a, &b, &f), Ok(false));
    // No room for `a`: passes.
    assert_eq!(sight_hidden(true, &unit(None, 0, 0, 1), &b, &f), Ok(false));
    // Close enough: passes without a line.
    assert_eq!(sight_hidden(true, &a, &unit(None, 1, 0, 1), &f), Ok(false));
    // A wall (bit 1) alone does not hide: mask 2.
    let mut g = Fake::one();
    g.set(3, 0, bits::WALL);
    assert_eq!(sight_hidden(true, &a, &b, &g), Ok(false));
    // Pulled ends skip the cells under the units.
    let mut h = Fake::one();
    h.set(0, 0, bits::VISIBLE);
    h.set(6, 0, bits::VISIBLE);
    assert_eq!(sight_hidden(true, &a, &b, &h), Ok(false));
}

#[test]
fn sight_negative_sizes_are_kept_and_equal_coordinates_pull_backwards() {
    // A negative size is kept (not clamped to 0): sa + sb = 0, so
    // dx + dy = 0 < 0 is false and a line is made. Equal coordinates take
    // the "else" branch of the pull (a −= sa, b += sb): (5 − (−1), 5 + 1).
    let a = unit(R0, 5, 5, -1);
    let b = unit(None, 5, 5, 1);
    assert_eq!(sight_line(&a, &b), Some(((6, 6), (6, 6))));
    // A large size is capped at 2 whichever end it is on.
    assert_eq!(
        sight_line(&unit(R0, 0, 0, 1), &unit(None, 10, 0, 9)),
        Some(((1, 0), (8, 0)))
    );
}

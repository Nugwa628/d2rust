//! The sight test that hides units in `LOSDraw` levels (the unit entry of
//! the draw order runs it) and the line test it runs.
//!
//! The rooms are reached through d2-sim's [`CollisionRooms`] (sub-tile
//! rect, adjacency array, collision grid) and the cell lookup is
//! d2-sim's [`find_room`]; the client's DRLG copy provides
//! them, with its room indices as [`RoomId`]s. Plain integer math.

use d2_sim::drlg::collision::bits;
use d2_sim::path::collision::find_room;
use d2_sim::path::CollisionRooms;
use d2_sim::units::RoomId;

#[cfg(test)]
#[path = "sight_tests.rs"]
mod tests;

/// Mask of the sight test's line: collision bit 0x0002.
pub const SIGHT_MASK: u16 = bits::VISIBLE;

/// Sizes at or above this read as [`SIZE_CAP`].
const SIZE_CAP: i32 = 2;

/// Result of the line test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineResult {
    /// 0: every cell up to "to" tested zero.
    Clear,
    /// 1: blocked. `at` is the stop cell written back to "to"; `None`
    /// when no cell was reached (room null, or the start in no room),
    /// where nothing is written back.
    Blocked { at: Option<(i32, i32)> },
}

impl LineResult {
    pub fn is_blocked(self) -> bool {
        matches!(self, LineResult::Blocked { .. })
    }
}

/// Inputs whose outcome is not known.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SightError {
    /// The current room holds the cell by its rect but has no collision
    /// grid cell there; what such a cell tests is not known.
    #[error(
        "room {} has no collision grid cell at ({x}, {y}): draw-order-2.md §16 r2 \
         names no value for a missing grid",
        .room.0
    )]
    NoGridCell { room: RoomId, x: i32, y: i32 },
}

/// One end of the sight line: the unit's room, sub-tile position
/// and path size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SightUnit {
    pub room: Option<RoomId>,
    pub x: i32,
    pub y: i32,
    /// Signed (a monster's `SizeX` is): negative values are kept.
    pub size: i32,
}

/// The line test: room `room`, `from` → `to` in sub-tiles,
/// cells tested against `mask`.
pub fn line_test<R: CollisionRooms + ?Sized>(
    rooms: &R,
    room: Option<RoomId>,
    from: (i32, i32),
    to: (i32, i32),
    mask: u16,
) -> Result<LineResult, SightError> {
    // Null room → blocked; a start outside the room switches to the
    // room containing it among the room and its adjacent rooms.
    let Some(room) = room else {
        return Ok(LineResult::Blocked { at: None });
    };
    let (x0, y0) = from;
    let (x1, y1) = to;
    let Some(mut current) = find_room(rooms, Some(room), x0, y0) else {
        return Ok(LineResult::Blocked { at: None });
    };

    // Distances and steps (sign +1 for 0).
    let dx = (x1 - x0).abs();
    let dy = (y1 - y0).abs();
    let sx = if x1 < x0 { -1 } else { 1 };
    let sy = if y1 < y0 { -1 } else { 1 };
    // The major axis: y when dx < dy, else x (dx = dy = 0, dx = 0 and
    // dy = 0 are the degenerate cases of the same walk).
    let y_major = dx < dy;
    let (major, minor) = if y_major { (dy, dx) } else { (dx, dy) };

    let (mut x, mut y) = (x0, y0);
    let mut e = 0;
    let mut first = true;
    loop {
        if !first {
            // Leaving the current room's rect switches rooms.
            let inside = rooms
                .subtile_rect(current)
                .is_some_and(|r| r.contains(x, y));
            if !inside {
                match find_room(rooms, Some(current), x, y) {
                    Some(r) => current = r,
                    None => return Ok(LineResult::Blocked { at: Some((x, y)) }),
                }
            }
        }
        first = false;

        // The first non-zero cell blocks.
        let value =
            rooms
                .grid(current)
                .and_then(|g| g.get(x, y))
                .ok_or(SightError::NoGridCell {
                    room: current,
                    x,
                    y,
                })?;
        if value & mask != 0 {
            return Ok(LineResult::Blocked { at: Some((x, y)) });
        }

        // Ends after the cell of row y1 (y major) / column x1.
        if (y_major && y == y1) || (!y_major && x == x1) {
            return Ok(LineResult::Clear);
        }
        // Major step, then the error term (starts at 0) steps the minor
        // coordinate when it reaches the major distance.
        if y_major {
            y += sy;
        } else {
            x += sx;
        }
        e += minor;
        if e >= major {
            e -= major;
            if y_major {
                x += sx;
            } else {
                y += sy;
            }
        }
    }
}

/// The sight test before the line test: the line's ends `(a, b)` after the size
/// test and the end pulling, or `None` when the units are close enough to
/// pass without a line test.
pub fn sight_line(a: &SightUnit, b: &SightUnit) -> Option<((i32, i32), (i32, i32))> {
    let sa = a.size.min(SIZE_CAP);
    let sb = b.size.min(SIZE_CAP);
    let (mut ax, mut ay, mut bx, mut by) = (a.x, a.y, b.x, b.y);
    let dx = (bx - ax).abs();
    let dy = (by - ay).abs();
    if dx + dy < sa + sb {
        return None;
    }
    if sa != 0 || sb != 0 {
        // Each end moves its size toward the other on the pulled axis;
        // the axis distance is non-zero there (dx = dy = 0 passed above).
        if dy <= dx {
            let s = if ax < bx { 1 } else { -1 };
            ax += s * sa;
            bx -= s * sb;
        }
        if dy >= dx {
            let s = if ay < by { 1 } else { -1 };
            ay += s * sa;
            by -= s * sb;
        }
    }
    Some(((ax, ay), (bx, by)))
}

/// Sight test: whether unit `b` is hidden from the local player
/// `a`. `los_draw` is the leveldefs `LOSDraw` of the local player's level.
pub fn sight_hidden<R: CollisionRooms + ?Sized>(
    los_draw: bool,
    a: &SightUnit,
    b: &SightUnit,
    rooms: &R,
) -> Result<bool, SightError> {
    // The level gate.
    if !los_draw {
        return Ok(false);
    }
    // No room for `a` → passes.
    let Some(room) = a.room else {
        return Ok(false);
    };
    let Some((from, to)) = sight_line(a, b) else {
        return Ok(false);
    };
    Ok(line_test(rooms, Some(room), from, to, SIGHT_MASK)?.is_blocked())
}

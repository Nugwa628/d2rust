//! Footprints: pattern and size stamps with their markers, the
//! per-kind add and remove, mask and shape changes; moving a footprint
//! try move, forced move, missile move and teleport.

use crate::drlg::collision::bits;
use crate::units::RoomId;

use super::collision::{box_apply, find_room, pattern_value, size_value, CollisionRooms, PLUS};
use super::record::{flags, pattern_of_size, DynamicPath, ObjectShape, PathPoint, UnitShape};
use super::tables::PathTables;
use super::PathError;

/// Cells of a pattern: 0 the cell; 1, 3, 5 plus; 2, 4 3×3 box.
/// `None` for any other pattern.
fn pattern_cells(pattern: u32) -> Option<&'static [(i32, i32)]> {
    const POINT: [(i32, i32); 1] = [(0, 0)];
    const BOX: [(i32, i32); 9] = [
        (-1, -1),
        (0, -1),
        (1, -1),
        (-1, 0),
        (0, 0),
        (1, 0),
        (-1, 1),
        (0, 1),
        (1, 1),
    ];
    match pattern {
        0 => Some(&POINT),
        1 | 3 | 5 => Some(&PLUS),
        2 | 4 => Some(&BOX),
        _ => None,
    }
}

/// Marker of a pattern stamp: (marker, cells).
fn pattern_marker(pattern: u32) -> Option<(u16, &'static [(i32, i32)])> {
    const CENTRE: [(i32, i32); 1] = [(0, 0)];
    match pattern {
        1 => Some((bits::NO_PATH, &CENTRE)),
        2 => Some((bits::NO_PATH, &PLUS)),
        3 => Some((bits::PET, &CENTRE)),
        4 => Some((bits::PET, &PLUS)),
        _ => None,
    }
}

/// OR (set) or AND out (clear) `mask` on each cell (dx, dy) around
/// (x, y). Every cell is looked up separately from the room argument
/// (box walk rule), not from the centre's room; cells without a room are
/// skipped.
fn apply_cells<R: CollisionRooms + ?Sized>(
    rooms: &mut R,
    room: Option<RoomId>,
    x: i32,
    y: i32,
    cells: &[(i32, i32)],
    mask: u16,
    set: bool,
) {
    for &(dx, dy) in cells {
        let (cx, cy) = (x.wrapping_add(dx), y.wrapping_add(dy));
        let Some(r) = find_room(&*rooms, room, cx, cy) else {
            continue;
        };
        if let Some(m) = rooms.grid_mut(r).and_then(|g| g.get_mut(cx, cy)) {
            if set {
                *m |= mask;
            } else {
                *m &= !mask;
            }
        }
    }
}

fn pattern_apply<R: CollisionRooms + ?Sized>(
    rooms: &mut R,
    room: Option<RoomId>,
    x: i32,
    y: i32,
    pattern: u32,
    mask: u16,
    set: bool,
) {
    if room.is_none() {
        return;
    }
    // Pattern 0 stamps and clears nothing (its jump-table entry is
    // empty; the pattern query still tests the point); patterns above 5
    // do nothing.
    if pattern == 0 {
        return;
    }
    let Some(cells) = pattern_cells(pattern) else {
        return;
    };
    apply_cells(rooms, room, x, y, cells, mask, set);
    if mask != 0 {
        if let Some((marker, cells)) = pattern_marker(pattern) {
            apply_cells(rooms, room, x, y, cells, marker, set);
        }
    }
}

/// Pattern stamp: the
/// pattern's cells with `mask`, then, only when `mask` ≠ 0, its marker.
pub fn stamp_pattern<R: CollisionRooms + ?Sized>(
    rooms: &mut R,
    room: Option<RoomId>,
    x: i32,
    y: i32,
    pattern: u32,
    mask: u16,
) {
    pattern_apply(rooms, room, x, y, pattern, mask, true);
}

/// Pattern clear: AND out the mask (and, when the
/// mask ≠ 0, the marker); a null room does nothing.
pub fn clear_pattern<R: CollisionRooms + ?Sized>(
    rooms: &mut R,
    room: Option<RoomId>,
    x: i32,
    y: i32,
    pattern: u32,
    mask: u16,
) {
    pattern_apply(rooms, room, x, y, pattern, mask, false);
}

fn size_apply<R: CollisionRooms + ?Sized>(
    rooms: &mut R,
    room: Option<RoomId>,
    x: i32,
    y: i32,
    size: i32,
    mask: u16,
    set: bool,
) {
    let cells = match size {
        1 => pattern_cells(0),
        2 => pattern_cells(1),
        3 => pattern_cells(2),
        _ => None,
    };
    if let Some(cells) = cells {
        if room.is_some() {
            apply_cells(rooms, room, x, y, cells, mask, set);
        }
    }
}

/// Size stamp: size 1 the cell, 2 plus, 3 box,
/// others nothing; no marker.
pub fn stamp_size<R: CollisionRooms + ?Sized>(
    rooms: &mut R,
    room: Option<RoomId>,
    x: i32,
    y: i32,
    size: i32,
    mask: u16,
) {
    size_apply(rooms, room, x, y, size, mask, true);
}

/// Size clear.
pub fn clear_size<R: CollisionRooms + ?Sized>(
    rooms: &mut R,
    room: Option<RoomId>,
    x: i32,
    y: i32,
    size: i32,
    mask: u16,
) {
    size_apply(rooms, room, x, y, size, mask, false);
}

/// The shape a unit's footprint is stamped with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FootShape {
    /// Players and monsters: the path's pattern (+0x48).
    Pattern(u32),
    /// Objects: sizeX × sizeY box.
    Box { size_x: u32, size_y: u32 },
    /// Others: the unit size.
    Size(i32),
}

/// A unit's footprint: where and with what.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Footprint {
    pub room: Option<RoomId>,
    pub x: i32,
    pub y: i32,
    pub shape: FootShape,
    pub mask: u16,
}

impl Footprint {
    fn apply<R: CollisionRooms + ?Sized>(&self, rooms: &mut R, set: bool) {
        match self.shape {
            FootShape::Pattern(p) => {
                pattern_apply(rooms, self.room, self.x, self.y, p, self.mask, set)
            }
            FootShape::Box { size_x, size_y } => box_apply(
                rooms,
                self.room,
                self.x,
                self.y,
                (size_x, size_y),
                self.mask,
                set,
            ),
            FootShape::Size(s) => size_apply(rooms, self.room, self.x, self.y, s, self.mask, set),
        }
    }
}

/// Footprint add: stamp at the unit's position
/// and room with its footprint mask.
pub fn add_footprint<R: CollisionRooms + ?Sized>(rooms: &mut R, fp: &Footprint) {
    fp.apply(rooms, true);
}

/// Who a footprint removal is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoveRule {
    /// Player in `mode` (0 DT, 17 DD keep their footprint).
    Player { mode: u32 },
    /// Monster in `mode` (0 DT, 12 DD keep it).
    Monster { mode: u32 },
    /// Object in `mode`: cleared when `HasCollision[mode]` ≠ 0.
    Object { mode: u32, shape: ObjectShape },
    /// Missiles, items, tiles: always.
    Other,
}

/// Footprint remove: clears when
/// `force` or the type's condition holds; returns whether it cleared.
pub fn remove_footprint<R: CollisionRooms + ?Sized>(
    rooms: &mut R,
    fp: &Footprint,
    rule: RemoveRule,
    force: bool,
) -> bool {
    let clear = force
        || match rule {
            RemoveRule::Player { mode } => mode != 0 && mode != 17,
            RemoveRule::Monster { mode } => mode != 0 && mode != 12,
            RemoveRule::Object { mode, shape } => shape.collides_in(mode),
            RemoveRule::Other => true,
        };
    if clear {
        fp.apply(rooms, false);
    }
    clear
}

/// Footprint mask change: clear
/// the old footprint with the current pattern (missiles: size) and the
/// old mask, store `mask`, stamp again.
pub fn set_foot_mask<R: CollisionRooms + ?Sized>(
    rooms: &mut R,
    path: &mut DynamicPath,
    is_missile: bool,
    mask: u16,
) {
    let (x, y) = (path.x(), path.y());
    if is_missile {
        clear_size(rooms, path.room, x, y, path.unit_size, path.foot_mask);
        path.foot_mask = mask;
        stamp_size(rooms, path.room, x, y, path.unit_size, mask);
    } else {
        clear_pattern(rooms, path.room, x, y, path.pattern, path.foot_mask);
        path.foot_mask = mask;
        stamp_pattern(rooms, path.room, x, y, path.pattern, mask);
    }
}

/// Pattern set: writes +0x48, no restamp.
pub fn set_pattern(path: &mut DynamicPath, pattern: u32) {
    path.pattern = pattern;
}

/// Path reset: the pattern recomputed
/// from the stored size (so a wraith's pattern 5 becomes
/// the size pattern); with a room, the old footprint is removed (forced)
/// first and the new one stamped with the path's footprint mask.
pub fn reset_pattern<R: CollisionRooms + ?Sized>(
    rooms: &mut R,
    tables: &PathTables,
    path: &mut DynamicPath,
    shape: &UnitShape,
) {
    let pattern = pattern_of_size(tables, path.unit_size, shape);
    if path.room.is_none() {
        path.pattern = pattern;
        return;
    }
    let (x, y) = (path.x(), path.y());
    clear_pattern(rooms, path.room, x, y, path.pattern, path.foot_mask);
    path.pattern = pattern;
    stamp_pattern(rooms, path.room, x, y, pattern, path.foot_mask);
}

/// Dead-body footprint: remove (force), pattern := 5, mask
/// := 0x8000 through the mask change.
pub fn make_corpse_footprint<R: CollisionRooms + ?Sized>(rooms: &mut R, path: &mut DynamicPath) {
    let fp = Footprint {
        room: path.room,
        x: path.x(),
        y: path.y(),
        shape: FootShape::Pattern(path.pattern),
        mask: path.foot_mask,
    };
    remove_footprint(rooms, &fp, RemoveRule::Other, true);
    set_pattern(path, 5);
    set_foot_mask(rooms, path, false, bits::CORPSE);
}

/// Try move `(room, old, new, pattern, foot, test)` (rule
/// 1): clear at old; r := pattern query at new with `test`; r ≠ 0 →
/// stamp at old again and return r; else stamp at new and return 0.
pub fn try_move<R: CollisionRooms + ?Sized>(
    rooms: &mut R,
    room: Option<RoomId>,
    old: (i32, i32),
    new: (i32, i32),
    pattern: u32,
    foot: u16,
    test: u16,
) -> u16 {
    clear_pattern(rooms, room, old.0, old.1, pattern, foot);
    let r = pattern_value(&*rooms, room, new.0, new.1, pattern, test);
    if r != 0 {
        stamp_pattern(rooms, room, old.0, old.1, pattern, foot);
        r
    } else {
        stamp_pattern(rooms, room, new.0, new.1, pattern, foot);
        0
    }
}

/// Forced move with one room for both points, as the
/// unit footprint move passes it.
pub fn forced_move<R: CollisionRooms + ?Sized>(
    rooms: &mut R,
    room: Option<RoomId>,
    old: (i32, i32),
    new: (i32, i32),
    pattern: u32,
    foot: u16,
) {
    forced_move_rooms(rooms, room, old, room, new, pattern, foot);
}

/// Forced move `(room1, old, room2, new, pattern, foot)` (rule
/// rule 2): clear at old looked up from `room1`, stamp at new looked up
/// from `room2`, no test; `room1` null → nothing (no stamp either).
pub fn forced_move_rooms<R: CollisionRooms + ?Sized>(
    rooms: &mut R,
    room1: Option<RoomId>,
    old: (i32, i32),
    room2: Option<RoomId>,
    new: (i32, i32),
    pattern: u32,
    foot: u16,
) {
    if room1.is_none() {
        return;
    }
    clear_pattern(rooms, room1, old.0, old.1, pattern, foot);
    stamp_pattern(rooms, room2, new.0, new.1, pattern, foot);
}

/// Missile move (rule 3, size shapes): clear at old,
/// query at new; stamp at new unless the result has 0x1 or 0x4, else at
/// old; returns the result.
pub fn missile_move<R: CollisionRooms + ?Sized>(
    rooms: &mut R,
    room: Option<RoomId>,
    old: (i32, i32),
    new: (i32, i32),
    size: i32,
    foot: u16,
    test: u16,
) -> u16 {
    clear_size(rooms, room, old.0, old.1, size, foot);
    let r = size_value(&*rooms, room, new.0, new.1, size, test);
    let at = if r & 0x5 == 0 { new } else { old };
    stamp_size(rooms, room, at.0, at.1, size, foot);
    r
}

/// What teleport needs from the walk code (provider: the wiring,
/// with `path::walk`). Teleport takes one context that is both the
/// rooms and the motion: set position's room recache reads the rooms
/// the footprint move just wrote.
pub trait PathMotion {
    /// Set position `(Q, hint)` to the
    /// cell centre of (x, y), with the room recache (rule 9) when flag
    /// 0x1 is set, `hint` the destination room.
    fn set_position(&mut self, path: &mut DynamicPath, x: i32, y: i32, hint: Option<RoomId>);
    /// Movement reset.
    fn reset(&mut self, path: &mut DynamicPath);
}

/// Teleport, always succeeds;
/// a non-zero point without a room is a fatal error,
/// returned as an error before any change. Clears use the path's room;
/// the destination's query and stamp use the destination room `room`.
pub fn teleport<C: CollisionRooms + PathMotion + ?Sized>(
    c: &mut C,
    path: &mut DynamicPath,
    is_missile: bool,
    room: Option<RoomId>,
    x: i32,
    y: i32,
) -> Result<(), PathError> {
    if (x, y) != (0, 0) && room.is_none() {
        return Err(PathError::TeleportNoRoom { x, y });
    }
    let old = (path.x(), path.y());
    let zero = (x, y) == (0, 0);
    if is_missile {
        if zero {
            clear_size(c, path.room, old.0, old.1, path.unit_size, path.foot_mask);
            path.collided_mask = 0;
        } else {
            // Flag 0x8 := the cell changed.
            let moved = old != (x, y);
            path.flags = (path.flags & !flags::MOVED) | if moved { flags::MOVED } else { 0 };
            // Clear at old (path room), query and stamp at
            // new (destination room).
            clear_size(c, path.room, old.0, old.1, path.unit_size, path.foot_mask);
            path.collided_mask = size_value(&*c, room, x, y, path.unit_size, path.move_mask);
            stamp_size(c, room, x, y, path.unit_size, path.foot_mask);
            path.saved_count = 1;
            path.saved_steps[0] = PathPoint {
                x: x as u16,
                y: y as u16,
            };
        }
    } else if zero {
        clear_pattern(c, path.room, old.0, old.1, path.pattern, path.foot_mask);
    } else {
        // A warp to a room not adjacent to the old one still stamps at
        // the destination.
        forced_move_rooms(
            c,
            path.room,
            old,
            room,
            (x, y),
            path.pattern,
            path.foot_mask,
        );
    }
    if room != path.room {
        path.flags |= flags::OUTSIDE_ROOM;
    }
    c.set_position(path, x, y, room);
    c.reset(path);
    Ok(())
}

/// Teleport, then point count := 0.
pub fn teleport_and_clear<C: CollisionRooms + PathMotion + ?Sized>(
    c: &mut C,
    path: &mut DynamicPath,
    is_missile: bool,
    room: Option<RoomId>,
    x: i32,
    y: i32,
) -> Result<(), PathError> {
    teleport(c, path, is_missile, room, x, y)?;
    path.point_count = 0;
    Ok(())
}

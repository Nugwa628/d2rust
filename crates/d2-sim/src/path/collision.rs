//! Collision queries over the active rooms' grids: cell lookup
//! through a room's adjacency array, masked cell values (0x27 where no
//! room or grid exists), point / plus / box shapes, and the box set and
//! clear of objects. The rooms are reached through [`CollisionRooms`].

use crate::drlg::{CollisionGrid, TileRect};
use crate::units::RoomId;

/// Value of a cell without a room or grid, returned unmasked.
pub const MISSING_ROOM: u16 = 0x27;

/// Result of a size / pattern query with an unknown shape.
pub const UNKNOWN_SHAPE: u16 = 0xFFFF;

/// Collision masks the path code uses (bits: `drlg::collision::bits`).
pub mod masks {
    /// Player move / placement: WALL, NOPLAYER, OBJECT, DOOR, NO_PATH.
    pub const PLAYER_MOVE: u16 = 0x1C09;
    /// Monster move: WALL, OBJECT, DOOR, NO_PATH, PET.
    pub const MONSTER_MOVE: u16 = 0x3C01;
    /// Item floor: WALL, ITEM, OBJECT, DOOR, NO_PATH, PET.
    pub const ITEM_FLOOR: u16 = 0x3E01;
    /// Walk-back field: WALL, DOOR.
    pub const WALK_BACK: u16 = 0x801;
}

/// The active rooms as the path code sees them. Provider: the wiring,
/// on `crate::drlg::Drlg` (active room by
/// `RoomId`: its `subtiles` rect, `adjacency` mapped to `RoomId`s and
/// `collision` grid).
pub trait CollisionRooms {
    /// The room's sub-tile rect (room +0x4C x, +0x50 y, +0x54 w, +0x58 h);
    /// `None` for a room that is not active.
    fn subtile_rect(&self, room: RoomId) -> Option<TileRect>;
    /// Length of the room's adjacency array.
    fn adjacent_count(&self, room: RoomId) -> usize;
    /// Entry `i` of the adjacency array, index order 0..count.
    fn adjacent(&self, room: RoomId, i: usize) -> Option<RoomId>;
    /// The room's collision grid, if it has one.
    fn grid(&self, room: RoomId) -> Option<&CollisionGrid>;
    /// Mutable collision grid, for footprints.
    fn grid_mut(&mut self, room: RoomId) -> Option<&mut CollisionGrid>;
}

/// Cell lookup: the room itself if
/// its rect contains the cell, else the first room of its adjacency
/// array that does, else none. A null room gives none.
pub fn find_room<R: CollisionRooms + ?Sized>(
    rooms: &R,
    room: Option<RoomId>,
    x: i32,
    y: i32,
) -> Option<RoomId> {
    let room = room?;
    if rooms.subtile_rect(room)?.contains(x, y) {
        return Some(room);
    }
    (0..rooms.adjacent_count(room))
        .filter_map(|i| rooms.adjacent(room, i))
        .find(|&r| {
            rooms
                .subtile_rect(r)
                .is_some_and(|rect| rect.contains(x, y))
        })
}

/// Value of the cell in a found room: no room or no grid →
/// 0x27 unmasked.
fn value_in<R: CollisionRooms + ?Sized>(
    rooms: &R,
    found: Option<RoomId>,
    x: i32,
    y: i32,
    mask: u16,
) -> u16 {
    found
        .and_then(|r| rooms.grid(r))
        .and_then(|g| g.get(x, y))
        .map_or(MISSING_ROOM, |v| v & mask)
}

/// Point query: the masked value of the
/// cell looked up from `room`.
pub fn point_value<R: CollisionRooms + ?Sized>(
    rooms: &R,
    room: Option<RoomId>,
    x: i32,
    y: i32,
    mask: u16,
) -> u16 {
    value_in(rooms, find_room(rooms, room, x, y), x, y, mask)
}

/// The five plus cells: centre, x ± 1, y ± 1.
pub(super) const PLUS: [(i32, i32); 5] = [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)];

/// Plus query: the centre's room from `room`; each cell
/// looked up from the centre's room; OR of the masked values. A missing
/// centre room gives 0x27.
pub fn plus_value<R: CollisionRooms + ?Sized>(
    rooms: &R,
    room: Option<RoomId>,
    x: i32,
    y: i32,
    mask: u16,
) -> u16 {
    let Some(centre) = find_room(rooms, room, x, y) else {
        return MISSING_ROOM;
    };
    PLUS.iter().fold(0, |acc, &(dx, dy)| {
        let (cx, cy) = (x.wrapping_add(dx), y.wrapping_add(dy));
        acc | value_in(rooms, find_room(rooms, Some(centre), cx, cy), cx, cy, mask)
    })
}

/// Box corners: left = x − sx/2, bottom = y − sy/2 (unsigned
/// halving), right = left + sx − 1, top = bottom + sy − 1.
pub(super) fn box_corners(x: i32, y: i32, sx: u32, sy: u32) -> (i32, i32, i32, i32) {
    let left = x.wrapping_sub((sx / 2) as i32);
    let bottom = y.wrapping_sub((sy / 2) as i32);
    (
        left,
        bottom,
        left.wrapping_add(sx as i32).wrapping_sub(1),
        bottom.wrapping_add(sy as i32).wrapping_sub(1),
    )
}

/// Box walk (split): the room of (left, bottom)
/// from `room`; an empty box (bottom > top or left > right) or a
/// lower-left cell without a room gives no boxes (`missing`: the query
/// reads 0x27). Box 1 is the box clipped at that room's right and top
/// edges; `inside` runs on it; the right strip {R, bottom, right, top}
/// (full height: it takes the corner beyond both edges) and the top
/// strip {left, T, box 1 right, top} follow, in that order, each walked
/// again with that room as the lookup start.
fn walk_box<R: CollisionRooms + ?Sized, A>(
    rooms: &mut R,
    room: Option<RoomId>,
    (left, bottom, right, top): (i32, i32, i32, i32),
    acc: &mut A,
    inside: &mut impl FnMut(&mut R, RoomId, (i32, i32, i32, i32), &mut A),
    missing: &mut impl FnMut(&mut A),
) {
    if left > right || bottom > top {
        missing(acc);
        return;
    }
    let Some(r) = find_room(&*rooms, room, left, bottom) else {
        missing(acc);
        return;
    };
    let rect = rooms.subtile_rect(r).expect("found rooms are active");
    let in_right = right.min(rect.x + rect.w - 1);
    let in_top = top.min(rect.y + rect.h - 1);
    inside(rooms, r, (left, bottom, in_right, in_top), acc);
    if right > in_right {
        walk_box(
            rooms,
            Some(r),
            (in_right + 1, bottom, right, top),
            acc,
            inside,
            missing,
        );
    }
    if top > in_top {
        walk_box(
            rooms,
            Some(r),
            (left, in_top + 1, in_right, top),
            acc,
            inside,
            missing,
        );
    }
}

/// A read-only view for [`walk_box`], which takes `&mut R` to share the
/// walk with set and clear.
struct ReadOnly<'a, R: ?Sized>(&'a R);

impl<R: CollisionRooms + ?Sized> CollisionRooms for ReadOnly<'_, R> {
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
    fn grid_mut(&mut self, _: RoomId) -> Option<&mut CollisionGrid> {
        None
    }
}

/// Box query: OR of the masked cells of an sx × sy box
/// centred as rule 4 says; 0x27 for any (sub-)box without a room.
pub fn box_value<R: CollisionRooms + ?Sized>(
    rooms: &R,
    room: Option<RoomId>,
    x: i32,
    y: i32,
    (sx, sy): (u32, u32),
    mask: u16,
) -> u16 {
    let mut view = ReadOnly(rooms);
    let mut acc = 0u16;
    walk_box(
        &mut view,
        room,
        box_corners(x, y, sx, sy),
        &mut acc,
        &mut |v, r, (l, b, rt, t), acc| {
            for cy in b..=t {
                for cx in l..=rt {
                    *acc |= value_in(&*v, Some(r), cx, cy, mask);
                }
            }
        },
        &mut |acc| *acc |= MISSING_ROOM,
    );
    acc
}

/// Box set (set = true) / clear: OR `mask` into / AND its
/// complement out of each cell of the
/// box; (sub-)boxes without a room are skipped.
pub fn box_apply<R: CollisionRooms + ?Sized>(
    rooms: &mut R,
    room: Option<RoomId>,
    x: i32,
    y: i32,
    (sx, sy): (u32, u32),
    mask: u16,
    set: bool,
) {
    walk_box(
        rooms,
        room,
        box_corners(x, y, sx, sy),
        &mut (),
        &mut |rooms, r, (l, b, rt, t), _| {
            let Some(g) = rooms.grid_mut(r) else { return };
            for cy in b..=t {
                for cx in l..=rt {
                    if let Some(m) = g.get_mut(cx, cy) {
                        if set {
                            *m |= mask;
                        } else {
                            *m &= !mask;
                        }
                    }
                }
            }
        },
        &mut |_| {},
    );
}

/// Size query: size 0, 1 point; 2 plus; 3 box
/// 3×3; other → 0xFFFF.
pub fn size_value<R: CollisionRooms + ?Sized>(
    rooms: &R,
    room: Option<RoomId>,
    x: i32,
    y: i32,
    size: i32,
    mask: u16,
) -> u16 {
    match size {
        0 | 1 => point_value(rooms, room, x, y, mask),
        2 => plus_value(rooms, room, x, y, mask),
        3 => box_value(rooms, room, x, y, (3, 3), mask),
        _ => UNKNOWN_SHAPE,
    }
}

/// Pattern query: pattern 0 point; 1, 3, 5
/// plus; 2, 4 box 3×3; other → 0xFFFF.
pub fn pattern_value<R: CollisionRooms + ?Sized>(
    rooms: &R,
    room: Option<RoomId>,
    x: i32,
    y: i32,
    pattern: u32,
    mask: u16,
) -> u16 {
    match pattern {
        0 => point_value(rooms, room, x, y, mask),
        1 | 3 | 5 => plus_value(rooms, room, x, y, mask),
        2 | 4 => box_value(rooms, room, x, y, (3, 3), mask),
        _ => UNKNOWN_SHAPE,
    }
}

/// Pattern test: 1 if any cell collides, else
/// 0; an unknown pattern gives 1.
pub fn pattern_collides<R: CollisionRooms + ?Sized>(
    rooms: &R,
    room: Option<RoomId>,
    x: i32,
    y: i32,
    pattern: u32,
    mask: u16,
) -> bool {
    match pattern {
        0..=5 => pattern_value(rooms, room, x, y, pattern, mask) != 0,
        _ => true,
    }
}

/// What the unit search reads besides the rooms.
pub trait UnitsAtPoint: CollisionRooms {
    /// A unit.
    type Unit: Copy;
    /// The room's unit list from its head (+0x74) along unit +0xE8
    /// (unit-list order); `None`: no list head.
    fn room_units(&self, room: RoomId) -> Option<Vec<Self::Unit>>;
    /// Unit type (unit +0x00).
    fn unit_type(&self, unit: Self::Unit) -> crate::units::UnitType;
    /// Current mode (unit +0x10).
    fn unit_mode(&self, unit: Self::Unit) -> u32;
    /// Unit size.
    fn unit_size(&self, unit: Self::Unit) -> i32;
    /// The dynamic path's sub-tile position (path null → (0, 0)).
    fn unit_point(&self, unit: Self::Unit) -> (i32, i32);
}

/// Hit test of the box walk, step 5 (jump table): the query
/// shape of size `r` overlaps the unit shape of size `s` (both 1..=3) at
/// distance (dx, dy).
pub fn shapes_overlap(r: i32, s: i32, dx: i32, dy: i32) -> bool {
    let plus_box = (dx <= 2 && dy <= 1) || (dx <= 1 && dy <= 2);
    match (r, s) {
        (1, 1) => dx == 0 && dy == 0,
        (1, 2) | (2, 1) => dx + dy <= 1,
        (1, 3) | (3, 1) => dx <= 1 && dy <= 1,
        (2, 2) => dx + dy <= 2,
        (2, 3) | (3, 2) => plus_box,
        (3, 3) => dx <= 2 && dy <= 2,
        _ => false,
    }
}

/// Unit at a point:
/// the first unit, over the room's adjacency array (which holds the
/// room itself) in index order and each room's unit list in order, whose
/// shape overlaps the query shape of size `r` at (x, y) and that
/// `accept` takes. Players in mode 0 / 17, monsters in mode 0 / 12,
/// objects, items and tiles are skipped; missiles are candidates.
pub fn unit_at_point<V: UnitsAtPoint + ?Sized>(
    v: &V,
    room: Option<RoomId>,
    x: i32,
    y: i32,
    r: i32,
    mut accept: impl FnMut(V::Unit) -> bool,
) -> Option<V::Unit> {
    use crate::units::UnitType;
    // Step 1.
    let room = room?;
    if !(1..=3).contains(&r) {
        return None;
    }
    // Step 2.
    for i in 0..v.adjacent_count(room) {
        let Some(adj) = v.adjacent(room, i) else {
            continue;
        };
        // Near-rect test (margin 2), as written: rejects only
        // when both sides fail on one axis.
        if let Some(rect) = v.subtile_rect(adj) {
            let (right, top) = (rect.x + rect.w, rect.y + rect.h);
            if (x + 2 < rect.x && x - 2 > right) || (y + 2 < rect.y && y - 2 > top) {
                continue;
            }
        }
        let Some(units) = v.room_units(adj) else {
            continue;
        };
        // Step 3.
        for u in units {
            let mode = v.unit_mode(u);
            match v.unit_type(u) {
                UnitType::Player if mode == 0 || mode == 17 => continue,
                UnitType::Monster if mode == 0 || mode == 12 => continue,
                UnitType::Player | UnitType::Monster | UnitType::Missile => {}
                _ => continue,
            }
            // Step 4.
            let s = v.unit_size(u);
            if s <= 0 {
                continue;
            }
            let s = s.min(3);
            let (ux, uy) = v.unit_point(u);
            // Step 5.
            if !shapes_overlap(r, s, (x - ux).abs(), (y - uy).abs()) {
                continue;
            }
            // Step 6.
            if accept(u) {
                return Some(u);
            }
        }
    }
    None
}

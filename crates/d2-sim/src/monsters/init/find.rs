//! The unit find with its filter
//! The rooms around a point (the start room alone when the
//! circle lies strictly inside it, else its adjacency array) and, per
//! room, the units of its list that pass the filter, in list order.
//! The world (rooms, unit records, tables) is reached through
//! [`FindWorld`]; the umod callbacks reach the whole find through
//! [`super::InitHost::find_units`].

use crate::units::{RoomId, UnitId, UnitType};

/// Finder flags (context +0x00).
pub mod find_flag {
    pub const PLAYERS: u32 = 0x01;
    pub const MONSTERS: u32 = 0x02;
    /// Monsters must be undead.
    pub const UNDEAD: u32 = 0x04;
    pub const MISSILES: u32 = 0x08;
    pub const OBJECTS: u32 = 0x10;
    pub const ITEMS: u32 = 0x20;
    /// Stop at the count limit (+0x18).
    pub const LIMIT: u32 = 0x40;
    /// Unit flags +0xC4 bit 0x4 set.
    pub const UNIT_FLAG_4: u32 = 0x80;
    /// The unit's room is not in town.
    pub const NOT_IN_TOWN: u32 = 0x100;
    /// The line from the context's line start is not blocked.
    pub const LINE: u32 = 0x200;
    /// Unit flags +0xC4 bit 0x8 set.
    pub const UNIT_FLAG_8: u32 = 0x400;
    /// The context callback (+0x24) returns 0.
    pub const CALLBACK: u32 = 0x800;
    /// Dead units only (players mode 17, monsters mode 12).
    pub const DEAD_ONLY: u32 = 0x1000;
    /// Town rooms are skipped.
    pub const SKIP_TOWN_ROOMS: u32 = 0x2000;
}

/// The callers' part of the 0x38-byte context.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindQuery {
    /// +0x00.
    pub flags: u32,
    /// +0x08 excluded unit.
    pub exclude: Option<UnitId>,
    /// +0x0C, +0x10.
    pub x: i32,
    pub y: i32,
    /// +0x14.
    pub r: i32,
    /// +0x18 count limit ([`find_flag::LIMIT`]).
    pub limit: i32,
    /// The line start of [`find_flag::LINE`].
    pub line_from: (i32, i32),
}

/// What the find reads.
pub trait FindWorld {
    /// Room box (x0, y0, w, h); `None` for no room.
    fn room_box(&self, room: RoomId) -> Option<(i32, i32, i32, i32)>;
    /// The adjacency array (the room
    /// itself included), in array order.
    fn adjacent(&self, room: RoomId) -> Vec<RoomId>;
    fn room_in_town(&self, room: RoomId) -> bool;
    /// The room's unit list (head +0x74, next +0xE8) in list order.
    fn room_units(&self, room: RoomId) -> Vec<UnitId>;
    fn unit_type(&self, unit: UnitId) -> Option<UnitType>;
    fn mode(&self, unit: UnitId) -> u32;
    fn position(&self, unit: UnitId) -> (i32, i32);
    /// Unit flags +0xC4.
    fn unit_flags(&self, unit: UnitId) -> u32;
    fn is_undead(&self, unit: UnitId) -> bool;
    /// The missile's row has `Explosion` (+0x04 bit 1); `None` without a
    /// row.
    fn missile_explosion(&self, unit: UnitId) -> Option<bool>;
    /// The line from `from` to the unit is blocked (steps,
    /// = 4).
    fn line_blocked(&self, from: (i32, i32), unit: UnitId) -> bool;
    /// The context callback (+0x24) on the unit.
    fn context_callback(&mut self, _unit: UnitId) -> i32 {
        0
    }
}

/// The circle lies strictly inside the room box.
pub fn strictly_inside((x0, y0, w, h): (i32, i32, i32, i32), x: i32, y: i32, r: i32) -> bool {
    x0 < x.wrapping_sub(r)
        && y0 < y.wrapping_sub(r)
        && x.wrapping_add(r) < x0.wrapping_add(w)
        && y.wrapping_add(r) < y0.wrapping_add(h)
}

/// The room is rejected when `x + r < x0` and `x − r > x0 +
/// w`, or `y + r < y0` and `y − r > y0 + h` (collect step 3). For r ≥
/// 0 and a box of non-negative size this never holds; for r < 0 it
/// rejects a room whose box lies strictly inside (x − a, x + a) or
/// (y − a, y + a) with a = |r|.
pub fn room_rejected((x0, y0, w, h): (i32, i32, i32, i32), x: i32, y: i32, r: i32) -> bool {
    (x.wrapping_add(r) < x0 && x.wrapping_sub(r) > x0.wrapping_add(w))
        || (y.wrapping_add(r) < y0 && y.wrapping_sub(r) > y0.wrapping_add(h))
}

/// The find from `start` (`None`: nothing found).
pub fn find_units<W: FindWorld>(w: &mut W, start: Option<RoomId>, q: &FindQuery) -> Vec<UnitId> {
    let Some(start) = start else {
        return Vec::new();
    };
    let rooms = match w.room_box(start) {
        Some(b) if strictly_inside(b, q.x, q.y, q.r) => vec![start],
        _ => w.adjacent(start),
    };
    let mut found = Vec::new();
    for room in rooms {
        if q.flags & find_flag::SKIP_TOWN_ROOMS != 0 && w.room_in_town(room) {
            continue;
        }
        // The room-box test.
        if w.room_box(room)
            .is_some_and(|b| room_rejected(b, q.x, q.y, q.r))
        {
            continue;
        }
        for u in w.room_units(room) {
            if accepts(w, u, room, q, found.len()) {
                found.push(u);
            }
        }
    }
    found
}

/// The filter on a unit of `room`'s list, with `count`
/// units accepted so far.
pub fn accepts<W: FindWorld>(
    w: &mut W,
    u: UnitId,
    room: RoomId,
    q: &FindQuery,
    count: usize,
) -> bool {
    let f = q.flags;
    if f & find_flag::LIMIT != 0 && i64::try_from(count).unwrap_or(i64::MAX) >= i64::from(q.limit) {
        return false;
    }
    let (ux, uy) = w.position(u);
    let dx = ux.wrapping_sub(q.x);
    let dy = uy.wrapping_sub(q.y);
    let d2 = dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx));
    if d2 > q.r.wrapping_mul(q.r) {
        return false;
    }
    let dead_only = f & find_flag::DEAD_ONLY != 0;
    let mode = w.mode(u);
    let ok = match w.unit_type(u) {
        Some(UnitType::Player) => {
            // The excluded unit is tested in the player branch (filter
            // step 3).
            f & find_flag::PLAYERS != 0
                && if dead_only {
                    mode == 17
                } else {
                    mode != 0 && mode != 17
                }
                && q.exclude != Some(u)
        }
        Some(UnitType::Monster) => {
            f & find_flag::MONSTERS != 0
                && if dead_only {
                    mode == 12
                } else {
                    mode != 0 && mode != 12
                }
                && (f & find_flag::UNDEAD == 0 || w.is_undead(u))
        }
        Some(UnitType::Object) => f & find_flag::OBJECTS != 0,
        Some(UnitType::Missile) => {
            f & find_flag::MISSILES != 0 && w.missile_explosion(u) == Some(false)
        }
        Some(UnitType::Item) => f & find_flag::ITEMS != 0,
        _ => false,
    };
    if !ok {
        return false;
    }
    let flags = w.unit_flags(u);
    if f & find_flag::UNIT_FLAG_4 != 0 && flags & 0x4 == 0 {
        return false;
    }
    if f & find_flag::UNIT_FLAG_8 != 0 && flags & 0x8 == 0 {
        return false;
    }
    // The unit's room is the room whose list holds it.
    if f & find_flag::NOT_IN_TOWN != 0 && w.room_in_town(room) {
        return false;
    }
    if f & find_flag::LINE != 0 && w.line_blocked(q.line_from, u) {
        return false;
    }
    if f & find_flag::CALLBACK != 0 && w.context_callback(u) != 0 {
        return false;
    }
    true
}

//! The skill bodies' room seams on the DRLG rooms: the level's `Teleport`
//! flag, the box collision, the free point and the placement of a unit.
//! Once the host has turned the path provider on
//! ([`crate::wiring::action::ActionHooks::enable_paths`]) they are
//! answered here, as [`UseView::line_blocked`] already is; without it
//! they stay [`Pending`]'s (tests, hosts without rooms).

use super::skill_use::UseView;
use crate::path::search::free_point;
use crate::path::Point;
use crate::units::{RoomId, UnitId};
use crate::wiring::action::Pending;
use crate::wiring::path::place::{place_unit, Rooms};
use crate::wiring::path::PathCtx;

use super::UseRest;

impl<X: Pending + UseRest> UseView<'_, X> {
    fn on_rooms(&self) -> bool {
        self.cv.v.h.paths.is_some()
    }

    /// The line from the unit's position to `to` is clear under `mask`
    /// (a cell walk from the unit's room); `None`
    /// without the provider. A unit without a room is blocked (rule 1).
    // D2RUST's own: the use pipeline's line test is the
    // plain cell walk from the caster's path position, as the server's
    // area-find line is (`umod_line_clear`).
    pub(super) fn rooms_line_clear(&self, u: UnitId, to: (i32, i32), mask: u32) -> Option<bool> {
        if !self.on_rooms() {
            return None;
        }
        let from = self.cv.v.h.path_position(u);
        let room = self.cv.game.lists.unit(u).and_then(|e| e.room());
        Some(
            !crate::path::line::line_test(
                &self.cv.v.h.drlg,
                room,
                Point::new(from.0, from.1),
                Point::new(to.0, to.1),
                mask as u16,
            )
            .blocked(),
        )
    }

    /// Whether the masked cell value at (x, y), looked up from `room`, is
    /// non-zero; `None` without the provider.
    pub(super) fn rooms_point_collides(
        &self,
        room: RoomId,
        (x, y): (i32, i32),
        mask: u32,
    ) -> Option<bool> {
        if !self.on_rooms() {
            return None;
        }
        Some(
            crate::path::collision::point_value(&self.cv.v.h.drlg, Some(room), x, y, mask as u16)
                != 0,
        )
    }

    /// The `Teleport` column of the room's level (`levels.txt`); `None`
    /// without the provider, a level or a row.
    pub(super) fn level_teleport(&self, r: RoomId) -> Option<i32> {
        if !self.on_rooms() {
            return None;
        }
        let level = self.cv.v.h.drlg.level_id(&*self.cv.game, r)?;
        let row = self
            .cv
            .v
            .h
            .tables
            .levels
            .get(usize::try_from(level).ok()?)?;
        Some(i32::from(row.teleport))
    }

    /// Whether the box of `size` at `at` collides on the rooms' grids.
    pub(super) fn rooms_box_collides(
        &self,
        r: RoomId,
        at: (i32, i32),
        size: i32,
        mask: u32,
    ) -> Option<bool> {
        use crate::path::place_seams::CollisionView;
        if !self.on_rooms() {
            return None;
        }
        let rooms = Rooms(&self.cv.v.h.drlg);
        // A size of 1 or less is a point query.
        let v = if size <= 1 {
            rooms.point_query(r, at.0, at.1, mask)
        } else {
            let s = size as u32;
            rooms.box_query(r, at.0, at.1, s, s, mask)
        };
        Some(v != 0)
    }

    /// Collision test `(room, x, y, pattern, mask)` ≠ 0 on the rooms' grids,
    /// with the unit's own collision pattern (the
    /// rule 5; the unit's path record, pattern 0 without one).
    // D2RUST's own: the leap's check reads the unit's path
    // pattern (taken from the call's arguments).
    pub(super) fn rooms_pattern_collides(
        &self,
        r: RoomId,
        at: (i32, i32),
        u: UnitId,
        mask: u32,
    ) -> Option<bool> {
        if !self.on_rooms() {
            return None;
        }
        let pattern = self
            .cv
            .v
            .h
            .paths
            .as_ref()
            .and_then(|p| p.dynamic(u))
            .map_or(0, |d| d.pattern);
        Some(crate::path::collision::pattern_collides(
            &self.cv.v.h.drlg,
            Some(r),
            at.0,
            at.1,
            pattern,
            mask as u16,
        ))
    }

    /// The free point near `at` on the rooms, with the room it is in.
    pub(super) fn rooms_free_point(
        &self,
        r: RoomId,
        at: (i32, i32),
        size: i32,
        mask: u32,
        fallback: bool,
    ) -> Option<Option<(RoomId, (i32, i32))>> {
        if !self.on_rooms() {
            return None;
        }
        let mut p = Point::new(at.0, at.1);
        let found = free_point(
            &Rooms(&self.cv.v.h.drlg),
            Some(r),
            &mut p,
            size,
            mask,
            fallback,
        );
        Some(found.ok().flatten().map(|room| (room, (p.x, p.y))))
    }

    /// Places the unit in room `r` at `at` through the path provider.
    pub(super) fn rooms_place_unit(
        &mut self,
        u: UnitId,
        r: Option<RoomId>,
        at: (i32, i32),
    ) -> Option<bool> {
        self.rooms_place_unit_exact(u, r, at, false)
    }

    /// Places the unit in room `r` at `at`: `exact` skips the free-point
    /// search.
    pub(super) fn rooms_place_unit_exact(
        &mut self,
        u: UnitId,
        r: Option<RoomId>,
        at: (i32, i32),
        exact: bool,
    ) -> Option<bool> {
        if !self.on_rooms() {
            return None;
        }
        let c = PathCtx::of(&mut self.cv.v, &mut *self.cv.game);
        Some(place_unit(c, u, r, at.0, at.1, exact, false))
    }
}

impl<X: Pending + UseRest> UseView<'_, X> {
    /// The path's target point (a cast at a point reads it back from the
    /// path record).
    // The point form of the mode start stores the cast point in the
    // unit's path target, so a cast at a point reads it back from the
    // path record. A host without the path provider has no record: the
    // point kept at the mode start (`UseRest::target_position`) stands in
    // when [`Pending::path_target_point`] has none (0, 0).
    // The path provider's record is read first: a monster's mode request
    // writes the point there (Imp Teleport's "no O, no T" branch reads it
    // at the action frame), and the host seam only answers for hosts
    // without the provider.
    pub(super) fn cast_target_point(&self, u: UnitId) -> (i32, i32) {
        let path = self
            .cv
            .v
            .h
            .paths
            .as_ref()
            .and_then(|p| p.dynamic(u))
            .map(|d| (i32::from(d.target_x), i32::from(d.target_y)));
        match path.unwrap_or_else(|| self.x().path_target_point(u)) {
            (0, 0) => self.x().target_position(u).unwrap_or((0, 0)),
            p => p,
        }
    }
}

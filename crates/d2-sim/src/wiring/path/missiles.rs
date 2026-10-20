//! The path seams of the missile server-do / server-hit bodies
//! ([`crate::missiles::MissileBodies`]) answered by the path provider:
//! the target point, the target position, set type, the step counts, the
//! direction and path-point setters, and the teleport. `None` from each
//! helper: the provider is off, the caller keeps its default answer.

use crate::game::Game;
use crate::path::record::PATH_POINTS;
use crate::path::walk::seams::PathWorld;
use crate::path::DynamicPath;
use crate::units::{RoomId, UnitId, UnitType};
use crate::wiring::action::{Pending, View, WiringError};

use super::PathCtx;

/// The step-count cap: 77.
pub const STEP_COUNT_CAP: i32 = PATH_POINTS as i32 - 1;

/// The step count stored for `n`: only the low byte of n, unsigned, capped
/// at 77. So -1 -> 255 -> 77, -256 -> 0, -200 -> 56, 300 -> 44.
pub fn step_count_byte(n: i32) -> u8 {
    (n as u8).min(STEP_COUNT_CAP as u8)
}

/// Set the path's distance budget and max path distance to
/// [`step_count_byte`].
pub fn set_step_counts(d: &mut DynamicPath, n: i32) {
    let b = step_count_byte(n);
    d.dist_budget = b;
    d.max_distance = b;
}

impl<X: Pending> View<'_, X> {
    /// The path's target point.
    pub(crate) fn path_target_xy(&self, unit: UnitId) -> Option<(i32, i32)> {
        let p = self.h.paths.as_ref()?;
        Some(
            p.dynamic(unit)
                .map_or((0, 0), |d| (i32::from(d.target_x), i32::from(d.target_y))),
        )
    }

    /// The path's target unit after a staleness check: a target whose
    /// stored type and GUID no longer resolve to the stored unit, or an
    /// item in mode 1 or 2, is cleared to none. The unit itself counts as
    /// none.
    pub(crate) fn path_target_checked(&mut self, game: &Game, unit: UnitId) -> Option<UnitId> {
        let d = self.h.paths.as_ref()?.dynamic(unit)?;
        let t = d.target_unit?;
        let stale = match game.lists.find_unit(t.ty, t.guid) {
            Some(f) if f == t.unit => self
                .units
                .get(f)
                .is_some_and(|r| r.ty == UnitType::Item && matches!(r.mode, 1 | 2)),
            _ => true,
        };
        if stale {
            if let Some(d) = self.h.paths.as_mut().and_then(|p| p.dynamic_mut(unit)) {
                d.target_unit = None;
            }
            return None;
        }
        (t.unit != unit).then_some(t.unit)
    }

    /// The target position: T := the checked path target
    /// ([`View::path_target_checked`], which clears a stale one); T present
    /// -> its position, else the path's target point. `Some(None)` when
    /// either coordinate is 0.
    pub(crate) fn path_target_position(
        &mut self,
        game: &Game,
        unit: UnitId,
    ) -> Option<Option<(i32, i32)>> {
        self.h.paths.as_ref()?;
        let t = self.path_target_checked(game, unit);
        let Some(d) = self.h.paths.as_ref()?.dynamic(unit) else {
            return Some(None);
        };
        let (x, y) = match t {
            Some(t) => self.h.path_position(t),
            None => (i32::from(d.target_x), i32::from(d.target_y)),
        };
        Some((x != 0 && y != 0).then_some((x, y)))
    }

    /// Set the path type on a missile's path.
    pub(crate) fn path_set_type(&mut self, unit: UnitId, ty: i32) -> Option<()> {
        let p = self.h.paths.as_mut()?;
        let tables = p.tables.clone();
        if let Some(d) = p.dynamic_mut(unit) {
            if let Err(e) = d.set_path_type(&tables, false, ty as u32) {
                self.h.errors.push(WiringError::Path(e));
            }
        }
        Some(())
    }

    /// Set the step counts ([`set_step_counts`]).
    pub(crate) fn path_set_step_counts(&mut self, unit: UnitId, n: i32) -> Option<()> {
        let p = self.h.paths.as_mut()?;
        if let Some(d) = p.dynamic_mut(unit) {
            set_step_counts(d, n);
        }
        Some(())
    }

    /// The direction (0..63) from the unit's position to (x, y), from the
    /// direction vector.
    ///
    /// TODO: it is not known whether the game feeds the direction vector
    /// sub-tile positions or the path's 16.16 ones (and the target's
    /// fraction); sub-tiles are used (equal to 16.16 whenever both
    /// fractions match). The direction vector's extra rule for its other
    /// caller is not applied.
    pub(crate) fn path_dir64(&self, unit: UnitId, at: (i32, i32)) -> Option<i32> {
        let p = self.h.paths.as_ref()?;
        let (x, y) = self.h.path_position(unit);
        let (_, d) = crate::path::walk::geom::direction_vector(
            &p.tables,
            (x as u32, y as u32),
            (at.0 as u32, at.1 as u32),
        );
        Some(i32::from(d))
    }

    /// Snap the direction: direction := new direction := d & 63. `None`
    /// without the path provider or a dynamic record.
    pub(crate) fn path_snap_direction(&mut self, unit: UnitId, d: i32) -> Option<()> {
        let p = self.h.paths.as_mut()?;
        let r = p.dynamic_mut(unit)?;
        r.direction = (d & 63) as u8;
        r.new_direction = r.direction;
        Some(())
    }

    /// Path point i := (x, y); an index outside the array is not written.
    pub(crate) fn path_set_point(&mut self, unit: UnitId, i: i32, at: (u16, u16)) -> Option<()> {
        let p = self.h.paths.as_mut()?;
        if let Some(d) = p.dynamic_mut(unit) {
            if let Some(pt) = usize::try_from(i).ok().and_then(|i| d.points.get_mut(i)) {
                pt.x = at.0;
                pt.y = at.1;
            }
        }
        Some(())
    }

    /// Point count := n.
    pub(crate) fn path_set_point_count(&mut self, unit: UnitId, n: i32) -> Option<()> {
        let p = self.h.paths.as_mut()?;
        if let Some(d) = p.dynamic_mut(unit) {
            d.point_count = n as u32;
        }
        Some(())
    }

    /// The teleport, then point count := 0.
    pub(crate) fn path_teleport_to(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        room: Option<RoomId>,
        x: i32,
        y: i32,
    ) -> Option<()> {
        self.h.paths.as_ref()?;
        let mut c = PathCtx::of(self, game);
        c.teleport(unit, room, x, y);
        if let Some(mut d) = c.load_path(unit) {
            d.point_count = 0;
            c.store_path(unit, &d);
        }
        Some(())
    }

    /// The tail of the pet warp: when the unit's room changed, path flag
    /// 0x2 (`ROOM_CHANGED`) is set and the room-change messages run with
    /// the old room as the previous one, so a client whose rooms held the
    /// unit only before the move gets its removal at once.
    pub(crate) fn path_pet_room_change(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        old: Option<RoomId>,
    ) {
        if self.h.paths.is_none() {
            return;
        }
        let mut c = PathCtx::of(self, game);
        if let Some(mut d) = c.load_path(unit) {
            d.flags |= crate::path::record::flags::ROOM_CHANGED;
            d.prev_room = old;
            c.store_path(unit, &d);
        }
        c.room_change_messages(unit);
    }
}

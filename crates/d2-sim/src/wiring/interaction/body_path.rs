//! The path operations the skill bodies make on a unit's path
//! ([`PathOp`]), on the path provider of the action wiring: velocity,
//! target point and unit, path type, step counts, the previous-target
//! clear, the two masks, the compute toward the target and the facing
//! toward a point. Charge, Blessed Hammer and Bone Prison use these; the
//! other operations (`Reset`, ...) stay with the [`Pending`] seam's
//! default.

use crate::skills::use_::bodies::PathOp;
use crate::units::UnitId;
use crate::wiring::action::{Pending, View};
use crate::wiring::path::walk::build_path;

/// Runs `op` on `unit`'s path; `None` for an operation this module does
/// not carry (the caller falls back to the seam).
pub fn path_op<X: Pending>(
    v: &mut View<'_, X>,
    game: &mut crate::game::Game,
    unit: UnitId,
    op: PathOp<UnitId>,
) -> Option<i32> {
    match op {
        PathOp::Velocity(n) => v.h.path_set_velocity(unit, n),
        PathOp::TargetPoint(x, y) => v.h.path_set_target_point(unit, x, y),
        PathOp::TargetUnit(Some(t)) => v.path_set_target_unit(unit, t),
        PathOp::Type(t) => {
            v.path_set_type(unit, i32::from(t))?;
        }
        PathOp::Steps(n) => {
            v.path_set_step_counts(unit, n)?;
        }
        // Sets the previous target to (0, 0), so the arrival check's 5-cell
        // test re-paths at the next step.
        PathOp::Clear14 => {
            let p = v.h.paths.as_mut()?;
            if let Some(d) = p.dynamic_mut(unit) {
                d.put_prev_target(crate::path::Point::new(0, 0));
            }
        }
        PathOp::MoveMask(m) => v.h.path_set_move_mask(unit, m as u16),
        PathOp::FootprintMask(m) => v.path_set_foot_mask(unit, m as u16),
        PathOp::Compute => {
            build_path(v, game, unit);
            return Some(1);
        }
        // Takes the direction from the unit's position to (x, y) and snaps
        // it onto the path (each Bone Prison segment faces the target).
        PathOp::Face(x, y) => {
            let d = v.path_dir64(unit, (x, y))?;
            v.path_snap_direction(unit, d)?;
        }
        _ => return None,
    }
    Some(0)
}

/// A skill mode started at a point: sets the path's target point (target
/// unit := none). Every skill reads that point back as its target position
/// (the path's target point when the target unit is none), and the skill
/// message 0x4D sends it as target x, y. Charge's run (mode 3) computes its
/// path to it. A unit target sets the path's target unit instead.
pub fn point_target<X: Pending>(
    v: &mut View<'_, X>,
    unit: UnitId,
    target: crate::skills::use_::ModeTarget<UnitId>,
) {
    use crate::skills::use_::ModeTarget;
    match target {
        ModeTarget::Point(x, y) => v.h.path_set_target_point(unit, x, y),
        // The unit-form start sets the path's target unit to `t`; the
        // moving skills' path (Whirlwind's type 7) is computed toward it.
        ModeTarget::Unit(t) => v.path_set_target_unit(unit, t),
    }
}

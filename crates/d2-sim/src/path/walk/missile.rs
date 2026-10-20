//! Missile paths: path compute step 1 hands a path with flag 0x40000 here, by path
//! type: 4 straight missile, 10 charged bolt
//! (draws on the unit seed), 14 blessed hammer
//! (the float32 sine table as integers, [`super::sine`]).

use super::geom::{octant, step};
use super::seams::{room_contains, PathWorld, Point, WalkError, WalkUnits};
use super::sine::trunc_mul;
use super::velocity::aim;
use crate::path::record::{flags, path_types, DynamicPath, PathPoint, PATH_POINTS};
use crate::path::tables::PathTables;
use crate::units::UnitId;

/// Path type 10, charged bolt.
pub const CHARGED_BOLT: u32 = 10;
/// Path type 14, blessed hammer.
pub const BLESSED_HAMMER: u32 = 14;
/// Range per axis of the straight missile.
pub const MISSILE_RANGE: i32 = 100;
/// Charged-bolt offsets: (−1, 0, +1) ten times, −1, +1.
pub const BOLT_OFFSETS: [i32; 32] = {
    let mut t = [0i32; 32];
    let mut i = 0;
    while i < 30 {
        t[i] = (i % 3) as i32 - 1;
        i += 1;
    }
    t[30] = -1;
    t[31] = 1;
    t
};
/// Blessed-hammer radius step per point (0x2580).
pub const HAMMER_RADIUS_STEP: i64 = 0x2580;
/// Blessed-hammer angle step per point.
pub const HAMMER_ANGLE_STEP: u32 = 16;
/// Blessed-hammer point count.
pub const HAMMER_POINTS: usize = 77;

/// Missile path: by path type; any other type is
/// a fatal assert. A non-zero result sets flag 0x20; type 4 with result
/// 0 clears it. Returns the count.
pub fn missile_path<C: PathWorld + WalkUnits + ?Sized>(
    t: &PathTables,
    c: &mut C,
    path: &mut DynamicPath,
    unit: UnitId,
) -> Result<i32, WalkError> {
    let ty = path.path_type;
    let n = match ty {
        path_types::MISSILE => straight_missile(t, c, path, unit),
        CHARGED_BOLT => charged_bolt(t, c, path, unit)?,
        BLESSED_HAMMER => blessed_hammer(path)?,
        _ => return Err(WalkError::Fatal("missile path type")),
    };
    if n != 0 {
        path.flags |= flags::ACTIVE;
    } else if ty == path_types::MISSILE {
        path.flags &= !flags::ACTIVE;
    }
    Ok(n)
}

/// Straight missile.
fn straight_missile<C: PathWorld + WalkUnits + ?Sized>(
    t: &PathTables,
    c: &mut C,
    path: &mut DynamicPath,
    unit: UnitId,
) -> i32 {
    // Rule 1.
    path.cur_point = 0;
    if let Some(tu) = path.target_unit {
        path.put_target(c.position(tu.unit));
    }
    // Rule 2.
    let (cell, target) = (path.cell(), path.target());
    if (target.x - cell.x).abs() >= MISSILE_RANGE
        || (target.y - cell.y).abs() >= MISSILE_RANGE
        || target.x == 0
        || target.y == 0
    {
        return 0;
    }
    // Rule 3.
    path.points[0] = PathPoint::from_point(target);
    path.point_count = 1;
    path.field_38 = 0;
    aim(t, path, c.unit_type(unit));
    // Rule 4: no path room, or a target outside it, sets the flag; it is
    // never cleared here.
    if path
        .room
        .is_none_or(|room| !room_contains(&*c, room, target))
    {
        path.flags |= flags::OUTSIDE_ROOM;
    }
    1
}

/// Charged bolt: n = max distance >> 1 draws on the
/// unit seed.
fn charged_bolt<C: PathWorld + WalkUnits + ?Sized>(
    t: &PathTables,
    c: &mut C,
    path: &mut DynamicPath,
    unit: UnitId,
) -> Result<i32, WalkError> {
    // Rule 1.
    let n = usize::from(path.max_distance >> 1);
    if n >= PATH_POINTS {
        // The generated path needs n points plus its endpoint; all must fit
        // in the 78 point slots.
        return Err(WalkError::Fatal(
            "charged-bolt path longer than the point slots",
        ));
    }
    let start = path.cell();
    let d0 = t.testdir[octant(start, path.target())][0];
    // Rule 3.
    let mut cur = start;
    for k in 0..n {
        let lo = c.seed(unit).step();
        let d = (BOLT_OFFSETS[(lo & 31) as usize] + d0) & 7;
        path.points[k] = PathPoint::from_point(cur);
        let s = step(t, d as u8);
        cur = Point::new(cur.x + 2 * s.x, cur.y + 2 * s.y);
    }
    // Rule 4.
    path.points[n] = PathPoint::from_point(cur);
    path.point_count = (n + 1) as u32;
    Ok((n + 1) as i32)
}

/// Blessed hammer: a spiral of 77 distinct cells
/// from the precise position; no draw.
fn blessed_hammer(path: &mut DynamicPath) -> Result<i32, WalkError> {
    let cells = hammer_cells((path.precise_x, path.precise_y))?;
    for (slot, &(x, y)) in path.points.iter_mut().zip(cells.iter()) {
        *slot = PathPoint {
            x: x as u16,
            y: y as u16,
        };
    }
    // Rule 3.
    path.point_count = HAMMER_POINTS as u32;
    Ok(HAMMER_POINTS as i32)
}

/// The 77 cells of the blessed-hammer spiral from the
/// precise start `s` (16.16); shared with the client's path of type 14.
pub fn hammer_cells(s: (u32, u32)) -> Result<[(u32, u32); HAMMER_POINTS], WalkError> {
    // Rule 1.
    let (sx, sy) = s;
    let mut prev = (sx, sy);
    let mut out = [(0, 0); HAMMER_POINTS];
    let mut count = 0usize;
    let mut k: i64 = 0;
    // Rule 2.
    while count < HAMMER_POINTS {
        k += 1;
        let r = HAMMER_RADIUS_STEP * k;
        if r >= 1 << 24 {
            // r is no longer exact in float32 (unreachable: 77 cells are
            // found within a few hundred steps).
            return Err(WalkError::Fatal("blessed-hammer spiral does not end"));
        }
        let a = (HAMMER_ANGLE_STEP as i64 * k) as u32;
        let x = sx.wrapping_add(trunc_mul(a.wrapping_add(128), r) as u32);
        let y = sy.wrapping_add(trunc_mul(a, r) as u32);
        let cell = (x >> 16, y >> 16);
        if cell != (prev.0 >> 16, prev.1 >> 16) {
            out[count] = cell;
            count += 1;
            prev = (x, y);
        }
    }
    Ok(out)
}

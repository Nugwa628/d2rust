//! Per-tick movement: the player event-0 step, the step, movement, arrival
//! check, one step, cell walk, footprint move, set position, room recache,
//! reset, room-change messages, run drain and re-path. 16.16 fixed point
//! on u32 positions (wrapping where it wraps).

use super::find::compute;
use super::geom::unit_distance;
use super::request::{neutral_start, start_movement};
use super::seams::{count, index, room_contains, PathWorld, Point, WalkError, WalkUnits};
use super::velocity::aim;
use crate::path::collision::find_room;
use crate::path::coords::to_fp16_center;
use crate::path::footprint::{forced_move, missile_move, try_move};
use crate::path::record::{flags, path_types, DynamicPath, PathPoint, PATH_POINTS, SAVED_STEPS};
use crate::path::tables::PathTables;
use crate::units::{RoomId, UnitId, UnitType};

/// Step base, 100 %.
pub const STEP_BASE: i32 = 0x400;
/// Base ticks between two speed changes of an accelerating path.
pub const ACCEL_INTERVAL: i32 = 5;
/// Stamina stat (8.8 fixed point).
pub const STAT_STAMINA: u16 = 10;
/// `item_staminadrainpct`.
pub const STAT_STAMINADRAIN: u16 = 154;
/// Body location of the torso item.
pub const BODYLOC_TORSO: u8 = 3;
/// Bound on the cell walk's loop: no rule; a step of at most 0x10000 per
/// iteration reaches any reachable cell far sooner. Exceeding it is
/// reported as fatal instead of hanging.
const CELL_WALK_GUARD: u32 = 1 << 20;
/// The longest run (16.16, along the step's axis) a straight line may make
/// inside a refused cell's corner before it crosses into the next row or
/// column, and still slide past it ([`Walk::graze_slide`]): an eighth of a
/// sub-tile.
const GRAZE: i64 = 0x2000;

/// Result of the step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// 1: still moving.
    Moving = 1,
    /// 2: stopped.
    Stopped = 2,
}

/// The walk context: tables and the one object that holds the game,
/// units, DRLG grids and path store ([`PathWorld`] + [`WalkUnits`]).
pub struct Walk<'a, C: ?Sized> {
    pub t: &'a PathTables,
    pub c: &'a mut C,
}

fn cell_of(p: u32) -> i32 {
    (p >> 16) as i32
}

fn centre_of(path: &DynamicPath) -> (u32, u32) {
    let c = path.cell();
    (to_fp16_center(c.x), to_fp16_center(c.y))
}

impl<C: PathWorld + WalkUnits + ?Sized> Walk<'_, C> {
    /// Player event 0. Returns the step result.
    pub fn player_event0(&mut self, unit: UnitId) -> Result<Step, WalkError> {
        let Some(mut path) = self.c.load_path(unit) else {
            return Ok(Step::Stopped);
        };
        let r = self.player_event0_on(unit, &mut path);
        self.c.store_path(unit, &path);
        let s = r?;
        if s == Step::Stopped {
            // Step 6: no server code stores a non-zero queued action
            // (player data +0x150), so a stop is a neutral start.
            neutral_start(self.c, unit, &path);
        }
        Ok(s)
    }

    fn player_event0_on(
        &mut self,
        unit: UnitId,
        path: &mut DynamicPath,
    ) -> Result<Step, WalkError> {
        // Step 1.
        self.target_check(path);
        // Step 2: the skills code's hook; its result is ignored and
        // the step goes on.
        if self.c.has_state(unit, 13) {
            self.c.state13_step(unit);
        }
        // Step 3: exhausted → restart with mode 2 (walk, or town walk in
        // a town), path recomputed.
        if self.c.mode(unit) == 3
            && self.run_drain(unit, path)
            && start_movement(self.t, self.c, unit, path, 2)? == 0
        {
            neutral_start(self.c, unit, path);
        }
        // Step 4.
        let s = self.step(unit, path)?;
        // Step 5: position history (the placement history rule; the
        // 25 ms wall-clock gate reads as open).
        let cell = path.cell();
        if let Some(h) = self.c.position_history(unit) {
            h.walk_write(cell.x, cell.y);
        }
        Ok(s)
    }

    /// Target check.
    pub fn target_check(&mut self, path: &mut DynamicPath) {
        let Some(tu) = path.target_unit else { return };
        let found = self.c.find_unit(tu.ty, tu.guid);
        let drop = match found {
            Some(f) if f == tu.unit => {
                self.c.unit_type(f) == UnitType::Item && matches!(self.c.mode(f), 1 | 2)
            }
            _ => true,
        };
        if drop {
            path.target_unit = None;
        }
    }

    /// Step.
    pub fn step(&mut self, unit: UnitId, path: &mut DynamicPath) -> Result<Step, WalkError> {
        self.target_check(path);
        let m = self.movement(unit, path, STEP_BASE)?;
        if path.flags & flags::ROOM_CHANGED != 0 {
            self.room_change_messages(unit, path);
        }
        Ok(if m { Step::Moving } else { Step::Stopped })
    }

    /// Movement. True = result 1.
    pub fn movement(
        &mut self,
        unit: UnitId,
        path: &mut DynamicPath,
        base: i32,
    ) -> Result<bool, WalkError> {
        let ty = self.c.unit_type(unit);
        let missile = ty == UnitType::Missile;
        // Rule 1.
        path.flags &= !flags::MOVED;
        if missile {
            path.collided_mask = 0;
        }
        // Rule 2.
        let go = path.flags & flags::ACTIVE != 0
            && count(path) > 0
            && path.velocity != 0
            && (missile || self.arrival(unit, path)?)
            && index(path) < count(path);
        if go {
            // Velocities and the acceleration interval are stated at
            // the base tick rate; the tick rate turns them into this
            // tick's share (identity at 25 Hz).
            let (rate, frame) = (self.c.tick_rate(), self.c.frame());
            let base = if base <= 0 { STEP_BASE } else { base };
            if path.acceleration != 0 {
                path.accel_counter += 1;
                // Speed changes every 5 base ticks (200 ms).
                if path.accel_counter >= rate.ticks(ACCEL_INTERVAL) {
                    path.velocity = (path.velocity + path.acceleration).clamp(0, path.max_velocity);
                    if path.velocity == path.max_velocity {
                        path.acceleration = 0;
                    }
                    path.accel_counter = 0;
                }
            }
            let m = rate.share(base.wrapping_mul(path.velocity) >> 6, frame);
            path.vel_vec_x = m.wrapping_mul(path.dir_vec_x) >> 12;
            path.vel_vec_y = m.wrapping_mul(path.dir_vec_y) >> 12;
            // 2.2.
            if (path.vel_vec_x, path.vel_vec_y) != (0, 0) {
                // 2.3, 2.4.
                let q = self.one_step(unit, path)?;
                self.set_position(unit, path, q, None);
                // 2.5: path type ≠ 4.
                if path.path_type != path_types::MISSILE && index(path) < count(path) {
                    aim(self.t, path, ty);
                }
                // 2.6.
                if index(path) < count(path) {
                    return Ok(true);
                }
            }
        }
        // Rule 3.
        self.reset(unit, path);
        Ok(false)
    }

    /// Arrival check. The re-path's result decides
    /// where the rule says "re-path".
    fn arrival(&mut self, unit: UnitId, path: &mut DynamicPath) -> Result<bool, WalkError> {
        let pos = path.cell();
        if matches!(path.path_type, 5 | 6) && index(path) >= count(path) {
            return Ok(true);
        }
        let Some(tu) = path.target_unit else {
            if index(path) >= count(path) && pos != path.final_target() {
                return Ok(self.repath(unit, path, false)? != 0);
            }
            return Ok(true);
        };
        let tpos = self.c.position(tu.unit);
        let d = unit_distance(self.t, pos, path.unit_size, tpos, self.c.unit_size(tu.unit));
        if d <= path.stop_distance as i32 {
            return Ok(false);
        }
        let refreshed = super::find::refresh_point(&*self.c, path, tu.unit);
        let tty = self.c.unit_type(tu.unit);
        let prev = path.prev_target();
        if matches!(tty, UnitType::Player | UnitType::Monster)
            && ((refreshed.x - prev.x).abs() > 5 || (refreshed.y - prev.y).abs() > 5)
        {
            return Ok(self.repath(unit, path, true)? != 0);
        }
        if index(path) < count(path) || pos == path.final_target() {
            return Ok(true);
        }
        Ok(self.repath(unit, path, true)? != 0)
    }

    /// One step: the new precise position Q.
    fn one_step(&mut self, unit: UnitId, path: &mut DynamicPath) -> Result<(u32, u32), WalkError> {
        let ty = self.c.unit_type(unit);
        // Rule 1.
        path.collided_mask = 0;
        let monster_repath = ty == UnitType::Monster && path.flags & flags::KEEP_TARGET != 0;
        // Rule 2 (dead code: the only caller, arrival rule 2.2, resets
        // before calling with a (0, 0) vector).
        if (path.vel_vec_x, path.vel_vec_y) == (0, 0) {
            path.point_count = 0;
            path.cur_point = 0;
            return Ok(centre_of(path));
        }
        // Rule 3 (snap): the snap to the
        // current point tests only the path type, so missiles snap like
        // players and monsters on every type but 4 (footprint move rule 3;
        // recorded `dru-tornado` frames 30–31).
        let (mut dx, mut dy) = (path.vel_vec_x, path.vel_vec_y);
        let mut reaches = false;
        let i = index(path).clamp(0, PATH_POINTS as i32 - 1) as usize;
        let p = path.point(i);
        let rx = to_fp16_center(p.x).wrapping_sub(path.precise_x) as i32;
        let ry = to_fp16_center(p.y).wrapping_sub(path.precise_y) as i32;
        let m = dx.abs().max(dy.abs());
        // Path type 4 (every missile aimed at a point, missile flight
        // step 5) never snaps: it keeps its direction past the target until
        // its frames run out or it hits (check `diff-a4-nm-unique`, the
        // Gloam's bolt, class 320: flies on past the aim point).
        let snaps = !(ty == UnitType::Missile && path.path_type == path_types::MISSILE);
        if snaps && rx.abs() <= m && ry.abs() <= m {
            dx = rx;
            dy = ry;
            reaches = true;
        }
        // Rule 4.
        let nx = path.precise_x.wrapping_add_signed(dx);
        let ny = path.precise_y.wrapping_add_signed(dy);
        let mut q = (nx, ny);
        if (cell_of(nx), cell_of(ny)) != (cell_of(path.precise_x), cell_of(path.precise_y)) {
            if path.dist_budget > 0
                && path.path_type != path_types::KNOCKBACK_SERVER
                && path.path_type != path_types::KNOCKBACK_CLIENT
            {
                path.dist_budget -= 1;
            }
            match self.cell_walk(unit, path, (dx, dy))? {
                Ok(end) => q = end,
                Err(last_free) => {
                    if monster_repath {
                        let finish = path.target_unit.is_some();
                        self.repath(unit, path, finish)?;
                    } else {
                        path.cur_point = path.point_count;
                    }
                    return Ok(last_free);
                }
            }
        }
        // Rule 7.
        // A corner graze slide ends the step short of the point.
        if reaches && q == (nx, ny) {
            path.cur_point += 1;
        }
        Ok(q)
    }

    /// Cell walk. `Ok(Q)` or `Err(centre of the
    /// last free cell)`.
    #[allow(clippy::type_complexity)]
    pub(crate) fn cell_walk(
        &mut self,
        unit: UnitId,
        path: &mut DynamicPath,
        d: (i32, i32),
    ) -> Result<Result<(u32, u32), (u32, u32)>, WalkError> {
        let mut k = 0usize;
        path.saved_count = 0;
        let (mut sx, mut sy) = d;
        while sx.abs() > 0x10000 || sy.abs() > 0x10000 {
            sx >>= 1;
            sy >>= 1;
        }
        let end = (
            path.precise_x.wrapping_add_signed(d.0),
            path.precise_y.wrapping_add_signed(d.1),
        );
        let end_cell = (cell_of(end.0), cell_of(end.1));
        let mut c = (path.precise_x, path.precise_y);
        let mut guard = 0u32;
        while (cell_of(c.0), cell_of(c.1)) != end_cell {
            guard += 1;
            if guard > CELL_WALK_GUARD {
                return Err(WalkError::Fatal("cell walk does not reach its cell"));
            }
            let n = (c.0.wrapping_add_signed(sx), c.1.wrapping_add_signed(sy));
            let (cc, nc) = (
                Point::new(cell_of(c.0), cell_of(c.1)),
                Point::new(cell_of(n.0), cell_of(n.1)),
            );
            if cc != nc {
                if !self.footprint_move(unit, path, cc, nc)? {
                    if let Some(slid) = self.graze_slide(unit, path, c, d, cc, nc)? {
                        // The slide crossed into the diagonal cell: it is a
                        // saved step like any accepted footprint move, and the
                        // unit moved even when it was this tick's first cell.
                        if path.flags & flags::SAVE_STEPS != 0 && k < SAVED_STEPS {
                            path.saved_steps[k] =
                                PathPoint::from_point(Point::new(cell_of(slid.0), cell_of(slid.1)));
                            k += 1;
                        }
                        path.saved_count = k as u32;
                        path.flags |= flags::MOVED;
                        return Ok(Ok(slid));
                    }
                    path.saved_count = k as u32;
                    if k > 0 {
                        path.flags |= flags::MOVED;
                    }
                    return Ok(Err((to_fp16_center(cc.x), to_fp16_center(cc.y))));
                }
                if path.flags & flags::SAVE_STEPS != 0 {
                    path.saved_steps[k] = PathPoint::from_point(nc);
                    k += 1;
                    if k >= SAVED_STEPS {
                        break;
                    }
                }
            }
            c = n;
        }
        path.saved_count = k as u32;
        if k > 0 {
            path.flags |= flags::MOVED;
        }
        Ok(Ok(end))
    }

    /// The corner graze slide of a refused cell step. A straight line that
    /// only clips the corner of the refused cell `nc` (it runs less than
    /// [`GRAZE`] inside it along the step's axis before it crosses into the
    /// next row or column it is heading for) goes on in the diagonal cell
    /// past that corner instead, when the footprint fits there: the unit is
    /// placed where the line enters that cell. The test reads the line, not
    /// where a tick's step happens to end, so every tick rate walks the same
    /// cells. `None`: no graze, or the diagonal cell is blocked too.
    fn graze_slide(
        &mut self,
        unit: UnitId,
        path: &mut DynamicPath,
        c: (u32, u32),
        d: (i32, i32),
        cc: Point,
        nc: Point,
    ) -> Result<Option<(u32, u32)>, WalkError> {
        // One axis changes (`a`), the line also runs along the other (`b`).
        let x_step = nc.x != cc.x;
        if x_step == (nc.y != cc.y) {
            return Ok(None);
        }
        let (ca, cb, da, db) = if x_step {
            (c.0, c.1, d.0, d.1)
        } else {
            (c.1, c.0, d.1, d.0)
        };
        if db == 0 || da == 0 {
            return Ok(None);
        }
        let cell_b = cell_of(cb);
        let cell_a = cell_of(ca);
        // Where the line enters the next cell along b, and the refused cell
        // along a (16.16, inside the new cell).
        let enter = |cell: i32, up: bool| -> i64 {
            if up {
                i64::from(cell + 1) << 16
            } else {
                (i64::from(cell) << 16) - 1
            }
        };
        let to_b = (enter(cell_b, db > 0) - i64::from(cb)).abs();
        let to_a = (enter(cell_a, da > 0) - i64::from(ca)).abs();
        // The run along a until the line crosses into the next row (column).
        let run = to_b * i64::from(da.abs()) / i64::from(db.abs());
        if run - to_a >= GRAZE {
            return Ok(None);
        }
        let along = run.max(to_a);
        let pa = (i64::from(ca) + along * i64::from(da.signum())) as u32;
        let pb = enter(cell_b, db > 0) as u32;
        let (p, dc) = if x_step {
            ((pa, pb), Point::new(cell_of(pa), cell_of(pb)))
        } else {
            ((pb, pa), Point::new(cell_of(pb), cell_of(pa)))
        };
        if dc == cc || dc == nc {
            return Ok(None);
        }
        path.collided_mask = 0;
        if !self.footprint_move(unit, path, cc, dc)? {
            return Ok(None);
        }
        Ok(Some(p))
    }

    /// Footprint move. True = accepted.
    fn footprint_move(
        &mut self,
        unit: UnitId,
        path: &mut DynamicPath,
        old: Point,
        new: Point,
    ) -> Result<bool, WalkError> {
        let ty = self.c.unit_type(unit);
        let (o, n) = ((old.x, old.y), (new.x, new.y));
        if path.flags & flags::NO_TEST != 0 {
            if !matches!(ty, UnitType::Player | UnitType::Monster) {
                return Err(WalkError::Fatal("forced footprint move of a non-walker"));
            }
            forced_move(self.c, path.room, o, n, path.pattern, path.foot_mask);
            return Ok(true);
        }
        if ty == UnitType::Missile {
            let r = missile_move(
                self.c,
                path.room,
                o,
                n,
                path.unit_size,
                path.foot_mask,
                path.move_mask,
            );
            path.collided_mask |= r;
            return Ok(path.collided_mask & 0x5 == 0);
        }
        let test = if path.move_mask == 0x3401 {
            0x3C01
        } else {
            path.move_mask
        };
        let r = try_move(self.c, path.room, o, n, path.pattern, path.foot_mask, test);
        path.collided_mask |= r;
        Ok(path.collided_mask == 0)
    }

    /// Set position.
    pub fn set_position(
        &mut self,
        unit: UnitId,
        path: &mut DynamicPath,
        q: (u32, u32),
        hint: Option<RoomId>,
    ) {
        let missile = self.c.unit_type(unit) == UnitType::Missile;
        if missile && find_room(&*self.c, path.room, cell_of(q.0), cell_of(q.1)).is_none() {
            path.point_count = 0;
            return;
        }
        path.precise_x = q.0;
        path.precise_y = q.1;
        path.update_client();
        if path.flags & flags::OUTSIDE_ROOM != 0 {
            self.room_recache(unit, path, hint, missile);
        }
    }

    /// Room recache.
    fn room_recache(
        &mut self,
        unit: UnitId,
        path: &mut DynamicPath,
        hint: Option<RoomId>,
        missile: bool,
    ) {
        let c = path.cell();
        if let Some(r) = path.room {
            if room_contains(&*self.c, r, c) {
                return;
            }
        }
        let new = find_room(&*self.c, path.room, c.x, c.y)
            .or_else(|| find_room(&*self.c, hint, c.x, c.y));
        if new.is_none() && missile {
            path.point_count = 0;
            return;
        }
        path.prev_room = path.room;
        if let Some(old) = path.room {
            self.c.room_list_remove(unit, old);
        }
        path.flags |= flags::ROOM_CHANGED;
        path.room = new;
        if let Some(n) = new {
            self.c.room_list_insert(unit, n);
            self.c.queue_for_update(unit);
        }
    }

    /// Reset.
    pub fn reset(&mut self, unit: UnitId, path: &mut DynamicPath) {
        let q = centre_of(path);
        self.set_position(unit, path, q, None);
        path.flags &= !flags::ACTIVE;
        path.point_count = 0;
        path.cur_point = 0;
        path.vel_vec_x = 0;
        path.vel_vec_y = 0;
    }

    /// Room-change messages.
    pub fn room_change_messages(&mut self, unit: UnitId, path: &mut DynamicPath) {
        if path.flags & flags::ROOM_CHANGED == 0 {
            return;
        }
        if self.c.unit_type(unit) == UnitType::Monster {
            self.c.clear_ai_room_memo(unit);
        }
        // The previous room: path +0x20 while it is still a
        // room of the unit's act, else none.
        // Publish the working path before sending add messages so their
        // position lookup sees this step's position.
        self.c.store_path(unit, path);
        let old = path
            .prev_room
            .filter(|&r| self.c.room_in_unit_act(unit, r))
            .map(|r| self.c.room_clients(r))
            .unwrap_or_default();
        let new = path
            .room
            .map(|r| self.c.room_clients(r))
            .unwrap_or_default();
        let (mut i, mut j) = (0, 0);
        while i < old.len() || j < new.len() {
            let a = old.get(i).copied();
            let b = new.get(j).copied();
            match (a, b) {
                (Some(x), Some(y)) if x == y => {
                    i += 1;
                    j += 1;
                }
                (Some(x), y) if y.is_none_or(|y| x < y) => {
                    if self.c.client_player(x) != Some(unit) {
                        self.c.send_unit_removal(x, unit);
                    }
                    i += 1;
                }
                (_, Some(y)) => {
                    if self.c.client_player(y) != Some(unit) {
                        self.c.send_unit_add(y, unit);
                    }
                    j += 1;
                }
                _ => break,
            }
        }
        path.flags &= !flags::ROOM_CHANGED;
    }

    /// Run drain. True = exhausted.
    pub fn run_drain(&mut self, unit: UnitId, path: &DynamicPath) -> bool {
        if path.room.is_some_and(|r| self.c.room_in_town(r)) {
            return false;
        }
        let mut d = 2 * self.c.charstats_velocity(unit).2;
        if let Some(speed) = self.c.torso_speed(unit) {
            d *= speed / 10 + 1;
        }
        let s = self.c.item_stat(unit, STAT_STAMINADRAIN);
        d -= s.wrapping_mul(d) / 100;
        // At least 1 per base tick; a faster tick rate drains its share
        // of that each tick, the same amount a second.
        if d < 1 {
            d = 1;
        }
        d = self.c.tick_rate().share(d, self.c.frame());
        self.c.add_base_stat(unit, STAT_STAMINA, -d);
        if self.c.stat(unit, STAT_STAMINA) > 0 {
            return false;
        }
        self.c.set_base_stat(unit, STAT_STAMINA, 0);
        true
    }

    /// Re-path. Unless flag 0x10: a
    /// monster whose re-path budget (path +0x94) is 0 → 0
    /// (players skip the test); else queue for update, unit flag 1, and
    /// budget −= index, clamped to 0..255. Types are written
    /// to +0x3C directly (flags and direction offset unchanged); every
    /// compute passes town access 0.
    pub fn repath(
        &mut self,
        unit: UnitId,
        path: &mut DynamicPath,
        finish: bool,
    ) -> Result<i32, WalkError> {
        let ty = self.c.unit_type(unit);
        if path.flags & flags::KEEP_TARGET == 0 {
            if ty == UnitType::Monster && path.repath_budget == 0 {
                return Ok(0);
            }
            self.c.queue_for_update(unit);
            self.c.set_unit_flag(unit, 1);
            path.add_repath_budget((path.cur_point as i32).wrapping_neg());
        }
        let town = false;
        if matches!(
            path.path_type,
            path_types::TOWARD | path_types::TOWARD_FINISH | path_types::WALL_FOLLOW
        ) {
            if finish {
                path.path_type = path_types::TOWARD_FINISH;
            } else {
                path.path_type = path_types::TOWARD;
                path.put_target(path.final_target());
            }
            let r = compute(self.t, self.c, path, unit, town)?;
            if r != 0 {
                return Ok(r);
            }
            path.path_type = path_types::WALL_FOLLOW;
            return compute(self.t, self.c, path, unit, town);
        }
        compute(self.t, self.c, path, unit, town)
    }
}

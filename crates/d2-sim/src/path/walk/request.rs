//! C→S 0x01–0x04 after transport validation (the client-message
//! rules own the parsing and range checks): the player mode request,
//! the mode and interrupt checks, and the movement start.

use super::find::{compute, reset_type};
use super::seams::{count, PathWorld, Point, StartTarget, TargetUnit, WalkError, WalkUnits};
use super::step::STAT_STAMINA;
use super::velocity::{mode_velocity, run_velocity_bonus, set_velocity};
use crate::path::record::{path_types, DynamicPath};
use crate::path::tables::PathTables;
use crate::units::{UnitId, UnitType};

/// Player modes used here.
pub mod mode {
    pub const DEATH: u32 = 0;
    pub const NEUTRAL: u32 = 1;
    pub const WALK: u32 = 2;
    pub const RUN: u32 = 3;
    pub const TOWN_NEUTRAL: u32 = 5;
    pub const TOWN_WALK: u32 = 6;
    pub const DEAD: u32 = 17;
    pub const KNOCKBACK: u32 = 19;
}

/// States read here.
pub mod state {
    pub const MOVE_STUCK_54: u16 = 54;
    pub const CONCENTRATION: u16 = 42;
    pub const STATE_15: u16 = 15;
}

/// Stat of the concentration state list read by mode check rule 5.
pub const STAT_164: u16 = 164;

/// The "early" window of the mode and interrupt checks: a request counts
/// as early up to this many base ticks (200 ms) past the unit's first
/// type-1 timer. Converted through the game's tick rate.
pub const EARLY_WINDOW: i32 = 5;

/// The request target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkTarget {
    /// Point form.
    Point(Point),
    /// Unit form: type and GUID from the message.
    Unit { ty: UnitType, guid: u32 },
}

/// What a request did (for callers and tests; the dispatcher result is
/// 0 in every case).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Unit form: no unit of that type and GUID (a log line).
    NoTargetUnit,
    /// Mode check refused.
    ModeRefused,
    /// Interrupt check refused.
    InterruptRefused,
    /// Knockback on a unit already in mode 19: nothing.
    KnockbackIgnored,
    /// The mode's start function is not a walk mode (seam).
    OtherMode,
    /// Path empty: neutral start.
    Neutral,
    /// Mode set with a path of this many points.
    Moving(i32),
}

/// C→S message ids 0x01–0x04 → (mode, unit form).
pub fn message_request(id: u8) -> Option<(u32, bool)> {
    match id {
        0x01 => Some((mode::WALK, false)),
        0x02 => Some((mode::WALK, true)),
        0x03 => Some((mode::RUN, false)),
        0x04 => Some((mode::RUN, true)),
        _ => None,
    }
}

/// Handler of C→S 0x01–0x04 after transport validation accepts
/// it: `a`, `b` are (x, y) for the point forms and (type, GUID) for the
/// unit forms. Returns the dispatcher result (always 0) and the outcome.
pub fn handle_message<C: PathWorld + WalkUnits + ?Sized>(
    t: &PathTables,
    c: &mut C,
    player: UnitId,
    id: u8,
    a: u32,
    b: u32,
) -> Result<(u32, Option<Outcome>), WalkError> {
    let Some((m, unit_form)) = message_request(id) else {
        return Ok((0, None));
    };
    let target = if unit_form {
        let Some(ty) = UnitType::ALL.get(a as usize).copied() else {
            return Ok((0, Some(Outcome::NoTargetUnit)));
        };
        WalkTarget::Unit { ty, guid: b }
    } else {
        WalkTarget::Point(Point::new(a as i32, b as i32))
    };
    let o = request(t, c, player, None, m, target, false)?;
    Ok((0, Some(o)))
}

/// Player mode request.
#[allow(clippy::too_many_arguments)]
pub fn request<C: PathWorld + WalkUnits + ?Sized>(
    t: &PathTables,
    c: &mut C,
    unit: UnitId,
    skill: Option<u16>,
    m: u32,
    target: WalkTarget,
    reentry: bool,
) -> Result<Outcome, WalkError> {
    // Step 1.
    let target = match target {
        WalkTarget::Point(p) => StartTarget::Point(p),
        WalkTarget::Unit { ty, guid } => match c.find_unit(ty, guid) {
            Some(t) => StartTarget::Unit(t),
            None => return Ok(Outcome::NoTargetUnit),
        },
    };
    // Step 2.
    if !mode_check(&*c, unit, m) {
        return Ok(Outcome::ModeRefused);
    }
    // Step 3.
    if !reentry && !interrupt_check(t, c, unit, m, skill)? {
        return Ok(Outcome::InterruptRefused);
    }
    // Step 4.
    if matches!(target, StartTarget::Unit(_)) && c.unit_type(unit) == UnitType::Player {
        c.clear_queued_action(unit);
    }
    // Step 5.
    if let Some(s) = skill {
        c.set_used_skill(unit, Some(s));
    }
    if !matches!(
        m,
        mode::WALK | mode::RUN | mode::TOWN_WALK | mode::KNOCKBACK
    ) {
        c.start_other_mode(unit, m, target);
        return Ok(Outcome::OtherMode);
    }
    let Some(mut path) = c.load_path(unit) else {
        return Err(WalkError::Fatal("walk request on a unit without a path"));
    };
    match target {
        StartTarget::Point(p) => {
            path.set_target_point(p.x as u16, p.y as u16);
        }
        StartTarget::Unit(tu) => {
            if m == mode::KNOCKBACK && c.mode(unit) == mode::KNOCKBACK {
                return Ok(Outcome::KnockbackIgnored);
            }
            path.target_unit = Some(TargetUnit {
                unit: tu,
                ty: c.unit_type(tu),
                guid: c.guid(tu),
            });
        }
    }
    let r = start_movement(t, c, unit, &mut path, m);
    c.store_path(unit, &path);
    let n = r?;
    if n == 0 {
        neutral_start(c, unit, &path);
        Ok(Outcome::Neutral)
    } else {
        Ok(Outcome::Moving(n))
    }
}

/// Mode check.
pub fn mode_check<U: WalkUnits + ?Sized>(u: &U, unit: UnitId, m: u32) -> bool {
    if u.has_cursor_item(unit) && m != 0 {
        return m == 17;
    }
    if matches!(m, 0 | 1 | 5 | 17) {
        return true;
    }
    let frame = u.frame();
    let e = u.first_type1_expire(unit);
    let early = frame <= e + u.tick_rate().ticks(EARLY_WINDOW);
    match u.mode(unit) {
        0 | 4 | 9 | 17 => false,
        7 | 8 | 10 | 11 => early || m == 4 || m == 9,
        12 => early,
        13 => u.class(unit) != 0,
        15 => u.class(unit) != 5,
        18 => match u.used_skill(unit) {
            Some(s) if s.seq_input > 0 => true,
            _ => early,
        },
        _ => true,
    }
}

/// Interrupt check.
pub fn interrupt_check<C: PathWorld + WalkUnits + ?Sized>(
    t: &PathTables,
    c: &mut C,
    unit: UnitId,
    m: u32,
    skill: Option<u16>,
) -> Result<bool, WalkError> {
    let cur = c.mode(unit);
    // Rule 1.
    if c.has_state(unit, state::MOVE_STUCK_54) || cur == mode::DEATH || cur == mode::DEAD {
        return Ok(false);
    }
    // Rule 2.
    if m == 0 {
        return Ok(true);
    }
    let Some(used) = c.used_skill(unit) else {
        return Ok(true);
    };
    // Rule 3.
    if skill == Some(used.id) && matches!(used.srvdofunc, 67 | 76) {
        return Ok(true);
    }
    // Rule 4.
    if !used.interrupt {
        if cur == mode::NEUTRAL {
            reenter_neutral(t, c, unit)?;
            return Ok(true);
        }
        let e = c.first_type1_expire(unit);
        return Ok(matches!(m, 7 | 8 | 10 | 11 | 13 | 18)
            && c.frame() <= e + c.tick_rate().ticks(EARLY_WINDOW));
    }
    // Rule 5: state 42 → one roll; r < v → rule 6. A failed roll, or no
    // state 42, goes on to the state 15 test.
    let mut to_rule6 = false;
    if c.has_state(unit, state::CONCENTRATION) {
        let v = c.state_stat(unit, state::CONCENTRATION, STAT_164);
        let r = c.seed(unit).roll(100);
        to_rule6 = (r as i64) < v as i64;
    }
    if !to_rule6 {
        to_rule6 = c.has_state(unit, state::STATE_15);
    }
    if !to_rule6 {
        return Ok(true);
    }
    // Rule 6.
    if cur == mode::NEUTRAL {
        reenter_neutral(t, c, unit)?;
        return Ok(true);
    }
    Ok(false)
}

fn reenter_neutral<C: PathWorld + WalkUnits + ?Sized>(
    t: &PathTables,
    c: &mut C,
    unit: UnitId,
) -> Result<(), WalkError> {
    request(
        t,
        c,
        unit,
        None,
        mode::NEUTRAL,
        WalkTarget::Point(Point::default()),
        true,
    )?;
    Ok(())
}

/// Starting the movement `(mode)` on a loaded path,
/// steps 1–4 and 6; returns the point count (0: the caller runs the
/// neutral start of step 5).
pub fn start_movement<C: PathWorld + WalkUnits + ?Sized>(
    t: &PathTables,
    c: &mut C,
    unit: UnitId,
    path: &mut DynamicPath,
    m: u32,
) -> Result<i32, WalkError> {
    let ty = c.unit_type(unit);
    // Step 1.
    reset_type(t, path, ty)?;
    // Step 2.
    let mut m = m;
    let stamina_less_run = m == mode::RUN && c.stat(unit, STAT_STAMINA) == 0;
    if m == mode::WALK || m == mode::TOWN_WALK || stamina_less_run {
        m = if path.room.is_some_and(|r| c.room_in_town(r)) {
            mode::TOWN_WALK
        } else {
            mode::WALK
        };
    } else if m == mode::KNOCKBACK {
        path.set_path_type(t, ty == UnitType::Player, path_types::KNOCKBACK_SERVER)?;
        path.dist_budget = 5;
    }
    // Step 3.
    let n = compute(t, c, path, unit, false)?;
    // Step 4.
    c.set_used_skill(unit, None);
    // Step 5.
    if count(path) == 0 {
        return Ok(0);
    }
    // Step 6.
    set_mode_and_velocity(t, c, unit, path, m);
    c.cancel_events(unit, 0);
    c.cancel_events(unit, 1);
    c.schedule_event0(unit);
    Ok(n)
}

/// Mode set plus the velocity half of the animation-rate routine.
pub fn set_mode_and_velocity<U: WalkUnits + ?Sized>(
    t: &PathTables,
    u: &mut U,
    unit: UnitId,
    path: &mut DynamicPath,
    m: u32,
) {
    u.set_mode(unit, m);
    if (m == mode::RUN || m == mode::KNOCKBACK) && u.unit_type(unit) == UnitType::Player {
        let (walk, run, _) = u.charstats_velocity(unit);
        if let Some(v) = run_velocity_bonus(walk, run) {
            u.attach_run_stats(unit, v);
        }
    }
    if let Some(v) = mode_velocity(t, u, unit, m) {
        set_velocity(path, v);
    }
}

/// Neutral start: cancel event types 0 and 1, mode 5 in a
/// town room else 1, used skill none.
pub fn neutral_start<C: PathWorld + WalkUnits + ?Sized>(
    c: &mut C,
    unit: UnitId,
    path: &DynamicPath,
) {
    c.cancel_events(unit, 0);
    c.cancel_events(unit, 1);
    let town = path.room.is_some_and(|r| c.room_in_town(r));
    c.set_mode(
        unit,
        if town {
            mode::TOWN_NEUTRAL
        } else {
            mode::NEUTRAL
        },
    );
    c.set_used_skill(unit, None);
}

//! cltstfunc 30 (Leap Attack): one function of the client skill start
//! table (`super::client_start_apply`); the shared helpers live in `super`.

use super::*;

/// `cltstfunc` 30 (Leap Attack):
/// a used entry E, none → 0. The local player with state 0x36,
/// or with E already this skill with E-flag bit 0, → 0. T := U's target
/// unit. The landing point with (U, T): T within
/// U's reach → none (then the hit start; not modelled: no
/// change, as where the reach is not modelled, [`target_in_reach`]); else
/// c := T's position, or the path target without T; d := the path
/// distance (U, c) (max + min / 2), 0 → none; the point := c + 2 · (c
/// − U) / d per axis (C division). Its blocking (with mask
/// 0x1C09 → the free point), line (0x804) and
/// room tests are read as passing: exact on open ground (Leap Attack
/// at a ground point in the Blood Moor, cast and landing equal).
/// PROVISIONAL (a blocked landing point): the free-point search is not
/// run. With a point: E +0x18 / +0x1C := it, +0x20 := 6 (no T)
/// or T's type and +0x24 its GUID, E-flags := 0x1080, the stat list;
/// 1.
pub(super) fn leap_start(w: &mut ClientWorld, inputs: &ModelInputs, caster: UnitKey, skill: u16) {
    const STATE_CHARGE_TARGET: u8 = 0x36;
    let Some(u) = w.units.get(&caster) else {
        return;
    };
    let Some(e) = u
        .skills
        .as_ref()
        .and_then(|l| l.entries.get(l.current?))
        .copied()
    else {
        return;
    };
    if w.local_player == Some(caster)
        && (u.states.contains(&STATE_CHARGE_TARGET) || (e.skill == skill && e.flags & 1 != 0))
    {
        return;
    }
    let (c, t) = match u.cast_target {
        Some(CastTarget::Unit(t)) => {
            if target_in_reach(w, inputs, caster, t) != Some(false) {
                return;
            }
            let Some((x, y)) = w.units.get(&t).and_then(|t| t.position) else {
                return;
            };
            ((i32::from(x), i32::from(y)), Some(t))
        }
        Some(CastTarget::Point(x, y)) => ((i32::from(x), i32::from(y)), None),
        None => return,
    };
    let (ux, uy) = u.cell();
    let (ux, uy) = (i32::from(ux), i32::from(uy));
    let (dx, dy) = ((c.0 - ux).abs(), (c.1 - uy).abs());
    let d = dx.max(dy) + dx.min(dy) / 2;
    if d == 0 {
        return;
    }
    let point = (c.0 + (c.0 - ux) * 2 / d, c.1 + (c.1 - uy) * 2 / d);
    if let Some(e) = w
        .units
        .get_mut(&caster)
        .and_then(|u| u.skills.as_mut())
        .and_then(|l| l.current.and_then(|i| l.entries.get_mut(i)))
    {
        (e.param, e.param2) = point;
        match t {
            Some(t) => (e.counter, e.param3) = (i32::from(t.unit_type), t.guid as i32),
            None => e.counter = 6,
        }
        e.flags = 0x1080;
    }
}

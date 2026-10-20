//! cltstfunc 25 (Charge): one function of the client skill start
//! table (`super::client_start_apply`); the shared helpers live in `super`.

use super::*;

/// The state the Charge start (`cltstfunc` 25)
/// leaves on U's used entry E: T := the
/// path target unit (the player's unit cast target); no T,
/// or T with state 0x36 → parameter 1 := 6, parameter 2 := 0; else T's
/// type and GUID. Parameter 3 := 7 (15 for monster class 211, not
/// modelled: no monster runs this start here), E-flags := 0x1001 and the
/// counter := 0. The path speed and path compute are the server copy's
/// in single player.
pub(super) fn charge_start(w: &mut ClientWorld, caster: UnitKey) {
    const STATE_CHARGE_TARGET: u8 = 0x36;
    let target = match w.units.get(&caster).and_then(|u| u.cast_target) {
        Some(CastTarget::Unit(t)) => w
            .units
            .get(&t)
            .filter(|t| !t.states.contains(&STATE_CHARGE_TARGET))
            .map(|t| t.key),
        _ => None,
    };
    // The start points U's path at the target with the charge velocity:
    // a moving path.
    if let Some(u) = w.units.get_mut(&caster) {
        u.path_live = Some(true);
    }
    let Some(e) = w
        .units
        .get_mut(&caster)
        .and_then(|u| u.skills.as_mut())
        .and_then(|l| l.current.and_then(|i| l.entries.get_mut(i)))
    else {
        return;
    };
    (e.param, e.param2) = target.map_or((6, 0), |t| (i32::from(t.unit_type), t.guid as i32));
    e.param3 = 7;
    e.flags = 0x1001;
    e.counter = 0;
}

//! cltstfunc 31 (Whirlwind): one function of the client skill start
//! table (`super::client_start_apply`); the shared helpers live in `super`.

use super::*;

/// `cltstfunc` 31 (Whirlwind):
/// a player or monster U with a path and a used entry E. The
/// local player with state 0x36, or with E already this skill with
/// E-flag bit 0, → 0. A target unit within U's reach →
/// the hit start (not modelled: no change, as where the
/// reach is not modelled, [`target_in_reach`]). Else the path
/// takes flags 0xC01 and type 7, the path
/// compute and the speed (a player's `charstats`
/// WalkVelocity +0x40, a monster's `monstats` Walk +0x32, << 8), all the
/// server copy's in single player; the path
/// is then moving. With a last point: E-flags := 1, E +0x18 / +0x1C :=
/// that point (not kept here: no modelled reader), the stat list;
/// 1.
pub(super) fn whirlwind_start(w: &mut ClientWorld, inputs: &ModelInputs, caster: UnitKey) {
    const STATE_CHARGE_TARGET: u8 = 0x36;
    let Some(u) = w.units.get(&caster) else {
        return;
    };
    if u.key.unit_type > 1 {
        return;
    }
    if w.local_player == Some(caster) && u.states.contains(&STATE_CHARGE_TARGET) {
        return;
    }
    if let Some(CastTarget::Unit(t)) = u.cast_target {
        if target_in_reach(w, inputs, caster, t) != Some(false) {
            return;
        }
    }
    if let Some(u) = w.units.get_mut(&caster) {
        u.path_live = Some(true);
    }
    if let Some(e) = w
        .units
        .get_mut(&caster)
        .and_then(|u| u.skills.as_mut())
        .and_then(|l| l.current.and_then(|i| l.entries.get_mut(i)))
    {
        e.flags = 1;
    }
}

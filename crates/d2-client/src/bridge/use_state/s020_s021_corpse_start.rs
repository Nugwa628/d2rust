//! cltstfunc 20 (Raise) and 21 (the explosions): two functions of the
//! client skill start table (`super::client_start_apply`) and their pass
//! tests (`super::client_start_passes`); the shared helpers live in
//! `super`.

use super::*;

/// U has a used skill entry.
pub(super) fn has_entry(w: &ClientWorld, caster: UnitKey) -> bool {
    w.units
        .get(&caster)
        .and_then(|u| u.skills.as_ref())
        .is_some_and(|l| l.current.is_some_and(|i| i < l.entries.len()))
}

/// The corpse tests of `cltstfunc` 20 and 21:
/// T a monster in mode 12 (dead), in no state of the `udead`
/// group, its class `corpseSel` (monstats flag 7);
/// the raise test with (T, 0) adds a non-zero `monstats`
/// `Velocity` (+0x32), the corpse test with (T) (`corpse`) does
/// not.
pub(super) fn raise_corpse(
    inputs: &ModelInputs,
    u: &crate::bridge::world::ClientUnit,
    corpse: bool,
) -> bool {
    const MODE_DEAD: u32 = 12;
    if u.key.unit_type != MONSTER || u.mode != MODE_DEAD {
        return false;
    }
    let t = &inputs.tables;
    let udead = u
        .states
        .iter()
        .any(|&s| t.states.get(usize::from(s)).is_some_and(|r| r.udead));
    let Some(class) = t.monsters.get(u.class as usize).copied().flatten() else {
        return false;
    };
    !udead && class.corpse_sel && (corpse || class.setup.is_some_and(|s| s.velocity != 0))
}

/// `cltstfunc` 20 and 21:
/// U's used entry E +0x18 / +0x1C := the target's position;
/// function 21 also writes E +0x20 := its class.
pub(super) fn corpse_start(
    w: &mut ClientWorld,
    caster: UnitKey,
    target: Option<UnitKey>,
    class: bool,
) {
    let Some(t) = target.and_then(|k| w.units.get(&k)) else {
        return;
    };
    let (x, y) = t.position.unwrap_or((0, 0));
    let c = t.class as i32;
    if let Some(e) = w
        .units
        .get_mut(&caster)
        .and_then(|u| u.skills.as_mut())
        .and_then(|l| l.current.and_then(|i| l.entries.get_mut(i)))
    {
        (e.param, e.param2) = (i32::from(x), i32::from(y));
        if class {
            e.counter = c;
        }
    }
}

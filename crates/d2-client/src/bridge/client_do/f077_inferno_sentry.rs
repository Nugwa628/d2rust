//! cltdofunc 77 (mon inferno sentry) and its start, `cltstfunc` 52: one
//! function of the client skill do table (`super::client_do`); the
//! shared helpers live in `super`.

use super::*;

/// The state the sentry's start sets and its do clears: 12, as Inferno's
/// (the state bit only).
const STATE: u8 = 12;

/// `cltstfunc` 52: the skill row not
/// valid or no used entry E → 0. d := the skill's `calc2` (+0x13C),
/// at least 1; E +0x18 := the client update counter plus
/// d; state 12 on U when it is off; 1.
pub(super) fn inferno_sentry_start(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    if inputs.tables.skills.get(usize::from(skill)).is_none()
        || crate::bridge::skills::used_entry(w, key).is_none()
    {
        return Ok(0);
    }
    let d = calc2(w, inputs, key, skill, level)?.max(1);
    let until = (w.drlg_updates as i32).wrapping_add(d);
    if let Some(e) = crate::bridge::skills::used_entry_mut(w, key) {
        e.param = until;
    }
    if let Some(u) = w.units.get_mut(&key) {
        u.states.insert(STATE);
    }
    Ok(1)
}

/// `cltdofunc` 77: the skill row not
/// valid or no used entry E → 0. E +0x18 (unsigned) below the counter
/// → state 12 off on U, 0. Else X := the Inferno create
/// with (U, skill, level) ([`super::f024_inferno::inferno_core`]);
/// none → 0. X's motion record zeroed, its position :=
/// (0, 0, v ≪ 11) with v the skill's `cltcalc1` (+0x114),
/// its flag 0x20 on;
/// U's flag 0x40 cleared; 1.
pub(super) fn inferno_sentry(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    if inputs.tables.skills.get(usize::from(skill)).is_none() {
        return Ok(0);
    }
    let Some(until) = crate::bridge::skills::used_entry(w, key).map(|e| e.param as u32) else {
        return Ok(0);
    };
    if until < w.drlg_updates {
        crate::bridge::msg::states::state_bit_off(w, key, STATE);
        return Ok(0);
    }
    let Some(x) = super::f024_inferno::inferno_core(w, inputs, key, skill, level)? else {
        return Ok(0);
    };
    let v = cltcalc1(w, inputs, key, skill, level)?;
    if let Some(m) = w.objclient.missiles.get_mut(&x) {
        m.motion = Default::default();
        m.motion.pos = [0, 0, v << 11];
        m.motion.flags |= crate::rules::unit_composite::motion::FROM_BELOW;
    }
    if let Some(u) = w.units.get_mut(&key) {
        u.flag_40 = false;
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_inferno_sentry_casts_until_its_start_deadline_then_clears_state_12() {
        let (mut w, key) = aimed_at((5010, 5000));
        let mut i = inputs(77);
        i.skill_tables = Some(calc1_tables(38));
        // No used entry: start and do both 0, nothing made.
        let list = w.units.get_mut(&key).unwrap().skills.as_mut().unwrap();
        list.current = None;
        assert_eq!(
            inferno_sentry_start(&mut w, &i, key, FROST_NOVA, 3).unwrap(),
            0
        );
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 0);
        assert!(missiles(&w).is_empty());
        // The start: E +0x18 := counter + calc2 (at least 1), state 12.
        let list = w.units.get_mut(&key).unwrap().skills.as_mut().unwrap();
        list.current = Some(0);
        w.drlg_updates = 100;
        assert_eq!(
            inferno_sentry_start(&mut w, &i, key, FROST_NOVA, 3).unwrap(),
            1
        );
        let until = crate::bridge::skills::used_entry(&w, key).unwrap().param;
        assert!(until > 100);
        assert!(w.units[&key].states.contains(&12));
        // Before the deadline: one missile, motion reset with flag 0x20,
        // U's flag 0x40 cleared.
        w.units.get_mut(&key).unwrap().flag_40 = true;
        w.drlg_updates = until as u32;
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        let ms = missiles(&w);
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].motion.flags, 0x20);
        assert_eq!(ms[0].motion.pos[..2], [0, 0]);
        assert!(!w.units[&key].flag_40);
        // Past it: state 12 off, nothing made, 0.
        w.drlg_updates = until as u32 + 1;
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 0);
        assert_eq!(missiles(&w).len(), 1);
        assert!(!w.units[&key].states.contains(&12));
    }
}

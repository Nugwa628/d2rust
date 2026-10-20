//! cltdofunc 78 (mon death sentry) and 79 (fenris rage) with their
//! starts, `cltstfunc` 50 and 51: functions of the client skill do table
//! (`super::client_do`); the shared helpers live in `super`.

use super::*;
use crate::bridge::skills::{used_entry, used_entry_mut};

/// `cltstfunc` 50 (mon death sentry): no used
/// entry E or no path target unit T → 0. E +0x18 := T's x,
/// E +0x1C := T's y (an object, item or tile
/// its static position, else its path position, 0 without a path); 1.
pub(super) fn death_sentry_start(w: &mut ClientWorld, key: UnitKey) -> i32 {
    if used_entry(w, key).is_none() {
        return 0;
    }
    let Some(t) = path_target_unit(w, key) else {
        return 0;
    };
    let (x, y) = w.units.get(&t).map_or((0, 0), |u| u.cell());
    if let Some(e) = used_entry_mut(w, key) {
        e.param = i32::from(x);
        e.param2 = i32::from(y);
    }
    1
}

/// `cltstfunc` 51 (fenris rage): no used entry
/// E, no path target unit T, or T not a corpse → 0. E
/// +0x18 := T's x, +0x1C := T's y, +0x20 := T's GUID; 1.
pub(super) fn fenris_rage_start(w: &mut ClientWorld, key: UnitKey) -> i32 {
    if used_entry(w, key).is_none() {
        return 0;
    }
    let Some(t) = path_target_unit(w, key) else {
        return 0;
    };
    let Some(u) = w.units.get(&t) else {
        return 0;
    };
    if !crate::bridge::hover::soft_corpse(&w.corpse_tables, u) {
        return 0;
    }
    let (x, y) = u.cell();
    if let Some(e) = used_entry_mut(w, key) {
        e.param = i32::from(x);
        e.param2 = i32::from(y);
        e.counter = t.guid as i32;
    }
    1
}

/// The used entry's point (E +0x18, +0x1C), both nonzero.
fn entry_point(w: &ClientWorld, key: UnitKey) -> Option<(i32, i32)> {
    let e = used_entry(w, key)?;
    (e.param != 0 && e.param2 != 0).then_some((e.param, e.param2))
}

/// `cltdofunc` 78 (mon death sentry):
/// the skill row not valid, no used entry E, or E +0x18 or +0x1C
/// zero → 0. (x, y) := (E +0x18, E +0x1C). `cltmissilea` in the missile
/// table → one create with (U, a, x, y, skill, level); none made
/// → 0 (a made one with `prgsound` > 0 would request it on the missile:
/// blank for the only row, 312). `cltmissileb` in the table → one more
/// create at (x, y). 1. Flag 0x40 untouched.
pub(super) fn death_sentry(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let Some(p) = entry_point(w, key) else {
        return Ok(0);
    };
    let [a, b, _] = row.cltmissile_abc.map(i32::from);
    if valid_missile(inputs, a) && create_at(w, inputs, key, a, p, skill, level)?.is_none() {
        return Ok(0);
    }
    if valid_missile(inputs, b) {
        create_at(w, inputs, key, b, p, skill, level)?;
    }
    Ok(1)
}

/// `cltdofunc` 79 (fenris rage):
/// the skill row not valid, no used entry E, or E +0x18 or +0x1C zero →
/// 0. (x, y) := (E +0x18, E +0x1C); U's path target := (x, y) and its
/// target unit cleared; `cltmissilea` in the missile
/// table → one create with (U, a, x, y, skill, level). 1. Flag
/// 0x40 untouched.
pub(super) fn fenris_rage(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let Some((x, y)) = entry_point(w, key) else {
        return Ok(0);
    };
    let who = super::super::critter_path::Who::server(key);
    super::super::critter_path::set_target(w, who, x as u16, y as u16);
    super::super::critter_path::set_target_unit(w, who, None);
    let a = i32::from(row.cltmissile_abc[0]);
    if valid_missile(inputs, a) {
        create_at(w, inputs, key, a, (x, y), skill, level)?;
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    fn set_point(w: &mut ClientWorld, key: UnitKey, p: (i32, i32)) {
        let e = used_entry_mut(w, key).unwrap();
        (e.param, e.param2) = p;
    }

    #[test]
    fn the_death_sentry_makes_both_missiles_at_its_start_point() {
        let (mut w, key, cow) = aimed_at_cow();
        let mut i = inputs(78);
        i.tables.missiles = vec![ClientMissileRow::default(); 5];
        i.tables.skills[usize::from(FROST_NOVA)].cltmissile_abc = [CLASS as i16, 4, -1];
        // The start takes the target unit's position.
        assert_eq!(death_sentry_start(&mut w, key), 1);
        let e = used_entry(&w, key).unwrap();
        let (cx, cy) = w.units[&cow].cell();
        assert_eq!((e.param, e.param2), (i32::from(cx), i32::from(cy)));
        // The do: two creates at that point; flag 0x40 untouched.
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        assert_eq!(missiles(&w).len(), 2);
        assert!(!w.units[&key].flag_40);
        // A zero coordinate: 0, nothing more made.
        set_point(&mut w, key, (0, 5000));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 0);
        assert_eq!(missiles(&w).len(), 2);
    }

    #[test]
    fn the_fenris_rage_start_needs_a_corpse_and_the_do_makes_one_missile() {
        let (mut w, key, cow) = aimed_at_cow();
        let i = inputs(79);
        // A live target is not a corpse: the start returns 0.
        assert_eq!(fenris_rage_start(&mut w, key), 0);
        // The do at a set point: one create.
        set_point(&mut w, key, (5003, 5004));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        assert_eq!(missiles(&w).len(), 1);
        // A zero point: 0.
        set_point(&mut w, key, (5003, 0));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 0);
        let _ = cow;
    }
}

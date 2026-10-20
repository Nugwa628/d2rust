//! cltdofunc 73 (Teleport 2, Baal Teleport): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 73 (Teleport 2, Baal
/// Teleport, Baal Clone Teleport): b := `cltmissileb` (+0xEC,
/// signed); outside the missile table → 1. Else one create
/// with (U, b, U.x, U.y, skill, 1) at U's own position (level
/// 1): made → 1, else 0.
pub(super) fn teleport_end(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let b = i32::from(row.cltmissile_abc[1]);
    if !valid_missile(inputs, b) {
        return Ok(1);
    }
    let Some((x, y)) = w.units.get(&key).map(|u| u.cell()) else {
        return Ok(0);
    };
    let made = create_at(w, inputs, key, b, (i32::from(x), i32::from(y)), skill, 1)?;
    Ok(i32::from(made.is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_teleport_end_do_makes_cltmissileb_at_the_caster_at_level_1() {
        // cltmissileb outside the table: 1, nothing made.
        let (mut w, key) = aimed_at((5006, 4997));
        assert_eq!(
            client_do(&mut w, &inputs(73), key, FROST_NOVA, 3).unwrap(),
            1
        );
        assert!(missiles(&w).is_empty());
        let (mut w, key) = aimed_at((5006, 4997));
        assert_eq!(
            client_do(&mut w, &inputs_b(73), key, FROST_NOVA, 3).unwrap(),
            1
        );
        let ms = missiles(&w);
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].level, 1);
        assert!(!w.units[&key].flag_40);
        let (mut w, key) = aimed_at((5006, 4997));
        assert_eq!(client_do(&mut w, &inputs_b(73), key, 9999, 3).unwrap(), 0);
    }
}

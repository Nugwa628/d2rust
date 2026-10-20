//! cltdofunc 7 (Dragon Tail): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 7 (Dragon Tail):
/// a target unit ([`path_target_unit`]) is required (none → 0); m :=
/// [`missile_class`], invalid → 0; [`cast_at`] (U, m) (made or not);
/// flag 0x40 on U; 1.
pub(super) fn dragon_tail(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    if path_target_unit(w, key).is_none() {
        return Ok(0);
    }
    let m = missile_class(w, key, &row);
    if !valid_missile(inputs, m) {
        return Ok(0);
    }
    cast_at(w, inputs, key, m, skill, level)?;
    set_flag_40(w, key);
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_dragon_tail_do_needs_a_target_unit_and_makes_one_missile_at_it() {
        let (mut w, key) = aimed_at((5004, 5002));
        assert_eq!(
            client_do(&mut w, &inputs(7), key, FROST_NOVA, 3).unwrap(),
            0
        );
        assert!(missiles(&w).is_empty());
        let (mut w, key, _) = aimed_at_cow();
        assert_eq!(
            client_do(&mut w, &inputs(7), key, FROST_NOVA, 3).unwrap(),
            1
        );
        assert!(w.units[&key].flag_40);
        assert_eq!(missiles(&w).len(), 1);
    }
}

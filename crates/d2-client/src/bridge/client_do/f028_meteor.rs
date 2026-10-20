//! cltdofunc 28 (Meteor, Blizzard): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 28 (Meteor, Blizzard):
/// m := [`missile_class`]; invalid → 0. Flag 0x40 on U;
/// [`target_free`] (U, m) → whether [`cast_at`] (U, m) made one; else 0.
pub(super) fn meteor(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let m = missile_class(w, key, &row);
    if !valid_missile(inputs, m) {
        return Ok(0);
    }
    set_flag_40(w, key);
    if !target_free(w, inputs, key, m) {
        return Ok(0);
    }
    Ok(i32::from(
        cast_at(w, inputs, key, m, skill, level)?.is_some(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_meteor_do_makes_its_missile_at_a_target_within_100() {
        let (mut w, key) = aimed_at((5060, 5080));
        assert_eq!(
            client_do(&mut w, &inputs(28), key, FROST_NOVA, 3).unwrap(),
            1
        );
        assert!(w.units[&key].flag_40);
        assert_eq!(
            w.objclient.set_c.values().next().unwrap().position,
            Some((5060, 5080))
        );
        // 61² + 80² > 10000: nothing made, flag 0x40 still set.
        let (mut w, key) = aimed_at((5061, 5080));
        assert_eq!(
            client_do(&mut w, &inputs(28), key, FROST_NOVA, 3).unwrap(),
            0
        );
        assert!(w.units[&key].flag_40 && missiles(&w).is_empty());
    }
}

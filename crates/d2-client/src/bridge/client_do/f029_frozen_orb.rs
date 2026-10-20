//! cltdofunc 29 (Frozen Orb): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 29 (Frozen Orb):
/// m := [`missile_class`]; invalid → 0. Flag 0x40 on U; whether
/// [`cast_toward`] (U, m) made one.
pub(super) fn frozen_orb(
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
    Ok(i32::from(
        cast_toward(w, inputs, key, m, skill, level)?.is_some(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_frozen_orb_do_sends_one_missile_toward_the_target() {
        let (mut w, key) = aimed_at((5010, 5000));
        assert_eq!(
            client_do(&mut w, &inputs(29), key, FROST_NOVA, 3).unwrap(),
            1
        );
        assert!(w.units[&key].flag_40);
        let ms = missiles(&w);
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].target_point, (5010, 5000));
        assert_eq!(ms[0].owner, Some(key));
    }
}

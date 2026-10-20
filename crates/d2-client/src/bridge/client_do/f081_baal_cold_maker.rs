//! cltdofunc 81 (Baal Cold Missiles): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 81 (Baal Cold Missiles):
/// m := [`missile_class`]; invalid → 0. T := the cast target
/// point; X := [`cast_toward`] m (no flag 0x40); none → 0.
/// X's d28 := −(T.y − U.y), d2C := T.x − U.x;
/// 1.
pub(super) fn baal_cold_maker(
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
    let (tx, ty) = cast_target_point(w, key);
    let Some(x) = cast_toward(w, inputs, key, m, skill, level)? else {
        return Ok(0);
    };
    let Some((ux, uy)) = w.units.get(&key).map(|u| u.cell()) else {
        return Ok(0);
    };
    if let Some(mm) = w.objclient.missiles.get_mut(&x) {
        mm.d28 = -(ty - i32::from(uy));
        mm.d2c = tx - i32::from(ux);
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_baal_cold_maker_do_aims_toward_the_target_without_flag_40() {
        let (mut w, key) = aimed_at((5006, 4997));
        assert_eq!(
            client_do(&mut w, &inputs(81), key, FROST_NOVA, 3).unwrap(),
            1
        );
        assert!(!w.units[&key].flag_40);
        let ms = missiles(&w);
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].owner, Some(key));
        assert_eq!((ms[0].d28, ms[0].d2c), (3, 6));
        let (mut w, key) = aimed_at((5006, 4997));
        assert_eq!(client_do(&mut w, &inputs(81), key, 9999, 3).unwrap(), 0);
        assert!(missiles(&w).is_empty());
    }
}

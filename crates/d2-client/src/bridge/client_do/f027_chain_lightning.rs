//! cltdofunc 27 (Chain Lightning): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 27 (Chain Lightning):
/// m := [`missile_class`]; invalid → 0. Flag 0x40 on U; v := the
/// skill's `calc1`; X := [`cast_toward`] (U, m); X made → its d28 := v,
/// 1; else 0.
pub(super) fn chain_lightning(
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
    let v = calc1(w, inputs, key, skill, level)?;
    let Some(x) = cast_toward(w, inputs, key, m, skill, level)? else {
        return Ok(0);
    };
    if let Some(mm) = w.objclient.missiles.get_mut(&x) {
        mm.d28 = v;
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_chain_lightning_do_stores_calc1_in_the_missile() {
        let (mut w, key) = aimed_at((5010, 5000));
        let mut i = inputs(27);
        i.skill_tables = Some(calc1_tables(7));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        let ms = missiles(&w);
        assert_eq!((ms.len(), ms[0].d28), (1, 7));
    }
}

//! cltdofunc 36 (Fist of the Heavens): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 36 (Fist of the Heavens):
/// m := [`missile_class`]; invalid → 0. Flag 0x40 on U; T :=
/// [`path_target_unit`]; none → 0. One create, record flags 1, owner U,
/// class m, start pos(T), skill, level; made → its d28 := T's unit type,
/// d2C := T's GUID, 1; else 0.
pub(super) fn fist_of_heavens(
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
    let Some(t) = path_target_unit(w, key) else {
        return Ok(0);
    };
    let Some((tx, ty)) = w.units.get(&t).map(|u| u.cell()) else {
        return Ok(0);
    };
    let Some(k) = create_at(
        w,
        inputs,
        key,
        m,
        (i32::from(tx), i32::from(ty)),
        skill,
        level,
    )?
    else {
        return Ok(0);
    };
    if let Some(mm) = w.objclient.missiles.get_mut(&k) {
        mm.d28 = i32::from(t.unit_type);
        mm.d2c = t.guid as i32;
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_fist_of_heavens_do_drops_its_missile_on_the_target_unit() {
        let (mut w, key, t) = aimed_at_unit();
        assert_eq!(
            client_do(&mut w, &inputs(36), key, FROST_NOVA, 3).unwrap(),
            1
        );
        assert!(w.units[&key].flag_40);
        let ms = missiles(&w);
        assert_eq!(ms.len(), 1);
        assert_eq!((ms[0].d28, ms[0].d2c), (i32::from(MONSTER), 77));
        assert_eq!(ms[0].owner, Some(key));
        assert_eq!(
            w.objclient.set_c.values().next().unwrap().position,
            w.units[&t].position
        );
        // No target unit: flag 0x40 set, nothing made.
        let (mut w, key) = aimed_at((5010, 5000));
        assert_eq!(
            client_do(&mut w, &inputs(36), key, FROST_NOVA, 3).unwrap(),
            0
        );
        assert!(w.units[&key].flag_40 && missiles(&w).is_empty());
    }
}

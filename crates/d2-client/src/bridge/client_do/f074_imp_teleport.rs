//! cltdofunc 74 (Imp Teleport): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// Component 14 (S7), the layer is called with (EDX = 14).
const S7: u8 = 14;

/// `cltdofunc` 74 (Imp Teleport):
/// m := `cltmissileb` (+0xEC, signed); outside the missile table → 1
/// with no missile. X := [`cast_at`] m with level 1; none → 0. U's path
/// target unit K ([`path_target_unit`]) → X's motion record zeroed,
/// position := (a ≪ 11, 0, −b ≪ 11) with
/// (a, b) K's S7 cel offset (EDX = 14,
/// [`ModelInputs::cel_offset`]); 1.
pub(super) fn imp_teleport(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let m = i32::from(row.cltmissile_abc[1]);
    if !valid_missile(inputs, m) {
        return Ok(1);
    }
    let Some(x) = cast_at(w, inputs, key, m, skill, 1)? else {
        return Ok(0);
    };
    if let Some(k) = path_target_unit(w, key) {
        // K's layer-14 (S7) cel offset, read from the view's unit
        // art. PROVISIONAL: when the request fails, (a, b) would be
        // left unset; D2RUST takes (0, 0) there (no base-game player
        // COF has an S7 layer).
        let (a, b) = w
            .units
            .get(&k)
            .zip(inputs.cel_offset.as_ref())
            .and_then(|(u, f)| f.cel_offset(u, S7))
            .unwrap_or((0, 0));
        if let Some(mm) = w.objclient.missiles.get_mut(&x) {
            mm.motion = Default::default();
            mm.motion.pos = [a << 11, 0, (-b) << 11];
        }
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;
    use crate::bridge::world::CelOffsetFn;

    #[test]
    fn the_imp_teleport_do_makes_its_missile_at_the_target_and_zeroes_its_motion() {
        // cltmissileb outside the table: 1, nothing made.
        let (mut w, key) = aimed_at((5006, 4997));
        assert_eq!(
            client_do(&mut w, &inputs(74), key, FROST_NOVA, 3).unwrap(),
            1
        );
        assert!(missiles(&w).is_empty());
        // A target point: one missile, level 1, no flag 0x40.
        let (mut w, key) = aimed_at((5006, 4997));
        assert_eq!(
            client_do(&mut w, &inputs_b(74), key, FROST_NOVA, 3).unwrap(),
            1
        );
        let ms = missiles(&w);
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].level, 1);
        assert!(!w.units[&key].flag_40);
        // A target unit: the missile's motion record is reset.
        let (mut w, key, _) = aimed_at_cow();
        assert_eq!(
            client_do(&mut w, &inputs_b(74), key, FROST_NOVA, 3).unwrap(),
            1
        );
        let ms = missiles(&w);
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].motion, Default::default());
        // K's S7 cel offset (a, b) = (-14, -91): position (a << 11, 0,
        // -b << 11); the layer asked for is 14.
        let (mut w, key, _) = aimed_at_cow();
        let mut i = inputs_b(74);
        i.cel_offset = Some(CelOffsetFn::new(|_, c| (c == 14).then_some((-14, -91))));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        let ms = missiles(&w);
        assert_eq!(ms[0].motion.pos, [-14 << 11, 0, 91 << 11]);
        assert_eq!(ms[0].motion.flags, 0);
        // Unknown skill: 0.
        let (mut w, key) = aimed_at((5006, 4997));
        assert_eq!(client_do(&mut w, &inputs_b(74), key, 9999, 3).unwrap(), 0);
    }
}

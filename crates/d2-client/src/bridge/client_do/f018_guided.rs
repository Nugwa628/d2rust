//! cltdofunc 18 (Guided Arrow, Bone Spirit): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 18 (Guided Arrow, Bone
/// Spirit): flag 0x40 on U; P := [`cast_target_point`]; m :=
/// [`missile_class`], invalid → 0. T := [`path_target_unit`]. One create:
/// record flags 0x20 (0x420 without T), owner and origin U, target unit
/// T, class m, target P, skill, level. A made missile: d28 := 1 with T, 2
/// without; d2C := (P.y − U.y) << 16 + (P.x − U.x), each
/// difference as a signed 16-bit value. 1.
pub(super) fn guided(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    set_flag_40(w, key);
    let (tx, ty) = cast_target_point(w, key);
    let m = missile_class(w, key, &row);
    if !valid_missile(inputs, m) {
        return Ok(0);
    }
    let t = path_target_unit(w, key);
    let mut flags = flag::TARGET_ABSOLUTE;
    if t.is_none() {
        flags |= flag::FRAMES_FROM_DISTANCE;
    }
    let rec = CreateRecord {
        flags,
        owner: Some(key),
        origin: Some(key),
        target: t,
        class: m as u32,
        tx,
        ty,
        skill: i32::from(skill),
        level,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    let Some(x) = create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)? else {
        return Ok(1);
    };
    let (ux, uy) = w.units.get(&key).map_or((0, 0), |u| u.cell());
    let dx = i32::from(tx.wrapping_sub(i32::from(ux)) as i16);
    let dy = i32::from(ty.wrapping_sub(i32::from(uy)) as i16);
    if let Some(mm) = w.objclient.missiles.get_mut(&x) {
        mm.d28 = if t.is_some() { 1 } else { 2 };
        mm.d2c = (dy << 16).wrapping_add(dx);
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_guided_do_stores_the_target_kind_and_the_packed_offset() {
        let (mut w, key, cow) = aimed_at_cow();
        assert_eq!(
            client_do(&mut w, &inputs(18), key, FROST_NOVA, 3).unwrap(),
            1
        );
        assert!(w.units[&key].flag_40);
        let ms = missiles(&w);
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].target_unit, Some(cow));
        assert_eq!((ms[0].d28, ms[0].d2c), (1, (2 << 16) + 4));
        let (mut w, key) = aimed_at((4997, 4999));
        client_do(&mut w, &inputs(18), key, FROST_NOVA, 3).unwrap();
        let ms = missiles(&w);
        assert_eq!((ms[0].d28, ms[0].d2c), (2, (-1 << 16) - 3));
    }
}

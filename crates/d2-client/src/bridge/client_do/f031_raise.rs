//! cltdofunc 31 (Raise Skeleton, Raise Skeletal Mage): one function of
//! the client skill do table (`super::client_do`); the shared helpers
//! live in `super`.

use super::*;

/// `cltdofunc` 31 (Raise Skeleton, Raise
/// Skeletal Mage): a valid skill row, m :=
/// the progressive missile class inside the missiles table and U's used entry
/// E; one create of flags 0x2001 (start position, random direction),
/// owner U, class m, start (E +0x18, E +0x1C) (`cltstfunc` 20 wrote T's
/// position there); its result is ignored. E +0x18 and +0x1C := 0, 1.
/// No flag 0x40.
pub(super) fn raise(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)) else {
        return Ok(0);
    };
    let m = missile_class(w, key, row);
    if !valid_missile(inputs, m) {
        return Ok(0);
    }
    let Some(e) = current_entry(w, key).copied() else {
        return Ok(0);
    };
    let rec = CreateRecord {
        flags: flag::POSITION | flag::RANDOM_DIRECTION,
        owner: Some(key),
        class: m as u32,
        x: e.param,
        y: e.param2,
        skill: i32::from(skill),
        level,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?;
    if let Some(e) = current_entry_mut(w, key) {
        (e.param, e.param2) = (0, 0);
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn raise_creates_one_missile_at_the_entry_point_and_clears_it() {
        let (mut w, key) = world();
        let i = inputs(31);
        corpse_entry(&mut w, key, 5003, 5002, 0);
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 2).unwrap(), 1);
        assert_eq!(made(&w).len(), 1);
        assert_eq!((made(&w)[0].0, made(&w)[0].1), (CLASS, (5003, 5002)));
        let e = current_entry(&w, key).expect("entry");
        assert_eq!((e.param, e.param2), (0, 0));
        assert!(!w.units[&key].flag_40);
        // An invalid class: 0, nothing.
        let mut i = inputs(31);
        i.tables.skills[usize::from(FROST_NOVA)].cltmissile_abc = [9, -1, -1];
        let (mut w, key) = world();
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 2).unwrap(), 0);
        assert!(made(&w).is_empty());
    }
}

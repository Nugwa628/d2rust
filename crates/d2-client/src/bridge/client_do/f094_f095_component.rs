//! cltdofunc 94 and 95 (Doom Knight and Necromage missiles): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 94 (Doom Knight missile, skill 335) and 95
/// (Necromage missile, skill 338): m :=
/// `cltmissilea` + the monster's component `slot` (10 = S3 for 94, 11
/// = S4 for 95; monster data +0x0E / +0x0F, 0 for any other unit); a
/// negative `cltmissilea`, or m outside the missile table → 0. Flag
/// 0x40 on U, then [`cast_toward`] (U, m); returns 1 whether or not a
/// missile was made.
pub(super) fn component_missile(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
    slot: usize,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let a = i32::from(row.cltmissile_abc[0]);
    if !valid_missile(inputs, a) {
        return Ok(0);
    }
    let comp = match w.units.get(&key).map(|u| &u.kind) {
        Some(KindData::Monster(m)) if key.unit_type == MONSTER => i32::from(m.components[slot]),
        _ => 0,
    };
    let m = a + comp;
    if !valid_missile(inputs, m) {
        return Ok(0);
    }
    set_flag_40(w, key);
    if row.lob {
        return Err(HandlerError::Invalid(
            "client/client-do/f094-f095-component.md §1: the lob create is not modelled",
        ));
    }
    cast_toward(w, inputs, key, m, skill, level)?;
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_component_dos_offset_cltmissilea_by_a_monster_component() {
        use crate::bridge::world::MonsterData;
        for (func, slot) in [(94, 10), (95, 11)] {
            let with_component = |c: u8| {
                let (mut w, mut i, key) = caster((5010, 5000));
                let row = &mut i.tables.skills[usize::from(FROST_NOVA)];
                row.cltdofunc = func;
                row.cltmissile = -1;
                row.cltmissile_abc = [1, -1, -1];
                let mut data = MonsterData::default();
                data.components[slot] = c;
                w.units.get_mut(&key).unwrap().kind = KindData::Monster(Box::new(data));
                (w, i, key)
            };
            // 1 + 2 = 3: the class the create takes, aimed at T.
            let (mut w, i, key) = with_component(2);
            assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
            assert!(w.units[&key].flag_40);
            assert_eq!(missiles(&w).len(), 1);
            assert_eq!(missiles(&w)[0].target_point, (5010, 5000));
            assert!(w.objclient.set_c.values().all(|u| u.class == CLASS));
            // a + comp past the missile table: 0, no flag, nothing made.
            let (mut w, i, key) = with_component(3);
            assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 0);
            assert!(!w.units[&key].flag_40);
            assert!(missiles(&w).is_empty());
        }
    }
}

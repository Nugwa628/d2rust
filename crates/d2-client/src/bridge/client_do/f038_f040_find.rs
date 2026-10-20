//! cltdofunc 38 and 40 (Find Potion, Find Item): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 38 (Find Potion): T := U's
/// path target unit, none → 0. Flag 0x40 on U; the blood spray
/// with (T, 5, 0) on T; 1.
pub(super) fn find_potion(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
) -> Result<i32, HandlerError> {
    let Some(t) = path_target_unit(w, key) else {
        return Ok(0);
    };
    set_flag_40(w, key);
    crate::bridge::blood_spray::spray(w, inputs, t, 5, false)?;
    Ok(1)
}

/// `cltdofunc` 40 (Find Item): as 38, but T
/// must also pass: a monster whose `monstats2` has `soft`
/// (monstats2 flag 0xC); else 0 with no flag.
pub(super) fn find_item(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
) -> Result<i32, HandlerError> {
    let Some(t) = path_target_unit(w, key) else {
        return Ok(0);
    };
    if soft_monster(w, inputs, t).is_none() {
        return Ok(0);
    }
    set_flag_40(w, key);
    crate::bridge::blood_spray::spray(w, inputs, t, 5, false)?;
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn find_potion_and_find_item_spray_on_their_target_corpse() {
        use crate::bridge::world::MonsterClass;
        let cow = UnitKey::new(MONSTER, 9);
        let setup = |soft: bool| {
            let (mut w, key) = world();
            let mut c = ClientUnit::new(cow);
            c.class = 2;
            c.mode = 12;
            c.position = Some((5003, 5000));
            c.seed = Some((1234, 666));
            w.units.insert(cow, c);
            let mut i = inputs(38);
            i.tables.monsters = vec![None; 3];
            i.tables.monsters[2] = Some(MonsterClass {
                bleed: 1,
                soft,
                ..MonsterClass::default()
            });
            (w, key, i)
        };
        // No target unit: 0, no flag.
        let (mut w, key, i) = setup(true);
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 0);
        assert!(!w.units[&key].flag_40);
        // Function 38: flag 0x40, five sprays drawn on the target's seed.
        w.units.get_mut(&key).unwrap().cast_target = Some(CastTarget::Unit(cow));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 1);
        assert!(w.units[&key].flag_40);
        assert_ne!(w.units[&cow].seed, Some((1234, 666)));
        // Function 40 needs `soft`.
        let (mut w, key, mut i) = setup(false);
        i.tables.skills[usize::from(FROST_NOVA)].cltdofunc = 40;
        w.units.get_mut(&key).unwrap().cast_target = Some(CastTarget::Unit(cow));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 0);
        assert!(!w.units[&key].flag_40);
        assert_eq!(w.units[&cow].seed, Some((1234, 666)));
        i.tables.monsters[2].as_mut().unwrap().soft = true;
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 1);
        assert!(w.units[&key].flag_40);
    }
}

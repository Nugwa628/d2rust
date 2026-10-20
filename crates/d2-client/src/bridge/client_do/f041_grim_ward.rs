//! cltdofunc 41 (Grim Ward): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// arguments of Grim Ward's spot: size 2, mask 0x1000
/// (the unit marker), fallback on.
pub(super) const GRIM_WARD_SIZE: i32 = 2;

pub(super) const GRIM_WARD_MASK: u16 = 0x1000;

/// The second missile of Grim Ward: `corpseexplosion`.
pub(super) const CORPSE_EXPLOSION: i32 = 0x73;

/// `cltdofunc` 41 (Grim Ward):
/// T := U's path target unit, which must pass (a `soft`
/// monster, as 40); else 0. Flag 0x40 on U. (x, y) := T's position, moved
/// to the free point of the room holding it (size 2, mask
/// 0x1000, fallback); none → 0. m := `cltmissilec` for a `large`
/// corpse, else `cltmissileb` for a `small` one, else `cltmissilea`;
/// not a valid missile → 0. Two creates of a record with flags 0x2001
/// (start (x, y), random facing), owner U, skill, level: class m, then
/// `corpseexplosion`; 1.
pub(super) fn grim_ward(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)) else {
        return Ok(0);
    };
    let Some(t) = path_target_unit(w, key) else {
        return Ok(0);
    };
    let Some(class) = soft_monster(w, inputs, t) else {
        return Ok(0);
    };
    set_flag_40(w, key);
    let Some((x, y)) = w.units.get(&t).and_then(|u| u.position) else {
        return Ok(0);
    };
    let at = (i32::from(x), i32::from(y));
    let Some((x, y)) =
        crate::bridge::client_missiles::free_point(w, at, GRIM_WARD_SIZE, GRIM_WARD_MASK, true)
    else {
        return Ok(0);
    };
    let [a, b, c] = row.cltmissile_abc.map(i32::from);
    let m = if class.large {
        c
    } else if class.small {
        b
    } else {
        a
    };
    if !valid_missile(inputs, m) {
        return Ok(0);
    }
    for class in [m, CORPSE_EXPLOSION] {
        let rec = CreateRecord {
            flags: flag::POSITION | flag::RANDOM_DIRECTION,
            owner: Some(key),
            class: class as u32,
            x,
            y,
            skill: i32::from(skill),
            level,
            owner_dir64: owner_dir64(w, key),
            ..CreateRecord::default()
        };
        create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?;
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn grim_ward_makes_the_corpse_sized_ward_and_the_explosion_at_a_soft_corpse() {
        use crate::bridge::world::MonsterClass;
        let fallen = UnitKey::new(MONSTER, 9);
        let (mut w, key) = world();
        let mut c = ClientUnit::new(fallen);
        c.class = 2;
        c.mode = 12;
        c.position = Some((5003, 5000));
        w.units.insert(fallen, c);
        let mut i = inputs(41);
        i.tables.skills[usize::from(FROST_NOVA)].cltmissile_abc = [1, 2, 3];
        i.tables.missiles = vec![ClientMissileRow::default(); 116];
        i.tables.monsters = vec![None; 3];
        i.tables.monsters[2] = Some(MonsterClass {
            small: true,
            ..MonsterClass::default()
        });
        // No target unit, or a target that is not `soft`: 0, no flag.
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 4).unwrap(), 0);
        w.units.get_mut(&key).unwrap().cast_target = Some(CastTarget::Unit(fallen));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 4).unwrap(), 0);
        assert!(!w.units[&key].flag_40);
        // A small soft corpse: `cltmissileb`, then `corpseexplosion`, both
        // at the corpse.
        i.tables.monsters[2].as_mut().unwrap().soft = true;
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 4).unwrap(), 1);
        assert!(w.units[&key].flag_40);
        let mut made: Vec<_> = w
            .objclient
            .set_c
            .values()
            .map(|u| (u.class, u.position))
            .collect();
        made.sort_unstable();
        assert_eq!(made, [(2, Some((5003, 5000))), (115, Some((5003, 5000)))]);
        assert!(missiles(&w)
            .iter()
            .all(|m| (m.skill, m.level) == (i32::from(FROST_NOVA), 4)));
        // Large → `cltmissilec`; neither → `cltmissilea`; an invalid one → 0.
        for (small, large, want) in [(false, true, 3), (false, false, 1)] {
            let m = i.tables.monsters[2].as_mut().unwrap();
            (m.small, m.large) = (small, large);
            w.objclient = Default::default();
            assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 4).unwrap(), 1);
            assert!(w.objclient.set_c.values().any(|u| u.class == want));
        }
        i.tables.skills[usize::from(FROST_NOVA)].cltmissile_abc[0] = 999;
        w.objclient = Default::default();
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 4).unwrap(), 0);
        assert!(w.objclient.set_c.is_empty());
    }
}

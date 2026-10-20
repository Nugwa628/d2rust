//! cltdofunc 30 (the curses): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// The client missile every curse do makes (`cursecenter`).
pub(super) const CURSE_CENTER: i32 = 191;

/// `cltdofunc` 30 (the curses):
/// c := `cltmissilea`, not a valid missile → 0. Flag 0x40 on U; T :=
/// [`cast_target_point`]; r := the skill's `aurarangecalc` − 3, at least
/// 2. One [`create_at`] T of class 191 (`cursecenter`); made → its light
/// is replaced by a kind 1, radius 1, red one (I 255, R 255) whose target
/// radius is r, its d04 := c and d06 := r, 1; else 0.
pub(super) fn curse(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let c = row.cltmissile_abc[0];
    if !valid_missile(inputs, i32::from(c)) {
        return Ok(0);
    }
    set_flag_40(w, key);
    let t = cast_target_point(w, key);
    let r = skill_calc(w, inputs, key, skill, level, |s| s.aurarangecalc)?
        .wrapping_sub(3)
        .max(2);
    let Some(k) = create_at(w, inputs, key, CURSE_CENTER, t, skill, level)? else {
        return Ok(0);
    };
    crate::bridge::client_missiles::replace_missile_light(
        w,
        k,
        LightKind::Plain,
        1,
        255,
        (255, 0, 0),
        Some(r),
    );
    if let Some(mm) = w.objclient.missiles.get_mut(&k) {
        mm.d04 = c;
        mm.d06 = r as i16;
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_curse_do_makes_a_cursecenter_with_a_red_light() {
        for (n, want) in [(9u8, 6i16), (3, 2)] {
            let (mut w, key) = aimed_at((5010, 5000));
            let mut i = inputs(30);
            i.skill_tables = Some(calc1_tables(n));
            i.tables.missiles.resize(192, ClientMissileRow::default());
            assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
            assert!(w.units[&key].flag_40);
            let ms = missiles(&w);
            assert_eq!(ms.len(), 1);
            assert_eq!((ms[0].d04, ms[0].d06), (CLASS as i16, want));
            assert_eq!(w.lights.len(), 1);
            let (id, _) = w.lights.iter().next().unwrap();
            assert_eq!(w.lights.radius(id), Some(1));
        }
    }

    #[test]
    fn the_curse_start_makes_its_cast_missile_toward_the_target() {
        use crate::bridge::use_state::client_start_apply;
        let (mut w, key) = aimed_at((5010, 5000));
        let mut i = inputs(30);
        i.tables.missiles.resize(192, ClientMissileRow::default());
        i.tables.skills[usize::from(FROST_NOVA)].cltstfunc = 18;
        // No `cltmissilec`: nothing.
        i.tables.skills[usize::from(FROST_NOVA)].cltmissile_abc = [CLASS as i16, -1, -1];
        client_start_apply(&mut w, &i, key, FROST_NOVA, 3).unwrap();
        assert!(missiles(&w).is_empty());
        // `cltmissilec` set: one create toward the target, no flag 0x40.
        i.tables.skills[usize::from(FROST_NOVA)].cltmissile_abc = [CLASS as i16, -1, 147];
        client_start_apply(&mut w, &i, key, FROST_NOVA, 3).unwrap();
        let ms = missiles(&w);
        assert_eq!(ms.len(), 1);
        assert_eq!((ms[0].skill, ms[0].level), (i32::from(FROST_NOVA), 3));
        assert!(!w.units[&key].flag_40);
    }
}

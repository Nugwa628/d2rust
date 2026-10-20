//! cltdofunc 22 (Lightning Strike): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 22 (Lightning Strike):
/// m := [`missile_class`]; invalid → 0. T := [`path_target_unit`];
/// none → 0. n := `calc1`; X := the next-GUID pick with (U,
/// pos(T), n, 3, T's GUID); none → 0. v := `calc2`; one create, record
/// flags 0x20, owner U, origin T, class m, target pos(X), skill, level;
/// a made missile's d28 := v. 1. No flag 0x40.
pub(super) fn lightning_strike(
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
    let Some(t) = path_target_unit(w, key) else {
        return Ok(0);
    };
    let Some((tx, ty)) = w.units.get(&t).map(|u| u.cell()) else {
        return Ok(0);
    };
    let n = calc1(w, inputs, key, skill, level)?;
    let env = crate::bridge::client_missiles::Env {
        rows: &inputs.tables.missiles,
        lights: inputs.high_light_quality,
        skills: inputs.skill_tables.as_deref(),
        monsters: &inputs.tables.monsters,
        model: Some(inputs),
    };
    let at = (i32::from(tx), i32::from(ty));
    let Some(x) = crate::bridge::client_missiles::next_guid(w, &env, key, at, n, 3, t.guid)? else {
        return Ok(0);
    };
    let Some((xx, xy)) = w.units.get(&x).map(|u| u.cell()) else {
        return Ok(0);
    };
    let v = calc2(w, inputs, key, skill, level)?;
    let rec = CreateRecord {
        flags: flag::TARGET_ABSOLUTE,
        owner: Some(key),
        origin: Some(t),
        class: m as u32,
        tx: i32::from(xx),
        ty: i32::from(xy),
        skill: i32::from(skill),
        level,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    if let Some(k) = create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)? {
        if let Some(mm) = w.objclient.missiles.get_mut(&k) {
            mm.d28 = v;
        }
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_lightning_strike_do_needs_a_target_unit_and_a_pick() {
        // A point target: 0, no flag 0x40.
        let (mut w, key) = aimed_at((5010, 5000));
        let mut i = inputs(22);
        i.skill_tables = Some(calc1_tables(5));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 0);
        assert!(!w.units[&key].flag_40 && missiles(&w).is_empty());
        // A target unit but no room to search: no pick, 0, no flag 0x40.
        let (mut w, key, _) = aimed_at_unit();
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 0);
        assert!(!w.units[&key].flag_40 && missiles(&w).is_empty());
    }
}

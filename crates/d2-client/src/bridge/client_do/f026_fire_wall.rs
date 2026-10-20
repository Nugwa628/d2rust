//! cltdofunc 26 (Fire Wall): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 26 (Fire Wall):
/// a := `cltmissilea`, next := a's server `SubMissile1`, both valid
/// missiles, else 0. T := [`cast_target_point`]; [`target_free`] (U, a)
/// and U in a room, else 0. A skill without `InTown` whose T lies in a
/// town room (from U's room) → 0. Record flags
/// 0x21, owner U, class a, start T, skill, level; with (dx, dy) := U − T
/// two creates toward (tx − dy, ty + dx) and (tx + dy, ty − dx), then
/// flags 1 and class next, one more; 1.
pub(super) fn fire_wall(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let a = i32::from(row.cltmissile_abc[0]);
    if !valid_missile(inputs, a) {
        return Ok(0);
    }
    let next = i32::from(inputs.tables.missiles[a as usize].sub1_server);
    if !valid_missile(inputs, next) {
        return Ok(0);
    }
    let (tx, ty) = cast_target_point(w, key);
    if !target_free(w, inputs, key, a) {
        return Ok(0);
    }
    let Some((ux, uy)) = w.units.get(&key).map(|u| u.cell()) else {
        return Ok(0);
    };
    let Some(home) = w.room_at(ux, uy) else {
        return Ok(0);
    };
    if row.flags & crate::controls::click::skill_flag::IN_TOWN == 0 {
        let town = match (u16::try_from(tx), u16::try_from(ty)) {
            (Ok(x), Ok(y)) => w
                .room_from(Some(&home), x, y)
                .is_some_and(|r| d2_sim::drlg::is_town(u32::from(r.level))),
            _ => false,
        };
        if town {
            return Ok(0);
        }
    }
    let (dx, dy) = (i32::from(ux) - tx, i32::from(uy) - ty);
    let mut rec = CreateRecord {
        flags: flag::POSITION | flag::TARGET_ABSOLUTE,
        owner: Some(key),
        class: a as u32,
        x: tx,
        y: ty,
        tx: tx - dy,
        ty: ty + dx,
        skill: i32::from(skill),
        level,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    let rows = &inputs.tables.missiles;
    create(w, rows, &rec, inputs.high_light_quality)?;
    (rec.tx, rec.ty) = (tx + dy, ty - dx);
    create(w, rows, &rec, inputs.high_light_quality)?;
    rec.flags = flag::POSITION;
    rec.class = next as u32;
    (rec.tx, rec.ty) = (0, 0);
    create(w, rows, &rec, inputs.high_light_quality)?;
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_fire_wall_do_needs_a_room_and_a_valid_sub_missile() {
        let (mut w, key) = aimed_at((5010, 5000));
        let mut i = inputs(26);
        i.tables.missiles[CLASS as usize].sub1_server = -1;
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 0);
        i.tables.missiles[CLASS as usize].sub1_server = 2;
        // No room for the caster: 0, flag 0x40 untouched.
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 0);
        assert!(!w.units[&key].flag_40 && missiles(&w).is_empty());
    }
}

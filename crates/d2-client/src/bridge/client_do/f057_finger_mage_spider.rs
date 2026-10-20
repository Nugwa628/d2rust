//! cltdofunc 57 (Finger Mage Spider): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// The target point of U's path: a
/// monster's client path, a player's cast point; none → (0, 0).
pub(super) fn path_target_point(w: &ClientWorld, key: UnitKey) -> (i32, i32) {
    let p = if key.unit_type == MONSTER {
        crate::bridge::critter_path::target(w, crate::bridge::critter_path::Who::server(key))
    } else {
        match w.units.get(&key).and_then(|u| u.cast_target) {
            Some(CastTarget::Point(x, y)) => Some((x, y)),
            _ => None,
        }
    };
    p.map_or((0, 0), |(x, y)| (i32::from(x), i32::from(y)))
}

/// `cltdofunc` 57 (Finger Mage Spider):
/// m := `cltmissilea`; invalid → 0. Flag 0x40 on U; a record
/// with flags 3, owner and origin U, class m, start U's cell, skill,
/// level, loops field := `calc1`. With U's path target unit T: U turns
/// toward T's cell and the relative target := T's cell − U;
/// else the path's target point − U. A made missile's d28 := level; 1.
pub(super) fn finger_mage_spider(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let m = i32::from(row.cltmissile_abc[0]);
    if !valid_missile(inputs, m) {
        return Ok(0);
    }
    set_flag_40(w, key);
    let loops = calc1(w, inputs, key, skill, level)?;
    let Some((x, y)) = w.units.get(&key).map(|u| u.cell()) else {
        return Ok(0);
    };
    let (x, y) = (i32::from(x), i32::from(y));
    let (tx, ty) = match path_target_unit(w, key) {
        Some(t) => {
            if key.unit_type == MONSTER {
                crate::bridge::critter_path::face_target(w, key);
            }
            crate::bridge::critter_path::unit_cell(w, t)
                .map_or((0, 0), |(cx, cy)| (i32::from(cx), i32::from(cy)))
        }
        None => path_target_point(w, key),
    };
    let rec = CreateRecord {
        flags: flag::POSITION | flag::TARGET_RELATIVE,
        owner: Some(key),
        origin: Some(key),
        class: m as u32,
        x,
        y,
        tx: tx - x,
        ty: ty - y,
        skill: i32::from(skill),
        level,
        loops,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    if let Some(k) = create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)? {
        if let Some(mm) = w.objclient.missiles.get_mut(&k) {
            mm.d28 = level;
        }
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_finger_mage_spider_do_sends_one_missile_at_the_path_point_with_d28_level() {
        let (mut w, key) = aimed_at((5010, 5004));
        let mut i = inputs(57);
        i.skill_tables = Some(calc1_tables(2));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        assert!(w.units[&key].flag_40);
        let ms = missiles(&w);
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].target_point, (5010, 5004));
        assert_eq!(ms[0].d28, 3);
    }
}

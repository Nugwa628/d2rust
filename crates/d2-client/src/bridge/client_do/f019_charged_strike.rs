//! cltdofunc 19 (Charged Strike): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 19 (Charged Strike):
/// T := [`path_target_unit`]; none → 0. m := [`missile_class`],
/// invalid → 0. n := the skill's `calc1`. A record with flags 0x21,
/// owner U, class m, start T's cell, target 2·T − U (the point past T
/// mirrored from U), skill, level and the callback
/// ([`crate::bridge::client_missiles::bolt_init`]) with i = 0 … n − 1, one
/// create each; 1. No flag 0x40.
pub(super) fn charged_strike(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let Some(t) = path_target_unit(w, key) else {
        return Ok(0);
    };
    let m = missile_class(w, key, &row);
    if !valid_missile(inputs, m) {
        return Ok(0);
    }
    let cell = |k: UnitKey| {
        w.units
            .get(&k)
            .map_or((0, 0), |u| (i32::from(u.cell().0), i32::from(u.cell().1)))
    };
    let ((x, y), (ux, uy)) = (cell(t), cell(key));
    let n = calc1(w, inputs, key, skill, level)?;
    let rec = CreateRecord {
        flags: flag::POSITION | flag::TARGET_ABSOLUTE,
        owner: Some(key),
        class: m as u32,
        x,
        y,
        tx: 2 * x - ux,
        ty: 2 * y - uy,
        skill: i32::from(skill),
        level,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    for i in 0..n {
        if let Some(k) = create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)? {
            crate::bridge::client_missiles::bolt_init(w, k, i);
        }
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_charged_strike_do_sends_calc1_bolts_from_the_target_onward() {
        let (mut w, key, _) = aimed_at_cow();
        let mut i = inputs(19);
        i.skill_tables = Some(calc1_tables(2));
        i.tables.missiles[CLASS as usize].range = 98;
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        assert!(!w.units[&key].flag_40);
        let ms = missiles(&w);
        assert_eq!(ms.len(), 2);
        for m in ms {
            let b = m.bolt.expect("a bolt path");
            assert_eq!(b.points[0], (5004, 5002));
            assert_eq!(m.target_point, (5008, 5004));
        }
    }
}

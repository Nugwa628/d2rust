//! cltdofunc 58 (Diablo's fire wall): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 58 (DiabWall): m
/// := `cltmissilea`; invalid → 0. Flag 0x40 on U; n := `calc1`; a record
/// with flags 0x21, owner U, class m, start U's cell, target
/// [`cast_target_point`] (a zero coordinate → 0), level (no skill) and
/// the callback ([`crate::bridge::client_missiles::wall_init`])
/// with i = 0 … n − 1, one create each; 1.
pub(super) fn diablo_wall(
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
    let n = calc1(w, inputs, key, skill, level)?;
    let Some((x, y)) = w.units.get(&key).map(|u| u.cell()) else {
        return Ok(0);
    };
    let (tx, ty) = cast_target_point(w, key);
    if tx == 0 || ty == 0 {
        return Ok(0);
    }
    let rec = CreateRecord {
        flags: flag::POSITION | flag::TARGET_ABSOLUTE,
        owner: Some(key),
        class: m as u32,
        x: i32::from(x),
        y: i32::from(y),
        tx,
        ty,
        level,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    for i in 0..n {
        if let Some(k) = create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)? {
            crate::bridge::client_missiles::wall_init(w, k, i);
        }
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_diablo_wall_do_makes_calc1_gated_bolts() {
        let (mut w, key) = aimed_at((5010, 5000));
        let mut i = inputs(58);
        i.skill_tables = Some(calc1_tables(12));
        i.tables.missiles[CLASS as usize].range = 98;
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        assert!(w.units[&key].flag_40);
        let ms = missiles(&w);
        assert_eq!(ms.len(), 12);
        // Each seed is init_low(5010 + i); its first step decides the gate.
        let mut gated = 0;
        for k in 0..12u32 {
            let mut s = d2_sim::rng::Seed::init_low(5010 + k);
            gated += u32::from(s.step() % 100 < 20);
        }
        let bolts = ms.iter().filter(|m| m.bolt.is_some()).count() as u32;
        assert_eq!(bolts, 12 - gated);
        assert!(ms.iter().all(|m| (m.total, m.current) == (77, 77)));
    }
}

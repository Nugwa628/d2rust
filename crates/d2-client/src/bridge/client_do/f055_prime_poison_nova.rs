//! cltdofunc 55 (Prime Poison Nova): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// The 16 relative targets of function 55 (X,
/// Y).
pub(super) const POISON_X: [i32; 16] = [0, 1, 2, 2, 2, 2, 2, 1, 0, -1, -2, -2, -2, -2, -2, -1];

pub(super) const POISON_Y: [i32; 16] = [2, 2, 2, 1, 0, -1, -2, -2, -2, -2, -2, -1, 0, 1, 2, 2];

/// `cltdofunc` 55 (Prime Poison Nova):
/// m := `cltmissilea`; invalid → 0. Flag 0x40 on U; a record with
/// flags 0x1F, owner and origin U, class m, start U's cell, level (no
/// skill), loops := `calc1`, velocity := m's `Param1` · 64; n := `calc2`,
/// at least 1. Creates toward the even offsets of the 16, then velocity
/// := `Param2` · 64 and, n > 1, toward the offsets 1 + k · n below 16; 1.
pub(super) fn prime_poison_nova(
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
    let param = inputs.tables.missiles[m as usize].param;
    set_flag_40(w, key);
    let loops = calc1(w, inputs, key, skill, level)?;
    let n = calc2(w, inputs, key, skill, level)?.max(1);
    let Some((x, y)) = w.units.get(&key).map(|u| u.cell()) else {
        return Ok(0);
    };
    let mut rec = CreateRecord {
        flags: flag::POSITION
            | flag::TARGET_RELATIVE
            | flag::VELOCITY
            | flag::LOOPS
            | flag::VELOCITY_FIXED,
        owner: Some(key),
        origin: Some(key),
        class: m as u32,
        x: i32::from(x),
        y: i32::from(y),
        level,
        loops,
        velocity: param[0].wrapping_shl(6),
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    for i in (0..16).step_by(2) {
        (rec.tx, rec.ty) = (POISON_X[i], POISON_Y[i]);
        create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?;
    }
    rec.velocity = param[1].wrapping_shl(6);
    if n > 1 {
        let mut k = 0;
        while k < 15 {
            let i = (1 + k) as usize;
            (rec.tx, rec.ty) = (POISON_X[i], POISON_Y[i]);
            create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?;
            k += n;
        }
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_prime_poison_nova_do_makes_the_even_ring_then_every_calc2_odd_one() {
        let (mut w, key) = world();
        let mut i = inputs(55);
        i.tables.missiles[CLASS as usize].param = [2, 3];
        let (cx, cy) = w.units[&key].cell();
        let (cx, cy) = (i32::from(cx), i32::from(cy));
        let targets = |w: &ClientWorld| {
            let mut t: Vec<_> = missiles(w)
                .iter()
                .map(|m| (m.target_point.0 - cx, m.target_point.1 - cy))
                .collect();
            t.sort_unstable();
            t
        };
        let ring = |idx: &mut dyn Iterator<Item = usize>| {
            let mut t: Vec<_> = idx.map(|k| (POISON_X[k], POISON_Y[k])).collect();
            t.sort_unstable();
            t
        };
        // calc2 = 2: the 8 even offsets, then 1, 3, …, 15.
        i.skill_tables = Some(calc12_tables(4, 2));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        assert!(w.units[&key].flag_40);
        assert_eq!(targets(&w), ring(&mut (0..16)));
        assert!(missiles(&w).iter().all(|m| m.owner == Some(key)));
        // calc2 = 1: the even offsets only.
        let (mut w, key) = world();
        i.skill_tables = Some(calc12_tables(4, 1));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        assert_eq!(targets(&w), ring(&mut (0..16).step_by(2)));
        // calc2 = 5: the odd offsets 1, 6, 11.
        let (mut w, key) = world();
        i.skill_tables = Some(calc12_tables(4, 5));
        client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap();
        let mut want: Vec<usize> = (0..16).step_by(2).collect();
        want.extend([1, 6, 11]);
        assert_eq!(targets(&w), ring(&mut want.into_iter()));
    }
}

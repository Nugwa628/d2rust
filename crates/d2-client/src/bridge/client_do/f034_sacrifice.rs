//! cltdofunc 34 (Sacrifice): one function of the client skill do table
//! (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 34 (Sacrifice, skill 96):
/// m := the row's `cltmissilea` (+0xEA); outside the missile
/// table → 0. Flag 0x40 on U. T := [`path_target_unit`]; none → 0. d :=
/// U's direction / 8 while U's path velocity (+0x7C) ≠ 0 (a player:
/// always), else a roll of 8; i := (roll 3 − 1 + (d + 4) mod 8) & 7: one of the three cells
/// behind U's facing. v := `cltcalc1`; v > 0 → m += roll v. One
/// [`create_toward`] U's cell + the step i, and 1.
pub(super) fn sacrifice(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    // The steps: x and y.
    const DX: [i32; 8] = [0, -1, -2, -1, 0, 1, 2, 1];
    const DY: [i32; 8] = [2, 1, 0, -1, -2, -1, 0, 1];
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let mut m = i32::from(row.cltmissile_abc[0]);
    if !valid_missile(inputs, m) {
        return Ok(0);
    }
    set_flag_40(w, key);
    if path_target_unit(w, key).is_none() {
        return Ok(0);
    }
    // A player's path velocity is never 0: the path allocation
    // sets 0x800 and every later write is a positive rate.
    let moving = key.unit_type == PLAYER
        || crate::bridge::critter_path::velocity(w, crate::bridge::critter_path::Who::server(key))
            .is_some_and(|v| v != 0);
    let d = if moving {
        u32::from(owner_dir64(w, key).unwrap_or(0)) >> 3
    } else {
        unit_roll(w, key, 8)
    };
    let r = unit_roll(w, key, 3);
    let i = (r.wrapping_sub(1).wrapping_add((d + 4) & 7) & 7) as usize;
    let Some((ux, uy)) = w.units.get(&key).map(|u| u.cell()) else {
        return Ok(0);
    };
    let at = (i32::from(ux) + DX[i], i32::from(uy) + DY[i]);
    let v = cltcalc1(w, inputs, key, skill, level)?;
    if v > 0 {
        m = m.wrapping_add(unit_roll(w, key, v) as i32);
    }
    create_toward(w, inputs, key, m, at, skill, level)?;
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_sacrifice_do_makes_its_missile_behind_u_by_two_rolls() {
        let (mut w, key, _) = aimed_at_unit();
        w.units.get_mut(&key).unwrap().seed = Some((1234, 666));
        // A player: its path velocity is never 0, so d := its direction
        // (0) / 8 with no roll; then r := roll 3, on U's seed.
        // `cltcalc1` (always evaluated) is the constant 0 here.
        let mut inp = inputs(34);
        inp.skill_tables = Some(calc1_tables(0));
        let mut seed = d2_sim::rng::Seed::new(1234, 666);
        let d = 0;
        let r = seed.roll(3);
        let i = (r.wrapping_sub(1).wrapping_add((d + 4) & 7) & 7) as usize;
        let dx = [0, -1, -2, -1, 0, 1, 2, 1][i];
        let dy = [2, 1, 0, -1, -2, -1, 0, 1][i];
        assert_eq!(client_do(&mut w, &inp, key, FROST_NOVA, 3).unwrap(), 1);
        assert!(w.units[&key].flag_40);
        let ms = missiles(&w);
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].target_point, (5000 + dx, 5000 + dy));
        assert_eq!(w.units[&key].seed, Some((seed.lo, seed.hi)));
        // No target unit: flag 0x40 set, no roll, nothing made.
        let (mut w, key) = aimed_at((5010, 5000));
        w.units.get_mut(&key).unwrap().seed = Some((1234, 666));
        assert_eq!(
            client_do(&mut w, &inputs(34), key, FROST_NOVA, 3).unwrap(),
            0
        );
        assert!(w.units[&key].flag_40 && missiles(&w).is_empty());
        assert_eq!(w.units[&key].seed, Some((1234, 666)));
    }
}

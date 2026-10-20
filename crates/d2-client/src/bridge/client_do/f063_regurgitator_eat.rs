//! cltdofunc 63 (Regurgitator Eat): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 63 (Regurgitator Eat):
/// a := `cltmissilea`; invalid → 0. Flag 0x40 on U; a record with
/// flags 0x2021, owner and origin U, start U's cell, skill, level: the
/// [`spray`] of a (2, 1), then of b := `cltmissileb` (4, 5) when valid; 1.
pub(super) fn regurgitator_eat(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let [a, b, _] = row.cltmissile_abc.map(i32::from);
    if !valid_missile(inputs, a) {
        return Ok(0);
    }
    set_flag_40(w, key);
    let Some((x, y)) = w.units.get(&key).map(|u| u.cell()) else {
        return Ok(0);
    };
    let mut rec = CreateRecord {
        flags: flag::POSITION | flag::TARGET_ABSOLUTE | flag::RANDOM_DIRECTION,
        owner: Some(key),
        origin: Some(key),
        class: a as u32,
        x: i32::from(x),
        y: i32::from(y),
        skill: i32::from(skill),
        level,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    spray(w, inputs, key, &mut rec, 2, 1)?;
    if valid_missile(inputs, b) {
        rec.class = b as u32;
        spray(w, inputs, key, &mut rec, 4, 5)?;
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_regurgitator_do_sprays_one_a_and_five_b_toward_the_corners() {
        let (mut w, key) = world();
        w.units.get_mut(&key).unwrap().seed = Some((0x1234, 0x5678));
        let mut i = inputs(63);
        i.tables.skills[usize::from(FROST_NOVA)].cltmissile_abc = [CLASS as i16, 2, -1];
        i.tables.missiles[2] = i.tables.missiles[CLASS as usize];
        let (cx, cy) = w.units[&key].cell();
        let (cx, cy) = (i32::from(cx), i32::from(cy));
        // The same draws as the spray: q, then two rolls per create.
        let mut s = d2_sim::rng::Seed::new(0x1234, 0x5678);
        // The spray writes its starts into the record: the second spray
        // reads the first one's last start.
        let mut want = Vec::new();
        let (mut x, mut y) = (cx, cy);
        for (spread, n) in [(2, 1), (4, 5)] {
            let mut q = s.step() & 3;
            let (bx, by) = (x, y);
            for _ in 0..n {
                let c = [(-16, -16), (-16, 16), (16, 16), (16, -16)][q as usize];
                q = (q + 1) & 3;
                x = bx + s.roll(2 * spread) as i32 - spread;
                y = by + s.roll(2 * spread) as i32 - spread;
                want.push(((x + c.0, y + c.1), ((q << 4) & 63) as u8));
            }
        }
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        assert!(w.units[&key].flag_40);
        let mut got: Vec<_> = missiles(&w)
            .iter()
            .map(|m| (m.target_point, m.direction))
            .collect();
        got.sort_unstable();
        want.sort_unstable();
        assert_eq!(got, want);
    }
}

//! cltdofunc 61 (ArcaneTower): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 61 (ArcaneTower):
/// m := `cltmissilea`; outside the missile table → 0. Flag 0x40
/// on U; no path target unit ([`path_target_unit`]) → 0. v := m's
/// `Vel` plus `VelLev` · level / 8; the 64 creates of the ring
/// (record flags 7: position, relative target, velocity v; owner U,
/// class m, start U's cell, skill, level; targets the ring offsets, the
/// client's tables); 1. Each made missile's
/// draw-light parameters (…) are not in the
/// model (nothing in the unit sets reads them).
pub(super) fn arcane_tower(
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
    let Some(mrow) = usize::try_from(m)
        .ok()
        .and_then(|i| inputs.tables.missiles.get(i))
    else {
        return Ok(0);
    };
    let v = mrow.vel + mrow.vel_lev * level / 8;
    set_flag_40(w, key);
    if path_target_unit(w, key).is_none() {
        return Ok(0);
    }
    let Some((x, y)) = w.units.get(&key).map(|u| u.cell()) else {
        return Ok(0);
    };
    let mut rec = CreateRecord {
        flags: flag::POSITION | flag::TARGET_RELATIVE | flag::VELOCITY,
        owner: Some(key),
        class: m as u32,
        x: i32::from(x),
        y: i32::from(y),
        velocity: v,
        skill: i32::from(skill),
        level,
        ..CreateRecord::default()
    };
    for i in 0..64 {
        (rec.tx, rec.ty) = d2_sim::skills::use_::bodies::ring_offset(i);
        create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?;
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_arcane_tower_do_needs_a_target_unit_then_makes_the_ring() {
        let (mut w, key) = world();
        assert_eq!(
            client_do(&mut w, &inputs(61), key, FROST_NOVA, 3).unwrap(),
            0
        );
        assert!(w.units[&key].flag_40);
        assert!(missiles(&w).is_empty());
        let t = UnitKey::new(MONSTER, 9);
        let mut tu = ClientUnit::new(t);
        tu.position = Some((5030, 5000));
        w.units.insert(t, tu);
        w.units.get_mut(&key).unwrap().cast_target = Some(CastTarget::Unit(t));
        assert_eq!(
            client_do(&mut w, &inputs(61), key, FROST_NOVA, 3).unwrap(),
            1
        );
        let ms = missiles(&w);
        assert_eq!(ms.len(), 64);
        let mut targets: Vec<_> = ms.iter().map(|m| m.target_point).collect();
        let mut want: Vec<_> = (0..64)
            .map(|i| {
                let (dx, dy) = d2_sim::skills::use_::bodies::ring_offset(i);
                (5000 + dx, 5000 + dy)
            })
            .collect();
        targets.sort_unstable();
        want.sort_unstable();
        assert_eq!(targets, want);
        assert!(ms.iter().all(|m| m.velocity == (9 << 8) * 75 / 100));
    }
}

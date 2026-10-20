//! cltdofunc 25 (Nova, Frost Nova): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 25: class m :=
/// the skill's progressive missile; m outside the missile table → 0. Else flag
/// 0x40 on U; v := m's `Vel` + `VelLev` · level / 8 +
/// the `cltcalc1` value; the ring with (U, U, m, −1, skill, level,
/// v); 1.
pub(super) fn nova(
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
    let rows = &inputs.tables.missiles;
    let Some(mrow) = usize::try_from(m).ok().and_then(|i| rows.get(i)) else {
        return Ok(0);
    };
    set_flag_40(w, key);
    let mut speed = mrow.vel + mrow.vel_lev * level / 8;
    if row.cltcalc1 != u32::MAX {
        speed = speed.wrapping_add(cltcalc1(w, inputs, key, skill, level)?);
    }
    ring(w, inputs, key, m as u32, skill, level, speed)?;
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_nova_do_makes_the_64_missile_ring_and_sets_flag_40() {
        let (mut w, key) = world();
        assert_eq!(
            client_do(&mut w, &inputs(25), key, FROST_NOVA, 3).unwrap(),
            1
        );
        assert!(w.units[&key].flag_40);
        let ms = missiles(&w);
        assert_eq!(ms.len(), 64);
        assert!(w.objclient.set_c.iter().all(|(k, u)| k.unit_type == MISSILE
            && u.class == CLASS
            && u.position == Some((5000, 5000))));
        // Targets: the caster's position + the ring offsets, in create order.
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
        // v = 8 + 4 · 3 / 8 = 9 (flag 4, shifted by the create), × 75 / 100.
        assert!(ms.iter().all(|m| m.velocity == (9 << 8) * 75 / 100));
        assert!(ms
            .iter()
            .all(|m| (m.skill, m.level) == (i32::from(FROST_NOVA), 3)));
    }

    #[test]
    fn the_nova_do_adds_the_cltcalc1_value_to_the_ring_velocity() {
        use d2_data::tables::{Missiles, Record, Skilldesc, Skills};
        fn blank<T: Record>() -> T {
            T::decode(&vec![0u8; T::SIZE])
        }
        let (mut w, key) = world();
        let mut i = inputs(25);
        // cltcalc1 at offset 0 of `skillscode`: the constant 5.
        i.tables.skills[usize::from(FROST_NOVA)].cltcalc1 = 0;
        let mut skills = vec![blank::<Skills>(); 50];
        skills[usize::from(FROST_NOVA)].cltcalc1 = 0;
        i.skill_tables = Some(Arc::new(d2_sim::skills::SkillTables {
            skills,
            skilldesc: vec![blank::<Skilldesc>()],
            missiles: vec![blank::<Missiles>()],
            skills_code: vec![0x07, 5, 0x00],
            miss_code: Vec::new(),
            level_cap: 99,
            stat_count: 359,
        }));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        // v = 8 + 4 · 3 / 8 + 5 = 14, × 75 / 100.
        let ms = missiles(&w);
        assert_eq!(ms.len(), 64);
        assert!(ms.iter().all(|m| m.velocity == (14 << 8) * 75 / 100));
    }
}

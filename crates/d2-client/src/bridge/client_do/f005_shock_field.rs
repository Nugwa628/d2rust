//! cltdofunc 5 (Shock Field): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 5 (Shock Field):
/// flag 0x40 on U; m := ([`missile_class`]), m < 0 → 0;
/// n := ([`prog_calc`]), n ≤ 0 → 0; r := the skill's
/// `aurarangecalc` (+0x64); the scatter with (U, m, n, r, skill,
/// level); 1.
pub(super) fn shock_field(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    set_flag_40(w, key);
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let m = missile_class(w, key, &row);
    if m < 0 {
        return Ok(0);
    }
    let t = inputs.skill_tables.as_deref().ok_or(HandlerError::Invalid(
        "client/client-do/f005-shock-field.md §1 r:shock-field: function 5 needs the skills tables",
    ))?;
    let Some(srow) = t.skills.get(usize::from(skill)) else {
        return Ok(0);
    };
    let n = prog_calc(w, inputs, key, skill, level)?;
    if n <= 0 {
        return Ok(0);
    }
    let r = {
        let mut units = crate::bridge::passive::ClientSkills::new(w, key);
        d2_sim::skills::levels::eval_skill(
            &mut units,
            t,
            Some(key),
            srow.aurarangecalc,
            i32::from(skill),
            level,
        )
    };
    scatter(w, inputs, key, m as u32, n, r, skill, level)?;
    Ok(1)
}

/// The scatter with (U, m, n, r, skill, level): the
/// target T := [`cast_target_point`]; U's seed := (lo T.x, hi 0x29A);
/// n times x := T.x + rnd(2r) − r, then y := T.y +
/// rnd(2r) − r on that seed; a point at squared distance ≥ 4 from U
/// gets a [`lob`] of m toward it.
#[allow(clippy::too_many_arguments)]
pub(super) fn scatter(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    m: u32,
    n: i32,
    r: i32,
    skill: u16,
    level: i32,
) -> Result<(), HandlerError> {
    let (tx, ty) = cast_target_point(w, key);
    let Some((ux, uy)) = w.units.get(&key).map(|u| u.cell()) else {
        return Ok(());
    };
    let (ux, uy) = (i32::from(ux), i32::from(uy));
    let mut seed = d2_sim::rng::Seed::new(tx as u32, 0x29A);
    for _ in 0..n {
        let x = (seed.roll(r.wrapping_mul(2)) as i32)
            .wrapping_sub(r)
            .wrapping_add(tx);
        let y = (seed.roll(r.wrapping_mul(2)) as i32)
            .wrapping_sub(r)
            .wrapping_add(ty);
        let (dx, dy) = (x - ux, y - uy);
        if dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) >= 4 {
            lob(w, inputs, key, m, (x, y), skill, level)?;
        }
    }
    if let Some(u) = w.units.get_mut(&key) {
        u.seed = Some((seed.lo, seed.hi));
        u.seed_sets = u.seed_sets.wrapping_add(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    /// The skill tables with code `[7, a, 0, 7, b, 0]`: calc offset 0
    /// gives a, offset 3 gives b.
    fn code_tables(
        f: impl FnOnce(&mut d2_data::tables::Skills),
        a: u8,
        b: u8,
    ) -> Arc<d2_sim::skills::SkillTables> {
        use d2_data::tables::{Missiles, Record, Skilldesc, Skills};
        fn blank<T: Record>() -> T {
            T::decode(&vec![0u8; T::SIZE])
        }
        let mut row: Skills = blank();
        f(&mut row);
        let mut skills = vec![blank::<Skills>(); 50];
        skills[usize::from(FROST_NOVA)] = row;
        Arc::new(d2_sim::skills::SkillTables {
            skills,
            skilldesc: vec![blank::<Skilldesc>()],
            missiles: vec![blank::<Missiles>()],
            skills_code: vec![0x07, a, 0x00, 0x07, b, 0x00],
            miss_code: Vec::new(),
            level_cap: 99,
            stat_count: 359,
        })
    }

    #[test]
    fn the_shock_field_do_lobs_a_seeded_scatter_around_the_target() {
        let (mut w, key) = world();
        w.units.get_mut(&key).unwrap().cast_target = Some(CastTarget::Point(5010, 5000));
        let mut i = inputs(5);
        // prgcalc1 at offset 0 → 4 missiles, aurarangecalc at 3 → r = 2.
        i.skill_tables = Some(code_tables(
            |r| (r.prgcalc1, r.aurarangecalc) = (0, 3),
            4,
            2,
        ));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        assert!(w.units[&key].flag_40);
        // The draws: U's seed (lo 5010, hi 0x29A), x then y, rnd(4) − 2.
        let mut seed = d2_sim::rng::Seed::new(5010, 0x29A);
        let mut want: Vec<_> = (0..4)
            .map(|_| {
                let x = 5010 + seed.roll(4) as i32 - 2;
                (x, 5000 + seed.roll(4) as i32 - 2)
            })
            .collect();
        assert_eq!(w.units[&key].seed, Some((seed.lo, seed.hi)));
        let mut got: Vec<_> = missiles(&w).iter().map(|m| m.target_point).collect();
        want.sort_unstable();
        got.sort_unstable();
        assert_eq!(got, want);
        // Lobs: arc 30 over frames from the distance, from U's position.
        assert!(w
            .objclient
            .set_c
            .values()
            .all(|u| u.position == Some((5000, 5000))));
    }

    #[test]
    fn the_scatter_skips_points_next_to_the_caster() {
        let (mut w, key) = world();
        // The target on the caster, r = 0: every point is U itself.
        w.units.get_mut(&key).unwrap().cast_target = Some(CastTarget::Point(5000, 5000));
        let mut i = inputs(5);
        i.skill_tables = Some(code_tables(
            |r| (r.prgcalc1, r.aurarangecalc) = (0, 3),
            3,
            0,
        ));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        assert!(missiles(&w).is_empty());
        // rnd(0) draws nothing: the seed is the one set.
        assert_eq!(w.units[&key].seed, Some((5000, 0x29A)));
    }
}

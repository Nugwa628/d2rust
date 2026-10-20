//! cltdofunc 17 (Teeth): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 17 (Teeth): flag
/// 0x40 on U; m := `cltmissilea`, or `cltmissileb` when U's hand class
/// is not 1 (bow) and b ≥ 0; m outside the missile table
/// → 0. n := the skill's `calc1`, a := its `calc2`. The
/// target T := [`cast_target_point`]; the step (sx, sy) := [`fan_step`]
/// of T − U. From T − (s · n) / 2 per axis, n client creates (record
/// flags 0x820: absolute target + activate frames a; owner and origin U,
/// class m, skill, level), the target moving by the step after each; 1.
///
/// The hand class is [`hand_class`]: a bow (1) fires `cltmissilea`
/// (Multiple Shot's arrows), a crossbow (7) `cltmissileb` (its bolts).
pub(super) fn spread(
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
    let [a, b, _] = row.cltmissile_abc.map(i32::from);
    let m = if hand_class(w, inputs, key) != 1 && b >= 0 {
        b
    } else {
        a
    };
    if m < 0 || m as usize >= inputs.tables.missiles.len() {
        return Ok(0);
    }
    let t = inputs.skill_tables.as_deref().ok_or(HandlerError::Invalid(
        "client/client-do/f017-teeth.md §1 r1: function 17 needs the skills tables",
    ))?;
    let Some(srow) = t.skills.get(usize::from(skill)) else {
        return Ok(0);
    };
    let (n, activate) = {
        let mut units = crate::bridge::passive::ClientSkills::new(w, key);
        let s = i32::from(skill);
        let n = d2_sim::skills::levels::eval_skill(&mut units, t, Some(key), srow.calc1, s, level);
        let a = d2_sim::skills::levels::eval_skill(&mut units, t, Some(key), srow.calc2, s, level);
        (n, a)
    };
    let Some((ux, uy)) = w.units.get(&key).map(|u| u.cell()) else {
        return Ok(0);
    };
    let (tx, ty) = cast_target_point(w, key);
    let (sx, sy) = fan_step(tx - i32::from(ux), ty - i32::from(uy));
    let mut rec = CreateRecord {
        flags: flag::TARGET_ABSOLUTE | flag::ACTIVATE,
        owner: Some(key),
        origin: Some(key),
        class: m as u32,
        tx: tx - sx.wrapping_mul(n) / 2,
        ty: ty - sy.wrapping_mul(n) / 2,
        skill: i32::from(skill),
        level,
        activate,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    for _ in 0..n.max(0) {
        create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?;
        rec.tx += sx;
        rec.ty += sy;
    }
    Ok(1)
}

/// The fan step of a direction (dx, dy): grows
/// a short one (|d|² < 4 → ×4, then |d|² < 16 → ×2); turns
/// it a quarter, (−dx, dy) halved per axis (truncating) while |·|² > 3,
/// and returns (dy', −dx').
pub(super) fn fan_step(mut dx: i32, mut dy: i32) -> (i32, i32) {
    let len = |x: i32, y: i32| x.wrapping_mul(x).wrapping_add(y.wrapping_mul(y));
    if len(dx, dy) < 4 {
        dx *= 4;
        dy *= 4;
    }
    if len(dx, dy) < 16 {
        dx *= 2;
        dy *= 2;
    }
    let (mut a, mut b) = (-dx, dy);
    while len(a, b) > 3 {
        a /= 2;
        b /= 2;
    }
    (b, a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_teeth_do_fans_calc1_missiles_across_the_target() {
        use d2_data::tables::{Missiles, Record, Skilldesc, Skills};
        fn blank<T: Record>() -> T {
            T::decode(&vec![0u8; T::SIZE])
        }
        let (mut w, key) = world();
        w.units.get_mut(&key).unwrap().cast_target = Some(CastTarget::Point(5010, 5000));
        let mut i = inputs(17);
        i.tables.skills[usize::from(FROST_NOVA)].cltmissile_abc = [-1, CLASS as i16, -1];
        let mut row: Skills = blank();
        // calc1 at offset 0 → 3 missiles, calc2 at offset 3 → 1 frame.
        (row.calc1, row.calc2) = (0, 3);
        let mut skills = vec![blank::<Skills>(); 50];
        skills[usize::from(FROST_NOVA)] = row;
        i.skill_tables = Some(Arc::new(d2_sim::skills::SkillTables {
            skills,
            skilldesc: vec![blank::<Skilldesc>()],
            missiles: vec![blank::<Missiles>()],
            skills_code: vec![0x07, 3, 0x00, 0x07, 1, 0x00],
            miss_code: Vec::new(),
            level_cap: 99,
            stat_count: 359,
        }));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        assert!(w.units[&key].flag_40);
        // Aim (10, 0) → step (0, −1); first target 5000 − (−3) / 2 = 5001.
        let mut targets: Vec<_> = missiles(&w).iter().map(|m| m.target_point).collect();
        targets.sort_unstable();
        assert_eq!(targets, [(5010, 4999), (5010, 5000), (5010, 5001)]);
        assert!(missiles(&w).iter().all(|m| m.total - m.activate == 1));

        // Multiple Shot: a bow (hand class 1) fires a; anything else b,
        // here outside the missile table.
        #[derive(Debug)]
        struct Bow(i32);
        impl PlayerAnims for Bow {
            fn anim(&self, _: &ClientWorld, _: UnitKey, _: u32) -> Option<PlayerAnim> {
                Some(PlayerAnim {
                    frames: 6,
                    speed: 256,
                    weapon: self.0,
                    events: None,
                })
            }
        }
        i.tables.skills[usize::from(FROST_NOVA)].cltmissile_abc = [CLASS as i16, 999, -1];
        for (weapon, done) in [(7, 0), (1, 1)] {
            let (mut w, key) = world();
            w.units.get_mut(&key).unwrap().cast_target = Some(CastTarget::Point(5010, 5000));
            i.player_anims = Some(Arc::new(Bow(weapon)));
            assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), done);
            assert_eq!(missiles(&w).len(), 3 * done as usize);
        }
    }

    #[test]
    fn the_fan_step_is_a_short_quarter_turn_of_the_aim() {
        assert_eq!(fan_step(10, 0), (0, -1));
        assert_eq!(fan_step(1, 0), (0, -1));
        assert_eq!(fan_step(1, 1), (1, -1));
        assert_eq!(fan_step(0, 0), (0, 0));
        assert_eq!(fan_step(-7, 3), (0, 1));
    }
}

//! cltdofunc 67 and 68 and 69 (Firestorm, Twister, Tornado): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// The arguments functions 67–69 hand the helper
/// ([`calc1_spread`]): the missiles' animation speed, whether the first
/// missile goes straight at the target, and whether the
/// rest face a random way and keep it (record flag 0x2000, path flag
/// 0x40).
pub(super) struct Spread {
    speed: i32,
    first_direct: bool,
    random_facing: bool,
}

/// Function 67 (Firestorm).
pub(super) const FIRESTORM: Spread = Spread {
    speed: 0x200,
    first_direct: true,
    random_facing: false,
};

/// Function 68 (Twister).
pub(super) const TWISTER: Spread = Spread {
    speed: 0x100,
    first_direct: false,
    random_facing: true,
};

/// Function 69 (Tornado).
pub(super) const TORNADO: Spread = Spread {
    speed: 0x200,
    first_direct: false,
    random_facing: true,
};

/// `cltdofunc` 67–69: m := [`missile_class`];
/// invalid → 0. Flag 0x40 on U; n := the skill's `calc1`, ≤ 0 → 0; then
/// the helper with (U, n, m, skill, level, speed, first_direct,
/// random_facing): T := [`cast_target_point`]. With first_direct, one
/// missile [`cast_toward`] T and n − 1 left. Then a record with flags
/// 0x21 (| 0x2000 with random_facing), owner U, class m, start U's cell,
/// target T, skill, level and the callback
/// ([`crate::bridge::client_missiles::bolt_init`]) with i = 0 … n − 1, one create
/// each. Every made missile (the direct one too) keeps its facing
/// when random_facing and takes the animation
/// speed (0 … 0x7FFF); 1.
pub(super) fn calc1_spread(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
    how: Spread,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let m = missile_class(w, key, &row);
    if !valid_missile(inputs, m) {
        return Ok(0);
    }
    set_flag_40(w, key);
    let mut n = calc1(w, inputs, key, skill, level)?;
    if n <= 0 {
        return Ok(0);
    }
    let made = |w: &mut ClientWorld, k: UnitKey| {
        if let Some(mis) = w.objclient.missiles.get_mut(&k) {
            mis.keep_facing |= how.random_facing;
            mis.anim_speed = how.speed.clamp(0, 0x7FFF);
        }
    };
    let (tx, ty) = cast_target_point(w, key);
    if how.first_direct {
        n -= 1;
        if let Some(k) = cast_toward(w, inputs, key, m, skill, level)? {
            made(w, k);
        }
    }
    let Some((x, y)) = w.units.get(&key).map(|u| u.cell()) else {
        return Ok(0);
    };
    let mut flags = flag::POSITION | flag::TARGET_ABSOLUTE;
    if how.random_facing {
        flags |= flag::RANDOM_DIRECTION;
    }
    let rec = CreateRecord {
        flags,
        owner: Some(key),
        class: m as u32,
        x: i32::from(x),
        y: i32::from(y),
        tx,
        ty,
        skill: i32::from(skill),
        level,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    for i in 0..n {
        if let Some(k) = create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)? {
            crate::bridge::client_missiles::bolt_init(w, k, i);
            made(w, k);
        }
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_twister_do_makes_calc1_bolts_that_keep_a_random_facing() {
        let (mut w, key) = aimed_at((5010, 5000));
        let mut i = inputs(68);
        i.skill_tables = Some(calc1_tables(3));
        i.tables.missiles[CLASS as usize].range = 20;
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        assert!(w.units[&key].flag_40);
        let ms = missiles(&w);
        assert_eq!(ms.len(), 3);
        for m in &ms {
            assert!(m.bolt.is_some() && m.keep_facing);
            assert_eq!(m.anim_speed, 0x100);
        }
    }

    #[test]
    fn the_firestorm_do_sends_one_straight_and_n_minus_1_bolts() {
        let (mut w, key) = aimed_at((5010, 5000));
        let mut i = inputs(67);
        i.skill_tables = Some(calc1_tables(3));
        i.tables.missiles[CLASS as usize].range = 20;
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        let ms = missiles(&w);
        assert_eq!(ms.len(), 3);
        assert_eq!(ms.iter().filter(|m| m.bolt.is_some()).count(), 2);
        assert!(ms.iter().all(|m| !m.keep_facing && m.anim_speed == 0x200));
        // calc1 0: nothing made, the do did not run.
        let (mut w, key) = aimed_at((5010, 5000));
        i.skill_tables = Some(calc1_tables(0));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 0);
        assert!(missiles(&w).is_empty());
    }
}

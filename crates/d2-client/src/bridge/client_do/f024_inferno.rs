//! cltdofunc 24 (Inferno): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 24
/// (Inferno): a := `cltmissilea`, not a valid missile → 0; b :=
/// `cltmissileb`, −1 when not valid. T := [`cast_target_point`] (always
/// found). With b ≥ 0, one roll of U's seed with (seed, 2) ≠ 0
/// takes b. n := the `calc1` value, at least 1. One create, record flags
/// 0x8020 (absolute target + range), owner and origin U, target T, skill,
/// level, range n; a made missile's animation speed (+0x4C)
/// := (its row's `AnimLen` · 256) / n, ≤ 0 → 0, ≥ 0x7FFF
/// → 0x7FFF. Then with state 12 on U: with a sequence the position :=
/// (position & 0xFF) + 0x800 (frame 8) and the frame count
/// +0x48 := 0x500; without one the frame +0x44 := (frame & ~0xFF) −
/// 0x200. Result: a missile was made. No flag 0x40.
pub(super) fn inferno(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    Ok(i32::from(
        inferno_core(w, inputs, key, skill, level)?.is_some(),
    ))
}

/// The Inferno create with (U, skill, level) itself (see [`inferno`]): the missile
/// made, if any. Also called by `cltdofunc` 77.
pub(super) fn inferno_core(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<Option<UnitKey>, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(None);
    };
    let mut m = i32::from(row.cltmissile_abc[0]);
    if !valid_missile(inputs, m) {
        return Ok(None);
    }
    let b = i32::from(row.cltmissile_abc[1]);
    let (tx, ty) = cast_target_point(w, key);
    if valid_missile(inputs, b) && unit_roll(w, key, 2) != 0 {
        m = b;
    }
    let n = calc1(w, inputs, key, skill, level)?.max(1);
    let rec = CreateRecord {
        flags: flag::TARGET_ABSOLUTE | flag::RANGE,
        owner: Some(key),
        origin: Some(key),
        class: m as u32,
        tx,
        ty,
        skill: i32::from(skill),
        level,
        range: n,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    let made = create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?;
    if let Some(k) = made {
        let len = i32::from(inputs.tables.missiles[m as usize].anim_len) << 8;
        if let Some(mis) = w.objclient.missiles.get_mut(&k) {
            mis.anim_speed = (len / n).clamp(0, 0x7FFF);
        }
    }
    if let Some(u) = w.units.get_mut(&key) {
        if u.states.contains(&crate::bridge::use_state::STATE_INFERNO) {
            if let Some(q) = u.sequence.as_mut() {
                q.pos = (q.pos & 0xFF) + 0x800;
                u.frame_count = 0x500;
            } else {
                u.frame = (u.frame & !0xFF) - 0x200;
            }
        }
    }
    Ok(made)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_inferno_do_makes_one_ranged_missile_and_rewinds_the_sequence() {
        let (mut w, key) = aimed_at((5010, 5000));
        let mut i = inputs(24);
        i.skill_tables = Some(calc1_tables(38));
        i.tables.missiles[CLASS as usize].anim_len = 15;
        // No state 12: one missile, range 38, speed 3840 / 38; frame kept;
        // no flag 0x40.
        w.units.get_mut(&key).unwrap().frame = 0x900;
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        let ms = missiles(&w);
        assert_eq!(ms.len(), 1);
        assert_eq!((ms[0].target_point, ms[0].anim_speed), ((5010, 5000), 101));
        assert_eq!(ms[0].total, 38);
        assert!(!w.units[&key].flag_40);
        assert_eq!(w.units[&key].frame, 0x900);
        // State 12 without a sequence: the frame steps back two frames.
        w.units.get_mut(&key).unwrap().states.insert(12);
        w.units.get_mut(&key).unwrap().frame = 0x980;
        client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap();
        assert_eq!(w.units[&key].frame, 0x700);
        // With a sequence: position 8 frames in, count 5 frames.
        w.units.get_mut(&key).unwrap().sequence = Some(crate::bridge::world::MonsterSequence {
            frames: Vec::new(),
            length: 0x1000,
            pos: 0xA40,
            step: 0x100,
            drawn_mode: 18,
        });
        client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap();
        let u = &w.units[&key];
        assert_eq!(
            (u.sequence.as_ref().unwrap().pos, u.frame_count),
            (0x840, 0x500)
        );
    }

    #[test]
    fn the_inferno_start_makes_a_timed_state_12_list_then_restarts_it() {
        use crate::bridge::use_state::client_start_apply;
        use crate::bridge::world::expire_timed_lists;
        let (mut w, key) = world();
        let mut i = inputs(24);
        i.tables.skills[usize::from(FROST_NOVA)].cltstfunc = 15;
        client_start_apply(&mut w, &i, key, FROST_NOVA, 1).unwrap();
        let u = &w.units[&key];
        assert!(u.states.contains(&12) && u.state_lists.contains_key(&12));
        assert_eq!(u.timed_lists.get(&12), Some(&21));
        // The restart: expire 7, the position 8 frames in, count 0x500.
        let u = w.units.get_mut(&key).unwrap();
        u.sequence = Some(crate::bridge::world::MonsterSequence {
            frames: Vec::new(),
            length: 0x1000,
            pos: 0x10,
            step: 0x100,
            drawn_mode: 18,
        });
        client_start_apply(&mut w, &i, key, FROST_NOVA, 1).unwrap();
        let u = w.units.get_mut(&key).unwrap();
        assert_eq!(u.timed_lists.get(&12), Some(&7));
        assert_eq!(
            (u.sequence.as_ref().unwrap().pos, u.frame_count),
            (0x810, 0x500)
        );
        // Seven unit updates free the list and clear the state.
        for _ in 0..6 {
            expire_timed_lists(u);
        }
        assert!(u.states.contains(&12));
        expire_timed_lists(u);
        assert!(!u.states.contains(&12) && !u.state_lists.contains_key(&12));
        assert!(u.timed_lists.is_empty());
    }
}

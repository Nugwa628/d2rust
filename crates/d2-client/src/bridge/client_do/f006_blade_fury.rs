//! cltdofunc 6 (Blade Fury): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 6 (Blade Fury):
/// m := [`missile_class`] < 0, no used entry E or n :=
/// [`prog_calc`] ≤ 0 → 0. Flag 0x40 off U. E's parameter 1 < 1: one
/// create, record flags 0x20 (absolute target), owner and origin U, class
/// m, target [`cast_target_point`], skill, level, and parameter 1 := n;
/// else parameter 1 − 1. With state 12 on U the sequence position :=
/// (position & 0xFF) + (`seqinput` − 1) · 256 and the
/// frame count +0x48 := 0x800. Result 1.
pub(super) fn blade_fury(
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
    if m < 0 || current_entry(w, key).is_none() {
        return Ok(0);
    }
    let n = prog_calc(w, inputs, key, skill, level)?;
    if n <= 0 {
        return Ok(0);
    }
    if let Some(u) = w.units.get_mut(&key) {
        u.flag_40 = false;
    }
    let (tx, ty) = cast_target_point(w, key);
    let delay = current_entry(w, key).map_or(0, |e| e.param);
    let next = if delay < 1 {
        let rec = CreateRecord {
            flags: flag::TARGET_ABSOLUTE,
            owner: Some(key),
            origin: Some(key),
            class: m as u32,
            tx,
            ty,
            skill: i32::from(skill),
            level,
            owner_dir64: owner_dir64(w, key),
            ..CreateRecord::default()
        };
        create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?;
        n
    } else {
        delay - 1
    };
    if let Some(e) = current_entry_mut(w, key) {
        e.param = next;
    }
    if let Some(u) = w.units.get_mut(&key) {
        if u.states.contains(&crate::bridge::use_state::STATE_INFERNO) {
            if let Some(q) = u.sequence.as_mut() {
                q.pos = (q.pos & 0xFF) + (i32::from(row.seqinput) - 1) * 256;
            }
            u.frame_count = 0x800;
        }
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_blade_fury_do_fires_every_n_plus_one_calls_and_holds_the_sequence() {
        use crate::bridge::use_state::client_start_apply;
        let (mut w, key) = aimed_at((5010, 5000));
        let mut i = inputs(6);
        i.skill_tables = Some(calc1_tables(2));
        let r = &mut i.tables.skills[usize::from(FROST_NOVA)];
        (r.cltstfunc, r.seqinput) = (8, 12);
        // The start: a new state-12 list (expire 20), parameter 1 := 0.
        w.units
            .get_mut(&key)
            .unwrap()
            .skills
            .as_mut()
            .unwrap()
            .entries[0]
            .param = 5;
        client_start_apply(&mut w, &i, key, FROST_NOVA, 1).unwrap();
        let u = &w.units[&key];
        assert_eq!(u.timed_lists.get(&12), Some(&20));
        assert_eq!(u.skills.as_ref().unwrap().entries[0].param, 0);
        let u = w.units.get_mut(&key).unwrap();
        u.flag_40 = true;
        u.sequence = Some(crate::bridge::world::MonsterSequence {
            frames: Vec::new(),
            length: 0x1000,
            pos: 0x1234,
            step: 0x100,
            drawn_mode: 18,
        });
        // prgcalc1 = 2: a missile on calls 1 and 4, parameter 1 2 → 1 → 0.
        let mut made = Vec::new();
        for _ in 0..4 {
            assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
            let u = &w.units[&key];
            made.push((
                missiles(&w).len(),
                u.skills.as_ref().unwrap().entries[0].param,
            ));
            assert!(!u.flag_40);
            assert_eq!(
                (u.sequence.as_ref().unwrap().pos, u.frame_count),
                (0xB34, 0x800)
            );
        }
        assert_eq!(made, [(1, 2), (1, 1), (1, 0), (2, 2)]);
        assert!(missiles(&w).iter().all(|m| m.target_point == (5010, 5000)));
        // The restart with a list: expire 7, the same position and count.
        let u = w.units.get_mut(&key).unwrap();
        (u.sequence.as_mut().unwrap().pos, u.frame_count) = (0x77, 0);
        client_start_apply(&mut w, &i, key, FROST_NOVA, 1).unwrap();
        let u = &w.units[&key];
        assert_eq!(u.timed_lists.get(&12), Some(&7));
        assert_eq!(
            (u.sequence.as_ref().unwrap().pos, u.frame_count),
            (0xB77, 0x800)
        );
    }
}

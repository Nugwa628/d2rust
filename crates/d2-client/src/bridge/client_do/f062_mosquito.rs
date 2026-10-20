//! cltdofunc 62 (Mosquito): one function of the client skill do table
//! (`super::client_do`); the shared helpers live in `super`.

use super::*;
use crate::bridge::skills::{used_entry, used_entry_mut};

/// `cltdofunc` 62: E := U's used
/// entry (none → 0); T := the unit of type E +0x20, GUID E +0x1C; none
/// → the param := 0, 0. T in melee range of U and n := param − 1 > 0 →
/// the param := n, the sequence position := `Param1` · 256, the frame
/// count := (count & ~0xFF) + 0x100; 1. Else 0.
pub(super) fn mosquito(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
) -> Result<i32, HandlerError> {
    if inputs.tables.skills.get(usize::from(skill)).is_none() {
        return Ok(0);
    }
    let Some(e) = used_entry(w, key).copied() else {
        return Ok(0);
    };
    let target = u8::try_from(e.counter)
        .ok()
        .map(|ty| UnitKey::new(ty, e.param2 as u32))
        .filter(|t| w.units.contains_key(t));
    let Some(t) = target else {
        if let Some(e) = used_entry_mut(w, key) {
            e.param = 0;
        }
        return Ok(0);
    };
    let env = search_env(inputs);
    if !crate::bridge::client_missiles::in_melee_range(w, &env, key, t, 0)? {
        return Ok(0);
    }
    let n = e.param.wrapping_sub(1);
    if n <= 0 {
        return Ok(0);
    }
    let p1 = inputs
        .skill_tables
        .as_deref()
        .and_then(|t| t.skills.get(usize::from(skill)))
        .map(|r| r.param1 as i32)
        .ok_or(HandlerError::Invalid(
            "client/client-do/f062-mosquito.md §2 r5: the skill's Param1 needs the skills tables",
        ))?;
    if let Some(e) = used_entry_mut(w, key) {
        e.param = n;
    }
    if let Some(u) = w.units.get_mut(&key) {
        if let Some(q) = u.sequence.as_mut() {
            q.pos = (q.pos & 0xFF) + (p1 << 8);
        }
        u.frame_count = (u.frame_count & !0xFF) + 0x100;
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;
    use crate::bridge::world::MonsterSequence;
    use d2_data::tables::{Missiles, Skilldesc, Skills};

    fn blank<T: d2_data::tables::Record>() -> T {
        T::decode(&vec![0u8; T::SIZE])
    }

    /// A mosquito-like monster U (used entry of FROST_NOVA, param `n`,
    /// target the player P) `dx` cells east of P, at sequence position
    /// 16 with a frame count of 3.5 frames; Mosquito's `Param1` 12.
    fn mosquito(n: i32, dx: u16) -> (ClientWorld, ModelInputs, UnitKey) {
        let (mut w, p) = world();
        let key = UnitKey::new(MONSTER, 8);
        let mut u = w.units[&p].clone();
        u.key = key;
        u.mode = 14;
        u.position = Some((5000 + dx, 5000));
        u.frame = 16 << 8;
        u.frame_count = 0x380;
        u.used_skill = Some(i32::from(FROST_NOVA));
        let e = &mut u.skills.as_mut().unwrap().entries[0];
        (e.param, e.param2, e.counter) = (n, p.guid as i32, i32::from(PLAYER));
        u.sequence = Some(MonsterSequence {
            frames: Vec::new(),
            length: 20 << 8,
            pos: (16 << 8) | 0x40,
            step: 0x100,
            drawn_mode: 14,
        });
        w.units.insert(key, u);
        let mut i = inputs(62);
        let mut skills = vec![blank::<Skills>(); 50];
        skills[usize::from(FROST_NOVA)].param1 = 12;
        i.skill_tables = Some(Arc::new(d2_sim::skills::SkillTables {
            skills,
            skilldesc: vec![blank::<Skilldesc>()],
            missiles: vec![blank::<Missiles>()],
            skills_code: Vec::new(),
            miss_code: Vec::new(),
            level_cap: 99,
            stat_count: 359,
        }));
        (w, i, key)
    }

    fn state(w: &ClientWorld, key: UnitKey) -> (i32, i32, i32) {
        let u = &w.units[&key];
        (
            u.skills.as_ref().unwrap().entries[0].param,
            u.sequence.as_ref().unwrap().pos,
            u.frame_count,
        )
    }

    #[test]
    fn a_target_in_reach_loops_the_draw_back_to_param1() {
        let (mut w, i, key) = mosquito(5, 1);
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 1);
        assert_eq!(state(&w, key), (4, (12 << 8) | 0x40, 0x400));
    }

    #[test]
    fn the_last_repeat_lets_the_sequence_run_out() {
        let (mut w, i, key) = mosquito(1, 1);
        client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap();
        assert_eq!(state(&w, key), (1, (16 << 8) | 0x40, 0x380));
    }

    #[test]
    fn a_target_out_of_reach_keeps_the_count() {
        let (mut w, i, key) = mosquito(5, 6);
        client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap();
        assert_eq!(state(&w, key), (5, (16 << 8) | 0x40, 0x380));
    }

    #[test]
    fn no_target_clears_the_count() {
        let (mut w, i, key) = mosquito(5, 1);
        w.units
            .get_mut(&key)
            .unwrap()
            .skills
            .as_mut()
            .unwrap()
            .entries[0]
            .param2 = 99;
        client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap();
        assert_eq!(state(&w, key), (0, (16 << 8) | 0x40, 0x380));
    }
}

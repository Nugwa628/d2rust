//! cltdofunc 59 (DiabRun): one function of the client skill do table
//! (`super::client_do`); the shared helpers live in `super`.

use super::*;
use crate::bridge::skills::{used_entry, used_entry_mut};

/// `cltdofunc` 59:
/// P1…P6 the row's `Param1`…`Param6`. No row or no used entry → 0;
/// f := U's frame; f > P4 → 0. The path's target unit next to U (unit
/// distance < 2) or E-flag bit 1 → the end: E-flags := 0, the frame
/// count := P1 · 256, the sequence position := P2. Else f = P3 →
/// E-flags := 1 (the path writes are the server copy's); f = P4 → the
/// frame count := P5 · 256, the position := P6. 1.
pub(super) fn diablo_run(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
) -> Result<i32, HandlerError> {
    if inputs.tables.skills.get(usize::from(skill)).is_none() {
        return Ok(0);
    }
    let p = inputs
        .skill_tables
        .as_deref()
        .and_then(|t| t.skills.get(usize::from(skill)))
        .map(|r| [r.param1, r.param2, r.param3, r.param4, r.param5, r.param6].map(|v| v as i32))
        .ok_or(HandlerError::Invalid(
            "client/client-do/f059-diablo-run.md §1: the skill's Param1…Param6 need the skills tables",
        ))?;
    let [p1, p2, p3, p4, p5, p6] = p;
    let Some(flags) = used_entry(w, key).map(|e| e.flags) else {
        return Ok(0);
    };
    let Some(f) = w.units.get(&key).map(|u| u.frame >> 8) else {
        return Ok(0);
    };
    if f > p4 {
        return Ok(0);
    }
    let near = path_target_unit(w, key).is_some_and(|t| unit_distance(w, inputs, t, key) < 2);
    if near || flags & 2 != 0 {
        if let Some(e) = used_entry_mut(w, key) {
            e.flags = 0;
        }
        jump(w, key, p1, p2);
    } else if f == p3 {
        if let Some(e) = used_entry_mut(w, key) {
            e.flags = 1;
        }
    } else if f == p4 {
        jump(w, key, p5, p6);
    }
    Ok(1)
}

/// The frame count (+0x48) := `count` · 256 and the sequence position
/// (+0x38) := (position & 0xFF) + `pos` · 256.
fn jump(w: &mut ClientWorld, key: UnitKey, count: i32, pos: i32) {
    let Some(u) = w.units.get_mut(&key) else {
        return;
    };
    u.frame_count = count << 8;
    if let Some(q) = u.sequence.as_mut() {
        q.pos = (q.pos & 0xFF) + (pos << 8);
    }
}

/// The unit distance with (T, U) on the
/// cells, with the unit sizes (player 2, monster
/// `monstats2` `SizeX`, else 0); a missing unit: far.
fn unit_distance(w: &ClientWorld, inputs: &ModelInputs, a: UnitKey, b: UnitKey) -> i32 {
    let size = |u: &crate::bridge::world::ClientUnit| match u.key.unit_type {
        PLAYER => 2,
        MONSTER => inputs
            .tables
            .monsters
            .get(u.class as usize)
            .and_then(|r| r.as_ref())
            .map_or(0, |r| i32::from(r.size_x)),
        _ => 0,
    };
    match (w.units.get(&a), w.units.get(&b)) {
        (Some(ua), Some(ub)) => {
            super::super::objects::unit_distance_at(ua.cell(), size(ua), ub.cell(), size(ub))
        }
        _ => i32::MAX,
    }
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

    /// Diablo-like U (a monster with a used entry of FROST_NOVA, flags
    /// `flags`) at sequence position `frame`, DiabRun's parameters.
    fn diablo(frame: i32, flags: u32) -> (ClientWorld, ModelInputs, UnitKey) {
        let (mut w, p) = world();
        let key = UnitKey::new(MONSTER, 8);
        let mut u = w.units.remove(&p).unwrap();
        u.key = key;
        u.frame = frame << 8;
        u.frame_count = (22 - frame) << 8;
        u.used_skill = Some(i32::from(FROST_NOVA));
        u.skills.as_mut().unwrap().entries[0].flags = flags;
        u.sequence = Some(MonsterSequence {
            frames: Vec::new(),
            length: 22 << 8,
            pos: frame << 8,
            step: 0x100,
            drawn_mode: 14,
        });
        w.units.insert(key, u);
        let mut i = inputs(59);
        let mut skills = vec![blank::<Skills>(); 50];
        let r = &mut skills[usize::from(FROST_NOVA)];
        (r.param1, r.param2, r.param3, r.param4, r.param5, r.param6) = (8, 14, 5, 13, 16, 6);
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

    fn state(w: &ClientWorld, key: UnitKey) -> (u32, i32, i32) {
        let u = &w.units[&key];
        (
            u.skills.as_ref().unwrap().entries[0].flags,
            u.frame_count >> 8,
            u.sequence.as_ref().unwrap().pos >> 8,
        )
    }

    #[test]
    fn the_run_starts_at_frame_5_and_loops_frame_13_back_to_6() {
        let (mut w, i, key) = diablo(5, 0);
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 1);
        assert_eq!(state(&w, key), (1, 17, 5));
        let (mut w, i, key) = diablo(13, 1);
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 1);
        assert_eq!(state(&w, key), (1, 16, 6));
        let (mut w, i, key) = diablo(9, 1);
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 1);
        assert_eq!(state(&w, key), (1, 13, 9));
    }

    #[test]
    fn the_ended_path_jumps_to_the_closing_frames() {
        let (mut w, i, key) = diablo(9, 3);
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 1);
        assert_eq!(state(&w, key), (0, 8, 14));
    }

    #[test]
    fn past_the_loop_frame_or_without_an_entry_the_do_does_not_run() {
        let (mut w, i, key) = diablo(14, 3);
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 0);
        assert_eq!(state(&w, key), (3, 8, 14));
        let (mut w, i, key) = diablo(5, 0);
        w.units
            .get_mut(&key)
            .unwrap()
            .skills
            .as_mut()
            .unwrap()
            .current = None;
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 0);
    }
}

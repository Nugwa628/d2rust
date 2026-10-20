//! cltdofunc 64 (QueenDeath): one function of the client skill do table
//! (`super::client_do`); the shared helpers live in `super`.

use super::*;
use crate::bridge::skills::{used_entry, used_entry_mut};

/// `cltdofunc` 64: n := the used entry's
/// param (none: fatal); f := U's frame. f = 6 → the sequence position
/// := 7 and the frame count := 50 · 256; f = 22 → n ≥ 4: the position :=
/// 40 and the count := 17 · 256, else the param := n + 1; f = 30 → the
/// mode set 12. 1.
pub(super) fn queen_death(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
) -> Result<i32, HandlerError> {
    let n = used_entry(w, key)
        .map(|e| e.param)
        .ok_or(HandlerError::Invalid(
            "client/client-do/f064-queen-death.md §1 r1: QueenDeath without a used skill entry",
        ))?;
    let Some(f) = w.units.get(&key).map(|u| u.frame >> 8) else {
        return Ok(1);
    };
    match f {
        6 => jump(w, key, 7, 0x3200),
        22 if n >= 4 => jump(w, key, 40, 0x1100),
        22 => {
            if let Some(e) = used_entry_mut(w, key) {
                e.param = n + 1;
            }
        }
        // The flag +0xC4 |= 0x10000 has no model field.
        30 => crate::bridge::monster_anim::mode_set(w, inputs, key, 12),
        _ => {}
    }
    Ok(1)
}

/// The sequence position (+0x38) := (position & 0xFF) + `pos` · 256
/// and the frame count (+0x48) := `count`.
fn jump(w: &mut ClientWorld, key: UnitKey, pos: i32, count: i32) {
    let Some(u) = w.units.get_mut(&key) else {
        return;
    };
    if let Some(q) = u.sequence.as_mut() {
        q.pos = (q.pos & 0xFF) + (pos << 8);
    }
    u.frame_count = count;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;
    use crate::bridge::world::MonsterSequence;

    /// A monster with a used entry of FROST_NOVA (param `n`) in mode 14
    /// at frame `frame`, sequence position `frame`.
    fn queen(frame: i32, n: i32) -> (ClientWorld, ModelInputs, UnitKey) {
        let (mut w, p) = world();
        let key = UnitKey::new(MONSTER, 8);
        let mut u = w.units.remove(&p).unwrap();
        u.key = key;
        u.mode = 14;
        u.frame = frame << 8;
        u.frame_count = 20 << 8;
        u.used_skill = Some(i32::from(FROST_NOVA));
        u.skills.as_mut().unwrap().entries[0].param = n;
        u.sequence = Some(MonsterSequence {
            frames: Vec::new(),
            length: 54 << 8,
            pos: (frame << 8) | 0x20,
            step: 0x100,
            drawn_mode: 14,
        });
        w.units.insert(key, u);
        (w, inputs(64), key)
    }

    fn state(w: &ClientWorld, key: UnitKey) -> (i32, i32, i32) {
        let u = &w.units[&key];
        (
            u.skills.as_ref().unwrap().entries[0].param,
            u.sequence.as_ref().map_or(-1, |q| q.pos),
            u.frame_count,
        )
    }

    #[test]
    fn frame_6_skips_to_position_7_with_50_frames() {
        let (mut w, i, key) = queen(6, 0);
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 1);
        assert_eq!(state(&w, key), (0, 0x720, 0x3200));
    }

    #[test]
    fn frame_22_counts_to_4_then_jumps_to_position_40() {
        let (mut w, i, key) = queen(22, 3);
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 1);
        assert_eq!(state(&w, key), (4, 0x1620, 20 << 8));
        let (mut w, i, key) = queen(22, 4);
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 1);
        assert_eq!(state(&w, key), (4, 0x2820, 0x1100));
    }

    #[test]
    fn frame_30_sets_the_dead_mode() {
        let (mut w, i, key) = queen(30, 4);
        w.units.get_mut(&key).unwrap().flag_40 = true;
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 1);
        assert_eq!(w.units[&key].mode, 12);
        assert!(w.units[&key].sequence.is_none());
        let (mut w, i, key) = queen(12, 0);
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 1);
        assert_eq!(w.units[&key].mode, 14);
    }

    #[test]
    fn no_used_entry_is_an_error() {
        let (mut w, i, key) = queen(6, 0);
        w.units
            .get_mut(&key)
            .unwrap()
            .skills
            .as_mut()
            .unwrap()
            .current = None;
        assert!(client_do(&mut w, &i, key, FROST_NOVA, 1).is_err());
    }
}

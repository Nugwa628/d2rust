//! cltdofunc 21 (Fend, Zeal, Fury): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 21 (Fend 30, Zeal 106, Fury 248):
/// an invalid row or no used skill entry E → 0. r := U's melee
/// range + 4. T := the unit of (E +0x1C, E +0x20) when
/// U is in melee range of it, else the pick
/// with (U, 0, 0, r, 0x20003, E +0x20); none → 0. E +0x18 := n −
/// 1 (n := E +0x18). n − 1 > 0: N := the pick with g := T's GUID; none →
/// 0. U faces N, E +0x1C, +0x20 := N's type and GUID, and
/// the frame rewind with (U, Param2) (skills +0x14C) runs; the swing
/// sound (of U's weapon) is the audio model's. 1; no flag 0x40, no
/// missile.
pub(super) fn melee_repeat(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
) -> Result<i32, HandlerError> {
    if inputs.tables.skills.get(usize::from(skill)).is_none() {
        return Ok(0);
    }
    let Some(e) = w
        .units
        .get(&key)
        .and_then(|u| u.skills.as_ref())
        .and_then(|l| l.current.and_then(|i| l.entries.get(i)))
        .copied()
    else {
        return Ok(0);
    };
    let env = search_env(inputs);
    let r = reach(w, &env, key)? + 4;
    let stored = u8::try_from(e.param2)
        .ok()
        .map(|ty| UnitKey::new(ty, e.counter as u32))
        .filter(|k| w.units.contains_key(k));
    let near = match stored {
        Some(t) if crate::bridge::client_missiles::in_melee_range(w, &env, key, t, 0)? => Some(t),
        _ => None,
    };
    let target = match near {
        Some(t) => t,
        None => {
            let pick = crate::bridge::client_missiles::search_pick(
                w,
                &env,
                key,
                (0, 0),
                r,
                0x20003,
                e.counter as u32,
            )?;
            let Some(t) = pick.0 else {
                return Ok(0);
            };
            t
        }
    };
    let left = e.param - 1;
    set_entry(w, key, |e| e.param = left);
    if left > 0 {
        let Some(next) = crate::bridge::client_missiles::search_pick(
            w,
            &env,
            key,
            (0, 0),
            r,
            0x20003,
            target.guid,
        )?
        .0
        else {
            return Ok(0);
        };
        let p = inputs
            .skill_tables
            .as_deref()
            .and_then(|t| t.skills.get(usize::from(skill)))
            .ok_or(HandlerError::Invalid(
                "client/client-do/f021-melee-repeat.md §1 r1: function 21 needs the skills tables",
            ))?
            .param2 as i32;
        face_unit(w, key, next);
        set_entry(w, key, |e| {
            e.param2 = i32::from(next.unit_type);
            e.counter = next.guid as i32;
        });
        if let Some(u) = w.units.get_mut(&key) {
            rewind(&mut u.frame, p);
        }
    }
    Ok(1)
}

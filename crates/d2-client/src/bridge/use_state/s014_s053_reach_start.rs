//! cltstfunc 14 and 53 (Fend, Zeal, Fury): one function of the client skill start
//! table (`super::client_start_apply`); the shared helpers live in `super`.

use super::*;

/// `cltstfunc` 14 (`count`: Fend) and 53
/// (Zeal, Fury) `(U, skill, L)`: E :=
/// the used skill entry; r := U's melee range + 4. T :=
/// U's target unit; no T → T := the pick with (U, 0, 0, r,
/// 0x20003, −1, &n), none → E +0x18 := 0. With a T, n (function 14
/// only; the count with (U, 0, 0, r, 0x20003) when T was U's
/// target): E +0x18 := `calc1` (+0x138) at L, capped at n by function
/// 14. U faces T, E +0x1C, +0x20 := T's type and GUID.
pub(super) fn reach_start(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    caster: UnitKey,
    skill: u16,
    count: bool,
) -> Result<(), crate::bridge::dispatch::HandlerError> {
    let Some(e) = w
        .units
        .get(&caster)
        .and_then(|u| u.skills.as_ref())
        .and_then(|l| l.current.and_then(|i| l.entries.get(i)))
        .copied()
    else {
        return Ok(());
    };
    let env = crate::bridge::client_do::search_env(inputs);
    let r = crate::bridge::client_do::reach(w, &env, caster)? + 4;
    let (target, n) = match crate::bridge::client_do::path_target_unit(w, caster) {
        Some(t) => {
            let n = if count {
                crate::bridge::client_missiles::search_count(w, &env, caster, (0, 0), r, 0x20003)?
            } else {
                0
            };
            (Some(t), n)
        }
        None => crate::bridge::client_missiles::search_pick(
            w,
            &env,
            caster,
            (0, 0),
            r,
            0x20003,
            u32::MAX,
        )?,
    };
    let Some(target) = target else {
        set_param(w, caster, |e| e.param = 0);
        return Ok(());
    };
    let Some(t) = inputs.skill_tables.as_deref() else {
        return Ok(());
    };
    let Some(srow) = t.skills.get(usize::from(skill)) else {
        return Ok(());
    };
    let tb = &inputs.tables;
    let level = level_with_bonuses(w, caster, &tb.skills, &tb.skilldesc, &e);
    let c1 = {
        let mut units = crate::bridge::passive::ClientSkills::new(w, caster);
        d2_sim::skills::levels::eval_skill(
            &mut units,
            t,
            Some(caster),
            srow.calc1,
            i32::from(skill),
            level,
        )
    };
    let hits = if count { c1.min(n) } else { c1 };
    crate::bridge::client_do::face_unit(w, caster, target);
    set_param(w, caster, |e| {
        e.param = hits;
        e.param2 = i32::from(target.unit_type);
        e.counter = target.guid as i32;
    });
    Ok(())
}

/// Writes U's used skill entry through `f`.
pub(super) fn set_param(w: &mut ClientWorld, key: UnitKey, f: impl FnOnce(&mut SkillEntry)) {
    if let Some(e) = w
        .units
        .get_mut(&key)
        .and_then(|u| u.skills.as_mut())
        .and_then(|l| l.current.and_then(|i| l.entries.get_mut(i)))
    {
        f(e);
    }
}

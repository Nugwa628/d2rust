//! cltstfunc 13 (Strafe): one function of the client skill start
//! table (`super::client_start_apply`); the shared helpers live in `super`.

use super::*;

/// `cltstfunc` 13: the ammunition test (a player's weapon and its
/// quantity; any other unit passes) is read as passing. E := the used
/// skill entry; v := `aurarangecalc` (+0x64) at L. T := U's target unit
/// with n := the count with (U, 0, 0, v, 3); no T
/// → (T, n) := the pick with (U, 0, 0, v, 3, −1). a := `calc1`
/// (+0x138), b := `calc3` (+0x140), m := min(a, b): E +0x18 := a when a
/// ≤ max(n, m), else max(n, m). T → E +0x1C, +0x20 := T's type and GUID
/// and U faces T; no T → 1 and −1.
pub(super) fn strafe_start(w: &mut ClientWorld, inputs: &ModelInputs, caster: UnitKey, skill: u16) {
    let Some(e) = w
        .units
        .get(&caster)
        .and_then(|u| u.skills.as_ref())
        .and_then(|l| l.current.and_then(|i| l.entries.get(i)))
        .copied()
    else {
        return;
    };
    let Some(t) = inputs.skill_tables.as_deref() else {
        return;
    };
    let Some(srow) = t.skills.get(usize::from(skill)) else {
        return;
    };
    let tb = &inputs.tables;
    let level = level_with_bonuses(w, caster, &tb.skills, &tb.skilldesc, &e);
    let [v, a, b] = {
        let mut units = crate::bridge::passive::ClientSkills::new(w, caster);
        [srow.aurarangecalc, srow.calc1, srow.calc3].map(|c| {
            d2_sim::skills::levels::eval_skill(
                &mut units,
                t,
                Some(caster),
                c,
                i32::from(skill),
                level,
            )
        })
    };
    let env = crate::bridge::client_do::search_env(inputs);
    let (target, n) = match crate::bridge::client_do::path_target_unit(w, caster) {
        Some(t) => {
            let n = crate::bridge::client_missiles::search_count(w, &env, caster, (0, 0), v, 3)
                .unwrap_or(0);
            (Some(t), n)
        }
        None => {
            crate::bridge::client_missiles::search_pick(w, &env, caster, (0, 0), v, 3, u32::MAX)
                .unwrap_or((None, 0))
        }
    };
    let hi = n.max(a.min(b));
    let count = if a <= hi { a } else { hi };
    let (ty, guid) = target.map_or((1, -1), |t| (i32::from(t.unit_type), t.guid as i32));
    if let Some(t) = target {
        crate::bridge::client_do::face_unit(w, caster, t);
    }
    if let Some(e) = w
        .units
        .get_mut(&caster)
        .and_then(|u| u.skills.as_mut())
        .and_then(|l| l.current.and_then(|i| l.entries.get_mut(i)))
    {
        e.param = count;
        e.param2 = ty;
        e.counter = guid;
    }
}

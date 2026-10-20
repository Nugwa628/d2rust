//! cltstfunc 6 (Dragon Talon): one function of the client skill start
//! table (`super::client_start_apply`); the shared helpers live in `super`.

use super::*;

/// `cltstfunc` 6: see [`client_start_apply`].
pub(super) fn talon_start(w: &mut ClientWorld, inputs: &ModelInputs, caster: UnitKey, skill: u16) {
    let Some(e) = w
        .units
        .get(&caster)
        .and_then(|u| u.skills.as_ref())
        .and_then(|l| l.current.and_then(|i| l.entries.get(i)))
        .copied()
    else {
        return;
    };
    let v = match crate::bridge::client_do::path_target_unit(w, caster) {
        None => 0,
        Some(_) => {
            let Some(t) = inputs.skill_tables.as_deref() else {
                return;
            };
            let Some(srow) = t.skills.get(usize::from(skill)) else {
                return;
            };
            let calc = srow.calc1;
            let tb = &inputs.tables;
            let level = level_with_bonuses(w, caster, &tb.skills, &tb.skilldesc, &e);
            let mut units = crate::bridge::passive::ClientSkills::new(w, caster);
            d2_sim::skills::levels::eval_skill(
                &mut units,
                t,
                Some(caster),
                calc,
                i32::from(skill),
                level,
            )
        }
    };
    if let Some(e) = w
        .units
        .get_mut(&caster)
        .and_then(|u| u.skills.as_mut())
        .and_then(|l| l.current.and_then(|i| l.entries.get_mut(i)))
    {
        e.param = v;
    }
}

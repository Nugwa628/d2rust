//! cltstfunc 32 (MagottUp): one function of the client skill start table
//! (`super::client_start_apply`); the shared helpers live in `super`.

use super::*;

/// `cltstfunc` 32: the animation re-init
/// with (U) (the sequence of the used skill is loaded again), U
/// flags +0xC4 |= 0xE, then m := `cltmissilea` (+0xEA) when 0 < m < the
/// missile count, replaced by `cltmissileb` (+0xEC, same test) when U's
/// level is in act 2 (= 1), created at U's own position
/// (target (x, y), skill, level).
// PROVISIONAL: the tail is not run: the free-spot
// search (mask 0x1C09) from U's path position, the placement
// there and the room marking, and flag bit 0x8
// of +0xC4 (no model field); they need the client model's rooms.
pub(super) fn magott_up_start(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    caster: UnitKey,
    skill: u16,
    level: i32,
) -> Result<(), HandlerError> {
    let Some(r) = inputs.tables.skills.get(usize::from(skill)) else {
        return Ok(());
    };
    let Some(mode) = w.units.get(&caster).map(|u| u.mode) else {
        return Ok(());
    };
    if caster.unit_type == MONSTER {
        super::super::monster_anim::mode_init(w, inputs, caster, mode);
    }
    if let Some(u) = w.units.get_mut(&caster) {
        u.flag_2 = Some(true);
        u.flag_4 = true;
    }
    let valid = |m: i16| m > 0 && super::super::client_do::valid_missile(inputs, i32::from(m));
    let [a, b, _] = r.cltmissile_abc;
    if valid(a) {
        let act2 = w.act.as_ref().is_some_and(|l| l.act == 1);
        let m = if act2 && valid(b) { b } else { a };
        if let Some((x, y)) = w.units.get(&caster).map(|u| u.cell()) {
            super::super::client_do::create_toward(
                w,
                inputs,
                caster,
                i32::from(m),
                (i32::from(x), i32::from(y)),
                skill,
                level,
            )?;
        }
    }
    Ok(())
}

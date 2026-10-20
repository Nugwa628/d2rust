//! cltdofunc 42 (Double Throw): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 42 (Double Throw):
/// I := the attack weapon with (U, 1) by the used skill's
/// `weapsel` ([`crate::bridge::attack_weapon::attack_weapon`]), none →
/// 0. From sequence frame 9 (U +0x38 ≥ 0x900, fraction cleared) flag
/// 0x40 on U, before the item tests. I's primary type not `throwable`
/// and I's stat 125 ≠ 0 → 0. m := I's `missiletype`; m
/// outside (0, missile rows) → 0. I of type 0x26 → the lob
/// (not modelled, as in function 2); else with (U, m, skill,
/// level) ([`cast_toward`]) and 1.
pub(super) fn double_throw(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    use crate::bridge::attack_weapon::{attack_weapon, item_total};
    let weapsel = crate::bridge::player_anim::used_row(w, inputs, key).map(|r| r.weapsel);
    let Some(item) = attack_weapon(w, key, weapsel) else {
        return Ok(0);
    };
    let pos = w
        .units
        .get(&key)
        .and_then(|u| u.sequence.as_ref())
        .map_or(0, |s| s.pos & !0xFF);
    if pos > 0x8FF {
        set_flag_40(w, key);
    }
    let (Some(tables), Some(code)) = (w.item_tables.0.clone(), item.code) else {
        return Ok(0);
    };
    if !tables.type_throwable(code) && item_total(w, item.key, 125) != 0 {
        return Ok(0);
    }
    let m = tables
        .missile_and_type(code)
        .map_or(0, |(m, _)| i32::from(m));
    if m <= 0 || m as usize >= inputs.tables.missiles.len() {
        return Ok(0);
    }
    if tables.item_is_type(code, 0x26) {
        return Err(HandlerError::Invalid(
            "client/client-do/f042-double-throw.md §1 r5: the progressive lob is not modelled",
        ));
    }
    cast_toward(w, inputs, key, m, skill, level)?;
    Ok(1)
}

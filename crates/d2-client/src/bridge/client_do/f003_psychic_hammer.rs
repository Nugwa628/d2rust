//! cltdofunc 3 (Psychic Hammer): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 3 (Psychic Hammer):
/// T := [`path_target_unit`]; none → 0. U's room and T's room not
/// in town, T a player or monster, and the hostility test with (U,
/// T) → flag 0x40 on U, 1; else 0. No missile.
pub(super) fn psychic_hammer(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
) -> Result<i32, HandlerError> {
    let Some(t) = path_target_unit(w, key) else {
        return Ok(0);
    };
    if crate::bridge::modes::in_town(w, key)
        || crate::bridge::modes::in_town(w, t)
        || t.unit_type > MONSTER
    {
        return Ok(0);
    }
    if !crate::bridge::combat::hostile_between(w, &inputs.tables.monsters, key, t) {
        return Ok(0);
    }
    set_flag_40(w, key);
    Ok(1)
}

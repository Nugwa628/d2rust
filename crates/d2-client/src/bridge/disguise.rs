//! The disguise of a client unit: the flag-ex bit 3 its states toggle,
//! the draw identity it is animated as,
//! the transform of a state on and the state on hooks
//! (the `setfunc` 14 of `wolf` / `bear`).

use d2_sim::units::anim_rate::{mode_fallback, MONSTER_TO_PLAYER, PLAYER_TO_MONSTER};

use super::dispatch::{HandlerError, Message};
use super::world::{ClientWorld, ModelInputs, UnitKey, MONSTER, PLAYER};

/// Unit flag-ex (+0xC8) bit 3: the unit is drawn as its gfx state's
/// class (reads it).
pub const FLAG_EX_DISGUISE: u32 = 0x8;

/// The disguise part of the state toggle:
/// a state of the `disguise` flag
/// newly turned on sets flag-ex bit 3; one turned off clears it when the
/// unit has no other state of that flag left. Run after the bit changed.
pub fn toggle(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, state: u8, on: bool) {
    let rows = &inputs.tables.states;
    let disguise = |s: u8| rows.get(usize::from(s)).is_some_and(|r| r.disguise);
    if !disguise(state) {
        return;
    }
    let Some(u) = w.units.get_mut(&key) else {
        return;
    };
    if on {
        u.flag_ex |= FLAG_EX_DISGUISE;
    } else if !u.states.iter().any(|&s| disguise(s)) {
        u.flag_ex &= !FLAG_EX_DISGUISE;
    }
}

/// A draw identity: type 0 player or 1 monster, class, mode.
pub type Identity = (u8, u32, u32);

/// The draw identity of a
/// player or monster in `mode`: its own unless flag-ex bit 3 is set;
/// then the first state (ascending id) with a `gfxtype` the unit has
/// gives the type and class, and a player shown as a monster has its
/// mode mapped (table, then the monster-mode
/// fallback until the class has the mode, `monstats2` +0xF0), a monster
/// shown as a player by table (class 6 in mode 4 → 12).
pub fn identity(w: &ClientWorld, inputs: &ModelInputs, key: UnitKey, mode: u32) -> Identity {
    let Some(u) = w.units.get(&key) else {
        return (key.unit_type, 0, mode);
    };
    let own = (key.unit_type, u.class, mode);
    if u.flag_ex & FLAG_EX_DISGUISE == 0 {
        return own;
    }
    let rows = &inputs.tables.states;
    let Some(row) = u
        .states
        .iter()
        .filter_map(|&s| rows.get(usize::from(s)))
        .find(|r| r.gfxtype != 0)
    else {
        return own;
    };
    let class = u32::from(row.gfxclass);
    match (row.gfxtype, key.unit_type) {
        (1, PLAYER) => {
            let modes = inputs
                .tables
                .monsters
                .get(class as usize)
                .and_then(|c| c.as_ref())
                .map_or(0, |c| c.modes);
            let has = |m: u32| m < 16 && modes & (1 << m) != 0;
            let mut m = PLAYER_TO_MONSTER.get(mode as usize).copied().unwrap_or(1);
            while m != 1 && !has(m) {
                m = mode_fallback(m);
            }
            (MONSTER, class, m)
        }
        (1, _) => (MONSTER, class, mode),
        (_, MONSTER) => {
            let m = if class == 6 && mode == 4 {
                12
            } else {
                MONSTER_TO_PLAYER.get(mode as usize).copied().unwrap_or(1)
            };
            (PLAYER, class, m)
        }
        _ => (PLAYER, class, mode),
    }
}

/// The transform of a state on whose row has the
/// `transform` flag: a player in mode
/// 0x12 keeps everything; a player or monster takes mode 1 (the mode
/// set), then every unit restarts its animation,
/// frame := 0, used skill := none.
pub fn transform(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey) {
    let Some(u) = w.units.get(&key) else {
        return;
    };
    if key.unit_type == PLAYER && u.mode == 0x12 {
        return;
    }
    let mode = if matches!(key.unit_type, PLAYER | MONSTER) {
        1
    } else {
        u.mode
    };
    match key.unit_type {
        PLAYER => {
            if let Some(u) = w.units.get_mut(&key) {
                u.flag_40 = false;
            }
            super::player_anim::mode_init(w, inputs, key, mode);
        }
        MONSTER => {
            if let Some(u) = w.units.get_mut(&key) {
                u.flag_40 = false;
            }
            super::monster_anim::mode_init(w, inputs, key, mode);
        }
        _ => {}
    }
    if let Some(u) = w.units.get_mut(&key) {
        u.frame = 0;
        if let Some(l) = u.skills.as_mut() {
            l.current = None;
        }
        u.used_skill = None;
    }
}

/// The state on hooks, skipped
/// for a unit dead for the state flag: the state's `setfunc`. Only
/// `setfunc` 14 and 19 (the skill-use lock)
/// change the model; a state missile is presentation.
pub fn hooks(
    w: &mut ClientWorld,
    msg: &Message<'_>,
    key: UnitKey,
    state: u8,
) -> Result<(), HandlerError> {
    let inputs = msg.inputs;
    let Some(row) = inputs.tables.states.get(usize::from(state)).copied() else {
        return Ok(());
    };
    if row.dead_bit_only && super::msg::states::dead_for_states(w, key) {
        return Ok(());
    }
    // setfunc 1: the progressive overlay.
    if row.setfunc == 1 {
        super::state_overlays::pgsv_set(w, inputs, key, state);
    }
    if row.setfunc == 14 {
        stand_still(w, msg, key)?;
    }
    // setfunc 19: the skill-use lock := 0.
    if row.setfunc == 19 {
        w.skill_lock = 0;
    }
    Ok(())
}

/// `setfunc` 14: the mode request code 0x19
/// with the record [x, y, 0, 0, 0, 0, 0] (the unit's sub-tile) and flag
/// 0 (a player in mode 0x13 ignores it), then the face of
/// direction 0.
fn stand_still(w: &mut ClientWorld, msg: &Message<'_>, key: UnitKey) -> Result<(), HandlerError> {
    let Some(u) = w.units.get(&key) else {
        return Ok(());
    };
    // Types 2, 4, 5 read the static path's +0xC / +0x10, the others:
    // in the model both are the sub-tile.
    let (x, y) = u.cell();
    let skip = key.unit_type == PLAYER && u.mode == super::modes::player_mode::SEQUENCE;
    if !skip {
        let record = [i32::from(x), i32::from(y), 0, 0, 0, 0, 0];
        super::modes::mode_request(w, msg.inputs, key, 0x19, record, msg.out)?;
    }
    super::critter_path::face_direction(w, key, 0);
    Ok(())
}

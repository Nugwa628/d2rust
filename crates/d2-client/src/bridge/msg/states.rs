//! Unit states: S→C 0xA7 DelayedState, 0xA8 SetState, 0xA9 EndState
//! and 0xAA AddUnit,
//! over the state bits and state lists of
//! `ClientUnit` ([`state_on`], [`state_stat`], [`state_off`]).
//!
//! The disguise bit, the transform and the state on hooks
//! are [`super::super::disguise`]'s; the overlay and light refreshes of
//! state on / off and a state missile are client presentation
//! (open question 6, Phase 6): no model field
//! changes.

use super::super::bits::BitReader;
use super::super::disguise;
use super::super::dispatch::{HandlerError, Message};
use super::super::world::{ClientWorld, ModelInputs, UnitKey, MONSTER, PLAYER};
use super::Bytes;

/// Dead for the state flag: monster mode 12, player mode 17.
pub(crate) fn dead_for_states(w: &ClientWorld, key: UnitKey) -> bool {
    w.units.get(&key).is_some_and(|u| {
        (u.key.unit_type == MONSTER && u.mode == 12) || (u.key.unit_type == PLAYER && u.mode == 17)
    })
}

/// State on with (unit, state). A
/// flag of the states row is read only when it decides the outcome; a
/// missing row then is a handler error (the tables are an input).
/// Records the order a state bit was turned on in (`ClientWorld::state_on_seq`).
fn note_state_on(w: &mut ClientWorld, key: UnitKey, state: u8) {
    w.state_on_next += 1;
    w.state_on_seq.insert((key, state), w.state_on_next);
}

pub fn state_on(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    state: u8,
) -> Result<(), HandlerError> {
    let row = inputs.tables.states.get(usize::from(state)).copied();
    let missing =
        || HandlerError::Invalid("client/stat-lists.md §3 r3: no states row for the state's flags");
    if dead_for_states(w, key) && row.ok_or_else(missing)?.dead_bit_only {
        if w.units
            .get_mut(&key)
            .is_some_and(|u| u.states.insert(state))
        {
            note_state_on(w, key, state);
            disguise::toggle(w, inputs, key, state, true);
        }
        return Ok(());
    }
    let Some(u) = w.units.get_mut(&key) else {
        return Ok(());
    };
    if u.state_lists.contains_key(&state) && !row.ok_or_else(missing)?.keep_list {
        u.state_lists.remove(&state);
    }
    let newly = u.states.insert(state);
    if newly {
        note_state_on(w, key, state);
        // The bit set.
        disguise::toggle(w, inputs, key, state, true);
    }
    // Colorshift ≠ 0 → the colour call (its model part: the local
    // player's light colour), then the
    // overlays.
    colour_call(w, inputs, key, row)?;
    super::super::state_overlays::on(w, inputs, key, state);
    // A `transform` state.
    if row.is_some_and(|r| r.transform) {
        disguise::transform(w, inputs, key);
    }
    Ok(())
}

/// The colour call of state on / off when the state's
/// `colorshift` ≠ 0: the unit's palette index and the local player's light
/// colour ([`super::lighting::state_colour_call`]). Without the state's
/// row only the local player with a light is a handler error (the tables
/// are an input).
fn colour_call(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    row: Option<super::super::world::StateRow>,
) -> Result<(), HandlerError> {
    let Some(row) = row else {
        if super::lighting::colour_call_lights(w, key) {
            return Err(HandlerError::Invalid(
                "client/stat-lists.md §3 r6: no states row for the state's colorshift",
            ));
        }
        return Ok(());
    };
    if row.colorshift != 0 {
        super::lighting::state_colour_call(w, &inputs.tables.states, key);
    }
    Ok(())
}

/// State off with (unit, state, 0): the bit only.
pub fn state_bit_off(w: &mut ClientWorld, key: UnitKey, state: u8) {
    if let Some(u) = w.units.get_mut(&key) {
        u.states.remove(&state);
    }
}

/// State off with its list (0xA9: then): the
/// bit cleared, the state's list detached and freed, then the colour call
/// for a state with `colorshift` ≠ 0.
pub fn state_off(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    state: u8,
) -> Result<(), HandlerError> {
    if !w.units.contains_key(&key) {
        return Ok(());
    }
    // The remfunc first. Remfunc 12: the
    // skill-use lock := 0, whether or not the bit was set; remfunc 1:
    // the progressive overlay removed.
    let row = inputs.tables.states.get(usize::from(state)).copied();
    if row.is_some_and(|r| r.remfunc == 1) {
        super::super::state_overlays::pgsv_rem(w, inputs, key, state);
    }
    let Some(u) = w.units.get_mut(&key) else {
        return Ok(());
    };
    let lock_clear = row.is_some_and(|r| r.remfunc == 12);
    let was = u.states.remove(&state);
    u.state_lists.remove(&state);
    if lock_clear {
        w.skill_lock = 0;
    }
    if was {
        disguise::toggle(w, inputs, key, state, false);
    }
    // Colorshift ≠ 0 → the colour call after the bit is cleared,
    // then the overlays (`removerlay`).
    colour_call(w, inputs, key, row)?;
    super::super::state_overlays::off(w, inputs, key, state);
    // Last the animation-rate refresh.
    super::super::player_anim::refresh_rate(w, inputs, key);
    Ok(())
}

/// The state stat with (unit, list, state, stat, value, param):
/// the unit's list of the state (made
/// and attached when it has none), then the set
/// (a value 0 removes the entry).
/// Stat 172 also runs with (value): a monster with old ≠
/// new (bytes; old = the existing list's stat 172 at that param, 4 for a
/// new list) and a room changes the room's allied count (+0x28): new = 2
/// → += 1, else old = 2 → −= 1 (fatal 0x27C when already < 1); the path
/// reset has no model field.
pub fn state_stat(
    w: &mut ClientWorld,
    key: UnitKey,
    state: u8,
    stat: u16,
    param: u16,
    value: i32,
) -> Result<(), HandlerError> {
    if stat == 172 && key.unit_type == MONSTER {
        let old = w
            .units
            .get(&key)
            .and_then(|u| u.state_lists.get(&state))
            .map_or(4, |l| l.get(&(172, param)).copied().unwrap_or(0)) as u8;
        let new = value as u8;
        if let Some(room) = w.room_units.room_of(key).filter(|_| old != new) {
            let count = w.room_allied.entry(room).or_insert(0);
            if new == 2 {
                *count += 1;
            } else if old == 2 {
                if *count < 1 {
                    return Err(HandlerError::Fatal(0x27C));
                }
                *count -= 1;
            }
        }
    }
    if let Some(u) = w.units.get_mut(&key) {
        let list = u.state_lists.entry(state).or_default();
        if value == 0 {
            list.remove(&(stat, param));
        } else {
            list.insert((stat, param), value);
        }
    }
    Ok(())
}

/// Why a state stat stream stopped.
enum StatsEnd {
    /// 0x1FF: the state's hooks run.
    Done,
    /// A stat outside the table or with `Send Bits` 0.
    Abort,
    /// A fatal assert of a state stat.
    Fatal(HandlerError),
}

/// One state's stat stream (0xA8 step 1, 0xAA step 2.3): stat id 9 bits,
/// param, value; each added to the state's list.
fn read_stats(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    state: u8,
    r: &mut BitReader<'_>,
) -> StatsEnd {
    loop {
        let id = r.read(9);
        if id == 0x1FF {
            return StatsEnd::Done;
        }
        let Some(row) = inputs.tables.stats.get(id as usize) else {
            return StatsEnd::Abort;
        };
        if row.bits == 0 {
            return StatsEnd::Abort;
        }
        let param = if row.param_bits > 0 {
            r.read_signed(u32::from(row.param_bits)) as u16
        } else {
            0
        };
        let bits = u32::from(row.bits);
        let value = if bits < 32 && row.signed {
            r.read_signed(bits)
        } else {
            r.read(bits) as i32
        };
        if let Err(e) = state_stat(w, key, state, id as u16, param, value) {
            return StatsEnd::Fatal(e);
        }
        if row.update_anim_rate {
            super::super::player_anim::refresh_rate(w, inputs, key);
        }
    }
}

/// 0xA7 DelayedState, 0xA9 EndState: type
/// u8@1, GUID u32@2, state u8@6; a unit in S → 0xA7 state on (then the
/// hooks), 0xA9 state off.
pub fn delayed_or_end_state(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 7 {
        return Err(HandlerError::Invalid("0xA7 / 0xA9 are 7 bytes"));
    }
    let key = UnitKey::new(b.u8(1)?, b.u32(2)?);
    let state = b.u8(6)?;
    if !w.units.contains_key(&key) {
        return Ok(());
    }
    if msg.id == 0xA7 {
        state_on(w, msg.inputs, key, state)?;
        disguise::hooks(w, msg, key, state)
    } else {
        state_off(w, msg.inputs, key, state)
    }
}

/// 0xA8 SetState: type u8@1, GUID u32@2,
/// size u8@6, state u8@7, the stat stream from @8.
pub fn set_state(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    let size = usize::from(b.u8(6)?);
    if size != msg.bytes.len() || size < 8 {
        return Err(HandlerError::Invalid("0xA8 size byte"));
    }
    let key = UnitKey::new(b.u8(1)?, b.u32(2)?);
    let state = b.u8(7)?;
    if !w.units.contains_key(&key) {
        return Ok(());
    }
    state_on(w, msg.inputs, key, state)?;
    let mut r = BitReader::new(&msg.bytes[8..]);
    // Either end runs the hooks (step 1: "after the stream").
    if let StatsEnd::Fatal(e) = read_stats(w, msg.inputs, key, state, &mut r) {
        return Err(e);
    }
    disguise::hooks(w, msg, key, state)
}

/// 0xAA AddUnit: type u8@1, GUID u32@2,
/// size u8@6, the state stream from @7.
pub fn add_unit_states(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    let size = usize::from(b.u8(6)?);
    if size != msg.bytes.len() || size < 7 {
        return Err(HandlerError::Invalid("0xAA size byte"));
    }
    let key = UnitKey::new(b.u8(1)?, b.u32(2)?);
    if !w.units.contains_key(&key) {
        return Ok(());
    }
    let mut r = BitReader::new(&msg.bytes[7..]);
    loop {
        let s = r.read(8);
        if r.overflow {
            // Step 3 (D2RUST): a stream without its closing 0xFF.
            return Err(HandlerError::Invalid(
                "0xAA: the state stream ran past its end (no closing 0xFF)",
            ));
        }
        if s >= 255 {
            return Ok(());
        }
        let state = s as u8;
        state_on(w, msg.inputs, key, state)?;
        if r.read(1) == 1 {
            match read_stats(w, msg.inputs, key, state, &mut r) {
                // Step 2.3: the whole message ends, no hooks.
                StatsEnd::Abort => return Ok(()),
                StatsEnd::Fatal(e) => return Err(e),
                StatsEnd::Done => {}
            }
        }
        if r.overflow {
            return Err(HandlerError::Invalid(
                "0xAA: the state stream ran past its end (no closing 0xFF)",
            ));
        }
        // Step 2.4: the state on hooks.
        disguise::hooks(w, msg, key, state)?;
    }
}

//! Object mode sounds: Cain's gibbet line, the mode loop and the
//! transition.

use super::tables::{ObjectSoundRow, ObjectSounds, MAX_OBJECT_CLASS};
use super::{Ctx, TriggerError, Unit, UnitSound};

/// Class 26 (Cain's gibbet), its line 3,671 `cain_act1_help` and the
/// distance below which it plays.
pub const CAIN_GIBBET: i32 = 26;
pub const CAIN_HELP: i32 = 3671;
pub const CAIN_DIST: i32 = 20;
/// Class 59 (town portal): transitions also on the first call.
pub const TOWN_PORTAL: i32 = 59;
/// Class 61: cairn stone loops by mode.
pub const CAIRN: i32 = 61;
/// The cairn loop table by mode: modes 1–5 → 413–417
/// (`cairn_stone_1` … `cairn_stone_5`), modes 0, 6, 7 → 0.
pub const CAIRN_LOOPS: [i32; 8] = [0, 413, 414, 415, 416, 417, 0, 0];
/// Loop mode value meaning "any mode" (object sound table).
pub const ANY_MODE: u8 = 8;

/// Object mode sound call on object U in its current mode.
/// A class without a TSV row (120 of 573) behaves as an all-zero row:
/// no transition, loop 0 (U is detached from its looping
/// requests), and U+0x74 / U+0x70 still updated. Class ≥ 573 or mode ≥ 8
/// is fatal. Class 61 loops the cairn table by mode.
pub fn object_mode(
    cx: &mut Ctx,
    table: &ObjectSounds,
    u: &Unit,
    us: &mut UnitSound,
) -> Result<(), TriggerError> {
    let c = u.class;
    let m = u.mode;
    if !(0..=MAX_OBJECT_CLASS).contains(&c) {
        return Err(TriggerError::ObjectClass(c));
    }
    if m >= 8 {
        return Err(TriggerError::ObjectMode(m));
    }
    let zero = ObjectSoundRow {
        class: c,
        modes: [0; 8],
        loop_a: 0,
        loop_a_mode: 0,
        loop_b: 0,
        loop_b_mode: 0,
        ordered: false,
    };
    let has_record = table.get(c).is_some();
    let row = table.get(c).unwrap_or(&zero);
    // Step 2.
    if c == CAIN_GIBBET && m == 0 && us.last_idle == 0 && u.local_dist < CAIN_DIST {
        cx.req(CAIN_HELP, Some(u.key), 0);
        us.last_idle = cx.c;
    }
    // Step 3.
    if us.obj_seen && m == us.obj_prev_mode {
        return Ok(());
    }
    // Step 4: the mode loop (a null record gives 0).
    let s = if !has_record {
        0
    } else if c == CAIRN {
        CAIRN_LOOPS[m as usize]
    } else if row.loop_a_mode == m || row.loop_a_mode == ANY_MODE {
        row.loop_a
    } else if row.loop_b_mode == m || row.loop_b_mode == ANY_MODE {
        row.loop_b
    } else {
        0
    };
    if s != 0 {
        if !super::request_in_group(&*cx.s, u.key, s) {
            cx.req(s, Some(u.key), 0);
        }
    } else {
        for (h, id) in cx.s.unit_requests(u.key) {
            if cx.s.looping(id) {
                cx.s.detach(h, u.key, false);
            }
        }
    }
    // Step 5.
    if us.obj_seen || c == TOWN_PORTAL {
        let mut t = row.modes.get(m as usize).copied().unwrap_or(0);
        if row.ordered && m < us.obj_prev_mode {
            t = 0;
        }
        if t != 0 {
            cx.req(t, Some(u.key), 0);
        }
    }
    // Step 6.
    us.obj_prev_mode = m;
    us.obj_seen = true;
    Ok(())
}

// ---------------------------------------------------------------------------
// When object units make their mode sounds
// ---------------------------------------------------------------------------

/// Entries of the client object function table; a `ClientFn`
/// at or above it is fatal (`0x546`).
pub const CLIENT_FN_COUNT: u8 = 19;
/// `ClientFn` of the keeper (class 568).
pub const CLIENT_FN_KEEPER: u8 = 18;
/// `barbarian_grunt_small_1`.
pub const KEEPER_GRUNT: i32 = 2505;

/// The client object function for the functions that make
/// the mode sound call themselves: `ClientFn` 3 calls it and
/// returns 1 (so the update calls it again: twice); 4, 5, 6 call it and
/// return 0 (once); case 0 returns 1 at once; other functions make no
/// call here. Returns the function's return value.
pub fn client_function_sound(
    cx: &mut Ctx,
    table: &ObjectSounds,
    u: &Unit,
    us: &mut UnitSound,
    client_fn: u8,
) -> Result<bool, TriggerError> {
    match client_fn {
        3 => {
            object_mode(cx, table, u, us)?;
            Ok(true)
        }
        4..=6 => {
            object_mode(cx, table, u, us)?;
            Ok(false)
        }
        0 => Ok(true),
        _ => Ok(false),
    }
}

/// The object part of a client update, after the light update.
/// `ClientFn` ≤ 3 → the mode sound call; ≥ 4 → the client object function
/// first (≥ 19 fatal), then the mode sound call only if it returned
/// non-zero. `client_fn_result` is that function's result for the
/// functions this module does not own (`client_function_sound` covers 3–6).
pub fn object_update_sounds(
    cx: &mut Ctx,
    table: &ObjectSounds,
    u: &Unit,
    us: &mut UnitSound,
    client_fn: u8,
    client_fn_result: bool,
) -> Result<(), TriggerError> {
    if client_fn <= 3 {
        return object_mode(cx, table, u, us);
    }
    if client_fn >= CLIENT_FN_COUNT {
        return Err(TriggerError::ClientFn(client_fn));
    }
    let r = match client_fn {
        4..=6 => client_function_sound(cx, table, u, us, client_fn)?,
        _ => client_fn_result,
    };
    if r {
        object_mode(cx, table, u, us)?;
    }
    Ok(())
}

/// The client-only (C) object walk runs the client function once
/// more for type-2 units, for every `ClientFn`.
pub fn client_only_second_call(
    cx: &mut Ctx,
    table: &ObjectSounds,
    u: &Unit,
    us: &mut UnitSound,
    client_fn: u8,
) -> Result<bool, TriggerError> {
    client_function_sound(cx, table, u, us, client_fn)
}

/// `ClientFn` 18: when `GetTickCount` > U+0xD4
/// (unsigned), one step of U's own seed giving s1; s1 mod 100 < 10 (low
/// dword, unsigned) → request 2,505 on U; a second step s2; U+0xD4 :=
/// `GetTickCount` + (s2 mod 60) × 1,000. `step` is the seed step (low
/// dword). Wall-clock driven.
pub fn keeper_grunt(
    cx: &mut Ctx,
    u: &Unit,
    now_ms: u32,
    next_ms: &mut u32,
    step: &mut dyn FnMut() -> u32,
) {
    if now_ms <= *next_ms {
        return;
    }
    let r1 = step();
    if r1 % 100 < 10 {
        cx.req(KEEPER_GRUNT, Some(u.key), 0);
    }
    let r2 = step();
    *next_ms = now_ms.wrapping_add((r2 % 60).wrapping_mul(1000));
}

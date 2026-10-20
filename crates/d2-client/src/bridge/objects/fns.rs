//! The client object functions `ClientFn` 1–18 (table) and
//! the day-period object refresh that `ClientFn` 14 runs.
//! Each returns the entry's byte result as a bool.

use super::super::dispatch::HandlerError;
use super::super::world::{UnitKey, MONSTER};
use super::{
    create_client_unit, distance, interact, quest_bit, remove_client_unit, step_seed, unit_size,
    Cx, ObjFx, ObjSound, CLIENT_FNS, FATAL_CLIENT_FN,
};
use crate::bridge::output::Output;

/// Overlay 72 `npcalert` of `ClientFn` 15.
pub const OVERLAY_NPCALERT: u16 = 72;
/// Overlay kind 3 (a loop, no seed draw).
pub const OVERLAY_KIND_LOOP: u8 = 3;
/// The orifice preload: monstats 211 `duriel`, flag 1.
pub const ORIFICE_PRELOAD: (u16, u8) = (211, 1);
/// The altar preloads: monstats 537–539 `ancientstatue1`–`3`, flag 0.
pub const ALTAR_PRELOADS: [u16; 3] = [537, 538, 539];
/// The zoo's chicken class (monstats 149).
pub const CHICKEN: u32 = 149;
/// The Keeper's sound, 2,505 `barbarian_grunt_small_1`.
pub const KEEPER_SOUND: i32 = 2505;
/// Quest 39 (Rite of Passage) bits of `ClientFn` 13.
pub const RITE_QUEST: u8 = 39;

/// The table entry `ClientFn` of U's class.
pub fn call(cx: &mut Cx<'_>) -> Result<bool, HandlerError> {
    let f = cx.row.client_fn;
    if f >= CLIENT_FNS {
        return Err(HandlerError::Fatal(FATAL_CLIENT_FN));
    }
    match f {
        0 => Ok(true),
        1 | 11 => Ok(true),
        2 => ripple(cx, false),
        3 => ripple(cx, true),
        4 => drinker(cx),
        5 => gesturer(cx),
        6 => turner(cx),
        7 => timed_spawn(cx, 10_000, (4_500, 7_500)),
        8 => orifice(cx),
        9 => smoke(cx),
        10 => timed_spawn(cx, 3_000, (2_500, 7_500)),
        12 => altar(cx),
        13 => ancient(cx),
        14 => bonfire(cx),
        15 => anya(cx),
        16 => baal_portal(cx),
        17 => zoo(cx),
        _ => keeper(cx),
    }
}

/// `ClientFn` 2, and `ClientFn` 3 (then `sound(U)`).
fn ripple(cx: &mut Cx<'_>, with_sound: bool) -> Result<bool, HandlerError> {
    // The whole flag-ex dword as the "initialised" marker.
    if cx.u()?.flag_ex == 0 {
        let lo = cx.step()?;
        let u = cx.u()?;
        u.interact_ms = lo & 0x3F;
        u.flag_ex = 1;
    }
    let u = cx.u()?;
    if u.interact_ms & 0x3F == 0 && u.mode != 1 {
        u.mode = 1;
        cx.reinit()?;
    }
    let u = cx.u()?;
    u.interact_ms = u.interact_ms.wrapping_add(1);
    if cx.u()?.mode == 2 {
        let lo = cx.step()?;
        let u = cx.u()?;
        u.interact_ms = u.interact_ms.wrapping_add(lo & 0x3F);
        u.mode = 0;
        cx.reinit()?;
    }
    if with_sound {
        cx.sound()?;
    }
    Ok(true)
}

/// The timer of `ClientFn` 4–6: T < now (unsigned) → T := now +
/// `range(lo, hi)`.
fn timer_due(cx: &mut Cx<'_>) -> Result<bool, HandlerError> {
    let now = cx.now();
    Ok(cx.u()?.interact_ms < now)
}

fn set_timer(cx: &mut Cx<'_>, lo: i32, hi: i32) -> Result<(), HandlerError> {
    let now = cx.now();
    let r = cx.range(lo, hi)?;
    cx.u()?.interact_ms = now.wrapping_add(r);
    Ok(())
}

/// frame := raw `Start[k]` (no × 256, edge case 3).
fn raw_start(cx: &mut Cx<'_>, k: usize) -> Result<(), HandlerError> {
    let s = i32::from(cx.row.start[k]);
    cx.u()?.frame = s;
    Ok(())
}

/// The return to mode 0 of `ClientFn` 4–6.
fn back_to_neutral(cx: &mut Cx<'_>) -> Result<(), HandlerError> {
    raw_start(cx, 0)?;
    cx.set_mode(0)?;
    cx.refresh()
}

/// `ClientFn` 4 (110 `drinker`).
fn drinker(cx: &mut Cx<'_>) -> Result<bool, HandlerError> {
    if timer_due(cx)? {
        set_timer(cx, 7_000, 12_000)?;
        raw_start(cx, 5)?;
        cx.set_mode(3)?;
        cx.refresh()?;
    }
    let end = cx.end(3)?;
    let u = cx.u()?;
    if u.mode == 3 && u.frame >= end {
        back_to_neutral(cx)?;
    }
    cx.sound()?;
    Ok(false)
}

/// `ClientFn` 5 (112 `gesturer`).
fn gesturer(cx: &mut Cx<'_>) -> Result<bool, HandlerError> {
    if timer_due(cx)? {
        set_timer(cx, 23_000, 33_000)?;
        raw_start(cx, 5)?;
        let m = if cx.step()? & 1 != 0 { 3 } else { 4 };
        cx.set_mode(m)?;
        cx.refresh()?;
    }
    let mode = cx.u()?.mode;
    if matches!(mode, 3 | 4) && cx.u()?.frame >= cx.end(mode)? {
        back_to_neutral(cx)?;
    }
    cx.sound()?;
    Ok(false)
}

/// `ClientFn` 6 (114 `turner`).
fn turner(cx: &mut Cx<'_>) -> Result<bool, HandlerError> {
    if timer_due(cx)? {
        set_timer(cx, 20_000, 30_000)?;
        raw_start(cx, 5)?;
        cx.set_mode(3)?;
        cx.refresh()?;
    }
    let mode = cx.u()?.mode;
    if mode >= 3 && cx.u()?.frame == cx.end(mode)? {
        back_to_neutral(cx)?;
    }
    cx.sound()?;
    Ok(false)
}

/// `ClientFn` 7 (259, 373) and `ClientFn` 10 (528): mode 0
/// waits `first` ms then goes to mode 1 from raw `Start1`; mode ≥ 1
/// waits `range(next)` then returns to mode 0. now is read once.
fn timed_spawn(cx: &mut Cx<'_>, first: u32, next: (i32, i32)) -> Result<bool, HandlerError> {
    let now = cx.now();
    let (mode, t) = {
        let u = cx.u()?;
        (u.mode, u.interact_ms)
    };
    if t < now {
        if mode == 0 {
            cx.u()?.interact_ms = now.wrapping_add(first);
            raw_start(cx, 1)?;
            cx.set_mode(1)?;
        } else {
            set_timer(cx, next.0, next.1)?;
            cx.set_mode(0)?;
        }
        cx.refresh()?;
    }
    Ok(true)
}

/// `ClientFn` 8 (152 `orifice`):.
fn orifice(cx: &mut Cx<'_>) -> Result<bool, HandlerError> {
    if cx.u()?.mode != 0 && !cx.w.objclient.latches.orifice {
        cx.w.objclient.latches.orifice = true;
        let (class, flag) = ORIFICE_PRELOAD;
        cx.fx(ObjFx::GfxLoad { class, flag });
    }
    Ok(true)
}

/// `ClientFn` 12 (546 `ancientsaltar`):.
fn altar(cx: &mut Cx<'_>) -> Result<bool, HandlerError> {
    if cx.u()?.mode != 0 && !cx.w.objclient.latches.altar {
        cx.w.objclient.latches.altar = true;
        for class in ALTAR_PRELOADS {
            cx.fx(ObjFx::GfxLoad { class, flag: 0 });
        }
    }
    Ok(true)
}

/// The local player, read without a null test (edge case 5): none is
/// an access violation in the base game.
fn local_player_unchecked(cx: &Cx<'_>, at: u32) -> Result<UnitKey, HandlerError> {
    cx.w.local_player
        .filter(|k| cx.w.units.contains_key(k))
        .ok_or(HandlerError::Crash {
            at,
            what: "the local player (none)",
        })
}

/// The path distance from U to P.
fn distance_to(cx: &mut Cx<'_>, p: UnitKey) -> Result<i32, HandlerError> {
    let rows = &cx.inputs.objclient.rows;
    let u = cx.u()?.clone();
    let pu =
        cx.w.units
            .get(&p)
            .ok_or(HandlerError::Invalid("local player not in set S"))?;
    Ok(distance(&u, unit_size(&u, rows), pu, unit_size(pu, rows)))
}

/// `ClientFn` 9 (478 `clientsmoke`).
fn smoke(cx: &mut Cx<'_>) -> Result<bool, HandlerError> {
    let now = cx.now();
    let t = cx.u()?.interact_ms;
    if t == 0 {
        cx.u()?.interact_ms = now;
        return Ok(true);
    }
    if now.wrapping_sub(t) <= 1_000 {
        return Ok(true);
    }
    let p = local_player_unchecked(cx, 0x004B_DBC4)?;
    if distance_to(cx, p)? > 25 {
        // The client-only removal.
        remove_client_unit(cx.w, cx.unit.key);
        return Ok(false);
    }
    cx.u()?.interact_ms = now;
    Ok(true)
}

/// `ClientFn` 13 (561, the invisible Ancient).
fn ancient(cx: &mut Cx<'_>) -> Result<bool, HandlerError> {
    if cx.u()?.mode != 0 {
        return Ok(true);
    }
    let Some(p) = cx.w.local_player.filter(|k| cx.w.units.contains_key(k)) else {
        return Ok(true);
    };
    // A null record is fatal 0x1F.
    let q = cx
        .inputs
        .objclient
        .quest_flags
        .ok_or(HandlerError::Fatal(0x1F))?;
    if !quest_bit(&q, RITE_QUEST, 0) || quest_bit(&q, RITE_QUEST, 4) {
        return Ok(true);
    }
    if distance_to(cx, p)? >= 25 {
        return Ok(true);
    }
    // P's path stops. Every client player has a
    // path (P+0x2C).
    if let Some(pu) = cx.w.units.get_mut(&p) {
        pu.path_stopped = true;
    }
    // The local-player command 0x13 on (2, GUID).
    let guid = cx.unit.key.guid;
    let outs = interact::command_13(cx.w, cx.inputs, p, guid)?;
    cx.out.extend(outs);
    cx.set_mode(2)?;
    cx.refresh()?;
    Ok(true)
}

/// The day-period object refresh with (U, 0): only objects with
/// `EnvEffect` ≠ 0; the
/// period type p of the client act's environment (+0x04; no record → 0,
/// no act → fatal 0x547): p 1–3 puts mode 0 in mode 1; p 0 puts modes 1
/// and 2 in mode 0; each change refreshes the graphics, re-inits the
/// animation and sets unit flag 0x2 from `Selectable<mode>`; the light
/// follows `Lit<mode>` (p 1–3 whatever the mode; p 0 only on a change).
/// p > 3 → fatal 0x66. Returns whether the object changed.
/// TODO: p 0 with a mode ≥ 3 is not settled; nothing changes.
pub fn day_refresh(cx: &mut Cx<'_>) -> Result<bool, HandlerError> {
    if cx.row.env_effect == 0 {
        return Ok(false);
    }
    if cx.w.act.is_none() {
        return Err(HandlerError::Fatal(0x547));
    }
    let p = cx.w.environment.map_or(0, |e| e.kind);
    let mode = cx.u()?.mode;
    let changed = match p {
        1..=3 => {
            if mode == 0 {
                period_mode(cx, 1)?;
            }
            true
        }
        0 if matches!(mode, 1 | 2) => {
            period_mode(cx, 0)?;
            true
        }
        0 => false,
        _ => return Err(HandlerError::Fatal(0x66)),
    };
    if changed {
        let m = cx.u()?.mode as usize;
        let lit = *cx.row.lit.get(m).ok_or(HandlerError::Invalid(
            "object mode past the eight objects.txt modes",
        ))?;
        let rgb = cx.row.rgb;
        cx.fx(ObjFx::Light {
            unit: cx.unit,
            lit,
            rgb,
        });
    }
    Ok(changed)
}

/// One mode change of the day refresh.
fn period_mode(cx: &mut Cx<'_>, m: u32) -> Result<(), HandlerError> {
    let sel = cx.row.selectable[m as usize] != 0;
    cx.u()?.mode = m;
    cx.refresh()?;
    cx.reinit()?;
    cx.u()?.flag_2 = Some(sel);
    Ok(())
}

/// The 500 ms re-arm of `ClientFn` 14 and 15: T := `GetTickCount` +
/// 500 (a second read, edge case 8; D2RUST: the update's one `now`).
fn rearm_500(cx: &mut Cx<'_>) -> Result<(), HandlerError> {
    let now = cx.now();
    cx.u()?.interact_ms = now.wrapping_add(500);
    Ok(())
}

/// `ClientFn` 14 (39 `fire`).
fn bonfire(cx: &mut Cx<'_>) -> Result<bool, HandlerError> {
    if cx.now() > cx.u()?.interact_ms {
        day_refresh(cx)?;
        rearm_500(cx)?;
    }
    Ok(true)
}

/// `ClientFn` 15 (558 `fana`): overlay 72 on in mode 0 (a new
/// record every 500 ms, no duplicate test), removed in other modes.
fn anya(cx: &mut Cx<'_>) -> Result<bool, HandlerError> {
    if cx.now() > cx.u()?.interact_ms {
        let unit = cx.unit;
        if cx.u()?.mode == 0 {
            cx.fx(ObjFx::OverlayCreate {
                unit,
                overlay: OVERLAY_NPCALERT,
                kind: OVERLAY_KIND_LOOP,
            });
        } else {
            cx.fx(ObjFx::OverlayRemove {
                unit,
                overlay: OVERLAY_NPCALERT,
            });
        }
        rearm_500(cx)?;
    }
    Ok(true)
}

/// `ClientFn` 16 (563, 569 Baal's portals).
/// Measured:
/// the rule is implemented as stated; with the generic step's speed it
/// never fires, the generic step reaches mode 2 itself (a class 563
/// object is in mode 2 frame 0 exactly 50 frames after mode 1).
fn baal_portal(cx: &mut Cx<'_>) -> Result<bool, HandlerError> {
    let start1 = i32::from(cx.row.start[1]) * 256;
    let u = cx.u()?;
    if u.mode == 1 && u.frame == start1 {
        u.mode = 2;
        u.frame = 0;
        cx.refresh()?;
        cx.reinit()?;
    }
    Ok(true)
}

/// `ClientFn` 17 (567 `Zoo`).
fn zoo(cx: &mut Cx<'_>) -> Result<bool, HandlerError> {
    let l = cx.w.objclient.latches;
    if !l.zoo {
        return Ok(true);
    }
    if l.zoo_spawned
        && cx
            .w
            .objclient
            .set_c
            .contains_key(&UnitKey::new(MONSTER, l.last_chicken))
    {
        return Ok(true);
    }
    let (x, y) = cx.u()?.cell();
    zoo_spawn(cx, x, y)?;
    Ok(true)
}

/// With (x, y): one step of P's client seed, then 2–4
/// client chickens on the diagonal.
fn zoo_spawn(cx: &mut Cx<'_>, x: u16, y: u16) -> Result<(), HandlerError> {
    if !cx.w.objclient.latches.zoo {
        return Ok(());
    }
    let p = local_player_unchecked(cx, 0x004A_3150)?;
    let lo = step_seed(cx.w.units.get_mut(&p).expect("checked above"))?;
    let n = lo % 3 + 2;
    for i in 0..n {
        let o = (i % 2) as u16;
        let key = create_client_unit(cx.w, MONSTER, CHICKEN, x.wrapping_add(o), y.wrapping_add(o));
        let l = &mut cx.w.objclient.latches;
        l.last_chicken = key.map_or(u32::MAX, |k| k.guid);
        l.zoo_spawned = true;
    }
    Ok(())
}

/// `ClientFn` 18 (568 `Keeper`): two draws per firing.
fn keeper(cx: &mut Cx<'_>) -> Result<bool, HandlerError> {
    let now = cx.now();
    if now > cx.u()?.interact_ms {
        if cx.step()? % 100 < 10 {
            cx.out.push(Output::ObjectSound(ObjSound::Request {
                id: KEEPER_SOUND,
                unit: cx.unit,
            }));
        }
        let r = cx.step()?;
        cx.u()?.interact_ms = now.wrapping_add((r % 60) * 1_000);
    }
    Ok(true)
}

//! Per-update unit sounds: footsteps, the monster `Neutral` voice, `Init`
//! and `Flee`. Per unit and update the order is: neutral, then
//! footsteps (Randomness).

use super::modes::skill_voice;
use super::{class_record, sid, uniform_gap, Ctx, TriggerError, Unit, UnitSound, MONSTER, PLAYER};

/// Whether the per-update code calls the footstep rule for this unit and
/// mode. Players: modes with movement entry 1:
/// 2 WL, 3 RN, 6 TW, 19 KB. Monsters: 2 WL, 15 RN; class 15 (`foulcrow1`)
/// also 1; class 110 (`vulture1`) only 8.
pub fn footstep_called(unit_type: u8, class: i32, mode: u8) -> bool {
    match unit_type {
        PLAYER => matches!(mode, 2 | 3 | 6 | 19),
        MONSTER => match class {
            110 => mode == 8,
            15 => matches!(mode, 1 | 2 | 15),
            _ => matches!(mode, 2 | 15),
        },
        _ => false,
    }
}

/// Circular distance of `x` and `o` modulo `step`, with the footstep
/// integer types: when step is a power of two or 0, both are masked
/// with step − 1 (two's complement; step 0 masks with 0xFFFFFFFF, no
/// reduction); otherwise both are reduced by signed division (the
/// remainder has the sign of the value). Then the minimum, by signed
/// compares, of |b − a|, |b − a − step|, |a − b − step| in signed 32-bit
/// wrapping arithmetic.
pub fn circular_distance(x: i32, o: u32, step: u32) -> i32 {
    let red = |v: i32| {
        if step == 0 || step.is_power_of_two() {
            (v as u32 & step.wrapping_sub(1)) as i32
        } else {
            v.wrapping_rem(step as i32)
        }
    };
    let (a, b, st) = (red(x), red(o as i32), step as i32);
    let d1 = b.wrapping_sub(a).wrapping_abs();
    let d2 = b.wrapping_sub(a).wrapping_sub(st).wrapping_abs();
    let d3 = a.wrapping_sub(b).wrapping_sub(st).wrapping_abs();
    d1.min(d2).min(d3)
}

/// Floor lookup result for the material rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Floor {
    /// U has no room: k = 0.
    NoRoom,
    /// No tile found in U's room or the rooms near it: the default.
    NotFound,
    /// A tile was found; its DT1 material flags.
    Tile(u16),
}

/// Footstep material k. `material1` = `soundenviron.Material 1`
/// of the current environment.
pub fn footstep_material(material1: i32, floor: Floor) -> u8 {
    let default = if (1..=6).contains(&material1) {
        material1 as u8
    } else {
        1
    };
    match floor {
        Floor::NoRoom => 0,
        Floor::NotFound => default,
        Floor::Tile(f) => {
            for (bit, k) in [
                (0x20, 1),
                (0x08, 2),
                (0x10, 3),
                (0x40, 4),
                (0x80, 6),
                (0x400, 5),
            ] {
                if f & bit != 0 {
                    return k;
                }
            }
            default
        }
    }
}

/// Footstep (U), material `k` from
/// [`footstep_material`], with these integer types: s = U+0x4C as
/// signed 16-bit; `n × s` a 32-bit product used as an unsigned divisor;
/// step = F / n unsigned; `f ± s` signed 32-bit wrapping sums; the step
/// test signed; the elapsed tests unsigned.
pub fn footstep(cx: &mut Ctx, u: &Unit, us: &mut UnitSound, k: u8) -> Result<(), TriggerError> {
    // Step 2.
    let s = i32::from(u.speed as i16);
    if s == 0 {
        return Ok(());
    }
    let (mut n, mut o, mut layer, mut p) = (2u32, 0u32, 0i32, 100u32);
    let monster = u.identity_type == MONSTER;
    if monster {
        if let Some(r) = u.monsounds {
            n = r.fscnt;
            o = r.fsoff.wrapping_mul(256);
            layer = sid(r.footsteplayer);
            p = r.fsprb;
        }
    }
    if n == 0 {
        return Ok(());
    }
    // Step 3.
    let frames = u.frame_count;
    let f = u.frame as i32;
    let divisor = n.wrapping_mul(s as u32);
    if divisor == 0 {
        return Err(TriggerError::FootstepStep { frames, count: n });
    }
    let period =
        cx.s.tick_rate()
            .ticks_for_ms((frames / divisor).saturating_mul(40));
    let s = cx.s.tick_rate().share(s, cx.c as i32);
    let elapsed = cx.c.wrapping_sub(us.last_footstep);
    if n > 1 && elapsed < period.wrapping_mul(2) / 3 {
        return Ok(());
    }
    // Step 4.
    let step = frames / n;
    let d = |x: i32| circular_distance(x, o, step);
    if !(d(f) < d(f.wrapping_add(s)) && d(f) < d(f.wrapping_sub(s))) {
        return Ok(());
    }
    // Step 5.
    let mut id = if monster {
        u.monsounds.map_or(0, |r| sid(r.footstep))
    } else {
        class_record(u.class)?.footstep_base
    };
    if id == 0 {
        return Ok(());
    }
    if (2..=6).contains(&k) {
        id += 4 * (k as i32 - 1);
    }
    let running = if monster { u.mode == 15 } else { u.mode == 3 };
    if running {
        id += 24;
    }
    // Step 6.
    let h = cx.req(id, Some(u.key), 0);
    if h != 0 {
        let mut v = if u.is_local { 255 } else { 200 };
        if elapsed > period.wrapping_mul(3) / 2 {
            v = v * 160 / 255;
        }
        cx.s.set_volume(h, v);
    }
    // Step 7.
    if layer != 0 && cx.s.roll(100) < p {
        cx.req(layer, Some(u.key), 0);
    }
    us.last_footstep = cx.c;
    Ok(())
}

/// Town levels.
pub const TOWN_LEVELS: [i32; 5] = [1, 40, 75, 103, 109];

/// `Neutral` idle voice (monsters).
pub fn neutral(cx: &mut Ctx, u: &Unit, us: &mut UnitSound) {
    let b = u.base_class;
    let mode_ok =
        matches!(u.mode, 1 | 2 | 15) || (b == 110 && u.mode == 8) || (b == 136 && u.mode == 11);
    if !mode_ok {
        return;
    }
    let Some(r) = u.monsounds else {
        return;
    };
    let neutral = sid(r.neutral);
    if neutral == 0 {
        return;
    }
    // 1. U already has a request in the group (false while the sound system is off).
    let in_group = super::request_in_group(&*cx.s, u.key, neutral);
    let c = cx.c;
    let ok = !in_group
        && c.wrapping_sub(cx.g.last_idle_any)
            >= cx
                .s
                .tick_rate()
                .ticks_for_ms(cx.g.idle_gap.saturating_mul(40))
        && c.wrapping_sub(us.last_voice)
            >= cx.s.tick_rate().ticks_for_ms(r.neutime.saturating_mul(40))
        && c.wrapping_sub(us.last_idle)
            >= cx.s.tick_rate().ticks_for_ms(r.neutime.saturating_mul(40))
        && !u.in_town
        && u.near_local;
    // 6.
    if !ok || cx.s.tick_rate().share(1, c as i32) == 0 || cx.s.roll(15) != 0 {
        return;
    }
    cx.req(neutral, Some(u.key), 0);
    cx.g.last_idle_any = c;
    us.last_idle = c;
    cx.g.idle_gap = uniform_gap(cx);
}

/// `Init` voice. Called once per client monster creation (S→C 0xAC
/// AssignMonster and the other creations); no path plays it on first
/// sight. Which creations reach it is the client model's business.
pub fn init_voice(cx: &mut Ctx, u: &Unit, us: &mut UnitSound) {
    let Some(r) = u.monsounds else {
        return;
    };
    let init = sid(r.init);
    if init != 0 && !u.dying && us.last_idle == 0 {
        cx.req(init, Some(u.key), 0);
        cx.g.last_idle_any = cx.c;
        us.last_idle = cx.c;
        cx.g.idle_gap = uniform_gap(cx);
    } else if u.mode == 8 {
        skill_voice(cx, u, us, r, 8);
    }
}

/// Flee voice gap (C).
pub const FLEE_GAP: u32 = 4;

/// `Flee` voice (event 17).
pub fn flee(cx: &mut Ctx, u: &Unit, us: &mut UnitSound) {
    let Some(r) = u.monsounds else {
        return;
    };
    if u.identity_type != MONSTER
        || cx.c.wrapping_sub(cx.g.last_voice_any) < cx.s.tick_rate().ticks_for_ms(FLEE_GAP * 40)
    {
        return;
    }
    let d = 6 + cx.s.roll(3);
    cx.req(sid(r.flee), Some(u.key), d);
    cx.voiced(us);
}

//! Monster thinks, with the init functions of FoulCrowNest
//! (also Sarcophagus and MinionSpawner) and BloodRaven, and SandMaggot's
//! alternate function. Brackets in the comments are the Normal
//! difficulty values.

use crate::game::Game;
use crate::units::{UnitId, UnitType};

use super::tactics::*;
use super::{
    delete_thinks, idle, idle_if_later, idle_raw, install, mode, request_mode, AiCommand, AiHost,
    Ctx, ModeTarget, TickParam,
};

pub(super) fn param<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, u: UnitId, n: usize) -> i32 {
    cx.store.control(u).map_or(0, |c| c.params[n])
}

pub(super) fn set_param<W: AiHost + ?Sized>(cx: &mut Ctx<'_, W>, u: UnitId, n: usize, v: i32) {
    if let Some(c) = cx.store.control_mut(u) {
        c.params[n] = v;
    }
}

pub(super) fn at(t: Option<UnitId>) -> ModeTarget {
    match t {
        Some(t) => ModeTarget::Unit(t),
        None => ModeTarget::Point(0, 0),
    }
}

pub(super) fn a1<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    t: Option<UnitId>,
) {
    mode_at(game, cx, u, mode::ATTACK1, t);
}

pub(super) fn a2<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    t: Option<UnitId>,
) {
    mode_at(game, cx, u, mode::ATTACK2, t);
}

/// `roll(n)` on the unit seed as a signed value.
pub(super) fn roll<W: AiHost + ?Sized>(cx: &mut Ctx<'_, W>, u: UnitId, n: i32) -> i32 {
    cx.world.seed(u).roll(n) as i32
}

/// One raw step, `lo' % 100`.
pub(super) fn pct<W: AiHost + ?Sized>(cx: &mut Ctx<'_, W>, u: UnitId) -> i32 {
    (cx.world.seed(u).step() % 100) as i32
}

/// Skill `n` of the row in its `Sk*mode` at `target`.
pub(super) fn skill_at<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
    n: usize,
    target: ModeTarget,
) {
    let (s, m) = cx.skill(p, n);
    use_skill(game, cx, u, m, s, target);
}

/// T's life percent in Fetish (target mode 1). T = 0 is
/// unreachable (no mode-1 or mode-4 body runs without a target):
/// asserted, not handled.
fn life_of<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, t: Option<UnitId>) -> i32 {
    let t = t.expect("Fetish think without a target (ai.md §2.3)");
    cx.world.life_percent(t)
}

/// `Run` × 100 / `Velocity` − 100 (monstats +52, +50, signed,
/// truncating), clamped to 0..120; 0 when `Velocity` ≤ 0.
pub(super) fn run_bonus<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, p: &TickParam) -> i32 {
    let Some(r) = cx.tables.monstats.get(p.class) else {
        return 0;
    };
    let vel = i32::from(r.velocity as i16);
    if vel <= 0 {
        return 0;
    }
    (i32::from(r.run as i16) * 100 / vel - 100).clamp(0, 120)
}

/// The unit a command names by GUID (param 1) and type (param 2),
/// (the shape FetishShaman
/// copies).
pub(super) fn commanded_unit(game: &Game, cmd: &AiCommand) -> Option<UnitId> {
    let ty = UnitType::ALL
        .get(usize::try_from(cmd.params[2]).ok()?)
        .copied()?;
    game.lists.find_unit(ty, cmd.params[1] as u32)
}

// ---- CorruptRogue ----------------------------------------------

/// CorruptRogue (10).
pub fn corrupt_rogue<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    // 1. The player-count record's difficulty field: game +0x6D clamped
    // to 2.
    let l = 20 - 3 * i32::from(cx.info.difficulty.min(2));
    let run = |game: &mut Game, cx: &mut Ctx<'_, W>| {
        let speed = cx.aip(p, 4);
        set_velocity(cx, u, 13, speed, 0);
        move_steps(game, cx, u, t, true, 3);
    };
    // 2.
    if p.distance > l {
        run(game, cx);
        return;
    }
    // 3.
    if p.combat {
        if cx.chance(u, cx.aip(p, 3)) {
            a1(game, cx, u, t);
        } else {
            idle(game, cx, u, cx.aip(p, 2));
        }
        return;
    }
    // 4.
    if !cx.chance(u, cx.aip(p, 1)) {
        idle(game, cx, u, cx.aip(p, 2));
        return;
    }
    if cx.chance(u, cx.aip(p, 5)) {
        run(game, cx);
    } else {
        walk_to(game, cx, u, t, 7);
    }
}

// ---- SkeletonBow -----------------------------------------------

/// SkeletonBow (37).
pub fn skeleton_bow<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    // 1.
    if cx.ai_state_set(u) {
        if let (Some(s), _, _) = cx.world.secondary_target(game, u, cx.store) {
            a1(game, cx, u, Some(s));
            return;
        }
    }
    // 2.
    let (s, e, _) = cx.world.secondary_target(game, u, cx.store);
    // 3.
    let Some(s) = s.filter(|_| e < 20) else {
        if cx.chance(u, cx.aip(p, 3)) {
            // T = 0 is unreachable (target mode 1):
            // asserted, not handled.
            let tt = t.expect("SkeletonBow think without a target (ai.md §2.3)");
            let (a, b) = (cx.aip(p, 4), cx.aip(p, 5));
            walk_in_radius(game, cx, u, tt, a, b);
        } else {
            idle(game, cx, u, 20);
        }
        return;
    };
    // 4.
    if cx.chance(u, cx.aip(p, 1)) {
        a1(game, cx, u, Some(s));
    } else if pct(cx, u) < 20 {
        circle(game, cx, u, t, 3, false);
    } else {
        idle(game, cx, u, cx.aip(p, 2));
    }
}

// ---- FoulCrowNest ----------------------------------------------

/// FoulCrowNest's init (also Sarcophagus 45, MinionSpawner
/// 121): AI param 0 := frame, param 1 := 0.
pub fn nest_init<W: AiHost + ?Sized>(game: &Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    set_param(cx, u, 0, game.frame);
    set_param(cx, u, 1, 0);
}

/// Unit flag 0x20000 (unit +0xC4): no drop.
const FLAG_NO_DROP: u32 = 0x2_0000;
/// crownest1, the footprint test's class.
const CROWNEST1: i32 = 206;

/// FoulCrowNest (43).
pub fn foul_crow_nest<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    // 1.
    if p.distance > 20 {
        idle(game, cx, u, 25);
        return;
    }
    // 2.
    if param(cx, u, 1) >= cx.aip(p, 3) {
        cx.world.set_unit_flag(u, FLAG_NO_DROP);
        request_mode(game, cx, u, mode::DEATH, ModeTarget::Point(0, 0));
        return;
    }
    // 3.
    let (s1, _) = cx.skill(p, 1);
    if s1 >= 0
        && game.frame.wrapping_sub(param(cx, u, 0)).wrapping_abs() >= game.ticks(cx.aip(p, 1))
    {
        set_param(cx, u, 0, game.frame);
        let class = if cx.tables.monstats.len() > CROWNEST1 as usize {
            CROWNEST1
        } else {
            -1
        };
        let room = game.lists.unit(u).and_then(|e| e.room());
        let (x, y) = cx.world.position(u);
        if cx.world.footprint_ok(game, class, room, x, y) {
            set_param(cx, u, 1, param(cx, u, 1).wrapping_add(1));
            skill_at(game, cx, u, p, 1, at(p.target));
            return;
        }
    }
    // 4.
    let n = (cx.world.seed(u).step() % 10) as i32 + 20;
    idle(game, cx, u, n);
}

// ---- BloodRaven ------------------------------------------------

/// BloodRaven's init: AI param 2 := 0.
pub fn blood_raven_init<W: AiHost + ?Sized>(cx: &mut Ctx<'_, W>, u: UnitId) {
    set_param(cx, u, 2, 0);
}

/// BloodRaven (59).
pub fn blood_raven<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let mut d = p.distance;
    let difficulty = i32::from(cx.info.difficulty);
    // 1.
    let mut home = find_command(cx, u, 10, false);
    if home.is_none() {
        let (x, y) = cx.world.position(u);
        // Params 3, 4 are uninitialised stack in the base game and unused.
        copy_command(
            cx,
            u,
            AiCommand {
                params: [10, x, y, 0, 0],
            },
        );
        home = find_command(cx, u, 10, false);
    }
    // 2.
    let mut h = 0;
    if let Some(k) = home {
        let hp = command_mut(cx, u, k).map_or([0; 5], |c| c.params);
        let (hx, hy) = (hp[1], hp[2]);
        // T = 0 is unreachable (target mode 1): asserted.
        let tt = t.expect("BloodRaven think without a target (ai.md §2.3)");
        h = half_size_distance(cx, tt, hx, hy);
        if d > 45 {
            idle(game, cx, u, 5);
            return;
        }
        let own = half_size_distance(cx, u, hx, hy);
        if h >= 50 || own > 50 {
            set_param(cx, u, 2, 1);
            set_velocity(cx, u, 7, 100, 0);
            if run_to_point(game, cx, u, hx, hy) {
                return;
            }
            delete_thinks(game, u);
        }
        if param(cx, u, 2) != 0 && own > 5 {
            set_velocity(cx, u, 7, 100, 0);
            if run_to_point(game, cx, u, hx, hy) {
                return;
            }
            delete_thinks(game, u);
        }
        set_param(cx, u, 2, 0);
    }
    // 3.
    if d > 20 && h < 50 {
        d = (d / 2).max(12);
        set_velocity(cx, u, 7, 100, 0);
        if run_near(game, cx, u, t, d) {
            return;
        }
        delete_thinks(game, u);
    }
    // 4.
    let chance = param(cx, u, 0).wrapping_add(3);
    set_param(cx, u, 0, chance);
    let (s1, m1) = cx.skill(p, 1);
    if s1 >= 0 && !p.combat && param(cx, u, 1) < 2 * difficulty + 8 && roll(cx, u, 100) < chance {
        let l = roll(cx, u, 15) + 5;
        let (mut dx, mut dy) = if cx.world.seed(u).step() & 1 == 1 {
            (l, roll(cx, u, l))
        } else {
            let r = roll(cx, u, l);
            (r, l)
        };
        if cx.world.seed(u).step() & 1 == 1 {
            dx = -dx;
        }
        if cx.world.seed(u).step() & 1 == 1 {
            dy = -dy;
        }
        // T = 0 is unreachable (target mode 1): asserted.
        let tt = t.expect("BloodRaven think without a target (ai.md §2.3)");
        let (tx, ty) = cx.world.position(tt);
        use_skill(
            game,
            cx,
            u,
            m1,
            s1,
            ModeTarget::Point(tx.wrapping_add(dx), ty.wrapping_add(dy)),
        );
        set_param(cx, u, 1, param(cx, u, 1).wrapping_add(1));
        set_param(cx, u, 0, 0);
        return;
    }
    // 5.
    if d > 5 {
        if pct(cx, u) < 5 && h < 50 {
            set_velocity(cx, u, 7, 100, 0);
            run_near(game, cx, u, t, 12);
            return;
        }
        let (s, _, _) = cx.world.secondary_target(game, u, cx.store);
        if let Some(s) = s {
            if !cx.ai_state_set(u) && roll(cx, u, 100) < 80 {
                let (s2, m2) = cx.skill(p, 2);
                if s2 >= 0 && roll(cx, u, 100) < 10 * (difficulty + 4) {
                    use_skill(game, cx, u, m2, s2, ModeTarget::Unit(s));
                } else {
                    a1(game, cx, u, Some(s));
                }
                return;
            }
        }
        set_velocity(cx, u, 0, 50, 0);
        if circle(game, cx, u, t, 4, true) {
            return;
        }
    }
    // 6.
    if pct(cx, u) < 30 && d < 12 {
        set_velocity(cx, u, 7, 100, 0);
        let n = i32::from((12 - d) as u8);
        if escape(game, cx, u, t, n, false, true) {
            return;
        }
        delete_thinks(game, u);
    }
    // 7.
    a1(game, cx, u, t);
}

// ---- SkeletonMage ----------------------------------------------

/// SkeletonMage (64).
pub fn skeleton_mage<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let (s, e, _) = cx.world.secondary_target(game, u, cx.store);
    let aip2 = cx.aip(p, 2);
    let steps = i32::from(aip2 as u8);
    // 1.
    if let Some(s) = s {
        if e > aip2 && cx.chance(u, cx.aip(p, 3)) {
            set_velocity(cx, u, 0, 10, 0);
            move_steps(game, cx, u, Some(s), false, steps);
            return;
        }
        if e <= cx.aip(p, 4) && cx.chance(u, cx.aip(p, 5)) {
            set_velocity(cx, u, 0, 25, 0);
            if !escape(game, cx, u, Some(s), 5, true, false) {
                a1(game, cx, u, t);
            }
            return;
        }
        if e < cx.aip(p, 6) && cx.chance(u, cx.aip(p, 1)) {
            a1(game, cx, u, Some(s));
            return;
        }
    }
    // 2.
    if e > aip2 && cx.chance(u, cx.aip(p, 3)) {
        set_velocity(cx, u, 0, 10, 0);
        move_steps(game, cx, u, t, false, steps);
        return;
    }
    // 3.
    if cx.chance(u, cx.aip(p, 7)) {
        circle(game, cx, u, t, 4, false);
    } else {
        idle(game, cx, u, cx.aip(p, 8));
    }
}

// ---- Arach -----------------------------------------------------

/// State 22 (in Arach step 2.2).
const STATE_22: u16 = 22;

/// Arach (26).
pub fn arach<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    let t = p.target;
    let l = cx.world.life_percent(u);
    let lunge = |game: &mut Game, cx: &mut Ctx<'_, W>| {
        walk_to_method13(game, cx, u, t, 0);
    };
    // 1.
    if param(cx, u, 0) == 1 {
        set_param(cx, u, 1, 0);
        if l > 75 {
            set_param(cx, u, 0, 0);
            if cx.chance(u, cx.aip(p, 3)) {
                set_param(cx, u, 0, 2);
                lunge(game, cx);
            } else {
                circle(game, cx, u, t, 6, false);
            }
            return;
        }
        let aip1 = cx.aip(p, 1);
        if p.combat && aip1 > 25 && roll(cx, u, 100) < aip1 - 25 {
            a1(game, cx, u, t);
            return;
        }
        if p.distance >= cx.aip(p, 4) && !cx.ai_state_set(u) {
            set_param(cx, u, 0, 0);
            circle(game, cx, u, t, 12, false);
            return;
        }
        escape(game, cx, u, t, 4, false, false);
        return;
    }
    // 2.
    if p.combat {
        set_param(cx, u, 0, 2);
        if cx.chance(u, cx.aip(p, 1)) {
            a1(game, cx, u, t);
            return;
        }
        if l < cx.aip(p, 5) {
            set_param(cx, u, 0, 1);
            let (s1, m1) = cx.skill(p, 1);
            if s1 >= 0 && !cx.world.has_state(u, STATE_22) {
                use_skill(game, cx, u, m1, s1, ModeTarget::Point(0, 0));
            } else {
                escape(game, cx, u, t, 8, false, false);
            }
            return;
        }
        if roll(cx, u, 100) < cx.aip(p, 2) {
            circle(game, cx, u, t, 4, false);
        } else {
            idle(game, cx, u, 15);
        }
        return;
    }
    // 3.
    if cx.ai_state_set(u) || param(cx, u, 1) == 1 {
        set_param(cx, u, 1, 1);
        lunge(game, cx);
        return;
    }
    let mut n = param(cx, u, 2).wrapping_add(1);
    if n > 20 {
        n = 0;
    }
    set_param(cx, u, 2, n);
    set_param(cx, u, 1, 0);
    if n == 1 && roll(cx, u, 100) < cx.aip(p, 3) {
        set_param(cx, u, 1, 1);
        lunge(game, cx);
        return;
    }
    if roll(cx, u, 100) < 20 {
        wander(game, cx, u, 6);
    } else {
        idle(game, cx, u, 15);
    }
}

// ---- Fetish ----------------------------------------------------

/// Fetish (30).
pub fn fetish<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    let t = p.target;
    let l = life_of(cx, t);
    // 1.
    if let Some(k) = current_command(cx, u) {
        if matches!(k.params[0], 1 | 14) {
            if let Some(other) = commanded_unit(game, &k) {
                set_param(cx, u, 0, 0);
                set_param(cx, u, 1, 0);
                set_velocity(cx, u, 13, 50, 0);
                walk_to(game, cx, u, Some(other), 0);
                free_current_command(cx, u);
                return;
            }
        }
        free_current_command(cx, u);
    }
    let walk = |game: &mut Game, cx: &mut Ctx<'_, W>| {
        set_velocity(cx, u, 13, 50, 0);
        walk_to(game, cx, u, t, 7);
    };
    match param(cx, u, 0) {
        // 2.
        0 => {
            if !p.combat {
                walk(game, cx);
                return;
            }
            set_param(cx, u, 1, 0);
            set_param(cx, u, 0, 1);
            if cx.chance(u, cx.aip(p, 1)) {
                a1(game, cx, u, t);
            } else {
                idle(game, cx, u, cx.aip(p, 2));
            }
        }
        // 3.
        1 => {
            let n = param(cx, u, 1).wrapping_add(1);
            set_param(cx, u, 1, n);
            if n > cx.aip(p, 3) && l > cx.aip(p, 4) {
                set_param(cx, u, 0, 2);
                set_param(cx, u, 1, 0);
                set_velocity(cx, u, 2, 50, 0);
                escape(game, cx, u, t, 14, true, false);
                return;
            }
            if !p.combat {
                walk(game, cx);
            } else if roll(cx, u, 100) < cx.aip(p, 1) {
                a1(game, cx, u, t);
            } else {
                idle(game, cx, u, cx.aip(p, 2));
            }
        }
        // 4.
        2 => {
            if p.distance > 12 {
                let n = param(cx, u, 1).wrapping_add(1);
                set_param(cx, u, 1, n);
                if n > 1 {
                    set_param(cx, u, 0, 0);
                    set_param(cx, u, 1, 0);
                }
                if pct(cx, u) < 20 {
                    circle(game, cx, u, t, 4, false);
                } else {
                    idle(game, cx, u, 10);
                }
                return;
            }
            set_velocity(cx, u, 2, 50, 0);
            if escape(game, cx, u, t, 14, true, false) {
                return;
            }
            set_param(cx, u, 0, 0);
            set_param(cx, u, 1, 0);
            idle(game, cx, u, 10);
        }
        _ => idle(game, cx, u, 10),
    }
}

// ---- Vampire ---------------------------------------------------

/// Vampire (28).
pub fn vampire<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    let t = p.target;
    let d = p.distance;
    let f = cx.aip(p, 5);
    let l = cx.world.life_percent(u);
    // "Bolt at X": `Skill1` or `Skill4`, neither tested for < 0 (edge 15).
    let bolt = |game: &mut Game, cx: &mut Ctx<'_, W>, x: UnitId| {
        let n = if roll(cx, u, 100) < 50 { 1 } else { 4 };
        skill_at(game, cx, u, p, n, ModeTarget::Unit(x));
    };
    // "Upgrade": true when it ended the think.
    let upgrade = |game: &mut Game, cx: &mut Ctx<'_, W>| {
        for (bit, n) in [(2, 2), (4, 3)] {
            if f & bit != 0 && param(cx, u, 2) <= 0 && roll(cx, u, 100) < cx.aip(p, 4) {
                skill_at(game, cx, u, p, n, at(t));
                set_param(cx, u, 2, 11);
                return true;
            }
        }
        false
    };
    // 1.
    if param(cx, u, 2) > 0 {
        set_param(cx, u, 2, param(cx, u, 2) - 1);
    }
    let (s, e, _) = cx.world.secondary_target(game, u, cx.store);
    // 2.
    if cx.ai_state_set(u) {
        if param(cx, u, 0) == 0 {
            set_param(cx, u, 0, 1);
        }
        if d < 30 && d > param(cx, u, 1) {
            set_param(cx, u, 1, d);
        }
        if p.combat {
            if pct(cx, u) > 30 || f & 1 == 0 {
                a1(game, cx, u, t);
            } else if let Some(tt) = t {
                bolt(game, cx, tt);
            }
            return;
        }
    }
    // 3.
    if param(cx, u, 0) == 2 {
        if l >= 75 {
            set_param(cx, u, 0, 1);
            walk_to(game, cx, u, t, 7);
            return;
        }
        if d < 14 || d <= param(cx, u, 1) {
            let v = run_bonus(cx, p);
            set_velocity(cx, u, 0, v, 0);
            if escape(game, cx, u, t, 8, true, false) {
                return;
            }
        }
        if d >= cx.aip(p, 3) || roll(cx, u, 100) >= cx.aip(p, 2) {
            idle(game, cx, u, 15);
            return;
        }
        if upgrade(game, cx) {
            return;
        }
        match s {
            Some(s) if f & 1 != 0 && e <= 20 => bolt(game, cx, s),
            _ => {
                circle(game, cx, u, t, 4, false);
            }
        }
        return;
    }
    // 4.1.
    if l < 33 {
        set_param(cx, u, 0, 2);
        if escape(game, cx, u, t, 8, false, false) {
            return;
        }
    }
    // 4.2.
    if p.combat {
        set_param(cx, u, 0, 1);
        if cx.chance(u, cx.aip(p, 1)) {
            let Some(s) = s.filter(|_| f & 1 != 0) else {
                a1(game, cx, u, t);
                return;
            };
            if roll(cx, u, 100) > 30 {
                a1(game, cx, u, t);
                return;
            }
            if e <= 20 {
                bolt(game, cx, s);
                return;
            }
        }
        if roll(cx, u, 100) < 33 {
            circle(game, cx, u, t, 4, false);
        } else {
            idle(game, cx, u, 10);
        }
        return;
    }
    // 4.3.
    if d >= cx.aip(p, 3) {
        if param(cx, u, 0) == 1 {
            walk_to(game, cx, u, t, 7);
        } else {
            idle(game, cx, u, 15);
        }
        return;
    }
    // 4.4.
    set_param(cx, u, 0, 1);
    if roll(cx, u, 100) >= cx.aip(p, 2) {
        if d > 20 {
            walk_to(game, cx, u, t, 7);
        } else if d < 9 && roll(cx, u, 100) < 50 {
            escape(game, cx, u, t, 8, false, false);
        } else if roll(cx, u, 100) < 50 {
            circle(game, cx, u, t, 4, false);
        } else {
            idle(game, cx, u, 10);
        }
        return;
    }
    // 4.5.
    if upgrade(game, cx) {
        return;
    }
    // 4.6.
    match s {
        Some(s) if f & 1 != 0 && e <= 20 => {
            if roll(cx, u, 100) < 75 {
                bolt(game, cx, s);
            } else {
                circle(game, cx, u, t, 4, false);
            }
        }
        _ => {
            walk_to(game, cx, u, t, 7);
        }
    }
}

// ---- Bighead ---------------------------------------------------

/// Bighead (4).
pub fn bighead<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    let t = p.target;
    let d = p.distance;
    // 1.
    if !p.combat && cx.ai_state_set(u) {
        a2(game, cx, u, t);
        return;
    }
    let l = cx.world.life_percent(u);
    // 2.
    if l >= cx.aip(p, 1) {
        if p.combat {
            a1(game, cx, u, t);
            return;
        }
        if d < 15 {
            let (s, _, _) = cx.world.secondary_target(game, u, cx.store);
            if s.is_some() && roll(cx, u, 100) < cx.aip(p, 3) {
                a2(game, cx, u, t);
                return;
            }
        }
        walk_to(game, cx, u, t, 7);
        return;
    }
    // 3.
    if d < 3 {
        set_velocity(cx, u, 0, 50, 0);
        if !escape(game, cx, u, t, 5, true, false) {
            a2(game, cx, u, t);
        }
        return;
    }
    if d > 15 {
        move_steps(game, cx, u, t, false, 6);
        return;
    }
    let (s, _, _) = cx.world.secondary_target(game, u, cx.store);
    if s.is_some() && roll(cx, u, 100) < cx.aip(p, 4) {
        a2(game, cx, u, t);
        return;
    }
    if roll(cx, u, 100) < cx.aip(p, 2) {
        circle(game, cx, u, t, 3, false);
    } else {
        idle(game, cx, u, 10);
    }
}

// ---- BloodHawk -------------------------------------------------

/// BloodHawk (5).
pub fn blood_hawk<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let back_off = |game: &mut Game, cx: &mut Ctx<'_, W>| {
        let speed = cx.aip(p, 4);
        set_velocity(cx, u, 0, speed, 0);
        if !escape(game, cx, u, t, 4, true, false) {
            a1(game, cx, u, t);
        }
    };
    // 1.
    if param(cx, u, 0) == 1 && p.combat {
        set_param(cx, u, 0, 0);
        a1(game, cx, u, t);
        return;
    }
    // 2.
    set_param(cx, u, 0, 0);
    // 3.
    if p.combat {
        if cx.chance(u, cx.aip(p, 3)) {
            a1(game, cx, u, t);
        } else {
            back_off(game, cx);
        }
        return;
    }
    // 4.
    if cx.chance(u, cx.aip(p, 1)) {
        let speed = cx.aip(p, 5);
        set_velocity(cx, u, 0, speed, p.distance);
        set_param(cx, u, 0, 1);
        walk_to(game, cx, u, t, 0);
        return;
    }
    if p.distance > 3 {
        if cx.chance(u, cx.aip(p, 2)) {
            set_velocity(cx, u, 0, -50, 0);
            wander(game, cx, u, 4);
        } else {
            set_velocity(cx, u, 0, 0, 0);
            wander(game, cx, u, 3);
        }
        return;
    }
    back_off(game, cx);
}

// ---- HellMeteor ------------------------------------------------

/// HellMeteor (33).
pub fn hell_meteor<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let (s1, m1) = cx.skill(p, 1);
    if s1 >= 0 && cx.chance(u, cx.aip(p, 1)) {
        let aip3 = cx.aip(p, 3);
        let (ox, oy) = cx.world.position(u);
        let x = ox - aip3 + roll(cx, u, 2 * aip3);
        let y = oy - aip3 + roll(cx, u, 2 * aip3);
        use_skill(game, cx, u, m1, s1, ModeTarget::Point(x, y));
        return;
    }
    idle(game, cx, u, cx.aip(p, 2));
}

// ---- SandRaider ------------------------------------------------

/// SandRaider (8).
pub fn sand_raider<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let aip5 = cx.aip(p, 5);
    let blue = cx.aip(p, 6) == 1;
    let rest = |game: &mut Game, cx: &mut Ctx<'_, W>| {
        let k = (24 - aip5).max(6);
        if param(cx, u, 0) > aip5 + k {
            set_param(cx, u, 0, 0);
            set_param(cx, u, 1, 0);
        }
        idle(game, cx, u, 15);
    };
    // 1.
    if param(cx, u, 0) == 0 {
        cx.world.set_state(game, u, 90, false);
        cx.world.set_state(game, u, 91, false);
        set_param(cx, u, 1, 0);
    }
    // 2.
    let n = param(cx, u, 0).wrapping_add(1);
    set_param(cx, u, 0, n);
    if n == aip5 {
        cx.world.start_overlay(u, if blue { 150 } else { 46 });
        // `aidel` of the difficulty, a byte read with no game-type gate.
        let aidel = cx
            .tables
            .monstats
            .get(p.class)
            .map_or(0, |r| match cx.info.difficulty {
                0 => r.aidel,
                1 => r.aidel_n,
                _ => r.aidel_h,
            });
        idle_raw(game, cx, u, game.ticks(i32::from(aidel)).wrapping_add(1));
        return;
    }
    // 3.
    if n > aip5 {
        cx.world
            .set_state(game, u, if blue { 90 } else { 91 }, true);
        set_param(cx, u, 1, 1);
    }
    // 4.
    if param(cx, u, 2) < 7 && cx.world.life_percent(u) < cx.aip(p, 1) {
        if let Some(m) = cx.world.nearest_evil_monster(game, u) {
            walk_to(game, cx, u, Some(m), 0);
            return;
        }
        set_param(cx, u, 2, param(cx, u, 2).wrapping_add(1));
    }
    // 5.
    if p.distance > 4 && param(cx, u, 1) == 0 && cx.chance(u, cx.aip(p, 2)) {
        circle(game, cx, u, t, 0, false);
        return;
    }
    // 6.
    if !p.combat {
        if param(cx, u, 1) == 0 && !cx.chance(u, cx.aip(p, 4)) {
            rest(game, cx);
        } else {
            walk_to(game, cx, u, t, 0);
        }
        return;
    }
    // 7.
    let (s1, _) = cx.skill(p, 1);
    if param(cx, u, 1) == 1 && s1 >= 0 {
        skill_at(game, cx, u, p, 1, at(t));
        set_param(cx, u, 0, 0);
        set_param(cx, u, 1, 0);
        return;
    }
    if cx.chance(u, cx.aip(p, 3)) {
        if roll(cx, u, 100) < cx.aip(p, 7) {
            a2(game, cx, u, t);
        } else {
            a1(game, cx, u, t);
        }
        return;
    }
    rest(game, cx);
}

// ---- Baboon ----------------------------------------------------

/// Stat 74 `hpregen`.
const HPREGEN: u16 = 74;

/// Baboon (11).
pub fn baboon<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    let t = p.target;
    let l = cx.world.life_percent(u);
    let v = run_bonus(cx, p);
    let a1_or_a2 = |game: &mut Game, cx: &mut Ctx<'_, W>, attack1: bool| {
        if attack1 {
            a1(game, cx, u, t);
        } else {
            a2(game, cx, u, t);
        }
    };
    // 1.
    if param(cx, u, 0) != 0 {
        let n = param(cx, u, 0) - 1;
        set_param(cx, u, 0, n);
        set_param(cx, u, 1, 0);
        if n == 0 || l > 75 {
            let r = cx.world.stat(u, HPREGEN);
            cx.world
                .set_stat(u, HPREGEN, r.wrapping_sub(param(cx, u, 2)));
        }
        if !p.combat {
            if l > 75 {
                set_param(cx, u, 0, 0);
                set_velocity(cx, u, 13, v, 0);
                walk_to_method13(game, cx, u, t, 7);
                return;
            }
        } else if pct(cx, u) < 33 {
            let first = pct(cx, u) < cx.aip(p, 4);
            a1_or_a2(game, cx, first);
            return;
        }
        // 1.4.
        if p.distance >= 24 && !cx.ai_state_set(u) {
            if roll(cx, u, 100) < 33 {
                circle(game, cx, u, t, 4, false);
            }
            // Bug kept: the idle's neutral request ends the circle walk.
            idle(game, cx, u, 20);
            return;
        }
        // 1.5.
        set_velocity(cx, u, 2, v, 0);
        if escape(game, cx, u, t, 15, true, false) {
            return;
        }
        if !p.combat {
            wander(game, cx, u, 5);
        } else {
            let first = roll(cx, u, 100) < cx.aip(p, 4);
            a1_or_a2(game, cx, first);
        }
        return;
    }
    // 2.1.
    if !p.combat {
        walk_to_method13(game, cx, u, t, 7);
        return;
    }
    // 2.2.
    if cx.ai_state_set(u) {
        if l < cx.aip(p, 1) && roll(cx, u, 100) < 50 {
            let n = roll(cx, u, 5) + 2;
            set_param(cx, u, 0, n);
            let r = cx.world.stat(u, HPREGEN);
            if r != 0 {
                let bonus = cx.aip(p, 5).wrapping_mul(r) / 8;
                set_param(cx, u, 2, bonus);
                cx.world.set_stat(u, HPREGEN, r.wrapping_add(bonus));
            } else {
                set_param(cx, u, 2, 0);
            }
            set_velocity(cx, u, 2, v, 0);
            escape(game, cx, u, t, 15, false, false);
            return;
        }
        if param(cx, u, 1) != 0 && roll(cx, u, 100) < 20 {
            circle(game, cx, u, t, 3, false);
            set_param(cx, u, 1, 0);
            return;
        }
    }
    // 2.3.
    if param(cx, u, 1) != 0 && !cx.chance(u, cx.aip(p, 3)) {
        if roll(cx, u, 100) < cx.aip(p, 2) {
            circle(game, cx, u, t, 3, false);
            set_param(cx, u, 1, 0);
        }
        idle(game, cx, u, 15);
        return;
    }
    // 2.4.
    set_param(cx, u, 1, 1);
    let first = cx.chance(u, cx.aip(p, 4));
    a1_or_a2(game, cx, first);
}

// ---- SandMaggot ------------------------------------------------

/// The "Burrow (X)" step: `Skill2` in `Sk2mode` at X with (0, 0); wait
/// 30; param 1 := frame + aip5; state := 3.
fn burrow<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
    x: Option<UnitId>,
) {
    skill_at(game, cx, u, p, 2, at(x));
    idle_if_later(game, cx, u, 30);
    let until = game.after(cx.aip(p, 5));
    set_param(cx, u, 1, until);
    set_param(cx, u, 0, 3);
}

/// SandMaggot (15).
pub fn sand_maggot<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let state = param(cx, u, 0);
    let (s, e, _) = cx.world.secondary_target(game, u, cx.store);
    let (s1, _) = cx.skill(p, 1);
    let (s2, _) = cx.skill(p, 2);
    let (s3, _) = cx.skill(p, 3);
    let frame = game.frame;
    if state < 3 {
        // 1.
        if t.is_none() && (s.is_none() || e > 10) && frame > param(cx, u, 1) && s2 >= 0 {
            burrow(game, cx, u, p, None);
            return;
        }
    } else if state == 3 {
        // 2.
        if !(t.is_some() || (s.is_some() && e < 16)) {
            idle_if_later(game, cx, u, 20);
            return;
        }
        if frame > param(cx, u, 1) && s1 >= 0 {
            skill_at(game, cx, u, p, 1, at(t));
            idle_if_later(game, cx, u, 25);
            set_param(cx, u, 1, game.after(cx.aip(p, 5)));
            set_param(cx, u, 0, 1);
        } else {
            idle_if_later(game, cx, u, 20);
        }
        return;
    }
    // A state above 3 (never written here) takes the above-ground steps
    // whatever T and S are.
    // 3.1.
    if cx.world.life_percent(u) < 25
        && s2 >= 0
        && e < 7
        && frame > param(cx, u, 1)
        && roll(cx, u, 100) < 20
    {
        burrow(game, cx, u, p, t);
        return;
    }
    // 3.2.
    if p.combat && cx.chance(u, cx.aip(p, 4)) {
        a1(game, cx, u, t);
        return;
    }
    // 3.3.
    if let Some(s) = s {
        if e < 15 && roll(cx, u, 100) < cx.aip(p, 2) {
            a2(game, cx, u, Some(s));
            return;
        }
    }
    // 3.4.
    if pct(cx, u) < 20 {
        circle(game, cx, u, t, 6, false);
        return;
    }
    // 3.5.
    if param(cx, u, 2) < cx.aip(p, 3) && roll(cx, u, 100) < cx.aip(p, 1) {
        if state == 2 && s3 >= 0 {
            set_param(cx, u, 2, param(cx, u, 2).wrapping_add(1));
            set_param(cx, u, 0, 1);
            skill_at(game, cx, u, p, 3, at(t));
            idle_if_later(game, cx, u, 20);
        } else {
            circle(game, cx, u, t, 6, false);
            set_param(cx, u, 0, 2);
        }
        return;
    }
    // 3.6.
    idle_if_later(game, cx, u, 12);
}

/// SandMaggot's alternate. No draws.
pub fn sand_maggot_alt<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let k = get_or_create_command(cx, u, 14, false);
    let kp = k
        .and_then(|k| command_mut(cx, u, k).map(|c| c.params))
        .unwrap_or([0; 5]);
    let (s1, _) = cx.skill(p, 1);
    if kp[4] == 1 && s1 >= 0 {
        let target = cx.world.path_target(u).filter(|&x| x != u);
        skill_at(game, cx, u, p, 1, at(target));
        idle_if_later(game, cx, u, 30);
        if let Some(c) = k.and_then(|k| command_mut(cx, u, k)) {
            c.params[4] = 0;
        }
        return;
    }
    let keep = param(cx, u, 2);
    let state = cx.store.control(u).map_or(0, |c| c.special_state);
    install(game, cx, u, state);
    set_param(cx, u, 2, keep);
    idle_if_later(game, cx, u, 1);
}

// ---- Scarab ----------------------------------------------------

/// Scarab (20).
pub fn scarab<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    let t = p.target;
    let (s1, _) = cx.skill(p, 1);
    // 1.
    let mut k = current_command(cx, u);
    if k.is_none()
        && p.distance < 20
        && minion_owner(game, cx, u) == Some(u)
        && cx.chance(u, cx.aip(p, 5))
    {
        // Params 1–4 are uninitialised stack in the base game and unused.
        let cmd = AiCommand {
            params: [1, 0, 0, 0, 0],
        };
        command_minions(game, cx, u, cmd);
        copy_command(cx, u, cmd);
        k = Some(cmd);
    }
    // 2.
    if k.is_some_and(|k| k.params[0] == 1) {
        if p.combat && s1 >= 0 {
            free_current_command(cx, u);
            skill_at(game, cx, u, p, 1, at(t));
            return;
        }
        set_velocity(cx, u, 2, 100, 0);
        if !walk_to(game, cx, u, t, 0) {
            free_current_command(cx, u);
        }
        return;
    }
    // 3.
    if !p.combat {
        if param(cx, u, 0) != 0 {
            set_velocity(cx, u, 2, 0, 4);
            walk_to(game, cx, u, t, 7);
            if pct(cx, u) > 10 {
                set_param(cx, u, 0, 0);
            }
        } else {
            circle(game, cx, u, t, 0, false);
            set_param(cx, u, 0, 1);
        }
        return;
    }
    // 4.
    if !cx.chance(u, cx.aip(p, 1)) {
        idle(game, cx, u, cx.aip(p, 3));
        return;
    }
    if s1 >= 0 && cx.chance(u, cx.aip(p, 4)) {
        skill_at(game, cx, u, p, 1, at(t));
        return;
    }
    if cx.chance(u, cx.aip(p, 2)) {
        a1(game, cx, u, t);
    } else {
        a2(game, cx, u, t);
    }
}

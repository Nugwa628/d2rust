//! The Act II thinks, BatDemon's alternate and the special-state
//! thinks 10 / 17, 11 (with its init) and 12. Brackets in the
//! comments are the Normal difficulty values.

use crate::game::Game;
use crate::units::{UnitId, UnitType};

use super::bodies::{a1, a2, param, pct, roll, run_bonus, set_param};
use super::common::*;
use super::tactics::*;
use super::target::find_mode1;
use super::{
    delete_thinks, idle, install, mode, schedule_think, AiHost, Ctx, ModeTarget, QuestCall,
    TickParam,
};

/// The current special state of the control.
fn special<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, u: UnitId) -> u32 {
    cx.store.control(u).map_or(0, |c| c.special_state)
}

/// Re-install the AI for the control's current state (the
/// control holds the alternate, so this is the full reset of the AI
/// install).
pub(super) fn reinstall<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    let s = special(cx, u);
    install(game, cx, u, s);
}

/// Install special state 0, delete the thinks and add one at frame + 1.
fn leave_special<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    install(game, cx, u, 0);
    delete_thinks(game, u);
    let at = game.frame.wrapping_add(1);
    schedule_think(game, cx, u, at);
}

// ---- PantherJavelin -------------------------------------------------

/// PantherJavelin (95).
pub fn panther_javelin<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let (s, e, _) = cx.world.secondary_target(game, u, cx.store);
    // 1.
    if e < 8 && cx.chance(u, cx.aip(p, 4)) {
        escape(game, cx, u, t, 16, true, false);
        return;
    }
    // 2.
    if e > cx.aip(p, 6) - 6 && cx.chance(u, cx.aip(p, 1)) {
        wander_near_opt(game, cx, u, t, 4);
        return;
    }
    // 3. aip3 against a squared distance (edge case 7).
    let Some(s) = s.filter(|_| e < cx.aip(p, 6)) else {
        match pack_scan(game, cx, u) {
            Some((m, d)) if d > cx.aip(p, 3) => {
                walk_to(game, cx, u, Some(m), 7);
            }
            _ => idle(game, cx, u, cx.aip(p, 5)),
        }
        return;
    };
    // 4.
    if cx.chance(u, cx.aip(p, 2)) {
        a1(game, cx, u, Some(s));
    } else {
        idle(game, cx, u, cx.aip(p, 5));
    }
}

// ---- GreaterMummy ---------------------------------------------------

/// radament.
const RADAMENT: i32 = 229;
/// State flag groups `hide` and `udead`.
pub(super) const GROUP_HIDE: u8 = 2;
pub(super) const GROUP_UDEAD: u8 = 33;

/// The mummy scan's record R.
#[derive(Default)]
struct MummyScan {
    corpse: Option<UnitId>,
    hurt: Option<UnitId>,
    matches: i32,
    seen: i32,
    max: i32,
    wide: bool,
    normal: bool,
}

/// Callback over scan 1.
fn mummy_scan<W: AiHost + ?Sized>(game: &Game, cx: &Ctx<'_, W>, m: UnitId, r: &mut MummyScan) {
    for v in scan_units(game, m) {
        // 1.
        if v == m
            || !is_monster(game, v)
            || !pairing(cx, m, v)
            || cx.world.alignment(v) == 2
            || cx.world.unit_flags(v) & 0x2 == 0
            || cx.world.has_state_group(v, GROUP_UDEAD)
        {
            continue;
        }
        // 2.
        let row = cx.monstats(cx.world.class(v));
        let undead = row.is_some_and(|r2| r2.lundead || (r.wide && r2.hundead));
        if !undead {
            continue;
        }
        // 3, 4.
        if !(r.normal || !cx.world.is_unique(v)) || sq_dist(cx, m, v) > r.max {
            continue;
        }
        r.seen += 1;
        let am = cx.world.anim_mode(v);
        if am == mode::DEAD && !cx.world.has_state_group(v, GROUP_HIDE) {
            r.matches += 1;
            r.corpse = Some(v);
        } else if life(cx, v) != cx.world.max_life(v) && am != mode::DEATH {
            r.matches += 1;
            r.hurt = Some(v);
        }
    }
}

/// GreaterMummy (22).
pub fn greater_mummy<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    // 1, 2.
    if p.combat && cx.chance(u, cx.aip(p, 1)) {
        a1(game, cx, u, t);
        return;
    }
    if p.distance < 5 && cx.chance(u, cx.aip(p, 1)) {
        a2(game, cx, u, t);
        return;
    }
    // 3.
    let aip5 = cx.aip(p, 5);
    let mut r = MummyScan {
        max: aip5.wrapping_mul(aip5),
        ..MummyScan::default()
    };
    if cx.world.class(u) == fixed_class(cx, RADAMENT) {
        cx.world.quest_call(game, u, QuestCall::RadamentActivated);
        let w = aip5 + 10;
        r.max = w.wrapping_mul(w);
        r.wide = true;
        r.normal = cx.info.difficulty == 0;
    }
    // 4.
    mummy_scan(game, cx, u, &mut r);
    // 5.
    let (s2, m2) = cx.skill(p, 2);
    if let Some(h) = r.hurt.filter(|_| s2 >= 0) {
        if cx.chance(u, cx.aip(p, 3)) {
            cx.world.set_path_target(u, h);
            use_skill(game, cx, u, m2, s2, ModeTarget::Unit(h));
            return;
        }
    }
    // 6. The check runs only after the draw passes.
    if let Some(c) = r.corpse {
        let (s1, _) = cx.skill(p, 1);
        if cx.chance(u, cx.aip(p, 2)) && cx.world.skill_check(game, u, s1, Some(c), 0, 0) {
            cx.world.set_path_target(u, c);
            use_sequence_skill(game, cx, u, s1, ModeTarget::Unit(c));
            return;
        }
    }
    // 7.
    let (s3, m3) = cx.skill(p, 3);
    if s3 >= 0 && cx.chance(u, cx.aip(p, 4)) {
        if let (Some(s), _, _) = cx.world.secondary_target(game, u, cx.store) {
            use_skill(game, cx, u, m3, s3, ModeTarget::Unit(s));
            return;
        }
    }
    // 8.
    if r.seen <= 0 {
        set_velocity(cx, u, 0, 50, 0);
        move_steps(game, cx, u, t, false, 3);
        return;
    }
    // 9.
    if pct(cx, u) >= 50 {
        idle(game, cx, u, 6);
    } else {
        circle(game, cx, u, t, 3, false);
    }
}

// ---- Mummy ----------------------------------------------------------

/// Mummy (21).
pub fn mummy<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    let t = p.target;
    // 1.
    if cx.ai_state_set(u) && !p.combat {
        walk_to(game, cx, u, t, 7);
        return;
    }
    // 2.
    if p.distance > cx.aip(p, 1) {
        if cx.chance(u, cx.aip(p, 2)) {
            wander(game, cx, u, 3);
        } else {
            idle(game, cx, u, cx.aip(p, 5));
        }
        return;
    }
    // 3.
    if !p.combat {
        walk_to(game, cx, u, t, 7);
        return;
    }
    // 4.
    if cx.chance(u, cx.aip(p, 3)) {
        if cx.chance(u, cx.aip(p, 4)) {
            a1(game, cx, u, t);
        } else {
            a2(game, cx, u, t);
        }
        return;
    }
    // 5. Both requests (edge case 1).
    idle(game, cx, u, cx.aip(p, 5));
    walk_to(game, cx, u, t, 7);
}

// ---- PantherWoman ---------------------------------------------------

/// PantherWoman (18).
pub fn panther_woman<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    // 1.
    if p.combat {
        if cx.chance(u, cx.aip(p, 2)) {
            a1(game, cx, u, t);
        } else {
            idle(game, cx, u, cx.aip(p, 4));
        }
        return;
    }
    // 2.
    if cx.chance(u, cx.aip(p, 1)) {
        set_velocity(cx, u, 0, 75, 0);
        walk_to_method13(game, cx, u, t, 7);
        return;
    }
    // 3. aip3 squared here.
    let aip3 = cx.aip(p, 3);
    if let Some((m, d)) = pack_scan(game, cx, u) {
        if d > aip3.wrapping_mul(aip3) {
            set_velocity(cx, u, 0, 75, 0);
            walk_to(game, cx, u, Some(m), 7);
            return;
        }
    }
    // 4.
    if pct(cx, u) < 25 {
        circle(game, cx, u, t, 3, false);
    } else {
        idle(game, cx, u, cx.aip(p, 4));
    }
}

// ---- MaggotLarva ----------------------------------------------------

/// MaggotLarva (38).
pub fn maggot_larva<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    // 1.
    if !p.combat {
        set_param(cx, u, 0, 0);
        if cx.chance(u, cx.aip(p, 3)) {
            walk_to(game, cx, u, t, 1);
        } else {
            idle(game, cx, u, cx.aip(p, 4));
        }
        return;
    }
    // 2.
    if param(cx, u, 0) == 0 && cx.chance(u, cx.aip(p, 1)) {
        set_param(cx, u, 0, 1);
        a1(game, cx, u, t);
        return;
    }
    // 3.
    set_param(cx, u, 0, 0);
    idle(game, cx, u, cx.aip(p, 2));
}

// ---- SandLeaper -----------------------------------------------------

/// SandLeaper (17).
pub fn sand_leaper<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let (s1, m1) = cx.skill(p, 1);
    // 1.
    if s1 >= 0
        && p.distance < 5
        && cx.chance(u, cx.aip(p, 1))
        && cx.world.skill_check(game, u, s1, t, 0, 0)
    {
        use_skill(game, cx, u, m1, s1, target_of(t));
        return;
    }
    // 2.
    if p.combat {
        if cx.chance(u, cx.aip(p, 2)) {
            a2(game, cx, u, t);
        } else {
            idle(game, cx, u, 10);
        }
        return;
    }
    // 3.
    if p.distance > 10 {
        set_velocity(cx, u, 0, 75, 0);
        wander_near_opt(game, cx, u, t, 5);
        return;
    }
    // 4–6.
    if cx.chance(u, cx.aip(p, 3)) {
        walk_to(game, cx, u, t, 7);
    } else if cx.chance(u, cx.aip(p, 4)) {
        circle(game, cx, u, t, 4, false);
    } else {
        idle(game, cx, u, 10);
    }
}

// ---- MaggotEgg ------------------------------------------------------

/// MaggotEgg (40).
pub fn maggot_egg<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let (s1, _) = cx.skill(p, 1);
    // 1.
    if s1 >= 0 && param(cx, u, 0) != 1 && cx.chance(u, cx.aip(p, 2)) {
        skill_k(game, cx, u, p, 1, p.target);
        wait(game, cx, u, cx.aip(p, 1));
        set_param(cx, u, 0, 1);
        return;
    }
    // 2. K = the path target, 0 when that is the unit itself.
    if param(cx, u, 0) == 1 {
        let k = cx.world.path_target(u).filter(|&k| k != u);
        cx.world.kill(game, u, k);
        return;
    }
    // 3.
    idle(game, cx, u, cx.aip(p, 1));
}

// ---- PinHead --------------------------------------------------------

/// PinHead (39).
pub fn pin_head<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    // 1.
    if !p.combat {
        set_param(cx, u, 0, 0);
        if cx.chance(u, cx.aip(p, 3)) {
            walk_to(game, cx, u, t, 7);
        } else {
            idle(game, cx, u, cx.aip(p, 4));
        }
        return;
    }
    // 2.
    if param(cx, u, 0) != 0 && !cx.chance(u, cx.aip(p, 1)) {
        idle(game, cx, u, cx.aip(p, 2));
        return;
    }
    // 3–5.
    set_param(cx, u, 0, 1);
    for (k, aip) in [(1, 5), (2, 6)] {
        if cx.skill(p, k).0 >= 0 && cx.chance(u, cx.aip(p, aip)) {
            skill_k(game, cx, u, p, k, t);
            return;
        }
    }
    a1(game, cx, u, t);
}

// ---- ClawViper -----------------------------------------------------

/// The glow state of aip6: 0 none, 2 → 91, other → 90.
pub(super) fn glow(g: i32) -> Option<u16> {
    match g {
        0 => None,
        2 => Some(91),
        _ => Some(90),
    }
}

/// ClawViper (16).
pub fn claw_viper<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let g = glow(cx.aip(p, 6));
    // 1. Param 0 is never cleared (edge case 4).
    if let Some(st) = g.filter(|_| param(cx, u, 0) != 0) {
        cx.world.set_state(game, u, st, false);
    }
    // 3.
    if p.combat {
        if !cx.chance(u, cx.aip(p, 3)) {
            idle(game, cx, u, cx.aip(p, 5));
        } else if cx.chance(u, cx.aip(p, 4)) {
            a1(game, cx, u, t);
        } else {
            a2(game, cx, u, t);
        }
        return;
    }
    // 2.1.
    let (s1, m1) = cx.skill(p, 1);
    if s1 >= 0 && p.distance < cx.aip(p, 2) && cx.chance(u, cx.aip(p, 1)) {
        let (tx, ty) = t.map_or((0, 0), |t| cx.world.position(t));
        if cx.world.skill_check(game, u, s1, t, tx, ty) {
            use_skill(game, cx, u, m1, s1, target_of(t));
            if let Some(st) = g {
                cx.world.set_state(game, u, st, true);
            }
            set_param(cx, u, 0, 1);
            return;
        }
    }
    // 2.2.
    if pct(cx, u) < 50 {
        walk_to(game, cx, u, t, 7);
    } else {
        idle(game, cx, u, cx.aip(p, 5));
    }
}

// ---- Vulture -------------------------------------------------------

/// Touch down: mode 9 toward the unit itself through the
/// walk-in-radius core (2, 3); wait 12; p := −1.
fn touch_down<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    cx.world.move_in_radius(game, u, u, 9, 2, 3);
    wait(game, cx, u, 12);
    set_param(cx, u, 0, -1);
}

/// The carrion scan (callback).
fn carrion<W: AiHost + ?Sized>(
    game: &Game,
    cx: &Ctx<'_, W>,
    u: UnitId,
    pct: i32,
) -> Option<UnitId> {
    let mut best = None;
    let mut max = 121;
    for v in scan_units(game, u) {
        let ty = type_of(game, v);
        if v == u
            || !matches!(ty, Some(UnitType::Monster | UnitType::Player))
            || (ty == Some(UnitType::Monster) && dying(cx, v))
        {
            continue;
        }
        let d = sq_dist(cx, u, v);
        if d <= max && life(cx, v) <= (cx.world.max_life(v) / 100).wrapping_mul(pct) {
            best = Some(v);
            max = d;
        }
    }
    best
}

/// Vulture (23).
pub fn vulture<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    let mut pv = param(cx, u, 0);
    // 1.
    // T = 0 is unreachable (target mode 1, open
    // question 6): asserted, not handled.
    let t = p
        .target
        .expect("Vulture think without a target (ai-bodies-2.md OQ6)");
    let far = sq_dist(cx, u, t) > 144;
    // 2.
    if room_of(game, t) != room_of(game, u) && far {
        if pv < 1 {
            walk_in_radius(game, cx, u, t, 9, 0);
            return;
        }
        if land(game, cx, u) {
            touch_down(game, cx, u);
            return;
        }
    }
    // 3. The draw only when the first three hold.
    if pv == 0 && minion_owner(game, cx, u).is_none() && far && roll(cx, u, 100) < 60 {
        take_off(game, cx, u);
        let v = cx.world.seed(u).mask(8) as i32 + 24;
        set_param(cx, u, 0, v);
        cx.world.move_in_radius(game, u, t, 8, 8, 8);
        wait(game, cx, u, 12);
        return;
    }
    // 4.
    let found = carrion(game, cx, u, cx.aip(p, 3));
    // 5.
    let mut land_now = pv == 1;
    if pv >= 2 {
        if found.is_some() || (p.distance < 6 && roll(cx, u, 100) < 15) {
            pv = 1;
            set_param(cx, u, 0, 1);
            land_now = true;
        } else {
            take_off(game, cx, u);
            let (x1, y1) = (param(cx, u, 1), param(cx, u, 2));
            if x1 != 0 && y1 != 0 && path_distance(cx, u, x1, y1) > 1 {
                let size = cx.world.size(u);
                let pt = match cx.world.free_point(game, room_of(game, t), x1, y1, size) {
                    Some(pt) => Some(pt),
                    None => cx.world.free_point(game, room_of(game, u), x1, y1, size),
                };
                if let Some((x, y)) = pt {
                    set_param(cx, u, 1, x);
                    set_param(cx, u, 2, y);
                }
                set_param(cx, u, 0, pv - 1);
                set_velocity(cx, u, 7, 0, 0);
                let (x, y) = (param(cx, u, 1), param(cx, u, 2));
                mode_point(game, cx, u, 8, x, y);
                wait(game, cx, u, 12);
                return;
            }
            let r = pv + 8;
            let (tx, ty) = cx.world.position(t);
            let mut x = tx - r + roll(cx, u, 2 * r);
            let mut y = ty - r + roll(cx, u, 2 * r);
            let m = (2 * r).clamp(12, 36);
            let o = cx.world.position(u);
            while path_distance(cx, u, x, y) < m {
                x += (x - o.0).signum();
                y += (y - o.1).signum();
                if (x, y) == o {
                    x += 1;
                    y += 1;
                }
            }
            set_param(cx, u, 1, x);
            set_param(cx, u, 2, y);
            set_velocity(cx, u, 1, 0, 0);
            mode_point(game, cx, u, 8, x, y);
            wait(game, cx, u, 12);
            set_param(cx, u, 0, pv - 1);
            return;
        }
    }
    // 6. A failed landing goes on with the old p (≥ 1).
    if land_now {
        if land(game, cx, u) {
            touch_down(game, cx, u);
            return;
        }
        set_param(cx, u, 0, 8);
        wait(game, cx, u, 12);
    }
    // 7.
    if p.combat {
        if cx.chance(u, cx.aip(p, 1)) {
            a1(game, cx, u, Some(t));
        } else {
            wait(game, cx, u, cx.aip(p, 2));
        }
        return;
    }
    // 8.
    if pv != -1 && roll(cx, u, 100) >= cx.aip(p, 5) {
        wait(game, cx, u, cx.aip(p, 2));
        return;
    }
    if roll(cx, u, 100) < cx.aip(p, 4) {
        circle(game, cx, u, Some(t), 6, false);
    } else {
        walk_in_radius(game, cx, u, t, 9, 0);
    }
    set_param(cx, u, 0, 0);
    wait(game, cx, u, 12);
}

// ---- BatDemon ------------------------------------------------------

/// Stat 74 `hpregen`.
const STAT_HPREGEN: u16 = 74;

/// BatDemon (29).
pub fn bat_demon<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let d = p.distance;
    let s = param(cx, u, 0);
    let c = param(cx, u, 1);
    let l = cx.world.life_percent(u);
    let flee =
        |game: &mut Game, cx: &mut Ctx<'_, W>, n: i32| escape(game, cx, u, t, n, false, false);
    match s {
        // 1.
        0 | 4 => {
            mode_at(game, cx, u, 10, None);
            wait(game, cx, u, 8);
            set_param(cx, u, 0, 1);
            set_param(cx, u, 1, 0);
            let r = cx.world.stat(u, STAT_HPREGEN);
            if r != 0 {
                let h = cx.aip(p, 5).wrapping_mul(r) / 8;
                set_param(cx, u, 2, h);
                cx.world.set_stat(u, STAT_HPREGEN, r.wrapping_add(h));
            } else {
                set_param(cx, u, 2, 0);
            }
        }
        // 2.
        1 => {
            if c > 1 && (p.combat || d < 7 || cx.ai_state_set(u) || (d < 14 && l > 50)) {
                let h = param(cx, u, 2);
                if h != 0 {
                    let v = cx.world.stat(u, STAT_HPREGEN);
                    cx.world.set_stat(u, STAT_HPREGEN, v.wrapping_sub(h));
                }
                mode_at(game, cx, u, 9, None);
                wait(game, cx, u, 6);
                set_param(cx, u, 0, 3);
            } else {
                mode_at(game, cx, u, 11, None);
                wait(game, cx, u, if c == 0 { 10 } else { 15 });
                set_param(cx, u, 1, c + 1);
            }
        }
        // 3.
        2 => {
            if l < cx.aip(p, 1) && flee(game, cx, 15) {
                set_param(cx, u, 0, 4);
            } else if roll(cx, u, 100) < 33 {
                walk_to(game, cx, u, t, 0);
                set_param(cx, u, 0, 3);
            } else if roll(cx, u, 100) < 15 {
                wander(game, cx, u, 6);
            } else {
                idle(game, cx, u, 10);
            }
        }
        // 4.
        3 => {
            if l < cx.aip(p, 1) && flee(game, cx, 15) {
                set_param(cx, u, 0, 4);
            } else if !p.combat {
                walk_to(game, cx, u, t, 0);
            } else if c > 0 {
                a2(game, cx, u, t);
                set_param(cx, u, 1, 0);
            } else if cx.ai_state_set(u) && roll(cx, u, 100) < cx.aip(p, 2) && flee(game, cx, 12) {
                set_param(cx, u, 0, 2);
            } else if roll(cx, u, 100) < cx.aip(p, 3) {
                if roll(cx, u, 100) < cx.aip(p, 4) {
                    a2(game, cx, u, t);
                } else {
                    a1(game, cx, u, t);
                }
            } else {
                idle(game, cx, u, 10);
            }
        }
        // 5.
        _ => {
            if d < 15 {
                walk_to(game, cx, u, t, 0);
                set_param(cx, u, 0, 3);
            } else {
                set_param(cx, u, 0, 4);
                idle(game, cx, u, 15);
            }
        }
    }
}

/// BatDemon's alternate: no draws.
pub fn bat_demon_alt<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    if cx.world.anim_mode(u) == 11 {
        set_param(cx, u, 0, 1);
        wait(game, cx, u, 1);
    } else if param(cx, u, 0) == 4 {
        set_param(cx, u, 0, 1);
        mode_at(game, cx, u, 10, None);
    } else {
        reinstall(game, cx, u);
        idle(game, cx, u, 1);
    }
}

// ---- SandMaggotQueen -----------------------------------------------

/// Unit flag 0x04000000: suppress experience awards.
const FLAG_NO_XP: u32 = 0x0400_0000;

/// SandMaggotQueen (66): no draws of its own.
pub fn sand_maggot_queen<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let aip2 = cx.aip(p, 2);
    // 1.
    if param(cx, u, 2) != 0 {
        idle(game, cx, u, aip2.wrapping_mul(25));
        set_param(cx, u, 2, 0);
        return;
    }
    // 2. At the cap nothing is scheduled (edge case 2).
    if param(cx, u, 1) == 0 {
        if param(cx, u, 0) < cx.aip(p, 1) {
            let (x, y) = cx.world.position(u);
            mode_point(game, cx, u, 8, x, y);
            wait(game, cx, u, aip2);
            set_param(cx, u, 1, 1);
        }
        return;
    }
    // 3. The spawn info: chain(68 sandmaggot1) at (x + 8, y), mode 8.
    let info = spawn_info(game, cx, u, 0);
    let (class, x, y) = (info.class, info.x, info.y);
    if let Some(room) = cx.world.room_at(game, u, x, y) {
        if let Some(m) = cx.spawn_monster(game, room, x, y, class, info.mode, 2, 0x42) {
            cx.world.set_unit_flag(m, FLAG_NO_XP);
            set_param(cx, u, 0, param(cx, u, 0).wrapping_add(1));
        }
    }
    wait(game, cx, u, aip2);
    set_param(cx, u, 1, 0);
    set_param(cx, u, 2, 1);
}

// ---- Duriel --------------------------------------------------------

/// Duriel (44).
pub fn duriel<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    let t = p.target;
    // 1. The aura.
    let (s4, _) = cx.skill(p, 4);
    if s4 >= 0 && cx.world.hand_skill(u, true).is_none() {
        let level = cx.aip(p, 1);
        cx.world.add_right_skill(game, u, s4, level);
    }
    // 2. aip5 = 0 draws without firing (edge case 5).
    if !p.combat {
        if cx.skill(p, 1).0 >= 0 && cx.chance(u, cx.aip(p, 5)) {
            skill_k(game, cx, u, p, 1, t);
        } else {
            set_velocity(cx, u, 13, 0, 0);
            walk_to(game, cx, u, t, 7);
        }
        return;
    }
    // 3.
    for (k, aip) in [(3, 2), (2, 3)] {
        if cx.skill(p, k).0 >= 0 && cx.chance(u, cx.aip(p, aip)) {
            skill_k(game, cx, u, p, k, t);
            return;
        }
    }
    if cx.chance(u, cx.aip(p, 4)) {
        a2(game, cx, u, t);
    } else {
        a1(game, cx, u, t);
    }
}

// ---- Summoner ------------------------------------------------------

/// Stats 39 `fireresist`, 43 `coldresist`.
const STAT_FIRERESIST: u16 = 39;
const STAT_COLDRESIST: u16 = 43;

/// Skill k in `Skkmode` at `x`.
///
/// The request holds the unit and T's position (X, Y) at once; the mode
/// set aims at the unit whenever it is ≠ 0 and uses (X, Y) only for a
/// unit of 0 (the mode request record; not settled for every caller).
fn summoner_cast<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
    k: usize,
    x: Option<UnitId>,
) {
    let at = match x {
        Some(x) => ModeTarget::Unit(x),
        None => {
            let (tx, ty) = p.target.map_or((0, 0), |t| cx.world.position(t));
            ModeTarget::Point(tx, ty)
        }
    };
    let (s, m) = cx.skill(p, k);
    use_skill(game, cx, u, m, s, at);
}

/// Summoner (53).
pub fn summoner<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let d = p.distance;
    let frame = game.frame;
    // 1.
    if param(cx, u, 0) == 0 {
        cx.world.quest_call(game, u, QuestCall::SummonerActivated);
        set_param(cx, u, 0, 1);
    }
    // 2. The result is ignored (edge case 6).
    if d < 5 && cx.chance(u, cx.aip(p, 6)) {
        escape(game, cx, u, t, 6, false, false);
    }
    // 3.
    let mut k =
        t.is_some_and(|t| cx.world.stat(t, STAT_FIRERESIST) >= cx.world.stat(t, STAT_COLDRESIST));
    // 4.
    if !cx.chance(u, cx.aip(p, 1)) {
        wander(game, cx, u, 4);
        return;
    }
    let has = |cx: &Ctx<'_, W>, n: usize| cx.skill(p, n).0 >= 0;
    // 5.
    if has(cx, 5) && cx.chance(u, cx.aip(p, 2)) {
        summoner_cast(game, cx, u, p, 5, t);
        return;
    }
    // 6.
    let (s, _, _) = cx.world.secondary_target(game, u, cx.store);
    if pct(cx, u) > cx.aip(p, 3) {
        k = !k;
    }
    let nova_ok = |cx: &Ctx<'_, W>| has(cx, 2) && frame > param(cx, u, 1) && d < cx.aip(p, 7);
    let nova = |game: &mut Game, cx: &mut Ctx<'_, W>| {
        set_param(cx, u, 1, game.after(cx.aip(p, 4)));
        summoner_cast(game, cx, u, p, 2, t);
    };
    let aip8 = cx.aip(p, 8);
    // 7.
    if k {
        if nova_ok(cx) {
            nova(game, cx);
            return;
        }
        if has(cx, 1) && s.is_some() && d < aip8 {
            summoner_cast(game, cx, u, p, 1, s);
            return;
        }
    }
    // 8.
    if has(cx, 4) && frame > param(cx, u, 2) {
        set_param(cx, u, 2, game.after(cx.aip(p, 5)));
        summoner_cast(game, cx, u, p, 4, t);
        return;
    }
    // 9.
    if has(cx, 3) && s.is_some() && d < aip8 {
        summoner_cast(game, cx, u, p, 3, s);
        return;
    }
    // 10.
    if !k && nova_ok(cx) {
        nova(game, cx);
        return;
    }
    // 11, 12.
    if has(cx, 5) {
        summoner_cast(game, cx, u, p, 5, t);
    } else {
        wander(game, cx, u, 4);
    }
}

// ---- special-state thinks -------------------------------------------

/// The class lacks the monstats `interact` bit (a missing row lacks it).
fn no_interact<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, u: UnitId) -> bool {
    !cx.monstats(cx.world.class(u)).is_some_and(|r| r.interact)
}

/// C, a class without `interact` and with mode A1.
fn may_strike<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, u: UnitId, p: &TickParam) -> bool {
    p.combat && no_interact(cx, u) && cx.world.class_has_mode(cx.world.class(u), mode::ATTACK1)
}

/// Special state 10 / 17 (dim vision).
pub fn dim_vision<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    if may_strike(cx, u, p) {
        a1(game, cx, u, p.target);
    } else if pct(cx, u) < 20 {
        wander(game, cx, u, 3);
    } else {
        idle(game, cx, u, 10);
    }
}

/// Special state 11 init (terror): the target override
/// from the path target's type.
pub fn terror_init<W: AiHost + ?Sized>(game: &Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    let Some(pt) = cx.world.path_target(u) else {
        return;
    };
    let kind = match type_of(game, pt) {
        Some(UnitType::Player) => 1,
        Some(UnitType::Monster) => 2,
        Some(UnitType::Missile) => 4,
        _ => return,
    };
    let guid = guid_of(game, Some(pt)) as u32;
    cx.world.set_target_override(u, kind, guid);
}

/// State 56 `terror`.
const STATE_TERROR: u16 = 56;

/// Special state 11 think (terror).
pub fn terror<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    let t = p.target;
    let v = run_bonus(cx, p);
    let runs = cx.world.class_has_mode(cx.world.class(u), mode::RUN);
    // 1.
    if !cx.world.has_state(u, STATE_TERROR) {
        leave_special(game, cx, u);
        return;
    }
    // 2.
    let r = match param(cx, u, 0) {
        0 => 30,
        r => r,
    };
    if p.distance > r {
        idle(game, cx, u, 10);
        return;
    }
    // 3.
    if param(cx, u, 2) == 0 {
        set_param(cx, u, 2, 1);
        cx.world.ai_reset(u);
        delete_thinks(game, u);
        set_velocity(cx, u, 2, v, 0);
        escape(game, cx, u, t, 30, false, runs);
        return;
    }
    // 4.
    if may_strike(cx, u, p) {
        a1(game, cx, u, t);
        return;
    }
    // 5.
    set_velocity(cx, u, 2, v, 0);
    if !escape(game, cx, u, t, 30, true, runs) {
        wander(game, cx, u, 6);
    }
}

/// Special state 12 think (taunt, target mode 0).
pub fn taunted<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    let pt = cx.world.path_target(u);
    let outside =
        pt.is_some_and(|pt| room_of(game, pt).is_some_and(|r| !cx.world.in_town(game, r)));
    let Some(pt) = pt.filter(|_| outside) else {
        // 2.
        leave_special(game, cx, u);
        return;
    };
    // 1.1.
    if param(cx, u, 0) == 0 {
        set_param(cx, u, 0, 1);
        walk_to_method13(game, cx, u, Some(pt), 7);
        return;
    }
    // 1.2.
    if cx.world.in_melee_range(game, u, pt) {
        a1(game, cx, u, Some(pt));
        return;
    }
    // 1.3.
    let mut q = *p;
    if find_mode1(game, cx, u, &mut q) && q.combat {
        set_param(cx, u, 0, 0);
        a1(game, cx, u, q.target);
        return;
    }
    // 1.4.
    set_param(cx, u, 0, 1);
    cx.world.set_path_target(u, pt);
    walk_to_method13(game, cx, u, Some(pt), 7);
}

// ---- special state 16 (imp possession) ----------------------------------

/// The imp record (monstats 492) and the one whose AI parameters the
/// think reads (493, `imp2`; the base game indexes the table at fixed offsets).
const IMP1: i32 = 492;
const IMP2: i32 = 493;
/// State 143 (`attached`): the imp rides its source.
const STATE_ATTACHED: u16 = 143;

/// Whether the unit has a skill-list entry of `skill` with owner −1
/// (skill-list lookup with owner −1): an added entry, else the ones monster
/// creation gives for a `Skill<i>` ≥ 0 with `Sk<i>lvl` > 0.
fn has_skill_entry<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, u: UnitId, skill: i32) -> bool {
    if skill < 0 {
        return false;
    }
    if cx.world.skill_level(u, skill, false).is_some() {
        return true;
    }
    let Some(r) = cx.monstats(cx.world.class(u)) else {
        return false;
    };
    let lvls = [
        r.sk1lvl, r.sk2lvl, r.sk3lvl, r.sk4lvl, r.sk5lvl, r.sk6lvl, r.sk7lvl, r.sk8lvl,
    ];
    (1..=8).any(|n| cx.class_skill(cx.world.class(u), n).0 == skill && lvls[n - 1] > 0)
}

/// Special state 16 init: one raw step of the unit seed; its
/// low bit picks the first of two skill slots (3, then 5: the table
/// Holds slot 2 and 4, second dword 0); the first slot whose
/// skill the unit has is stored in the control's +0x18 (slot) and +0x1C
/// (second dword); neither → 0 and 0.
pub fn possess_init<W: AiHost + ?Sized>(cx: &mut Ctx<'_, W>, u: UnitId) {
    const SLOTS: [(i32, i32); 2] = [(2, 0), (4, 0)];
    let mut idx = (cx.world.seed(u).step() & 1) as usize;
    let class = cx.world.class(u);
    let mut found = (0, 0);
    for _ in 0..2 {
        idx = (idx + 1) & 1;
        let (slot, second) = SLOTS[idx];
        if has_skill_entry(cx, u, cx.class_skill(class, slot as usize + 1).0) {
            found = (slot, second);
            break;
        }
    }
    set_param(cx, u, 1, found.0);
    set_param(cx, u, 2, found.1);
}

/// Special state 16 think (the possessed imp, target mode 1).
pub fn possessed_imp<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let src = cx.world.source_unit(u);
    // 1. Not riding any more: back to state 0, idle 1.
    let Some(src) = src.filter(|_| cx.world.has_state(u, STATE_ATTACHED)) else {
        install(game, cx, u, 0);
        idle(game, cx, u, 1);
        return;
    };
    let (slot, second) = (param(cx, u, 1), param(cx, u, 2));
    let out =
        p.distance > 0x18 || cx.world.is_dead(src) || cx.world.alignment(src) != 0 || slot == 0;
    if out {
        // 2. Out of reach: a teleport in range of imp1's second AIP, then
        // the first parameter is reset.
        let r = cx.class_aip(IMP1, 2);
        let (s, m) = cx.class_skill(IMP1, 1);
        teleport_in_range(game, cx, u, r, s, m);
        set_param(cx, u, 0, -1);
        return;
    }
    // 3. Within imp2's second AIP: act with chance imp2's third AIP
    // (`+0x5C` / `+0x62` of the row; aip1 is `+0x56`).
    if p.distance < cx.class_aip(IMP2, 2) && roll(cx, u, 100) < cx.class_aip(IMP2, 3) {
        if second != 0 {
            let n = cx.class_aip(IMP2, 4).wrapping_mul(2);
            roll(cx, u, n);
            roll(cx, u, n);
        }
        let (s, m) = cx.class_skill(IMP1, slot as usize + 1);
        if s >= 0 {
            use_skill(game, cx, u, m, s, super::bodies::at(t));
            return;
        }
        idle(game, cx, u, 0x14);
        return;
    }
    // 4.
    if roll(cx, u, 100) < 50 {
        mode_at(game, cx, u, 8, t);
    } else {
        idle(game, cx, u, 0x14);
    }
}

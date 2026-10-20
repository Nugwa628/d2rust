//! The Act IV thinks, the Diablo alternate and the boss pick,
//! target score and choice of Diablo (the pick and score are shared with the
//! Ancients and Baal). Brackets in the comments are
//! the Normal difficulty values.

use crate::game::Game;
use crate::units::{UnitId, UnitType};

use super::bodies::{a1, param, pct, roll, run_bonus, set_param};
use super::bodies2::reinstall;
use super::bodies3::{skill_range, STATE_INFERNO};
use super::common::*;
use super::tactics::*;
use super::{delete_thinks, idle, mode, AiCommand, AiHost, Ctx, ModeTarget, QuestCall, TickParam};

// ---- VileMother -----------------------------------------------------

/// vilechild1, vilemother1.
const VILECHILD1: i32 = 301;
const VILEMOTHER1: i32 = 298;

/// The 64 → 8 direction table: 0–3 → 0, then 8 per step,
/// 60–63 → 0.
pub(super) fn dir8(d: i32) -> usize {
    ((d.wrapping_add(4) >> 3) & 7) as usize
}

/// Birth offsets: e by k, dx, dy.
const BIRTH_E: [usize; 8] = [6, 4, 2, 2, 2, 0, 6, 6];
pub(super) const DIR_DX: [i32; 8] = [0, -1, -1, -1, 0, 1, 1, 1];
pub(super) const DIR_DY: [i32; 8] = [-1, -1, 0, 1, 1, 1, 0, -1];

/// The child class Y: row 301's `BaseId` followed n steps along
/// `NextInClass`, n = the chain
/// byte of the unit's class (class −1 → 0).
fn child_class<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, u: UnitId) -> i32 {
    let class = cx.world.class(u);
    let n = if class < 0 {
        0
    } else {
        cx.world.chain_index(class)
    };
    let mut y = base_id(cx, fixed_class(cx, VILECHILD1));
    for _ in 0..n.max(0) {
        y = cx
            .monstats(y)
            .map_or(-1, |r| i32::from(r.nextinclass as i16));
    }
    y
}

/// Birth → true / false. No draws.
fn birth<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) -> bool {
    // T = 0 is unreachable (VileMother is target mode 1):
    // asserted, not handled.
    let t = p
        .target
        .expect("VileMother birth without a target (ai.md §2.3)");
    let d = cx.world.direction64(u, t);
    let d8 = dir8(d);
    let (s1, m1) = cx.skill(p, 1);
    let (ox, oy) = cx.world.position(u);
    let room = room_of(game, u);
    let class = fixed_class(cx, VILEMOTHER1);
    for i in 0..8 {
        let e = BIRTH_E[(d8 + i) & 7];
        let (x, y) = (ox + 3 * DIR_DX[e], oy + 3 * DIR_DY[e]);
        if s1 >= 0 && cx.world.footprint_ok(game, class, room, x, y) {
            use_skill(game, cx, u, m1, s1, ModeTarget::Point(x, y));
            return true;
        }
    }
    false
}

/// VileMother (68).
pub fn vile_mother<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let b = param(cx, u, 0);
    // 1.
    let y = child_class(cx, u);
    // 2. Only a birth ends the think; every failed test falls through to 3.
    if b < cx.aip(p, 1) && cx.world.alignment(u) == 0 && cx.chance(u, cx.aip(p, 3)) {
        let count = scan_units(game, u)
            .into_iter()
            .filter(|&v| {
                is_monster(game, v)
                    && cx.world.class(v) == y
                    && !cx.world.is_dead(v)
                    && unit_distance(cx, u, v) <= 25
            })
            .count() as i32;
        if count < cx.aip(p, 2) && birth(game, cx, u, p) {
            set_param(cx, u, 0, b + 1);
            return;
        }
    }
    // 3.
    if p.combat {
        if cx.chance(u, cx.aip(p, 4)) {
            a1(game, cx, u, t);
        } else {
            idle(game, cx, u, cx.aip(p, 7));
        }
        return;
    }
    // 4.
    if b < cx.aip(p, 1) && pct(cx, u) >= cx.aip(p, 5) {
        if pct(cx, u) < cx.aip(p, 6) {
            circle(game, cx, u, t, 4, false);
        } else {
            idle(game, cx, u, 15);
        }
        return;
    }
    // 5.
    walk_to(game, cx, u, t, 7);
}

// ---- VileDog --------------------------------------------------------

/// VileDog (69).
pub fn vile_dog<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    if param(cx, u, 0) == 0 {
        set_param(cx, u, 0, 1);
        idle(game, cx, u, 5);
    } else if p.combat {
        if cx.chance(u, cx.aip(p, 1)) {
            a1(game, cx, u, t);
        } else {
            idle(game, cx, u, cx.aip(p, 2));
        }
    } else if cx.chance(u, cx.aip(p, 3)) {
        walk_to(game, cx, u, t, 7);
    } else {
        idle(game, cx, u, 10);
    }
}

// ---- FingerMage -----------------------------------------------------

/// State 84 `fingermagecurse`.
const STATE_FINGERMAGECURSE: u16 = 84;

/// FingerMage (70).
pub fn finger_mage<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let d = p.distance;
    let l = cx.world.life_percent(u);
    let has1 = cx.skill(p, 1).0 >= 0;
    let aip5 = cx.aip(p, 5);
    // 1.
    if cx.ai_state_set(u) {
        set_param(cx, u, 0, 0);
        if p.combat {
            a1(game, cx, u, t);
            return;
        }
        if has1 && d < aip5 {
            skill_k(game, cx, u, p, 1, t);
            return;
        }
    }
    // 2.
    if param(cx, u, 0) == 1 {
        let n = param(cx, u, 1) + 1;
        set_param(cx, u, 1, n);
        if l > cx.aip(p, 3) || pct(cx, u) < 25 || n > cx.aip(p, 6) {
            set_param(cx, u, 0, 0);
            idle(game, cx, u, 15);
        } else if d < 14 {
            let v = run_bonus(cx, p);
            set_velocity(cx, u, 0, v, 0);
            escape(game, cx, u, t, 14, false, false);
        } else {
            idle(game, cx, u, 15);
        }
        return;
    }
    // 3.
    if minion_owner(game, cx, u).is_none() && l < cx.aip(p, 4) {
        set_param(cx, u, 0, 1);
        set_param(cx, u, 1, 0);
        escape(game, cx, u, t, 9, false, false);
        return;
    }
    // 4.
    if p.combat {
        if cx.chance(u, cx.aip(p, 1)) {
            a1(game, cx, u, t);
        } else if has1 && pct(cx, u) < cx.aip(p, 2) {
            skill_k(game, cx, u, p, 1, t);
        } else {
            circle(game, cx, u, t, 5, false);
        }
        return;
    }
    // 5.
    if d < aip5 {
        if has1 && pct(cx, u) < cx.aip(p, 2) {
            skill_k(game, cx, u, p, 1, t);
        } else if t.is_some_and(|t| cx.world.has_state(t, STATE_FINGERMAGECURSE)) {
            walk_to(game, cx, u, t, 0);
        } else {
            idle(game, cx, u, cx.aip(p, 8));
        }
        return;
    }
    // 6.
    if d < cx.aip(p, 7) {
        walk_to(game, cx, u, t, 7);
    } else {
        idle(game, cx, u, 15);
    }
}

// ---- Regurgitator ---------------------------------------------------

/// "Reset": s, g, k := 0.
fn regurg_reset<W: AiHost + ?Sized>(cx: &mut Ctx<'_, W>, u: UnitId) {
    for i in 0..3 {
        set_param(cx, u, i, 0);
    }
}

/// "Go to corpse c": walk to it with 1 step, s := 2, g := its GUID, k := 0.
fn go_to_corpse<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, c: UnitId) {
    move_steps(game, cx, u, Some(c), false, 1);
    set_param(cx, u, 0, 2);
    set_param(cx, u, 1, guid_of(game, Some(c)));
    set_param(cx, u, 2, 0);
}

/// Regurgitator (71).
pub fn regurgitator<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let s = param(cx, u, 0);
    let corpse = game
        .lists
        .find_unit(UnitType::Monster, param(cx, u, 1) as u32)
        .filter(|&k| cx.world.anim_mode(k) == mode::DEAD);
    match s {
        // 1.
        5 => {
            escape(game, cx, u, t, 16, true, false);
            set_param(cx, u, 0, 0);
            return;
        }
        // 2. Walk then idle on the same think (edge case 2).
        // "Then s := 3; idle 8" follows both the walk and the near case.
        2 => {
            let Some(k) = corpse else {
                regurg_reset(cx, u);
                wander(game, cx, u, 8);
                return;
            };
            if sq_dist(cx, u, k) > 4 {
                let n = param(cx, u, 2);
                if n >= 6 {
                    regurg_reset(cx, u);
                    wander(game, cx, u, 8);
                    return;
                }
                move_steps(game, cx, u, Some(k), false, 1);
                set_param(cx, u, 2, n + 1);
            }
            set_param(cx, u, 0, 3);
            idle(game, cx, u, 8);
            return;
        }
        // 3.
        3 => {
            match corpse.filter(|_| cx.skill(p, 1).0 >= 0) {
                Some(k) => {
                    skill_k(game, cx, u, p, 1, Some(k));
                    set_param(cx, u, 0, 4);
                }
                None => {
                    regurg_reset(cx, u);
                    wander(game, cx, u, 8);
                }
            }
            return;
        }
        // 4.
        4 => {
            if p.combat {
                escape(game, cx, u, t, 8, true, false);
            } else {
                mode_at(game, cx, u, mode::ATTACK2, t);
                set_param(cx, u, 0, 5);
            }
            return;
        }
        _ => {}
    }
    // 5.
    let aip6 = cx.aip(p, 6);
    let mut max = aip6.wrapping_mul(aip6);
    let mut best = None;
    for v in scan_units(game, u) {
        let f = cx.world.unit_flags(v);
        if v == u
            || !is_monster(game, v)
            || f & 0x200 != 0
            || f & 0x2 == 0
            || !dying(cx, v)
            || cx.world.has_state_group(v, super::bodies2::GROUP_UDEAD)
            || cx.world.alignment(v) != 0
        {
            continue;
        }
        let soft = cx
            .monstats(cx.world.class(v))
            .and_then(|r| cx.tables.monstats2.get(r.monstatsex as usize))
            .is_some_and(|r| r.soft);
        let d = sq_dist(cx, u, v);
        if soft && d <= max {
            best = Some(v);
            max = d;
        }
    }
    let d = max;
    // 6. No corpse and a draw < 20: nothing scheduled (edge case 1).
    if s == 1 {
        match best {
            Some(b) => {
                set_param(cx, u, 1, guid_of(game, Some(b)));
                if d > 2 {
                    move_steps(game, cx, u, Some(b), false, 1);
                    set_param(cx, u, 0, 2);
                } else {
                    idle(game, cx, u, 8);
                    set_param(cx, u, 0, 3);
                }
                set_param(cx, u, 2, 0);
            }
            None => {
                if pct(cx, u) < 20 {
                    regurg_reset(cx, u);
                } else {
                    wander(game, cx, u, 8);
                }
            }
        }
        return;
    }
    // 7.
    if let Some(b) = best {
        if (d < 9 && pct(cx, u) < cx.aip(p, 2)) || pct(cx, u) < cx.aip(p, 5) {
            go_to_corpse(game, cx, u, b);
            return;
        }
    }
    // 8.
    let r = pct(cx, u);
    if p.combat {
        if r < cx.aip(p, 1) {
            a1(game, cx, u, t);
        } else {
            idle(game, cx, u, 15);
        }
        return;
    }
    if r < cx.aip(p, 3) {
        walk_to(game, cx, u, t, 0);
        return;
    }
    if let Some(b) = best {
        if pct(cx, u) < cx.aip(p, 4) {
            go_to_corpse(game, cx, u, b);
            return;
        }
    }
    idle(game, cx, u, 8);
}

// ---- Megademon ------------------------------------------------------

/// Megademon (89).
pub fn megademon<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let frame = game.frame;
    let (s1, _) = cx.skill(p, 1);
    let r = skill_range(cx, u, s1);
    let i = cx.world.has_state(u, STATE_INFERNO);
    let cast = |game: &mut Game, cx: &mut Ctx<'_, W>| {
        set_param(cx, u, 0, game.after(cx.aip(p, 6)));
        skill_k(game, cx, u, p, 1, t);
    };
    if s1 >= 0 && !p.combat && p.distance < r {
        // 1.
        if i {
            cx.world.set_state(game, u, STATE_INFERNO, false);
        } else if frame > param(cx, u, 0) && pct(cx, u) < cx.aip(p, 1) {
            cast(game, cx);
            return;
        }
    } else {
        // 2.
        if i {
            cx.world.set_state(game, u, STATE_INFERNO, false);
        }
        if p.combat {
            if s1 >= 0 && frame > param(cx, u, 0) && pct(cx, u) < cx.aip(p, 2) {
                cast(game, cx);
            } else if pct(cx, u) < cx.aip(p, 3) {
                a1(game, cx, u, t);
            } else if pct(cx, u) < cx.aip(p, 5) {
                circle(game, cx, u, t, 3, false);
            } else {
                idle(game, cx, u, 5);
            }
            return;
        }
    }
    // 3.
    if pct(cx, u) < cx.aip(p, 4) {
        walk_to(game, cx, u, t, 7);
    } else {
        idle(game, cx, u, 10);
    }
}

// ---- Diablo: boss pick, score, choice --------------------------------

/// A home point (the type-10 command's params 1, 2).
pub(super) type Home = Option<(i32, i32)>;

/// The type-10 command; none → copy {10, own x,
/// own y} and look again.
pub(super) fn home_point<W: AiHost + ?Sized>(cx: &mut Ctx<'_, W>, u: UnitId) -> Home {
    let mut k = find_command(cx, u, 10, false);
    if k.is_none() {
        let (x, y) = cx.world.position(u);
        copy_command(
            cx,
            u,
            AiCommand {
                params: [10, x, y, 0, 0],
            },
        );
        k = find_command(cx, u, 10, false);
    }
    k.and_then(|k| command_mut(cx, u, k).map(|c| (c.params[1], c.params[2])))
}

/// Special state 13 (the Countess-only state installed by her
/// superunique finish): keeps the unit at its home point.
pub fn special_state_13<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let Some(t) = p.target else {
        return;
    };
    let Some(room) = game.lists.unit(u).and_then(|e| e.room()) else {
        return;
    };
    let _ = room;
    // 1. The home point; a missing record is copied from the own position
    // (the home-point copy), a record with a zero coordinate is overwritten by it.
    let (ox, oy) = cx.world.position(u);
    let _ = home_point(cx, u);
    let Some(k) = find_command(cx, u, 10, false) else {
        return;
    };
    let (mut hx, mut hy) = command_mut(cx, u, k).map_or((0, 0), |c| (c.params[1], c.params[2]));
    if hx == 0 || hy == 0 {
        if let Some(c) = command_mut(cx, u, k) {
            c.params[1] = ox;
            c.params[2] = oy;
        }
        (hx, hy) = (ox, oy);
    }
    // "Velocity (1, 0, 0)" written to the record directly.
    let head_home = |game: &mut Game, cx: &mut Ctx<'_, W>| -> bool {
        cx.store.entry(u).velocity.method = 1;
        if run_to_point(game, cx, u, hx, hy) {
            return true;
        }
        delete_thinks(game, u);
        false
    };
    // 2. Home in another room than the unit: run home.
    let home_room = cx.world.room_at(game, u, hx, hy);
    if home_room != cx.world.room_at(game, u, ox, oy) && head_home(game, cx) {
        return;
    }
    // 3. The target in another room than home.
    let (tx, ty) = cx.world.position(t);
    if home_room != cx.world.room_at(game, u, tx, ty) {
        if (ox, oy) == (hx, hy) {
            if p.distance < 25 && state13_nodes(game, cx, u) {
                return;
            }
            idle(game, cx, u, 10);
            return;
        }
        if head_home(game, cx) {
            return;
        }
    }
    // 4. Too far from home (path distance, unsigned).
    if path_distance(cx, u, hx, hy) as u32 > 0x28 && head_home(game, cx) {
        return;
    }
    if state13_nodes(game, cx, u) {
        return;
    }
    // 5. Fight or return to idle.
    let r = roll(cx, u, 100);
    if p.combat {
        if r < cx.aip(p, 3) + 10 {
            mode_at(game, cx, u, mode::ATTACK1, Some(t));
        } else {
            idle(game, cx, u, cx.aip(p, 2));
        }
    } else if r < cx.aip(p, 1) {
        set_velocity(cx, u, 0, 100, 0);
        run_to(game, cx, u, Some(t), 0);
    } else {
        idle(game, cx, u, cx.aip(p, 2));
    }
}

/// The map-AI node walk of state 13. With no nodes (or all
/// used) a 700-frame-old stamp resets the index; returns whether a node
/// action started.
// TODO: the node action itself (a skill at the
// node) is not modelled; no scenario gives the Countess a
// node list.
fn state13_nodes<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) -> bool {
    let len = cx
        .store
        .control(u)
        .and_then(|c| c.map_ai.as_ref().map(Vec::len));
    let idx = param(cx, u, 0);
    if len.is_none_or(|n| n as i32 <= idx) {
        if (game.frame.wrapping_sub(param(cx, u, 1))).abs() > game.ticks(700) {
            set_param(cx, u, 0, 0);
        }
        return false;
    }
    false
}

/// Which score a pick uses.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Score {
    /// Diablo's score with the home point.
    Diablo(Home),
    /// B = 0, stat 43 read always.
    Baal,
}

/// The cull of a pick.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Cull {
    /// Same act, no-size distance < 1020.
    Act,
    /// The level ids of both rooms are equal (Ancients).
    Level,
}

fn culled<W: AiHost + ?Sized>(
    game: &Game,
    cx: &Ctx<'_, W>,
    cull: Cull,
    boss: UnitId,
    v: UnitId,
) -> bool {
    match cull {
        Cull::Act => {
            cx.world.act(v) == cx.world.act(boss)
                && distance_no_size(cx.world.position(v), cx.world.position(boss)) < 1020
        }
        Cull::Level => cx.world.level_id(game, v) == cx.world.level_id(game, boss),
    }
}

/// `attackrank` × level of the left and right skills.
fn attack_rank<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, v: UnitId) -> i32 {
    [false, true]
        .into_iter()
        .filter_map(|right| cx.world.hand_skill(v, right))
        .map(|(s, l)| skill_row(cx, s).map_or(0, |r| i32::from(r.attackrank).wrapping_mul(l)))
        .fold(0i32, i32::wrapping_add)
}

/// State 11 `cold`.
const STATE_COLD: u16 = 11;

/// The target score (Diablo, and Baal with [`Score::Baal`]).
pub(super) fn score<W: AiHost + ?Sized>(
    game: &Game,
    cx: &Ctx<'_, W>,
    boss: UnitId,
    v: UnitId,
    sc: Score,
) -> i32 {
    let st = |s: u16| cx.world.stat(v, s);
    // 1.
    let m = cx.world.in_melee_range(game, boss, v);
    let b = match sc {
        Score::Diablo(Some((hx, hy)))
            if half_size_distance(cx, boss, hx, hy) > 85
                && half_size_distance(cx, v, hx, hy) < 85 =>
        {
            100
        }
        _ => 0,
    };
    // 2.
    let r43 = if m || sc == Score::Baal { st(43) } else { 0 };
    let (r36, r39, r41, r37) = (st(36), st(39), st(41), st(37));
    let cold = i32::from(cx.world.has_state(v, STATE_COLD));
    let low = i32::from(cx.world.life_percent(v) <= 19);
    // 3.
    let k = attack_rank(cx, v);
    // 4.
    let g = if m {
        100
    } else if !cx.world.line_blocked(game, boss, v) {
        75
    } else {
        0
    };
    // 5.
    if is_monster(game, v) && cx.monstats(cx.world.class(v)).is_none_or(|r| r.threat < 2) {
        return 0;
    }
    // 6.
    // Only d58 is shifted.
    let dmg = (st(58) >> 8)
        .wrapping_add(st(55))
        .wrapping_add(st(53))
        .wrapping_add(st(51))
        .wrapping_add(st(49))
        .wrapping_add(st(22));
    let res = r43
        .wrapping_add(4i32.wrapping_mul(r36.wrapping_add(2i32.wrapping_mul(r37))))
        .wrapping_add(r41)
        .wrapping_add(r39);
    let inner = (k / 4).wrapping_add(200 * cold).wrapping_add(dmg / 2);
    let s = (300 * low)
        .wrapping_add(res / 15)
        .wrapping_add(5i32.wrapping_mul(b + g))
        .wrapping_add(2i32.wrapping_mul(inner))
        / 22;
    if s == 0 {
        1
    } else {
        s
    }
}

/// The boss pick: (best, max, count). No draws.
pub(super) fn boss_pick<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    boss: UnitId,
    cull: Cull,
    sc: Score,
) -> (Option<UnitId>, i32, i32) {
    let nodes = cx.world.target_nodes(game);
    let mut best: Option<UnitId> = None;
    let mut max = 0;
    let mut count = 0;
    let act = cx.world.act(boss);
    // 1.
    for slot in &nodes.players {
        let Some(&head) = slot.first() else {
            continue;
        };
        // Skip heads that are not players or fail the cull.
        if !is_player(game, head) || !culled(game, cx, cull, boss, head) {
            continue;
        }
        for (i, &v) in slot.iter().enumerate() {
            let s = if i == 0 && matches!(cx.world.anim_mode(v), 0 | 17) {
                0
            } else {
                score(game, cx, boss, v, sc)
            };
            if s > max {
                max = s;
                best = Some(v);
            }
            count += 1;
        }
    }
    // 2.
    for &v in &nodes.good {
        if cx.world.act(v) == act {
            let s = score(game, cx, boss, v, sc);
            if s > max {
                max = s;
                best = Some(v);
            }
            count += 1;
        }
    }
    // 3.
    let mut alt: Option<(UnitId, i32)> = None;
    for &v in &nodes.walls {
        if cx.world.act(v) == act {
            let s = score(game, cx, boss, v, sc);
            if alt.is_none_or(|a| s > a.1) {
                alt = Some((v, s));
            }
        }
    }
    if let Some((a, sa)) = alt {
        if distance_no_size(cx.world.position(a), cx.world.position(boss)) < 5 {
            // With no best the melee test is false and the path is computed
            // toward its stored target point.
            let swap = match best {
                Some(b) => {
                    !cx.world.in_melee_range(game, boss, b)
                        && !cx.world.path_has_points(game, boss, b)
                }
                None => !cx.world.path_has_points_no_target(game, boss),
            };
            if swap {
                best = Some(a);
                max = sa;
            }
        }
    }
    (best, max, count)
}

/// X's left or right skill is Meteor (56) or Blizzard (59), Fire Wall
/// (51) at level > 3 or Immolation Arrow (27) at level > 7 (step 3 of
/// the boss choice).
pub(super) fn special_caster<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, x: UnitId) -> bool {
    [false, true]
        .into_iter()
        .filter_map(|right| cx.world.hand_skill(x, right))
        .any(|(s, l)| matches!(s, 56 | 59) || (s == 51 && l > 3) || (s == 27 && l > 7))
}

/// X's portal object.
pub(super) fn portal_of<W: AiHost + ?Sized>(
    game: &Game,
    cx: &Ctx<'_, W>,
    x: UnitId,
) -> Option<UnitId> {
    if !is_player(game, x) {
        return None;
    }
    let g = cx.world.portal_guid(x)?;
    game.lists.find_unit(UnitType::Object, g)
}

/// The weighted pick of the boss choice (Diablo step 6, Baal step 5): one
/// step when Σ > 0, r := `lo' % Σ` (unsigned; `& (Σ − 1)` for a power of
/// two is the same value); the first k with r < W0 + … + Wk, else
/// `none`.
pub(super) fn weighted<W: AiHost + ?Sized>(
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    w: &[i32],
    none: i32,
) -> i32 {
    let sum = w.iter().fold(0i32, |a, &b| a.wrapping_add(b));
    let r = if sum > 0 {
        (cx.world.seed(u).step() % sum as u32) as i32
    } else {
        0
    };
    let mut acc = 0i32;
    for (k, &wk) in w.iter().enumerate() {
        acc = acc.wrapping_add(wk);
        if r < acc {
            return k as i32;
        }
    }
    none
}

/// Player-count record difficulty field (+0xC): game +0x6D clamped to 2.
pub(super) fn record_difficulty<W: AiHost + ?Sized>(cx: &Ctx<'_, W>) -> i32 {
    i32::from(cx.info.difficulty.min(2))
}

/// Chaos Sanctuary (levels row 108).
const CHAOS_SANCTUARY: i32 = 108;
/// DiabPrison.
const SKILL_DIABPRISON: i32 = 199;

/// The choice.
fn diablo_choice<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    boss: UnitId,
    x: Option<UnitId>,
    big_m: i32,
    n: i32,
    h: Home,
) -> i32 {
    // 1.
    let pending = param(cx, boss, 0);
    if pending != 0 {
        return pending;
    }
    // 2.
    let Some(x) = x else {
        if cx.world.pattern_collides(game, boss, 2, 0x40) {
            return 16;
        }
        return if cx.world.seed(boss).step().is_multiple_of(1000) {
            4
        } else {
            11
        };
    };
    // 3.
    let portal = portal_of(game, cx, x);
    let near = match (portal, h) {
        (Some(pt), Some((hx, hy))) => {
            cx.world.level_id(game, pt) == CHAOS_SANCTUARY && point_distance(cx, pt, hx, hy) < 85
        }
        _ => false,
    };
    let c = special_caster(cx, x);
    let hs = h.map_or(0, |(hx, hy)| half_size_distance(cx, x, hx, hy));
    let far = h.is_some() && hs > 85;
    let further = h.is_some() && hs > 105;
    let m = cx.world.in_melee_range(game, boss, x);
    let clear = !cx.world.line_blocked(game, boss, x);
    let (f39, f41) = (cx.world.stat(x, 39), cx.world.stat(x, 41));
    // 4.
    let mut w = [0i32; 17];
    if m {
        w[..9].copy_from_slice(&[0, 0, 40, 70, 0, 40, 24, 40, 15]);
        if cx.world.life_percent(x) < 20 {
            w[2] = 50;
        }
        if cx.world.has_state(x, STATE_COLD) {
            w[7] = 0;
            w[8] = 0;
        }
        w[6] += (f41 - f39).signum() * 10;
        if !clear {
            w[5] = 0;
            w[6] = 0;
        }
        if near {
            w[15] = 10;
        }
    } else if clear {
        w[..13].copy_from_slice(&[0, 0, 0, 0, 0, 25, 25, 0, 15, 20, 10, 0, 20]);
        if unit_distance(cx, boss, x) > 25 {
            w[8] -= 5;
            w[10] = 20;
            w[5] = 0;
        }
        let f = (f41 - f39).signum() * 10;
        w[6] += f;
        w[8] += f;
        if n < 2 {
            w[6] -= 10;
        }
        if n > 3 {
            w[6] += 5;
        }
        if big_m > 60 {
            w[9] += 10;
        }
        if n < 2 && record_difficulty(cx) < 2 {
            w[9] = 0;
        }
        if c && !far {
            w[6] += 10;
            w[10] = 30;
            w[13] = 15;
        }
        if far {
            w[1] = 0;
            w[12] = 10;
            w[10] = 0;
            w[13] = 15;
            if w[9] == 0 {
                w[9] = 10;
            }
        }
        if further {
            w[9] = 20;
            w[14] = 60;
        }
        if near {
            w[15] = 15;
        }
    } else {
        w[..13].copy_from_slice(&[0, 0, 0, 0, 5, 0, 25, 0, 25, 40, 0, 0, 25]);
        if n < 2 {
            w[8] -= 5;
            w[6] = 0;
            w[1] = 25;
            w[9] = 0;
        }
        if c && !far {
            w[13] = 15;
        }
        if far {
            w[1] = 0;
            w[12] = 15;
            w[13] = 25;
            if w[9] == 0 {
                w[9] = 20;
            }
            if n < 2 {
                w[9] -= 5;
            }
        }
        if further {
            w[14] = 60;
        }
        if near {
            w[15] = 20;
        }
    }
    // 5.
    if cx
        .world
        .skill_check(game, boss, SKILL_DIABPRISON, Some(x), 0, 0)
    {
        w[9] = 0;
    }
    if w[15] != 0 {
        if let Some(pt) = portal {
            if !cx
                .world
                .skill_check(game, boss, SKILL_DIABPRISON, Some(pt), 0, 0)
            {
                w[15] = 0;
            }
        }
    }
    // 6.
    weighted(cx, boss, &w, 11)
}

/// "walk with think delete to (x, y)".
pub(super) fn walk_point_del<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    x: i32,
    y: i32,
) -> bool {
    move_to(
        game,
        cx,
        u,
        ModeTarget::Point(x, y),
        mode::WALK,
        1,
        move_flag::DELETE_THINKS,
    )
}

/// Diablo (51). No aip is read.
pub fn diablo<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    // 1, 2.
    let h = home_point(cx, u);
    let (x, big_m, n) = boss_pick(game, cx, u, Cull::Act, Score::Diablo(h));
    // 3.
    let k = diablo_choice(game, cx, u, x, big_m, n, h);
    // 4. The aura (Diablo has no `Skill8`).
    let (s8, m8) = cx.skill(p, 8);
    if k != 5 && s8 >= 0 {
        if let Some((st, _)) = skill_row(cx, s8).map(aura_states) {
            if st >= 0
                && !cx.world.has_state(u, st as u16)
                && use_skill(game, cx, u, m8, s8, target_of(x))
            {
                set_param(cx, u, 0, 0);
                return;
            }
        }
    }
    // 5.
    let cast = |game: &mut Game, cx: &mut Ctx<'_, W>, k: usize| {
        if cx.skill(p, k).0 < 0 {
            idle(game, cx, u, 2);
        } else {
            skill_k(game, cx, u, p, k, x);
        }
    };
    let mut keep = false;
    match k {
        1 => {
            set_velocity(cx, u, 0, 20, 0);
            // X = 0 is unreachable here: the choice with no X returns
            // 16, 11 or 4, and its only stored choice is 5, so case 1
            // always has X. The (0, 0) fallback never runs.
            let (xx, xy) = x.map_or((0, 0), |x| cx.world.position(x));
            walk_to_point(game, cx, u, xx, xy);
        }
        2 => {
            mode_at(game, cx, u, mode::ATTACK1, x);
        }
        3 => {
            mode_at(game, cx, u, mode::ATTACK2, x);
        }
        4 => {
            mode_at(game, cx, u, 11, x);
        }
        5 => {
            if cx.skill(p, 1).0 < 0 {
                idle(game, cx, u, 2);
            } else if cx.world.has_state(u, STATE_INFERNO) {
                cx.world.set_state(game, u, STATE_INFERNO, false);
                idle(game, cx, u, 2);
            } else {
                skill_k(game, cx, u, p, 1, x);
                set_param(cx, u, 0, 5);
                keep = true;
            }
        }
        6 => cast(game, cx, 3),
        7 => cast(game, cx, 2),
        8 => cast(game, cx, 4),
        9 => cast(game, cx, 7),
        10 => cast(game, cx, 5),
        13 => cast(game, cx, 6),
        12 => {
            circle(game, cx, u, x, 4, false);
        }
        14 => {
            set_velocity(cx, u, 0, 50, 100);
            let (hx, hy) = h.unwrap_or((0, 0));
            if !walk_point_del(game, cx, u, hx, hy) {
                delete_thinks(game, u);
                let d = path_distance(cx, u, hx, hy);
                let (ox, oy) = cx.world.position(u);
                let (sx, sy) = ((ox - hx).signum(), (oy - hy).signum());
                set_velocity(cx, u, 0, 50, 100);
                let (tx, ty) = (hx + (d >> 1) * sx, hy + (d >> 1) * sy);
                if !walk_point_del(game, cx, u, tx, ty) {
                    delete_thinks(game, u);
                    idle(game, cx, u, 2);
                }
            }
        }
        15 => match x.filter(|&x| is_player(game, x)) {
            None => idle(game, cx, u, 3),
            Some(xx) => {
                let g = cx.world.portal_guid(xx);
                let portal = g.and_then(|g| game.lists.find_unit(UnitType::Object, g));
                let (s7, m7) = cx.skill(p, 7);
                match (portal, g) {
                    (Some(_), Some(g)) if s7 >= 0 => {
                        use_skill(game, cx, u, m7, s7, ModeTarget::Point(g as i32, 2));
                    }
                    _ => idle(game, cx, u, 2),
                }
            }
        },
        16 => {
            set_velocity(cx, u, 0, 20, 0);
            wander(game, cx, u, 5);
        }
        _ => {
            let n = match cx.info.difficulty {
                0 => 12,
                1 => 8,
                _ => 4,
            };
            idle(game, cx, u, n);
        }
    }
    if !keep {
        set_param(cx, u, 0, 0);
    }
}

/// Diablo's alternate (also BaalCrab's): keep the type-10 command's
/// point across the re-install. No
/// draws.
pub fn diablo_alt<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    let (x, y) = find_command(cx, u, 10, false)
        .and_then(|k| command_mut(cx, u, k).map(|c| (c.params[1], c.params[2])))
        .unwrap_or((0, 0));
    if cx.world.has_state(u, STATE_INFERNO) {
        cx.world.set_state(game, u, STATE_INFERNO, false);
    }
    reinstall(game, cx, u);
    if find_command(cx, u, 10, false).is_none() && x != 0 && y != 0 {
        copy_command(
            cx,
            u,
            AiCommand {
                params: [10, x, y, 0, 0],
            },
        );
    }
    idle(game, cx, u, 1);
}

// ---- Izual ----------------------------------------------------------

/// `aidel` of the difficulty (monstats u8 +0x4F + d), no game-type gate.
fn aidel_raw<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, p: &TickParam) -> i32 {
    cx.tables.monstats.get(p.class).map_or(0, |r| {
        i32::from(match cx.info.difficulty {
            0 => r.aidel,
            1 => r.aidel_n,
            _ => r.aidel_h,
        })
    })
}

/// Izual (55).
pub fn izual<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    // 1.
    if param(cx, u, 0) == 0 {
        set_param(cx, u, 0, 1);
        cx.world.quest_call(game, u, QuestCall::IzualActivated);
    }
    izual_steps(game, cx, u, p);
}

/// Izual's steps 2–6 (also UberIzual).
pub(super) fn izual_steps<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    // "Nova": the skill use `(Sk1mode, Skill1, T, x, y)` with (x, y) = T's
    // position (one target, T, here); s := aip5, w := aip6.
    let nova = |game: &mut Game, cx: &mut Ctx<'_, W>| {
        skill_k(game, cx, u, p, 1, t);
        set_param(cx, u, 1, cx.aip(p, 5));
        set_param(cx, u, 2, cx.aip(p, 6));
    };
    // 2.
    let s = param(cx, u, 1);
    if s != 0 {
        idle(game, cx, u, s);
        set_param(cx, u, 1, 0);
        return;
    }
    let w = param(cx, u, 2);
    let has1 = cx.skill(p, 1).0 >= 0;
    // 3.
    if p.combat {
        if w > 0 || cx.chance(u, cx.aip(p, 1)) {
            if w > 0 {
                set_param(cx, u, 2, w - 1);
            }
            a1(game, cx, u, t);
        } else {
            set_param(cx, u, 2, 0);
            if has1 && pct(cx, u) < cx.aip(p, 4) {
                nova(game, cx);
            } else {
                idle(game, cx, u, aidel_raw(cx, p));
            }
        }
        return;
    }
    // 4.
    if has1 && p.distance < 10 {
        if w > 0 {
            walk_to(game, cx, u, t, 7);
            return;
        }
        if pct(cx, u) < cx.aip(p, 3) {
            nova(game, cx);
            return;
        }
    }
    // 5.
    if w < 1 && pct(cx, u) >= cx.aip(p, 2) {
        if p.distance < 11 {
            idle(game, cx, u, aidel_raw(cx, p));
        } else {
            radius(game, cx, u, t, 6, 9);
        }
        return;
    }
    // 6.
    walk_to(game, cx, u, t, 7);
}

// ---- the knights -------------------------------------------------

/// DoomKnight (72).
pub fn doom_knight<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    if !p.combat {
        if cx.chance(u, cx.aip(p, 3)) {
            walk_to_method13(game, cx, u, t, 7);
        } else {
            idle(game, cx, u, cx.aip(p, 4));
        }
    } else if cx.chance(u, cx.aip(p, 1)) {
        a1(game, cx, u, t);
    } else {
        idle(game, cx, u, cx.aip(p, 2));
    }
}

/// "Shot allowed": not a monster whose monster data byte +0x0E
/// (`nComponent[10]`) is ≥ 4.
fn shot_allowed<W: AiHost + ?Sized>(game: &Game, cx: &Ctx<'_, W>, u: UnitId) -> bool {
    !(is_monster(game, u) && cx.world.component(u, 10) >= 4)
}

/// AbyssKnight (73).
pub fn abyss_knight<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let d = p.distance;
    // 1.
    let (s2, m2) = cx.skill(p, 2);
    if s2 >= 0 {
        if let Some((st, _)) = skill_row(cx, s2).map(aura_states) {
            if st >= 0
                && !cx.world.has_state(u, st as u16)
                && cx.world.life_percent(u) < cx.aip(p, 1)
                && pct(cx, u) < cx.aip(p, 2)
            {
                use_skill(game, cx, u, m2, s2, ModeTarget::Point(0, 0));
                return;
            }
        }
    }
    // 2.
    if p.combat {
        if cx.chance(u, cx.aip(p, 3)) {
            a1(game, cx, u, t);
        } else {
            idle(game, cx, u, cx.aip(p, 4));
        }
        return;
    }
    // 3.
    let mut c = param(cx, u, 0);
    if d < cx.aip(p, 5) && c <= 0 {
        c = cx.aip(p, 6);
        set_param(cx, u, 0, c);
    }
    // 4.
    if cx.skill(p, 1).0 >= 0 && c == 0 && shot_allowed(game, cx, u) {
        skill_k(game, cx, u, p, 1, t);
        set_param(cx, u, 0, cx.aip(p, 6));
        return;
    }
    // 5.
    if c > 0 {
        set_param(cx, u, 0, c - 1);
    }
    // 6. Steps stored as a byte.
    if pct(cx, u) < cx.aip(p, 7) {
        let steps = (cx.world.seed(u).step() & 1) as i32 + 6;
        set_velocity(cx, u, 2, 0, steps);
        walk_to(game, cx, u, t, 7);
        return;
    }
    // 7.
    if d < cx.aip(p, 8) {
        circle(game, cx, u, t, 3, false);
    } else {
        idle(game, cx, u, 15);
    }
}

/// doomknight1 (`BaseId` of the knight scan).
const DOOMKNIGHT1: i32 = 310;

/// OblivionKnight (74).
pub fn oblivion_knight<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let d = p.distance;
    let frame = game.frame;
    // 1. The nearest knight N (W, the nearest hurt one, is unused).
    let mut knight: Option<(UnitId, i32)> = None;
    for v in scan_units(game, u) {
        if v == u
            || !is_monster(game, v)
            || dying(cx, v)
            || cx.world.unit_flags(v) & 0x2 == 0
            || unit_base(cx, v) != DOOMKNIGHT1
            || !pairing(cx, u, v)
            || cx.world.alignment(v) != 0
        {
            continue;
        }
        let dv = sq_dist(cx, u, v);
        if dv <= 2500 && dv < knight.map_or(0x7FFF_FFFF, |k| k.1) {
            knight = Some((v, dv));
        }
    }
    // 2.
    if d < cx.aip(p, 1) {
        // 2.1: `Skill4` > 0 and the state > 0 (edge case 6).
        let (s4, _) = cx.skill(p, 4);
        if s4 > 0 {
            if let Some((_, ts)) = skill_row(cx, s4).map(aura_states) {
                if ts > 0 && !t.is_some_and(|t| cx.world.has_state(t, ts as u16)) {
                    skill_k(game, cx, u, p, 4, t);
                    set_param(cx, u, 0, game.after(cx.aip(p, 3)));
                    return;
                }
            }
        }
        // 2.2.
        if let Some((nk, nd)) = knight {
            if nd > d.wrapping_mul(d) && walk_to(game, cx, u, Some(nk), 4) {
                return;
            }
        }
        // 2.3.
        set_velocity(cx, u, 2, 50, 0);
        if escape(game, cx, u, t, 10, true, false) {
            return;
        }
        // 2.4.
        if cx.skill(p, 3).0 >= 0 {
            skill_k(game, cx, u, p, 3, t);
            return;
        }
    }
    // 3.
    let (s, e, _) = cx.world.secondary_target(game, u, cx.store);
    if s.is_some() && e < cx.aip(p, 2) {
        if cx.skill(p, 6).0 > 0 && frame > param(cx, u, 0) && pct(cx, u) < cx.aip(p, 4) {
            skill_k(game, cx, u, p, 6, t);
            set_param(cx, u, 0, game.after(cx.aip(p, 3)));
            return;
        }
        if pct(cx, u) < cx.aip(p, 5) {
            if cx.skill(p, 3).0 >= 0 && pct(cx, u) < cx.aip(p, 6) {
                skill_k(game, cx, u, p, 3, s);
                return;
            }
            if cx.skill(p, 1).0 >= 0 && shot_allowed(game, cx, u) {
                skill_k(game, cx, u, p, 1, s);
                return;
            }
        }
    }
    // 4.
    if d > cx.aip(p, 8) && pct(cx, u) < cx.aip(p, 7) {
        wander_near_opt(game, cx, u, t, 6);
        return;
    }
    // 5.
    if pct(cx, u) >= 70 {
        idle(game, cx, u, 10);
    } else {
        circle(game, cx, u, t, 3, false);
    }
}

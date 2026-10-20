//! The pets, towns, traps and spawners of the one-row AIs, with
//! the shared pet helpers (run-del, wander', search capped, pet
//! move, pet follow) and the hireling helpers. Brackets in the
//! comments are the usual (Normal) values.

use crate::game::Game;
use crate::units::{UnitId, UnitType};

use super::bodies::{a1, a2, param, pct, roll, set_param};
use super::bodies4::{dir8, walk_point_del, DIR_DX, DIR_DY};
use super::common::*;
use super::npc::{npc_commands, npc_home, npc_map_ai};
use super::tactics::*;
use super::{
    delete_thinks, idle, install, main_search, main_search_with, mode, AiCommand, AiHost, Ctx,
    ModeTarget, QuestHook, TickParam,
};

/// Unit flag 0x20000 (unit +0xC4): no drop.
pub(super) const FLAG_NO_DROP: u32 = 0x2_0000;
/// Unit flag 0x40000000 (`petIgnore`).
const FLAG_PET_IGNORE: u32 = 0x4000_0000;
/// necroskeleton, the catch-up probe class (pet move k 3).
const NECROSKELETON: i32 = 363;
/// Event type 0 (MODECHANGE).
const EVENT_MODECHANGE: u32 = 0;

/// "death": mode 0 at (0, 0).
pub(super) fn death<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    mode_point_raw(game, cx, u, mode::DEATH, 0, 0);
}

/// Unit flag 0x20000, then death.
pub(super) fn no_drop_death<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    cx.world.set_unit_flag(u, FLAG_NO_DROP);
    death(game, cx, u);
}

/// The unit's room is in town; no room counts as out of
/// town.
pub(super) fn in_town<W: AiHost + ?Sized>(game: &Game, cx: &Ctx<'_, W>, u: UnitId) -> bool {
    room_of(game, u).is_some_and(|r| cx.world.in_town(game, r))
}

/// `Run` × 100 / `Velocity` − 100 (signed, truncating); 100 when
/// `Velocity` ≤ 0 or the value is ≥ 100 (negative values are kept, as
/// in the melee step).
pub(super) fn run_speed<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, p: &TickParam) -> i32 {
    let Some(r) = cx.tables.monstats.get(p.class) else {
        return 100;
    };
    let vel = i32::from(r.velocity as i16);
    if vel <= 0 {
        return 100;
    }
    let v = i32::from(r.run as i16) * 100 / vel - 100;
    if v >= 100 {
        100
    } else {
        v
    }
}

/// The player with GUID `g`.
pub(super) fn player_by_guid(game: &Game, g: i32) -> Option<UnitId> {
    game.lists.find_unit(UnitType::Player, g as u32)
}

/// Adds a mode-change timer event `n` base ticks from now (1 stays the
/// next tick), no delete.
pub(super) fn modechange_at(game: &mut Game, u: UnitId, n: i32) {
    let at = game
        .frame
        .wrapping_add(if n == 1 { 1 } else { game.ticks(n) });
    // A monster has a timer class: the error cannot happen.
    let _ = game.schedule_event(u, EVENT_MODECHANGE, at, None, 0, 0);
}

/// The path record written directly (pet
/// move k 1): each nonzero argument overwrites its field, steps capped at
/// 77, no method mapping.
fn velocity_raw<W: AiHost + ?Sized>(cx: &mut Ctx<'_, W>, u: UnitId, method: i32, speed: i32) {
    let v = &mut cx.store.entry(u).velocity;
    if method != 0 {
        v.method = method;
    }
    if speed != 0 {
        v.speed = speed;
    }
    v.steps = 77;
}

// ---- shared pet helpers ----------------------------------------------

/// Run-del: run (walk with the velocity reset under
/// state 60), path step 1; on failure delete the thinks.
pub(super) fn run_del<W: AiHost + ?Sized>(
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
        mode::RUN,
        1,
        move_flag::DELETE_THINKS,
    )
}

/// Wander': X is not read (edge case 2); the
/// wander draws around U's own position with n as a byte, then mode 2
/// there, path step 1. The mode-change result.
pub(super) fn wander_prime<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    n: i32,
) -> bool {
    let center = cx.world.position(u);
    let (x, y) = wander_point(cx.world.seed(u), center, i32::from(n as u8));
    move_to(game, cx, u, ModeTarget::Point(x, y), mode::WALK, 1, 0)
}

/// Search capped: the main search's
/// target only when E ≤ k (signed); E and M as the search wrote them.
pub(super) fn search_capped<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    k: i32,
) -> (Option<UnitId>, i32, bool) {
    let s = main_search(game, cx, u);
    let t = if s.distance <= k { s.target } else { None };
    (t, s.distance, s.combat)
}

/// The owner view T0: the main search run for O with the pet's
/// control; a T0 with unit flag 0x40000000 (`petIgnore`) is dropped.
/// Returns T0 and its search distance.
pub(super) fn owner_view<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    o: UnitId,
) -> (Option<UnitId>, i32) {
    let s = main_search_with(game, cx, o, u);
    let t = s
        .target
        .filter(|&t| cx.world.unit_flags(t) & FLAG_PET_IGNORE == 0);
    (t, s.distance)
}

/// The direction table (pet move k 0): j by e.
const PET_J: [usize; 8] = [4, 3, 2, 1, 0, 7, 6, 5];

/// Which helper moves a pet: the pet move or the hireling
/// move.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mover {
    Pet,
    Hireling,
}

/// "Walk-del" or "run-del" to (x, y) by `run`.
fn go_del<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    run: bool,
    x: i32,
    y: i32,
) -> bool {
    if run {
        run_del(game, cx, u, x, y)
    } else {
        walk_point_del(game, cx, u, x, y)
    }
}

/// O's other pets crowding U (pet move k 4, callback): living
/// nodes other than U within full-size distance 1 of U.
fn crowding<W: AiHost + ?Sized>(game: &Game, cx: &Ctx<'_, W>, o: UnitId, u: UnitId) -> usize {
    cx.world
        .pets(game, o)
        .into_iter()
        .filter(|&v| v != u && unit_distance(cx, u, v) <= 1)
        .count()
}

/// Pet move k 3 (catch up), pet and hireling form.
fn catch_up<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    o: UnitId,
    u: UnitId,
) -> bool {
    let room = room_of(game, o);
    let (ox, oy) = cx.world.position(o);
    let cl = cx.world.spot_class(game, room, ox, oy);
    let class = fixed_class(cx, NECROSKELETON);
    let Some((x, y)) = cx.world.free_spot(game, room, cl, class) else {
        return false;
    };
    let Some(r) = cx.world.room_at(game, o, x, y) else {
        return false;
    };
    if !cx.world.place_unit(game, u, Some(r), x, y) {
        return false;
    }
    let _ = game.lists.queue_update(u);
    cx.world.set_unit_flags2(u, 0x1_0000);
    idle(game, cx, u, 5);
    true
}

/// Pet movement and hireling
/// move → true when something was started or
/// scheduled.
#[allow(clippy::too_many_arguments)]
pub(super) fn pet_move<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    who: Mover,
    o: UnitId,
    u: UnitId,
    k: i32,
    run: bool,
    speed: i32,
    n: i32,
) -> bool {
    let f = cx.world.path_final_point(o);
    let hire = who == Mover::Hireling;
    match k {
        0 => {
            let (ox, oy) = cx.world.position(o);
            let mut e = dir8(cx.world.direction64_to(o, f.0, f.1));
            let o_room = room_of(game, o);
            let c0 = cx.world.coord_index(game, o_room, ox, oy);
            for _ in 0..8 {
                let j = PET_J[e];
                let q = (f.0 + 8 * DIR_DX[j], f.1 + 8 * DIR_DY[j]);
                // Pet move k 0: the midpoint try belongs to the "index
                // is c0" case only (Q's try did not start; no second
                // velocity call); an index ≠ c0 tries nothing. The
                // hireling move has no midpoint try.
                if cx.world.coord_index(game, o_room, q.0, q.1) == c0 {
                    set_velocity(cx, u, 0, speed, 40);
                    if go_del(game, cx, u, run && !hire, q.0, q.1) {
                        return true;
                    }
                    if !hire {
                        let (ux, uy) = cx.world.position(u);
                        let mx = ((ux.wrapping_add(q.0) as u32) >> 1) as i32;
                        let my = ((uy.wrapping_add(q.1) as u32) >> 1) as i32;
                        if go_del(game, cx, u, run, mx, my) {
                            return true;
                        }
                    }
                }
                e = (e + 1) & 7;
            }
            delete_thinks(game, u);
            if hire {
                wander_prime(game, cx, u, 4);
                return true;
            }
            if wander_prime(game, cx, u, 4) {
                return true;
            }
            let (ux, uy) = cx.world.position(u);
            let (mx, my) = ((ux.wrapping_add(ox)) / 2, (uy.wrapping_add(oy)) / 2);
            if run && run_del(game, cx, u, mx, my) {
                return true;
            }
            if walk_point_del(game, cx, u, mx, my) {
                return true;
            }
            walk_to_point(game, cx, u, ox, oy)
        }
        1 => {
            let walking = cx.world.anim_mode(o) == mode::WALK;
            if hire && walking {
                set_velocity(cx, u, 0, 0, 100);
                walk_to_point(game, cx, u, f.0, f.1);
                return true;
            }
            if !hire && walking {
                set_velocity(cx, u, 0, 0, 100);
                if walk_point_del(game, cx, u, f.0, f.1) {
                    return true;
                }
                let (ux, uy) = cx.world.position(u);
                if walk_point_del(
                    game,
                    cx,
                    u,
                    ux.wrapping_add(f.0) / 2,
                    uy.wrapping_add(f.1) / 2,
                ) {
                    return true;
                }
            }
            let s = match (speed, hire) {
                (0, false) => roll(cx, u, 40) + 40,
                (0, true) => roll(cx, u, 15) + 50,
                (s, _) => s,
            };
            let (index, hist) = cx.world.position_history(o);
            let mut b = if index == 0 { 19 } else { (index - 1) % 20 };
            let mut raw_done = false;
            for _ in 0..20 {
                let (x, y) = hist[b];
                b = if b == 0 { 19 } else { b - 1 };
                if !hire && (x == 0 || y == 0) {
                    continue;
                }
                if path_distance(cx, u, x, y) as u32 <= 5 {
                    continue;
                }
                set_velocity(cx, u, 0, s, 100);
                // Edge case 3: the first try runs for a pet.
                let first = if run && !hire {
                    run_to_point(game, cx, u, x, y)
                } else {
                    walk_point_del(game, cx, u, x, y)
                };
                if first {
                    return true;
                }
                delete_thinks(game, u);
                set_velocity(cx, u, 15, s, 100);
                let second = if run && hire {
                    run_to_point(game, cx, u, x, y)
                } else {
                    go_del(game, cx, u, run, x, y)
                };
                if second {
                    return true;
                }
                if hire {
                    delete_thinks(game, u);
                }
                if !raw_done {
                    raw_done = true;
                    velocity_raw(cx, u, 1, s);
                    let third = if run && hire {
                        run_to_point(game, cx, u, x, y)
                    } else {
                        go_del(game, cx, u, run, x, y)
                    };
                    if third {
                        return true;
                    }
                    if hire {
                        delete_thinks(game, u);
                    }
                }
            }
            let d = ((unit_distance(cx, u, o) as u32) >> 2).max(4) as i32;
            set_velocity(cx, u, 15, 0, 0);
            let started = wander_prime(game, cx, u, d);
            hire || started
        }
        2 => {
            if hire {
                if roll(cx, u, 100) >= 10 {
                    idle(game, cx, u, 15);
                    return true;
                }
                let r = roll(cx, u, 3) + 3;
                if wander_prime(game, cx, u, r) {
                    return true;
                }
                delete_thinks(game, u);
                set_velocity(cx, u, 0, 0, 40);
                walk_to_point(game, cx, u, f.0, f.1);
                return true;
            }
            if roll(cx, u, 100) < 10 {
                let r = roll(cx, u, 3) + 3;
                if wander_prime(game, cx, u, r) {
                    return true;
                }
                delete_thinks(game, u);
                set_velocity(cx, u, 0, 0, 40);
                if walk_point_del(game, cx, u, f.0, f.1) {
                    return true;
                }
            }
            idle(game, cx, u, 15);
            true
        }
        3 => catch_up(game, cx, o, u),
        4 => {
            if crowding(game, cx, o, u) >= 1 {
                delete_thinks(game, u);
                if escape(game, cx, u, Some(o), n, true, false) {
                    return true;
                }
                delete_thinks(game, u);
                let started = wander_prime(game, cx, u, n);
                if hire || started {
                    return true;
                }
            } else if hire {
                idle(game, cx, u, 15);
                return true;
            }
            idle(game, cx, u, 15);
            true
        }
        5 => {
            set_velocity(cx, u, if hire { 0 } else { 7 }, 0, 0);
            if wander_near(game, cx, u, o, n) {
                return true;
            }
            delete_thinks(game, u);
            if escape(game, cx, u, Some(o), n, true, false) {
                return true;
            }
            set_velocity(cx, u, 0, 0, 40);
            if hire {
                walk_to_point(game, cx, u, f.0, f.1);
                return true;
            }
            walk_point_del(game, cx, u, f.0, f.1)
        }
        _ => false,
    }
}

/// Pet follow `(game, U, S, O, M, tick, quiet, n)` → the
/// pet-move result, or false.
#[allow(clippy::too_many_arguments)]
pub(super) fn pet_follow<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    s: Option<UnitId>,
    o: UnitId,
    m: bool,
    quiet: bool,
    n: i32,
) -> bool {
    // 1.
    let d = unit_distance(cx, u, o);
    let r = n.wrapping_add(cx.world.pet_count(o) >> 1).min(36);
    let om = cx.world.anim_mode(o);
    // 2.
    if d <= 1 && om == mode::NEUTRAL && !m {
        return pet_move(game, cx, Mover::Pet, o, u, 5, false, 0, r);
    }
    // 3.
    if s.is_some() && !in_town(game, cx, u) {
        if d <= 80 {
            return false;
        }
        return pet_move(game, cx, Mover::Pet, o, u, 3, false, 0, r);
    }
    // 4.
    let f = cx.world.path_final_point(o);
    let mut k = 2;
    if matches!(om, 2 | 3 | 6) {
        k = 0;
    }
    let tp = cx.world.path_target_point(o);
    if tp.0 != f.0 && tp.1 != f.1 {
        k = 0;
    }
    let (ox, oy) = cx.world.position(o);
    let (ux, uy) = cx.world.position(u);
    if cx.world.coord_index(game, room_of(game, o), ox, oy)
        != cx.world.coord_index(game, room_of(game, u), ux, uy)
    {
        k = 1;
    }
    if d > r {
        k = 1;
    }
    if d > 50 {
        k = 3;
    }
    let (px, py) = cx.world.last_placed_point(o);
    if (path_distance(cx, u, px, py) as u32) < 28 && ((k == 1 && d < 30) || k == 2) {
        k = 4;
    }
    // 5.
    if quiet && k == 2 {
        return false;
    }
    pet_move(game, cx, Mover::Pet, o, u, k, false, 0, r)
}

// ---- NecroPet ----------------------------------------------------------

/// The start both NecroPet branches share: `Some(O)` to go on.
fn pet_start<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
) -> Option<UnitId> {
    let Some(o) = minion_owner(game, cx, u) else {
        let g = param(cx, u, 2);
        match player_by_guid(game, g).filter(|_| g != 0) {
            Some(pl) => {
                pet_move(game, cx, Mover::Pet, pl, u, 3, false, 0, 0);
            }
            None => idle(game, cx, u, 10),
        }
        return None;
    };
    set_param(cx, u, 2, guid_of(game, Some(o)));
    Some(o)
}

/// NecroPet (67).
pub fn necro_pet<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    delete_thinks(game, u);
    let ranged = param(cx, u, 0) != 0;
    let Some(o) = pet_start(game, cx, u) else {
        return;
    };
    if ranged {
        necro_ranged(game, cx, u, p, o);
    } else {
        necro_melee(game, cx, u, p, o);
    }
}

/// The follow of melee step 6 / ranged step 3: draw r, quiet below 15.
fn necro_follow<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    s: Option<UnitId>,
    o: UnitId,
    m: bool,
) -> bool {
    let r = pct(cx, u);
    if r < 15 {
        pet_follow(game, cx, u, s, o, m, true, 8)
    } else {
        pet_follow(game, cx, u, s, o, m, false, 7)
    }
}

/// Melee.
fn necro_melee<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
    o: UnitId,
) {
    // 1.
    let d = unit_distance(cx, u, o);
    if d > 50 {
        pet_move(game, cx, Mover::Pet, o, u, 3, false, 0, 0);
        return;
    }
    // 2., 3.
    let v = run_speed(cx, p);
    if d > 28 {
        pet_move(game, cx, Mover::Pet, o, u, 0, false, v, 0);
        return;
    }
    // 4.
    let (t0, _) = owner_view(game, cx, u, o);
    let (mut s, e, m) = search_capped(game, cx, u, 24);
    if s.is_none() || e > 6 {
        s = t0.filter(|&t| unit_distance(cx, u, t) < 36);
    }
    // 5.
    s = s.filter(|&s| cx.world.can_reach_directly(game, u, s));
    // 6.
    if necro_follow(game, cx, u, s, o, m) {
        return;
    }
    // 7.
    let Some(st) = s.filter(|_| !in_town(game, cx, u)) else {
        wander_prime(game, cx, u, 4);
        return;
    };
    // 8.
    if m {
        if roll(cx, u, 100) < 80 {
            a1(game, cx, u, Some(st));
        } else {
            idle(game, cx, u, 10);
        }
        return;
    }
    // 9.
    set_velocity(cx, u, 0, 0, 12);
    walk_to_method13(game, cx, u, Some(st), 7);
}

/// Ranged (the skeleton mage).
fn necro_ranged<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
    o: UnitId,
) {
    // 1. The distance U→O is computed and discarded.
    let (t0, _) = owner_view(game, cx, u, o);
    // 2.
    let (mut s, e, m) = cx.world.secondary_target(game, u, cx.store);
    if s.is_none() || e > 15 {
        s = t0.filter(|&t| !cx.world.line_blocked(game, u, t) && unit_distance(cx, u, t) < 20);
    }
    // 3.
    if necro_follow(game, cx, u, s, o, m) {
        return;
    }
    // 4.
    let Some(st) = s.filter(|_| !in_town(game, cx, u)) else {
        wander_prime(game, cx, u, 4);
        return;
    };
    // 5.
    if roll(cx, u, 100) < 80 {
        skill_k(game, cx, u, p, 1, Some(st));
    } else if roll(cx, u, 100) < 75 {
        idle(game, cx, u, 10);
    } else {
        circle(game, cx, u, Some(st), 3, false);
    }
}

// ---- MinionSpawner -----------------------------------------------------

/// minion1, suicideminion1 (the count scan's `BaseId`s).
const MINION1: i32 = 453;
const SUICIDEMINION1: i32 = 461;

/// MinionSpawner (121). No draws.
pub fn minion_spawner<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let n = param(cx, u, 1);
    // 1. Nothing scheduled (edge case 1).
    if n >= cx.aip(p, 1) {
        return;
    }
    let aip2 = cx.aip(p, 2);
    // 2., 3.
    if p.distance > cx.aip(p, 4) || game.frame < param(cx, u, 0) {
        idle(game, cx, u, aip2);
        return;
    }
    // 4.
    let (b1, b2) = (fixed_class(cx, MINION1), fixed_class(cx, SUICIDEMINION1));
    let c = scan_units(game, u)
        .into_iter()
        .filter(|&v| {
            v != u
                && is_monster(game, v)
                && !dying(cx, v)
                && matches!(unit_base(cx, v), b if b == b1 || b == b2)
                && cx.world.alignment(v) == 0
        })
        .count() as i32;
    // 5.
    if cx.skill(p, 1).0 >= 0 && c < aip2 {
        set_param(cx, u, 1, n.wrapping_add(1));
        set_param(cx, u, 0, game.after(cx.aip(p, 3)));
        skill_k(game, cx, u, p, 1, p.target);
        return;
    }
    // 6.
    idle(game, cx, u, aip2);
}

// ---- Towner ------------------------------------------------------------

/// Towner (41).
pub fn towner<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    if npc_home(game, cx, u) || npc_commands(game, cx, u) || npc_map_ai(game, cx, u) {
        return;
    }
    idle(game, cx, u, 12);
}

// ---- EvilHole ----------------------------------------------------------

/// demonhole (the Uber hole's class).
const DEMONHOLE: i32 = 711;
/// State 184 `uberminion`, overlay 202.
const STATE_UBERMINION: u16 = 184;
const OVERLAY_UBERMINION: i32 = 202;

/// EvilHole (76). No draws.
pub fn evil_hole<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let m = cx.world.anim_mode(u);
    // 1.
    if param(cx, u, 0) <= 0 {
        set_param(cx, u, 0, game.after(cx.aip(p, 2)));
        set_param(cx, u, 1, cx.aip(p, 1));
    }
    let n = param(cx, u, 1);
    match m {
        // 2.
        mode::NEUTRAL => {
            if p.distance > 5 {
                idle(game, cx, u, 5);
            } else {
                mode_point_raw(game, cx, u, 10, 0, 0);
                wait(game, cx, u, 20);
            }
        }
        // 3.
        10 => {
            mode_point_raw(game, cx, u, 11, 0, 0);
            wait(game, cx, u, 20);
        }
        // 4.
        11 if n > 0 => {
            if game.frame > param(cx, u, 0) {
                set_param(cx, u, 0, game.after(cx.aip(p, 2)));
                let info = spawn_info(game, cx, u, 0);
                if let Some(room) = cx.world.room_at(game, u, info.x, info.y) {
                    if let Some(c) =
                        cx.spawn_monster(game, room, info.x, info.y, info.class, info.mode, 2, 0x42)
                    {
                        set_param(cx, u, 1, n - 1);
                        cx.world.set_unit_flag(c, 0x0402_0000);
                        if cx.world.class(u) == DEMONHOLE {
                            cx.world.set_state(game, c, STATE_UBERMINION, true);
                            cx.world.start_overlay(c, OVERLAY_UBERMINION);
                        }
                    }
                }
            }
            wait(game, cx, u, cx.aip(p, 2));
        }
        // 5.
        _ => death(game, cx, u),
    }
}

// ---- Hireable ------------------------------------------------------------

/// The hirelings by class.
const ROGUEHIRE: i32 = 271;
const ACT3HIRE: i32 = 359;

fn melee_hire(cls: i32) -> bool {
    matches!(cls, 338 | 560 | 561)
}

/// State 12.
pub(super) const STATE_12: u16 = 12;

/// Hireable (61).
pub fn hireable<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let cls = cx.world.class(u);
    // 1.
    let Some(o) = minion_owner(game, cx, u).filter(|&o| is_player(game, o)) else {
        install(game, cx, u, if cls == ROGUEHIRE { 5 } else { 6 });
        idle(game, cx, u, 10);
        return;
    };
    // 2.
    if cx.world.has_state(u, STATE_12) {
        cx.world.set_state(game, u, STATE_12, false);
    }
    // 3. Nothing scheduled.
    let m = cx.world.anim_mode(u);
    if m == mode::WALK || m == mode::RUN {
        return;
    }
    // 4.
    let pf = param(cx, u, 0);
    let (far, near, h) = if 16 < pf && pf < 20 {
        (2 * pf, pf, pf >> 1)
    } else {
        (24, 16, 5)
    };
    let mv = |game: &mut Game, cx: &mut Ctx<'_, W>, k, run, speed| {
        pet_move(game, cx, Mover::Hireling, o, u, k, run, speed, 0);
    };
    // 5.
    let d = unit_distance(cx, u, o) as u32;
    if d > 100 {
        mv(game, cx, 3, false, 0);
        return;
    }
    if d > far as u32 {
        mv(game, cx, 1, true, 60);
        return;
    }
    if d > near as u32 {
        match cx.world.anim_mode(o) {
            2 | 6 => {
                mv(game, cx, 0, false, 0);
                return;
            }
            3 => {
                mv(game, cx, 0, true, 60);
                return;
            }
            _ => {}
        }
    }
    // 6.
    if m != mode::NEUTRAL {
        delete_thinks(game, u);
        idle(game, cx, u, 5);
        return;
    }
    // 7.
    let q = !melee_hire(cls);
    let pattern = cx.world.path_pattern(u);
    let b = cx.world.pattern_collides(game, u, pattern, 0x40);
    if b && (cx.world.seed(u).mask(128) as i32) < if q { 12 } else { 6 } {
        wander(game, cx, u, 5);
        return;
    }
    // 8.
    if !in_town(game, cx, u) {
        let (s, e, _) = cx.world.secondary_target(game, u, cx.store);
        if let Some(s) = s.filter(|_| (e as u32) < 25) {
            hireling_attack(game, cx, u, p, cls, o, s);
            return;
        }
    }
    // 9.
    if b {
        wander(game, cx, u, 5);
        return;
    }
    // 10.
    let (ox, oy) = cx.world.position(o);
    let (ux, uy) = cx.world.position(u);
    if cx.world.coord_index(game, room_of(game, o), ox, oy)
        != cx.world.coord_index(game, room_of(game, u), ux, uy)
    {
        mv(game, cx, 0, true, 60);
        return;
    }
    // 11.
    if unit_distance(cx, u, o) <= 1 {
        let h = i32::from(h.wrapping_sub(1) as u8);
        set_velocity(cx, u, 7, 0, 0);
        if wander_near(game, cx, u, o, h) {
            return;
        }
        delete_thinks(game, u);
        if escape(game, cx, u, Some(o), h, true, false) {
            return;
        }
        set_velocity(cx, u, 0, 0, 40);
        let f = cx.world.path_final_point(o);
        walk_to_point(game, cx, u, f.0, f.1);
        return;
    }
    // 12.
    if pct(cx, u) < 5 {
        wander_near(game, cx, u, o, near);
        return;
    }
    // 13.
    idle(game, cx, u, 5);
}

/// Hireling attack.
fn hireling_attack<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
    cls: i32,
    o: UnitId,
    s: UnitId,
) {
    let pf = param(cx, u, 0);
    // 1.
    let a = if melee_hire(cls) {
        98
    } else {
        pf.wrapping_add(40)
            .wrapping_add(2i32.wrapping_mul(cx.world.stat(u, 12)))
            .min(95)
    };
    // 2.
    let d = unit_distance(cx, u, s);
    // 3.
    let Some(w) = cx.world.hireling_id(game, o, u) else {
        idle(game, cx, u, 10);
        return;
    };
    // 4.
    let ok = pct(cx, u) < a;
    set_param(cx, u, 0, if ok { 0 } else { pf.wrapping_add(10) });
    // 5.
    if cx.aip(p, 1) == 0 {
        if d < 4 && pct(cx, u) < 50 {
            if wander_near(game, cx, u, o, 4) {
                return;
            }
            delete_thinks(game, u);
            if escape(game, cx, u, Some(s), 4, true, false) {
                return;
            }
            hireling_skill(game, cx, u, p, cls, w, s);
        } else if ok {
            hireling_skill(game, cx, u, p, cls, w, s);
        } else {
            idle(game, cx, u, 10);
        }
        return;
    }
    // 6.
    if d >= 3 || !cx.world.in_melee_range(game, u, s) {
        // `MeleeRng` as the step count, overwritten by the run's step 1.
        let rng = cx
            .tables
            .monstats2
            .get(p.class2)
            .map_or(0, |r| i32::from(r.meleerng));
        cx.world.set_path_steps(u, rng);
        run_to(game, cx, u, Some(s), move_flag::FORCE_LOS);
        return;
    }
    if ok {
        hireling_skill(game, cx, u, p, cls, w, s);
    } else {
        idle(game, cx, u, 10);
    }
}

/// Inferno.
const SKILL_INFERNO: i32 = 41;

/// Hireling skill.
fn hireling_skill<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
    cls: i32,
    w: i32,
    s: UnitId,
) {
    // 1.
    if cx.monstats(cls).is_none() {
        return;
    }
    // 2.
    let lvl = cx.world.stat(u, 12);
    let Some(h) = cx.world.hireling_row(game, w, lvl) else {
        return;
    };
    // 3. Unwritten weights count as −1 (edge case 4).
    let delta = (lvl - h.level).max(0);
    let mut acc = h.default_chance;
    let mut c = [-1i32; 6];
    let count = cx.tables.skills.len() as i32;
    let states = cx.world.states_count();
    for (i, &k) in h.skill.iter().enumerate() {
        if k < 1 || k >= count {
            break;
        }
        c[i] = 0;
        let Some(level) = cx.world.skill_level(u, k, false).filter(|&l| l > 0) else {
            continue;
        };
        let row = skill_row(cx, k);
        let (aura, _) = row.map_or((-1, -1), aura_states);
        let aitype = row.map_or(0, |r| r.aitype);
        let ok = aitype != 1 || aura < 0 || aura >= states || !cx.world.has_state(u, aura as u16);
        if !ok {
            continue;
        }
        if k == SKILL_INFERNO && level / 2 + 4 < reach_distance(cx, u, s) {
            continue;
        }
        acc = acc
            .wrapping_add(h.chance[i])
            .wrapping_add(h.chance_per_lvl[i].wrapping_mul(delta) / 4);
        c[i] = acc;
    }
    // 4.
    let r = roll(cx, u, acc.wrapping_add(1));
    // 5.
    if r >= h.default_chance {
        if let Some(i) = (0..6).find(|&i| c[i] >= r) {
            let k = h.skill[i];
            if k > 0 {
                if skill_row(cx, k).is_some_and(|r| r.aura) {
                    cx.world.make_right_skill(game, u, k);
                    idle(game, cx, u, 10);
                } else {
                    use_skill(game, cx, u, h.mode[i], k, ModeTarget::Unit(s));
                }
                return;
            }
        }
    }
    // 6.
    match cls {
        ROGUEHIRE => skill_k(game, cx, u, p, 1, Some(s)),
        338 | ACT3HIRE | 560 | 561 => {
            if cx.world.in_melee_range(game, u, s) {
                a1(game, cx, u, Some(s));
            } else {
                idle(game, cx, u, 10);
            }
        }
        _ => idle(game, cx, u, 10),
    }
}

// ---- QuillMother ---------------------------------------------------------

/// QuillMother (75).
pub fn quill_mother<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    // 1.
    if cx.ai_state_set(u) {
        let ty = t.and_then(|t| type_of(game, t)).map_or(6, |ty| ty as i32);
        // Params 3 and 4 are not written (no minion AI reads
        // them); 0 here.
        let cmd = AiCommand {
            params: [1, ty, guid_of(game, t), 0, 0],
        };
        command_minions(game, cx, u, cmd);
        if p.combat {
            a1(game, cx, u, t);
        } else {
            walk_to(game, cx, u, t, 7);
        }
        return;
    }
    // 2.
    if !p.combat {
        if pct(cx, u) < cx.aip(p, 2) {
            walk_to(game, cx, u, t, 7);
        } else {
            idle(game, cx, u, cx.aip(p, 4));
        }
        return;
    }
    // 3.
    if pct(cx, u) < cx.aip(p, 1) {
        a1(game, cx, u, t);
    } else {
        idle(game, cx, u, cx.aip(p, 3));
    }
}

// ---- BaalTentacle --------------------------------------------------------

/// BaalTentacle (139).
pub fn baal_tentacle<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    // 1.
    if cx.world.owner(game, u).is_none_or(|o| cx.world.is_dead(o)) {
        cx.world.kill(game, u, None);
        return;
    }
    // 2.
    let mut e = param(cx, u, 2);
    if e == 0 {
        let a3 = cx.aip(p, 3);
        e = game.after((roll(cx, u, a3).wrapping_add(a3)).wrapping_mul(25));
        set_param(cx, u, 2, e);
    }
    // 3.
    if game.frame > e {
        cx.world.kill(game, u, None);
        return;
    }
    // 4.
    if p.combat && roll(cx, u, 100) < cx.aip(p, 1) {
        a1(game, cx, u, p.target);
        return;
    }
    // 5.
    idle(game, cx, u, cx.aip(p, 2));
}

// ---- ElementalBeast -----------------------------------------------------

/// ElementalBeast (46).
pub fn elemental_beast<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let s = param(cx, u, 0);
    // 1.
    if s == 2 {
        cx.world.kill(game, u, t);
        return;
    }
    // 2.
    if s == 0 && (p.distance < cx.aip(p, 2) || cx.ai_state_set(u)) {
        mode_at(game, cx, u, mode::SKILL1, t);
        set_param(cx, u, 0, 1);
        return;
    }
    // 3.
    if s != 1 {
        idle(game, cx, u, cx.aip(p, 3));
        return;
    }
    // 4.
    if p.combat {
        mode_at(game, cx, u, mode::SKILL1, t);
        modechange_at(game, u, 1);
        set_param(cx, u, 0, 2);
        return;
    }
    // 5.
    if pct(cx, u) < cx.aip(p, 1) {
        walk_to(game, cx, u, t, 0);
    } else {
        wander(game, cx, u, 8);
    }
}

// ---- NpcStationary ------------------------------------------------------

/// tyrael1, izualghost.
const TYRAEL1: i32 = 251;
const IZUALGHOST: i32 = 406;

/// "Free": an empty interaction list → death, true.
fn npc_free<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) -> bool {
    if cx.world.interacting(u) {
        return false;
    }
    death(game, cx, u);
    true
}

/// NpcStationary (54). No draws.
pub fn npc_stationary<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    // 1.
    let (pl, _) = cx.world.nearest_player(game, u);
    // 2.
    if pl == u {
        let hooks = match cx.world.class(u) {
            TYRAEL1 => Some((QuestHook::TyraelLeave, QuestHook::TyraelGone)),
            IZUALGHOST => Some((QuestHook::IzualGhostLeave, QuestHook::IzualGhostGone)),
            _ => None,
        };
        if let Some((may, gone)) = hooks {
            if cx.world.quest_hook(game, u, None, may) && npc_free(game, cx, u) {
                death(game, cx, u);
                cx.world.quest_hook(game, u, None, gone);
                return;
            }
        }
        idle(game, cx, u, 20);
        return;
    }
    // 3.
    let d = unit_distance(cx, u, pl);
    let player = is_player(game, pl);
    if (player && cx.world.busy(pl)) || cx.world.interacting(u) {
        idle(game, cx, u, 10);
        return;
    }
    let mut g = param(cx, u, 1);
    // 4.
    if d >= 24 {
        if g <= 0 {
            g = 60;
        }
        set_param(cx, u, 1, g - 1);
        idle(game, cx, u, 20);
        return;
    }
    // 5.
    if g != 0 {
        set_param(cx, u, 1, g - 1);
    } else {
        set_param(cx, u, 1, 60);
        if player {
            cx.world.play_sound(game, u, 18, Some(pl));
        }
    }
    idle(game, cx, u, 20);
}

// ---- MosquitoNest -------------------------------------------------------

/// suckernest1, the footprint class.
const SUCKERNEST1: i32 = 334;

/// MosquitoNest (83). No draws.
pub fn mosquito_nest<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    // 1.
    if p.distance > cx.aip(p, 2) {
        idle(game, cx, u, 25);
        return;
    }
    // 2.
    let n = param(cx, u, 1);
    if n > cx.aip(p, 1) {
        no_drop_death(game, cx, u);
        return;
    }
    // 3.
    let class = fixed_class(cx, SUCKERNEST1);
    let room = room_of(game, u);
    let (x, y) = cx.world.position(u);
    if cx.skill(p, 1).0 >= 0
        && game.frame > param(cx, u, 0)
        && cx.world.footprint_ok(game, class, room, x, y)
    {
        set_param(cx, u, 1, n.wrapping_add(1));
        set_param(cx, u, 0, game.after(cx.aip(p, 3)));
        skill_k(game, cx, u, p, 1, p.target);
        return;
    }
    // 4.
    idle(game, cx, u, 25);
}

// ---- DesertTurret -------------------------------------------------------

/// Aim offsets V.
const TURRET_V: [(i32, i32); 8] = [
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
    (1, 0),
];

/// Aim turns J[e][j].
const TURRET_J: [[i32; 8]; 8] = [
    [0, 0, 1, 2, 5, 6, 7, 0],
    [1, 1, 1, 2, 3, 6, 7, 0],
    [1, 2, 2, 2, 3, 4, 7, 0],
    [1, 2, 3, 3, 3, 4, 5, 6],
    [1, 2, 3, 4, 4, 4, 5, 6],
    [7, 2, 3, 4, 5, 5, 5, 6],
    [7, 0, 3, 4, 5, 6, 6, 6],
    [7, 0, 1, 4, 5, 6, 7, 7],
];

/// DesertTurret (94). No draws.
pub fn desert_turret<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let frame = game.frame;
    let (s1, m1) = cx.skill(p, 1);
    let f = param(cx, u, 0);
    // 1.
    if s1 >= 0 && f == 0 {
        use_skill(game, cx, u, m1, s1, ModeTarget::Point(0, 0));
        set_param(cx, u, 2, 0);
        set_param(cx, u, 0, frame);
        return;
    }
    // 2.
    if frame < f {
        idle(game, cx, u, 10);
        return;
    }
    let n = param(cx, u, 1);
    // 3.
    if p.distance > cx.aip(p, 4) {
        if n > 0 {
            set_param(cx, u, 1, n - 1);
        }
        idle(game, cx, u, 15);
        return;
    }
    // 4. T = 0 is unreachable (target mode 1: no mode-1 or mode-4 body
    // has target 0): asserted, not handled.
    let t = p
        .target
        .expect("DesertTurret think without a target (ai.md §2.3)");
    let e = dir8(cx.world.direction64(u, t));
    let j = TURRET_J[e][param(cx, u, 2).rem_euclid(8) as usize];
    set_param(cx, u, 2, j);
    let a5 = cx.aip(p, 5);
    let (ox, oy) = cx.world.position(u);
    let (vx, vy) = TURRET_V[j as usize];
    let (qx, qy) = (ox.wrapping_add(a5 * vx), oy.wrapping_add(a5 * vy));
    cx.world.set_path_target(u, t);
    // 5. The second check is at T's own position.
    let (tx, ty) = cx.world.position(t);
    if s1 >= 0
        && cx.world.skill_check(game, u, s1, p.target, qx, qy)
        && cx.world.skill_check(game, u, s1, p.target, tx, ty)
    {
        use_skill(game, cx, u, m1, s1, ModeTarget::Point(qx, qy));
        cx.world.set_path_target_point(u, qx, qy);
        let n = n.wrapping_add(1);
        if n > cx.aip(p, 2) {
            set_param(cx, u, 0, game.after(cx.aip(p, 3)));
            set_param(cx, u, 1, 0);
        } else {
            set_param(cx, u, 1, n);
            set_param(cx, u, 0, game.after(cx.aip(p, 1)));
        }
        return;
    }
    // 6.
    if n > 0 {
        set_param(cx, u, 1, n - 1);
    }
    idle(game, cx, u, 10);
}

// ---- AssassinSentry -----------------------------------------------------

/// The sentry init (AssassinSentry, DeathSentry): param 0
/// := frame, param 1 := −1.
pub fn sentry_init<W: AiHost + ?Sized>(game: &Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    set_param(cx, u, 0, game.frame);
    set_param(cx, u, 1, -1);
}

/// Charges `(game, unit, tick, use)` → true when the trap was
/// removed.
pub(super) fn sentry_charges<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
    spend: bool,
) -> bool {
    // 1.
    let owner_ok = minion_owner(game, cx, u)
        .and_then(|o| room_of(game, o))
        .is_some_and(|r| !cx.world.in_town(game, r));
    if !owner_ok {
        death(game, cx, u);
        return true;
    }
    // 2.
    let mut c = param(cx, u, 1);
    if c < 0 {
        let (s1, _) = cx.skill(p, 1);
        let Some(row) = skill_row(cx, s1) else {
            death(game, cx, u);
            return true;
        };
        let calc = row.calc4;
        if cx.world.skill_entry(u, s1).is_none() {
            death(game, cx, u);
            return true;
        }
        let lvl = cx.world.skill_level(u, s1, true).unwrap_or(0);
        c = cx.world.skill_calc(game, u, s1, calc, lvl);
        set_param(cx, u, 1, c);
    }
    // 3.
    if c > 0 {
        if spend {
            set_param(cx, u, 1, c - 1);
        }
        return false;
    }
    // 4.
    death(game, cx, u);
    true
}

/// AssassinSentry (101).
pub fn assassin_sentry<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    // 1.
    if sentry_charges(game, cx, u, p, false) {
        return;
    }
    // 2.
    let (s1, _) = cx.skill(p, 1);
    let Some((_, em)) = cx.world.skill_entry(u, s1) else {
        death(game, cx, u);
        return;
    };
    // 3.
    if cx.world.has_state(u, STATE_12) {
        cx.world.set_state(game, u, STATE_12, false);
    }
    // 4.
    let (s, e2, _) = cx.world.secondary_target(game, u, cx.store);
    let Some(s) = s.filter(|_| e2 < cx.aip(p, 4)) else {
        idle(game, cx, u, cx.aip(p, 3));
        return;
    };
    // 5.
    if roll(cx, u, 100) >= cx.aip(p, 1) {
        idle(game, cx, u, cx.aip(p, 2));
        return;
    }
    // 6.
    if sentry_charges(game, cx, u, p, true) {
        return;
    }
    // 7.
    delete_thinks(game, u);
    use_skill(game, cx, u, em, s1, ModeTarget::Unit(s));
}

// ---- Catapult, CatapultSpotter --------------------------------------

/// Catapult (123).
pub fn catapult<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    if pct(cx, u) < cx.aip(p, 1) {
        mode_at(game, cx, u, mode::ATTACK1, None);
    } else {
        idle(game, cx, u, 15);
    }
}

/// The spotter's skills (first 5 of 8).
const SPOTTER_SKILLS: [i32; 5] = [287, 288, 303, 304, 305];
/// A table by chain position (negated in the check).
const SPOTTER_DIR: [(i32, i32); 4] = [(0, 1), (1, 0), (1, 0), (-1, 0)];

/// Catapult check → true when the spotter's catapult is dead.
fn catapult_dead<W: AiHost + ?Sized>(game: &Game, cx: &Ctx<'_, W>, u: UnitId) -> bool {
    // A thinking monster has a room (none is a fatal error).
    let Some(r) = room_of(game, u) else {
        return false;
    };
    let Some((mut rx, mut ry, w, h)) = cx.world.room_box(game, r) else {
        return false;
    };
    let class = cx.world.class(u);
    let pos = cx.world.chain_index(class);
    assert!(
        (0..4).contains(&pos),
        "catapult spotter chain position outside 0..3 (ai-bodies-6.md §16)"
    );
    let (dx, dy) = SPOTTER_DIR[pos as usize];
    let (dx, dy) = (-dx, -dy);
    let c = class - 19;
    for _ in 1..=3 {
        rx = rx.wrapping_add(w.wrapping_mul(dx));
        ry = ry.wrapping_add(h.wrapping_mul(dy));
        if cx.world.class_dead_at(game, r, rx, ry, c) {
            return true;
        }
    }
    false
}

/// CatapultSpotter (126).
pub fn catapult_spotter<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let mut f = param(cx, u, 2);
    // 1.
    if (f == 0 || pct(cx, u) <= 2) && catapult_dead(game, cx, u) {
        mode_at(game, cx, u, mode::DEATH, t);
        return;
    }
    // 2.
    if f == 0 {
        f = 1;
        set_param(cx, u, 2, 1);
    }
    let aip2 = cx.aip(p, 2);
    // 3.
    if game.frame.wrapping_sub(f) < game.ticks(aip2) {
        idle(game, cx, u, aip2);
        return;
    }
    // 4.
    if t.is_none() || pct(cx, u) >= cx.aip(p, 1) {
        idle(game, cx, u, aip2);
        return;
    }
    // 5.
    let v = param(cx, u, 1);
    if v < 1 {
        let a = roll(cx, u, 5);
        set_param(cx, u, 0, a);
        set_param(cx, u, 1, cx.aip(p, 5));
    } else {
        set_param(cx, u, 1, v - 1);
    }
    // 6.
    let a4 = cx.aip(p, 4);
    let (ox, oy) = cx.world.position(u);
    let x = ox.wrapping_add(roll(cx, u, 2 * a4)).wrapping_sub(a4);
    let y = oy.wrapping_add(roll(cx, u, 2 * a4)).wrapping_sub(a4);
    // 7.
    let room = room_of(game, u);
    let Some((x, y)) = cx.world.free_point_masked(game, room, x, y, 2, 0x805, 3) else {
        idle(game, cx, u, 15);
        return;
    };
    // 8.
    let a = param(cx, u, 0);
    let skill = SPOTTER_SKILLS[a.rem_euclid(5) as usize];
    use_skill(game, cx, u, mode::ATTACK1, skill, ModeTarget::Point(x, y));
    set_param(cx, u, 2, game.frame);
}

// ---- Tentacle, TentacleHead -----------------------------------------

/// The submerge of Tentacle and TentacleHead: `Skill1` at T; wait `w`;
/// t := frame + aip3 × 25; s := 1.
fn submerge<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
    w: i32,
) {
    skill_k(game, cx, u, p, 1, p.target);
    wait(game, cx, u, w);
    set_param(cx, u, 1, game.after(cx.aip(p, 3).wrapping_mul(25)));
    set_param(cx, u, 2, 1);
}

/// The emerge: `Skill2` at the unit itself; t := frame + aip4 × 25; s := 2.
fn emerge<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    skill_k(game, cx, u, p, 2, Some(u));
    set_param(cx, u, 1, game.after(cx.aip(p, 4).wrapping_mul(25)));
    set_param(cx, u, 2, 2);
}

/// Tentacle (56).
pub fn tentacle<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let frame = game.frame;
    // 1.
    let Some(o) = minion_owner(game, cx, u) else {
        cx.world.kill(game, u, None);
        return;
    };
    let om = cx.world.anim_mode(o);
    // 2.
    if om == mode::DEAD && pct(cx, u) < 40 {
        let killer = cx.world.path_target(o);
        cx.world.kill(game, u, killer);
        return;
    }
    let (t, s) = (param(cx, u, 1), param(cx, u, 2));
    // 3.
    if cx.skill(p, 1).0 >= 0 {
        if s == 0 {
            submerge(game, cx, u, p, 8);
            return;
        }
        if s == 2
            && frame > t
            && (p.distance > cx.aip(p, 6)
                || (!p.combat && pct(cx, u) < cx.aip(p, 2))
                || (om == mode::SEQUENCE && pct(cx, u) < 50))
        {
            submerge(game, cx, u, p, 8);
            return;
        }
    }
    // 4.
    let s2 = cx.skill(p, 2).0 >= 0;
    if !(s2 && s != 1) {
        if s2
            && s == 1
            && frame > t
            && (p.combat || p.distance < cx.aip(p, 6) || (om != mode::SEQUENCE && pct(cx, u) < 5))
        {
            emerge(game, cx, u, p);
            return;
        }
        // 5.
        if s == 1 {
            wait(game, cx, u, cx.aip(p, 5));
            return;
        }
    }
    // 6.
    if p.combat && pct(cx, u) < cx.aip(p, 1) {
        a1(game, cx, u, p.target);
    } else {
        idle(game, cx, u, cx.aip(p, 5));
    }
}

/// TentacleHead (57).
pub fn tentacle_head<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let frame = game.frame;
    let (t, s) = (param(cx, u, 1), param(cx, u, 2));
    // 1.
    if cx.skill(p, 1).0 >= 0 {
        if s == 0 {
            submerge(game, cx, u, p, 8);
            return;
        }
        if s == 2
            && frame > t
            && (p.distance > cx.aip(p, 6) || (!p.combat && pct(cx, u) < cx.aip(p, 2)))
        {
            submerge(game, cx, u, p, 20);
            return;
        }
    }
    // 2.
    let s2 = cx.skill(p, 2).0 >= 0;
    if !(s2 && s != 1) {
        if s2 && s == 1 && frame > t && (p.combat || p.distance < cx.aip(p, 6)) {
            emerge(game, cx, u, p);
            return;
        }
        // 3.
        if s == 1 {
            wait(game, cx, u, cx.aip(p, 5));
            return;
        }
    }
    // 4.
    let (st, _, _) = cx.world.secondary_target(game, u, cx.store);
    if pct(cx, u) < cx.aip(p, 1) {
        mode_at(game, cx, u, mode::ATTACK1, st);
    } else {
        idle(game, cx, u, cx.aip(p, 5));
    }
}

// ---- Hydra, Totem, Vendor ---------------------------------------

/// Hydra (86).
pub fn hydra<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    // 1.
    if game.frame > param(cx, u, 0) {
        death(game, cx, u);
        return;
    }
    // 2.
    if p.target.is_some() && p.distance < 25 && pct(cx, u) < 60 {
        skill_k(game, cx, u, p, 1, p.target);
        return;
    }
    // 3.
    idle(game, cx, u, 10);
}

/// Totem (109).
pub fn totem<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    // 1.
    let Some(o) = minion_owner(game, cx, u) else {
        idle(game, cx, u, 10);
        return;
    };
    // 2.
    let (mut s, _, mut m) = search_capped(game, cx, u, 24);
    if let Some(st) = s.filter(|_| m) {
        if pct(cx, u) < cx.aip(p, 1) && escape(game, cx, u, Some(st), 6, true, false) {
            return;
        }
    }
    // 3.
    if pct(cx, u) < cx.aip(p, 2) {
        s = None;
        m = false;
    }
    // 4.
    let d = reach_distance(cx, u, o);
    if d > cx.aip(p, 3) {
        let (ox, oy) = cx.world.position(o);
        let room = room_of(game, o);
        if cx.world.place_unit(game, u, room, ox, oy) {
            idle(game, cx, u, 25);
            return;
        }
    }
    // 5.
    if d > cx.aip(p, 4) {
        match cx.world.anim_mode(o) {
            2 | 6 if pet_move(game, cx, Mover::Pet, o, u, 0, false, 0, 0) => return,
            3 if pet_move(game, cx, Mover::Pet, o, u, 0, false, 60, 0) => return,
            _ => {}
        }
    }
    // 6.
    if pet_follow(game, cx, u, s, o, m, false, 6) {
        return;
    }
    // 7.
    idle(game, cx, u, 25);
}

/// Vendor (42).
pub fn vendor<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    if pct(cx, u) < 20 {
        mode_point_raw(game, cx, u, mode::SKILL1, 0, 0);
    } else {
        idle(game, cx, u, 30);
    }
}

// ---- Trap-Missile, TrappedSoul --------------------------------------

/// Trap-Missile (77). No draws.
pub fn trap_missile<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let n = param(cx, u, 0);
    // 1.
    if p.target.is_some() && p.distance <= cx.aip(p, 1) && n < cx.aip(p, 2) {
        if param(cx, u, 1) == 0 {
            a1(game, cx, u, p.target);
            set_param(cx, u, 0, n.wrapping_add(1));
            set_param(cx, u, 1, 1);
        } else {
            set_param(cx, u, 1, 0);
            idle(game, cx, u, cx.aip(p, 3));
        }
        return;
    }
    // 2.
    no_drop_death(game, cx, u);
}

/// TrappedSoul (99). No draws.
pub fn trapped_soul<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let frame = game.frame;
    let t = p.target;
    // 1.
    cx.world.set_unit_flag(u, FLAG_NO_DROP);
    // 2.
    let Some(tt) = t.filter(|_| p.distance <= 4) else {
        if param(cx, u, 0) != 0 {
            mode_at(game, cx, u, mode::SKILL1, t);
        } else {
            idle(game, cx, u, 15);
        }
        return;
    };
    // 3.
    if param(cx, u, 0) == 0 {
        set_param(cx, u, 0, 1);
        set_param(cx, u, 1, frame);
        mode_at(game, cx, u, mode::SKILL2, t);
        return;
    }
    // 4.
    if !p.combat || frame <= param(cx, u, 1) {
        mode_at(game, cx, u, mode::SKILL1, t);
        return;
    }
    // 5.–7. Compared unsigned.
    let (xu, yu) = cx.world.position(u);
    let (xt, yt) = cx.world.position(tt);
    let (xu, yu, xt, yt) = (xu as u32, yu as u32, xt as u32, yt as u32);
    if xt <= xu && yt >= yu {
        a1(game, cx, u, t);
        set_param(cx, u, 1, game.after(35));
    } else if xt >= xu && yt <= yu {
        a2(game, cx, u, t);
        set_param(cx, u, 1, game.after(35));
    } else {
        mode_at(game, cx, u, mode::SKILL1, t);
        set_param(cx, u, 1, game.after(5));
    }
}

// ---- DruidWolf ----------------------------------------------------------

/// spiritwolf.
const SPIRITWOLF: i32 = 420;
/// The fenris rage state.
const STATE_138: u16 = 138;

/// "Port": the teleport skill in its mode, no target, at (x, y).
fn port<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    sk: (i32, u8),
    at: (i32, i32),
) {
    use_skill(game, cx, u, sk.1, sk.0, ModeTarget::Point(at.0, at.1));
}

/// DruidWolf (108).
pub fn druid_wolf<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let wolf = cx.world.class(u) == SPIRITWOLF;
    let porter = cx.skill(p, if wolf { 1 } else { 2 });
    // 1.
    let Some(o) = minion_owner(game, cx, u) else {
        let g = param(cx, u, 2);
        match player_by_guid(game, g).filter(|_| g != 0 && porter.0 >= 0) {
            Some(pl) => {
                let at = cx.world.position(pl);
                port(game, cx, u, porter, at);
                modechange_at(game, u, 6);
                wait(game, cx, u, 8);
            }
            None => idle(game, cx, u, 10),
        }
        return;
    };
    // 2.
    set_param(cx, u, 2, guid_of(game, Some(o)));
    // 3.
    if in_town(game, cx, u) {
        if !pet_follow(game, cx, u, None, o, false, false, 6) {
            idle(game, cx, u, 33);
        }
        return;
    }
    // 4.
    let r = cx.aip(p, 4);
    let (s, e, m) = search_capped(game, cx, u, r);
    let (t0, e0) = owner_view(game, cx, u, o);
    let v = run_speed(cx, p);
    if wolf {
        spirit_wolf(game, cx, u, p, o, (s, m), t0, porter, v);
    } else {
        fenris(game, cx, u, p, o, (s, e, m), (t0, e0), porter, v);
    }
}

/// The spirit wolf.
#[allow(clippy::too_many_arguments)]
fn spirit_wolf<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
    o: UnitId,
    (mut s, m): (Option<UnitId>, bool),
    t0: Option<UnitId>,
    porter: (i32, u8),
    v: i32,
) {
    let r = cx.aip(p, 4);
    // 5. Bug kept: the unreachable T0 is the one taken.
    s = s.filter(|&s| cx.world.can_reach_directly(game, u, s));
    if s.is_none() {
        s = t0.filter(|&t| unit_distance(cx, u, t) < r && !cx.world.can_reach_directly(game, u, t));
    }
    // 6.
    let d = reach_distance(cx, o, u);
    if porter.0 >= 0 && d > 50 {
        let at = cx.world.position(o);
        port(game, cx, u, porter, at);
        modechange_at(game, u, 4);
        wait(game, cx, u, 10);
        return;
    }
    // 7.
    if d > cx.aip(p, 5) && pet_move(game, cx, Mover::Pet, o, u, 0, true, 100, 0) {
        return;
    }
    if d > cx.aip(p, 3) {
        let started = match cx.world.anim_mode(o) {
            2 | 6 => pet_move(game, cx, Mover::Pet, o, u, 0, false, 0, 0),
            3 => pet_move(game, cx, Mover::Pet, o, u, 0, true, 100, 0),
            _ => false,
        };
        if started {
            return;
        }
    }
    // The pet follow is evaluated whatever d is (d ≤ aip3
    // and every not-started pet move fall through to it).
    if pet_follow(game, cx, u, s, o, m, true, 6) {
        return;
    }
    // 8.
    if let Some(st) = s {
        if m {
            a1(game, cx, u, Some(st));
            wait(game, cx, u, cx.aip(p, 1));
        } else if reach_distance(cx, o, st) < r {
            set_velocity(cx, u, 0, v, 0);
            run_to(game, cx, u, Some(st), 0);
        } else if d > 10 {
            walk_in_radius(game, cx, u, o, 8, 6);
        } else {
            idle(game, cx, u, 15);
        }
        return;
    }
    // 9.
    if roll(cx, u, 100) < cx.aip(p, 2) {
        wander(game, cx, u, 10);
    } else {
        idle(game, cx, u, 15);
    }
}

/// Fenris.
#[allow(clippy::too_many_arguments)]
fn fenris<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
    o: UnitId,
    (mut s, _e, m): (Option<UnitId>, i32, bool),
    (t0, e0): (Option<UnitId>, i32),
    porter: (i32, u8),
    v: i32,
) {
    let r = cx.aip(p, 4);
    // 5.
    s = s.filter(|&s| cx.world.can_reach_directly(game, u, s));
    if s.is_none() {
        s = t0.filter(|&t| e0 < r && cx.world.can_reach_directly(game, u, t));
    }
    // 6.
    let d = reach_distance(cx, o, u);
    if s.is_some_and(|st| d > r && reach_distance(cx, o, st) > r) {
        s = None;
    }
    // 7.
    if porter.0 >= 0 && d > 50 {
        let at = cx.world.position(o);
        port(game, cx, u, porter, at);
        modechange_at(game, u, 2);
        wait(game, cx, u, 10);
        return;
    }
    // 8.
    if d > cx.aip(p, 5) {
        pet_move(game, cx, Mover::Pet, o, u, 0, true, 100, 0);
        return;
    }
    if d > r {
        match cx.world.anim_mode(o) {
            3 => {
                pet_move(game, cx, Mover::Pet, o, u, 0, true, 100, 0);
                return;
            }
            2 | 6 => {
                pet_move(game, cx, Mover::Pet, o, u, 0, false, 0, 0);
                return;
            }
            _ => {}
        }
    }
    // 9.
    if pet_follow(game, cx, u, s, o, m, true, 6) {
        return;
    }
    // 10.
    let (s1, m1) = cx.skill(p, 1);
    let no_rage = s1 < 0
        || cx.world.has_state(u, STATE_138)
        || (roll(cx, u, 100) >= cx.aip(p, 3) && s.is_some() && param(cx, u, 0) == 0);
    if no_rage {
        set_param(cx, u, 0, 0);
    } else if let Some(k) = cx
        .world
        .corpse_find(game, u, 10)
        .filter(|&k| reach_distance(cx, k, u) < r / 2)
    {
        if cx.world.in_melee_range(game, u, k) {
            set_param(cx, u, 0, 0);
            use_skill(game, cx, u, m1, s1, ModeTarget::Unit(k));
            return;
        }
        if !m {
            run_to(game, cx, u, Some(k), 0);
            set_param(cx, u, 0, 1);
            set_param(cx, u, 1, guid_of(game, Some(k)));
            return;
        }
        // M ≠ 0: step 11's A1 branch.
        a1(game, cx, u, s);
        wait(game, cx, u, cx.aip(p, 1));
        return;
    }
    // 11.
    if m {
        a1(game, cx, u, s);
        wait(game, cx, u, cx.aip(p, 1));
        return;
    }
    if let Some(st) = s {
        set_velocity(cx, u, 0, v, 0);
        run_to(game, cx, u, Some(st), 0);
        return;
    }
    if roll(cx, u, 100) < cx.aip(p, 2) {
        wander(game, cx, u, 10);
    } else {
        idle(game, cx, u, 15);
    }
}

// ---- CycleOfLife --------------------------------------------------------

/// The init (CycleOfLife, Vines): AI param 0 := 0.
pub fn vine_init<W: AiHost + ?Sized>(cx: &mut Ctx<'_, W>, u: UnitId) {
    set_param(cx, u, 0, 0);
}

/// cycleoflife, vinecreature.
const CYCLEOFLIFE: i32 = 426;
const VINECREATURE: i32 = 427;

/// CycleOfLife (111).
pub fn cycle_of_life<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    // 1. Nothing scheduled.
    let Some(o) = minion_owner(game, cx, u) else {
        return;
    };
    // 2.
    let d = reach_distance(cx, u, o);
    if d >= cx.aip(p, 5) && pet_move(game, cx, Mover::Pet, o, u, 3, false, 0, 6) {
        return;
    }
    // 3.
    let (mut t, mut c) = (p.target, p.combat);
    if t.is_some_and(|t| cx.world.is_dead(t)) {
        t = None;
        c = false;
    }
    // 4.
    let (s1, _) = cx.skill(p, 1);
    let mut k = None;
    let mut dk = 0;
    if s1 > 0 {
        if let Some(row) = skill_row(cx, s1) {
            // E := the highest entry, L := its
            // level with bonus 1.
            if let Some(lvl) = cx.world.skill_level(u, s1, true) {
                let n = cx
                    .world
                    .skill_calc(game, u, s1, row.aurarangecalc, lvl)
                    .clamp(5, 50);
                k = cx.world.corpse_find(game, u, n);
                if let Some(kk) = k {
                    dk = reach_distance(cx, u, kk);
                }
            }
        }
    }
    // 5.
    let mut mk = false;
    if dk < cx.aip(p, 2) {
        if let Some(kk) = k {
            mk = cx.world.in_melee_range(game, u, kk);
        }
    } else {
        k = None;
    }
    // 6.
    if pet_follow(game, cx, u, k, o, mk, false, 6) {
        return;
    }
    // 7.
    let need = match cx.world.class(u) {
        CYCLEOFLIFE => life(cx, o) < cx.world.max_life(o),
        VINECREATURE => cx.world.stat(o, 8) < cx.world.max_mana(o),
        _ => true,
    };
    // 8.
    let f = param(cx, u, 1);
    if let Some(kk) =
        k.filter(|_| mk && need && game.frame > f.wrapping_add(game.ticks(cx.aip(p, 1))))
    {
        use_skill(game, cx, u, mode::SKILL1, s1, ModeTarget::Unit(kk));
        set_param(cx, u, 1, game.frame);
        return;
    }
    // 9.
    if c && t.is_some() && roll(cx, u, 100) < 25 {
        let n = i32::from(cx.aip(p, 4) as u8);
        escape(game, cx, u, t, n, false, false);
        return;
    }
    // 10., 11.
    match k {
        None => idle(game, cx, u, cx.aip(p, 3)),
        Some(_) => {
            walk_to(game, cx, u, k, 7);
        }
    }
}

// ---- NpcBarb ------------------------------------------------------------

/// The target-node slot "none" (unit +0xD0) and the good-NPC slot.
pub(super) const SLOT_NONE: i32 = 11;
pub(super) const SLOT_GOOD_NPC: i32 = 8;

/// NpcBarb's init: returns at once.
pub fn npc_barb_init() {}

/// NpcBarb (127).
pub fn npc_barb<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    // 1.
    if cx.world.target_slot(u) == SLOT_NONE {
        cx.world.register_target_node(game, u, SLOT_GOOD_NPC);
    }
    // 2.
    if t.is_some() {
        if p.combat {
            a1(game, cx, u, t);
            wait(game, cx, u, cx.aip(p, 1));
            return;
        }
        if p.distance < cx.aip(p, 3) && pct(cx, u) < cx.aip(p, 2) {
            set_velocity(cx, u, 0, 100, 0);
            run_to(game, cx, u, t, 0);
            return;
        }
    }
    // 3. `%` unsigned on each step's low word.
    let (ox, oy) = cx.world.position(u);
    let step = |cx: &mut Ctx<'_, W>| (cx.world.seed(u).step() % 20) as i32;
    let (a, b) = (step(cx), step(cx));
    if walk_point_del(game, cx, u, ox + a - 40, oy + b - 10) {
        return;
    }
    let c = cx.world.seed(u).step();
    let sigma = if c & 1 == 1 { 1 } else { -1 };
    let (d, e) = (step(cx), step(cx));
    if walk_point_del(game, cx, u, ox + d - 10, oy + e - 10 - 30 * sigma) {
        return;
    }
    let (f, g) = (step(cx), step(cx));
    if walk_point_del(game, cx, u, ox + f - 10, oy + g - 10 + 30 * sigma) {
        return;
    }
    idle(game, cx, u, 15);
}

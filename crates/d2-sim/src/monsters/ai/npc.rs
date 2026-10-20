//! Town and quest NPC thinks: Npc (32), GoodNpcRanged (60, also special
//! state 5), NpcOutOfTown (31, which calls the Npc interaction handler).

use crate::game::Game;
use crate::units::{UnitId, UnitType};
use crate::world::quests::act2::q4::JerhynStep;

use super::tactics::*;
use super::{idle, main_search, mode, request_mode, AiHost, Ctx, ModeTarget, PortalNpc, TickParam};

/// Command types of the AI command list used here.
mod cmd {
    /// Walk to (x, y, tries, delay).
    pub const WALK_TO: i32 = 4;
    /// NpcOutOfTown portal state (x, y, phase, tries).
    pub const PORTAL: i32 = 3;
    /// Home position (x, y).
    pub const HOME: i32 = 10;
    /// Wander (count, distance, idles, delay).
    pub const WANDER: i32 = 5;
    /// Mode action (mode, x, y, tries).
    pub const MODE_ACTION: i32 = 7;
}

/// Monstats rows named by the bodies.
mod class {
    /// roguehire.
    pub const ROGUEHIRE: i32 = 271;
    /// drehyaiced (the rebuilt catalogue's `monstats_rows` of
    /// index 31 lists `527 drehyaiced` too).
    pub const DREHYAICED: i32 = 527;
}

fn param<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, u: UnitId, n: usize) -> i32 {
    cx.store.control(u).map_or(0, |c| c.params[n])
}

fn set_param<W: AiHost + ?Sized>(cx: &mut Ctx<'_, W>, u: UnitId, n: usize, v: i32) {
    if let Some(c) = cx.store.control_mut(u) {
        c.params[n] = v;
    }
}

/// The command's params, or zeros when it is gone.
fn params<W: AiHost + ?Sized>(cx: &mut Ctx<'_, W>, u: UnitId, k: usize) -> [i32; 5] {
    command_mut(cx, u, k).map_or([0; 5], |c| c.params)
}

fn set_cmd<W: AiHost + ?Sized>(cx: &mut Ctx<'_, W>, u: UnitId, k: usize, n: usize, v: i32) {
    if let Some(c) = command_mut(cx, u, k) {
        c.params[n] = v;
    }
}

/// GoodNpcRanged (also special state 5).
pub fn good_npc_ranged<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    // 1.
    if cx.world.anim_mode(u) != mode::NEUTRAL {
        idle(game, cx, u, 5);
        return;
    }
    // 2.
    // A unit with no room counts as out of town (the town test of no
    // room returns 0).
    let room = game.lists.unit(u).and_then(|e| e.room());
    let in_town = room.is_some_and(|r| cx.world.in_town(game, r));
    if !in_town {
        // The "Else" belongs to the first 30 % roll: both rolls happen
        // only with S under 20; otherwise the think goes on to step 3.
        let (s, e, _) = cx.world.secondary_target(game, u, cx.store);
        if let Some(s) = s.filter(|_| e < 20) {
            if cx.world.seed(u).roll(100) < 30 {
                if cx.world.class(u) == class::ROGUEHIRE {
                    let (skill1, mode1) = cx.skill(p, 1);
                    use_skill(game, cx, u, mode1, skill1, ModeTarget::Unit(s));
                } else {
                    mode_at(game, cx, u, mode::ATTACK1, Some(s));
                }
            } else if cx.world.seed(u).roll(100) < 30 {
                circle(game, cx, u, Some(s), 4, false);
            } else {
                idle(game, cx, u, 10);
            }
            return;
        }
    }
    // 3.
    if cx.chance(u, 20) {
        wander(game, cx, u, 5);
    } else {
        idle(game, cx, u, 10);
    }
}

/// SpecialState06 (the hirelings' state when their
/// owner is gone).
pub fn special_state_06<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    // 1.
    if cx.world.anim_mode(u) != mode::NEUTRAL {
        idle(game, cx, u, 5);
        return;
    }
    // 2. Out of town (a unit with no room counts as out of town): the
    // main search; the rolls happen only when it finds a target.
    let room = game.lists.unit(u).and_then(|e| e.room());
    let in_town = room.is_some_and(|r| cx.world.in_town(game, r));
    if !in_town {
        let s = main_search(game, cx, u);
        if let Some(t) = s.target {
            let r = cx.world.seed(u).roll(100);
            if !s.combat {
                if r < 30 {
                    walk_to(game, cx, u, Some(t), 7);
                } else {
                    idle(game, cx, u, 10);
                }
            } else if r < 80 {
                mode_at(game, cx, u, mode::ATTACK1, Some(t));
            } else {
                idle(game, cx, u, 10);
            }
            return;
        }
    }
    // 3.
    if cx.chance(u, 20) {
        wander(game, cx, u, 5);
    } else {
        idle(game, cx, u, 10);
    }
}

/// Step 1, the home step: H = command 10 (created if
/// absent); params 1, 2 both 0 → the unit's position, idle 20, true.
pub fn npc_home<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) -> bool {
    let Some(h) = get_or_create_command(cx, u, cmd::HOME, false) else {
        return false;
    };
    let hp = params(cx, u, h);
    if hp[1] != 0 || hp[2] != 0 {
        return false;
    }
    let (x, y) = cx.world.position(u);
    set_cmd(cx, u, h, 1, x);
    set_cmd(cx, u, h, 2, y);
    idle(game, cx, u, 20);
    true
}

/// Interaction step 7, the home check.
fn home_check<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, n: i32) -> bool {
    let Some(h) = find_command(cx, u, cmd::HOME, false) else {
        return npc_home(game, cx, u);
    };
    let hp = params(cx, u, h);
    // Path distances compare unsigned.
    if path_distance(cx, u, hp[1], hp[2]) as u32 > n as u32 {
        if let Some(k) = get_or_create_command(cx, u, cmd::WALK_TO, false) {
            for (i, v) in [(1, hp[1]), (2, hp[2]), (3, 12), (4, 10)] {
                set_cmd(cx, u, k, i, v);
            }
        }
        idle(game, cx, u, 10);
        return true;
    }
    false
}

/// The interaction handler (D2MOO `sub_6FCE5EE0`).
/// True = handled.
pub fn npc_interaction_think<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
) -> bool {
    // 1.
    let (pl, _) = cx.world.nearest_player(game, u);
    let d = unit_distance(cx, u, pl);
    // 2.
    let is_monster = game
        .lists
        .unit(u)
        .is_some_and(|e| e.ty == UnitType::Monster);
    if !is_monster || !cx.world.has_interaction_block(u) {
        return false;
    }
    // 3.
    let pl_is_player = game
        .lists
        .unit(pl)
        .is_some_and(|e| e.ty == UnitType::Player);
    let busy = cx.world.in_interaction_list(u, pl) || (pl_is_player && cx.world.busy(pl));
    let talking = cx.world.interacting(u);
    // 4.
    let p0 = param(cx, u, 0);
    if talking || busy || p0 > 0 {
        let (x, y) = (param(cx, u, 1), param(cx, u, 2));
        if path_distance(cx, u, x, y) as u32 > 2 && p0 > 36 {
            walk_to_point(game, cx, u, x, y);
        } else if p0 > 0 {
            cx.world.stop_path(u);
            idle(game, cx, u, 8);
        }
        set_param(cx, u, 0, if p0 < 0 { 0 } else { p0 - 1 });
        return true;
    }
    // 5. `nearest_player` returns the NPC itself when there is none.
    if pl == u {
        return false;
    }
    // 6.
    if !(3..=23).contains(&d) {
        cx.world.stop_path(u);
        let p1 = param(cx, u, 1);
        if p1 == 0 {
            set_param(cx, u, 1, 60);
            if pl_is_player {
                cx.world.play_sound(game, u, 18, Some(pl));
            }
        } else {
            set_param(cx, u, 1, if p1 < 0 { 0 } else { p1 - 1 });
        }
        idle(game, cx, u, 20);
        return true;
    }
    // 7.
    if !home_check(game, cx, u, 16) {
        set_velocity(cx, u, 1, 0, 0);
        let (a, b) = if d < 5 { (d - 2, 2) } else { (3, 2) };
        walk_in_radius(game, cx, u, pl, a, b);
    }
    true
}

/// "leave": spawn the town portal, life := 0, mode 12 (dead) at
/// the unit's own position.
fn leave<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, npc: PortalNpc) {
    cx.world.spawn_town_portal(game, u, npc);
    cx.world.set_life(u, 0);
    let (x, y) = cx.world.position(u);
    request_mode(game, cx, u, mode::DEAD, ModeTarget::Point(x, y));
}

/// Step 1, the portal setup.
fn portal_setup<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    npc: PortalNpc,
) -> bool {
    let Some(k) = get_or_create_command(cx, u, cmd::PORTAL, false) else {
        return false;
    };
    let kp = params(cx, u, k);
    if kp[1] != 0 || kp[2] != 0 {
        return false;
    }
    let (x, y) = cx.world.position(u);
    set_cmd(cx, u, k, 1, x.wrapping_add(3));
    set_cmd(cx, u, k, 2, y.wrapping_add(3));
    if !cx.world.portal_setup(game, u, npc) {
        // The mode 12 request does not end the function: the param writes
        // and idle 1 follow a leave too.
        leave(game, cx, u, npc);
    }
    set_cmd(cx, u, k, 3, 1);
    set_cmd(cx, u, k, 4, 0);
    idle(game, cx, u, 1);
    true
}

/// NpcOutOfTown (cain1, drehyaiced). No draws.
pub fn npc_out_of_town<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    _p: &TickParam,
) {
    // Class 527 (drehyaiced) takes the Act 5 quest 3 functions; every
    // other class (cain1, and any class given AI 31 by edited data) the
    // Act 1 quest 4 ones.
    let npc = if cx.world.class(u) == class::DREHYAICED {
        PortalNpc::Drehya
    } else {
        PortalNpc::Cain
    };
    // 1.
    if portal_setup(game, cx, u, npc) {
        return;
    }
    // 2.
    if npc == PortalNpc::Drehya {
        cx.world.drehya_update(game);
    }
    // 3.
    if cx.world.interacting(u) {
        idle(game, cx, u, 40);
        return;
    }
    // 4.
    let k = find_command(cx, u, cmd::PORTAL, false);
    if cx.world.anim_mode(u) == mode::DEAD {
        return;
    }
    // 5.
    npc_interaction_think(game, cx, u);
    // 6.
    let Some(k) = k else {
        portal_setup(game, cx, u, npc);
        idle(game, cx, u, 40);
        return;
    };
    let kp = params(cx, u, k);
    // 7.
    if kp[3] >= 2 {
        let phase = kp[3].wrapping_add(1);
        set_cmd(cx, u, k, 3, phase);
        match cx.world.portal_coords(game, u, npc) {
            Some((x, y)) => {
                if phase < 8 && path_distance(cx, u, x, y) != 0 {
                    walk_to_point(game, cx, u, x, y);
                } else {
                    leave(game, cx, u, npc);
                }
            }
            None => idle(game, cx, u, 20),
        }
        return;
    }
    // 8.
    if path_distance(cx, u, kp[1], kp[2]) as u32 > 1 && kp[4] <= 5 {
        set_cmd(cx, u, k, 4, kp[4].wrapping_add(1));
        if npc == PortalNpc::Drehya && cx.world.drehya_wait(game) {
            idle(game, cx, u, 20);
            set_cmd(cx, u, k, 4, 0);
        } else {
            walk_to_point(game, cx, u, kp[1], kp[2]);
        }
        return;
    }
    if kp[3] == 1 {
        if !cx.world.spawn_outside_portal(game, u, npc) {
            set_cmd(cx, u, k, 3, 1);
            set_cmd(cx, u, k, 4, 1);
            idle(game, cx, u, 20);
            return;
        }
        set_cmd(cx, u, k, 3, 2);
    }
    idle(game, cx, u, 20);
}

// ---- Npc --------------------------------------------------------

/// Npc class cases of the Npc think's step 2 (monstats rows).
mod npc_class {
    pub const JERHYN: i32 = 201;
    pub const ALKOR: i32 = 254;
    pub const ORMUS: i32 = 255;
    pub const CAIN5: i32 = 265;
    pub const DREHYA: i32 = 512;
    pub const CHARSI: i32 = 154;
    pub const WARRIV1: i32 = 155;
    pub const FARA: i32 = 178;
    pub const JAMELLA: i32 = 405;
    pub const LARZUK: i32 = 511;
}

/// Npc (32).
pub fn npc<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, _p: &TickParam) {
    // 1.
    if npc_home(game, cx, u) {
        return;
    }
    // 2.
    match cx.world.class(u) {
        npc_class::JERHYN => {
            if !cx.world.jerhyn_palace_active(game) {
                idle(game, cx, u, 40);
                return;
            }
            let (a, b) = jerhyn_npc_state(game, cx, u);
            if b != 0 {
                idle(game, cx, u, 20);
            }
            if a == 0 {
                return;
            }
            if cx.world.guard_moving(game, u) {
                if let Some(h) = get_or_create_command(cx, u, cmd::HOME, false) {
                    let (x, y) = cx.world.position(u);
                    set_cmd(cx, u, h, 1, x.wrapping_add(9));
                    set_cmd(cx, u, h, 2, y);
                    idle(game, cx, u, 50);
                    return;
                }
            }
        }
        npc_class::ALKOR => {
            if cx.world.alkor_bird(game) {
                request_mode(game, cx, u, mode::SKILL1, ModeTarget::Point(0, 0));
                cx.world.alkor_reset(game);
                return;
            }
        }
        npc_class::ORMUS => {
            if let Some((x, y)) = cx.world.ormus_altar(game) {
                if path_distance(cx, u, x, y) as u32 > 3 {
                    walk_to_point(game, cx, u, x, y);
                } else {
                    request_mode(game, cx, u, mode::SKILL1, ModeTarget::Point(0, 0));
                    cx.world.ormus_set_altar_mode(game);
                }
                return;
            }
        }
        npc_class::CAIN5 => {
            if let Some((x, y)) = cx.world.cain_town_coords(game, u) {
                if path_distance(cx, u, x, y) as u32 > 2 {
                    walk_to_point(game, cx, u, x, y);
                    return;
                }
                cx.world.cain_in_town_activated(game, u);
            }
        }
        npc_class::DREHYA => cx.world.anya_open_portal(game, u),
        _ => {}
    }
    // 3.–5.
    if npc_interaction_think(game, cx, u) || npc_commands(game, cx, u) || npc_map_ai(game, cx, u) {
        return;
    }
    // 6.
    idle(game, cx, u, 8);
}

/// The
/// quest hook's outputs, with the walk or the placement
/// beside the harem blocker it makes on the unit. The placement's free
/// test is the free-point search with mask 0x3C01, sizes 1, 2, 3 in turn,
/// the first point found placed. No draw. "Sizes 1, 2, 3
/// in turn" is the first size whose search finds a point (checked
/// 2026-10-11: the three calls short-circuit); the free test is the
/// masked search seam.
fn jerhyn_npc_state<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
) -> (i32, i32) {
    match cx.world.jerhyn_npc_state(game, u) {
        JerhynStep::Out(a, b) => (a, b),
        JerhynStep::Walk(x, y) => {
            walk_to_point(game, cx, u, x, y);
            (0, 0)
        }
        JerhynStep::PlaceAt(x, y, room) => {
            let point = (1..=3).find_map(|size| {
                cx.world
                    .free_point_masked(game, Some(room), x, y, size, 0x3C01, 1)
            });
            if let Some((px, py)) = point {
                if cx.world.place_unit(game, u, Some(room), px, py) {
                    cx.world.jerhyn_placed(game);
                }
            }
            (0, 1)
        }
    }
}

/// The command handler. True = handled.
pub(super) fn npc_commands<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
) -> bool {
    // 1. Command 4: walk to (x, y), tries n, delay t.
    if let Some(k) = find_command(cx, u, cmd::WALK_TO, false) {
        let [_, x, y, n, t] = params(cx, u, k);
        if n > 0 {
            if x != 0 && y != 0 && path_distance(cx, u, x, y) as u32 > 3 {
                let g = cx.store.npc_walk_counter;
                set_velocity(cx, u, if g & 3 == 0 { 5 } else { 1 }, 0, 0);
                walk_step0(game, cx, u, x, y);
                cx.store.npc_walk_counter = g.wrapping_add(1);
            } else {
                idle(game, cx, u, t);
            }
            set_cmd(cx, u, k, 3, n - 1);
            return true;
        }
    }
    // 2. Command 5: wander count c, distance w, idles k, delay t.
    if let Some(k) = find_command(cx, u, cmd::WANDER, false) {
        let [_, c, w, idles, t] = params(cx, u, k);
        if c > 0 && c % 2 == 1 {
            wander(game, cx, u, i32::from(w as u8));
            set_cmd(cx, u, k, 1, c - 1);
            return true;
        }
        if c > 0 {
            if idles > 0 {
                idle(game, cx, u, t);
                set_cmd(cx, u, k, 1, c - 1);
                set_cmd(cx, u, k, 3, idles - 1);
                return true;
            }
            set_cmd(cx, u, k, 1, c - 1);
        } else if idles > 0 {
            idle(game, cx, u, t);
            set_cmd(cx, u, k, 3, idles - 1);
            return true;
        }
    }
    // 3. Command 7: mode m at (x, y), tries n.
    let Some(k) = find_command(cx, u, cmd::MODE_ACTION, false) else {
        return false;
    };
    let [_, mut m, x, y, n] = params(cx, u, k);
    if m == 0 {
        return false;
    }
    // 3.1.
    if !(8..=11).contains(&m) {
        set_cmd(cx, u, k, 1, 0);
        idle(game, cx, u, 50);
        return true;
    }
    // 3.2.
    let e = path_distance(cx, u, x, y);
    if e > 0 {
        if n > 0 {
            set_velocity(cx, u, 7, 0, 0);
            if !walk_step0(game, cx, u, x, y) {
                idle(game, cx, u, 25);
            }
            set_cmd(cx, u, k, 4, n - 1);
            return true;
        } else if e > 1 {
            m = 0;
        }
    }
    // 3.3.
    let class = cx.world.class(u);
    let facing = match class {
        npc_class::CHARSI => Some(56),
        npc_class::WARRIV1 => Some(52),
        npc_class::FARA => Some(4),
        npc_class::JAMELLA if m == 8 => Some(52),
        npc_class::JAMELLA if m == 9 => Some(48),
        _ => None,
    };
    if let Some(dir) = facing {
        cx.world.set_facing(u, dir);
    }
    if class == npc_class::LARZUK && cx.world.seed(u).roll(100) > 4 {
        set_cmd(cx, u, k, 1, 0);
        idle(game, cx, u, 50);
        return true;
    }
    // 3.4.
    if m != 0 {
        if i32::from(cx.world.anim_mode(u)) == m {
            set_cmd(cx, u, k, 1, m);
            idle(game, cx, u, 50);
            return true;
        }
        request_mode(game, cx, u, m as u8, ModeTarget::Unit(u));
        set_cmd(cx, u, k, 1, 0);
        return true;
    }
    // 3.5.
    if class == npc_class::FARA && cx.world.seed(u).roll(100) < 66 {
        m = 8;
    }
    set_cmd(cx, u, k, 1, m);
    false
}

/// Command 4 := (x, y, tries, delay), found or created.
fn set_walk_command<W: AiHost + ?Sized>(
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    x: i32,
    y: i32,
    tries: i32,
) {
    if let Some(k) = get_or_create_command(cx, u, cmd::WALK_TO, false) {
        for (i, v) in [(1, x), (2, y), (3, tries), (4, 10)] {
            set_cmd(cx, u, k, i, v);
        }
    }
}

/// Map action 1 (also 3).
fn map_walk<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    x: i32,
    y: i32,
) -> bool {
    if x == 0 || y == 0 || path_distance(cx, u, x, y) == 0 {
        return false;
    }
    set_velocity(cx, u, 7, 0, 0);
    walk_step0(game, cx, u, x, y);
    set_walk_command(cx, u, x, y, 12);
    true
}

/// The map AI. True = handled.
pub(super) fn npc_map_ai<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
) -> bool {
    // 1.
    let Some(nodes) = cx.store.control(u).and_then(|c| c.map_ai.clone()) else {
        return false;
    };
    // 2.
    if cx.world.seed(u).step() % 100 >= 66 {
        return false;
    }
    // 3.
    if nodes.is_empty() {
        return false;
    }
    let i = cx.world.seed(u).roll(nodes.len() as i32) as usize;
    let node = nodes[i];
    let (x, y) = (node.x, node.y);
    match node.action {
        1 | 3 => map_walk(game, cx, u, x, y),
        2 => {
            if x == 0 || y == 0 {
                return false;
            }
            let r = map_walk(game, cx, u, x, y);
            set_walk_command(cx, u, x, y, 20);
            r
        }
        a @ (4 | 5) => {
            if x == 0 || y == 0 {
                return false;
            }
            let r = map_walk(game, cx, u, x, y);
            let want = if a == 4 { mode::SKILL1 } else { mode::SKILL2 };
            let class = cx.world.class(u);
            let m = if cx.world.class_has_mode(class, want) {
                i32::from(want)
            } else {
                1
            };
            if let Some(k) = get_or_create_command(cx, u, cmd::MODE_ACTION, false) {
                for (j, v) in [(1, m), (2, x), (3, y), (4, 4)] {
                    set_cmd(cx, u, k, j, v);
                }
            }
            set_walk_command(cx, u, x, y, 12);
            r
        }
        _ => false,
    }
}

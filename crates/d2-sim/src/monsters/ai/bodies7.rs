//! The one-row AIs: traps, quest NPCs,
//! summons, spawners and the Ubers, with their init functions. The pet
//! helpers are in [`super::bodies6`]. Brackets in the
//! comments are the usual (Normal) values.

use crate::game::Game;
use crate::units::UnitId;

use super::bodies::{a1, nest_init, param, pct, roll, set_param};
use super::bodies4::{izual_steps, walk_point_del};
use super::bodies5::rider_scan;
use super::bodies6::{
    death, in_town, no_drop_death, owner_view, pet_follow, pet_move, player_by_guid, run_speed,
    search_capped, Mover, SLOT_GOOD_NPC, SLOT_NONE,
};
use super::common::*;
use super::npc::{npc_home, npc_interaction_think};
use super::tactics::*;
use super::{delete_thinks, idle, mode, AiHost, Ctx, ModeTarget, QuestHook, TickParam};

// ---- Sarcophagus ---------------------------------------------------------

/// sarcophagus, the footprint class.
const SARCOPHAGUS: i32 = 228;

/// Sarcophagus (45) (init, [`nest_init`]).
pub fn sarcophagus<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    // 1.
    if p.distance > 25 {
        idle(game, cx, u, 25);
        return;
    }
    // 2.
    let n = param(cx, u, 1);
    if n > cx.aip(p, 3) {
        no_drop_death(game, cx, u);
        return;
    }
    // 3.
    let class = fixed_class(cx, SARCOPHAGUS);
    let room = room_of(game, u);
    let (x, y) = cx.world.position(u);
    if cx.skill(p, 1).0 >= 0
        && game.frame.wrapping_sub(param(cx, u, 0)).wrapping_abs() >= game.ticks(cx.aip(p, 1))
        && cx.world.footprint_ok(game, class, room, x, y)
    {
        set_param(cx, u, 1, n.wrapping_add(1));
        set_param(cx, u, 0, game.frame);
        skill_k(game, cx, u, p, 1, p.target);
        return;
    }
    // 4.
    let n = (cx.world.seed(u).step() % 10) as i32 + 20;
    idle(game, cx, u, n);
}

// ---- FlyingScimitar ------------------------------------------------------

/// FlyingScimitar (47) (edge case 1: aip1 twice).
pub fn flying_scimitar<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    // 1.
    if !p.combat {
        if pct(cx, u) < cx.aip(p, 1) {
            radius(game, cx, u, t, 8, 1);
        } else if pct(cx, u) < cx.aip(p, 1) {
            walk_to(game, cx, u, t, 7);
        } else {
            idle(game, cx, u, cx.aip(p, 3));
        }
        return;
    }
    // 2.
    if pct(cx, u) < cx.aip(p, 2) {
        a1(game, cx, u, t);
    } else if pct(cx, u) < cx.aip(p, 4) {
        circle(game, cx, u, t, 2, false);
    } else {
        idle(game, cx, u, cx.aip(p, 3));
    }
}

// ---- GargoyleTrap --------------------------------------------------------

/// The facing table by quadrant q.
const GARGOYLE_FACING: [i32; 4] = [31, 49, 0, 17];

/// q by the 64-step direction e (a table).
fn gargoyle_quadrant(e: i32) -> usize {
    match e {
        9..=24 => 1,
        25..=40 => 2,
        41..=56 => 3,
        _ => 0,
    }
}

/// GargoyleTrap (63).
pub fn gargoyle_trap<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    // 1.
    let s = param(cx, u, 0);
    if s > 0 {
        idle(game, cx, u, s);
        set_param(cx, u, 0, 0);
        return;
    }
    // 2.
    if let Some(tt) = t {
        cx.world.set_path_target(u, tt);
    }
    // 3. T = 0 is unreachable (target mode 1): asserted.
    let tt = t.expect("GargoyleTrap think without a target (ai.md §2.3)");
    let (tx, ty) = cx.world.position(tt);
    let (ux, uy) = cx.world.position(u);
    let (dx, dy) = (tx.wrapping_sub(ux), ty.wrapping_sub(uy));
    let pt = if dx.wrapping_abs() < dy.wrapping_abs() {
        (ux, ty)
    } else {
        (tx, uy)
    };
    // 4.
    let e = cx.world.direction64_to(u, pt.0, pt.1);
    cx.world
        .snap_direction(u, GARGOYLE_FACING[gargoyle_quadrant(e)]);
    // 5.
    if dx.wrapping_abs() > 5 && dy.wrapping_abs() > 5 {
        idle(game, cx, u, cx.aip(p, 4));
        return;
    }
    // 6.
    if cx.skill(p, 1).0 >= 0 && p.distance < cx.aip(p, 1) && pct(cx, u) < cx.aip(p, 2) {
        skill_k(game, cx, u, p, 1, t);
        set_param(cx, u, 0, cx.aip(p, 3));
        return;
    }
    // 7.
    idle(game, cx, u, cx.aip(p, 4));
}

// ---- the arrow traps -----------------------------------------------------

/// Trap kind: cached per level; unset → Act I (level
/// id < 40) 0, else `roll(3)` on the trap's seed (edge case 4).
fn trap_kind<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) -> i32 {
    let k = cx.world.trap_kind(game, u);
    if k != -1 {
        return k;
    }
    let k = if cx.world.level_id(game, u) < 40 {
        0
    } else {
        roll(cx, u, 3)
    };
    cx.world.set_trap_kind(game, u, k);
    k
}

/// Trap-RightArrow (78) (`vertical` = false, the x axis)
/// and Trap-LeftArrow (79) (true, the y axis).
pub fn arrow_trap<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
    vertical: bool,
) {
    let t = p.target;
    // 1.
    if p.distance < cx.aip(p, 1) || p.distance > cx.aip(p, 2) {
        idle(game, cx, u, 40);
        return;
    }
    // 2.
    let (ox, oy) = cx.world.position(u);
    // T = 0 is unreachable (target mode 1): asserted.
    let tt = t.expect("arrow trap think without a target (ai.md §2.3)");
    let (tx, ty) = cx.world.position(tt);
    let axis = if vertical {
        oy.wrapping_sub(ty)
    } else {
        ox.wrapping_sub(tx)
    };
    let frame = game.frame;
    if axis.wrapping_abs() < 3 && frame > param(cx, u, 0) {
        set_param(cx, u, 0, game.after(cx.aip(p, 3)));
        let k = trap_kind(game, cx, u);
        if k != 1 {
            a1(game, cx, u, t);
            return;
        }
        if cx.skill(p, 1).0 >= 0 {
            // Edge case 2: written, never read.
            set_param(cx, u, 2, game.after(cx.aip(p, 4)));
            skill_k(game, cx, u, p, 1, t);
            return;
        }
    }
    // 3.
    idle(game, cx, u, 30);
}

// ---- Trap-Poison, Trap-Nova ----------------------------------------------

/// Trap-Poison (80) and Trap-Nova (92).
pub fn skill_trap<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let n = param(cx, u, 0);
    // 1.
    if p.target.is_some() && p.distance <= cx.aip(p, 1) && n < cx.aip(p, 2) {
        if cx.skill(p, 1).0 >= 0 && param(cx, u, 1) == 0 {
            skill_k(game, cx, u, p, 1, p.target);
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

// ---- JarJar --------------------------------------------------------------

/// The home command type.
const CMD_HOME: i32 = 10;

/// JarJar (81).
pub fn jar_jar<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    let frame = game.frame;
    // 1.
    if npc_home(game, cx, u) {
        return;
    }
    let h = find_command(cx, u, CMD_HOME, false);
    let hp = h.and_then(|i| command_mut(cx, u, i).map(|c| c.params));
    let set_w = |cx: &mut Ctx<'_, W>| {
        if let Some(c) = h.and_then(|i| command_mut(cx, u, i)) {
            c.params[4] = frame;
        }
    };
    // 2.
    if !cx
        .world
        .quest_hook(game, u, None, QuestHook::PalaceDoorOpen)
    {
        let [_, hx, hy, _, w] = hp.unwrap_or_default();
        // 2.1.
        let (moved, hx, hy) = cx.world.palace_guard_point(game, hx, hy);
        let stay = !moved;
        // 2.2.
        if w < frame && frame.wrapping_sub(w) < game.ticks(200) {
            idle(game, cx, u, 20);
            return;
        }
        // 2.3.
        if path_distance(cx, u, hx, hy) as u32 > 1 {
            walk_to_point(game, cx, u, hx, hy);
            set_w(cx);
            return;
        }
        // 2.4.
        if !stay {
            idle(game, cx, u, 20);
            return;
        }
        // 2.5.
        if npc_interaction_think(game, cx, u) {
            set_w(cx);
        } else {
            idle(game, cx, u, 20);
        }
        return;
    }
    // 3.
    let Some([_, hx, hy, _, _]) = hp else {
        idle(game, cx, u, 120);
        return;
    };
    // 3.1.
    let (pl, _) = cx.world.nearest_player(game, u);
    let d = unit_distance(cx, u, pl);
    // 3.2.
    let (gx, gy) = (hx, hy.wrapping_sub(3));
    let gd = path_distance(cx, u, gx, gy) as u32;
    if cx
        .world
        .quest_hook(game, u, None, QuestHook::PalaceGuardAside)
        && gd > 2
    {
        walk_to_point(game, cx, u, gx, gy);
        return;
    }
    if gd > 7 {
        walk_to_point(game, cx, u, gx, gy);
        return;
    }
    // 3.3. (Edge case 3: idle 120 also after a handler that scheduled.)
    if npc_interaction_think(game, cx, u) || pl == u {
        idle(game, cx, u, 120);
        return;
    }
    // 3.4.
    if is_player(game, pl) {
        if d < 12 {
            if roll(cx, u, 1000) < 100 {
                wander_near(game, cx, u, pl, 2);
            }
            idle(game, cx, u, 200);
            return;
        }
        idle(game, cx, u, 20);
    }
    // 3.5.
    if roll(cx, u, 1000) < 50 {
        wander(game, cx, u, 3);
    }
    idle(game, cx, u, 120);
}

// ---- InvisoSpawner -------------------------------------------------------

/// mummy1, the class-for-level base.
const MUMMY1: i32 = 96;

/// InvisoSpawner (82).
pub fn inviso_spawner<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let f = param(cx, u, 0);
    // 1.
    if f == 0 {
        set_param(cx, u, 1, cx.aip(p, 1));
    }
    // 2.
    if p.distance > cx.aip(p, 2) {
        idle(game, cx, u, 15);
        return;
    }
    // 3.
    let n = param(cx, u, 1);
    if n < 1 {
        death(game, cx, u);
        return;
    }
    // 4.
    if game.frame >= f {
        let room = room_of(game, u);
        let class = cx
            .world
            .class_for_level(game, room, fixed_class(cx, MUMMY1));
        if let (Some(r), Some((x, y))) = (room, cx.world.free_spot(game, room, 0, class)) {
            if cx
                .spawn_monster(game, r, x, y, class, mode::NEUTRAL, 2, 0x42)
                .is_some()
            {
                set_param(cx, u, 1, n - 1);
                set_param(cx, u, 0, game.after(cx.aip(p, 3)));
            }
        }
    }
    // 5.
    idle(game, cx, u, 15);
}

// ---- BoneWall, Trap-Melee, 7TIllusion ----------------------------

/// BoneWall's init: `Skill1` > 0 → param 0 := frame + the skill's
/// `Param2` (a lifetime in base ticks), or frame when its row is missing.
pub fn bone_wall_init<W: AiHost + ?Sized>(game: &Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    let class = cx.world.class(u);
    let (s1, _) = cx.class_skill(class, 1);
    if s1 > 0 {
        let add = skill_row(cx, s1).map_or(0, |r| r.param2 as i32);
        set_param(cx, u, 0, game.after(add));
    }
}

/// BoneWall (84).
pub fn bone_wall<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    if game.frame > param(cx, u, 0) {
        death(game, cx, u);
    } else {
        idle(game, cx, u, 15);
    }
}

/// Trap-Melee (87).
pub fn trap_melee<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    if !p.combat {
        idle(game, cx, u, 40);
    } else if pct(cx, u) < cx.aip(p, 1) {
        a1(game, cx, u, p.target);
    } else {
        idle(game, cx, u, cx.aip(p, 2));
    }
}

/// 7TIllusion (88).
pub fn seven_tombs<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    if param(cx, u, 1) == 1 {
        no_drop_death(game, cx, u);
        return;
    }
    let (x, y) = cx.world.position(u);
    mode_point_raw(game, cx, u, mode::ATTACK1, x.wrapping_sub(10), y);
    set_param(cx, u, 1, 1);
}

// ---- DarkWanderer, ArcaneTower, Spirit --------------------------

/// The Dark Wanderer's leave: flag 0x20000, death, the minion hook.
fn wanderer_leave<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    no_drop_death(game, cx, u);
    cx.world
        .quest_hook(game, u, None, QuestHook::DarkWandererGone);
}

/// DarkWanderer (91). No draws.
pub fn dark_wanderer<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    // 1.
    let Some((x, y)) = cx.world.dark_wanderer_target(game, u) else {
        idle(game, cx, u, 10);
        return;
    };
    // 2.
    if param(cx, u, 0) == 0 {
        set_param(cx, u, 0, 1);
        set_param(cx, u, 1, 0);
        set_param(cx, u, 2, 0);
    }
    // 3.
    if p.target.is_none() || p.distance >= 20 {
        idle(game, cx, u, 40);
        return;
    }
    match param(cx, u, 0) {
        // 4.
        1 => {
            walk_to_point(game, cx, u, x, y);
            set_param(cx, u, 0, 2);
        }
        // 5.
        2 => {
            if path_distance(cx, u, x, y) < 2 {
                wanderer_leave(game, cx, u);
                return;
            }
            let n = param(cx, u, 1);
            if n < 3 {
                set_param(cx, u, 1, n + 1);
                set_velocity(cx, u, 1, 0, 0);
                walk_to_point(game, cx, u, x, y);
            } else {
                wanderer_leave(game, cx, u);
            }
        }
        // 6.
        _ => idle(game, cx, u, 40),
    }
}

/// ArcaneTower (93). No draws.
pub fn arcane_tower<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let frame = game.frame;
    // 1.
    if param(cx, u, 1) == 0 {
        set_param(cx, u, 1, cx.aip(p, 1));
    }
    // 2.
    if frame < param(cx, u, 2) {
        idle(game, cx, u, 10);
        return;
    }
    let n = param(cx, u, 1) - 1;
    // 3.
    if cx.skill(p, 1).0 >= 0 && param(cx, u, 0) == 0 {
        skill_k(game, cx, u, p, 1, p.target);
        if n < 1 {
            set_param(cx, u, 0, 1);
            set_param(cx, u, 1, cx.aip(p, 4));
            set_param(cx, u, 2, game.after(cx.aip(p, 3)));
        } else {
            set_param(cx, u, 1, n);
            set_param(cx, u, 2, game.after(cx.aip(p, 2)));
        }
        return;
    }
    // 4.
    a1(game, cx, u, p.target);
    if n < 1 {
        set_param(cx, u, 0, 0);
        set_param(cx, u, 1, cx.aip(p, 1));
        set_param(cx, u, 2, game.after(cx.aip(p, 5)));
    } else {
        set_param(cx, u, 1, n);
        set_param(cx, u, 2, game.after(cx.aip(p, 6)));
    }
}

/// Spirit (97).
pub fn spirit<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    if param(cx, u, 0) != 0 {
        idle(game, cx, u, 50);
    } else if p.combat {
        set_param(cx, u, 0, 1);
        a1(game, cx, u, p.target);
    } else {
        idle(game, cx, u, 10);
    }
}

// ---- BladeCreeper, InvisoPet, DeathSentry -----------------------

/// BladeCreeper's init: params −1, 1, 0.
pub fn blade_creeper_init<W: AiHost + ?Sized>(cx: &mut Ctx<'_, W>, u: UnitId) {
    set_param(cx, u, 0, -1);
    set_param(cx, u, 1, 1);
    set_param(cx, u, 2, 0);
}

/// Stats 19 `tohit`, 119 `item_tohit_percent`.
const STAT_TOHIT: u16 = 19;
const STAT_TOHIT_PERCENT: u16 = 119;

/// BladeCreeper (102).
pub fn blade_creeper<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let frame = game.frame;
    let (s1, _) = cx.skill(p, 1);
    // 1.
    let row = skill_row(cx, s1);
    let (Some(row), true) = (row, cx.world.skill_entry(u, s1).is_some()) else {
        death(game, cx, u);
        return;
    };
    let (calc4, missile) = (row.calc4, i32::from(row.srvmissilea as i16));
    let lvl = cx.world.skill_level(u, s1, true).unwrap_or(0);
    // 2.
    if param(cx, u, 0) < 0 {
        // The calc gives the lifetime in base ticks.
        let life = cx.world.skill_calc(game, u, s1, calc4, lvl);
        set_param(cx, u, 0, frame.wrapping_add(game.ticks(life)));
    }
    // 3.
    if frame > param(cx, u, 0) {
        death(game, cx, u);
        return;
    }
    // 4.
    let valid = usize::try_from(missile).is_ok_and(|m| m < cx.tables.missiles.len());
    if param(cx, u, 2) == 0 && valid {
        let o = minion_owner(game, cx, u).unwrap_or(u);
        let (x, y) = cx.world.position(u);
        if let Some(m) = cx.world.skill_missile(game, o, s1, lvl, missile, x, y) {
            cx.world.link_owner(game, m, u);
            let ar = cx.world.attack_rating(o);
            cx.world.set_stat(u, STAT_TOHIT, ar);
            let pct = cx
                .world
                .stat(o, STAT_TOHIT_PERCENT)
                .wrapping_add(cx.world.mastery_tohit(o).unwrap_or(0));
            cx.world.set_stat(u, STAT_TOHIT_PERCENT, pct);
        }
        set_param(cx, u, 2, 1);
    }
    // 5.
    let Some(k) = current_command(cx, u) else {
        idle(game, cx, u, 3);
        return;
    };
    // 6.
    set_velocity(cx, u, 15, 0, 20);
    let (a, b) = ((k.params[1], k.params[2]), (k.params[3], k.params[4]));
    let (first, second, leg) = if param(cx, u, 1) == 0 {
        (a, b, 1)
    } else {
        (b, a, 0)
    };
    if walk_point_del(game, cx, u, first.0, first.1) {
        return;
    }
    set_param(cx, u, 1, leg);
    if walk_point_del(game, cx, u, second.0, second.1) {
        return;
    }
    let o = minion_owner(game, cx, u).unwrap_or(u);
    if !wander_near(game, cx, u, o, 5) {
        idle(game, cx, u, 5);
    }
}

/// Teleport 2.
const SKILL_TELEPORT2: i32 = 292;

/// InvisoPet (103).
pub fn inviso_pet<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    // Without a minion owner there is nothing to read
    // (unreachable in practice): asserted in debug
    // builds; a release build does nothing.
    let o = minion_owner(game, cx, u);
    debug_assert!(
        o.is_some(),
        "InvisoPet think without a minion owner (ai-bodies-7.md §16)"
    );
    let Some(o) = o else {
        return;
    };
    // 1.
    let running = cx.world.anim_mode(o) == 3;
    let a1v = cx.aip(p, 1);
    let r = if running { a1v / 2 } else { a1v };
    // 2.
    if game.frame <= param(cx, u, 0) && !running && !cx.ai_state_set(u) {
        idle(game, cx, u, cx.aip(p, 2));
        return;
    }
    // 3.
    let (ox, oy) = cx.world.position(o);
    let x = ox.wrapping_add(roll(cx, u, 2 * r)).wrapping_sub(r);
    let y = oy.wrapping_add(roll(cx, u, 2 * r)).wrapping_sub(r);
    // 4.
    use_skill(
        game,
        cx,
        u,
        mode::SKILL2,
        SKILL_TELEPORT2,
        ModeTarget::Point(x, y),
    );
    set_param(cx, u, 0, game.after(cx.aip(p, 2)));
}

/// DeathSentry (104) (init).
pub fn death_sentry<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    use super::bodies6::sentry_charges;
    // 1.
    if sentry_charges(game, cx, u, p, false) {
        return;
    }
    // 2. Nothing scheduled.
    let (s1, _) = cx.skill(p, 1);
    let lvl = cx.world.skill_level(u, s1, true).unwrap_or(0);
    if cx.world.skill_entry(u, s1).is_none() || lvl <= 0 {
        return;
    }
    // 3.
    let (s, e2, _) = cx.world.secondary_target(game, u, cx.store);
    let Some(s) = s else {
        idle(game, cx, u, cx.aip(p, 2));
        return;
    };
    // 4.
    let k = cx.world.corpse_search(game, u, Some(s), s1, lvl);
    let r = skill_row(cx, s1).map_or(0, |row| {
        (row.param3 as i32).wrapping_add((lvl - 1).wrapping_mul(row.param4 as i32)) / 2
    });
    // 5.
    if let Some(k) = k {
        let g = guid_of(game, Some(k));
        if g != param(cx, u, 0) && reach_distance(cx, s, k) < r {
            if sentry_charges(game, cx, u, p, true) {
                return;
            }
            set_param(cx, u, 0, g);
            use_skill(game, cx, u, mode::SKILL2, s1, ModeTarget::Unit(k));
            return;
        }
    }
    // 6.
    if e2 < cx.aip(p, 4) && roll(cx, u, 100) < cx.aip(p, 3) {
        if sentry_charges(game, cx, u, p, true) {
            return;
        }
        let (s2, _) = cx.skill(p, 2);
        use_skill(game, cx, u, mode::SEQUENCE, s2, ModeTarget::Unit(s));
        return;
    }
    // 7.
    idle(game, cx, u, cx.aip(p, 2));
}

// ---- ShadowWarrior ------------------------------------------------------

/// aip8's three columns read as constants (edge case 6): K0, K1, K2.
fn aip8_columns<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, class: i32) -> (i32, i32, i32) {
    cx.monstats(class).map_or((0, 0, 0), |r| {
        (
            i32::from(r.aip8 as i16),
            i32::from(r.aip8_n as i16),
            i32::from(r.aip8_h as i16),
        )
    })
}

/// ShadowWarrior's init: λ := 1, or O's level with bonus of
/// skill K0.
pub fn shadow_warrior_init<W: AiHost + ?Sized>(game: &Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    let (k0, _, _) = aip8_columns(cx, cx.world.class(u));
    // Init: O's entry is `highest_entry(O, K0)`, its
    // level with bonus.
    let lambda = minion_owner(game, cx, u)
        .and_then(|o| cx.world.skill_level(o, k0, true))
        .unwrap_or(1);
    set_param(cx, u, 2, lambda);
}

/// The skill's `range` as the range test reads it: 3 ("both") → 2 when
/// the unit has a ranged weapon, else 1; 1 again for a player with state 38.
fn range_kind<W: AiHost + ?Sized>(game: &Game, cx: &Ctx<'_, W>, u: UnitId, skill: i32) -> i32 {
    let r = skill_row(cx, skill).map_or(0, |r| i32::from(r.range));
    if r != 3 {
        return r;
    }
    if is_player(game, u) && cx.world.has_state(u, 38) {
        return 1;
    }
    if cx.world.ranged_both(u) {
        2
    } else {
        1
    }
}

/// The pet test (the pettype rules).
pub(super) fn pet_test<W: AiHost + ?Sized>(
    game: &Game,
    cx: &Ctx<'_, W>,
    o: Option<UnitId>,
    u: UnitId,
    skill: i32,
) -> bool {
    let Some(row) = skill_row(cx, skill) else {
        return true;
    };
    let summon = i32::from(row.summon);
    if summon == 0 {
        return true;
    }
    if summon == cx.world.class(u) {
        return false;
    }
    // Allowed: `pettype` (+0xBE, a signed byte) is valid when ≥ 0 and
    // below the pettype row count.
    let pettype = i32::from(row.pettype as i8);
    match o {
        Some(o) if pettype >= 0 && pettype < cx.world.pettype_count() => {
            cx.world.pet_type_of(game, o, u) != pettype
        }
        _ => true,
    }
}

/// Allowed.
pub(super) fn shadow_allowed<W: AiHost + ?Sized>(
    game: &Game,
    cx: &Ctx<'_, W>,
    o: Option<UnitId>,
    u: UnitId,
    skill: i32,
    t: Option<UnitId>,
    c: bool,
) -> bool {
    let Some(row) = skill_row(cx, skill) else {
        return false;
    };
    if !pet_test(game, cx, o, u, skill) {
        return false;
    }
    let ty = row.aitype;
    let close = matches!(ty, 4 | 13);
    if c != close {
        return false;
    }
    let (aura, tstate) = aura_states(row);
    if ty == 1 && aura >= 1 && cx.world.has_state(u, aura as u16) {
        return false;
    }
    if t.is_none() && matches!(ty, 2 | 4 | 5 | 11 | 12 | 13) {
        return false;
    }
    if ty == 2
        && ((aura >= 1 && cx.world.has_state(u, aura as u16))
            || (tstate >= 1 && t.is_some_and(|t| cx.world.has_state(t, tstate as u16))))
    {
        return false;
    }
    if row.progressive
        && aura >= 1
        && cx.world.has_state(u, aura as u16)
        && cx
            .world
            .state_stat(u, aura, i32::from(row.aurastat1))
            .is_some_and(|v| v >= 3)
    {
        return false;
    }
    true
}

/// Mana cost: ((`lvlmana` × (lvl − 1) + `mana`) <<
/// `manashift`) >> 8, at least 0.
fn mana_cost<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, skill: i32, lvl: i32) -> i32 {
    skill_row(cx, skill).map_or(0, |r| {
        let base = i32::from(r.lvlmana as i16)
            .wrapping_mul(lvl - 1)
            .wrapping_add(i32::from(r.mana as i16));
        (base.wrapping_shl(u32::from(r.manashift)) >> 8).max(0)
    })
}

/// Usable.
#[allow(clippy::too_many_arguments)]
pub(super) fn shadow_usable<W: AiHost + ?Sized>(
    game: &Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
    o: UnitId,
    skill: i32,
    t: Option<UnitId>,
    c: bool,
) -> bool {
    let (_, k1, k2) = aip8_columns(cx, p.class as i32);
    // 1.
    let Some(row) = skill_row(cx, skill) else {
        return false;
    };
    let cc = if (row.charclass as i8) < 0 {
        7
    } else {
        i32::from(row.charclass)
    };
    if cc != cx.world.class(o) {
        return false;
    }
    // 2.
    if !shadow_allowed(game, cx, Some(o), u, skill, t, c) {
        return false;
    }
    // 3.
    if skill == 0 {
        return true;
    }
    // 4.
    let lvl = cx.world.skill_level(u, skill, false).unwrap_or(0);
    let cost = mana_cost(cx, skill, lvl);
    if roll(cx, u, 100) > 100 - 160 * cost / 100 {
        return false;
    }
    // 5.
    if game.frame < param(cx, u, 0) {
        return false;
    }
    // 6.
    let lo = k1.clamp(1, 128);
    let hi = k2.clamp(1, 256);
    let mut m = param(cx, u, 1);
    if m < lo || m > 32 * hi {
        m = lo;
        set_param(cx, u, 1, m);
    }
    // 7.
    let a = roll(cx, u, m);
    let b = roll(cx, u, 100);
    if b < a {
        return false;
    }
    // 8.
    let lambda = param(cx, u, 2);
    let add = (320 - lambda).wrapping_mul(cost) / (lambda + 100);
    set_param(cx, u, 1, m.wrapping_add(add));
    true
}

/// ShadowWarrior (105) (init).
pub fn shadow_warrior<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let (_, _, k2) = aip8_columns(cx, p.class as i32);
    // 1.
    delete_thinks(game, u);
    let Some(o) = minion_owner(game, cx, u) else {
        idle(game, cx, u, 100);
        return;
    };
    // 2.
    let h = k2.clamp(1, 256);
    let mut m = param(cx, u, 1).wrapping_add(-1 - cx.aip(p, 4));
    if m < 0 || m > 64 * h {
        m = 0;
    }
    set_param(cx, u, 1, m);
    // 3.
    let mut t = p.target;
    let d_o = reach_distance(cx, u, o);
    if p.distance > cx.aip(p, 1) || d_o > cx.aip(p, 2) {
        t = None;
    }
    // 4.
    if pet_follow(game, cx, u, t, o, p.combat, false, 6) {
        return;
    }
    // 5.
    if let (Some(tt), Some(right), Some(left)) = (
        t,
        cx.world.hand_skill(o, true),
        cx.world.hand_skill(o, false),
    ) {
        if shadow_skill(game, cx, u, p, o, tt, right, left) {
            return;
        }
    }
    // 6.
    idle(game, cx, u, 25);
}

/// → true when it ended the think.
#[allow(clippy::too_many_arguments)]
fn shadow_skill<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
    o: UnitId,
    t: UnitId,
    right: (i32, i32),
    left: (i32, i32),
) -> bool {
    let lambda = param(cx, u, 2);
    let c = p.combat;
    // 5.1. Mimic levels.
    for (id, lvl) in [right, left] {
        let level = (lambda / 3 + lvl / 2).max(1);
        cx.world.assign_skill(game, u, id, level);
    }
    let has = |cx: &Ctx<'_, W>, id: i32| cx.world.skill_level(u, id, false).is_some();
    if !has(cx, right.0) || !has(cx, left.0) {
        return false;
    }
    // 5.2.
    let mut x = if roll(cx, u, 2) != 0 {
        Some(left.0)
    } else {
        Some(right.0)
    };
    // 5.3.
    let q = (cx.aip(p, 3) - 2 * lambda.max(1)).clamp(5, 100);
    if c && roll(cx, u, 100) < q {
        x = has(cx, 0).then_some(0);
    }
    // 5.4.
    let usable = |game: &Game, cx: &mut Ctx<'_, W>, x: Option<i32>| {
        x.is_some_and(|id| shadow_usable(game, cx, u, p, o, id, Some(t), c))
    };
    if !usable(game, cx, x) {
        x = Some(if x == Some(left.0) { right.0 } else { left.0 });
        if !usable(game, cx, x) {
            x = if has(cx, 0) {
                Some(0)
            } else {
                cx.world.assign_skill(game, u, 0, 1);
                Some(0)
            };
        }
    }
    // 5.5.
    let Some(id) = x else {
        return false;
    };
    // 5.6.
    if range_kind(game, cx, u, id) != 1 || c {
        let m = cx.world.entry_mode(u, id).unwrap_or(mode::ATTACK1);
        use_skill(game, cx, u, m, id, ModeTarget::Unit(t));
        let lvl = cx.world.skill_level(u, id, false).unwrap_or(0).max(1);
        let delay = skill_row(cx, id).map_or(0, |r| r.delay);
        let d = cx.world.skill_calc(game, u, id, delay, lvl);
        set_param(
            cx,
            u,
            0,
            game.frame.wrapping_add(game.ticks(18)).wrapping_add(d / 3),
        );
    } else {
        run_to(game, cx, u, Some(t), 0);
    }
    true
}

// ---- Raven --------------------------------------------------------------

/// Raven's init: c := −1; f := aip3 + 1; σ := one step's
/// `lo' & 1`.
pub fn raven_init<W: AiHost + ?Sized>(cx: &mut Ctx<'_, W>, u: UnitId) {
    let class = cx.world.class(u);
    set_param(cx, u, 0, -1);
    set_param(cx, u, 1, cx.class_aip(class, 3) + 1);
    let sigma = (cx.world.seed(u).step() & 1) as i32;
    set_param(cx, u, 2, sigma);
}

/// Walk to radius.
fn walk_to_radius<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    o: UnitId,
    r: i32,
) -> bool {
    let d = reach_distance(cx, u, o);
    if d == 0 {
        return false;
    }
    let (ux, uy) = cx.world.position(u);
    let (ox, oy) = cx.world.position(o);
    let x = ox.wrapping_add(ux.wrapping_sub(ox).wrapping_mul(r) / d);
    let y = oy.wrapping_add(uy.wrapping_sub(oy).wrapping_mul(r) / d);
    mode_point_raw(game, cx, u, mode::WALK, x, y)
}

/// Raven (107) (init).
pub fn raven<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    let frame = game.frame;
    let t = p.target;
    // 1.
    let Some(o) = minion_owner(game, cx, u) else {
        idle(game, cx, u, 10);
        return;
    };
    // 2.
    let c = param(cx, u, 0);
    if c == -1 {
        let (s1, _) = cx.skill(p, 1);
        let mut n = 3;
        if s1 >= 0 {
            // O's level of its highest `Skill1` entry, with
            // bonus (the init lookup).
            let lvl = cx.world.skill_level(o, s1, true).unwrap_or(0);
            n = if lvl <= 0 {
                0
            } else {
                skill_row(cx, s1).map_or(0, |r| {
                    (r.param5 as i32).wrapping_add((lvl - 1).wrapping_mul(r.param6 as i32))
                })
            };
        }
        set_param(cx, u, 0, n);
    } else if c == 0 {
        cx.world.kill(game, u, Some(u));
        return;
    }
    // 3.
    let d_o = unit_distance(cx, u, o);
    if d_o > 50 {
        pet_move(game, cx, Mover::Pet, o, u, 3, false, 0, 0);
        return;
    }
    // 4.
    let v = run_speed(cx, p);
    if d_o > 28 {
        pet_move(game, cx, Mover::Pet, o, u, 0, false, v, 0);
        return;
    }
    // 5. Its distance to the raven is computed and not used.
    let _ = owner_view(game, cx, u, o);
    // 6.
    let mid = (cx.aip(p, 2) + cx.aip(p, 1)) / 2;
    // 7.
    if t.is_some()
        && param(cx, u, 1) < frame
        && roll(cx, u, 100) < cx.aip(p, 4)
        && p.distance < cx.aip(p, 5)
    {
        if p.combat {
            a1(game, cx, u, t);
            set_param(cx, u, 0, param(cx, u, 0) - 1);
            set_param(cx, u, 1, game.after(cx.aip(p, 3).wrapping_mul(10)));
        } else {
            walk_to(game, cx, u, t, 0);
        }
        return;
    }
    // 8. Orbit.
    if !(d_o > cx.aip(p, 1) || d_o < cx.aip(p, 2)) {
        let orbit = |game: &mut Game, cx: &mut Ctx<'_, W>| {
            let sigma = param(cx, u, 2) != 0;
            set_velocity(cx, u, if sigma { 5 } else { 6 }, 0, 4);
            move_to(game, cx, u, ModeTarget::Unit(o), mode::WALK, 1, 0)
        };
        if orbit(game, cx) {
            return;
        }
        set_param(cx, u, 2, i32::from(param(cx, u, 2) == 0));
        if orbit(game, cx) {
            return;
        }
    }
    // 9.
    if !walk_to_radius(game, cx, u, o, mid) {
        pet_move(game, cx, Mover::Pet, o, u, 1, false, 0, 0);
    }
}

// ---- Vines, DruidBear -----------------------------------------------

/// State 2 `poison`, stat 45 `poisonresist`.
const STATE_POISON: u16 = 2;
const STAT_POISONRESIST: u16 = 45;

/// Vines (110) (init).
pub fn vines<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    // 1.
    let Some(o) = minion_owner(game, cx, u) else {
        idle(game, cx, u, 25);
        return;
    };
    // 2.
    if reach_distance(cx, u, o) >= cx.aip(p, 5)
        && pet_move(game, cx, Mover::Pet, o, u, 3, false, 0, 6)
    {
        return;
    }
    // 3.
    if in_town(game, cx, u) {
        if !pet_follow(game, cx, u, None, o, false, false, 6) {
            idle(game, cx, u, cx.aip(p, 3));
        }
        return;
    }
    // 4.
    let (mut s, e, m) = cx.world.secondary_target(game, u, cx.store);
    if e >= cx.aip(p, 2) {
        s = None;
    }
    // 5.
    if pet_follow(game, cx, u, s, o, m, false, 6) {
        return;
    }
    // 6.
    if let Some(st) = s {
        if cx.world.has_state(st, STATE_POISON) || cx.world.stat(st, STAT_POISONRESIST) == 100 {
            let n = i32::from(cx.aip(p, 4) as u8);
            escape(game, cx, u, s, n, false, false);
            return;
        }
        if !m {
            walk_to(game, cx, u, s, 7);
            return;
        }
        let f = param(cx, u, 1);
        if cx.skill(p, 1).0 >= 0 && game.frame > f.wrapping_add(game.ticks(cx.aip(p, 1))) {
            skill_k(game, cx, u, p, 1, s);
            set_param(cx, u, 1, game.frame);
            return;
        }
    }
    // 7.
    idle(game, cx, u, cx.aip(p, 3));
}

/// DruidBear (112).
pub fn druid_bear<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    // 1.
    let Some(o) = minion_owner(game, cx, u) else {
        let g = param(cx, u, 2);
        match player_by_guid(game, g).filter(|_| g != 0) {
            Some(pl) => {
                pet_move(game, cx, Mover::Pet, pl, u, 3, false, 0, 0);
            }
            None => idle(game, cx, u, 10),
        }
        return;
    };
    // 2.
    set_param(cx, u, 2, guid_of(game, Some(o)));
    let d_o = unit_distance(cx, u, o);
    let om = cx.world.anim_mode(o);
    // 3.
    if d_o > 50 {
        pet_move(game, cx, Mover::Pet, o, u, 3, false, 0, 0);
        return;
    }
    // 4.
    let v = run_speed(cx, p);
    if d_o > 28 {
        pet_move(game, cx, Mover::Pet, o, u, 0, false, 100, 0);
        return;
    }
    // 5.
    if d_o > 18 {
        let started = match om {
            2 | 6 => pet_move(game, cx, Mover::Pet, o, u, 0, false, 0, 0),
            3 => pet_move(game, cx, Mover::Pet, o, u, 0, false, 100, 0),
            _ => false,
        };
        if started {
            return;
        }
    }
    // 6.
    let (t0, _) = owner_view(game, cx, u, o);
    let (mut s, _, m) = search_capped(game, cx, u, 28);
    s = s.filter(|&s| cx.world.can_reach_directly(game, u, s));
    if s.is_none() {
        s = t0.filter(|&t| unit_distance(cx, u, t) < 28 && cx.world.can_reach_directly(game, u, t));
    }
    if let Some(st) = s {
        // 7.
        if !m {
            if roll(cx, u, 100) < cx.aip(p, 2) {
                set_velocity(cx, u, 0, v, 40);
                walk_to(game, cx, u, Some(st), 0);
                return;
            }
        } else {
            // 8.
            if roll(cx, u, 100) < cx.aip(p, 3) {
                let (s1, _) = cx.skill(p, 1);
                use_sequence_skill(game, cx, u, s1, ModeTarget::Unit(st));
            } else {
                a1(game, cx, u, Some(st));
                wait(game, cx, u, cx.aip(p, 1));
            }
            return;
        }
    }
    // 9.
    if d_o < 17 {
        idle(game, cx, u, 15);
    } else {
        pet_move(game, cx, Mover::Pet, o, u, 0, false, 0, 0);
    }
}

// ---- SiegeTower, GenericSpawner -------------------------------------

/// SiegeTower (113). No draws.
pub fn siege_tower<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let aip1 = cx.aip(p, 1);
    // 1.
    if cx
        .world
        .owner(game, u)
        .is_some_and(|o| !cx.world.is_dead(o))
    {
        idle(game, cx, u, aip1);
        return;
    }
    // 2. The second request replaces the first.
    if p.target.is_some() && rider_scan(game, cx, u, 400) {
        idle(game, cx, u, aip1);
    }
    // 3.
    idle(game, cx, u, aip1);
}

/// minion1, imp5 (the spawn class pick).
const MINION1: i32 = 453;
const IMP5: i32 = 496;
/// evilhut, the footprint class; skill 167 Nest.
const EVILHUT: i32 = 528;
const SKILL_NEST: i32 = 167;

/// GenericSpawner's init: FoulCrowNest's init, then the
/// minion spawn class (control +0x3C) := −1.
pub fn generic_spawner_init<W: AiHost + ?Sized>(game: &Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    nest_init(game, cx, u);
    if let Some(c) = cx.store.control_mut(u) {
        c.spawn_class = -1;
    }
}

/// Spawn class pick; always succeeds.
fn spawn_class_pick<W: AiHost + ?Sized>(game: &Game, cx: &mut Ctx<'_, W>, u: UnitId) -> i32 {
    let _ = fixed_class(cx, MINION1);
    let classes = cx.world.region_classes(game, u);
    let n = classes.len() as i32;
    let i = roll(cx, u, n);
    let flagged = |c: i32| cx.monstats(c).is_some_and(|r| r.genericspawn);
    if let Some(&c) = classes.get(i as usize).filter(|&&c| flagged(c)) {
        return c;
    }
    classes
        .iter()
        .copied()
        .find(|&c| flagged(c))
        .unwrap_or(IMP5)
}

/// GenericSpawner (129) (init).
pub fn generic_spawner<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    // 1.
    let Some(t) = p.target else {
        idle(game, cx, u, 20);
        return;
    };
    // 2. The pick always succeeds: its death branch is unreachable.
    if cx.store.control(u).is_some_and(|c| c.spawn_class == -1) {
        let c = spawn_class_pick(game, cx, u);
        if let Some(ctl) = cx.store.control_mut(u) {
            ctl.spawn_class = c;
        }
    }
    // 3.
    if p.distance >= 21 {
        idle(game, cx, u, 20);
        return;
    }
    // 4.
    let n = param(cx, u, 1);
    if n >= cx.aip(p, 3) {
        no_drop_death(game, cx, u);
        return;
    }
    // 5.
    if game.frame.wrapping_sub(param(cx, u, 0)).wrapping_abs() >= game.ticks(cx.aip(p, 1)) {
        set_param(cx, u, 0, game.frame);
        let class = fixed_class(cx, EVILHUT);
        let room = room_of(game, u);
        let (x, y) = cx.world.position(u);
        if cx.world.footprint_ok(game, class, room, x, y) {
            set_param(cx, u, 1, n.wrapping_add(1));
            use_sequence_skill(game, cx, u, SKILL_NEST, ModeTarget::Unit(t));
            return;
        }
    }
    // 6.
    idle(game, cx, u, 20);
}

// ---- Wussie -------------------------------------------------------------

/// Wussie (131).
pub fn wussie<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    // 1.
    if let Some(portal) = cx.world.rescue_portal(game, u) {
        // 1.1.
        let Some(pt) = portal else {
            idle(game, cx, u, 25);
            return;
        };
        // 1.2.
        let (px, py) = cx.world.position(pt);
        if cx.world.quest_hook(game, u, None, QuestHook::WussieLeaving)
            || (path_distance(cx, u, px, py) as u32) < 4
        {
            cx.world.quest_hook(game, u, None, QuestHook::WussieLeave);
            cx.world.stop_unit_path(u);
            delete_thinks(game, u);
            cx.world.remove_unit(game, u);
            return;
        }
        // 1.3.
        if pct(cx, u) < 70 {
            walk_to(game, cx, u, Some(pt), 7);
        } else if roll(cx, u, 100) < 10 {
            wander(game, cx, u, 4);
        } else {
            idle(game, cx, u, 10);
        }
        return;
    }
    // 2.
    let (pl, _) = cx.world.nearest_player(game, u);
    if is_player(game, pl) {
        if cx
            .world
            .quest_hook(game, u, Some(pl), QuestHook::WussieCanRescue)
        {
            cx.world
                .quest_hook(game, u, Some(pl), QuestHook::WussieRescue);
            if cx.world.target_slot(u) == SLOT_NONE {
                cx.world.register_target_node(game, u, SLOT_GOOD_NPC);
            }
            idle(game, cx, u, 25);
            return;
        }
        cx.world
            .quest_hook(game, u, Some(pl), QuestHook::WussieWait);
        if roll(cx, u, 1000) < 100 {
            cx.world.npc_wants_interact(game, pl, u);
        }
    }
    // 3.
    idle(game, cx, u, 25);
}

// ---- UberIzual, the Ubers -------------------------------------------

/// UberIzual (144).
pub fn uber_izual<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    // 1.
    let (s2, m2) = cx.skill(p, 2);
    if s2 >= 0 {
        if let Some(row) = skill_row(cx, s2) {
            let (aura, _) = aura_states(row);
            if aura >= 0 && !cx.world.has_state(u, aura as u16) {
                use_skill(game, cx, u, m2, s2, ModeTarget::Point(0, 0));
                return;
            }
        }
    }
    // 2.
    if let Some(t) = p.target {
        if cx.world.line_blocked_mask(game, u, t, 6) {
            let (s3, m3) = cx.skill(p, 3);
            let (x, y) = cx.world.position(t);
            use_skill(game, cx, u, m3, s3, ModeTarget::Point(x, y));
            return;
        }
    }
    // 3.
    izual_steps(game, cx, u, p);
}

/// UberBaal (145), UberMephisto (146),
/// UberDiablo (147): a bare return (no draw, no
/// mode, nothing scheduled).
pub fn uber_empty() {}

// ---- ShadowMaster -------------------------------------------------------

/// The fixed aip columns of ShadowMaster.
#[derive(Clone, Copy)]
struct MasterAips {
    a1n: i32,
    a1m: i32,
    a1h: i32,
    a2n: i32,
    a2m: i32,
    a2h: i32,
    k0: i32,
}

fn master_aips<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, class: i32) -> MasterAips {
    let s = |v: u16| i32::from(v as i16);
    cx.monstats(class).map_or(
        MasterAips {
            a1n: 0,
            a1m: 0,
            a1h: 0,
            a2n: 0,
            a2m: 0,
            a2h: 0,
            k0: 0,
        },
        |r| MasterAips {
            a1n: s(r.aip1),
            a1m: s(r.aip1_n),
            a1h: s(r.aip1_h),
            a2n: s(r.aip2),
            a2m: s(r.aip2_n),
            a2h: s(r.aip2_h),
            k0: s(r.aip8),
        },
    )
}

/// The init of ShadowMaster (106) (`class_skills` true) and
/// ShadowMasterNoInit (143) (false): init 143 is init 106
/// without the class skills.
pub fn shadow_master_init<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    class_skills: bool,
) {
    let k0 = master_aips(cx, cx.world.class(u)).k0;
    set_param(cx, u, 0, 0);
    set_param(cx, u, 1, 0);
    set_param(cx, u, 2, 1);
    let Some(o) = minion_owner(game, cx, u).filter(|&o| is_player(game, o)) else {
        return;
    };
    // Init: O's level of K0 with bonus (the init lookup: the highest
    // entry).
    let lambda = cx.world.skill_level(o, k0, true).unwrap_or(1);
    set_param(cx, u, 2, lambda);
    if cx.world.skill_level(u, 0, false).is_none() {
        cx.world.assign_skill(game, u, 0, 1);
    }
    cx.world.set_hand_skill(u, 0, false);
    cx.world.set_hand_skill(u, 0, true);
    if !class_skills {
        return;
    }
    for s in cx.world.class_skills(cx.world.class(o)) {
        if pet_test(game, cx, Some(o), u, s) {
            let v = cx.world.skill_base_level(o, s).unwrap_or(1);
            let lvl = (v / 2 + lambda / 2).clamp(1, 24);
            cx.world.assign_skill(game, u, s, lvl);
        }
    }
}

/// The use helper.
#[allow(clippy::too_many_arguments)]
fn master_use<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    o: Option<UnitId>,
    s: i32,
    m: bool,
    x: Option<UnitId>,
) -> bool {
    // Self targets are refused (edge case 5).
    if x.is_some_and(|x| Some(x) == o || x == u) {
        return false;
    }
    if cx.world.skill_level(u, s, false).is_none() {
        return false;
    }
    if let Some(xx) = x {
        if cx.world.unit_flags(xx) & 0x4 == 0 || in_town(game, cx, u) {
            return false;
        }
    }
    if range_kind(game, cx, u, s) == 1 && !m {
        return run_to(game, cx, u, x, move_flag::DELETE_THINKS);
    }
    let md = cx.world.entry_mode(u, s).unwrap_or(mode::ATTACK1);
    use_skill(game, cx, u, md, s, target_of(x))
}

/// "Use (s, X, 0, 0)" with M := melee range unit→X.
fn master_use_m<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    o: Option<UnitId>,
    s: i32,
    x: Option<UnitId>,
) -> bool {
    let m = x.is_some_and(|x| cx.world.in_melee_range(game, u, x));
    master_use(game, cx, u, o, s, m, x)
}

/// Notable.
fn notable<W: AiHost + ?Sized>(game: &Game, cx: &Ctx<'_, W>, u: UnitId, v: UnitId) -> bool {
    if v == u || !is_monster(game, v) || cx.world.is_dead(v) {
        return false;
    }
    let Some(r) = cx.monstats(cx.world.class(v)) else {
        return false;
    };
    !r.npc
        && r.killable
        && (cx.world.is_boss(v) || r.primeevil || cx.world.monster_type_flags(v) & 0x0E != 0)
}

/// The scan of step 8 (scan 1, callback).
#[derive(Default)]
pub(super) struct MasterScan {
    pub(super) best: Option<UnitId>,
    pub(super) dist: i32,
    pub(super) near: i32,
    pub(super) best_o: Option<UnitId>,
    pub(super) near_o: i32,
    pub(super) n: i32,
    pub(super) shadows: i32,
    pub(super) boss: Option<UnitId>,
}

/// The assassin traps' `BaseId`s (shadows counted by the scan).
fn is_trap_base(b: i32) -> bool {
    matches!(b, 410..=413 | 415 | 416)
}

pub(super) fn master_scan<W: AiHost + ?Sized>(
    game: &Game,
    cx: &Ctx<'_, W>,
    u: UnitId,
    o: Option<UnitId>,
) -> MasterScan {
    let mut r = MasterScan {
        dist: 0x7FFF_FFFF,
        ..Default::default()
    };
    let mut dist_o = 0x7FFF_FFFF;
    for v in scan_units(game, u) {
        if v == u || cx.world.is_dead(v) {
            continue;
        }
        if is_monster(game, v) && pairing(cx, u, v) && is_trap_base(unit_base(cx, v)) {
            r.shadows += 1;
            continue;
        }
        if cx.world.unit_flags(v) & 0x4 == 0 || !cx.world.hostile(game, u, v) {
            continue;
        }
        if let Some(o) = o {
            let d = sq_dist(cx, o, v);
            if d <= 100 {
                r.near_o += 1;
            }
            if d < dist_o {
                r.best_o = Some(v);
                dist_o = d;
            }
        }
        // Near, best and notable are tested only for a U within
        // 1024 of the unit, inside the n count.
        let d = sq_dist(cx, u, v);
        if d <= 1024 {
            r.n += 1;
            if d <= 100 {
                r.near += 1;
            }
            if d < r.dist {
                r.best = Some(v);
                r.dist = d;
            }
            if notable(game, cx, u, v) {
                r.boss = Some(v);
            }
        }
    }
    r
}

/// T's resist for `EType`.
fn etype_resist<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, t: UnitId, etype: u8) -> i32 {
    let stat = match etype {
        0 => 36,
        1 => 39,
        2 => 41,
        3 => 37,
        4 | 12 => 43,
        5 => 45,
        _ => return 0,
    };
    cx.world.stat(t, stat)
}

/// `srvdofunc` 19: the follow-up skill.
const SRVDOFUNC_FOLLOWUP: u16 = 19;

/// ShadowMaster (106) / ShadowMasterNoInit (143).
pub fn shadow_master<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let a = master_aips(cx, p.class as i32);
    let c = p.combat;
    let mut t = p.target;
    // 1.
    if !cx.world.has_skill_list(u) {
        idle(game, cx, u, 100);
        return;
    }
    // 2.
    delete_thinks(game, u);
    let o = minion_owner(game, cx, u);
    if let Some(oo) = o {
        if sq_dist(cx, u, oo) > a.a2h.wrapping_mul(a.a2h)
            && pet_follow(game, cx, u, None, oo, c, false, 6)
        {
            return;
        }
    }
    // 3.
    if param(cx, u, 0) > 0 {
        if let Some(pt) = cx.world.path_target(u) {
            t = Some(pt);
        }
        if t.is_none() {
            set_param(cx, u, 0, 0);
            set_param(cx, u, 1, 0);
        } else {
            set_param(cx, u, 0, param(cx, u, 0) - 1);
            if master_use(game, cx, u, o, param(cx, u, 1), c, t) {
                return;
            }
        }
    }
    // 4.
    if p.distance > a.a2m {
        t = None;
    }
    if t.is_none() && master_buffs(game, cx, u, c) {
        return;
    }
    // 5.
    let mut x = None;
    if let Some(oo) = o {
        x = cx
            .world
            .path_target(oo)
            .filter(|&v| !cx.world.is_dead(v) && cx.world.hostile(game, u, v));
        if x.is_some() {
            t = x;
        }
        if sq_dist(cx, u, oo) <= 144 && pet_follow(game, cx, u, t, oo, c, false, 6) {
            return;
        }
    }
    // 6.
    let Some(mut tt) = t else {
        idle(game, cx, u, 25);
        return;
    };
    // 7.
    let lambda = param(cx, u, 2);
    let q = (cx.aip(p, 3) - 2 * lambda.max(1)).clamp(5, 100);
    if c && roll(cx, u, 100) < q && master_use(game, cx, u, o, 0, c, Some(tt)) {
        return;
    }
    // 8.
    let sc = master_scan(game, cx, u, o);
    // 9.
    if !c {
        if let Some(xx) = x {
            tt = xx;
        } else if let Some(b) = sc.best_o {
            tt = b;
        } else if let Some(b) = sc.boss.filter(|&b| sq_dist(cx, u, b) < 1024) {
            tt = b;
        }
        if !notable(game, cx, u, tt) {
            if let Some(l) = minion_owner(game, cx, tt)
                .filter(|&l| !cx.world.is_dead(l) && sq_dist(cx, u, l) <= 1024)
            {
                tt = l;
            }
        }
    }
    // 10.
    let life = cx.world.life_percent(u);
    let dt = sq_dist(cx, u, tt);
    let pg = a.a1h >= 1 && cx.world.has_pgsv_state(u);
    let clear = !cx.world.line_blocked(game, u, tt);
    // 11.
    if sc.near > 3 && roll(cx, u, 32) < 2 * sc.near {
        if let Some(oo) = o.filter(|&oo| sq_dist(cx, u, oo) > 36) {
            run_to(game, cx, u, Some(oo), 0);
            return;
        }
        if escape(game, cx, u, Some(tt), 8, true, true) {
            return;
        }
    }
    // 12.
    let ctx = ScoreCtx {
        a,
        c,
        t: tt,
        o,
        life,
        dt,
        pg,
        clear,
    };
    let cands = master_scores(game, cx, u, &sc, &ctx);
    // 13. Last candidate first, each tried 3/4 of the time.
    for &(skill, target, _) in cands.iter().rev() {
        if cx.world.seed(u).step() & 3 == 0 {
            continue;
        }
        if master_use_m(game, cx, u, o, skill, target) {
            if skill_row(cx, skill).is_some_and(|r| r.srvdofunc == SRVDOFUNC_FOLLOWUP) {
                set_param(cx, u, 1, skill);
                set_param(cx, u, 0, 25);
            }
            return;
        }
    }
    // 14.
    if master_use_m(game, cx, u, o, 0, Some(tt)) {
        return;
    }
    idle(game, cx, u, 15);
}

/// The buff pass of step 4 → true when a mode change started.
fn master_buffs<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    c: bool,
) -> bool {
    for (s, _) in cx.world.unit_skills(u) {
        let Some(row) = skill_row(cx, s) else {
            continue;
        };
        let (aura, _) = aura_states(row);
        match row.aitype {
            1 => {
                if aura <= 0 || cx.world.has_state(u, aura as u16) {
                    continue;
                }
                if cx.world.state_group_active(u, aura) && roll(cx, u, 100) >= 4 {
                    continue;
                }
                if pct(cx, u) >= 60 || cx.world.skill_level(u, s, false).is_none() {
                    continue;
                }
                let started = if range_kind(game, cx, u, s) == 1 && !c {
                    run_to(game, cx, u, None, move_flag::DELETE_THINKS)
                } else {
                    let md = cx.world.entry_mode(u, s).unwrap_or(mode::ATTACK1);
                    use_skill(game, cx, u, md, s, ModeTarget::Point(0, 0))
                };
                if started {
                    return true;
                }
            }
            6 if cx.world.hand_skill(u, false).is_none() && pct(cx, u) < 20 => {
                cx.world.set_hand_skill(u, s, false);
            }
            _ => {}
        }
    }
    false
}

/// The inputs of the scoring.
struct ScoreCtx {
    a: MasterAips,
    c: bool,
    t: UnitId,
    o: Option<UnitId>,
    life: i32,
    dt: i32,
    pg: bool,
    clear: bool,
}

/// The scoring of step 12: the candidates (skill, target, score), slot
/// 0 = (0, T, 0); a skill is appended only when its score is strictly
/// greater than the last appended one.
fn master_scores<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    sc: &MasterScan,
    k: &ScoreCtx,
) -> Vec<(i32, Option<UnitId>, i32)> {
    let a = k.a;
    let t = k.t;
    let mut out = vec![(0, Some(t), 0)];
    let mut charges = 0i32;
    let difficulty = usize::from(cx.info.difficulty.min(2));
    for (skill, lvl) in cx.world.unit_skills(u) {
        let Some(row) = skill_row(cx, skill) else {
            continue;
        };
        let (aura, tstate) = aura_states(row);
        let r = etype_resist(cx, t, row.etype);
        let mut base =
            i32::from(row.aibonus as i16) + i32::from(row.reqlevel as i16) / 4 + lvl - r / 10;
        let mut target = Some(t);
        let pick = |cx: &mut Ctx<'_, W>, base: i32| roll(cx, u, a.a2n).wrapping_add(base);
        let has_aura = aura > 0 && cx.world.has_state(u, aura as u16);
        let s = match row.aitype {
            1 => {
                // Aitype 1: `aurastate` > 0 and the unit lacks it →
                // the case breaks with s = 0 (no pick, no draw).
                if aura > 0 && !cx.world.has_state(u, aura as u16) {
                    0
                } else {
                    if sc.dist <= 25 {
                        base -= 6;
                    }
                    base += if cx.world.state_group_active(u, aura) {
                        -10
                    } else {
                        10
                    };
                    target = Some(u);
                    pick(cx, base)
                }
            }
            2 => {
                let t_has = tstate > 0 && cx.world.has_state(t, tstate as u16);
                if has_aura || t_has {
                    0
                } else {
                    if sc.dist <= 25 {
                        base -= 10;
                    }
                    pick(cx, base)
                }
            }
            3 => {
                if sc.shadows > 5 {
                    base -= 2 * sc.shadows;
                }
                if sc.dist <= 25 {
                    base -= 7;
                }
                if sc.n < 3 {
                    base -= 10;
                }
                pick(cx, base) + 3 * sc.n - 9
            }
            ty @ (4 | 12) => {
                let drain_ok = ty == 4
                    || !is_monster(game, t)
                    || cx
                        .monstats(cx.world.class(t))
                        .is_some_and(|m| [m.drain, m.drain_n, m.drain_h][difficulty] >= 25);
                if !drain_ok {
                    0
                } else {
                    if k.dt > a.a1n.wrapping_mul(a.a1n) {
                        base -= 10;
                    }
                    base += a.a1m;
                    if k.c || k.dt <= 25 {
                        base += 10;
                    }
                    let mut spent = false;
                    if row.progressive && has_aura {
                        if let Some(v) = cx.world.state_stat(u, aura, i32::from(row.aurastat1)) {
                            charges += v;
                            spent = v >= 3;
                        }
                    }
                    if spent {
                        0
                    } else if ty == 12 {
                        // Aitype 12: one pick on
                        // the base, progressive or not, then the life bonuses
                        let mut s = pick(cx, base);
                        if k.life < 75 {
                            s += 8;
                        }
                        if k.life < 50 {
                            s += 12;
                        }
                        s
                    } else if row.progressive {
                        pick(cx, base + a.a1h)
                    } else {
                        if a.a1h > 0 && !k.pg {
                            base -= 10;
                        } else {
                            base += 4 * charges + 3;
                        }
                        pick(cx, base)
                    }
                }
            }
            ty @ (5 | 11) => {
                let ma = i32::from(row.srvmissilea as i16);
                let range_ok = i32::from(row.srvmissile as i16) >= 0
                    || ma < 0
                    || usize::try_from(ma)
                        .ok()
                        .and_then(|m| cx.tables.missiles.get(m))
                        .is_none_or(|m| {
                            let r = i32::from(m.range as i16) - 1;
                            k.dt < r.wrapping_mul(r)
                        });
                if !(k.clear && range_ok) {
                    0
                } else {
                    if sc.dist <= 25 {
                        base -= 5;
                    }
                    if k.dt <= 25 {
                        base -= 5;
                    }
                    if k.pg {
                        base -= 5;
                    }
                    if ty == 5 {
                        pick(cx, base)
                    } else {
                        pick(cx, base) + 3 * sc.n
                    }
                }
            }
            6 => {
                if cx.world.hand_skill(u, false).is_none() {
                    if roll(cx, u, 100) < 20 {
                        cx.world.set_hand_skill(u, skill, false);
                    }
                } else if roll(cx, u, 100) < 6 {
                    cx.world.set_hand_skill(u, skill, false);
                }
                0
            }
            7 => {
                let mut s = pick(cx, base);
                if k.life > 66 {
                    s = 0;
                } else {
                    target = None;
                    if k.o.is_none() {
                        // Edge case 5: two steps for a point never stored.
                        roll(cx, u, 40);
                        roll(cx, u, 40);
                    }
                    s += 10;
                    if k.life < 45 {
                        s += 10;
                    }
                }
                s
            }
            8 => {
                let r = pick(cx, base);
                if k.life > 66 {
                    0
                } else {
                    target = Some(u);
                    if k.life < 45 {
                        4 * r
                    } else {
                        2 * r
                    }
                }
            }
            13 => {
                base += a.a1m;
                if a.a1h > 0 && !k.pg {
                    base -= 5;
                } else {
                    base += charges;
                }
                let flee_to = sc.best_o.filter(|&b| {
                    (k.life < 50 || sc.near > 3) && sc.near_o < 4 && sq_dist(cx, u, b) > 25
                });
                if let Some(b) = flee_to {
                    base += 20;
                    target = Some(b);
                    pick(cx, base)
                } else if k.dt < 25 {
                    0
                } else {
                    if k.dt > 324 {
                        base += 10;
                    }
                    pick(cx, base)
                }
            }
            _ => 0,
        };
        if s > out.last().map_or(0, |l| l.2) {
            out.push((skill, target, s));
        }
    }
    out
}

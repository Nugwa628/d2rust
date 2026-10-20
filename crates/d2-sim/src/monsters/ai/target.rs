//! The three prechecks of the think dispatch and the main target search.

use crate::game::Game;
use crate::units::{UnitId, UnitType};

use super::tactics::{
    distance_no_size, escape, set_velocity, unit_distance, use_skill, wander, wander_near,
};
use super::{flag, idle, idle_keep_mode, mode, state, AiHost, Ctx, ModeTarget, TickParam};

/// Distance when no player qualified.
pub const NO_DISTANCE: i32 = 0x7FFF_FFFF;

/// Precheck A: stun, doors, minion leash. True =
/// stop.
pub fn precheck_a<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    unit: UnitId,
    p: &TickParam,
) -> bool {
    // 1. Stun.
    if cx.world.has_state(unit, state::STUNNED) {
        idle(game, cx, unit, 3);
        return true;
    }
    // 2. Doors.
    let opendoors = cx.tables.monstats.get(p.class).is_some_and(|r| r.opendoors);
    if opendoors && cx.world.path_blocked(unit) {
        if let Some(door) = cx.world.find_door(game, unit) {
            if cx.world.door_monster_ok(door) {
                cx.world.operate_door(game, unit, door);
                idle(game, cx, unit, 5);
                return true;
            }
        }
    }
    // 3. Minion leash.
    let Some(owner_ref) = cx.store.control(unit).and_then(|c| c.owner) else {
        return false;
    };
    let owner = game.lists.find_unit(owner_ref.ty, owner_ref.guid);
    let owner = match owner {
        Some(o) if cx.can_walk(unit) && !cx.world.is_dead(o) => o,
        _ => {
            if let Some(c) = cx.store.control_mut(unit) {
                c.owner = None;
            }
            return false;
        }
    };
    let d = unit_distance(cx, unit, owner);
    if d <= 1 && owner_ref.ty == UnitType::Player {
        if !escape(game, cx, unit, Some(owner), 19, true, false) {
            wander_near(game, cx, unit, owner, 19);
        }
        return true;
    }
    if d > 20 {
        set_velocity(cx, unit, 7, 0, 40);
        wander_near(game, cx, unit, owner, 19);
        return true;
    }
    false
}

/// The result of the main search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Search {
    pub target: Option<UnitId>,
    pub distance: i32,
    pub combat: bool,
}

/// The main target search.
pub fn main_search<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    unit: UnitId,
) -> Search {
    main_search_with(game, cx, unit, unit)
}

/// The main search run for `unit` with the AI control of
/// `ctl` (the pets' "owner view": the search for the
/// owner with the pet's control record). Control flags 0x40 / 0x08 are
/// read and written on `ctl`'s control; everything else is `unit`'s.
pub fn main_search_with<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    unit: UnitId,
    ctl: UnitId,
) -> Search {
    let none = Search {
        target: None,
        distance: 0,
        combat: false,
    };
    // 1.
    let Some(room) = game.lists.unit(unit).and_then(|e| e.room()) else {
        return none;
    };
    // 2. Line-of-sight flag T.
    let flags = cx.store.control(ctl).map_or(0, |c| c.flags);
    // `vis`: the vision record's seen flag as step 2 read it (S, 0 when the
    // record was loaded but not read); `None` = no record loaded.
    let mut vis: Option<u32> = None;
    let los = if flags & flag::FORCE_LOS != 0 {
        if let Some(c) = cx.store.control_mut(ctl) {
            c.flags &= !flag::FORCE_LOS;
        }
        true
    } else if !cx.world.los_draw(game, room) {
        let t = flags & flag::TARGET_SEEN == 0;
        match cx.world.vision_seen(unit) {
            Some(v) if t => {
                vis = Some(v);
                v == 0
            }
            Some(_) => {
                vis = Some(0);
                t
            }
            None => t,
        }
    } else {
        false
    };
    // 3. Forced target: it takes step 7 like
    // any target.
    let align = cx.world.alignment(unit);
    if let Some((t, d)) = cx.world.forced_target(game, unit, los, false, cx.store) {
        return finish(game, cx, unit, ctl, align, vis, t, d);
    }
    // 4. Not evil.
    if align != 0 {
        return match cx.world.good_target_search(game, unit, los, cx.store) {
            Some((t, d)) => finish(game, cx, unit, ctl, align, vis, t, d),
            None => Search {
                target: None,
                distance: NO_DISTANCE,
                combat: false,
            },
        };
    }
    // 5. Evil: the target-node lists.
    let mut best = cx.aidist(cx.world.class(unit));
    let mut min_player = NO_DISTANCE;
    let mut chosen: Option<UnitId> = None;
    let my_act = cx.world.act(unit);
    let my_pos = cx.world.position(unit);
    let nodes = cx.world.target_nodes(game);
    let passes =
        |cx: &Ctx<'_, W>, game: &Game, n: UnitId| !los || !cx.world.line_blocked(game, unit, n);
    for slot in &nodes.players {
        let Some(&head) = slot.first() else {
            continue;
        };
        assert!(
            game.lists
                .unit(head)
                .is_some_and(|e| e.ty == UnitType::Player),
            "a player target list's head is not a player"
        );
        let head_room = game.lists.unit(head).and_then(|e| e.room());
        let qualifies =
            cx.world.act(head) == my_act && head_room.is_some_and(|r| !cx.world.in_town(game, r));
        if !qualifies {
            continue;
        }
        let d = distance_no_size(cx.world.position(head), my_pos);
        min_player = min_player.min(d);
        if d >= 55 {
            continue;
        }
        for (i, &n) in slot.iter().enumerate() {
            let d = if i == 0 {
                if cx.world.is_dead(head) {
                    NO_DISTANCE
                } else {
                    d
                }
            } else {
                distance_no_size(cx.world.position(n), my_pos)
            };
            if d < best && passes(cx, game, n) {
                chosen = Some(n);
                best = d;
            }
        }
    }
    // 5.2 The good NPCs' list.
    for &n in &nodes.good {
        if cx.world.act(n) != my_act {
            continue;
        }
        let d = distance_no_size(cx.world.position(n), my_pos);
        if d < best && passes(cx, game, n) {
            chosen = Some(n);
            best = d;
        }
    }
    // 5.3 The bone walls' list.
    let mut alt: Option<(UnitId, i32)> = None;
    for &n in &nodes.walls {
        if cx.world.act(n) != my_act {
            continue;
        }
        let d = distance_no_size(cx.world.position(n), my_pos);
        if alt.is_none_or(|(_, ad)| d < ad) && passes(cx, game, n) {
            alt = Some((n, d));
        }
    }
    if let Some((a, ad)) = alt {
        // The bone-wall pick: the alternative (B := its own no-size
        // distance, so step 7 reports it), main kept, or main replaced
        // by its scan 7 unit (B := that unit's distance).
        let main = chosen.map(|c| (c, best));
        if let Some((t, d)) = cx
            .world
            .choose_alternative(game, unit, main, (a, ad), cx.store)
        {
            chosen = Some(t);
            best = d;
        }
    }
    // 6.
    let Some(t) = chosen else {
        return Search {
            target: None,
            distance: min_player,
            combat: false,
        };
    };
    // 7.
    finish(game, cx, unit, ctl, align, vis, t, best)
}

#[allow(clippy::too_many_arguments)]
fn finish<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    unit: UnitId,
    ctl: UnitId,
    align: u8,
    vis: Option<u32>,
    t: UnitId,
    d: i32,
) -> Search {
    if align != 2 {
        if let Some(c) = cx.store.control_mut(ctl) {
            c.flags |= flag::TARGET_SEEN;
        }
        // Step 7: the seen flag := (S == 0), only when step 2 loaded the record.
        if let Some(seen) = vis {
            cx.world.mark_seen(unit, u32::from(seen == 0));
        }
    }
    let combat = cx.world.in_melee_range(game, unit, t);
    Search {
        target: Some(t),
        distance: d,
        combat,
    }
}

fn apply(p: &mut TickParam, s: Search) {
    p.target = s.target;
    p.distance = s.distance;
    p.combat = s.combat;
}

/// Target mode 1: true when a target was found.
pub(super) fn find_mode1<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    unit: UnitId,
    p: &mut TickParam,
) -> bool {
    let s = main_search(game, cx, unit);
    apply(p, s);
    if s.target.is_some() {
        return true;
    }
    if cx.ai_state_set(unit) && cx.can_walk(unit) {
        wander(game, cx, unit, 5);
        return false;
    }
    if cx.world.collides(game, unit, 0x40) && cx.can_walk(unit) {
        wander(game, cx, unit, 5);
        return false;
    }
    let n = if s.distance >= 35 {
        25
    } else if s.distance >= 25 {
        s.distance - 10
    } else {
        10
    };
    idle(game, cx, unit, n);
    false
}

/// Target modes 4 and 5: the collision test **and**
/// the class can walk → wander 5; otherwise idle 20 without a mode change.
fn find_mode45<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    unit: UnitId,
    p: &mut TickParam,
) -> bool {
    let s = main_search(game, cx, unit);
    apply(p, s);
    if s.target.is_some() {
        return true;
    }
    if cx.world.collides(game, unit, 0x40) && cx.can_walk(unit) {
        wander(game, cx, unit, 5);
    } else {
        idle_keep_mode(game, cx, unit, 20);
    }
    false
}

/// Precheck B: target acquisition by the record's
/// target mode. True = stop.
pub fn precheck_b<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    unit: UnitId,
    p: &mut TickParam,
) -> bool {
    match cx.record(unit).target_mode {
        0 => false,
        1 => !find_mode1(game, cx, unit, p),
        2 => {
            let s = main_search(game, cx, unit);
            apply(p, s);
            false
        }
        4 => !find_mode45(game, cx, unit, p),
        5 => {
            find_mode45(game, cx, unit, p);
            false
        }
        _ => true,
    }
}

/// Precheck C: boss sound, teleport, special walk.
/// Only with a target. True = stop.
pub fn precheck_c<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    unit: UnitId,
    p: &mut TickParam,
) -> bool {
    let Some(target) = p.target else {
        return false;
    };
    // 1. Boss sound.
    let flags = cx.store.control(unit).map_or(0, |c| c.flags);
    let bossy = cx.world.is_unique(unit) || cx.world.is_boss(unit) || cx.world.class(unit) == 250;
    let target_is_player = game
        .lists
        .unit(target)
        .is_some_and(|e| e.ty == UnitType::Player);
    if bossy && p.distance < 20 && target_is_player && flags & flag::BOSS_SOUND_DONE == 0 {
        cx.world.play_sound(game, unit, 16, None);
        if let Some(c) = cx.store.control_mut(unit) {
            c.flags |= flag::BOSS_SOUND_DONE;
        }
        idle(game, cx, unit, 20);
        return true;
    }
    // 2. Teleport.
    if !cx.world.is_dead(unit) && flags & flag::MAY_TELEPORT != 0 && cx.chance(unit, 40) {
        let melee = cx.tables.monstats.get(p.class).is_some_and(|r| r.ismelee)
            || matches!(cx.base_class(unit), 10 | 345 | 557);
        let hurt = cx.world.life_percent(unit) < 30;
        if (hurt || (!melee && p.distance < 10)) && cx.chance(unit, 15) {
            if let Some((x, y, room)) = cx.world.find_spot(game, unit) {
                if !cx.world.in_town(game, room) {
                    if hurt && cx.chance(unit, 25) && !cx.world.has_state(unit, state::PREVENTHEAL)
                    {
                        let amount = cx.world.monster_level(unit).wrapping_mul(256);
                        cx.world.add_life(unit, amount);
                    }
                    use_skill(game, cx, unit, mode::ATTACK1, 184, ModeTarget::Point(x, y));
                    return true;
                }
            }
        }
    }
    // 3. Special walk.
    let is_melee = cx.tables.monstats.get(p.class).is_some_and(|r| r.ismelee);
    if is_melee && cx.can_walk(unit) && game.lists.unit(unit).and_then(|e| e.room()).is_some() {
        let lvl = cx.world.level_id(game, unit);
        let spc = usize::try_from(lvl)
            .ok()
            .and_then(|l| cx.tables.levels.get(l))
            .map_or(0, |r| i32::from(r.monspcwalk));
        if spc != 0 && spc < p.distance && !cx.world.can_reach_directly(game, unit, target) {
            match cx.world.special_walk_target(game, unit) {
                None => {
                    wander(game, cx, unit, 4);
                    return true;
                }
                Some((t, d)) => {
                    p.target = Some(t);
                    p.distance = d;
                }
            }
        }
    }
    false
}

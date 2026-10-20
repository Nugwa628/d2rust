//! The Act III thinks with the FrogDemon and FetishShaman
//! alternates. Brackets in the comments are the Normal difficulty values.

use crate::game::Game;
use crate::units::{UnitId, UnitType};

use super::bodies::{a1, a2, param, pct, roll, run_bonus, set_param};
use super::bodies2::{reinstall, GROUP_UDEAD};
use super::common::*;
use super::tactics::*;
use super::{
    delete_thinks, idle, idle_keep_mode_raw, idle_raw, mode, AiCommand, AiHost, Ctx, ModeTarget,
    TickParam,
};

/// One raw step read as `lo' % 3`.
fn three<W: AiHost + ?Sized>(cx: &mut Ctx<'_, W>, u: UnitId) -> i32 {
    (cx.world.seed(u).step() % 3) as i32
}

/// The unit a command names with its type in param 2 and its GUID in
/// param 1.
fn named_unit(game: &Game, k: &AiCommand) -> Option<UnitId> {
    let ty = UnitType::ALL
        .get(usize::try_from(k.params[2]).ok()?)
        .copied()?;
    game.lists.find_unit(ty, k.params[1] as u32)
}

/// State 12 (`inferno`), used by FetishShaman, Megademon, Diablo,
/// FrozenHorror and Nihlathak.
pub(super) const STATE_INFERNO: u16 = 12;

/// The level of the unit's `skill` entry (with bonus, owner −1); 1 for
/// no skill, no entry or a level < 1.
pub(super) fn skill_range<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, u: UnitId, skill: i32) -> i32 {
    if skill < 0 {
        return 1;
    }
    match cx.world.skill_level(u, skill, false) {
        Some(l) if l >= 1 => l,
        _ => 1,
    }
}

// ---- Mosquito -------------------------------------------------------

/// Mosquito (24).
pub fn mosquito<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let mut s = param(cx, u, 0);
    let mut n = param(cx, u, 1);
    // 1.
    if p.combat {
        // The draw is made first, always.
        if pct(cx, u) < cx.aip(p, 3) || n == 0 {
            set_param(cx, u, 1, n + 1);
            if cx.skill(p, 1).0 >= 0 && pct(cx, u) < cx.aip(p, 4) {
                skill_k(game, cx, u, p, 1, t);
            } else {
                a1(game, cx, u, t);
            }
            return;
        }
        if pct(cx, u) > 20 {
            idle(game, cx, u, 15);
            return;
        }
        s = 2;
        n = 0;
    }
    match s {
        // 2.
        0 => {
            set_velocity(cx, u, 13, 100, 0);
            walk_to(game, cx, u, t, 0);
        }
        // 3.
        1 => {
            set_velocity(cx, u, 0, 50, 0);
            wander(game, cx, u, 4);
            n += 1;
            if n > cx.aip(p, 5) {
                s = 0;
                n = 0;
            }
        }
        // 4.
        2 => {
            set_velocity(cx, u, 2, 100, 0);
            escape(game, cx, u, t, 10, true, false);
            s = 1;
            n = 0;
        }
        // 5.
        _ => {
            s = 0;
            n = 0;
            idle(game, cx, u, 10);
        }
    }
    set_param(cx, u, 0, s);
    set_param(cx, u, 1, n);
}

// ---- ThornHulk ------------------------------------------------------

/// The earliest positive expiry frame of the unit's type-1 timers
/// (timer lookup of type 1); 0 when none.
fn skill_timer(game: &Game, u: UnitId) -> i32 {
    game.timers
        .unit_timers(u)
        .into_iter()
        .filter(|&t| game.timers.event(t).is_some_and(|e| e.0 == 1))
        .filter_map(|t| game.timers.expire(t))
        .filter(|&f| f > 0)
        .min()
        .unwrap_or(0)
}

/// ThornHulk (27).
pub fn thorn_hulk<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    // The frenzy swing: mode 5 fixed, `Skill1` not tested (edge case 8).
    let swing = |game: &mut Game, cx: &mut Ctx<'_, W>| {
        let (s1, _) = cx.skill(p, 1);
        use_skill(game, cx, u, mode::ATTACK2, s1, target_of(t));
        let f = skill_timer(game, u);
        if f > game.frame {
            let n = f - game.frame + game.ticks(cx.aip(p, 5));
            idle_keep_mode_raw(game, cx, u, n);
        }
    };
    // 1.
    if !p.combat || t.is_none() {
        set_param(cx, u, 0, 0);
        walk_to_method13(game, cx, u, t, 7);
        return;
    }
    // 2.
    let f = param(cx, u, 0);
    if f >= 1 {
        swing(game, cx);
        set_param(cx, u, 0, f - 1);
        if f - 1 < 1 {
            set_param(cx, u, 1, 3);
        }
        return;
    }
    // 3.
    if pct(cx, u) >= cx.aip(p, 1) {
        if pct(cx, u) < cx.aip(p, 3) {
            circle(game, cx, u, t, 4, false);
        } else {
            idle(game, cx, u, 15);
        }
        return;
    }
    // 4.
    let c = param(cx, u, 1);
    if c < 1 && pct(cx, u) < cx.aip(p, 4) {
        swing(game, cx);
        set_param(cx, u, 0, cx.aip(p, 6));
        return;
    }
    // 5.
    set_param(cx, u, 1, c - 1);
    if pct(cx, u) < cx.aip(p, 2) {
        a2(game, cx, u, t);
    } else {
        a1(game, cx, u, t);
    }
}

// ---- ZakarumZealot --------------------------------------------------

/// Quest 21 (A3Q5 The Blackened Temple).
const QUEST_A3Q5: i32 = 21;

/// The act of the unit's level (levels `Act`; −1 without a row).
pub(super) fn level_act<W: AiHost + ?Sized>(game: &Game, cx: &Ctx<'_, W>, u: UnitId) -> i32 {
    let l = cx.world.level_id(game, u);
    usize::try_from(l)
        .ok()
        .and_then(|l| cx.tables.levels.get(l))
        .map_or(-1, |r| i32::from(r.act))
}

/// ZakarumZealot (48).
pub fn zakarum_zealot<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let v = run_bonus(cx, p);
    let l = cx.world.life_percent(u);
    let flee = |game: &mut Game, cx: &mut Ctx<'_, W>| {
        set_velocity(cx, u, 0, v, 0);
        escape(game, cx, u, t, 8, true, true)
    };
    // 1.
    if let Some(tt) = t {
        let mut q = Some(tt);
        // A monster without a minion owner keeps Q = T, which is not a
        // player: no flight.
        if is_monster(game, tt) {
            if let Some((0, guid)) = cx.world.owner_record(tt) {
                q = game.lists.find_unit(UnitType::Player, guid);
            }
        }
        if let Some(q) = q.filter(|&q| is_player(game, q)) {
            let d = cx.info.difficulty;
            if level_act(game, cx, u) == 2 && cx.world.quest_flag(q, d, QUEST_A3Q5, 0) {
                if !flee(game, cx) {
                    wander(game, cx, u, 6);
                }
                return;
            }
        }
    }
    // 2.
    if cx.ai_state_set(u) {
        if param(cx, u, 1) == 0 && l < cx.aip(p, 3) {
            set_param(cx, u, 0, 0);
            if flee(game, cx) {
                return;
            }
            set_param(cx, u, 1, 5);
        }
        let (x, y) = cx.world.position(u);
        if cx.world.point_collides(game, room_of(game, u), x, y, 0x40) {
            if p.combat {
                circle(game, cx, u, t, 4, false);
            } else {
                walk_to(game, cx, u, t, 7);
            }
            return;
        }
    }
    // 3.
    let c = param(cx, u, 1);
    if c != 0 {
        set_param(cx, u, 1, c - 1);
    }
    // 4.
    if !p.combat {
        set_param(cx, u, 0, 0);
        if pct(cx, u) < cx.aip(p, 4) {
            set_velocity(cx, u, 0, v, 0);
            run_to(game, cx, u, t, 0);
        } else {
            walk_to(game, cx, u, t, 7);
        }
        return;
    }
    // 5.
    if param(cx, u, 0) != 0 && pct(cx, u) >= cx.aip(p, 1) {
        set_param(cx, u, 0, 0);
        if pct(cx, u) < 80 {
            idle(game, cx, u, 10);
        } else {
            circle(game, cx, u, t, 4, false);
        }
        return;
    }
    // 6.
    set_param(cx, u, 0, 1);
    if pct(cx, u) < cx.aip(p, 2) {
        a2(game, cx, u, t);
    } else {
        a1(game, cx, u, t);
    }
}

// ---- ZakarumPriest --------------------------------------------------

/// zealot1, cantor1 (`BaseId` of the heal scan).
const ZEALOT1: i32 = 235;
const CANTOR1: i32 = 238;

/// ZakarumPriest (49).
pub fn zakarum_priest<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let frame = game.frame;
    let l = cx.world.life_percent(u);
    // 1.
    if p.combat || cx.ai_state_set(u) {
        let (s3, m3) = cx.skill(p, 3);
        if s3 >= 0 && l < 33 && frame > param(cx, u, 0) {
            set_param(cx, u, 0, game.after(4 * cx.aip(p, 5)));
            let k = if p.combat { 4 } else { 1 };
            let (ox, oy) = cx.world.position(u);
            let (tx, ty) = t.map_or((0, 0), |t| cx.world.position(t));
            let x = tx.wrapping_add(k * tx.wrapping_sub(ox));
            let y = ty.wrapping_add(k * ty.wrapping_sub(oy));
            // A failed check goes on to step 1.2; the cooldown stays spent.
            if cx.world.skill_check(game, u, s3, None, x, y) {
                use_skill(game, cx, u, m3, s3, ModeTarget::Point(x, y));
                return;
            }
        }
        if p.combat && cx.chance(u, cx.aip(p, 1)) {
            a1(game, cx, u, t);
            return;
        }
    }
    // 2. The last qualifying unit wins (edge case 1).
    let aip6 = cx.aip(p, 6);
    let max = aip6.wrapping_mul(aip6);
    let mut best = None;
    for v in scan_units(game, u) {
        if v != u
            && is_monster(game, v)
            && pairing(cx, u, v)
            && cx.world.alignment(v) != 2
            && !dying(cx, v)
            && matches!(unit_base(cx, v), ZEALOT1 | CANTOR1)
            && sq_dist(cx, u, v) <= max
            && cx.world.life_percent(v) <= 60
        {
            best = Some(v);
        }
    }
    // 3.
    if let Some(b) = best.filter(|_| cx.skill(p, 1).0 >= 0) {
        if pct(cx, u) < 25 {
            skill_k(game, cx, u, p, 1, Some(b));
            return;
        }
    }
    let blizzard = |game: &mut Game, cx: &mut Ctx<'_, W>| {
        if cx.skill(p, 4).0 >= 0 && frame > param(cx, u, 1) && pct(cx, u) < cx.aip(p, 2) {
            skill_k(game, cx, u, p, 4, t);
            set_param(cx, u, 1, game.after(cx.aip(p, 5)));
            return true;
        }
        false
    };
    let clear = t.is_some_and(|t| !cx.world.line_blocked(game, u, t));
    if clear {
        // 4.
        if cx.chance(u, cx.aip(p, 4)) {
            if blizzard(game, cx) {
                return;
            }
            if cx.skill(p, 2).0 >= 0 && frame > param(cx, u, 2) && pct(cx, u) < cx.aip(p, 3) {
                skill_k(game, cx, u, p, 2, t);
                set_param(cx, u, 2, game.after(20));
                return;
            }
        }
    } else if cx.chance(u, cx.aip(p, 1)) && blizzard(game, cx) {
        // 5.
        return;
    }
    // 6.
    if pct(cx, u) < 30 {
        circle(game, cx, u, t, 4, false);
    } else {
        idle(game, cx, u, 20);
    }
}

// ---- FrogDemon ------------------------------------------------------

/// "Surface": `Skill2` in `Sk2mode` at the unit itself, s := 2 (after a
/// successful land).
fn surface<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    skill_k(game, cx, u, p, 2, Some(u));
    set_param(cx, u, 2, 2);
}

/// FrogDemon (52) (target mode 5).
pub fn frog_demon<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let d = p.distance;
    let (s1, _) = cx.skill(p, 1);
    let (s2, _) = cx.skill(p, 2);
    match param(cx, u, 2) {
        // 1.
        0 => {
            if s1 >= 0 && d > 12 {
                skill_k(game, cx, u, p, 1, t);
                wait(game, cx, u, 8);
                set_param(cx, u, 2, 1);
                take_off(game, cx, u);
                return;
            }
            if s2 >= 0 && land(game, cx, u) {
                surface(game, cx, u, p);
                return;
            }
            let (x, y) = cx.world.position(u);
            if !cx.world.point_collides(game, room_of(game, u), x, y, 0xC01) {
                set_param(cx, u, 2, 2);
            }
            idle(game, cx, u, 12);
        }
        // 2.
        1 => {
            let n = param(cx, u, 1);
            if s2 >= 0
                && ((d < cx.aip(p, 8) && land(game, cx, u))
                    || (d < 20 && n > 64 && land(game, cx, u)))
            {
                surface(game, cx, u, p);
                return;
            }
            take_off(game, cx, u);
            wait(game, cx, u, 24);
            set_param(cx, u, 1, n + 1);
            set_param(cx, u, 2, 1);
        }
        // 3.
        _ => {
            if t.is_none() {
                wait(game, cx, u, 32);
                return;
            }
            if p.combat {
                if pct(cx, u) < cx.aip(p, 2) {
                    a2(game, cx, u, t);
                } else if pct(cx, u) < cx.aip(p, 1) {
                    a1(game, cx, u, t);
                } else if pct(cx, u) < cx.aip(p, 3) {
                    circle(game, cx, u, t, 3, false);
                } else {
                    wait(game, cx, u, cx.aip(p, 7));
                }
            } else if d < cx.aip(p, 6) {
                if pct(cx, u) < cx.aip(p, 5) {
                    a2(game, cx, u, t);
                } else if pct(cx, u) < cx.aip(p, 4) {
                    circle(game, cx, u, t, 4, false);
                } else {
                    wait(game, cx, u, cx.aip(p, 7));
                }
            } else if pct(cx, u) < cx.aip(p, 4) {
                circle(game, cx, u, t, 3, false);
            } else {
                move_steps(game, cx, u, t, false, 4);
            }
        }
    }
}

/// FrogDemon's alternate: no draws.
pub fn frog_demon_alt<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let st = cx.store.control(u).map_or(0, |c| c.special_state);
    if !matches!(st, 10 | 11) && cx.skill(p, 1).0 >= 0 && param(cx, u, 2) >= 2 {
        skill_k(game, cx, u, p, 1, p.target);
        set_param(cx, u, 2, 1);
        take_off(game, cx, u);
    } else {
        reinstall(game, cx, u);
        wait(game, cx, u, 1);
    }
}

// ---- FetishShaman ---------------------------------------------------

/// fetish1, fetishblow1 (`BaseId` of the fetish scan).
const FETISH1: i32 = 141;
const FETISHBLOW1: i32 = 396;

/// FetishShaman (65).
pub fn fetish_shaman<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let (s1, _) = cx.skill(p, 1);
    // 1.
    let r = skill_range(cx, u, s1);
    // 2.
    if s1 >= 0 && p.distance < r && !cx.world.has_state(u, STATE_INFERNO) {
        let ty = t.and_then(|t| type_of(game, t)).map_or(6, |ty| {
            UnitType::ALL.iter().position(|&a| a == ty).unwrap_or(6) as i32
        });
        let cmd = AiCommand {
            params: [1, guid_of(game, t), ty, 0, 0],
        };
        command_minions(game, cx, u, cmd);
        skill_k(game, cx, u, p, 1, t);
        return;
    }
    // 3.
    if cx.world.has_state(u, STATE_INFERNO) {
        cx.world.set_state(game, u, STATE_INFERNO, false);
    }
    // 4.
    let h = cx.aip(p, 2);
    let aip5 = cx.aip(p, 5);
    let max = aip5.wrapping_mul(aip5);
    let mut corpse: Option<(UnitId, i32)> = None;
    for v in scan_units(game, u) {
        if v == u
            || !is_monster(game, v)
            || cx.world.unit_flags(v) & 0x2 == 0
            || cx.world.has_state_group(v, GROUP_UDEAD)
            || !(h != 0 || minion_owner(game, cx, v) == Some(u))
        {
            continue;
        }
        let b = unit_base(cx, v);
        if !(b == FETISH1 || (h != 1 && b == FETISHBLOW1))
            || cx.world.alignment(v) != 0
            || !(h >= 3 || !(cx.world.is_unique(v) || cx.world.is_champion(v)))
        {
            continue;
        }
        let d = sq_dist(cx, u, v);
        if d > max {
            continue;
        }
        if cx.world.anim_mode(v) == mode::DEAD && d < corpse.map_or(0x7FFF_FFFF, |c| c.1) {
            corpse = Some((v, d));
        }
    }
    // 5. The check only after the draw passes; squared d against aip3.
    if let Some((c, d)) = corpse {
        let (s3, _) = cx.skill(p, 3);
        if cx.chance(u, cx.aip(p, 1)) && cx.world.skill_check(game, u, s3, Some(c), 0, 0) {
            let cmd = AiCommand {
                params: [14, guid_of(game, Some(c)), 1, 0, 0],
            };
            command_minions(game, cx, u, cmd);
            if d <= cx.aip(p, 3) {
                use_sequence_skill(game, cx, u, s3, ModeTarget::Unit(c));
            } else {
                wander_near(game, cx, u, c, 10);
            }
            return;
        }
    }
    // 6.
    if pct(cx, u) < cx.aip(p, 4) {
        circle(game, cx, u, t, 4, false);
    } else {
        idle(game, cx, u, 10);
    }
}

/// FetishShaman's alternate: no draws.
pub fn fetish_shaman_alt<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    if cx.world.has_state(u, STATE_INFERNO) {
        cx.world.set_state(game, u, STATE_INFERNO, false);
    }
    reinstall(game, cx, u);
    idle(game, cx, u, 1);
}

// ---- HighPriest -----------------------------------------------------

/// Hydra offsets by `mask(4)` (table).
const HYDRA: [(i32, i32); 4] = [(-5, -5), (5, -5), (5, 5), (-5, 5)];

/// HighPriest (85).
pub fn high_priest<W: AiHost + ?Sized>(
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
        // 1.1.
        if p.combat {
            if cx.chance(u, cx.aip(p, 1)) {
                set_param(cx, u, 0, 1);
                a1(game, cx, u, t);
            } else {
                escape(game, cx, u, t, 6, true, false);
            }
            return;
        }
        // 1.2.
        if cx.skill(p, 2).0 >= 0 && frame > param(cx, u, 1) && pct(cx, u) < cx.aip(p, 2) {
            // The callback does not exclude the scanner (open
            // question 6).
            let mut best = None;
            let mut best_life = 75;
            for v in scan_units(game, u) {
                if !is_monster(game, v)
                    || dying(cx, v)
                    || !pairing(cx, u, v)
                    || cx.world.alignment(v) != 0
                    || sq_dist(cx, u, v) > 2500
                {
                    continue;
                }
                let lp = cx.world.life_percent(v);
                if lp <= 75 && lp < best_life {
                    best = Some(v);
                    best_life = lp;
                }
            }
            if let Some(b) = best {
                set_param(cx, u, 1, game.after(cx.aip(p, 3)));
                skill_k(game, cx, u, p, 2, Some(b));
                return;
            }
        }
        // 1.3.
        let (s1, m1) = cx.skill(p, 1);
        if s1 >= 0 && frame > param(cx, u, 1) && d < cx.aip(p, 8) && pct(cx, u) < cx.aip(p, 4) {
            let k = cx.world.seed(u).mask(4) as usize;
            let (tx, ty) = t.map_or((0, 0), |t| cx.world.position(t));
            let (ox, oy) = HYDRA[k];
            // The request holds T and the hydra's point at once; the mode
            // set aims at T whenever T ≠ 0 and uses the point only for T =
            // 0 (the mode request record; not settled for every caller).
            let at = match t {
                Some(t) => ModeTarget::Unit(t),
                None => ModeTarget::Point(tx.wrapping_add(ox), ty.wrapping_add(oy)),
            };
            use_skill(game, cx, u, m1, s1, at);
            set_param(cx, u, 1, game.after(100));
            return;
        }
        // 1.4.
        let aip5 = cx.aip(p, 5);
        let miss = cx
            .tables
            .monstats
            .get(p.class)
            .map_or(-1, |r| i32::from(r.misss1 as i16));
        if aip5 > 0 && miss > 0 {
            if let Some(row) = cx.tables.missiles.get(miss as usize) {
                let range = i32::from(row.range as i16);
                if d < range - 2 && pct(cx, u) < aip5 {
                    mode_at(game, cx, u, mode::SKILL1, t);
                    return;
                }
            }
        }
        // 1.5.
        if pct(cx, u) < 80 {
            if d <= cx.aip(p, 8) {
                circle(game, cx, u, t, 3, false);
            } else {
                move_steps(game, cx, u, t, false, 6);
            }
            return;
        }
    }
    // 2.
    set_param(cx, u, 0, 1);
    if p.combat {
        // 3.
        if pct(cx, u) < cx.aip(p, 7) {
            mode_at(game, cx, u, mode::SKILL1, t);
        } else if pct(cx, u) < cx.aip(p, 6) {
            escape(game, cx, u, t, 6, true, false);
            set_param(cx, u, 0, 0);
        } else if pct(cx, u) < 90 {
            a1(game, cx, u, t);
        } else {
            idle(game, cx, u, 10);
        }
        return;
    }
    // 4.
    if d < 6 && pct(cx, u) < cx.aip(p, 7) {
        mode_at(game, cx, u, mode::SKILL1, t);
    } else if pct(cx, u) < cx.aip(p, 6) {
        set_param(cx, u, 0, 0);
        idle(game, cx, u, 10);
    } else if pct(cx, u) < 70 {
        walk_to(game, cx, u, t, 7);
    } else {
        wander(game, cx, u, 12);
    }
}

// ---- FetishBlowgun --------------------------------------------------

/// FetishBlowgun (96).
pub fn fetish_blowgun<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let d = p.distance;
    // 1.
    if let Some(k) = current_command(cx, u) {
        let named = named_unit(game, &k);
        match (k.params[0], named) {
            (1, Some(x)) => {
                set_param(cx, u, 0, 0);
                set_param(cx, u, 1, 0);
                a1(game, cx, u, Some(x));
                free_current_command(cx, u);
                return;
            }
            (14, Some(x)) => {
                set_param(cx, u, 0, 0);
                set_param(cx, u, 1, 0);
                set_velocity(cx, u, 13, 50, 0);
                walk_to(game, cx, u, Some(x), 0);
                free_current_command(cx, u);
                return;
            }
            _ => free_current_command(cx, u),
        }
    }
    // 2, 3.
    if !p.combat && d > cx.aip(p, 1) {
        set_velocity(cx, u, 0, 50, 0);
        wander_near_opt(game, cx, u, t, 6);
        return;
    }
    if (p.combat || d < 6) && pct(cx, u) < cx.aip(p, 2) {
        set_param(cx, u, 0, 2);
    }
    // 4.
    let (s, _, _) = cx.world.secondary_target(game, u, cx.store);
    let n = param(cx, u, 1);
    match param(cx, u, 0) {
        // 5.
        0 => {
            let n = n + 1;
            set_param(cx, u, 1, n);
            if three(cx, u) + 3 < n {
                set_param(cx, u, 0, 1);
                set_param(cx, u, 1, 0);
            }
            if s.is_some() {
                a1(game, cx, u, s);
            } else {
                circle(game, cx, u, t, 4, false);
            }
        }
        // 6.
        1 => {
            set_param(cx, u, 0, 0);
            set_param(cx, u, 1, 0);
            set_velocity(cx, u, 0, 50, 0);
            if !circle(game, cx, u, t, 6, true) {
                idle(game, cx, u, 10);
            }
        }
        // 7.
        2 => {
            if d <= 12 {
                set_velocity(cx, u, 2, 50, 0);
                if !escape(game, cx, u, t, 14, true, false) {
                    // Also with S = 0 (edge case 4).
                    a1(game, cx, u, s);
                    set_param(cx, u, 0, 0);
                    set_param(cx, u, 1, 0);
                }
            } else if !(pct(cx, u) < 20 && circle(game, cx, u, t, 4, true)) {
                set_param(cx, u, 0, 0);
                set_param(cx, u, 1, 0);
                idle(game, cx, u, 10);
            }
        }
        // 8.
        _ => idle(game, cx, u, 10),
    }
}

// ---- WillOWisp -----------------------------------------------------

/// willowisp1 (`BaseId` of the gather find).
const WILLOWISP1: i32 = 118;

/// Ritual stand points A[s − 6][slot − 1].
const RITUAL_A: [[(i32, i32); 4]; 7] = [
    [(-10, -3), (1, -5), (6, 0), (4, 10)],
    [(-9, -2), (-6, -11), (-2, -5), (6, 1)],
    [(0, 13), (-12, 0), (7, 5), (-3, -11)],
    [(-15, -3), (-6, -3), (-4, -5), (-6, -13)],
    [(-12, -2), (-6, 2), (-5, 5), (1, 11)],
    [(-13, -6), (-8, -11), (-7, 0), (1, 8)],
    [(-5, -8), (-8, 1), (1, -6), (6, 9)],
];
/// Ritual cast points B[s − 6][slot − 1].
const RITUAL_B: [[(i32, i32); 4]; 7] = [
    [(1, -5), (6, 0), (4, 10), (-10, -3)],
    [(1, 8), (-9, -2), (-5, 4), (1, 8)],
    [(-12, 0), (7, 5), (-3, -11), (11, -2)],
    [(7, 9), (-15, -3), (-6, -13), (9, 7)],
    [(-5, -7), (-5, -7), (9, 0), (-12, -2)],
    [(-8, -11), (-2, -6), (-2, -6), (-13, -6)],
    [(-8, 1), (1, -6), (6, 9), (0, 0)],
];
/// Baptism stand points C[slot − 1] and cast points E[slot − 1].
const BAPTISM_C: [(i32, i32); 5] = [(-5, -5), (3, -5), (6, 3), (3, 6), (-5, 3)];
const BAPTISM_E: [(i32, i32); 5] = [(6, 3), (3, 6), (-5, 3), (-5, -5), (3, -5)];

/// A table entry by slot (1-based). Slots ≤ 0 would read below the
/// tables in the base game; they are unreachable and asserted (open question
/// 7).
fn slot<const N: usize>(t: &[(i32, i32); N], r: i32) -> (i32, i32) {
    let i = usize::try_from(r - 1).expect("ritual slot ≤ 0 (ai-bodies-3.md OQ7)");
    t.get(i).copied().unwrap_or((0, 0))
}

/// "Approach P": true = near.
fn approach<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    x: i32,
    y: i32,
) -> bool {
    if path_distance(cx, u, x, y) > 3 {
        if pct(cx, u) < 34 {
            wander(game, cx, u, 4);
        } else {
            mode_point_raw(game, cx, u, mode::WALK, x, y);
        }
        return false;
    }
    true
}

/// WillOWisp (25).
pub fn will_o_wisp<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let frame = game.frame;
    let (p67, p337) = (game.ticks(67), game.ticks(337));
    let m67 = frame % p67;
    let m337 = frame % p337;
    let mut s = param(cx, u, 0);
    let n = param(cx, u, 1);
    let r = param(cx, u, 2);
    // T = 0 never reaches the ritual points (target mode 1, open question
    // 7): asserted.
    let (tx, ty) = cx
        .world
        .position(t.expect("WillOWisp think without a target (ai-bodies-3.md OQ7)"));
    // 0. The draw only when the first four hold.
    if let Some(tt) = t {
        let dl = i32::from(cx.info.difficulty) + 2;
        if frame > r && sq_dist(cx, u, tt) < 1024 && s < 4 && roll(cx, u, 1000) <= dl {
            s = 4;
            set_param(cx, u, 0, 4);
        }
    }
    // 1.
    if s == 4 {
        let found: Vec<UnitId> = cx
            .world
            .wisp_find(game, u)
            .into_iter()
            .filter(|&v| is_monster(game, v) && !dying(cx, v) && unit_base(cx, v) == WILLOWISP1)
            .collect();
        let shape = match found.len() {
            4 => 6,
            5 => 5,
            _ => {
                set_param(cx, u, 0, 2);
                idle(game, cx, u, 8);
                return;
            }
        };
        for (i, &v) in found.iter().enumerate() {
            if let Some(c) = cx.store.control_mut(v) {
                c.params = [shape, 0, i as i32 + 1];
            }
            idle_raw(game, cx, v, p337 - m337);
        }
        idle_raw(game, cx, u, p337 - m337);
        return;
    }
    // 2.
    if s >= 6 {
        if r > 4 {
            set_param(cx, u, 0, 2);
            idle(game, cx, u, 8);
            return;
        }
        let i = (s - 6) as usize;
        let (ax, ay) = RITUAL_A.get(i).map_or((0, 0), |a| slot(a, r));
        if !approach(game, cx, u, tx + ax, ty + ay) {
            return;
        }
        if m67 != 0 {
            idle_raw(game, cx, u, p67 - m67);
            return;
        }
        if n < 3 {
            let (bx, by) = RITUAL_B.get(i).map_or((0, 0), |b| slot(b, r));
            if (bx, by) == (0, 0) {
                idle_raw(game, cx, u, p67);
            } else {
                mode_point_raw(game, cx, u, 7, tx + bx, ty + by);
            }
            set_param(cx, u, 1, n + 1);
            return;
        }
        set_param(cx, u, 0, s + 1);
        set_param(cx, u, 1, 0);
        if s + 1 > 12 {
            set_param(cx, u, 0, 2);
            set_param(cx, u, 2, game.after(1800));
        }
        idle_raw(game, cx, u, p337 - m337);
        // After every completed shape (edge case 7).
        if let Some(tt) = t.filter(|&tt| is_player(game, tt) && !cx.world.is_dead(tt)) {
            let v = 50 * (i32::from(cx.info.difficulty) + 1);
            cx.world.wisp_buff(game, tt, v, game.after(1_728_000));
        }
        return;
    }
    // 3.
    if s == 5 {
        if r > 5 {
            set_param(cx, u, 0, 2);
            idle(game, cx, u, 8);
            return;
        }
        let (cx_, cy) = slot(&BAPTISM_C, r);
        if !approach(game, cx, u, tx + cx_, ty + cy) {
            return;
        }
        if m67 != 0 {
            idle_raw(game, cx, u, p67 - m67);
            return;
        }
        if n >= 3 {
            set_param(cx, u, 0, 2);
            set_param(cx, u, 2, game.after(1800));
            idle_raw(game, cx, u, p337 - m337);
            return;
        }
        let (ex, ey) = slot(&BAPTISM_E, r);
        if (ex, ey) == (0, 0) {
            idle_raw(game, cx, u, p337 - m337);
        } else {
            mode_point_raw(game, cx, u, 7, tx + ex, ty + ey);
        }
        set_param(cx, u, 1, n + 1);
        return;
    }
    // 4.
    if s == 1 {
        if n <= 0 && p.combat {
            mode_at(game, cx, u, mode::SKILL1, None);
            set_param(cx, u, 0, 3);
            return;
        }
        if n <= 0 && pct(cx, u) < cx.aip(p, 1) {
            mode_at(game, cx, u, mode::SKILL1, None);
            set_param(cx, u, 0, 2);
            return;
        }
        set_param(cx, u, 1, n - 1);
        set_param(cx, u, 0, 1);
        if pct(cx, u) < cx.aip(p, 3) {
            walk_to(game, cx, u, t, 0);
        } else {
            wander(game, cx, u, 6);
        }
        return;
    }
    // 5.
    if !p.combat {
        if s == 2 || pct(cx, u) < cx.aip(p, 1) {
            mode_at(game, cx, u, 7, t);
            set_param(cx, u, 0, 0);
            return;
        }
    } else if s == 3 || pct(cx, u) < cx.aip(p, 2) {
        a1(game, cx, u, t);
        set_param(cx, u, 0, 0);
        return;
    }
    set_param(cx, u, 0, 1);
    if pct(cx, u) < cx.aip(p, 3) {
        walk_to(game, cx, u, t, 0);
    } else {
        wander(game, cx, u, 4);
    }
    set_param(cx, u, 1, 3);
}

// ---- Mephisto ------------------------------------------------------

/// The pick (callback): the lowest-life unit
/// (stat 6 >> 8, strictly lower) that is not the scanner, not dead, has
/// unit flag 0x4, is hostile and within squared distance 1024; taken when
/// a draw < aip2 passes, else T (the nearest-player branch is dead,
/// edge case 6).
fn mephisto_pick<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) -> Option<UnitId> {
    let mut lowest: Option<(UnitId, i32)> = None;
    for v in scan_units(game, u) {
        if v == u
            || cx.world.is_dead(v)
            || cx.world.unit_flags(v) & 0x4 == 0
            || !cx.world.hostile(game, u, v)
            || sq_dist(cx, u, v) > 1024
        {
            continue;
        }
        let l = life(cx, v) >> 8;
        if lowest.is_none_or(|b| l < b.1) {
            lowest = Some((v, l));
        }
    }
    match lowest {
        Some((v, _)) if pct(cx, u) < cx.aip(p, 2) => Some(v),
        _ => p.target,
    }
}

/// Mephisto (50).
pub fn mephisto<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let d = p.distance;
    let l = cx.world.life_percent(u);
    let k = ((100 - l) / 5).max(0);
    let mut s = param(cx, u, 2);
    let mut roam = false;
    // 1, 2.
    if p.combat {
        if pct(cx, u) > cx.aip(p, 1) {
            s = 3;
        } else {
            let v = three(cx, u) + 3;
            set_param(cx, u, 0, v);
            s = 2;
        }
    } else if d > 20 {
        s = 4;
    } else if s != 0 {
        roam = s as u32 > 4;
    } else if l <= 20 && d < 5 && pct(cx, u) < 40 {
        s = 1;
    } else if pct(cx, u) < k + 50 {
        let v = three(cx, u) + 3;
        set_param(cx, u, 0, v);
        s = 2;
    } else {
        roam = true;
    }
    if roam {
        // 4.
        s = 0;
        if pct(cx, u) >= 65 {
            idle(game, cx, u, 10);
        } else if pct(cx, u) < 65 || d > 5 {
            circle(game, cx, u, t, 4, false);
        } else {
            set_velocity(cx, u, 0, 0, 4);
            walk_to(game, cx, u, t, 7);
        }
        set_param(cx, u, 2, s);
        return;
    }
    // 3.
    let mut case = s;
    if case == 1 {
        s = 0;
        set_velocity(cx, u, 0, 50, 0);
        if escape(game, cx, u, t, 8, true, false) {
            set_param(cx, u, 2, s);
            return;
        }
        if p.combat {
            skill_k(game, cx, u, p, 3, t);
            set_param(cx, u, 2, s);
            return;
        }
        case = 2;
    }
    match case {
        0 => {
            s = 4;
            idle(game, cx, u, 5);
        }
        2 => {
            let v = param(cx, u, 0) - 1;
            set_param(cx, u, 0, v);
            if v == 0 {
                s = 0;
            }
            let c = param(cx, u, 1);
            if c == 0 {
                set_param(cx, u, 1, 2);
                if !circle(game, cx, u, t, 3, true) {
                    wander(game, cx, u, 12);
                }
            } else {
                set_param(cx, u, 1, c + 1);
                let x = mephisto_pick(game, cx, u, p);
                let n = (1..=8).take_while(|&i| cx.skill(p, i).0 >= 0).count() as i32;
                if n == 0 {
                    // The closing draw follows the wander (case 2, open
                    // question 8).
                    wander(game, cx, u, 6);
                    if pct(cx, u) < 50 - k {
                        set_param(cx, u, 1, 0);
                    }
                } else {
                    let q = 100 / n;
                    let r = pct(cx, u);
                    let hard = cx.info.difficulty > 0;
                    let blocked = x.is_some_and(|x| cx.world.line_blocked(game, u, x));
                    let kk = if hard && (blocked || d > 30) {
                        6
                    } else if d < 15 && r < q && hard {
                        5
                    } else if r < 2 * q {
                        4
                    } else if r < 3 * q {
                        2
                    } else {
                        1
                    };
                    skill_k(game, cx, u, p, kk, x);
                    if pct(cx, u) < 50 - k {
                        set_param(cx, u, 1, 0);
                    }
                }
            }
        }
        3 => {
            s = 0;
            if pct(cx, u) < k + 80 {
                if pct(cx, u) < 80 - k {
                    a1(game, cx, u, t);
                } else {
                    skill_k(game, cx, u, p, 3, t);
                }
            } else {
                set_velocity(cx, u, 0, 50, 0);
                if !circle(game, cx, u, t, 3, true) {
                    wander(game, cx, u, 12);
                }
            }
        }
        _ => {
            // 4.
            s = 0;
            set_velocity(cx, u, 0, 50, 0);
            if !wander_near_opt(game, cx, u, t, 6) {
                delete_thinks(game, u);
                if !radius(game, cx, u, t, 12, 6) {
                    wander(game, cx, u, 12);
                }
            }
        }
    }
    set_param(cx, u, 2, s);
}

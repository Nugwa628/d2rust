//! The Act V thinks with their init and alternate functions and
//! the Baal pick, choice, execution and clone. Brackets in the
//! comments are the Normal difficulty values.

use crate::game::Game;
use crate::units::{UnitId, UnitType};

use super::bodies::{a1, a2, param, pct, roll, set_param};
use super::bodies2::{glow, reinstall};
use super::bodies3::STATE_INFERNO;
use super::bodies4::{
    boss_pick, home_point, portal_of, record_difficulty, special_caster, weighted, Cull, Home,
    Score,
};
use super::common::*;
use super::tactics::*;
use super::{
    delete_thinks, idle, idle_raw, install, mode, request_mode_keep_skill, state, AiHost, Ctx,
    ModeTarget, QuestCall, TickParam, UnitRef,
};

// ---- Minion ---------------------------------------------------------

/// Minion (116).
pub fn minion<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    let mut target = p.target;
    let mut combat = p.combat;
    let mut ordered = false;
    // 1.
    if let Some(k) = current_command(cx, u) {
        let v = (k.params[0] == 1 && game.frame < k.params[3])
            .then(|| {
                let ty = UnitType::ALL
                    .get(usize::try_from(k.params[1]).ok()?)
                    .copied()?;
                game.lists.find_unit(ty, k.params[2] as u32)
            })
            .flatten()
            .filter(|&v| !cx.world.is_dead(v));
        match v {
            Some(v) => {
                ordered = true;
                if Some(v) != p.target {
                    target = Some(v);
                    combat = cx.world.in_melee_range(game, u, v);
                }
            }
            None => free_current_command(cx, u),
        }
    }
    // 2.
    if !combat {
        if !ordered && pct(cx, u) >= cx.aip(p, 3) {
            idle(game, cx, u, cx.aip(p, 4));
        } else {
            walk_to(game, cx, u, target, 0);
        }
        return;
    }
    // 3. aip2 is both the stall and the A2 chance (edge case 2).
    if !ordered && pct(cx, u) >= cx.aip(p, 1) {
        idle(game, cx, u, cx.aip(p, 2));
    } else if pct(cx, u) < cx.aip(p, 2) {
        a2(game, cx, u, target);
    } else {
        a1(game, cx, u, target);
    }
}

// ---- Imp ------------------------------------------------------------

/// imp1..imp4: the rows the Imp reads its aips from (edge case 1).
const IMP_ROWS: [i32; 4] = [492, 493, 494, 495];

/// Imp's init: m := −1.
pub fn imp_init<W: AiHost + ?Sized>(cx: &mut Ctx<'_, W>, u: UnitId) {
    set_param(cx, u, 0, -1);
}

/// Imp (122). A missing row I1..I4 is a null read in
/// the base game; live data has all four, so they are asserted.
pub fn imp<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    let t = p.target;
    assert!(
        cx.tables.monstats.len() > IMP_ROWS[3] as usize,
        "Imp think without monstats rows 492..495 (ai-bodies-5.md OQ4)"
    );
    let i = |cx: &Ctx<'_, W>, row: usize, n: usize| cx.class_aip(IMP_ROWS[row - 1], n);
    let (s1, m1) = cx.skill(p, 1);
    // 1.
    let m = param(cx, u, 0);
    if m != -1 && cx.world.alignment(u) == 0 {
        let b = game
            .lists
            .find_unit(UnitType::Monster, m as u32)
            .filter(|&b| {
                !cx.world.is_dead(b)
                    && cx.world.owner(game, b).is_none()
                    && cx.world.alignment(b) == 0
            });
        match b {
            None => set_param(cx, u, 0, -1),
            Some(b) => {
                let r = i(cx, 2, 1);
                if sq_dist(cx, u, b) > r.wrapping_mul(r) {
                    walk_to(game, cx, u, Some(b), 0);
                    return;
                }
                if s1 >= 0 {
                    use_skill(game, cx, u, m1, s1, ModeTarget::Unit(b));
                    return;
                }
            }
        }
    }
    let teleport = |game: &mut Game, cx: &mut Ctx<'_, W>| {
        let r = i(cx, 1, 2);
        teleport_in_range(game, cx, u, r, s1, m1);
    };
    // 2.
    if p.combat {
        if s1 >= 0 && cx.world.life_percent(u) < i(cx, 1, 1) {
            teleport(game, cx);
            return;
        }
        if pct(cx, u) < i(cx, 3, 2) && escape(game, cx, u, t, 5, true, false) {
            return;
        }
    }
    // 3.
    if s1 >= 0 && pct(cx, u) < i(cx, 1, 3) {
        teleport(game, cx);
        return;
    }
    // 4.
    if p.distance < i(cx, 3, 1) && pct(cx, u) < i(cx, 3, 2) {
        escape(game, cx, u, t, 5, false, false);
        return;
    }
    // 5. The "else" is the distance test E ≥ I3.aip3; a failed I3 draw
    // goes to step 6 (open question 4).
    if cx.skill(p, 4).0 >= 0 {
        if let (Some(s), e, _) = cx.world.secondary_target(game, u, cx.store) {
            let fire = if e < i(cx, 3, 3) {
                pct(cx, u) < i(cx, 3, 4)
            } else {
                e < i(cx, 4, 3) && pct(cx, u) < i(cx, 4, 4)
            };
            if fire {
                skill_k(game, cx, u, p, 4, Some(s));
                return;
            }
        }
    }
    // 6.
    if pct(cx, u) < 33 {
        move_steps(game, cx, u, t, false, 4);
    } else if pct(cx, u) < 20 {
        wander(game, cx, u, 8);
    } else {
        idle(game, cx, u, 10);
    }
}

// ---- Succubus, SuccubusWitch -------------------------------------

/// "Cursed": an active stat list with list flag 0x20.
fn cursed<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, t: UnitId) -> bool {
    cx.world.has_list_flag(t, 0x20)
}

/// The curse tests of the think's step 1 (skill tests > 0); true on a cast.
fn curse_tests<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
    t: UnitId,
) -> bool {
    let k = if cx.skill(p, 1).0 > 0 && cx.world.life_percent(t) >= cx.aip(p, 7) {
        1
    } else if cx.skill(p, 2).0 > 0 && cx.world.life_percent(u) <= cx.aip(p, 8) {
        2
    } else if cx.skill(p, 3).0 > 0
        && (cx.world.max_mana(t) < cx.world.max_life(t) || !is_player(game, t))
    {
        3
    } else if cx.skill(p, 4).0 > 0 && is_player(game, t) {
        4
    } else {
        return false;
    };
    skill_k(game, cx, u, p, k, Some(t));
    true
}

/// Succubus (118).
pub fn succubus<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    // 1.
    if let Some(tt) = t {
        if !cursed(cx, tt)
            && p.distance < cx.aip(p, 4)
            && cx.chance(u, cx.aip(p, 3))
            && curse_tests(game, cx, u, p, tt)
        {
            return;
        }
    }
    // 2.
    if p.combat {
        if cx.chance(u, cx.aip(p, 1)) {
            a1(game, cx, u, t);
        } else {
            idle(game, cx, u, cx.aip(p, 5));
        }
        return;
    }
    // 3.
    let aip8 = cx.aip(p, 8);
    if cx.skill(p, 5).0 > 0 && aip8 > 0 {
        if let (Some(s), _, _) = cx.world.secondary_target(game, u, cx.store) {
            if pct(cx, u) < aip8 {
                skill_k(game, cx, u, p, 5, Some(s));
                return;
            }
        }
    }
    // 4.
    if cx.chance(u, cx.aip(p, 2)) {
        walk_to(game, cx, u, t, 0);
    } else {
        idle(game, cx, u, cx.aip(p, 6));
    }
}

/// SuccubusWitch (119).
pub fn succubus_witch<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let mut e = p.distance;
    let aip4 = cx.aip(p, 4);
    let flee = |game: &mut Game, cx: &mut Ctx<'_, W>| {
        escape(game, cx, u, t, i32::from(aip4 as u8), true, false)
    };
    // 1.
    if let Some(tt) = t {
        if !cursed(cx, tt)
            && p.distance < aip4
            && cx.chance(u, cx.aip(p, 3))
            && curse_tests(game, cx, u, p, tt)
        {
            return;
        }
    }
    // 2.
    if p.combat {
        if pct(cx, u) < cx.aip(p, 3) && flee(game, cx) {
            return;
        }
        if pct(cx, u) < cx.aip(p, 1) {
            a1(game, cx, u, t);
        } else {
            idle(game, cx, u, cx.aip(p, 6));
        }
        return;
    }
    // 3.1.
    // The second draw only when S was found.
    let (s5, _) = cx.skill(p, 5);
    let aip8 = cx.aip(p, 8);
    if s5 >= 0 && aip8 > 0 && pct(cx, u) < cx.aip(p, 5) {
        let (s, se, _) = cx.world.secondary_target(game, u, cx.store);
        e = se;
        if let Some(s) = s {
            if pct(cx, u) < aip8 {
                skill_k(game, cx, u, p, 5, Some(s));
                return;
            }
        }
    }
    // 3.2.
    if e < aip4 && pct(cx, u) < cx.aip(p, 3) && flee(game, cx) {
        return;
    }
    // 3.3.
    if s5 < 0 {
        if let (Some(s), _, _) = cx.world.secondary_target(game, u, cx.store) {
            if pct(cx, u) < cx.aip(p, 5) {
                mode_at(game, cx, u, mode::SKILL2, Some(s));
                return;
            }
        }
    }
    // 3.4.
    if pct(cx, u) < cx.aip(p, 2) {
        walk_to(game, cx, u, t, 0);
    } else if !(pct(cx, u) < 50 && circle(game, cx, u, t, 6, true)) {
        idle(game, cx, u, cx.aip(p, 6));
    }
}

// ---- BloodLord, DeathMauler -------------------------------------

/// BloodLord (125).
pub fn blood_lord<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    if !p.combat {
        if cx.chance(u, cx.aip(p, 2)) {
            walk_to(game, cx, u, t, 0);
        } else {
            idle(game, cx, u, cx.aip(p, 4));
        }
    } else if cx.chance(u, cx.aip(p, 1)) {
        if pct(cx, u) < cx.aip(p, 3) {
            let (s1, _) = cx.skill(p, 1);
            use_skill(game, cx, u, mode::ATTACK2, s1, target_of(t));
        } else {
            a1(game, cx, u, t);
        }
    } else {
        idle(game, cx, u, cx.aip(p, 4));
    }
}

/// DeathMauler (130).
pub fn death_mauler<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    if p.combat {
        if cx.chance(u, cx.aip(p, 1)) {
            a1(game, cx, u, t);
        } else {
            idle(game, cx, u, 15);
        }
    } else if cx.skill(p, 1).0 >= 0 && p.distance < cx.aip(p, 3) && pct(cx, u) < cx.aip(p, 4) {
        skill_k(game, cx, u, p, 1, t);
    } else if pct(cx, u) < cx.aip(p, 2) {
        walk_to(game, cx, u, t, 0);
    } else {
        idle(game, cx, u, 15);
    }
}

// ---- Overseer -------------------------------------------------------

/// minion1 (`BaseId` of the Overseer scan).
const MINION1: i32 = 453;
/// State 141 `bloodlust`.
const STATE_BLOODLUST: u16 = 141;

/// The minion scan (callback): (hurt, whip, n)
/// with the hurt radius², whip radius² and hurt life percent of the arg.
fn minion_scan<W: AiHost + ?Sized>(
    game: &Game,
    cx: &Ctx<'_, W>,
    u: UnitId,
    hurt_max: i32,
    whip_max: i32,
    hurt_pct: i32,
) -> (Option<UnitId>, Option<UnitId>, i32) {
    let (mut hurt, mut whip, mut n) = (None, None, 0);
    for v in scan_units(game, u) {
        if v == u
            || !is_monster(game, v)
            || unit_base(cx, v) != MINION1
            || dying(cx, v)
            || cx.world.hostile(game, u, v)
            || cx.world.has_state(v, STATE_BLOODLUST)
        {
            continue;
        }
        n += 1;
        let d = sq_dist(cx, u, v);
        if hurt.is_none() && d <= hurt_max && cx.world.life_percent(v) < hurt_pct {
            hurt = Some(v);
        }
        if whip.is_none() && d <= whip_max && !cx.world.is_unique(v) {
            whip = Some(v);
        }
    }
    (hurt, whip, n)
}

/// Overseer (120).
pub fn overseer<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let d = p.distance;
    let frame = game.frame;
    // 1.
    cx.world.quest_call(game, u, QuestCall::Shenk);
    // 2.
    let v = cx
        .world
        .target_unit(game, u)
        .filter(|&v| match type_of(game, v) {
            Some(UnitType::Player) => !matches!(cx.world.anim_mode(v), 0 | 17),
            Some(UnitType::Monster) => !dying(cx, v) && cx.world.hostile(game, u, v),
            _ => false,
        });
    let (s1, _) = cx.skill(p, 1);
    // 3.
    if let Some(v) = v {
        if s1 >= 0 && cx.ai_state_set(u) && frame > param(cx, u, 0) && has_skill(cx, u, s1) {
            skill_k(game, cx, u, p, 1, Some(v));
            set_param(cx, u, 0, game.after(cx.aip(p, 1)));
            return;
        }
    }
    // 4.
    // A draw ≥ aip6 with C goes on to step 5.
    if p.combat && pct(cx, u) < cx.aip(p, 6) {
        if pct(cx, u) >= cx.aip(p, 7) {
            a1(game, cx, u, t);
        } else {
            a2(game, cx, u, t);
        }
        return;
    }
    // 5.
    let (hurt, whip, n) = minion_scan(game, cx, u, 576, 400, 50);
    // 6.
    let (s2, _) = cx.skill(p, 2);
    if let Some(h) = hurt.filter(|_| s2 >= 0) {
        if pct(cx, u) < cx.aip(p, 2) && has_skill(cx, u, s2) {
            skill_k(game, cx, u, p, 2, Some(h));
            return;
        }
    }
    // 7.
    let (s3, _) = cx.skill(p, 3);
    if let Some(w) = whip.filter(|_| s3 != 0) {
        if pct(cx, u) < cx.aip(p, 3) && cx.world.alignment(u) != 2 && has_skill(cx, u, s3) {
            skill_k(game, cx, u, p, 3, Some(w));
            return;
        }
    }
    // 8.
    if n == 0 {
        if pct(cx, u) >= 60 {
            idle(game, cx, u, 10);
        } else {
            walk_to(game, cx, u, t, 0);
        }
        return;
    }
    // 9.
    let (aip4, aip5) = (cx.aip(p, 4), cx.aip(p, 5));
    if d < aip4 - aip5 {
        escape(game, cx, u, t, i32::from((aip4 - d) as u8), false, false);
    } else if d <= aip4 + aip5 {
        if pct(cx, u) < 50 {
            circle(game, cx, u, t, 5, false);
        } else {
            idle(game, cx, u, 10);
        }
    } else {
        walk_to(game, cx, u, t, 0);
    }
}

// ---- ReanimatedHorde ------------------------------------------------

/// ReanimatedHorde (114).
pub fn reanimated_horde<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let d = p.distance;
    // 1.
    cx.world.set_unit_flag(u, FLAG_LANDED);
    // 2.
    if p.combat {
        if cx.chance(u, cx.aip(p, 1)) {
            a1(game, cx, u, t);
        } else {
            idle(game, cx, u, cx.aip(p, 2));
        }
        return;
    }
    // 3. The draw is the `roll(100)` helper.
    let direct = t.is_some_and(|tt| cx.world.can_reach_directly(game, u, tt));
    if cx.skill(p, 2).0 >= 0
        && direct
        && d < cx.aip(p, 3)
        && 5 < d
        && roll(cx, u, 100) < cx.aip(p, 4)
    {
        skill_k(game, cx, u, p, 2, t);
        return;
    }
    // 4.
    if pct(cx, u) < cx.aip(p, 5) {
        walk_to(game, cx, u, t, 0);
    } else if pct(cx, u) < cx.aip(p, 6) {
        radius(game, cx, u, t, 4, 0);
    } else {
        idle(game, cx, u, cx.aip(p, 7));
    }
}

// ---- ClawViperEx ----------------------------------------------------

/// ClawViperEx (142).
pub fn claw_viper_ex<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let g = glow(cx.aip(p, 6));
    // 1.
    if let Some(st) = g.filter(|_| param(cx, u, 0) != 0) {
        cx.world.set_state(game, u, st, false);
    }
    // 2.
    if p.combat {
        if cx.chance(u, cx.aip(p, 3)) {
            a2(game, cx, u, t);
        } else {
            idle(game, cx, u, cx.aip(p, 5));
        }
        return;
    }
    // 3.
    if p.distance < cx.aip(p, 7) && cx.chance(u, cx.aip(p, 4)) {
        if game.frame > param(cx, u, 1) {
            a1(game, cx, u, t);
            set_param(cx, u, 1, game.after(cx.aip(p, 8)));
        } else {
            idle(game, cx, u, cx.aip(p, 5));
        }
        return;
    }
    // 4.
    let (s1, _) = cx.skill(p, 1);
    if s1 >= 0 && p.distance < cx.aip(p, 2) && cx.chance(u, cx.aip(p, 1)) {
        let (tx, ty) = t.map_or((0, 0), |t| cx.world.position(t));
        if cx.world.skill_check(game, u, s1, t, tx, ty) {
            skill_k(game, cx, u, p, 1, t);
            if let Some(st) = g {
                cx.world.set_state(game, u, st, true);
            }
            set_param(cx, u, 0, 1);
            return;
        }
    }
    // 5.
    if pct(cx, u) < 50 {
        walk_to(game, cx, u, t, 7);
    } else {
        idle(game, cx, u, cx.aip(p, 5));
    }
}

// ---- PutridDefiler -------------------------------------------------

/// putriddefiler1, painworm1; Impregnate (300); state 110 `pregnant`.
const PUTRIDDEFILER1: i32 = 546;
const PAINWORM1: i32 = 551;
const SKILL_IMPREGNATE: i32 = 300;
const STATE_PREGNANT: u16 = 110;

/// PutridDefiler (137).
pub fn putrid_defiler<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    // 1.
    if t.is_some() && p.combat {
        a1(game, cx, u, t);
        return;
    }
    // 2. The first host (the scan stops at it).
    let host = scan_units(game, u).into_iter().find(|&v| {
        is_monster(game, v)
            && !cx.world.is_dead(v)
            && !matches!(unit_base(cx, v), PUTRIDDEFILER1 | PAINWORM1)
            && !cx.world.has_state(v, STATE_PREGNANT)
            && cx.world.alignment(v) != 2
            && unit_distance(cx, u, v) <= 25
    });
    // 3.
    let Some(h) = host else {
        if p.distance < cx.aip(p, 1) {
            let n = i32::from(cx.aip(p, 2) as u8);
            escape(game, cx, u, t, n, false, false);
        } else {
            idle(game, cx, u, 25);
        }
        return;
    };
    // 4.
    if cx.world.in_melee_range(game, u, h) {
        use_skill(
            game,
            cx,
            u,
            mode::SKILL1,
            SKILL_IMPREGNATE,
            ModeTarget::Unit(h),
        );
    } else {
        walk_to(game, cx, u, Some(h), 0);
    }
}

// ---- Ancient, AncientStatue -------------------------------------

/// ancientbarb1..3.
const TALIC: i32 = 540;
const MADAWC: i32 = 541;
const KORLIC: i32 = 542;

/// "Gate" (`ACT5Q5_IsNotActivatable`).
fn gate<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) -> bool {
    cx.world
        .quest_call(game, u, QuestCall::AncientsNotActivatable)
}

/// "Pick": the boss pick with no command and the level cull.
fn ancient_pick<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
) -> Option<UnitId> {
    boss_pick(game, cx, u, Cull::Level, Score::Diablo(None)).0
}

/// Ancient (133): by the unit's exact class; other
/// classes do nothing.
pub fn ancient<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    match cx.world.class(u) {
        TALIC => talic(game, cx, u, p),
        MADAWC => madawc(game, cx, u, p),
        KORLIC => korlic(game, cx, u, p),
        _ => {}
    }
}

/// A1 or A2 by `roll(2)` (A step 5, K step 4).
fn ancient_swing<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    x: UnitId,
    p: &TickParam,
) {
    if pct(cx, u) < cx.aip(p, 3) {
        if roll(cx, u, 2) != 0 {
            a1(game, cx, u, Some(x));
        } else {
            a2(game, cx, u, Some(x));
        }
    } else {
        idle(game, cx, u, 25);
    }
}

/// The Talic think.
fn talic<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    // 1.
    if gate(game, cx, u) || cx.world.has_state(u, state::UNINTERRUPTABLE) {
        idle(game, cx, u, 25);
        return;
    }
    // 2.
    let Some(x) = p.target.or_else(|| ancient_pick(game, cx, u)) else {
        idle(game, cx, u, 25);
        return;
    };
    let m = cx.world.in_melee_range(game, u, x);
    let d = reach_distance(cx, u, x);
    // 3. The whirlwind.
    let (s1, m1) = cx.skill(p, 1);
    if s1 > 0 && has_skill(cx, u, s1) && d < cx.aip(p, 1) && pct(cx, u) < cx.aip(p, 2) {
        let (tx, ty) = cx.world.position(x);
        let (ox, oy) = cx.world.position(u);
        let s = match point_distance(cx, u, tx, ty) {
            0 => 1,
            s => s,
        };
        let a4 = cx.aip(p, 4);
        let dx = a4.wrapping_mul(tx - ox) / s;
        let dy = a4.wrapping_mul(ty - oy) / s;
        let skill = cx.world.skill_entry(u, s1).map_or(s1, |e| e.0);
        cx.world.clear_current_skill(u);
        cx.world.set_current_skill(u, skill);
        cx.world.set_path_steps(u, 1);
        // The request byte +0x15 := 100: no path.
        request_mode_keep_skill(
            game,
            cx,
            u,
            m1,
            ModeTarget::Point(tx + dx, ty + dy),
            Some(100),
        );
        return;
    }
    // 4.
    if !m {
        walk_to(game, cx, u, Some(x), 0);
        wait(game, cx, u, 10);
        return;
    }
    // 5.
    ancient_swing(game, cx, u, x, p);
}

/// B (Madawc).
fn madawc<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    // 1.
    if gate(game, cx, u) {
        idle(game, cx, u, 25);
        return;
    }
    let Some(x) = ancient_pick(game, cx, u) else {
        idle(game, cx, u, 25);
        return;
    };
    let m = cx.world.in_melee_range(game, u, x);
    let d = reach_distance(cx, u, x);
    // 2.
    if m && pct(cx, u) < cx.aip(p, 4) {
        let n = i32::from(cx.aip(p, 5) as u8);
        if escape(game, cx, u, Some(x), n, true, false) {
            wait(game, cx, u, 25);
            return;
        }
    }
    // 3.
    let (s1, m1) = cx.skill(p, 1);
    let count = cx.world.states_count();
    if s1 > 0 {
        if let Some((st, ts)) = skill_row(cx, s1).map(aura_states) {
            let own = st < 1 || st > count || !cx.world.has_state(u, st as u16);
            let tgt = ts < 1 || ts > count || !cx.world.has_state(x, ts as u16);
            if own
                && tgt
                && pct(cx, u) < cx.aip(p, 3)
                && use_skill(game, cx, u, m1, s1, ModeTarget::Point(0, 0))
            {
                return;
            }
        }
    }
    // 4.
    let aip1 = cx.aip(p, 1);
    if d < aip1 && pct(cx, u) < cx.aip(p, 2) && mode_at(game, cx, u, mode::ATTACK1, Some(x)) {
        return;
    }
    // 5.
    if d < aip1 || !move_steps(game, cx, u, Some(x), false, i32::from((d - aip1) as u8)) {
        idle(game, cx, u, 10);
    }
}

/// K (Korlic).
fn korlic<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, p: &TickParam) {
    // 1.
    if gate(game, cx, u) {
        idle(game, cx, u, 25);
        return;
    }
    let Some(x) = ancient_pick(game, cx, u) else {
        idle(game, cx, u, 25);
        return;
    };
    let m = cx.world.in_melee_range(game, u, x);
    let d = reach_distance(cx, u, x);
    // 2.
    let (s1, _) = cx.skill(p, 1);
    if s1 > 0 && d < cx.aip(p, 1) && pct(cx, u) < cx.aip(p, 2) && has_skill(cx, u, s1) {
        use_sequence_skill(game, cx, u, s1, ModeTarget::Unit(x));
        return;
    }
    // 3.
    if !m {
        walk_to(game, cx, u, Some(x), 0);
        wait(game, cx, u, 10);
        return;
    }
    // 4.
    ancient_swing(game, cx, u, x, p);
}

/// MinionSpawner (302); state 146 `invis`.
const SKILL_MINIONSPAWNER: i32 = 302;
const STATE_INVIS: u16 = 146;

/// AncientStatue (132) (target mode 0).
pub fn ancient_statue<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    if cx.world.quest_call(game, u, QuestCall::AncientsActivatable)
        && !cx.world.quest_call(game, u, QuestCall::AncientsPortal)
        && !cx.world.has_state(u, STATE_INVIS)
    {
        let room = room_of(game, u);
        cx.world.clear_room_portal_flag(game, room);
        let (x, y) = cx.world.position(u);
        use_skill(
            game,
            cx,
            u,
            mode::ATTACK1,
            SKILL_MINIONSPAWNER,
            ModeTarget::Point(x, y),
        );
    } else {
        idle(game, cx, u, 25);
    }
}

// ---- FrozenHorror --------------------------------------------------

/// FrozenHorror (124).
pub fn frozen_horror<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let (s1, _) = cx.skill(p, 1);
    let r = cx.world.skill_level(u, s1, true).unwrap_or(0).max(0);
    // 1.
    if s1 >= 0
        && p.distance < r
        && cx.chance(u, cx.aip(p, 3))
        && !cx.world.has_state(u, STATE_INFERNO)
    {
        skill_k(game, cx, u, p, 1, t);
        return;
    }
    // 2.
    if cx.world.has_state(u, STATE_INFERNO) {
        cx.world.set_state(game, u, STATE_INFERNO, false);
    }
    // 3.
    if !p.combat {
        if cx.chance(u, cx.aip(p, 2)) {
            walk_to(game, cx, u, t, 0);
            return;
        }
    } else if cx.chance(u, cx.aip(p, 1)) {
        a1(game, cx, u, t);
        return;
    }
    // 4.
    idle(game, cx, u, cx.aip(p, 4));
}

// ---- SiegeBeast ----------------------------------------------------

/// imp1 (`BaseId` of the rider scan).
const IMP1: i32 = 492;

/// The rider scan of the think's step 2 (scan 1, callback, arg
/// {max}): the first free imp1-based monster within squared distance
/// `max` is assigned to this beast or tower: its
/// mount param 0 := this unit's GUID unless its current mount still
/// exists and is strictly nearer by squared distance (ties
/// reassign). Also SiegeTower. True when one
/// was found.
pub(super) fn rider_scan<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    max: i32,
) -> bool {
    let rider = scan_units(game, u).into_iter().find(|&v| {
        v != u
            && is_monster(game, v)
            && !dying(cx, v)
            && unit_base(cx, v) == IMP1
            && pairing(cx, u, v)
            && cx.world.alignment(v) == 0
            && cx.world.owner(game, v).is_none()
            && param(cx, v, 0) == -1
            && sq_dist(cx, u, v) <= max
    });
    let Some(v) = rider else {
        return false;
    };
    let cur = game
        .lists
        .find_unit(UnitType::Monster, param(cx, v, 0) as u32);
    let keep = cur.is_some_and(|c| sq_dist(cx, v, c) < sq_dist(cx, v, u));
    if !keep {
        set_param(cx, v, 0, guid_of(game, Some(u)));
    }
    true
}

/// SiegeBeast (115).
pub fn siege_beast<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    // 1. Nothing scheduled (edge case 3).
    if dying(cx, u) {
        return;
    }
    // 2.
    if cx.world.owner(game, u).is_none() && cx.world.alignment(u) == 0 {
        let aip1 = cx.aip(p, 1);
        rider_scan(game, cx, u, aip1.wrapping_mul(aip1));
    }
    let (s1, m1) = cx.skill(p, 1);
    let stomp = |game: &mut Game, cx: &mut Ctx<'_, W>| {
        use_skill(game, cx, u, m1, s1, ModeTarget::Point(0, 0));
    };
    // 3.
    if p.combat {
        if s1 >= 0 && pct(cx, u) < cx.aip(p, 3) {
            stomp(game, cx);
        } else if pct(cx, u) < cx.aip(p, 2) {
            a1(game, cx, u, t);
        } else {
            idle(game, cx, u, cx.aip(p, 4));
        }
        return;
    }
    // 4.
    let range = skill_row(cx, s1).map_or(0, |r| r.param5 as i32);
    if s1 >= 0 && p.distance < range && pct(cx, u) < cx.aip(p, 5) {
        stomp(game, cx);
        return;
    }
    // 5.
    let direct = t.is_some_and(|tt| cx.world.can_reach_directly(game, u, tt));
    if direct && pct(cx, u) < cx.aip(p, 6) {
        set_velocity(cx, u, 0, cx.aip(p, 7).clamp(0, 127), 0);
    }
    // 5, 6.
    radius(game, cx, u, t, 12, 0);
}

// ---- SuicideMinion, BaalMinion ----------------------------------

/// SuicideMinion (117) (also special state 15).
pub fn suicide_minion<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let f = param(cx, u, 0);
    // 1, 2.
    if f != 0 {
        if f < game.frame {
            mode_at(game, cx, u, mode::DEATH, t);
        } else {
            idle(game, cx, u, cx.aip(p, 2));
        }
        return;
    }
    // 3.
    if p.combat {
        set_param(cx, u, 0, game.after(cx.aip(p, 5)));
        idle(game, cx, u, cx.aip(p, 2));
    } else if pct(cx, u) < cx.aip(p, 3) {
        walk_to(game, cx, u, t, 0);
    } else {
        idle(game, cx, u, cx.aip(p, 2));
    }
}

/// BaalMinion (141).
pub fn baal_minion<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    if t.is_none() || !p.combat {
        // 1.
        if pct(cx, u) < cx.aip(p, 2) {
            walk_to(game, cx, u, t, 0);
        }
    } else if pct(cx, u) < cx.aip(p, 1) {
        // 2.
        if cx.skill(p, 1).0 >= 0 && pct(cx, u) < cx.aip(p, 3) {
            skill_k(game, cx, u, p, 1, t);
        } else {
            a1(game, cx, u, t);
        }
    } else {
        idle(game, cx, u, cx.aip(p, 3));
    }
    // 3.
    wait(game, cx, u, cx.aip(p, 4));
}

// ---- BaalTaunt, BaalToStairs -------------------------------------

/// Baal Taunt.
const SKILL_BAALTAUNT: i32 = 284;

/// BaalTaunt (136).
pub fn baal_taunt<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    // 1.
    let Some(t) = p.target else {
        idle(game, cx, u, 25);
        return;
    };
    // 2.
    let living = matches!(type_of(game, t), Some(UnitType::Player | UnitType::Monster));
    if living && cx.world.anim_mode(t) == mode::NEUTRAL {
        let n = param(cx, u, 0) + 1;
        set_param(cx, u, 0, n);
        if n > cx.aip(p, 2) {
            set_param(cx, u, 0, 0);
            use_skill(
                game,
                cx,
                u,
                mode::ATTACK1,
                SKILL_BAALTAUNT,
                ModeTarget::Unit(t),
            );
            return;
        }
    } else {
        set_param(cx, u, 0, 0);
    }
    // 3.
    if p.distance > cx.aip(p, 3) {
        let (x, y) = cx.world.position(t);
        let room = room_of(game, t);
        if cx.world.place_unit(game, u, room, x, y) {
            idle(game, cx, u, 25);
            return;
        }
    }
    // 4, 5.
    if p.distance > cx.aip(p, 1) {
        walk_to(game, cx, u, Some(t), 0);
    } else {
        idle(game, cx, u, 25);
    }
}

/// The Worldstone Chamber portal (objects row 563).
const OBJ_WORLDSTONE_PORTAL: i32 = 563;

/// BaalToStairs (138).
pub fn baal_to_stairs<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    // 1.
    let portal = scan_units(game, u).into_iter().find(|&v| {
        type_of(game, v) == Some(UnitType::Object)
            && cx.world.class(v) == OBJ_WORLDSTONE_PORTAL
            && reach_distance(cx, v, u) <= 25
    });
    let Some(o) = portal else {
        idle(game, cx, u, 25);
        return;
    };
    // 2.
    if reach_distance(cx, u, o) < cx.aip(p, 1) {
        cx.world.quest_call(game, u, QuestCall::BaalToStairs);
        cx.world.set_state(game, u, STATE_INVIS, true);
        cx.world.stop_unit_path(u);
        delete_thinks(game, u);
        cx.world.remove_unit(game, u);
        return;
    }
    // 3.
    set_velocity(cx, u, 1, 0, 0);
    walk_to(game, cx, u, Some(o), 0);
}

// ---- BaalThrone ----------------------------------------------------

/// Skills and classes of the throne.
const SKILL_CORPSE_EXPLODE: i32 = 285;
const SKILL_MONSTER_SPAWN: i32 = 286;
const BAALCRABSTAIRS: i32 = 559;
const STATE_CHANGECLASS: u16 = 142;

/// BaalThrone's init: does nothing.
pub fn baal_throne_init() {}

/// BaalThrone (134) (target mode 2).
pub fn baal_throne<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let frame = game.frame;
    // 1.
    if cx.monstats(cx.world.class(u)).is_none() {
        idle(game, cx, u, 10);
        return;
    }
    // 2. A negative delay (edge case 4).
    let q = param(cx, u, 2);
    if frame < q {
        idle_raw(game, cx, u, frame.wrapping_sub(q));
        return;
    }
    // 3.
    let n = scan_units(game, u)
        .into_iter()
        .filter(|&v| {
            v != u
                && is_monster(game, v)
                && !cx.world.is_dead(v)
                && unit_distance(cx, u, v) < 64
                && !cx.world.hostile(game, u, v)
        })
        .count();
    // 4.
    if n != 0 {
        let (s1, _) = cx.skill(p, 1);
        let count = cx.world.states_count();
        if let (Some(tt), true) = (t, s1 > 0) {
            if let Some((st, ts)) = skill_row(cx, s1).map(aura_states) {
                let own = st < 0 || st >= count || !cx.world.has_state(u, st as u16);
                let tgt = ts < 0 || ts >= count || !cx.world.has_state(tt, ts as u16);
                if own && tgt && pct(cx, u) < cx.aip(p, 1) {
                    skill_k(game, cx, u, p, 1, t);
                    return;
                }
            }
        }
        idle(game, cx, u, 10);
        return;
    }
    let w = param(cx, u, 0);
    let flags = param(cx, u, 1);
    // 5. A missing wave record schedules nothing (edge case 5).
    if flags & 1 == 0 {
        if (w as u32) <= 4 {
            match cx.world.wave(w) {
                Some((_, class)) if class >= 0 => {
                    cx.world.preload_class(game, u, class);
                    match class {
                        62 => cx.world.preload_class(game, u, 23),
                        105 => cx.world.preload_class(game, u, 381),
                        _ => {}
                    }
                }
                _ => return,
            }
        }
        use_skill(game, cx, u, 10, SKILL_CORPSE_EXPLODE, ModeTarget::Unit(u));
        set_param(cx, u, 1, flags | 1);
        set_param(cx, u, 2, game.after(250));
        cx.world.play_sound(game, u, 16, None);
        return;
    }
    // 6.
    if w >= 5 {
        cx.world
            .reinit_class(game, u, BAALCRABSTAIRS, mode::NEUTRAL);
        install(game, cx, u, 0);
        cx.world.set_state(game, u, STATE_CHANGECLASS, true);
        cx.world.change_class_list(game, u, BAALCRABSTAIRS);
        idle(game, cx, u, 5);
        return;
    }
    // 7. The spawn wave.
    cx.world.assign_skill(game, u, SKILL_MONSTER_SPAWN, 1);
    // A missing record returned in step 5 before flag 1 was set, so it is
    // never reached here; −1 is unreachable.
    let id = cx.world.wave(w).map_or(-1, |r| r.0);
    cx.world.set_skill_param(u, SKILL_MONSTER_SPAWN, id);
    let (x, y) = cx.world.position(u);
    use_skill(
        game,
        cx,
        u,
        10,
        SKILL_MONSTER_SPAWN,
        ModeTarget::Point(x, y.wrapping_add(13)),
    );
    set_param(cx, u, 0, w + 1);
    set_param(cx, u, 2, game.after(100));
    set_param(cx, u, 1, (flags | 2) & !1);
}

// ---- BaalCrab, BaalCrabClone -----------------------------------

/// The Worldstone Chamber (levels row 132); uberbaal (709); baalclone
/// (570).
const WORLDSTONE_CHAMBER: i32 = 132;
const UBERBAAL: i32 = 709;
const BAALCLONE: i32 = 570;

/// The Baal choice.
fn baal_choice<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    x: Option<UnitId>,
    big_m: i32,
    n: i32,
    h: Home,
) -> i32 {
    // 1.
    let pending = param(cx, u, 0);
    if pending != 0 {
        return pending;
    }
    // 2.
    let Some(x) = x else {
        if cx.world.pattern_collides(game, u, 2, 0x40) {
            return 4;
        }
        return if pct(cx, u) < 8 { 9 } else { 1 };
    };
    // 3.
    let close = portal_of(game, cx, x).is_some_and(|pt| {
        cx.world.level_id(game, pt) == WORLDSTONE_CHAMBER
            && h.is_none_or(|(hx, hy)| point_distance(cx, pt, hx, hy) < 75)
    });
    let hd = h.map(|(hx, hy)| half_size_distance(cx, x, hx, hy));
    let medium = hd.is_some_and(|d| d > 75);
    let far = hd.is_some_and(|d| d > 100);
    let c = special_caster(cx, x);
    let m = cx.world.in_melee_range(game, u, x);
    let clear = !cx.world.line_blocked(game, u, x);
    let l = cx.world.life_percent(u);
    let normal = cx.info.difficulty == 0;
    // 4.
    let mut w: [i32; 16];
    if m {
        let b = if normal { 75 } else { 50 };
        w = [0, 0, 0, 0, 0, 0, 0, 0, 20, 30, 150, 10, 10, 70, 40, 0];
        w[1] = b + 100 - l;
        if normal {
            w[13] = 45;
        }
        if cx.world.life_percent(x) < 33 {
            w[10] = 200;
        }
        if cx.world.has_state(x, 11) {
            w[8] = 0;
            w[13] = 0;
            w[11] = 0;
        }
        if !clear {
            w[11] = 0;
            w[13] = 0;
            w[12] = 0;
        }
    } else if clear {
        let b = if normal { 125 } else { 100 };
        let w15 = (10 * (2 - i32::from(c))).max(0);
        w = [0, 0, 5, 5, 5, 0, 5, 0, 40, 40, 0, 70, 80, 60, 20, 0];
        w[1] = b + 100 - l;
        w[15] = w15;
        if normal {
            w[13] = 40;
        }
        let d = unit_distance(cx, u, x);
        if d > 35 {
            w[6] = 15;
            w[13] = 0;
        }
        if d > 25 {
            w[14] = 30;
            w[11] = 0;
        }
        if n < 2 {
            w[11] -= 10;
        }
        if n > 3 {
            w[13] += 25;
        }
        if big_m > 60 {
            w[8] = 70;
        }
        if n < 2 && record_difficulty(cx) < 2 {
            w[8] = 0;
        }
        if medium {
            w[6] += 25;
            w[14] += 35;
            w[2] = 0;
            w[3] = 25;
        } else if c {
            w[6] += 30;
            w[11] += 10;
            w[14] += 15;
        }
        if far {
            w[5] = 60;
        }
        if close {
            w[7] = 0;
        }
    } else {
        let b = if normal { 125 } else { 100 };
        w = [0, 0, 20, 20, 20, 0, 20, 0, 80, 70, 0, 0, 0, 0, 0, 0];
        w[1] = b + 100 - l;
        if n < 2 {
            w[2] = 45;
            w[10] = 25;
        }
        if medium {
            w[13] = 25;
            w[2] = 0;
            w[6] = 0;
            w[3] = 35;
            w[11] = 25;
        } else if c {
            w[13] = 50;
            w[14] = 50;
        }
        if far {
            w[5] = 60;
        }
    }
    // 5.
    weighted(cx, u, &w, 1)
}

/// The Baal clone.
fn baal_clone<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    let minions = cx
        .store
        .control(u)
        .map(|c| c.minions.clone())
        .unwrap_or_default();
    let living_minion = minions.into_iter().any(|g| {
        game.lists
            .find_unit(UnitType::Monster, g)
            .is_some_and(|m| !cx.world.is_dead(m))
    });
    let living_owner = cx
        .world
        .owner(game, u)
        .is_some_and(|o| !cx.world.is_dead(o));
    if living_minion || living_owner {
        return;
    }
    // The spawn info (key 544): for class 570 it
    // keeps the class and mode 1 and spends two `roll(24)` steps on a point
    // overwritten below (open questions 2, 10).
    let info = spawn_info(game, cx, u, fixed_class(cx, BAALCLONE));
    let (class, m) = (info.class, info.mode);
    let base = cx
        .world
        .path_target(u)
        .map_or(cx.world.position(u), |t| cx.world.position(t));
    let x = base.0 + (cx.world.seed(u).step() % 24) as i32 - 12;
    let y = base.1 + (cx.world.seed(u).step() % 24) as i32 - 12;
    let Some(room) = cx.world.room_at(game, u, x, y) else {
        return;
    };
    let Some(c) = cx.spawn_monster(game, room, x, y, class, m, -1, 0) else {
        return;
    };
    cx.world.set_unit_flag(c, 0x0402_0000);
    wait(game, cx, c, 15);
    // Owner data and the minion list.
    let me = game.lists.unit(u).map(|e| UnitRef {
        ty: e.ty,
        guid: e.guid,
    });
    let cg = guid_of(game, Some(c)) as u32;
    if let Some(ctl) = cx.store.control_mut(u) {
        ctl.minion_owner = me;
        ctl.minions.push(cg);
    }
    if let Some(ctl) = cx.store.control_mut(c) {
        ctl.minion_owner = me;
    }
    let (max, cur) = (cx.world.max_life(u), life(cx, u));
    cx.world.set_stat(c, 7, max / 3);
    cx.world.set_stat(c, 6, cur / 3);
    cx.world.set_stat(c, 74, 0);
    cx.world.link_clone(game, u, c);
    cx.world.play_sound(game, u, 16, None);
    set_param(cx, u, 1, param(cx, u, 1) + 1);
}

/// The Baal execution.
fn baal_execute<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
    x: Option<UnitId>,
    k: i32,
    h: Home,
) {
    if cx.monstats(cx.world.class(u)).is_none() {
        return;
    }
    let fixed = |game: &mut Game, cx: &mut Ctx<'_, W>, m: u8, skill: i32| {
        use_skill(game, cx, u, m, skill, target_of(x));
    };
    match k {
        1 => {
            let n = match cx.info.difficulty {
                0 => 35,
                1 => 15,
                _ => 5,
            };
            idle(game, cx, u, n);
        }
        2 | 6 => {
            radius(game, cx, u, x, 12, 0);
        }
        3 => {
            circle(game, cx, u, x, 6, true);
        }
        4 => {
            wander(game, cx, u, 16);
        }
        5 => match h {
            Some((hx, hy)) => {
                walk_to_point(game, cx, u, hx, hy);
            }
            None => idle(game, cx, u, 5),
        },
        8 => match x {
            None => idle(game, cx, u, 5),
            Some(xx) => {
                let player = is_player(game, xx);
                if cx.skill(p, 6).0 > 0
                    && (cx.world.max_mana(xx) < cx.world.max_life(xx) || !player)
                {
                    skill_k(game, cx, u, p, 6, x);
                } else if cx.skill(p, 7).0 > 0 && player {
                    skill_k(game, cx, u, p, 7, x);
                }
            }
        },
        9 => fixed(game, cx, mode::SKILL2, 315),
        10 => {
            mode_at(game, cx, u, mode::ATTACK2, x);
        }
        11 => {
            cx.world.assign_skill(game, u, 316, 1);
            fixed(game, cx, 10, 316);
        }
        12 => fixed(game, cx, mode::SKILL1, 317),
        13 => fixed(game, cx, mode::ATTACK1, 318),
        14 => {
            let Some(xx) = x else {
                idle(game, cx, u, 5);
                return;
            };
            let d = match reach_distance(cx, u, xx) {
                0 => 1,
                d => d,
            };
            let (ox, oy) = cx.world.position(u);
            let (tx, ty) = cx.world.position(xx);
            let px = ox + 25 * (ox - tx) / d;
            let py = oy + 25 * (oy - ty) / d;
            let (s5, m5) = cx.skill(p, 5);
            let class = cx.world.class(u);
            if class == UBERBAAL {
                use_skill(game, cx, u, m5, s5, ModeTarget::Point(tx, ty));
            } else if let Some((fx, fy)) = cx.world.free_spot_for(game, u, class, px, py) {
                use_skill(game, cx, u, m5, s5, ModeTarget::Point(fx, fy));
            } else {
                idle(game, cx, u, 5);
            }
        }
        15 => {
            baal_clone(game, cx, u);
            idle(game, cx, u, 5);
        }
        _ => idle(game, cx, u, 5),
    }
}

/// BaalCrab (135) (target mode 0). No aip is read.
pub fn baal_crab<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    // 1–3.
    let (x, big_m, n) = boss_pick(game, cx, u, Cull::Act, Score::Baal);
    let h = home_point(cx, u);
    let k = baal_choice(game, cx, u, x, big_m, n, h);
    // 4.
    baal_execute(game, cx, u, p, x, k, h);
    wait(game, cx, u, 25);
}

/// BaalCrabClone (140).
pub fn baal_crab_clone<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    // 1.
    if minion_owner(game, cx, u).is_some_and(|o| cx.world.is_dead(o)) {
        cx.world.kill(game, u, None);
        return;
    }
    // 2.
    if cx.world.owner(game, u).is_none_or(|o| cx.world.is_dead(o)) {
        cx.world.kill(game, u, None);
        return;
    }
    // 3.
    let (x, big_m, n) = boss_pick(game, cx, u, Cull::Act, Score::Baal);
    let h = home_point(cx, u);
    let mut k = baal_choice(game, cx, u, x, big_m, n, h);
    if matches!(k, 7 | 9 | 14 | 15) {
        k = 2;
    }
    // 4.
    if x.is_some() {
        baal_execute(game, cx, u, p, x, k, h);
        wait(game, cx, u, 25);
    } else {
        idle(game, cx, u, 15);
    }
}

// ---- Nihlathak -----------------------------------------------------

/// evilhut (the spawner footprint class), minion1.
const EVILHUT: i32 = 528;

/// Nihlathak's init: does nothing.
pub fn nihlathak_init() {}

/// Nihlathak (128) (target mode 2).
pub fn nihlathak<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
) {
    let t = p.target;
    let d = p.distance;
    let (s1, _) = cx.skill(p, 1);
    let e1 = if s1 >= 0 {
        cx.world.skill_entry(u, s1)
    } else {
        None
    };
    let blink = |game: &mut Game, cx: &mut Ctx<'_, W>, (id, m): (i32, u8)| {
        let r = cx.aip(p, 2);
        teleport_in_range(game, cx, u, r, id, m);
    };
    // 1.
    cx.world.quest_call(game, u, QuestCall::Nihlathak);
    // 2.
    if cx.world.has_state(u, STATE_INFERNO) {
        cx.world.set_state(game, u, STATE_INFERNO, false);
    }
    // 3.
    if t.is_none() {
        match e1.filter(|_| cx.ai_state_set(u)) {
            Some(e) => blink(game, cx, e),
            None => idle(game, cx, u, 25),
        }
        return;
    }
    // 4.
    if let Some(e) = e1.filter(|_| p.combat) {
        if pct(cx, u) < cx.aip(p, 1) {
            blink(game, cx, e);
            return;
        }
    }
    // 5.
    if d < cx.aip(p, 5) && pct(cx, u) < 40 && escape(game, cx, u, t, 5, true, false) {
        return;
    }
    // 6.
    let (s3, m3) = cx.skill(p, 3);
    if s3 > 0 && cx.world.skill_entry(u, s3).is_some() && pct(cx, u) < cx.aip(p, 3) {
        let level = cx.world.skill_level(u, s3, true).unwrap_or(0);
        if let Some(k) = cx.world.corpse_search(game, u, t, s3, level) {
            if use_skill(game, cx, u, m3, s3, ModeTarget::Unit(k)) {
                return;
            }
        }
    }
    // 7.
    let (s4, _) = cx.skill(p, 4);
    let e4 = s4 > 0 && cx.world.skill_entry(u, s4).is_some();
    if e4 && pct(cx, u) < 60 && d < 14 {
        skill_k(game, cx, u, p, 4, t);
        return;
    }
    // 8.
    let (s2, _) = cx.skill(p, 2);
    if s2 > 0 {
        if pct(cx, u) < cx.aip(p, 4) && has_skill(cx, u, s2) {
            if let (_, Some(w), _) = minion_scan(game, cx, u, 0, 625, 50) {
                skill_k(game, cx, u, p, 2, Some(w));
                return;
            }
        }
        let room = room_of(game, u);
        let (x, y) = cx.world.position(u);
        if cx.skill(p, 5).0 > 0
            && cx
                .world
                .footprint_ok(game, fixed_class(cx, EVILHUT), room, x, y)
        {
            let class = cx.world.class_for_level(game, room, MINION1);
            if let Some(c) = cx.store.control_mut(u) {
                c.spawn_class = class;
            }
            cx.world.note_spawn_class(u, class);
            skill_k(game, cx, u, p, 5, t);
        } else {
            wander(game, cx, u, 6);
        }
        return;
    }
    // 9.
    if e4 && d <= 13 {
        skill_k(game, cx, u, p, 4, t);
        return;
    }
    // 10. Both requests.
    if pct(cx, u) < 60 {
        walk_to(game, cx, u, t, 0);
    }
    idle(game, cx, u, 5);
}

/// Nihlathak's alternate (also the Hireable alternate):
/// state 12 off, re-install, idle 1. No draws.
pub fn nihlathak_alt<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    if cx.world.has_state(u, STATE_INFERNO) {
        cx.world.set_state(game, u, STATE_INFERNO, false);
    }
    reinstall(game, cx, u);
    idle(game, cx, u, 1);
}

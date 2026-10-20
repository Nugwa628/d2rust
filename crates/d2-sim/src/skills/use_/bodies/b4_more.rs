//! Batch 4 bodies, second part: the monster skill
//! slots from Mosquito to NecromageMissile, then the slots no monster
//! skill row names (the Assassin progressive functions, the Hurricane /
//! Armageddon / attached state functions and the Chain Lightning item
//! effect). The pregnant remove callback is here too.

use super::b4_helpers::*;
use super::b4_mon::{mon_inferno_start, mon_teleport, room_at_of};
use super::dos::{curse_unit, CurseCtx};
use super::effects::{BodyEffect, PathOp};
use super::helpers::*;
use super::helpers2::*;
use super::helpers3::*;
use super::starts2::corpse_soft;
use super::{callback, BodyWorld, MissileRequest, MonsterSpawn};
use crate::combat::{
    apply, apply_melee, pct, start_combat, CombatTables, CombatWorld, DamageRecord,
};
use crate::missiles::bodies::area_hit;
use crate::rng::Seed;
use crate::skills::{elem_len, phys_max, phys_min, roll_elemental, roll_physical, SkillTables};
use crate::units::UnitType;

/// The unit of (E param `ty` type, E param `guid` GUID).
fn e_unit<W: BodyWorld>(w: &W, u: W::Unit, ty: u8, guid: u8) -> Option<W::Unit> {
    let (t, g) = (e_param(w, u, ty), e_param(w, u, guid));
    w.find_unit(t as u32, g as u32)
}

/// 1 ≤ m < missiles count.
fn missile_ok1(t: &SkillTables, m: i32) -> bool {
    m >= 1 && missile_ok(t, m)
}

// -----------------------------------------------------------------------

/// srvst 55 Mosquito.
pub fn mosquito_start<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some((c1, c2)) = rec(t, skill).map(|r| (r.calc1, r.calc2)) else {
        return 0;
    };
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let (ty, tgu) = ident(w, tg);
    let ug = guid(w, u);
    let mut s = Seed::init_low(tgu.wrapping_add(ug));
    let a = eval(w, t, u, c1, skill, lvl);
    let b = eval(w, t, u, c2, skill, lvl);
    let n = a.wrapping_add(s.roll(b.wrapping_sub(a)) as i32).max(1);
    w.set_entry_param_of(u, &e, 1, n);
    w.set_entry_param_of(u, &e, 2, tgu as i32);
    w.set_entry_param_of(u, &e, 3, ty.index() as i32);
    1
}

/// srvdo 107 Mosquito.
pub fn mosquito<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(calc3) = rec(t, skill).map(|r| r.calc3) else {
        return 0;
    };
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    let Some(k) = e_unit(w, u, 3, 2) else {
        return 0;
    };
    if !w.combat().in_melee_range(u, k, 0) {
        return 0;
    }
    let n = w.entry_param(u, &e, 1);
    let mut record = DamageRecord::default();
    let a = phys_min(w, t, Some(u), skill, lvl, true) >> 8;
    let b = phys_max(w, t, Some(u), skill, lvl, true) >> 8;
    roll_physical(w, t, u, &mut record, skill, lvl);
    roll_elemental(w, t, u, &mut record, skill, lvl);
    let d = b.wrapping_sub(a);
    record.poison = a.wrapping_add(w.seed(u).roll(d) as i32).wrapping_mul(2);
    record.poison_len = elem_len(w, t, Some(u), skill, lvl);
    record.mana_leech = a.wrapping_add(w.seed(u).roll(d) as i32).wrapping_shl(8);
    record.stamina_leech = a.wrapping_add(w.seed(u).roll(d) as i32).wrapping_shl(8);
    record.result = 1;
    let heal = pct(record.physical, eval(w, t, u, calc3, skill, lvl), 100);
    let life = w.stat(u, sid::LIFE, 0).wrapping_add(heal);
    let m = w.stat_max(u, sid::LIFE);
    w.set_stat(u, sid::LIFE, life.min(m));
    apply(w.combat(), ct, u, k, true, &mut record);
    if w.stat(k, sid::LIFE, 0) == 0 {
        record.result |= 2;
    }
    w.combat().reaction(u, k, &mut record);
    if n.wrapping_sub(1) > 0 {
        w.set_entry_param_of(u, &e, 1, n - 1);
        w.set_frame_event_index(u, param(t, skill, 1));
        let c = w.frame_count(u);
        w.set_frame_count(u, (c & !0xFF).wrapping_add(0x100));
    }
    1
}

// -----------------------------------------------------------------

/// The MonCurseCast curses.
pub const CURSES: [i32; 5] = [66, 72, 82, 87, 91];

/// srvdo 112 MonCurseCast.
pub fn mon_curse_cast<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    lvl: i32,
) -> i32 {
    let c = (w.seed(u).step() % 5) as usize;
    let k = CURSES[c];
    let Some(r) = rec(t, k).cloned() else {
        return 0;
    };
    let ts = s16(r.auratargetstate);
    if !state_ok(w, ts) {
        return 0;
    }
    flag_40(w, u);
    let range = eval(w, t, u, r.aurarangecalc, k, lvl);
    let d = eval(w, t, u, r.auralencalc, k, lvl);
    let mut cx = CurseCtx {
        ai: false,
        upd: false,
        skill: k,
        lvl,
        duration: d,
        stats: [-1; 6],
        values: [0; 6],
        state: ts,
        events: [(0, 0); 3],
    };
    let p7 = r.param7 as i32;
    match c {
        0 => (cx.stats[0], cx.values[0]) = (36, -100),
        1 => (cx.stats[0], cx.values[0]) = (25, -50),
        2 => cx.events[0] = (5, 4),
        3 => cx.events = [(1, 5), (2, 5), (0, 0)],
        _ => {
            cx.upd = true;
            for (i, (s, v)) in [(67, p7), (25, -50), (36, -50), (68, p7)]
                .into_iter()
                .enumerate()
            {
                cx.stats[i] = s;
                cx.values[i] = v;
            }
        }
    }
    let mut cb = |w: &mut W, x: W::Unit| curse_unit(w, t, ct, u, &cx, x);
    scan_point(w, t, ct, 3, u, range, &mut cb)
}

// -----------------------------------------------------------------------

/// srvdo 108 RegurgitatorEat.
pub fn regurgitator_eat<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(calc1) = rec(t, skill).map(|r| r.calc1) else {
        return 0;
    };
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if w.unit_type(tg) != UnitType::Monster || w.combat().is_hireling(tg) || w.mode(tg) != MODE_DEAD
    {
        return 0;
    }
    let mt = w.stat_max(tg, sid::LIFE);
    w.effect(BodyEffect::RoomDelete(tg));
    w.effect(BodyEffect::RemoveUnit(tg));
    let h = w.stat(u, sid::LIFE, 0);
    let m = w.stat_max(u, sid::LIFE);
    if mt > 0 {
        let v = h.wrapping_add(pct(mt, eval(w, t, u, calc1, skill, lvl), 100));
        w.set_stat(u, sid::LIFE, v.max(1).min(m));
    }
    1
}

/// srvdo 125 Wake Of Destruction Sentry.
pub fn wake_of_destruction<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    if rec(t, skill).is_none() {
        return 0;
    }
    let m = prog_missile(w, t, u, skill);
    if !missile_ok(t, m) {
        return 0;
    }
    let Some((tx, ty)) = tpos(w, u) else {
        return 0;
    };
    flag_40(w, u);
    let (ux, uy) = w.position(u);
    let (dx, dy) = (tx.wrapping_sub(ux), ty.wrapping_sub(uy));
    let Some(mm) = w.spawn_missile(MissileRequest {
        flags: 2,
        origin: Some(u),
        target_x: dx,
        target_y: dy,
        skill,
        level: lvl,
        ..MissileRequest::new(u, m)
    }) else {
        return 0;
    };
    w.effect(BodyEffect::MissileData28 {
        missile: mm,
        v: dy.wrapping_neg(),
    });
    w.effect(BodyEffect::MissileData2C { missile: mm, v: dx });
    1
}

// -----------------------------------------------------------------------

/// srvst 59 Imp Inferno.
pub fn imp_inferno_start<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    mon_inferno_start(w, t, u, skill, lvl, true)
}

/// srvdo 126 Imp Inferno.
pub fn imp_inferno<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill).cloned() else {
        return 0;
    };
    let m = s16(r.srvmissilea);
    let Some(p2) = missile_ok1(t, m)
        .then(|| t.missile(m).map(|x| x.param2 as i32))
        .flatten()
    else {
        return 0;
    };
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    if let Some(tg) = target(w, u) {
        if w.has_path(u) {
            let (x, y) = w.position(tg);
            w.path_op(u, PathOp::TargetPoint(x, y));
            w.path_op(u, PathOp::TargetUnit(Some(tg)));
        }
    }
    let (x, y) = w.position(u);
    let (tx, ty) = w.path_target_point(u);
    let mm = w.spawn_missile(MissileRequest {
        flags: 0x25,
        x,
        y,
        target_x: tx,
        target_y: ty,
        skill,
        level: lvl,
        velocity: missile_velocity(t, m, lvl),
        ..MissileRequest::new(u, m)
    });
    if let Some(mm) = mm {
        let n = eval(w, t, u, r.calc2, skill, lvl).wrapping_add(p2);
        w.path_op(mm, PathOp::Steps(n));
        w.set_missile_frames(mm, n, n);
    }
    channel_anim(w, ct, &r, u);
    let f = w.frame();
    if f < w.entry_param(u, &e, 1) && w.has_state(u, st::INFERNO as u16) {
        w.delete_timers(u, 1, 0);
        // The channel goes on: the next step in 3 base ticks.
        w.schedule(u, 0, super::after(w, 3), 0, 0);
        return 1;
    }
    // State 12 stays on; the AI thinks again 13 base ticks later.
    w.delete_timers(u, 0, 0);
    w.schedule(u, 2, super::after(w, 13), 0, 0);
    1
}

// -----------------------------------------------------------------

/// srvdo 141 Baal Corpse Explode.
pub fn baal_corpse_explode<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let mut r = if rec(t, skill).is_some() && lvl > 0 {
        param(t, skill, 5).wrapping_add(lvl.wrapping_sub(1).wrapping_mul(param(t, skill, 6)))
    } else {
        0
    };
    // `corpse_effect` run.
    if r == 0 {
        r = param(t, 285, 1).wrapping_add(lvl.wrapping_sub(1).wrapping_mul(param(t, 285, 2)));
    }
    let at = w.position(u);
    let Some(room) = room_at_of(w, u, at) else {
        return 1;
    };
    for x in w.unit_find(room, at, r, 0x3002) {
        // Callback: path target := U, then srvdo 55.
        w.path_op(u, PathOp::TargetUnit(Some(x)));
        super::b3_lvl06::corpse_explosion(w, t, ct, u, 285, lvl);
    }
    1
}

// ------------------------------------------------------------------------

/// srvst 60 Suck Blood.
pub fn suck_blood_start<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill).cloned() else {
        return 0;
    };
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let mut record = DamageRecord::default();
    skill_result(w, t, ct, u, tg, skill, lvl, &mut record, 0);
    if record.result & 1 == 0 {
        return 1;
    }
    record.hit_flags = 2 | r.hitflags;
    if r.hitclass != 0 {
        record.hit_class = r.hitclass;
    }
    weapon_roll(w, t, u, skill, lvl, &mut record);
    let p = eval(w, t, u, r.calc1, skill, lvl);
    record.physical = record.physical.wrapping_add(pct(record.physical, p, 100));
    let l = eval(w, t, u, r.calc2, skill, lvl);
    record.life_leech = record.life_leech.wrapping_add(l);
    let m = eval(w, t, u, r.calc3, skill, lvl);
    record.mana_leech = record.mana_leech.wrapping_add(m);
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, 128);
    1
}

/// srvdo 127 Suck Blood.
pub fn suck_blood<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(calc1) = rec(t, skill).map(|r| r.calc1) else {
        return 0;
    };
    let Some(tg) = target(w, u) else {
        return 0;
    };
    flag_40(w, u);
    let Some(p) = pair_damage(w, u, tg) else {
        return 0;
    };
    if p.result & 1 != 0 {
        // Edge case 3: a non-monster caster heals no unit.
        let h = if w.unit_type(u) == UnitType::Monster {
            Some(w.minion_owner(u).unwrap_or(u))
        } else {
            None
        };
        if let Some(h) = h {
            let d = p.physical.min(w.stat(tg, sid::LIFE, 0));
            let v =
                w.stat(h, sid::LIFE, 0)
                    .wrapping_add(pct(d, eval(w, t, u, calc1, skill, lvl), 100));
            let m = w.stat_max(h, sid::LIFE);
            w.set_stat(h, sid::LIFE, v.max(1).min(m));
        }
    }
    apply_melee(w.combat(), ct, u, tg);
    1
}

// -------------------------------------------------------------------------

/// srvdo 128 Cry Help.
pub fn cry_help<W: BodyWorld>(w: &mut W, t: &SkillTables, u: W::Unit, skill: i32, lvl: i32) -> i32 {
    let Some((calc1, ov)) = rec(t, skill).map(|r| (r.calc1, s16(r.srvoverlay))) else {
        return 0;
    };
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if w.unit_type(u) != UnitType::Monster {
        return 0;
    }
    let (ty, g) = ident(w, tg);
    // The command holds for `calc1` base ticks.
    let len = eval(w, t, u, calc1, skill, lvl).max(1);
    let frame = super::after(w, len);
    // Edge case 4: +0x18 is never written.
    w.effect(BodyEffect::MinionCommand {
        unit: u,
        kind: 1,
        ty: ty.index() as i32,
        guid: g,
        frame,
    });
    overlay_if(w, tg, ov, 1, false);
    1
}

/// srvst 61 Self-resurrect.
pub fn self_resurrect<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    if w.unit_type(u) != UnitType::Monster {
        return 0;
    }
    revive(w, u);
    flags_clear(w, u, 0xE);
    w.path_op(u, PathOp::FootprintMask(0x100));
    1
}

// ---------------------------------------------------------------- –

/// srvdo 130 Vine Attack.
pub fn vine_attack<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some((m, calc1)) = rec(t, skill).map(|r| (s16(r.srvmissilea), r.calc1)) else {
        return 0;
    };
    if !missile_ok1(t, m) {
        return 0;
    }
    flag_40(w, u);
    let n = eval(w, t, u, calc1, skill, lvl);
    if n <= 0 {
        return 0;
    }
    vines(w, u, n, m, skill, lvl);
    1
}

/// srvdo 131 Overseer Whip.
pub fn overseer_whip<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some((ts, calc1)) = rec(t, skill).map(|r| (s16(r.auratargetstate), r.calc1)) else {
        return 0;
    };
    if !state_ok(w, ts) {
        return 0;
    }
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if w.combat().is_dead(tg) {
        return 0;
    }
    if base_id(w, ct, tg) == 453 {
        let p = eval(w, t, u, calc1, skill, lvl);
        let r = w.seed(u).roll(100) as i32;
        if r >= p && !w.has_state(tg, ts as u16) {
            transform(w, t, ct, u, Some(tg), skill);
            return 1;
        }
    }
    whip_state(w, t, ct, tg, skill, lvl, u);
    1
}

/// srvdo 132 Imp Fire Missile.
pub fn imp_fire_missile<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(m0) = rec(t, skill).map(|r| s16(r.srvmissilea)) else {
        return 0;
    };
    if !missile_ok1(t, m0) {
        return 0;
    }
    flag_40(w, u);
    let m = m0.wrapping_add(w.chain_position(w.class_id(u)));
    skill_missile(w, m, u, skill, lvl, (0, 0), (0, 0), false, false);
    1
}

/// srvdo 133 Impregnate.
pub fn impregnate<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    if rec(t, skill).is_none() {
        return 0;
    }
    flag_40(w, u);
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let ok = w.unit_type(tg) == UnitType::Monster
        && w.is_alive(tg)
        && !matches!(base_id(w, ct, tg), 546 | 551)
        && !w.has_state(tg, st4::PREGNANT as u16)
        && w.combat().alignment(tg) != 2;
    if !ok {
        return 0;
    }
    apply_state(
        w,
        ct,
        StateRequest {
            source: u,
            target: tg,
            skill,
            level: lvl,
            duration: 0,
            stat: -1,
            value: 0,
            state: st4::PREGNANT,
            callback: callback::PREGNANT,
        },
    );
    1
}

/// Pregnant remove callback.
pub fn remove_pregnant<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    tg: W::Unit,
    s: i32,
    l: W::List,
) {
    w.state_on(tg, s, false);
    let (mut c, mut mode) = (551, 1);
    let (skill, _) = w.list_skill(l);
    if let Some(r) = rec(t, skill) {
        let sc = s16(r.summon);
        if (1..ct.monstats.len() as i32).contains(&sc) {
            c = sc;
        }
        let sm = i32::from(r.summode as i8);
        if (0..=15).contains(&sm) {
            mode = sm;
        }
    }
    if w.combat().is_dead(tg) {
        w.spawn_monster(MonsterSpawn::NearLevel {
            unit: tg,
            class: c,
            mode,
            spread: 1,
            flags: 0,
        });
    }
}

/// srvdo 134 Siege Beast Stomp.
pub fn siege_stomp<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill).cloned() else {
        return 0;
    };
    if !missile_ok1(t, s16(r.srvmissilea)) {
        return 0;
    }
    flag_40(w, u);
    let mut record = DamageRecord {
        hit_flags: 1 | r.hitflags,
        result: r.resultflags,
        ..DamageRecord::default()
    };
    if r.hitclass != 0 {
        record.hit_class = r.hitclass;
    }
    roll_physical(w, t, u, &mut record, skill, lvl);
    roll_elemental(w, t, u, &mut record, skill, lvl);
    let range = eval(w, t, u, r.aurarangecalc, skill, lvl);
    area_damage(w, t, ct, u, (0, 0), range, &record, 0);
    1
}

// -------------------------------------------------------------------------

/// srvst 62 MinionSpawner.
pub fn minion_spawner_start<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
) -> i32 {
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    // Edge case 10: c is not tested.
    let (c, mode, (x, y)) = spawn_class(w, t, ct, u, skill);
    // The AI-control fallback leaves x, y and mode unwritten: the base game's
    // uninitialised locals held the caster's position and mode 1 (gen-ai-
    // nihlathak: the minion appears next to Nihlathak, mode 1).
    let (mode, (x, y)) = if (x, y) == (0, 0) && mode == 0 {
        (1, w.position(u))
    } else {
        (mode, (x, y))
    };
    for (i, v) in [(1, c), (2, x), (3, y), (4, mode)] {
        w.set_entry_param_of(u, &e, i, v);
    }
    1
}

/// srvdo 135 MinionSpawner.
pub fn minion_spawner<W: BodyWorld>(w: &mut W, t: &SkillTables, u: W::Unit, skill: i32) -> i32 {
    let Some(ov) = rec(t, skill).map(|r| s16(r.sumoverlay)) else {
        return 0;
    };
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    let [c, x, y, mode] = [1, 2, 3, 4].map(|i| w.entry_param(u, &e, i));
    let Some(room) = room_at_of(w, u, (x, y)) else {
        return 0;
    };
    let at = |spread| MonsterSpawn::At {
        room,
        x,
        y,
        class: c,
        mode,
        spread,
        flags: 0,
    };
    let Some(m) = w.spawn_monster(at(-1)).or_else(|| w.spawn_monster(at(4))) else {
        return 0;
    };
    flags_or(w, m, 0x402_0000);
    link_source(w, m, Some(u));
    overlay_if(w, m, ov, 0, false);
    1
}

// -------------------------------------------------------------------------

/// srvdo 136 DeathMaul.
pub fn death_maul<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some((m, calc1)) = rec(t, skill).map(|r| (s16(r.srvmissilea), r.calc1)) else {
        return 0;
    };
    if !missile_ok1(t, m) {
        return 0;
    }
    let Some(tp) = tpos(w, u) else {
        return 0;
    };
    flag_40(w, u);
    let Some(mm) = missile_at(w, u, skill, lvl, m, (0, 0)) else {
        return 0;
    };
    let a = eval(w, t, u, calc1, skill, lvl);
    if a > 0 {
        w.set_anim_speed(mm, a.clamp(0, 0x7FFF));
    }
    let f = w.missile_frames(mm);
    let d = distance(w.position(u), tp).max(1);
    let n = if d < 10 {
        d.wrapping_mul(f) / 24 + f / 2
    } else {
        d.wrapping_mul(f) / 12
    };
    w.set_missile_frames(mm, n, n);
    1
}

/// srvdo 137 fenris rage.
pub fn fenris_rage<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(a) = rec(t, skill).map(|r| s16(r.aurastate)) else {
        return 0;
    };
    if !state_ok(w, a) {
        return 0;
    }
    flag_40(w, u);
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if !corpse_soft(w, ct, tg) {
        return 0;
    }
    w.state_on(tg, st::CORPSE_NODRAW, true);
    w.queue_update(tg);
    stack_list(w, t, u, u, a, skill, lvl);
    1
}

// -------------------------------------------------------------------------

/// The spawn info
/// as srvdo 140 calls it (the AI's spawn table, keyed by the unit's
/// `BaseId`): the
/// class and mode; the point is unused by the caller. `None`: the keys
/// 526 / 528 without a pick (a fatal assertion; refused).
fn tentacle_spawn_info<W: BodyWorld>(
    w: &mut W,
    ct: &CombatTables,
    u: W::Unit,
    d: i32,
) -> Option<(i32, i32)> {
    let count = ct.monstats.len() as i32;
    let clamp = |c: i32| if count > c { c } else { -1 };
    let class = w.class_id(u);
    let key = match base_id(w, ct, u) {
        b if ct.monstats(b).is_some() => b,
        _ => -1,
    };
    let p = if class < 0 {
        0
    } else {
        w.chain_position(class)
    };
    let chain = |b: i32| class_step(ct, clamp(b), p);
    Some(match key {
        206 => (chain(15), 1),
        228 => {
            let room = w.unit_room(u);
            (w.class_for_level(room, clamp(96)), 1)
        }
        267 => (clamp(6), 8),
        284 => (chain(68), 8),
        298 => (chain(301), 1),
        321 => (clamp(if class == 711 { 712 } else { 19 }), 1),
        334 => (chain(114), 1),
        484 => (chain(453), 1),
        526 | 528 => {
            debug_assert!(
                false,
                "spawn info key {key} without a pick (ai-bodies-2.md §13.1)"
            );
            return None;
        }
        537 => {
            let c = chain(540);
            w.state_on(u, 146, true);
            (c, 1)
        }
        544 => {
            // The incoming class is an uninitialised local here, read as
            // "not 570" (Edge case 7): three draws on the unit's seed.
            let k = (w.seed(u).roll(2) as i32).wrapping_add(d);
            let c = class_step(ct, 562, k);
            w.seed(u).roll(24);
            w.seed(u).roll(24);
            (c, 4)
        }
        _ => (clamp(0), 1),
    })
}

/// srvdo 140 Baal Tentacle.
pub fn baal_tentacle<W: BodyWorld>(w: &mut W, ct: &CombatTables, u: W::Unit) -> i32 {
    let d = w.combat().difficulty() as i32;
    let n = (w.seed(u).step() % 3) as i32 + d + 2;
    let Some((c, mode)) = tentacle_spawn_info(w, ct, u, d) else {
        return 1;
    };
    let (bx, by) = match target(w, u) {
        Some(tg) => w.position(tg),
        None => w.position(u),
    };
    for _ in 0..n {
        let x = bx.wrapping_add((w.seed(u).step() % 18) as i32 - 9);
        let y = by.wrapping_add((w.seed(u).step() % 18) as i32 - 9);
        let Some(room) = room_at_of(w, u, (x, y)) else {
            continue;
        };
        let Some(m) = w.spawn_monster(MonsterSpawn::At {
            room,
            x,
            y,
            class: c,
            mode,
            spread: -1,
            flags: 0,
        }) else {
            continue;
        };
        flags_or(w, m, 0x402_0000);
        w.effect(BodyEffect::WaitThink { m, frames: 15 });
        link_source(w, m, Some(u));
    }
    1
}

/// srvdo 139 Baal Cold Missiles.
pub fn baal_cold_missiles<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(m) = rec(t, skill).map(|r| s16(r.srvmissilea)) else {
        return 0;
    };
    if !missile_ok1(t, m) {
        return 0;
    }
    flag_40(w, u);
    let Some((tx, ty)) = tpos(w, u) else {
        return 0;
    };
    let Some(mm) = skill_missile_unit(w, m, u, skill, lvl, (0, 0), (0, 0), false, false) else {
        return 0;
    };
    let (ux, uy) = w.position(u);
    w.effect(BodyEffect::MissileData28 {
        missile: mm,
        v: ty.wrapping_sub(uy).wrapping_neg(),
    });
    w.effect(BodyEffect::MissileData2C {
        missile: mm,
        v: tx.wrapping_sub(ux),
    });
    1
}

// ---------------------------------------------------------------- –

/// srvdo 129 Imp Teleport.
pub fn imp_teleport<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(a) = rec(t, skill).map(|r| s16(r.aurastate)) else {
        return 0;
    };
    if !state_ok(w, a) {
        return 0;
    }
    let o = source_of(w, u);
    let tg = target(w, u);
    if let Some(o) = o {
        // The target position's result is not tested (Open
        // question 6): always writes the pair (the target
        // unit's position, else the path's target point) and "fails" only
        // when a coordinate is 0, which is then used as is.
        let at = w.target_position(u).unwrap_or_else(|| match tg {
            Some(t) => w.position(t),
            None => w.path_target_point(u),
        });
        w.place_unit(u, None, at);
        release(w, ct, u, o, skill);
        return 1;
    }
    let Some(tg) = tg else {
        return mon_teleport(w, u);
    };
    if source_of(w, tg).is_some() || w.combat().is_dead(tg) {
        return 0;
    }
    let room = w.unit_room(tg);
    let at = w.position(tg);
    if !w.place_unit_flag(u, room, at, 1) {
        return 0;
    }
    possess(w, ct, u, tg, skill, lvl);
    1
}

/// srvdo 148 DoomKnightMissile (component S3) and
/// srvdo 149 NecromageMissile (component S4).
pub fn component_missile<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
    k: usize,
) -> i32 {
    let Some((m0, lob)) = rec(t, skill).map(|r| (s16(r.srvmissilea), r.lob)) else {
        return 0;
    };
    if target(w, u).is_none() {
        return 0;
    }
    if m0 < 0 {
        return 0;
    }
    let mut m = m0;
    if w.unit_type(u) == UnitType::Monster {
        m = m.wrapping_add(w.component(u, k));
    }
    if !missile_ok(t, m) {
        return 0;
    }
    flag_40(w, u);
    skill_missile(w, m, u, skill, lvl, (0, 0), (0, 0), false, lob);
    1
}

// -----------------------------------------------------------------------

/// srvdo 36 Claws of Thunder `srvprgfunc2`.
pub fn thunder_ring<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if lvl <= 0 {
        return 0;
    }
    let Some(calc1) = rec(t, skill).map(|r| r.calc1) else {
        return 0;
    };
    let m = prog_missile(w, t, u, skill);
    if m <= 0 {
        return 0;
    }
    let v = missile_velocity(t, m, lvl).wrapping_add(eval(w, t, u, calc1, skill, lvl));
    ring(w, u, tg, m, skill, lvl, v);
    1
}

/// The progressive count, or `eval(aurarangecalc)` when 0.
fn count_or_range<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    match prog_count(w, t, u, skill, lvl) {
        0 => {
            let c = rec(t, skill).map_or(0, |r| r.aurarangecalc);
            eval(w, t, u, c, skill, lvl)
        }
        s => s,
    }
}

/// srvdo 37 Claws of Thunder `srvprgfunc3` and srvdo
/// 143 Fists of Fire `srvprgfunc1`, Royal Strike `srvprgfunc2`
/// The lightning ring (`ring`) or fan.
pub fn lightning<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
    ring: bool,
) -> i32 {
    let v = w.seed(u).step();
    if rec(t, skill).is_none() {
        return 0;
    }
    let m = prog_missile(w, t, u, skill);
    if m <= 0 {
        return 0;
    }
    let Some(at) = tpos(w, u) else {
        return 0;
    };
    let s = count_or_range(w, t, u, skill, lvl);
    zigzag(w, t, u, m, at, skill, lvl, s, v, ring);
    1
}

// -----------------------------------------------------------------------

/// srvdo 38 Fists of Fire, Blades of Ice `srvprgfunc2`.
pub fn prog_burst<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(f) = rec(t, skill).map(|r| r.aurafilter) else {
        return 0;
    };
    if let Some(tg) = target(w, u) {
        apply_melee(w.combat(), ct, u, tg);
    }
    let Some(at) = tpos(w, u) else {
        return 0;
    };
    let r = count_or_range(w, t, u, skill, lvl);
    let mut record = DamageRecord::default();
    roll_physical(w, t, u, &mut record, skill, lvl);
    roll_elemental(w, t, u, &mut record, skill, lvl);
    record.result |= 1;
    record.hit_flags |= 1;
    scan_unit(w, t, ct, u, at, r, f, false, &mut |w, x| {
        // Callback: the area-damage unit step on a copy.
        area_hit(w.combat(), ct, u, x, &record);
        1
    });
    1
}

/// srvdo 39 Fists of Fire, Blades of Ice `srvprgfunc3`.
pub fn prog_scatter<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let v = w.seed(u).step();
    if rec(t, skill).is_none() {
        return 0;
    }
    let Some((tx, ty)) = tpos(w, u) else {
        return 0;
    };
    let m = prog_missile(w, t, u, skill);
    if m <= 0 {
        return 0;
    }
    let r = count_or_range(w, t, u, skill, lvl);
    let mut req = MissileRequest {
        flags: 1,
        skill,
        level: lvl,
        ..MissileRequest::new(u, m)
    };
    let mut s = Seed::init_low(v);
    let r2 = r.wrapping_mul(r);
    for _ in 0..r2.max(0) {
        let a = r.wrapping_sub(s.roll(r.wrapping_mul(2)) as i32);
        let b = r.wrapping_sub(s.roll(r.wrapping_mul(2)) as i32);
        if a.wrapping_mul(a).wrapping_add(b.wrapping_mul(b)) > r2 {
            continue;
        }
        let (x, y) = (tx.wrapping_add(a), ty.wrapping_add(b));
        if room_at_of(w, u, (x, y)).is_none() {
            continue;
        }
        req.x = x;
        req.y = y;
        w.spawn_missile(req);
    }
    1
}

// -----------------------------------------------------------------------

/// srvdo 40 Royal Strike `srvprgfunc1`.
pub fn royal_meteor<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(at) = tpos(w, u) else {
        return 0;
    };
    if lvl <= 0 {
        return 0;
    }
    let m = prog_missile(w, t, u, skill);
    if m <= 0 {
        return 0;
    }
    missile_at(w, u, skill, lvl, m, at);
    1
}

/// srvdo 41 Royal Strike `srvprgfunc3`.
pub fn royal_shards<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let mut s = Seed::init_low(w.seed(u).step());
    if rec(t, skill).is_none() {
        return 0;
    }
    let Some((tx, ty)) = tpos(w, u) else {
        return 0;
    };
    let m = prog_missile(w, t, u, skill);
    if m <= 0 {
        return 0;
    }
    let n = prog_count(w, t, u, skill, lvl);
    if n == 0 {
        return 0;
    }
    let mut req = MissileRequest {
        flags: 3,
        x: tx,
        y: ty,
        skill,
        level: lvl,
        ..MissileRequest::new(u, m)
    };
    for _ in 0..n.max(0) {
        let mut a = (s.step() % 40) as i32 - 20;
        let b = (s.step() % 40) as i32 - 20;
        if a == 0 && b == 0 {
            a = 20;
        }
        req.target_x = a;
        req.target_y = b;
        if let Some(mm) = w.spawn_missile(req) {
            w.effect(BodyEffect::MissileData28 {
                missile: mm,
                v: s.lo as i32,
            });
            let packed = ((b as u32 & 0xFFFF) << 16) | (a as u32 & 0xFFFF);
            w.effect(BodyEffect::MissileData2C {
                missile: mm,
                v: packed as i32,
            });
        }
    }
    1
}

// ---------------------------------------------------------------- –

/// The state function "end": delete the unit's type-5 timers with
/// argument skill; 0.
fn state_end<W: BodyWorld>(w: &mut W, u: W::Unit, skill: i32) -> i32 {
    w.delete_timers(u, 5, skill);
    0
}

/// Steps 1–3 of the state functions; `None` = end.
fn state_fn_head<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> Option<d2_data::tables::Skills> {
    let r = rec(t, skill)?.clone();
    let a = s16(r.aurastate);
    if lvl <= 0 || !state_ok(w, a) {
        return None;
    }
    if !w.has_state(u, a as u16) {
        return None;
    }
    let town = w.unit_room(u).is_some_and(|room| w.room_in_town(room));
    if !r.intown && town {
        if let Some(l) = w.state_list(u, a) {
            w.detach_free(u, l);
        }
        w.state_on(u, a, false);
        return None;
    }
    Some(r)
}

/// Re-arm and the room test (step 6): false = return 0.
fn state_fn_rearm<W: BodyWorld>(w: &mut W, u: W::Unit, skill: i32, lvl: i32, p4: i32) -> bool {
    w.delete_timers(u, 5, skill);
    // The state acts again `Param4` base ticks later.
    w.schedule(u, 5, super::after(w, p4), skill, lvl);
    w.unit_room(u).is_some_and(|room| !w.room_in_town(room))
}

/// srvdo 145 Hurricane state function.
pub fn hurricane_state<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = state_fn_head(w, t, u, skill, lvl) else {
        return state_end(w, u, skill);
    };
    let range = eval(w, t, u, r.aurarangecalc, skill, lvl);
    if range <= 0 {
        return state_end(w, u, skill);
    }
    if !state_fn_rearm(w, u, skill, lvl, r.param4 as i32) {
        return 0;
    }
    let mut record = DamageRecord::default();
    roll_physical(w, t, u, &mut record, skill, lvl);
    roll_elemental(w, t, u, &mut record, skill, lvl);
    record.result |= r.resultflags;
    record.hit_flags |= r.hitflags;
    area_damage(w, t, ct, u, (0, 0), range, &record, r.aurafilter);
    1
}

/// srvdo 146 Armageddon state function.
pub fn armageddon_state<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = state_fn_head(w, t, u, skill, lvl) else {
        return state_end(w, u, skill);
    };
    let m = prog_missile(w, t, u, skill);
    if m <= 0 {
        return state_end(w, u, skill);
    }
    let Some(e) = w.find_entry(u, skill) else {
        return state_end(w, u, skill);
    };
    let range = eval(w, t, u, r.aurarangecalc, skill, lvl);
    if range <= 0 {
        return state_end(w, u, skill);
    }
    if !state_fn_rearm(w, u, skill, lvl, r.param4 as i32) {
        return 0;
    }
    let p1 = w.entry_param(u, &e, 1);
    *w.seed(u) = Seed::init_low(p1 as u32);
    let v = w.seed(u).roll(1_000_000) as i32;
    w.set_entry_param_of(u, &e, 1, v);
    let (ux, uy) = w.position(u);
    let n = range.wrapping_mul(2).wrapping_add(1);
    let mut hit = None;
    for _ in 0..5 {
        let x = ux
            .wrapping_add(w.seed(u).roll(n) as i32)
            .wrapping_sub(range);
        let y = uy
            .wrapping_add(w.seed(u).roll(n) as i32)
            .wrapping_sub(range);
        if !w.line_clear(u, (x, y), 0x805) {
            continue;
        }
        let Some(room) = room_at_of(w, u, (x, y)) else {
            continue;
        };
        if w.point_collides(room, (x, y), 1) {
            continue;
        }
        hit = Some((x, y));
        break;
    }
    let Some((x, y)) = hit else {
        return 0;
    };
    missile_at(w, u, skill, lvl, m, (x, y));
    w.effect(BodyEffect::MsgA3 {
        u,
        target: None,
        skill,
        lvl,
        x,
        y,
        v: 0,
    });
    1
}

/// srvdo 147 Attached state function.
pub fn attached_state<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    if let Some(o) = source_of(w, u) {
        w.effect(BodyEffect::PathFollow { u, from: o });
    }
    1
}

// ------------------------------------------------------------------

/// srvdo 151 Chain Lightning item effect.
pub fn chain_lightning_item<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some((m, calc1)) = rec(t, skill).map(|r| (s16(r.srvmissilea), r.calc1)) else {
        return 0;
    };
    if !missile_ok(t, m) {
        return 0;
    }
    flag_40(w, u);
    let n = eval(w, t, u, calc1, skill, lvl);
    let (ux, uy) = w.position(u);
    let d = match target(w, u) {
        Some(tg) => {
            let (x, y) = w.position(tg);
            (x.wrapping_sub(ux), y.wrapping_sub(uy))
        }
        None => (0, 0),
    };
    let Some(mm) = skill_missile_unit(w, m, u, skill, lvl, d, (0, 0), false, false) else {
        return 0;
    };
    w.effect(BodyEffect::MissileData28 { missile: mm, v: n });
    1
}

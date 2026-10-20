//! Batch 4 bodies, first part: the monster skill
//! slots used by several skills (Inferno channels, teleport, scrolls,
//! hireling missiles, nests, corpse cyclers, family bolts, heals,
//! resurrects, frenzy) and those of one skill up to ArcaneTower (Throw,
//! Unsummon, Fire Hit, maggots, Andariel's spray, Jump, Swarm Move, Quick
//! Strike, gargoyle traps, Submerge / Emerge, FetishAura, Diablo's
//! skills, desert turrets, arcane towers).

use super::b3_lvl30::walk_velocity;
use super::b4_helpers::*;
use super::effects::{BodyEffect, PathOp};
use super::helpers::*;
use super::helpers2::*;
use super::helpers3::*;
use super::{init_cb, BodyWorld, MissileRequest, MonsterSpawn};
use crate::combat::{
    apply, apply_melee, free_records, pct, start_combat, CombatTables, CombatWorld, DamageRecord,
};
use crate::skills::use_::start_core_of;
use crate::skills::{elem_len, roll_elemental, roll_physical, SkillTables};
use crate::units::UnitType;

/// Free T's combat records with (game, unit, T).
pub fn free_combat<W: BodyWorld>(w: &mut W, u: W::Unit, tg: W::Unit) {
    free_records(w.combat(), u, tg);
}

/// The room at (x, y) searched from the unit's room.
pub fn room_at_of<W: BodyWorld>(w: &W, u: W::Unit, (x, y): (i32, i32)) -> Option<W::Room> {
    let r = w.unit_room(u)?;
    w.room_at(r, x, y)
}

// -----------------------------------------------------------------

/// srvst 53 MonInferno; with `calc1` (not `calc2`) =
/// srvst 59 Imp Inferno.
pub fn mon_inferno_start<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
    calc1: bool,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let c = if calc1 { r.calc1 } else { r.calc2 };
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    // The channel's end, `c` base ticks from now (at least 1).
    let n = eval(w, t, u, c, skill, lvl).max(1);
    let v = super::after(w, n);
    w.set_entry_param_of(u, &e, 1, v);
    if !w.has_state(u, st::INFERNO as u16) {
        w.state_on(u, st::INFERNO, true);
    }
    1
}

// -----------------------------------------------------------------

/// srvdo 95 MonInferno, also srvdo 152 DiabLight.
pub fn mon_inferno<W: BodyWorld>(
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
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    let m = s16(r.srvmissilea);
    if !missile_ok(t, m) {
        return 0;
    }
    let (x, y) = w.position(u);
    let mut req = MissileRequest {
        flags: 0x25,
        x,
        y,
        level: lvl,
        velocity: missile_velocity(t, m, lvl),
        ..MissileRequest::new(u, m)
    };
    let Some((tx, ty)) = tpos(w, u) else {
        channel_end(w, ct, u);
        return 0;
    };
    req.target_x = tx;
    req.target_y = ty;
    let Some(mm) = w.spawn_missile(req) else {
        channel_end(w, ct, u);
        return 0;
    };
    channel_frames(w, t, u, Some(mm), skill, lvl);
    channel_anim(w, ct, &r, u);
    let f = w.frame();
    if f < w.entry_param(u, &e, 1) && w.has_state(u, st::INFERNO as u16) {
        let n = eval(w, t, u, r.calc3, skill, lvl).max(1);
        w.delete_timers(u, 1, 0);
        w.delete_timers(u, 0, 0);
        w.schedule(u, 0, f.wrapping_add(super::ticks(w, n)), 0, 0);
        return 1;
    }
    channel_end(w, ct, u);
    1
}

// -----------------------------------------------------------------

/// srvdo 98 MonTeleport.
pub fn mon_teleport<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    let at = w.path_target_point(u);
    let mut room = room_at_of(w, u, at);
    if room.is_none() {
        if let Some(o) = w.minion_owner(u) {
            room = w.unit_room(o);
        }
    }
    if !w.place_unit(u, room, at) {
        return 0;
    }
    w.queue_update(u);
    let c = w.unit_c8(u);
    w.set_unit_c8(u, c | 0x1_0000);
    1
}

// -----------------------------------------------------------------

/// srvdo 113 Scroll / Book.
pub fn scroll_book<W: BodyWorld>(w: &mut W, u: W::Unit, skill: i32) -> i32 {
    if !w.has_inventory(u) {
        return 0;
    }
    for (i, kind) in w.inventory_nodes(u) {
        let matched = if w.item_is(i, 18) {
            w.book_skills(i).is_some_and(|(_, b)| b == skill)
                && w.item_stat_of(i, stat::QUANTITY) > 0
        } else if w.item_is(i, 22) {
            w.book_skills(i).is_some_and(|(s, _)| s == skill)
        } else {
            false
        };
        if matched {
            return match kind {
                1 | 2 => {
                    w.effect(BodyEffect::UseItem { u, item: i, kind });
                    1
                }
                _ => 0,
            };
        }
    }
    0
}

// -----------------------------------------------------------------

/// srvdo 110 Hireable / Rogue missile.
pub fn hireable_missile<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some((ma, lob)) = rec(t, skill).map(|r| (s16(r.srvmissilea), r.lob)) else {
        return 0;
    };
    if target(w, u).is_none() {
        return 0;
    }
    flag_40(w, u);
    let (m, l2) = bow_missile(w, u);
    let lvl = l2.unwrap_or(lvl);
    if m != 0 && m != 31 {
        if t.missile(m).is_none() {
            return 0;
        }
        skill_missile(w, m, u, skill, lvl, (0, 0), (0, 0), false, false);
        return 1;
    }
    if !missile_ok(t, ma) || t.missile(ma).is_none() {
        return 0;
    }
    skill_missile(w, ma, u, skill, lvl, (0, 0), (0, 0), false, lob);
    1
}

// -----------------------------------------------------------------------

/// srvst 49 Nest, EvilHutSpawner.
pub fn nest_start<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
) -> i32 {
    if rec(t, skill).is_none() {
        return 0;
    }
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    let (c, mode, (x, y)) = spawn_class(w, t, ct, u, skill);
    if c < 0 {
        return 0;
    }
    for (i, v) in [(1, c), (2, x), (3, y), (4, mode)] {
        w.set_entry_param_of(u, &e, i, v);
    }
    if let Some(room) = room_at_of(w, u, (x, y)) {
        w.effect(BodyEffect::PatternStampN {
            room,
            x,
            y,
            pattern: 1,
            mask: 0x100,
        });
    }
    set_uninterruptable(w, u, true);
    w.delete_timers(u, 1, 0);
    1
}

/// srvdo 91 Nest, EvilHutSpawner.
pub fn nest<W: BodyWorld>(w: &mut W, t: &SkillTables, u: W::Unit, skill: i32) -> i32 {
    let Some(ov) = rec(t, skill).map(|r| s16(r.sumoverlay)) else {
        return 0;
    };
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    w.delete_timers(u, 1, 0);
    set_uninterruptable(w, u, false);
    let [c, x, y, mode] = [1, 2, 3, 4].map(|i| w.entry_param(u, &e, i));
    let Some(room) = room_at_of(w, u, (x, y)) else {
        return 0;
    };
    if x == 0 || y == 0 {
        return 0;
    }
    w.effect(BodyEffect::PatternClearN {
        room,
        x,
        y,
        pattern: 1,
        mask: 0x100,
    });
    let Some(m) = w.spawn_monster(MonsterSpawn::At {
        room,
        x,
        y,
        class: c,
        mode,
        spread: 1,
        flags: 0,
    }) else {
        return 0;
    };
    flags_or(w, m, 0x402_0000);
    overlay_if(w, m, ov, 0, false);
    1
}

// -----------------------------------------------------------------

/// srvst 63 CorpseCycler, VineCycler.
pub fn corpse_cycler<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    if w.unit_type(u) != UnitType::Monster {
        return 0;
    }
    let Some(m) = rec(t, skill).map(|r| s16(r.srvmissilea)) else {
        return 0;
    };
    if !(1..t.missiles.len() as i32).contains(&m) {
        return 0;
    }
    let Some(o) = w.minion_owner(u) else {
        return 0;
    };
    let Some(tg) = target(w, u) else {
        return 0;
    };
    if w.has_state(tg, st::CORPSE_NOSELECT as u16) {
        return 0;
    }
    w.state_on(tg, st::CORPSE_NOSELECT, true);
    w.queue_update(tg);
    w.spawn_missile(MissileRequest {
        origin: Some(tg),
        skill,
        level: lvl,
        ..MissileRequest::new(o, m)
    });
    w.effect(BodyEffect::MsgA3 {
        u,
        target: Some(tg),
        skill,
        lvl: 1,
        x: 0,
        y: 0,
        v: 0,
    });
    1
}

// -----------------------------------------------------------------

/// srvdo 85 UnHolyBolt, ShamanFire.
pub fn family_bolt<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(m0) = rec(t, skill).map(|r| s16(r.srvmissilea)) else {
        return 0;
    };
    if !missile_ok(t, m0) {
        return 0;
    }
    flag_40(w, u);
    let m = m0.wrapping_add(w.chain_position(w.class_id(u)));
    if !missile_ok(t, m) {
        return 0;
    }
    skill_missile(w, m, u, skill, lvl, (0, 0), (0, 0), false, false);
    1
}

// ------------------------------------------------------------------

/// srvdo 96 ZakarumHeal, Bestow.
pub fn zakarum_heal<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some((c1, c2)) = rec(t, skill).map(|r| (r.calc1, r.calc2)) else {
        return 0;
    };
    let Some(tg) = target(w, u) else {
        return 0;
    };
    flag_40(w, u);
    let hi = eval(w, t, u, c2, skill, lvl).clamp(0, 100);
    let lo = eval(w, t, u, c1, skill, lvl).max(0).min(hi);
    let p = lo.wrapping_add(w.seed(u).roll(hi.wrapping_sub(lo)) as i32);
    let m = w.stat_max(tg, sid::LIFE);
    let v = pct(m, p, 100).wrapping_add(w.stat(tg, sid::LIFE, 0));
    w.set_stat(tg, sid::LIFE, v.max(1).min(m));
    1
}

// ------------------------------------------------------------------

/// srvdo 97 Resurrect, Resurrect2.
pub fn resurrect<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
) -> i32 {
    let Some(ov) = rec(t, skill).map(|r| s16(r.srvoverlay)) else {
        return 0;
    };
    let Some(tg) = target(w, u) else {
        return 0;
    };
    flag_40(w, u);
    if w.unit_type(tg) != UnitType::Monster || !matches!(w.mode(tg), 0 | 12) {
        return 0;
    }
    if w.has_group(tg, GROUP_HIDE) {
        return 0;
    }
    let Some(room) = w.unit_room(tg) else {
        return 0;
    };
    let mask = if w.unit_type(tg) == UnitType::Player {
        0x1C09
    } else {
        0x3C01
    };
    let at = w.position(tg);
    if w.pattern_collides(room, at, tg, mask) {
        return 0;
    }
    w.path_op(u, PathOp::TargetUnit(Some(tg)));
    revive(w, tg);
    if w.unit_type(tg) == UnitType::Monster {
        let m = resurrect_mode(w, t, ct, tg, 1);
        w.effect(BodyEffect::ModeRequestBuild { m: tg, mode: m });
        resurrect_mode(w, t, ct, tg, 1);
        w.effect(BodyEffect::ModeRequestSend {
            m: tg,
            mode: m,
            flag: m != 14,
        });
    }
    overlay_if(w, tg, ov, 0, false);
    1
}

// -------------------------------------------------------------------------

/// srvst 64 MonFrenzy.
pub fn mon_frenzy_start<W: BodyWorld>(w: &W, u: W::Unit) -> i32 {
    i32::from(target(w, u).is_some())
}

/// srvdo 109 MonFrenzy, BloodLordFrenzy.
pub fn mon_frenzy<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    if w.used_skill(u).is_none() {
        return 0;
    }
    let Some(tg) = target(w, u) else {
        return 0;
    };
    free_combat(w, u, tg);
    flag_40(w, u);
    if next_event(w, u) != 0 {
        start_core_of(w, t, u, skill, lvl);
    }
    mon_swing(w, t, ct, u, Some(tg), skill, lvl);
    if e_param(w, u, 1) != 0 {
        frenzy_charge(w, t, u, skill, lvl);
    }
    1
}

// -----------------------------------------------------------------

/// srvdo 4 Unsummon.
pub fn unsummon_do<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    if w.used_skill(u).is_none() {
        return 0;
    }
    let guid = e_param(w, u, 1);
    w.effect(BodyEffect::PetRemove {
        owner: u,
        guid,
        kill: true,
    });
    1
}

// -----------------------------------------------------------------------

/// srvst 42 Fire Hit.
pub fn fire_hit_start<W: BodyWorld>(
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
    if w.unit_type(u) == UnitType::Player {
        return super::starts::bash(w, t, ct, u, skill, lvl);
    }
    let mut record = skill_melee(w, t, ct, u, tg, skill, lvl);
    if record.result & 1 != 0 {
        hit_fill(&r, &mut record);
        mode_damage(w, t, ct, u, 8);
    }
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, 128);
    1
}

/// srvdo 83 Fire Hit.
pub fn fire_hit<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(tg) = target(w, u) else {
        return 0;
    };
    apply_melee(w.combat(), ct, u, tg);
    free_combat(w, u, tg);
    start_core_of(w, t, u, skill, lvl)
}

// ---------------------------------------------------------------- –

/// srvst 43 MaggotEgg.
pub fn maggot_egg_start<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    flags_clear(w, u, 0xE);
    w.effect(BodyEffect::DeadFootprint(u));
    1
}

/// srvdo 84 MaggotEgg.
pub fn maggot_egg<W: BodyWorld>(
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
    let (c, mode, _) = spawn_class(w, t, ct, u, skill);
    if c < 0 {
        return 0;
    }
    if w.used_skill(u).is_none() {
        return 0;
    }
    w.set_seq_speed(u, 0);
    let n = eval(w, t, u, calc1, skill, lvl);
    if n > 0 {
        let near = |mode, spread| MonsterSpawn::Near {
            unit: u,
            class: c,
            mode,
            spread,
            flags: 0,
        };
        let mut k = 0;
        if let Some(m) = w.spawn_monster(near(mode, -1)) {
            flags_or(w, m, 0x400_0000);
            k = 1;
        }
        let (mut s, mut tries) = (1, 0);
        while k < n && s < 5 && tries < n.wrapping_mul(2) {
            match w.spawn_monster(near(8, s)) {
                Some(m) => {
                    flags_or(w, m, 0x400_0000);
                    k += 1;
                }
                None => s += 1,
            }
            tries += 1;
        }
    }
    w.effect(BodyEffect::KillBy {
        u,
        killer: Some(u),
        b: 1,
    });
    1
}

/// srvst 44 MagottUp.
pub fn maggot_up<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    let at = w.position(u);
    let room = w.unit_room(u);
    if !w.place_unit(u, room, at) {
        return 0;
    }
    flags_or(w, u, 0xE);
    let (x, y) = w.position(u);
    if let Some(room) = w.unit_room(u) {
        w.effect(BodyEffect::PatternStampN {
            room,
            x,
            y,
            pattern: 1,
            mask: 0x1000,
        });
    }
    1
}

/// srvst 45 MagottDown.
pub fn maggot_down_start<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    flags_clear(w, u, 0xE);
    let (x, y) = w.position(u);
    if let Some(room) = w.unit_room(u) {
        w.effect(BodyEffect::PatternClearN {
            room,
            x,
            y,
            pattern: 5,
            mask: 0x1000,
        });
    }
    1
}

/// srvdo 86 MagottDown.
pub fn maggot_down<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(calc1) = rec(t, skill).map(|r| r.calc1) else {
        return 0;
    };
    flag_40(w, u);
    if w.anim_frame(u) >> 8 <= 0 {
        w.set_seq_speed(u, 0);
    }
    let p = eval(w, t, u, calc1, skill, lvl);
    if p > 0 {
        let h = w.stat(u, sid::LIFE, 0);
        let m = w.stat_max(u, sid::LIFE);
        w.set_stat(u, sid::LIFE, h.wrapping_add(pct(h, p, 100)).min(m));
    }
    1
}

/// MagottLay egg offsets (pair indices by `dir8`).
pub const LAY_PAIRS: [usize; 8] = [10, 8, 22, 20, 18, 16, 14, 12];

/// srvdo 87 MagottLay.
pub fn maggot_lay<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
) -> i32 {
    if rec(t, skill).is_none() {
        return 0;
    }
    let (c, mode, _) = spawn_class(w, t, ct, u, skill);
    if c < 0 {
        return 0;
    }
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let tp = w.position(tg);
    let k = dir8(w.dir64(u, tp));
    let (dx, dy) = pair(LAY_PAIRS[k as usize]);
    let (ux, uy) = w.position(u);
    let (x, y) = (ux.wrapping_add(dx), uy.wrapping_add(dy));
    w.set_action_frame(u, 0);
    let Some(room) = w.unit_room(u) else {
        return 0;
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
        return 0;
    };
    flags_or(w, m, 0x400_0000);
    1
}

// ------------------------------------------------------------------

/// AndrialSpray start pairs by `dir8`.
pub const SPRAY_START: [usize; 8] = [29, 28, 27, 26, 25, 24, 31, 30];

/// The sweep offsets (8 × 9, by `dir8` and the clamped
/// frame; `None` = 99).
pub const SPRAY_SWEEP: [[Option<(i32, i32)>; 9]; 8] = {
    const fn s(x: i32, y: i32) -> Option<(i32, i32)> {
        Some((x, y))
    }
    [
        [
            s(-3, 3),
            s(-2, 2),
            s(-1, 2),
            s(-1, 1),
            None,
            s(1, -1),
            s(2, -1),
            s(2, -2),
            s(3, -3),
        ],
        [
            s(-3, 0),
            s(-2, 0),
            s(-2, 1),
            s(-1, 0),
            None,
            s(1, 0),
            s(2, 1),
            s(2, 0),
            s(3, 0),
        ],
        [
            s(-3, -3),
            s(-2, -2),
            s(-2, -1),
            s(-1, -1),
            None,
            s(1, 1),
            s(1, 2),
            s(2, 2),
            s(3, 3),
        ],
        [
            s(0, -3),
            s(0, -2),
            s(-1, -2),
            s(0, -1),
            None,
            s(0, 1),
            s(-1, 2),
            s(0, 2),
            s(0, 3),
        ],
        [
            s(3, -3),
            s(2, -2),
            s(1, -2),
            s(1, -1),
            None,
            s(-1, 1),
            s(-2, 1),
            s(-2, 2),
            s(-3, 3),
        ],
        [
            s(3, 0),
            s(2, 0),
            s(1, -1),
            s(1, 0),
            None,
            s(-1, 0),
            s(-1, -1),
            s(-2, 0),
            s(-3, 0),
        ],
        [
            s(3, 3),
            s(2, 2),
            s(2, 1),
            s(1, 1),
            None,
            s(-1, -1),
            s(-1, -2),
            s(-2, -2),
            s(-3, -3),
        ],
        [
            s(0, 3),
            s(0, 2),
            s(1, 2),
            s(0, 1),
            None,
            s(0, -1),
            s(1, -2),
            s(0, -2),
            s(0, -3),
        ],
    ]
};

/// srvdo 88 AndrialSpray.
pub fn andrial_spray<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(m) = rec(t, skill).map(|r| s16(r.srvmissilea)) else {
        return 0;
    };
    if !missile_ok(t, m) {
        return 0;
    }
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    let (mut x, mut y) = (w.entry_param(u, &e, 1), w.entry_param(u, &e, 2));
    if x == 0 || y == 0 {
        let Some(p) = tpos(w, u) else {
            return 0;
        };
        (x, y) = p;
    }
    let k = dir8(w.dir64(u, (x, y))) as usize;
    let (ux, uy) = w.position(u);
    let (sx, sy) = pair(SPRAY_START[k]);
    let (mut px, mut py) = (ux.wrapping_add(sx), uy.wrapping_add(sy));
    let f = ((w.anim_frame(u) >> 8) - 4).clamp(0, 8) as usize;
    if let Some((dx, dy)) = SPRAY_SWEEP[k][f] {
        px = px.wrapping_add(dx);
        py = py.wrapping_add(dy);
    }
    w.spawn_missile(MissileRequest {
        flags: 0x20,
        origin: Some(u),
        target_x: px,
        target_y: py,
        level: lvl,
        ..MissileRequest::new(u, m)
    });
    1
}

// -------------------------------------------------------------------------

/// srvst 47 Jump.
pub fn jump_start<W: BodyWorld>(
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
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    if !w.has_path(u) || w.has_state(u, st4::FREEZE as u16) {
        return 0;
    }
    let tg = target(w, u);
    let (x, y) = match tg {
        None => {
            w.set_entry_param_of(u, &e, 4, -1);
            w.path_target_point(u)
        }
        Some(tg) => {
            let ((tx, ty), (ux, uy)) = (w.position(tg), w.position(u));
            (
                tx.wrapping_mul(2).wrapping_sub(ux),
                ty.wrapping_mul(2).wrapping_sub(uy),
            )
        }
    };
    let Some(room) = room_at_of(w, u, (x, y)) else {
        return 0;
    };
    if let Some(tg) = tg {
        let mut record = skill_melee(w, t, ct, u, tg, skill, lvl);
        if record.result & 1 != 0 {
            hit_fill(&r, &mut record);
            record.enh_pct = eval(w, t, u, r.calc1, skill, lvl);
            roll_elemental(w, t, u, &mut record, skill, lvl);
        }
        start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, 128);
        let (ty, g) = ident(w, tg);
        w.set_entry_param_of(u, &e, 3, ty.index() as i32);
        w.set_entry_param_of(u, &e, 4, g as i32);
    }
    w.effect(BodyEffect::PatternStamp {
        room,
        x,
        y,
        u,
        mask: 0x100,
    });
    set_uninterruptable(w, u, true);
    w.set_entry_param_of(u, &e, 1, x);
    w.set_entry_param_of(u, &e, 2, y);
    w.set_entry_flags(u, &e, 0x80);
    1
}

/// The unit of (E param 3 type, E param 4 GUID).
fn e_unit<W: BodyWorld>(w: &W, u: W::Unit, ty: u8, guid: u8) -> Option<W::Unit> {
    let ty = e_param(w, u, ty);
    let g = e_param(w, u, guid);
    w.find_unit(ty as u32, g as u32)
}

/// srvdo 89 Jump.
pub fn jump<W: BodyWorld>(w: &mut W, ct: &CombatTables, u: W::Unit) -> i32 {
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    if !w.has_path(u) {
        return 0;
    }
    w.delete_timers(u, 1, 0);
    if w.action_frame(u) == 2 {
        w.set_action_frame(u, 0);
        if let Some(k) = e_unit(w, u, 3, 4) {
            apply_melee(w.combat(), ct, u, k);
        }
        return 1;
    }
    let g = w.entry_flags(u, &e);
    let (x, y) = (w.entry_param(u, &e, 1), w.entry_param(u, &e, 2));
    if g & 0x100 != 0 {
        let b = base_id(w, ct, u);
        if w.position(u) != (x, y) {
            w.set_frame_event_index(u, if b == 78 { 8 } else { 0 });
            let c = w.frame_count(u);
            w.set_frame_count(u, (c & !0xFF).wrapping_add(0x100));
            return 1;
        }
        w.set_entry_flags(u, &e, 0);
        set_uninterruptable(w, u, false);
        if let Some(room) = w.unit_room(u) {
            w.effect(BodyEffect::PatternClear {
                room,
                x,
                y,
                u,
                mask: 0x100,
            });
        }
        w.path_op(u, PathOp::FootprintMask(0x100));
        w.path_op(u, PathOp::MoveMask(0x3C01));
        // Edge case 1: path type 101, past the type table.
        w.path_op(u, PathOp::Type(101));
        if b != 78 {
            return 1;
        }
        let Some(k) = e_unit(w, u, 3, 4) else {
            return 1;
        };
        w.set_entry_flags(u, &e, 1);
        w.set_frame_event_index(u, 12);
        let c = w.frame_count(u);
        w.set_frame_count(u, (c & !0xFF).wrapping_add(0x100));
        let (kx, ky) = w.position(k);
        for op in [
            PathOp::SnapCenter,
            PathOp::Steps(5),
            PathOp::Op649070(1),
            PathOp::TargetUnit(None),
            PathOp::TargetPoint(
                x.wrapping_mul(3).wrapping_sub(kx.wrapping_mul(2)),
                y.wrapping_mul(3).wrapping_sub(ky.wrapping_mul(2)),
            ),
            PathOp::Type(8),
            PathOp::Op648E40(5),
            PathOp::Compute,
        ] {
            w.path_op(u, op);
        }
        return 1;
    }
    if g & 0x80 != 0 {
        w.set_entry_flags(u, &e, 0x101);
        let v = walk_velocity(w, ct, u);
        for op in [
            PathOp::MoveMask(0),
            PathOp::FootprintMask(0),
            PathOp::TargetPoint(x, y),
            PathOp::Type(9),
            PathOp::Velocity(v),
            PathOp::Compute,
        ] {
            w.path_op(u, op);
        }
        return 1;
    }
    w.set_entry_flags(u, &e, 0);
    1
}

// -------------------------------------------------------------------------

/// srvst 48 Swarm Move.
pub fn swarm_move_start<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    if target(w, u).is_none() || !w.has_path(u) {
        return 0;
    }
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    w.path_op(u, PathOp::Steps(5));
    w.path_op(u, PathOp::Type(2));
    w.path_op(u, PathOp::Compute);
    if w.path_point_count(u) == 0 {
        w.path_op(u, PathOp::Type(1));
        w.path_op(u, PathOp::Compute);
        if w.path_point_count(u) == 0 {
            return 0;
        }
    }
    w.set_entry_flags(u, &e, 1);
    1
}

/// E flags 2: the move ended (the skill use sets it); "E flags bit 2"
/// in the bodies' comments.
pub const MOVE_ENDED: u32 = 2;

/// srvdo 90 Swarm Move.
pub fn swarm_move<W: BodyWorld>(
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
    let v = if w.entry_flags(u, &e) & MOVE_ENDED != 0 {
        w.set_entry_flags(u, &e, 0);
        eval(w, t, u, c2, skill, lvl)
    } else {
        eval(w, t, u, c1, skill, lvl)
    };
    w.set_frame_event_index(u, v);
    1
}

// -------------------------------------------------------------------------

/// srvst 50 Quick Strike.
pub fn quick_strike_start<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
) -> i32 {
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let mut record = DamageRecord::default();
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, 128);
    1
}

/// The monstats missile of the unit's mode with (unit, 1) (step 3).
pub fn mode_missile<W: BodyWorld>(w: &W, ct: &CombatTables, u: W::Unit) -> i32 {
    mode_missile_of(w, ct, u, true)
}

/// With `check` the action-frame tests of
/// Come first; the column by mode follows. With 0 (the
/// monster attack event of a non-moving mode) the
/// tests are skipped.
pub fn mode_missile_of<W: BodyWorld>(w: &W, ct: &CombatTables, u: W::Unit, check: bool) -> i32 {
    if check {
        if w.action_frame(u) == 0 {
            return -1;
        }
        let (cur, speed) = (w.anim_frame(u), w.anim_speed(u));
        if !w.action_event_between(u, cur.wrapping_sub(speed) >> 8, cur >> 8) {
            return -1;
        }
    }
    let Some(ms) = ct.monstats(w.class_id(u)) else {
        return -1;
    };
    let col = match w.mode(u) {
        4 => ms.missa1,
        5 => ms.missa2,
        7 => ms.missc,
        8 => ms.misss1,
        9 => ms.misss2,
        10 => ms.misss3,
        11 => ms.misss4,
        14 => ms.misssq,
        _ => return -1,
    };
    s16(col)
}

/// Monster mode missile with (game, unit, moving):
/// m := the mode missile ([`mode_missile_of`] with the moving
/// flag as its argument); m < 0 → 0 (the caller's melee fallback). Level
/// `MonsterSkillBonus`(d) + 1, or a hireling's (unit flag 0x200) total
/// stat 12; created straight with skill 0, no offsets or aim, take ammo
/// 1; a missile made → the quill volley. Returns 1.
pub fn monster_mode_missile<W: BodyWorld>(
    w: &mut W,
    ct: &CombatTables,
    u: W::Unit,
    moving: bool,
) -> i32 {
    let m = mode_missile_of(w, ct, u, moving);
    if m < 0 {
        return 0;
    }
    let lvl = if w.unit_flags(u) & HIRELING != 0 {
        w.stat(u, LEVEL, 0)
    } else {
        let d = w.combat().difficulty();
        ct.difficulty(d)
            .map_or(0, |r| r.monsterskillbonus as i32)
            .wrapping_add(1)
    };
    if skill_missile_unit(w, m, u, 0, lvl, (0, 0), (0, 0), true, false).is_some() {
        quill_volley(w, ct, u, m);
    }
    1
}

/// Unit flag 0x200 (+0xC4 bit 9): a hireling.
const HIRELING: u32 = 0x200;
/// Stat 12 (`level`).
const LEVEL: u16 = 12;
/// `quillrat1`'s `BaseId`.
const QUILLRAT: i32 = 63;
/// The low word of the volley's local seed.
const VOLLEY_SEED_LO: u32 = 0x5345_4953;

/// Quill volley:
/// `aip3` extra spikes of class `m` at level 1 around the target, on a
/// local seed {0x53454953, GUID} (no game draw). Only for `BaseId` 63.
fn quill_volley<W: BodyWorld>(w: &mut W, ct: &CombatTables, u: W::Unit, m: i32) {
    if base_id(w, ct, u) != QUILLRAT {
        return;
    }
    let saved = w.target(u);
    let (x, y) = match saved {
        Some(t) => w.position(t),
        None => w.target_position(u).unwrap_or((0, 0)),
    };
    w.path_op(u, PathOp::TargetUnit(None));
    let g = guid(w, u);
    let mut seed = crate::rng::Seed::new(VOLLEY_SEED_LO, g);
    let d = w.combat().difficulty();
    let n = ct.monstats(w.class_id(u)).map_or(0, |r| {
        let v = match d {
            0 => r.aip3,
            1 => r.aip3_n,
            _ => r.aip3_h,
        };
        i32::from(v as i16)
    });
    let (mut sx, mut sy) = (5, 5);
    for _ in 0..n {
        if seed.step() & 1 != 0 {
            sx = -sx;
        }
        if seed.step() & 1 != 0 {
            sy = -sy;
        }
        w.path_op(u, PathOp::TargetPoint(x + sx, y + sy));
        skill_missile_unit(w, m, u, 0, 1, (0, 0), (0, 0), false, false);
    }
    w.path_op(u, PathOp::TargetUnit(saved));
}

/// srvdo 92 Quick Strike.
pub fn quick_strike<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(ma) = rec(t, skill).map(|r| s16(r.srvmissilea)) else {
        return 0;
    };
    let Some(tg) = target(w, u) else {
        return 0;
    };
    apply_melee(w.combat(), ct, u, tg);
    free_combat(w, u, tg);
    let mut m = ma;
    if m < 0 && w.unit_type(u) == UnitType::Monster {
        m = mode_missile(w, ct, u);
    }
    if !missile_ok(t, m) {
        return 0;
    }
    skill_missile(w, m, u, skill, lvl, (0, 0), (0, 0), true, false);
    1
}

// ------------------------------------------------------------------

/// v moved one step toward `to` `n` times, stopping at `to`.
fn toward(v: i32, to: i32, n: i32) -> i32 {
    let mut v = v;
    for _ in 0..n {
        if v < to {
            v += 1;
        } else if v > to {
            v -= 1;
        }
    }
    v
}

/// srvdo 93 GargoyleTrap.
pub fn gargoyle_trap<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(m) = rec(t, skill).map(|r| s16(r.srvmissilea)) else {
        return 0;
    };
    if !missile_ok(t, m) {
        return 0;
    }
    flag_40(w, u);
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let ((tx, ty), (ux, uy)) = (w.position(tg), w.position(u));
    let (x2, y2) = if tx.wrapping_sub(ux).wrapping_abs() < ty.wrapping_sub(uy).wrapping_abs() {
        (toward(ux, tx, 4), ty)
    } else {
        (tx, toward(uy, ty, 4))
    };
    let (ox, oy) = (x2.wrapping_sub(ux), y2.wrapping_sub(uy));
    w.spawn_missile(MissileRequest {
        flags: 3,
        x: ux.wrapping_add(ox / 6).wrapping_sub(1),
        y: uy.wrapping_add(oy / 6).wrapping_sub(1),
        target_x: ox,
        target_y: oy,
        skill,
        level: lvl,
        ..MissileRequest::new(u, m)
    });
    1
}

// ---------------------------------------------------------------- –

/// srvst 51 Submerge.
pub fn submerge_start<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    flags_clear(w, u, 0xE);
    1
}

/// srvdo 94 Submerge.
pub fn submerge<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    w.set_action_frame(u, 0);
    if w.anim_frame(u) >> 8 <= 0 {
        w.set_seq_speed(u, 0);
    }
    1
}

/// srvst 52 Emerge.
pub fn emerge_start<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    flags_or(w, u, 0xE);
    1
}

fn curse_target_ok<W: BodyWorld>(w: &mut W, u: W::Unit, x: W::Unit) -> bool {
    if w.unit_type(x) == UnitType::Monster
        && (!w.combat().monster_has_mode(x, 2) || w.combat().monster_flag(x, 0x20))
    {
        return false;
    }
    w.unit_flags(x) & 0xE == 0xE && w.is_alive(x) && w.is_hostile(u, x)
}

/// srvdo 111 FetishAura.
pub fn fetish_aura<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(at) = tpos(w, u) else {
        return 0;
    };
    flag_40(w, u);
    // An invalid skill reads a null record: fatal; refused.
    let Some([p1, p2, p3, p4]) = rec(t, skill).map(|_| [1, 2, 3, 4].map(|n| param(t, skill, n)))
    else {
        return 0;
    };
    let r = p4.wrapping_add(lvl.wrapping_mul(2));
    // The finder (as the umod callbacks use it): room R = the casting unit's
    // own room; R none → nothing found.
    let found = match w.unit_room(u) {
        Some(room) => w.unit_find(room, at, r, DEFAULT_FILTER),
        None => Vec::new(),
    };
    for x in found {
        if w.is_hostile(u, x) {
            continue;
        }
        if !matches!(w.class_id(x), 141..=145 | 396..=400) {
            continue;
        }
        if !curse_target_ok(w, u, x) {
            continue;
        }
        apply_state(
            w,
            ct,
            StateRequest {
                source: u,
                target: x,
                skill,
                level: lvl,
                duration: p2.wrapping_mul(lvl),
                stat: stat::ATTACKRATE,
                value: p1.wrapping_add(lvl.wrapping_sub(1).wrapping_mul(p3)),
                state: st4::FETISHAURA,
                callback: 0,
            },
        );
    }
    1
}

// ------------------------------------------------------------------

/// srvdo 99 PrimePoisonNova.
pub fn prime_poison_nova<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill).cloned() else {
        return 0;
    };
    let m = s16(r.srvmissilea);
    let Some((p1, p2)) = t.missile(m).map(|x| (x.param1 as i32, x.param2 as i32)) else {
        return 0;
    };
    flag_40(w, u);
    let (x, y) = w.position(u);
    let mut req = MissileRequest {
        flags: 0x1F,
        origin: Some(u),
        x,
        y,
        level: lvl,
        ..MissileRequest::new(u, m)
    };
    req.loops = eval(w, t, u, r.calc1, skill, lvl);
    req.velocity = p1.wrapping_shl(6);
    let n = eval(w, t, u, r.calc2, skill, lvl).max(1);
    for i in (0..16).step_by(2) {
        req.target_x = BURST_X[i];
        req.target_y = BURST_Y[i];
        w.spawn_missile(req);
    }
    req.velocity = p2.wrapping_shl(6);
    if n > 1 {
        let mut i = 0;
        while i < 15 {
            req.target_x = BURST_X[i as usize + 1];
            req.target_y = BURST_Y[i as usize + 1];
            w.spawn_missile(req);
            i += n;
        }
    }
    1
}

// ---------------------------------------------------------------- –

/// srvdo 100 DiabCold.
pub fn diab_cold<W: BodyWorld>(
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
    flag_40(w, u);
    let Some(tg) = target(w, u) else {
        return 1;
    };
    let mut record = DamageRecord {
        result: 1,
        ..DamageRecord::default()
    };
    hit_fill(&r, &mut record);
    record.freeze_len = elem_len(w, t, Some(u), skill, lvl);
    roll_elemental(w, t, u, &mut record, skill, lvl);
    apply(w.combat(), ct, u, tg, true, &mut record);
    if w.stat(tg, sid::LIFE, 0) == 0 {
        record.result |= 2;
    }
    w.combat().reaction(u, tg, &mut record);
    overlay_if(w, tg, s16(r.srvoverlay), 1, true);
    1
}

/// srvdo 101 FingerMageSpider.
pub fn finger_mage_spider<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill).cloned() else {
        return 0;
    };
    let m = s16(r.srvmissilea);
    if t.missile(m).is_none() {
        return 0;
    }
    flag_40(w, u);
    let (ux, uy) = w.position(u);
    let mut req = MissileRequest {
        flags: 3,
        origin: Some(u),
        x: ux,
        y: uy,
        skill,
        level: lvl,
        ..MissileRequest::new(u, m)
    };
    // Edge case 11: loops without flag 8.
    req.loops = eval(w, t, u, r.calc1, skill, lvl);
    let (px, py) = match target(w, u) {
        Some(tg) => {
            let p = w.position(tg);
            w.path_op(u, PathOp::TurnToward(p.0, p.1));
            p
        }
        None => w.path_target_point(u),
    };
    req.target_x = px.wrapping_sub(ux);
    req.target_y = py.wrapping_sub(uy);
    w.spawn_missile(req);
    1
}

/// srvdo 102 DiabWall.
pub fn diab_wall<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill).cloned() else {
        return 0;
    };
    let m = s16(r.srvmissilea);
    if !missile_ok(t, m) {
        return 0;
    }
    flag_40(w, u);
    let n = eval(w, t, u, r.calc1, skill, lvl);
    let (x, y) = w.position(u);
    let Some((tx, ty)) = tpos(w, u) else {
        return 0;
    };
    let mut req = MissileRequest {
        flags: 0x21,
        x,
        y,
        target_x: tx,
        target_y: ty,
        level: lvl,
        ..MissileRequest::new(u, m)
    };
    for i in 0..n.max(0) {
        req.init = Some((init_cb::DIAB_WALL, i as u32));
        w.spawn_missile(req);
    }
    1
}

// -------------------------------------------------------------------------

/// srvst 54 DiabRun.
pub fn diab_run_start<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    let Some(tg) = target(w, u) else {
        return 0;
    };
    w.set_entry_flags(u, &e, 0);
    let (ty, g) = ident(w, tg);
    w.set_entry_param_of(u, &e, 1, g as i32);
    w.set_entry_param_of(u, &e, 2, ty.index() as i32);
    let (x, y) = w.position(tg);
    w.path_op(u, PathOp::TargetUnit(None));
    w.path_op(u, PathOp::TargetPoint(x, y));
    1
}

/// srvdo 103 DiabRun.
pub fn diab_run<W: BodyWorld>(
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
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    let p = |n| param(t, skill, n);
    if w.entry_flags(u, &e) & MOVE_ENDED != 0 {
        w.set_entry_flags(u, &e, 0);
        w.set_frame_count(u, p(1).wrapping_shl(8));
        w.set_frame_event_index(u, p(2));
        return 1;
    }
    let cur = w.anim_frame(u) >> 8;
    if cur == p(3) {
        if !w.has_path(u) {
            return 0;
        }
        let v = eval(w, t, u, r.calc1, skill, lvl)
            .wrapping_shl(8)
            .max(0x100);
        let vp = w.stat(u, sid::VELOCITYPERCENT, 0);
        w.path_op(u, PathOp::Velocity(pct(v, vp, 100)));
        w.path_op(u, PathOp::Type(1));
        w.path_op(u, PathOp::Compute);
        w.set_entry_flags(u, &e, 1);
        return 1;
    }
    if cur == p(4) {
        w.set_frame_count(u, p(5).wrapping_shl(8));
        w.set_frame_event_index(u, p(6));
        return 1;
    }
    if w.action_frame(u) == 1 {
        let k = e_unit(w, u, 2, 1);
        if let Some(k) = k.filter(|&k| w.combat().in_melee_range(u, k, 0)) {
            w.set_action_frame(u, 0);
            let mut record = DamageRecord::default();
            skill_result(w, t, ct, u, k, skill, lvl, &mut record, 0);
            if record.result & 1 != 0 {
                record.hit_flags = r.hitflags | 0x20;
                if r.hitclass != 0 {
                    record.hit_class = r.hitclass;
                }
                roll_physical(w, t, u, &mut record, skill, lvl);
                roll_elemental(w, t, u, &mut record, skill, lvl);
                apply(w.combat(), ct, u, k, true, &mut record);
                if w.stat(k, sid::LIFE, 0) == 0 {
                    record.result |= 2;
                }
                w.combat().reaction(u, k, &mut record);
            }
        }
    }
    1
}

// ------------------------------------------------------------------

/// The DiabPrison pieces: (dx, dy, kind, class).
pub const PRISON_PIECES: [(i32, i32, i32, i32); 4] = [
    (1, 1, 1, 340),
    (1, -1, 3, 341),
    (-1, -1, 3, 342),
    (-1, 1, 3, 343),
];

/// srvdo 104 DiabPrison.
pub fn diab_prison<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
) -> i32 {
    if rec(t, skill).is_none() {
        return 0;
    }
    let (c, _) = summon_class(w, t, ct, u, skill);
    if c < 0 {
        return 0;
    }
    let k = match target(w, u) {
        Some(k) => Some(k),
        // Open question 6: P +0x10 / +0x12 are the path's
        // target position, read literally as (GUID, type 2).
        None if w.has_path(u) && w.path_target_point(u).1 == 2 => {
            let g = w.path_target_point(u).0;
            w.find_unit(2, g as u32)
        }
        None => return 0,
    };
    let Some(k) = k else {
        return 0;
    };
    let Some(room) = w.unit_room(k) else {
        return 0;
    };
    if w.room_in_town(room) {
        return 0;
    }
    // Prison spawn → pattern spawn.
    if c != 340 {
        return 1;
    }
    let (kx, ky) = w.position(k);
    let mut leader = None;
    for (dx, dy, kind, class) in PRISON_PIECES {
        let (x, y) = (kx.wrapping_add(dx), ky.wrapping_add(dy));
        if kind == 1 {
            leader = w.spawn_monster(MonsterSpawn::Leader {
                room,
                x,
                y,
                class,
                mode: 8,
                spread: -1,
            });
        } else if let Some(l) = leader {
            w.spawn_monster(MonsterSpawn::Minion {
                leader: l,
                x,
                y,
                class,
                mode: 8,
                spread: -1,
            });
        }
    }
    1
}

// -------------------------------------------------------------------------

/// srvdo 105 DesertTurret.
pub fn desert_turret<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill).cloned() else {
        return 0;
    };
    let m = s16(r.srvmissilea);
    if !missile_ok(t, m) {
        return 0;
    }
    flag_40(w, u);
    let Some((tx, ty)) = tpos(w, u) else {
        return 0;
    };
    let (ux, uy) = w.position(u);
    let (mut dx, mut dy) = (tx.wrapping_sub(ux), ty.wrapping_sub(uy));
    let (a, b) = match (dx.signum(), dy.signum()) {
        (0, 0) => return 0,
        (-1, -1) => (2, -2),
        (-1, 0) => (0, -2),
        (-1, _) => (-2, -2),
        (0, -1) => (2, 0),
        (0, _) => (-2, 0),
        (_, -1) => (2, 2),
        (_, 0) => (0, 2),
        (_, _) => (-2, 2),
    };
    let n = eval(w, t, u, r.calc1, skill, lvl);
    let h = n / 2;
    dx = dx.wrapping_sub(h.wrapping_mul(a));
    dy = dy.wrapping_sub(h.wrapping_mul(b));
    let mut req = MissileRequest {
        flags: 3,
        level: lvl,
        ..MissileRequest::new(u, m)
    };
    for _ in 0..n.max(0) {
        // Edge case 14: the x quotient for both coordinates.
        let q = dx / 6;
        req.x = ux.wrapping_add(q);
        req.y = uy.wrapping_add(q);
        req.target_x = dx;
        req.target_y = dy;
        w.spawn_missile(req);
        dx = dx.wrapping_add(a);
        dy = dy.wrapping_add(b);
    }
    1
}

/// srvdo 106 ArcaneTower.
pub fn arcane_tower<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(m) = rec(t, skill).map(|r| s16(r.srvmissilea)) else {
        return 0;
    };
    if !missile_ok(t, m) {
        return 0;
    }
    flag_40(w, u);
    if target(w, u).is_none() {
        return 0;
    }
    let v = missile_velocity(t, m, lvl);
    ring(w, u, u, m, skill, lvl, v);
    1
}

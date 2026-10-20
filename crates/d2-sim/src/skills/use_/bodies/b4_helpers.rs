//! The shared helpers of batch 4:
//! the spawn-column summon class, the Inferno channel pieces, Throw, the
//! monster swing, the next action event, revive in place, the resurrect
//! mode, the direction tables, the weapon skill roll, the stacking state
//! list, imp possess / release, the lightning fan and ring (with their
//! missile init callbacks), the whip helpers and the vine missiles.

use super::effects::{BodyEffect, PathOp};
use super::helpers::*;
use super::helpers2::*;
use super::helpers3::JitterMissile;
use super::{callback, init_cb, BodyWorld, MissileRequest};
use crate::combat::{apply_melee, bonuses, start_combat, CombatTables, CombatWorld, DamageRecord};
use crate::rng::Seed;
use crate::skills::{phys_max, phys_min, roll_elemental, throw_mastery, SkillTables};
use crate::units::UnitType;
use d2_data::tables::Skills;

/// States of batch 4.
pub mod st4 {
    pub const FREEZE: i32 = 1;
    pub const FETISHAURA: i32 = 25;
    pub const PREGNANT: i32 = 110;
    pub const CHANGECLASS: i32 = 142;
    pub const ATTACHED: i32 = 143;
}

/// The `hide` state group.
pub const GROUP_HIDE: usize = 2;

/// Stat 355 `shortparam1`.
pub const SHORTPARAM1: i32 = 355;

/// The monster `BaseId`; −1 without a
/// record or for a non-monster.
pub fn base_id<W: BodyWorld>(w: &W, ct: &CombatTables, u: W::Unit) -> i32 {
    if w.unit_type(u) != UnitType::Monster {
        return -1;
    }
    ct.monstats(w.class_id(u))
        .map_or(-1, |m| i32::from(m.baseid as i16))
}

/// C's `BaseId` followed k steps along
/// `NextInClass`, stopping early at an invalid class;
/// −1 for an invalid c.
pub fn class_step(ct: &CombatTables, c: i32, k: i32) -> i32 {
    let count = ct.monstats.len() as i32;
    let Some(m) = ct.monstats(c) else {
        return -1;
    };
    let mut cur = i32::from(m.baseid as i16);
    for _ in 0..k.max(0) {
        let Some(n) = ct.monstats(cur).map(|m| i32::from(m.nextinclass as i16)) else {
            break;
        };
        if !(0..count).contains(&n) {
            break;
        }
        cur = n;
    }
    cur
}

/// The used skill entry's param `i` (E none → 0).
pub fn e_param<W: BodyWorld>(w: &W, u: W::Unit, i: u8) -> i32 {
    match w.used_skill(u) {
        Some(e) => w.entry_param(u, &e, i),
        None => 0,
    }
}

/// The used skill entry's param `i` := v (E none → nothing).
pub fn set_e_param<W: BodyWorld>(w: &mut W, u: W::Unit, i: u8, v: i32) {
    if let Some(e) = w.used_skill(u) {
        w.set_entry_param_of(u, &e, i, v);
    }
}

/// The used skill entry's flags := f (E none → nothing).
pub fn set_e_flags<W: BodyWorld>(w: &mut W, u: W::Unit, f: u32) {
    if let Some(e) = w.used_skill(u) {
        w.set_entry_flags(u, &e, f);
    }
}

/// Missiles `Vel` + trunc(`VelLev` × L / 8) of m; 0
/// without a record.
pub fn missile_velocity(t: &SkillTables, m: i32, lvl: i32) -> i32 {
    t.missile(m).map_or(0, |r| {
        i32::from(r.vel).wrapping_add(i32::from(r.vellev).wrapping_mul(lvl) / 8)
    })
}

/// "Hit:" fill of the batch 4 melee bodies: result |= `ResultFlags`,
/// hit flags |= `HitFlags`, `HitClass` ≠ 0 → hit class.
pub fn hit_fill(r: &Skills, record: &mut DamageRecord) {
    record.result |= r.resultflags;
    record.hit_flags |= r.hitflags;
    if r.hitclass != 0 {
        record.hit_class = r.hitclass;
    }
}

/// `srvoverlay` / `sumoverlay` in `lo…count − 1` (or …count with
/// `inclusive`) → overlay on `u`.
pub fn overlay_if<W: BodyWorld>(w: &mut W, u: W::Unit, ov: i32, lo: i32, inclusive: bool) {
    let n = w.overlay_count();
    let ok = ov >= lo && if inclusive { ov <= n } else { ov < n };
    if ok {
        w.combat().overlay(u, ov);
    }
}

// ---------------------------------------------------------------- 3.1

/// `spawn_class(unit, skill, L, &mode, &x, &y)` with EDX =
/// 1: (class, mode, (x, y)); the outputs keep the
/// caller's 0 when not written.
pub fn spawn_class<W: BodyWorld>(
    w: &W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
) -> (i32, i32, (i32, i32)) {
    let count = ct.monstats.len() as i32;
    let (ux, uy) = w.position(u);
    let (mut mode, mut at) = (0, (0, 0));
    let c = if w.unit_type(u) == UnitType::Monster {
        match ct.monstats(w.class_id(u)) {
            Some(ms) => {
                let c = i32::from(ms.spawn as i16);
                if (0..count).contains(&c) {
                    at = (
                        ux.wrapping_add(i32::from(ms.spawnx as i8)),
                        uy.wrapping_add(i32::from(ms.spawny as i8)),
                    );
                    let m = i32::from(ms.spawnmode as i8);
                    mode = if (0..=15).contains(&m) { m } else { 1 };
                }
                c
            }
            None => -1,
        }
    } else {
        match rec(t, skill) {
            Some(r) => {
                let c = s16(r.summon);
                if (0..count).contains(&c) {
                    let m = i32::from(r.summode as i8);
                    mode = if (0..=15).contains(&m) { m } else { 1 };
                    at = (ux, uy);
                }
                c
            }
            None => -1,
        }
    };
    if (0..count).contains(&c) {
        return (c, mode, at);
    }
    // The AI control's spawn class, the outputs
    // unwritten.
    match w.minion_spawn_class(u) {
        Some(c2) if c2 > 0 && c2 < count => (c2, mode, at),
        _ => (-1, mode, at),
    }
}

// ---------------------------------------------------------------- 3.2

/// Inferno channel end.
pub fn channel_end<W: BodyWorld>(w: &mut W, ct: &CombatTables, u: W::Unit) {
    let d = if w.unit_type(u) == UnitType::Monster {
        ct.monstats2(w.class_id(u))
            .map_or(11, |m| i32::from(m.infernolen))
    } else {
        11
    };
    w.delete_timers(u, 0, 0);
    // The wind-down lasts `infernolen` base ticks (11 for players).
    w.schedule(u, 1, super::after(w, d), 0, 0);
}

/// Inferno frames with (skill, unit, L) on the missile `m`.
pub fn channel_frames<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    m: Option<W::Unit>,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(calc1) = rec(t, skill).map(|r| r.calc1) else {
        return 0;
    };
    let Some(m) = m else {
        return 0;
    };
    let Some((p2, anim)) = t
        .missile(w.class_id(m))
        .map(|r| (r.param2 as i32, i32::from(r.animlen)))
    else {
        return 0;
    };
    let mut n = eval(w, t, u, calc1, skill, lvl);
    if n <= 0 {
        n = p2.wrapping_add(lvl).wrapping_sub(1);
    }
    let n = if n < 2 { 1 } else { n.min(255) };
    w.path_op(m, PathOp::Steps(n));
    w.set_anim_speed(m, (anim.wrapping_shl(8) / n).clamp(0, 0x7FFF));
    w.set_missile_frames(m, n, n);
    1
}

/// Inferno animation.
pub fn channel_anim<W: BodyWorld>(w: &mut W, ct: &CombatTables, r: &Skills, u: W::Unit) {
    if w.unit_type(u) != UnitType::Monster {
        w.set_frame_event_index(u, 8);
        w.set_frame_count(u, 0x500);
        return;
    }
    let Some(a) = ct
        .monstats2(w.class_id(u))
        .map(|m| i32::from(m.infernoanim))
    else {
        return;
    };
    if r.monanim == 14 {
        w.set_frame_event_index(u, a - 1);
        w.set_frame_count(u, (a + 1) << 8);
    } else {
        w.set_anim_frame(u, a << 8);
    }
}

// ---------------------------------------------------------------- 3.3

/// The equipped item selected by Throw or Left Hand Throw.
pub(super) fn throw_item<W: BodyWorld>(w: &W, u: W::Unit, left: bool) -> Option<W::Item> {
    if !w.has_inventory(u) {
        return None;
    }
    let mut item = None;
    // Weapon pick.
    let pick = [4u8, 5].into_iter().find_map(|loc| {
        let i = w.item_at(u, loc)?;
        (w.item_usable(i) && (w.item_is(i, 45) || w.item_is(i, 38))).then_some((i, loc))
    });
    if let Some((i, loc)) = pick {
        let throwable = |i| w.item_flag_throw(i) || w.item_stat_of(i, stat::ITEM_THROWABLE) != 0;
        let active = w.weapon_in_use(u);
        let inuse = active == Some(i);
        let other = if left { inuse } else { !inuse };
        item = if other {
            w.item_at(u, if loc == 5 { 4 } else { 5 })
                .filter(|&other| left || active.is_some() || throwable(other))
                .or_else(|| (!left && active.is_none()).then_some(i))
        } else {
            Some(i)
        };
        if !item.is_some_and(throwable) {
            return None;
        }
    }
    item
}

/// Throw; `left` = srvdo 5.
pub fn throw<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
    left: bool,
) -> i32 {
    let Some(i) = throw_item(w, u, left) else {
        return 0;
    };
    if w.unit_type(u) == UnitType::Player
        && w.item_stat_of(i, stat::QUANTITY) <= 0
        && w.item_stat_of(i, stat::ITEM_THROWABLE) == 0
    {
        return 0;
    }
    flag_40(w, u);
    let m = w.item_missile_type(i);
    let lob = w.item_is(i, 38);
    let mm = skill_missile_unit(w, m, u, skill, lvl, (0, 0), (0, 0), false, lob);
    // Post-throw.
    if let Some(mm) = mm {
        // 0 for
        // an item that fails the throw gate (not stats 342 / 343).
        let h = throw_mastery(w, t, Some(u), Some(i), None, 0);
        let d = throw_mastery(w, t, Some(u), Some(i), None, 1);
        let v = w.stat(mm, sid::TOHIT, 0).wrapping_add(h);
        w.set_stat(mm, sid::TOHIT, v);
        let v = w.stat(mm, sid::DAMAGEPERCENT, 0).wrapping_add(d);
        w.set_stat(mm, sid::DAMAGEPERCENT, v);
        // Consume the item that produced this missile, after successful creation.
        if w.unit_type(u) == UnitType::Player {
            use_one(w, u, i);
            if w.item_max_stack(i) > 0 && w.item_stat_of(i, stat::QUANTITY) == 0 {
                w.effect(BodyEffect::ExhaustStack { owner: u, item: i });
            }
        }
    } else {
        return 0;
    }
    1
}

// ---------------------------------------------------------------- 3.4

/// Monster swing.
#[allow(clippy::too_many_arguments)]
pub fn mon_swing<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    tg: Option<W::Unit>,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill).cloned() else {
        return 0;
    };
    let Some(tg) = tg else {
        return 0;
    };
    w.path_op(u, PathOp::TargetUnit(Some(tg)));
    let mut record = skill_melee(w, t, ct, u, tg, skill, lvl);
    let hit = record.result & 1 != 0;
    if hit {
        hit_fill(&r, &mut record);
        record.enh_pct = eval(w, t, u, r.calc2, skill, lvl);
        roll_elemental(w, t, u, &mut record, skill, lvl);
    }
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, 128);
    apply_melee(w.combat(), ct, u, tg);
    set_e_param(w, u, 1, i32::from(hit));
    1
}

// ---------------------------------------------------------------- 3.5

/// Next action event.
pub fn next_event<W: BodyWorld>(w: &W, u: W::Unit) -> i32 {
    if let Some(nf) = w.sequence_frames(u) {
        let n = nf >> 8;
        let c = w.frame_event_index(u);
        // Edge case 3: the frame argument is never advanced.
        for _ in c.wrapping_add(1)..n {
            let e = w.sequence_event(u, c.wrapping_shl(8));
            if e != 0 {
                return e;
            }
        }
        return 0;
    }
    let (n, c) = (w.frame_count(u) >> 8, w.anim_frame(u) >> 8);
    let Some((frames, ev)) = w.anim_data(u) else {
        return 0;
    };
    if n as u32 > frames {
        return 0;
    }
    for i in c.wrapping_add(1)..n {
        let e = usize::try_from(i)
            .ok()
            .and_then(|i| ev.get(i))
            .copied()
            .unwrap_or(0);
        if e != 0 {
            return i32::from(e);
        }
    }
    0
}

// ---------------------------------------------------------------- 3.6

/// Revive in place.
pub fn revive<W: BodyWorld>(w: &mut W, tg: W::Unit) {
    let Some(room) = w.unit_room(tg) else {
        return;
    };
    let hp = w.stat(tg, sid::MAXHP, 0);
    w.set_stat(tg, sid::LIFE, hp);
    flags_or(w, tg, 0xE);
    let at = w.position(tg);
    if w.unit_type(tg) == UnitType::Monster {
        flags_or(w, tg, 0x402_0000);
        w.effect(BodyEffect::AiRefresh(tg));
        w.effect(BodyEffect::EvilCounterDec(tg));
        w.effect(BodyEffect::QuestChainLink { u: tg, room });
        w.path_op(tg, PathOp::FootprintMask(0x100));
        w.path_op(tg, PathOp::Reset);
        w.effect(BodyEffect::PatternStamp {
            room,
            x: at.0,
            y: at.1,
            u: tg,
            mask: 0x100,
        });
    } else {
        w.path_op(tg, PathOp::FootprintMask(0x80));
        w.path_op(tg, PathOp::Reset);
        w.effect(BodyEffect::PatternStamp {
            room,
            x: at.0,
            y: at.1,
            u: tg,
            mask: 0x80,
        });
    }
}

// ---------------------------------------------------------------- 3.7

/// Resurrect mode.
pub fn resurrect_mode<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    f: i32,
) -> i32 {
    if f == 0 {
        return 1;
    }
    let class = w.class_id(u);
    if ct.monstats(class).is_none() {
        return 1;
    }
    let Some((k, m)) = ct.monstats2(class).map(|m| {
        (
            i32::from(m.resurrectskill as i16),
            i32::from(m.resurrectmode),
        )
    }) else {
        return 1;
    };
    if (1..t.skills.len() as i32).contains(&k) {
        let e = w.find_entry(u, k);
        w.set_used_skill(u, e);
    }
    if m < 16 {
        m
    } else {
        1
    }
}

// ---------------------------------------------------------------- 3.8

/// `dir8(d)` = table: ((d + 4) >> 3) & 7.
pub fn dir8(d: i32) -> i32 {
    (d.wrapping_add(4) >> 3) & 7
}

/// Offset pair dx (32 entries).
pub const PAIR_DX: [i32; 32] = [
    0, -1, -1, -1, 0, 1, 1, 1, 0, -1, -2, -2, -2, -2, -2, -1, 0, 1, 2, 2, 2, 2, 2, 1, 0, -3, -3,
    -3, 0, 3, 3, 3,
];
/// Offset pair dy (32 entries).
pub const PAIR_DY: [i32; 32] = [
    -1, -1, 0, 1, 1, 1, 0, -1, -2, -2, -2, -1, 0, 1, 2, 2, 2, 2, 2, 1, 0, -1, -2, -2, -3, -3, 0, 3,
    3, 3, 0, -3,
];

/// Offset pair e.
pub fn pair(e: usize) -> (i32, i32) {
    (PAIR_DX[e], PAIR_DY[e])
}

// ---------------------------------------------------------------- 2.1 (bodies-4)

/// Weapon skill roll.
pub fn weapon_roll<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
    record: &mut DamageRecord,
) {
    if rec(t, skill).is_none() {
        return;
    }
    let mn = phys_min(w, t, Some(u), skill, lvl, false);
    let mx = phys_max(w, t, Some(u), skill, lvl, false);
    record.physical = bonuses(w.combat(), t, u, false, None, mn, mx, 0, 0, 128);
    roll_elemental(w, t, u, record, skill, lvl);
}

// ---------------------------------------------------------------- 2.2 (bodies-4)

/// Stacking state list.
#[allow(clippy::too_many_arguments)]
pub fn stack_list<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    src: W::Unit,
    x: W::Unit,
    s: i32,
    skill: i32,
    lvl: i32,
) -> Option<W::List> {
    let len = rec(t, skill)?.auralencalc;
    let len = eval(w, t, src, len, skill, lvl);
    let e = super::after(w, len);
    let l = w.alloc_list(2, e, Some(src))?;
    w.set_list_state(l, s);
    w.set_remove_callback(l, callback::DEFAULT);
    w.attach(x, l);
    w.state_on(x, s, true);
    w.combat().schedule_timer(x, 12, e);
    aura_fill(w, t, src, l, skill, lvl);
    Some(l)
}

// ---------------------------------------------------------------- 2.3 (bodies-4)

fn imp_pair<W: BodyWorld>(w: &W, ct: &CombatTables, imp: W::Unit, k: W::Unit) -> bool {
    base_id(w, ct, imp) == 492 && matches!(base_id(w, ct, k), 435 | 441)
}

/// The life percent: trunc(100·(life >> 8) / (max >> 8)),
/// 0 when the maximum is below 256.
pub fn life_percent<W: BodyWorld>(w: &W, u: W::Unit) -> i32 {
    let m = w.stat_max(u, sid::LIFE) >> 8;
    if m <= 0 {
        return 0;
    }
    (w.stat(u, sid::LIFE, 0) >> 8).wrapping_mul(100) / m
}

/// Imp possess.
pub fn possess<W: BodyWorld>(
    w: &mut W,
    ct: &CombatTables,
    imp: W::Unit,
    k: W::Unit,
    skill: i32,
    lvl: i32,
) {
    if !imp_pair(w, ct, imp, k) {
        return;
    }
    w.state_on(imp, st4::ATTACHED, true);
    link_source(w, imp, Some(k));
    link_source(w, k, Some(imp));
    w.delete_timers(imp, 5, skill);
    w.effect(BodyEffect::EveryTick {
        u: imp,
        kind: 5,
        a1: skill,
        a2: lvl,
    });
    flags_clear(w, imp, 0xE);
    w.set_ai_state(imp, 16);
}

/// Imp release with (game, imp, K, skill, L).
pub fn release<W: BodyWorld>(w: &mut W, ct: &CombatTables, imp: W::Unit, k: W::Unit, skill: i32) {
    if !imp_pair(w, ct, imp, k) {
        return;
    }
    w.state_on(imp, st4::ATTACHED, false);
    link_source(w, imp, None);
    link_source(w, k, None);
    if life_percent(w, imp) < 10 {
        w.effect(BodyEffect::KillBy {
            u: imp,
            killer: None,
            b: 1,
        });
        return;
    }
    flags_or(w, imp, 0xE);
    w.delete_timers(imp, 5, skill);
    w.set_ai_state(imp, 0);
}

/// The source of `u` (its owner unit), when the owner-link flag is set.
pub fn source_of<W: BodyWorld>(w: &W, u: W::Unit) -> Option<W::Unit> {
    if w.unit_c8(u) & 0x400 == 0 {
        return None;
    }
    w.missile_owner(u)
}

// ---------------------------------------------------------------- 2.4, 2.5 (bodies-4)

/// What the lightning callbacks and the DiabWall callback read and write
/// on a missile beyond [`JitterMissile`].
pub trait PathMissile: JitterMissile {
    /// The missile's seed.
    fn missile_seed(&mut self, m: Self::Missile) -> &mut Seed;
    /// The missile's position.
    fn missile_position(&self, m: Self::Missile) -> (i32, i32);
    /// The path's target point.
    fn path_target(&self, m: Self::Missile) -> (i32, i32);
    /// The 64-step direction from the missile to (x, y).
    fn missile_dir64(&self, m: Self::Missile, at: (i32, i32)) -> i32;
    /// Path point i := (x, y) (u16 each; the missile's path array).
    fn set_point(&mut self, m: Self::Missile, i: i32, at: (u16, u16));
    /// The path's point count := n.
    fn set_point_count(&mut self, m: Self::Missile, n: i32);
}

/// Steps of the zigzag (X, Y).
pub const ZIG_X: [i32; 8] = [2, 2, 0, -2, -2, -2, 0, 2];
pub const ZIG_Y: [i32; 8] = [0, 2, 2, 2, 0, -2, -2, -2];

/// Frames capped at 77 (jitter steps 1).
fn cap_frames<W: JitterMissile>(w: &mut W, m: W::Missile) -> i32 {
    let mut n = w.total_frames(m);
    if n >= 78 {
        n = 77;
        w.set_frames(m, 77);
    }
    n
}

/// Lightning fan callback.
pub fn zigzag_cb<W: PathMissile>(w: &mut W, m: W::Missile, a: u32) {
    let n = cap_frames(w, m);
    *w.missile_seed(m) = Seed::init_low(a);
    w.set_path_type(m, 10);
    w.set_steps(m, n);
    let (mut px, mut py) = w.missile_position(m);
    let tgt = w.path_target(m);
    let mut d = w.missile_dir64(m, tgt);
    let mut s: i32 = if w.missile_seed(m).step() & 1 != 0 {
        1
    } else {
        -1
    };
    let mut k = (w.missile_seed(m).step() % 3) as i32 + 2;
    for i in 0..n.max(0) {
        d = d.wrapping_add(k.wrapping_mul(s));
        let e = dir8(d & !0xC0 & 0xFF) as usize;
        px = px.wrapping_add(ZIG_X[e]);
        py = py.wrapping_add(ZIG_Y[e]);
        w.set_point(m, i, (px as u16, py as u16));
        if i % 15 == 0 {
            s = -s;
            k = (w.missile_seed(m).step() % 3) as i32 + 2;
        }
    }
    w.set_point_count(m, n);
}

/// Lightning ring callback.
pub fn zigzag_ring_cb<W: PathMissile>(w: &mut W, m: W::Missile, a: u32) {
    let n = cap_frames(w, m);
    *w.missile_seed(m) = Seed::init_low(a);
    w.set_path_type(m, 10);
    w.set_steps(m, n);
    w.compute_path(m);
}

/// DiabWall callback.
pub fn diab_wall_cb<W: PathMissile>(w: &mut W, m: W::Missile, a: u32) {
    let n = cap_frames(w, m);
    let x = w.path_target_x(m) as u32;
    *w.missile_seed(m) = Seed::init_low(x.wrapping_add(a));
    let r = w.missile_seed(m).step() % 100;
    if r >= 20 {
        w.set_path_type(m, 10);
        w.set_steps(m, n);
        w.compute_path(m);
    }
}

/// Lightning fan / ring:
/// 64 / step missiles from (x, y) seeded by `v`. `ring` =.
#[allow(clippy::too_many_arguments)]
pub fn zigzag<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    m: i32,
    (x, y): (i32, i32),
    skill: i32,
    lvl: i32,
    step: i32,
    v: u32,
    ring: bool,
) {
    let mut s = Seed::init_low(v);
    let cb = if ring {
        init_cb::ZIGZAG_RING
    } else {
        init_cb::ZIGZAG
    };
    let mut req = MissileRequest {
        flags: 3,
        x,
        y,
        skill,
        level: lvl,
        init: Some((cb, 0)),
        ..MissileRequest::new(u, m)
    };
    let p2 = param(t, 280, 2).wrapping_add(1);
    let mut i = 0;
    while i < 64 {
        let (ox, oy) = ring_offset(i as usize);
        req.target_x = ox;
        req.target_y = oy;
        req.init = Some((cb, s.step()));
        let mm = w.spawn_missile(req);
        // Written through M without a null test (fatal on a failed
        // creation; D2RUST skips it).
        if let Some(mm) = mm {
            if ring || m == 568 {
                w.effect(BodyEffect::MissileData28 { missile: mm, v: p2 });
            }
        }
        // Edge case 1 (D2RUST decision): a step ≤ 0 never
        // ends the base game's loop (it hangs, or a negative step reads
        // past the ring tables). D2RUST stops here, after the i = 0
        // missile the base game makes first, creates no further missile
        // and reports the fault.
        if step <= 0 {
            w.effect(BodyEffect::EndlessProgressive {
                unit: u,
                skill,
                step,
            });
            return;
        }
        i += step;
    }
}

// ---------------------------------------------------------------- 2.6 (bodies-4)

/// Whip state.
#[allow(clippy::too_many_arguments)]
pub fn whip_state<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    tg: W::Unit,
    skill: i32,
    lvl: i32,
    u: W::Unit,
) {
    let Some(r) = rec(t, skill) else {
        return;
    };
    let (len, s) = (r.auralencalc, s16(r.auratargetstate));
    let d = eval(w, t, u, len, skill, lvl);
    let l = apply_state(
        w,
        ct,
        StateRequest {
            source: u,
            target: tg,
            skill,
            level: lvl,
            duration: d,
            stat: -1,
            value: 0,
            state: s,
            callback: 0,
        },
    );
    if let Some(l) = l {
        aura_fill(w, t, u, l, skill, lvl);
    }
}

/// Transform.
#[allow(clippy::too_many_arguments)]
pub fn transform<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    tg: Option<W::Unit>,
    skill: i32,
) {
    let Some(r) = rec(t, skill) else {
        return;
    };
    let umod = i32::from(r.sumumod);
    let (c, mode) = summon_class(w, t, ct, u, skill);
    let Some(tg) = tg else {
        return;
    };
    if c < 0 || w.unit_type(tg) != UnitType::Monster {
        return;
    }
    let k = w.chain_position(w.class_id(tg));
    let c2 = class_step(ct, c, k);
    w.effect(BodyEffect::ClassChange {
        t: tg,
        class: c2,
        mode,
    });
    w.set_ai_state(tg, 15);
    w.mode_request(tg, mode, None);
    w.state_on(tg, st4::CHANGECLASS, true);
    if let Some(l) = w.alloc_list(0, 0, Some(u)) {
        w.set_list_state(l, st4::CHANGECLASS);
        w.attach(tg, l);
        w.list_set(l, SHORTPARAM1, c2);
    }
    if (1..=42).contains(&umod) {
        w.effect(BodyEffect::Umod {
            m: tg,
            umod,
            arg: 0,
        });
    }
}

// ---------------------------------------------------------------- 2.7 (bodies-4)

/// Vine offsets (X, Y).
pub const VINE_X: [i32; 4] = [0, 0, 1, -1];
pub const VINE_Y: [i32; 4] = [1, -1, 0, 0];

/// Vine missiles with (game, unit, n, m, skill, L).
pub fn vines<W: BodyWorld>(w: &mut W, u: W::Unit, n: i32, m: i32, skill: i32, lvl: i32) -> i32 {
    let Some((x, y)) = tpos(w, u) else {
        return 0;
    };
    let mut req = MissileRequest {
        flags: 3,
        x,
        y,
        skill,
        level: lvl,
        ..MissileRequest::new(u, m)
    };
    for i in 0..n.max(0) {
        req.target_x = VINE_X[(i & 3) as usize];
        req.target_y = VINE_Y[(i & 3) as usize];
        req.init = Some((init_cb::JITTER, i as u32));
        w.spawn_missile(req);
    }
    1
}

//! The do functions (`srvdofunc`) of `functions.tsv` status
//! `spec'd-here`: Attack, the melee-with-state do, buffs, curses and
//! the basic aura. Each returns the value the do core reads (0 = nothing
//! happened).

use super::helpers::*;
use super::{callback, BodyWorld};
use crate::combat::{
    apply_melee, fill, melee_result, pct, start_combat, CombatTables, CombatWorld, DamageRecord,
};
use crate::skills::use_::period;
use crate::skills::{mana_cost, SkillTables};
use crate::units::UnitType;

/// Item type 38 (missile potion).
const MISSILE_POTION: i32 = 38;

// -----------------------------------------------------------------

/// 1 Attack, Left Hand Swing.
pub fn attack<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    flag_40(w, u);
    if w.current_weapon(u)
        .is_some_and(|i| w.item_is(i, MISSILE_POTION))
    {
        return 0;
    }
    if is_bow(w, u) {
        let (m, l) = bow_missile(w, u);
        let l = l.unwrap_or(lvl);
        skill_missile(w, m, u, skill, l, (0, 0), (0, 0), m != 27, false);
        return 1;
    }
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let bonus = w.stat(u, stat::PROGRESSIVE_TOHIT, 0);
    let mut record = DamageRecord {
        result: melee_result(w.combat(), t, ct, Some(u), Some(tg), bonus, 0),
        ..DamageRecord::default()
    };
    record.hit_flags |= 2;
    charges_before(w, t, u, &mut record);
    fill(w.combat(), t, ct, u, tg, &mut record, false, 128);
    charges_after(w, t, u, &mut record);
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, 128);
    if w.has_group(u, group::MELEEONLY) {
        flag_40(w, u);
        apply_melee(w.combat(), ct, u, tg);
        return 1;
    }
    if let Some(p) = pair_damage(w, u, tg) {
        finisher(w, t, ct, u, &p);
    }
    apply_melee(w.combat(), ct, u, tg);
    1
}

// -----------------------------------------------------------------

/// 2 Kick, Power Strike, …: melee with an overlay,
/// a target state and a self state.
pub fn melee_state<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let (overlay, ts, ss, lencalc) = (
        s16(r.srvoverlay),
        s16(r.auratargetstate),
        s16(r.aurastate),
        r.auralencalc,
    );
    let tg = target(w, u);
    // Step 2: `srvoverlay` in 1…overlay count (Edge case 3).
    if (1..=w.overlay_count()).contains(&overlay) {
        let Some(tg) = tg else {
            return 0;
        };
        let Some(p) = pair_damage(w, u, tg) else {
            return 1;
        };
        if p.result & 1 != 0 {
            w.combat().overlay(tg, overlay);
        }
    }
    // Step 3: `auratargetstate`.
    if state_ok(w, ts) {
        let Some(tg) = tg else {
            return 0;
        };
        let Some(p) = pair_damage(w, u, tg) else {
            return 1;
        };
        if p.result & 1 != 0 {
            let len = eval(w, t, u, lencalc, skill, lvl).max(1);
            let e = super::after(w, len);
            let l = match w.state_list(tg, ts) {
                Some(l) => l,
                None => match w.alloc_list(2, e, Some(tg)) {
                    Some(l) => l,
                    None => return 1,
                },
            };
            w.combat().schedule_timer(tg, 12, e);
            w.state_on(tg, ts, true);
            w.set_list_state(l, ts);
            w.attach(tg, l);
            w.set_remove_callback(l, callback::DEFAULT);
            aura_fill(w, t, tg, l, skill, lvl);
        }
    }
    // Step 4: `aurastate` in 1…count − 1.
    if ss >= 1 && ss < w.state_count() {
        let l = match w.state_list(u, ss) {
            Some(l) => l,
            None => {
                let Some(l) = w.alloc_list(4, 0, Some(u)) else {
                    return 1;
                };
                w.set_list_state(l, ss);
                w.attach(u, l);
                w.set_remove_callback(l, callback::DEFAULT);
                l
            }
        };
        w.state_on(u, ss, true);
        aura_fill(w, t, u, l, skill, lvl);
    }
    flag_40(w, u);
    let Some(tg) = tg else {
        return 0;
    };
    apply_melee(w.combat(), ct, u, tg);
    1
}

// -----------------------------------------------------------------

/// `auraevent1–3` / `auraeventfunc1–3` of a record (i16 views).
pub(crate) fn events(r: &d2_data::tables::Skills) -> [(i32, i32); 3] {
    [
        (s16(r.auraevent1), s16(r.auraeventfunc1)),
        (s16(r.auraevent2), s16(r.auraeventfunc2)),
        (s16(r.auraevent3), s16(r.auraeventfunc3)),
    ]
}

/// 18 Defensive buff.
pub fn buff<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let (st, lencalc, ev) = (s16(r.aurastate), r.auralencalc, events(r));
    if !state_ok(w, st) {
        return 0;
    }
    flag_40(w, u);
    clear_group(w, u, st, true);
    let duration = eval(w, t, u, lencalc, skill, lvl);
    let Some(l) = apply_state(
        w,
        ct,
        StateRequest {
            source: u,
            target: u,
            skill,
            level: lvl,
            duration,
            stat: -1,
            value: 0,
            state: st,
            callback: callback::BUFF,
        },
    ) else {
        return 0;
    };
    aura_fill(w, t, u, l, skill, lvl);
    passive_fill(w, t, u, l, skill, lvl);
    w.list_set(l, stat::MODIFIERLIST_SKILL, skill);
    w.list_set(l, stat::MODIFIERLIST_LEVEL, lvl);
    if ev[0].0 >= 0 {
        unregister(w, u, 1, st);
        for &(e, f) in &ev {
            if e < 0 {
                break;
            }
            register(w, u, e, skill, lvl, f, 1, st);
        }
    }
    w.mark_state_changed(u, st);
    1
}

// -----------------------------------------------------------------

/// The curse context (0x68 bytes).
pub(crate) struct CurseCtx {
    pub(crate) ai: bool,
    pub(crate) upd: bool,
    pub(crate) skill: i32,
    pub(crate) lvl: i32,
    pub(crate) duration: i32,
    pub(crate) stats: [i32; 6],
    pub(crate) values: [i32; 6],
    pub(crate) state: i32,
    pub(crate) events: [(i32, i32); 3],
}

/// 30 Curse.
pub fn curse<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let a1 = s16(r.aurastat1);
    let ts = s16(r.auratargetstate);
    if a1 < -1 || a1 >= t.stat_count || !state_ok(w, ts) {
        return 0;
    }
    let pairs = [
        (r.aurastat1, r.aurastatcalc1),
        (r.aurastat2, r.aurastatcalc2),
        (r.aurastat3, r.aurastatcalc3),
        (r.aurastat4, r.aurastatcalc4),
        (r.aurastat5, r.aurastatcalc5),
        (r.aurastat6, r.aurastatcalc6),
    ];
    let (rangecalc, lencalc, filter, ev) =
        (r.aurarangecalc, r.auralencalc, r.aurafilter, events(r));
    flag_40(w, u);
    let ai = ts == state::DIMVISION || ts == state::TERROR;
    let range = eval(w, t, u, rangecalc, skill, lvl);
    let mut d = eval(w, t, u, lencalc, skill, lvl);
    if ai {
        let div = ct
            .difficulty(w.combat().difficulty())
            .map_or(0, |x| x.aicursedivisor as i32);
        if div != 0 {
            d /= div;
        }
    }
    let mut cx = CurseCtx {
        ai,
        upd: false,
        skill,
        lvl,
        duration: d,
        // Zeroed first: slots after a stop keep stat 0.
        stats: [0; 6],
        values: [0; 6],
        state: ts,
        events: ev,
    };
    for (i, &(s, c)) in pairs.iter().enumerate() {
        let s = s16(s);
        if !stat_ok(t, s) {
            cx.stats[i] = -1;
            break;
        }
        let Some(info) = w.stat_info(s) else {
            break;
        };
        if info.updateanimrate {
            cx.upd = true;
        }
        cx.stats[i] = s;
        cx.values[i] = eval(w, t, u, c, skill, lvl);
    }
    let mut cb = |w: &mut W, x: W::Unit| curse_unit(w, t, ct, u, &cx, x);
    scan_point(w, t, ct, filter, u, range, &mut cb)
}

/// `can_switch(U, k)`.
pub fn can_switch<W: BodyWorld>(w: &mut W, ct: &CombatTables, x: W::Unit, k: i32) -> bool {
    let ms = ct.monstats(w.class_id(x));
    if ms.is_some_and(|m| m.baseid == 492) && w.has_state(x, state::ATTACHED as u16) {
        return false;
    }
    if w.unit_type(x) != UnitType::Monster || k > 19 {
        return false;
    }
    if w.has_state(x, state::UNINTERRUPTABLE as u16) {
        return false;
    }
    // A walk mode, not `boss`, `switchai`.
    let capable = w.combat().monster_has_mode(x, 2) && ms.is_some_and(|m| !m.boss && m.switchai);
    if !capable {
        return false;
    }
    // Unit flag 0x4 or dead: an alive unit without the
    // flag fails, a corpse passes (Revive tests a corpse, of bodies-2b).
    if w.unit_flags(x) & 0x4 == 0 && w.is_alive(x) {
        return false;
    }
    let superunique = w.combat().monster_flag(x, 2);
    if superunique || w.combat().monster_flag(x, 8) {
        return false;
    }
    if k == 19 {
        return true;
    }
    if matches!(k, 10..=12) {
        return !superunique && capable;
    }
    true
}

/// Per-unit curse callback.
pub(crate) fn curse_unit<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    cx: &CurseCtx,
    x: W::Unit,
) -> i32 {
    // k = 10 for state 23, 12 for 27 (Taunt, Edge case
    // 10), 11 for 56, else 0.
    let k = match cx.state {
        state::DIMVISION => 10,
        27 => 12,
        state::TERROR => 11,
        _ => 0,
    };
    if cx.ai
        && (w.unit_type(x) != UnitType::Monster
            || w.combat().alignment(x) == 1
            || !can_switch(w, ct, x, k))
    {
        return 0;
    }
    // Step 2: stat1 = −1 goes on with v1 = 0.
    let mut v1 = 0;
    if cx.stats[0] >= 0 {
        v1 = scaled(w, x, cx.stats[0], cx.values[0]);
        if v1 == 0 {
            return 0;
        }
    }
    if w.unit_type(x) == UnitType::Monster
        && (!w.combat().monster_has_mode(x, 2) || w.combat().monster_flag(x, 0x20))
    {
        return 0;
    }
    if w.unit_flags(x) & 0xE != 0xE || !w.is_alive(x) || !w.combat().hostile(u, x) {
        return 0;
    }
    let Some(l) = apply_state(
        w,
        ct,
        StateRequest {
            source: u,
            target: x,
            skill: cx.skill,
            level: cx.lvl,
            duration: cx.duration,
            stat: cx.stats[0],
            value: v1,
            state: cx.state,
            callback: if cx.ai {
                callback::AI_CURSE
            } else {
                callback::DEFAULT
            },
        },
    ) else {
        return 0;
    };
    if cx.ai {
        w.set_ai_state(x, k);
    }
    for i in 1..6 {
        let s = cx.stats[i];
        if stat_ok(t, s) {
            let v = scaled(w, x, s, cx.values[i]);
            if v != 0 {
                w.list_set(l, s, v);
            }
        }
    }
    if cx.upd {
        w.combat().refresh_anim_rate(x);
    }
    let (e1, f1) = cx.events[0];
    if e1 >= 0 && f1 > 0 {
        unregister(w, x, 1, cx.state);
        // Edge case 4: the loop tests the first pair each time.
        for &(e, f) in &cx.events {
            if !(e1 >= 0 && f1 > 0) {
                break;
            }
            register(w, x, e, cx.skill, cx.lvl, f, 1, cx.state);
        }
    }
    1
}

/// Terror install:
/// state 56 for b frames under
/// skill 77 level 1, then AI special state 11 on the next think. The
/// caller's skill and `a` are not read.
pub fn terror_install<W: BodyWorld>(
    w: &mut W,
    ct: &CombatTables,
    source: W::Unit,
    x: W::Unit,
    b: i32,
) {
    // Steps 1–4.
    if !w.is_alive(x)
        || w.unit_type(x) != UnitType::Monster
        || w.has_state(x, state::UNINTERRUPTABLE as u16)
    {
        return;
    }
    let ms = ct.monstats(w.class_id(x));
    let capable = w.combat().monster_has_mode(x, 2) && ms.is_some_and(|m| !m.boss && m.switchai);
    if !capable || w.combat().monster_flag(x, 0xA) {
        return;
    }
    // Step 5: the result is not tested.
    let _ = apply_state(
        w,
        ct,
        StateRequest {
            source,
            target: x,
            skill: 77,
            level: 1,
            duration: b,
            stat: -1,
            value: 0,
            state: state::TERROR,
            callback: callback::DEFAULT,
        },
    );
    // Steps 6–8.
    w.path_op(x, super::effects::PathOp::TargetUnit(Some(source)));
    w.set_ai_state(x, 11);
    let f = w.frame();
    w.delete_timers(x, 2, 0);
    w.schedule(x, 2, f.wrapping_add(1), 0, 0);
}

// -----------------------------------------------------------------

/// The aura context (0x50 bytes).
pub(crate) struct AuraCtx<L> {
    pub(crate) source_state: i32,
    pub(crate) skill: i32,
    pub(crate) lvl: i32,
    /// Ticks at the game's rate to the aura's next period frame.
    pub(crate) duration: i32,
    pub(crate) stats: [i32; 6],
    pub(crate) values: [i32; 6],
    pub(crate) count: i32,
    pub(crate) list: Option<L>,
    pub(crate) passivestate: i32,
    pub(crate) callback: u32,
}

/// 65 Basic aura.
pub fn aura<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let cost = mana_cost(r, lvl);
    let mana = w.stat(u, stat::MANA, 0);
    let duration = period(w, t, u, skill, lvl)
        .wrapping_sub(w.frame())
        .wrapping_add(1);
    let ps = s16(r.passivestate);
    let mut cx = AuraCtx {
        source_state: 0,
        skill,
        lvl,
        duration,
        stats: [0; 6],
        values: [0; 6],
        count: 0,
        list: None,
        passivestate: if ps > 0 { ps } else { 0 },
        callback: 0,
    };
    let player = w.unit_type(u) == UnitType::Player;
    if !player || mana >= cost {
        let pairs = [
            (r.aurastat1, r.aurastatcalc1),
            (r.aurastat2, r.aurastatcalc2),
            (r.aurastat3, r.aurastatcalc3),
            (r.aurastat4, r.aurastatcalc4),
            (r.aurastat5, r.aurastatcalc5),
            (r.aurastat6, r.aurastatcalc6),
        ];
        for (i, &(s, c)) in pairs.iter().enumerate() {
            let s = s16(s);
            if s < 0 {
                break;
            }
            cx.stats[i] = s;
            cx.values[i] = eval(w, t, u, c, skill, lvl);
        }
    }
    let count = w.state_count();
    let ss = s16(r.aurastate);
    if ss >= 1 && ss < count {
        cx.source_state = ss;
        // The callback on the unit itself.
        aura_unit(w, t, ct, u, &mut cx, u);
        if let Some(l) = cx.list {
            if mana > cost && ps <= 0 {
                let pairs = [
                    (r.passivestat1, r.passivecalc1),
                    (r.passivestat2, r.passivecalc2),
                    (r.passivestat3, r.passivecalc3),
                    (r.passivestat4, r.passivecalc4),
                    (r.passivestat5, r.passivecalc5),
                ];
                for (s, c) in pairs {
                    let v = eval(w, t, u, c, skill, lvl);
                    let s = s16(s);
                    if v != 0 && s >= 1 && s < t.stat_count {
                        w.list_set(l, s, v);
                    }
                }
            }
        }
    }
    let ts = s16(r.auratargetstate);
    if (0..count).contains(&ts) {
        cx.source_state = ts;
        let range = eval(w, t, u, r.aurarangecalc, skill, lvl);
        let mut cb = |w: &mut W, x: W::Unit| aura_unit(w, t, ct, u, &mut cx, x);
        scan_unit(w, t, ct, u, (0, 0), range, r.aurafilter, true, &mut cb);
        if player && cost > 0 {
            if cx.count > 0 {
                w.state_on(u, state::NOMANAREGEN, true);
                spend_aura_mana(w, u, cost);
            } else {
                w.state_on(u, state::NOMANAREGEN, false);
            }
        }
    }
    1
}

/// Players only; blood mana (state 114) →;
/// mana < cost → nothing; else mana −= cost.
pub(crate) fn spend_aura_mana<W: BodyWorld>(w: &mut W, u: W::Unit, cost: i32) {
    if w.unit_type(u) != UnitType::Player {
        return;
    }
    if w.has_state(u, state::BLOOD_MANA as u16) {
        w.blood_mana(u, cost);
        return;
    }
    let mana = w.stat(u, stat::MANA, 0);
    if mana < cost {
        return;
    }
    w.set_stat(u, stat::MANA, mana.wrapping_sub(cost));
}

/// Aura callback.
pub(crate) fn aura_unit<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    cx: &mut AuraCtx<W::List>,
    x: W::Unit,
) -> i32 {
    let cb = if cx.callback != 0 {
        cx.callback
    } else if x == u {
        callback::SELF_AURA
    } else {
        callback::DEFAULT
    };
    // The aura's duration runs to its next period frame, already in
    // ticks at the game's rate.
    let Some(l) = apply_state_ticks(
        w,
        ct,
        StateRequest {
            source: u,
            target: x,
            skill: cx.skill,
            level: cx.lvl,
            duration: cx.duration,
            stat: -1,
            value: 0,
            state: cx.source_state,
            callback: cb,
        },
    ) else {
        return 0;
    };
    cx.list = Some(l);
    for i in 0..6 {
        let (s, v) = (cx.stats[i], cx.values[i]);
        if v == 0 || s < 1 || s >= t.stat_count {
            continue;
        }
        if s == stat::POISONLENGTHRESIST {
            if poison_length_resist(w, x, v) {
                cx.count += 1;
            }
            continue;
        }
        let v2 = scaled(w, x, s, v);
        let Some(info) = w.stat_info(s) else {
            continue;
        };
        if v2 == 0 {
            continue;
        }
        if !info.direct {
            let mut v2 = v2;
            if w.list_get(l, s) != v2 {
                w.mark_state_changed(x, cx.source_state);
            }
            if s == stat::VELOCITYPERCENT || s == stat::ATTACKRATE {
                v2 = v2.max(cold_floor(w, ct, x));
            }
            w.list_set(l, s, v2);
            if s == stat::ATTACKRATE {
                w.list_set(l, stat::OTHER_ANIMRATE, v2);
            }
            cx.count += 1;
        } else {
            let b = w.stat(x, s as u16, 0);
            let mut n = b.wrapping_add(v2);
            if stat_ok(t, info.maxstat) {
                n = n.min(w.stat(x, info.maxstat as u16, 0));
            }
            if n != b {
                w.set_stat(x, s as u16, n);
                cx.count += 1;
            }
        }
    }
    w.combat().refresh_anim_rate(x);
    w.list_set(l, stat::MODIFIERLIST_SKILL, cx.skill);
    w.list_set(l, stat::MODIFIERLIST_LEVEL, cx.lvl);
    if cx.passivestate > 0 {
        if let Some(p) = w.state_list(x, cx.passivestate) {
            w.detach_free(x, p);
            w.mark_state_changed(x, cx.passivestate);
        }
    }
    1
}

/// The velocity / attack-rate floor, monstats `ColdEffect`
/// for the game's difficulty (i8), −50 for non-monsters.
pub(crate) fn cold_floor<W: BodyWorld>(w: &mut W, ct: &CombatTables, x: W::Unit) -> i32 {
    if w.unit_type(x) != UnitType::Monster {
        return -50;
    }
    let d = w.combat().difficulty();
    ct.monstats(w.class_id(x)).map_or(-50, |m| {
        let c = match d {
            0 => m.coldeffect,
            1 => m.coldeffect_n,
            _ => m.coldeffect_h,
        };
        i32::from(c as i8)
    })
}

/// Scales the remaining time of U's poison
/// list and of each curable curse state's list; true when U has a poison
/// list or any curse state with a list.
fn poison_length_resist<W: BodyWorld>(w: &mut W, x: W::Unit, v: i32) -> bool {
    let f = w.frame();
    let mut found = false;
    let rescale = |w: &mut W, l: W::List| {
        let e = f.wrapping_add(pct(w.list_expire(l).wrapping_sub(f), v, 100));
        w.set_list_expire(l, e);
        w.combat().schedule_timer(x, 12, e);
    };
    if let Some(l) = w.state_list(x, state::POISON) {
        rescale(w, l);
        found = true;
    }
    for s in 0..w.state_count() {
        if !w.state_flag(s, group::CURSE) || !w.has_state(x, s as u16) {
            continue;
        }
        let Some(l) = w.state_list(x, s) else {
            continue;
        };
        found = true;
        if w.state_flag(s, group::CURABLE) {
            rescale(w, l);
        }
    }
    found
}

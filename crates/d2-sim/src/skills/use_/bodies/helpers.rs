//! The helpers the bodies share: target, pair record, weapon and
//! ammunition, skill missiles, quantity, stat fills, state lists, the
//! remove callbacks, same-group removal, resistance scaling, the
//! `aurafilter` unit filter and the area scans, unit event handlers and
//! the progressive charges.

use super::{callback, do_slot, BodyWorld, Handler, MissileRequest, ProgressiveMsg, ScanRoom};
use crate::combat::{pct, CombatTables, CombatWorld, DamageRecord, RoomKind};
use crate::skills::use_::table::{self, Kind};
use crate::skills::{
    add_element, eval_skill, highest_entry, roll_elemental, skill_level, special, SkillTables,
};
use crate::units::UnitType;
use d2_data::tables::Skills;

/// The missile-fired unit flag.
pub const FLAG_40: u32 = 0x40;

/// Stats the bodies name.
pub mod stat {
    pub const MANA: u16 = 8;
    pub const LIFE: u16 = 6;
    pub const STAMINA: u16 = 10;
    pub const TOHIT: u16 = 19;
    pub const VELOCITYPERCENT: i32 = 67;
    pub const ATTACKRATE: i32 = 68;
    pub const OTHER_ANIMRATE: i32 = 69;
    pub const QUANTITY: u16 = 70;
    pub const DURABILITY: u16 = 72;
    pub const CURSE_RESISTANCE: u16 = 109;
    pub const POISONLENGTHRESIST: i32 = 110;
    pub const ITEM_THROWABLE: u16 = 125;
    pub const MAGICARROW: u16 = 157;
    pub const EXPLOSIVEARROW: u16 = 158;
    pub const PROGRESSIVE_TOHIT: u16 = 325;
    pub const MODIFIERLIST_SKILL: i32 = 350;
    pub const MODIFIERLIST_LEVEL: i32 = 351;
}

/// States the bodies name.
pub mod state {
    pub const POISON: i32 = 2;
    pub const DIMVISION: i32 = 23;
    pub const UNINTERRUPTABLE: i32 = 54;
    pub const TERROR: i32 = 56;
    pub const ATTRACT: i32 = 57;
    pub const NOMANAREGEN: i32 = 85;
    pub const JUSTHIT: i32 = 86;
    pub const BLOOD_MANA: i32 = 114;
    pub const CORPSE_NOSELECT: i32 = 118;
    pub const ATTACHED: i32 = 143;
}

/// State flag groups (bit numbers of the state flags).
pub mod group {
    pub const PGSV: usize = 4;
    pub const CURSE: usize = 11;
    pub const CURABLE: usize = 12;
    pub const DISGUISE: usize = 16;
    pub const EXP: usize = 30;
    pub const UDEAD: usize = 33;
    pub const MELEEONLY: usize = 38;
}

/// The default `aurafilter` of [`scan_unit`].
pub const DEFAULT_FILTER: u32 = 0x583;

/// i32 view of an i16 record field stored as u16 (−1 = none).
pub(crate) fn s16(v: u16) -> i32 {
    i32::from(v as i16)
}

/// The skills record (R invalid → `None`).
pub(crate) fn rec(t: &SkillTables, skill: i32) -> Option<&Skills> {
    t.skill(skill)
}

/// 0 ≤ s < itemstatcost count.
pub(crate) fn stat_ok(t: &SkillTables, s: i32) -> bool {
    (0..t.stat_count).contains(&s)
}

/// 0 ≤ s < states count.
pub(crate) fn state_ok<W: BodyWorld>(w: &W, s: i32) -> bool {
    (0..w.state_count()).contains(&s)
}

/// `eval(c)` (`eval(unit, c, skill, L)`).
pub(crate) fn eval<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    field: u32,
    skill: i32,
    lvl: i32,
) -> i32 {
    eval_skill(w, t, Some(u), field, skill, lvl)
}

/// `T` = `target(game, unit)`. The refresh (drop a
/// stale or picked-up target) is the provider's [`UseWorld::target`];
/// the unit itself is no target.
pub fn target<W: BodyWorld>(w: &W, u: W::Unit) -> Option<W::Unit> {
    w.target(u).filter(|&x| x != u)
}

fn set_flags<W: BodyWorld>(w: &mut W, u: W::Unit, bits: u32) {
    let f = w.unit_flags(u);
    w.set_unit_flags(u, f | bits);
}

/// Unit flags |= 0x40.
pub(crate) fn flag_40<W: BodyWorld>(w: &mut W, u: W::Unit) {
    set_flags(w, u, FLAG_40);
}

// -----------------------------------------------------------------

/// `pair_record(attacker, defender)`: the index of
/// the first entry of the attacker's combat list for this pair.
pub fn pair_record<W: BodyWorld>(w: &mut W, a: W::Unit, d: W::Unit) -> Option<usize> {
    let c = w.combat();
    let (ai, di) = (c.ident(a), c.ident(d));
    c.combat_list(a)
        .iter()
        .position(|e| e.attacker == ai && e.defender == di)
}

/// The pair record's damage record (a copy), if any.
pub fn pair_damage<W: BodyWorld>(w: &mut W, a: W::Unit, d: W::Unit) -> Option<DamageRecord> {
    let i = pair_record(w, a, d)?;
    w.combat().combat_list(a).get(i).map(|e| e.record)
}

// -----------------------------------------------------------------

/// `is_bow(unit)`: composit weapon class 1 (bow) or 7
/// (crossbow).
pub fn is_bow<W: BodyWorld>(w: &W, u: W::Unit) -> bool {
    matches!(w.composit_weapon_class(u), 1 | 7)
}

/// `bow_missile(unit, &lvl)`: the missile class and the
/// replaced level (magic / exploding arrows).
pub fn bow_missile<W: BodyWorld>(w: &W, u: W::Unit) -> (i32, Option<i32>) {
    let bolts = if w.unit_type(u) == UnitType::Player {
        match w.hand_class(u) {
            1 => false,
            7 => true,
            _ => return (-1, None),
        }
    } else {
        false
    };
    let m = w.item_stat(u, stat::MAGICARROW, 0);
    if m > 0 {
        return (27, Some(m));
    }
    let e = w.item_stat(u, stat::EXPLOSIVEARROW, 0);
    if e > 0 {
        return (41, Some(e));
    }
    (if bolts { 31 } else { 0 }, None)
}

/// A stack: `stackable`, a `throwable` item type or
/// `item_throwable(125)` ≠ 0.
fn is_stack<W: BodyWorld>(w: &W, i: W::Item) -> bool {
    w.item_stackable(i) || w.item_flag_throw(i) || w.item_stat_of(i, stat::ITEM_THROWABLE) != 0
}

/// `has_ammo(unit)`.
pub fn has_ammo<W: BodyWorld>(w: &mut W, u: W::Unit) -> bool {
    if w.unit_type(u) != UnitType::Player {
        return true;
    }
    let Some(used) = w.used_skill(u) else {
        return false;
    };
    if matches!(used.skill, 2 | 4) {
        return super::b4_helpers::throw_item(w, u, used.skill == 4)
            .is_some_and(|item| has_qty(w, item));
    }
    let Some(wpn) = w.current_weapon(u) else {
        return false;
    };
    let mut any = true;
    if w.item_shoots(wpn) {
        any = false;
        if used.skill == 0 && w.item_stat_of(wpn, stat::MAGICARROW) != 0 {
            return true;
        }
    }
    for loc in [4, 5] {
        let Some(i) = w.item_at(u, loc) else {
            continue;
        };
        if (!any || i == wpn) && is_stack(w, i) {
            return has_qty(w, i);
        }
    }
    false
}

/// `has_qty(I)`.
pub fn has_qty<W: BodyWorld>(w: &mut W, i: W::Item) -> bool {
    if w.item_stat_of(i, stat::QUANTITY) > 0 {
        return true;
    }
    if w.item_stat_of(i, stat::ITEM_THROWABLE) != 0 {
        w.set_item_stat(i, stat::QUANTITY, 0);
        return true;
    }
    w.item_max_stack(i) <= 0
}

// -----------------------------------------------------------------

/// `skill_missile(game, missile, unit, skill, L, dx, dy, tx, ty, quant)`
/// = (straight) / (`lob`). False: no missile.
#[allow(clippy::too_many_arguments)]
pub fn skill_missile<W: BodyWorld>(
    w: &mut W,
    missile: i32,
    u: W::Unit,
    skill: i32,
    lvl: i32,
    d: (i32, i32),
    at: (i32, i32),
    quant: bool,
    lob: bool,
) -> bool {
    skill_missile_unit(w, missile, u, skill, lvl, d, at, quant, lob).is_some()
}

/// [`skill_missile`] returning the missile made.
#[allow(clippy::too_many_arguments)]
pub fn skill_missile_unit<W: BodyWorld>(
    w: &mut W,
    missile: i32,
    u: W::Unit,
    skill: i32,
    lvl: i32,
    (dx, dy): (i32, i32),
    (tx, ty): (i32, i32),
    quant: bool,
    lob: bool,
) -> Option<W::Unit> {
    let (mut tx, mut ty) = (tx, ty);
    if tx == 0 || ty == 0 {
        match w.target_position(u) {
            Some((a, b)) if a != 0 && b != 0 => (tx, ty) = (a, b),
            _ => return None,
        }
    }
    if quant && w.unit_type(u) == UnitType::Player && dec_quantity(w, u) < 1 {
        return None;
    }
    let (ux, uy) = w.position(u);
    let mut req = MissileRequest {
        flags: if lob { 0x420 } else { 0x21 },
        origin: lob.then_some(u),
        x: ux.wrapping_add(dx),
        y: uy.wrapping_add(dy),
        target_x: tx,
        target_y: ty,
        skill,
        level: lvl,
        ..MissileRequest::new(u, missile)
    };
    if !lob && w.unit_type(u) == UnitType::Monster {
        let b = w.stat(u, stat::TOHIT, 0);
        if b != 0 {
            req.flags |= 0x1000;
            req.attack_bonus = b;
        }
    }
    w.spawn_missile(req)
}

// -----------------------------------------------------------------

/// `dec_quantity(game, unit)` (`decquant`).
pub fn dec_quantity<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    if w.unit_type(u) != UnitType::Player {
        return 0;
    }
    let wpn = w.current_weapon(u);
    let weapon_only = !wpn.is_some_and(|i| w.item_is(i, 27) || w.item_is(i, 35));
    for loc in [4, 5] {
        let Some(i) = w.item_at(u, loc) else {
            continue;
        };
        if (!weapon_only || Some(i) == wpn) && is_stack(w, i) {
            return use_one(w, u, i);
        }
    }
    0
}

/// `use_one(game, player, I)`.
pub fn use_one<W: BodyWorld>(w: &mut W, u: W::Unit, i: W::Item) -> i32 {
    let mut q = w.item_stat_of(i, stat::QUANTITY).wrapping_sub(1);
    let empty = q < 0;
    if empty {
        q = 0;
    }
    w.set_item_stat(i, stat::QUANTITY, q);
    w.quantity_timer(i);
    w.send_item_stat(u, i, stat::QUANTITY, q);
    let m = w.item_max_durability(i);
    if m != w.item_stat_of(i, stat::DURABILITY) {
        w.set_item_stat(i, stat::DURABILITY, m);
        w.send_item_stat(u, i, stat::DURABILITY, m);
    }
    w.attack_cleanup(u);
    if empty {
        0
    } else {
        q.wrapping_add(1)
    }
}

// -----------------------------------------------------------------

fn fill<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    l: W::List,
    pairs: &[(u16, u32)],
    skill: i32,
    lvl: i32,
) {
    for &(s, c) in pairs {
        let s = s16(s);
        if !stat_ok(t, s) {
            continue;
        }
        let v = eval(w, t, u, c, skill, lvl);
        if v != 0 {
            w.list_set(l, s, v);
            if s == stat::ATTACKRATE {
                w.list_set(l, stat::OTHER_ANIMRATE, v);
            }
        }
    }
    w.combat().refresh_anim_rate(u);
}

/// `aura_fill(unit, list, R, skill, L)`: `aurastat1–6` /
/// `aurastatcalc1–6`, formulas on `u`.
pub fn aura_fill<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    l: W::List,
    skill: i32,
    lvl: i32,
) {
    let Some(r) = rec(t, skill) else {
        return;
    };
    let pairs = [
        (r.aurastat1, r.aurastatcalc1),
        (r.aurastat2, r.aurastatcalc2),
        (r.aurastat3, r.aurastatcalc3),
        (r.aurastat4, r.aurastatcalc4),
        (r.aurastat5, r.aurastatcalc5),
        (r.aurastat6, r.aurastatcalc6),
    ];
    fill(w, t, u, l, &pairs, skill, lvl);
}

/// `passive_fill`: `passivestat1–5` / `passivecalc1–5`.
pub fn passive_fill<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    l: W::List,
    skill: i32,
    lvl: i32,
) {
    let Some(r) = rec(t, skill) else {
        return;
    };
    let pairs = [
        (r.passivestat1, r.passivecalc1),
        (r.passivestat2, r.passivecalc2),
        (r.passivestat3, r.passivecalc3),
        (r.passivestat4, r.passivecalc4),
        (r.passivestat5, r.passivecalc5),
    ];
    fill(w, t, u, l, &pairs, skill, lvl);
}

// -----------------------------------------------------------------

/// The request record of `apply_state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateRequest<U> {
    pub source: U,
    pub target: U,
    pub skill: i32,
    pub level: i32,
    pub duration: i32,
    /// −1: none.
    pub stat: i32,
    pub value: i32,
    pub state: i32,
    /// 0 → [`callback::DEFAULT`].
    pub callback: u32,
}

/// Puts state `q.state` on the target for `q.duration` base ticks.
/// Callers state the duration in base ticks (skill calcs, item and combat
/// rules); it becomes ticks at the game's rate here.
pub fn apply_state<W: BodyWorld>(
    w: &mut W,
    ct: &CombatTables,
    q: StateRequest<W::Unit>,
) -> Option<W::List> {
    let duration = crate::skills::use_::UseWorld::tick_rate(w).ticks(q.duration);
    apply_state_ticks(w, ct, StateRequest { duration, ..q })
}

/// [`apply_state`] with `q.duration` already counted in ticks at the
/// game's rate (an aura's time to its next period frame).
pub fn apply_state_ticks<W: BodyWorld>(
    w: &mut W,
    ct: &CombatTables,
    q: StateRequest<W::Unit>,
) -> Option<W::List> {
    let (tg, s) = (q.target, q.state);
    if !state_ok(w, s) {
        return None;
    }
    if w.unit_type(tg) == UnitType::Monster {
        let class = w.class_id(tg);
        if ct.monstats(class).is_none_or(|m| m.npc) {
            return None;
        }
        if !ct.monstats2(class).is_some_and(|m| m.isatt) {
            return None;
        }
    }
    let mut dur = q.duration;
    let (old, mut flags) = if w.state_flag(s, group::CURSE) {
        let r = w.stat(tg, stat::CURSE_RESISTANCE, 0);
        if r >= 100 {
            return None;
        }
        if r != 0 {
            dur = dur.wrapping_sub(pct(dur, r, 100));
        }
        if w.has_state(tg, state::ATTRACT as u16) {
            return None;
        }
        (w.first_list_with_flags(tg, 0x20), 0x20)
    } else {
        (w.state_list(tg, s), 0)
    };
    let frame = w.frame();
    if let Some(o) = old {
        let (os, (ok, ol)) = (w.list_state(o), w.list_skill(o));
        if os == s && ok == q.skill && ol == q.level {
            if dur != 0 {
                let e = frame.wrapping_add(dur);
                w.set_list_expire(o, e);
                w.combat().schedule_timer(tg, 12, e);
            }
            return Some(o);
        }
        if os == s && ok == q.skill && q.level < ol {
            return None;
        }
        w.detach_free(tg, o);
    }
    w.queue_update(tg);
    w.state_on(tg, s, true);
    let mut e = 0;
    if dur != 0 {
        e = frame.wrapping_add(dur);
        w.combat().schedule_timer(tg, 12, e);
        flags |= 2;
    }
    if w.state_flag(s, group::EXP) {
        flags |= 0x800;
    }
    if w.state_is_aura(s) {
        flags |= 8;
    }
    let l = w.alloc_list(flags, e, Some(q.source))?;
    w.set_list_state(l, s);
    w.set_list_skill(l, q.skill, q.level);
    if q.stat != -1 {
        w.list_set(l, q.stat, q.value);
    }
    w.attach(tg, l);
    let cb = if q.callback == 0 {
        callback::DEFAULT
    } else {
        q.callback
    };
    w.set_remove_callback(l, cb);
    Some(l)
}

// -----------------------------------------------------------------

fn alive_or_drops<W: BodyWorld>(w: &W, u: W::Unit, s: i32) -> bool {
    w.is_alive(u) || !w.stays_on_death(u, s)
}

/// Default remove callback.
pub fn remove_default<W: BodyWorld>(w: &mut W, u: W::Unit, s: i32) {
    unregister(w, u, 1, s);
    if alive_or_drops(w, u, s) {
        w.state_on(u, s, false);
        w.combat().refresh_anim_rate(u);
        w.passive_refresh(u);
        if w.unit_type(u) == UnitType::Player {
            w.skill_resync(u);
        }
    }
}

/// Self-aura remove callback.
pub fn remove_self_aura<W: BodyWorld>(w: &mut W, t: &SkillTables, u: W::Unit, s: i32) {
    w.state_on(u, s, false);
    if w.has_state(u, state::NOMANAREGEN as u16) {
        w.state_on(u, state::NOMANAREGEN, false);
    }
    // The passive states of the unit's skills back on.
    for e in w.skill_list(u) {
        let ps = rec(t, e.skill).map_or(-1, |r| s16(r.passivestate));
        if ps > 0 {
            w.state_on(u, ps, true);
            w.passive_state_apply(u, &e);
        }
    }
    // Life, mana, stamina clamped to their maxima.
    for s in [stat::LIFE, stat::MANA, stat::STAMINA] {
        let m = w.stat_max(u, s);
        if w.stat(u, s, 0) > m {
            w.set_stat(u, s, m);
        }
    }
    w.combat().refresh_anim_rate(u);
}

/// Buff remove callback.
pub fn remove_buff<W: BodyWorld>(w: &mut W, u: W::Unit, s: i32) {
    unregister(w, u, 1, s);
    if alive_or_drops(w, u, s) {
        w.state_on(u, s, false);
        w.combat().refresh_anim_rate(u);
        w.buff_refresh(u);
    }
}

/// AI curse remove callback.
pub fn remove_ai_curse<W: BodyWorld>(w: &mut W, u: W::Unit, s: i32) {
    unregister(w, u, 1, s);
    if w.unit_type(u) == UnitType::Monster {
        w.set_ai_state(u, 0);
    }
    remove_default(w, u, s);
}

// -----------------------------------------------------------------

/// `clear_group(unit, s, include_self)`.
pub fn clear_group<W: BodyWorld>(w: &mut W, u: W::Unit, s: i32, include_self: bool) -> bool {
    if !state_ok(w, s) {
        return false;
    }
    let g = w.state_group(s);
    if g == 0 {
        return false;
    }
    let mut res = false;
    for x in 0..w.state_count() {
        if x == s && !include_self {
            continue;
        }
        if w.state_group(x) == g && w.has_state(u, x as u16) {
            w.state_on(u, x, false);
            if let Some(l) = w.state_list(u, x) {
                w.detach_free(u, l);
            }
            res = true;
        }
    }
    res
}

// ------------------------------------------------------------------

/// `scaled(target, s, v)`: a non-positive resistance
/// value on a monster (not a hireling) with base resist ≥ 100 is cut to
/// v / 5.
pub fn scaled<W: BodyWorld>(w: &mut W, tg: W::Unit, s: i32, v: i32) -> i32 {
    if matches!(s, 36 | 37 | 39 | 41 | 43 | 45)
        && v <= 0
        && w.unit_type(tg) != UnitType::Player
        && !w.combat().is_hireling(tg)
        && w.base_stat(tg, s as u16, 0) >= 100
    {
        return v / 5;
    }
    v
}

// ------------------------------------------------------------------

/// The unit a test reads for the source: a missile's owner (none →
/// refused).
fn effective_source<W: BodyWorld>(w: &W, source: W::Unit) -> Option<W::Unit> {
    if w.unit_type(source) == UnitType::Missile {
        w.missile_owner(source)
    } else {
        Some(source)
    }
}

/// `accepts(source, unit, f)` (`aurafilter`).
pub fn accepts<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    source: W::Unit,
    u: W::Unit,
    f: u32,
) -> bool {
    let mode = w.mode(u);
    match w.unit_type(u) {
        UnitType::Player => {
            if f & 0x1 == 0 {
                return false;
            }
            let ok = if f & 0x1000 != 0 {
                mode == 17
            } else {
                mode != 0 && mode != 17
            };
            if !ok {
                return false;
            }
        }
        UnitType::Monster => {
            if f & 0x2 == 0 {
                return false;
            }
            let ok = if f & 0x1000 != 0 {
                mode == 12
            } else {
                mode != 0 && mode != 12
            };
            if !ok {
                return false;
            }
            if f & 0x4 != 0 && !w.combat().is_undead(u) {
                return false;
            }
            let ms = ct.monstats(w.class_id(u));
            if f & 0x4000 != 0 && ms.is_some_and(|m| m.boss) {
                return false;
            }
            if f & 0x40000 != 0 && ms.is_some_and(|m| m.primeevil) {
                return false;
            }
        }
        UnitType::Object => {
            if f & 0x10 == 0 {
                return false;
            }
        }
        UnitType::Missile => {
            if f & 0x8 == 0 || !t.missile(w.class_id(u)).is_some_and(|m| m.explosion) {
                return false;
            }
        }
        UnitType::Item => {
            if f & 0x20 == 0 {
                return false;
            }
        }
        _ => return false,
    }
    let flags = w.unit_flags(u);
    if f & 0x80 != 0 && flags & 0x4 == 0 {
        return false;
    }
    if f & 0x400 != 0 && flags & 0x8 == 0 {
        return false;
    }
    if f & 0x100 != 0 && w.room(u) == RoomKind::Town {
        return false;
    }
    if f & 0x10000 != 0 {
        let Some(src) = effective_source(w, source) else {
            return false;
        };
        if !w.allied(src, u) {
            return false;
        }
        if w.unit_type(u) == UnitType::Monster && ct.monstats(w.class_id(u)).is_some_and(|m| m.npc)
        {
            return false;
        }
    }
    if f & 0x8000 != 0 {
        let Some(src) = effective_source(w, source) else {
            return false;
        };
        if !w.combat().hostile(src, u) {
            return false;
        }
    }
    if f & 0x20000 != 0 {
        let Some(src) = effective_source(w, source) else {
            return false;
        };
        if !w.combat().in_melee_range(src, u, 0) {
            return false;
        }
    }
    if f & 0x80000 != 0 && w.has_state(u, state::JUSTHIT as u16) {
        return false;
    }
    if f & 0x200 != 0 {
        if w.room(source) == RoomKind::None || w.room(u) == RoomKind::None {
            return false;
        }
        let to = w.position(u);
        if !w.line_clear(source, to, 4) {
            return false;
        }
    }
    true
}

// ------------------------------------------------------------------

fn dist2((ax, ay): (i32, i32), (bx, by): (i32, i32)) -> i32 {
    let (dx, dy) = (ax.wrapping_sub(bx), ay.wrapping_sub(by));
    dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy))
}

fn rooms<W: BodyWorld>(
    w: &W,
    source: W::Unit,
    at: Option<(i32, i32)>,
    f: u32,
) -> Option<Vec<ScanRoom<W::Unit>>> {
    let mut v = w.scan_rooms(source, at)?;
    if f & 0x2000 != 0 {
        v.retain(|r| !r.town);
    }
    Some(v)
}

/// `scan_unit(game, source, x, y, r, f, cb, arg, noaura)`:
/// `cb` on every accepted unit within `r` of (x, y); returns
/// the scan count (callbacks that returned non-zero).
#[allow(clippy::too_many_arguments)]
pub fn scan_unit<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    source: W::Unit,
    (x, y): (i32, i32),
    r: i32,
    f: u32,
    noaura: bool,
    cb: &mut dyn FnMut(&mut W, W::Unit) -> i32,
) -> i32 {
    if r <= 0 || w.room(source) == RoomKind::None {
        return 0;
    }
    let center = if x == 0 || y == 0 {
        w.position(source)
    } else {
        (x, y)
    };
    let f = if f == 0 { DEFAULT_FILTER } else { f };
    let Some(rs) = rooms(w, source, None, f) else {
        return 0;
    };
    let r2 = r.wrapping_mul(r);
    let mut count = 0;
    for room in rs {
        for u in room.units {
            if u == source || dist2(w.position(u), center) > r2 {
                continue;
            }
            if !accepts(w, t, ct, source, u, f) {
                continue;
            }
            if noaura
                && w.unit_type(u) == UnitType::Monster
                && ct.monstats(w.class_id(u)).is_some_and(|m| m.noaura)
            {
                continue;
            }
            if cb(w, u) != 0 {
                count += 1;
            }
        }
    }
    count
}

/// `scan_point(game, f, source, r, cb, arg)`:
/// around the source's target position, `f = 0` kept, the source not
/// excluded. Returns 1, or 0 without a target position or room.
#[allow(clippy::too_many_arguments)]
pub fn scan_point<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    f: u32,
    source: W::Unit,
    r: i32,
    cb: &mut dyn FnMut(&mut W, W::Unit) -> i32,
) -> i32 {
    let Some(center) = w.target_position(source) else {
        return 0;
    };
    let Some(rs) = rooms(w, source, Some(center), f) else {
        return 0;
    };
    let r2 = r.wrapping_mul(r);
    for room in rs {
        for u in room.units {
            if dist2(w.position(u), center) <= r2 && accepts(w, t, ct, source, u, f) {
                cb(w, u);
            }
        }
    }
    1
}

// ------------------------------------------------------------------

/// The event function table: entry
/// 0 is null, 1–31 are filled; slots 32–49 of the 50-slot table are null,
/// so register returns 0 for them.
pub fn event_func_filled(func: i32) -> bool {
    (1..=31).contains(&func)
}

/// Register:
/// false (0) for an unknown function.
#[allow(clippy::too_many_arguments)]
pub fn register<W: BodyWorld>(
    w: &mut W,
    u: W::Unit,
    event: i32,
    skill: i32,
    lvl: i32,
    func: i32,
    key_type: i32,
    key: i32,
) -> bool {
    if func > 49 || !event_func_filled(func) {
        return false;
    }
    w.add_handler(
        u,
        Handler {
            event: event as u8,
            key_type,
            key,
            skill,
            level: lvl,
            func,
        },
    );
    true
}

/// Unregister.
pub fn unregister<W: BodyWorld>(w: &mut W, u: W::Unit, key_type: i32, key: i32) {
    w.remove_handlers(u, key_type, key);
}

// ------------------------------------------------------------------

/// The pgsv states the unit has now, in index order.
fn pgsv_states<W: BodyWorld>(w: &W, u: W::Unit) -> Vec<i32> {
    (0..w.state_count())
        .filter(|&s| w.state_flag(s, group::PGSV) && w.has_state(u, s as u16))
        .collect()
}

/// The common lookup of a progressive list P: (k, lvl, n) when k's
/// record exists, lvl > 0, `aurastat1` is valid and n = P[`aurastat1`] >
/// 0.
fn charge_of<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    p: W::List,
) -> Option<(i32, i32, i32)> {
    let k = w.list_get(p, stat::MODIFIERLIST_SKILL);
    let r = rec(t, k)?;
    let a1 = s16(r.aurastat1);
    let list = w.skill_list(u);
    let e = highest_entry(&list, k);
    let lvl =
        w.list_get(p, stat::MODIFIERLIST_LEVEL)
            .max(skill_level(w, t, Some(u), e.as_ref(), true));
    if lvl <= 0 || !stat_ok(t, a1) {
        return None;
    }
    let n = w.list_get(p, a1);
    (n > 0).then_some((k, lvl, n))
}

/// The pgsv lists of the unit whose skill is > 0 with `prgdam` = `d`.
fn charges<W: BodyWorld>(w: &mut W, t: &SkillTables, u: W::Unit, d: u8) -> Vec<(i32, i32, i32)> {
    let mut out = Vec::new();
    for s in pgsv_states(w, u) {
        let Some(p) = w.state_list(u, s) else {
            continue;
        };
        let k = w.list_get(p, stat::MODIFIERLIST_SKILL);
        if k <= 0 || !rec(t, k).is_some_and(|r| r.prgdam == d) {
            continue;
        }
        if let Some(c) = charge_of(w, t, u, p) {
            out.push(c);
        }
    }
    out
}

/// Before the roll: `prgdam` 1 charges add
/// enhanced damage and the target overlay.
pub fn charges_before<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    record: &mut DamageRecord,
) {
    if record.result & 1 == 0 {
        return;
    }
    for (k, lvl, n) in charges(w, t, u, 1) {
        let Some(r) = rec(t, k) else { continue };
        let (calc1, ov) = (r.calc1, s16(r.tgtoverlay));
        let v = eval(w, t, u, calc1, k, lvl);
        record.enh_pct = record.enh_pct.wrapping_add(n.wrapping_mul(v));
        if let Some(tg) = target(w, u) {
            if ov >= 1 && ov < w.overlay_count() {
                w.combat().overlay(tg, ov);
            }
        }
    }
}

/// After the roll: `prgdam` 2 (leech), 3 (element,
/// freeze), 4 (element, freeze, physical conversion).
pub fn charges_after<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    record: &mut DamageRecord,
) {
    if record.result & 1 == 0 {
        return;
    }
    for d in pgsv_states(w, u) {
        let Some(p) = w.state_list(u, d) else {
            continue;
        };
        let k = w.list_get(p, stat::MODIFIERLIST_SKILL);
        let Some(r) = rec(t, k).filter(|_| k > 0) else {
            continue;
        };
        let (prg, etype, p2, p5, calc1) =
            (r.prgdam, r.etype, r.param2 as i32, r.param5 as i32, r.calc1);
        if !matches!(prg, 2..=4) {
            continue;
        }
        let Some((k, lvl, n)) = charge_of(w, t, u, p) else {
            continue;
        };
        match prg {
            2 => {
                // `ln12` of k at lvl (`skillcalc.tsv` code 0).
                let x = special(w, t, Some(u), 0, k, lvl);
                let rr = &mut *record;
                match n {
                    1 => rr.life_leech = rr.life_leech.wrapping_add(x),
                    2 => {
                        rr.life_leech = rr.life_leech.wrapping_add(x);
                        rr.mana_leech = rr.mana_leech.wrapping_add(x);
                    }
                    3 => {
                        rr.life_leech = rr.life_leech.wrapping_add(x.wrapping_mul(2));
                        rr.mana_leech = rr.mana_leech.wrapping_add(x.wrapping_mul(2));
                    }
                    _ => {}
                }
            }
            3 => {
                roll_elemental(w, t, u, record, k, lvl);
                if etype == 4 && matches!(n, 2 | 3) && p2 != 0 {
                    record.freeze_len = record.freeze_len.wrapping_add(record.cold_len / p2);
                }
                // `result |= 0x4000` follows the freeze clause
                // (unconditional).
                record.result |= 0x4000;
            }
            _ => {
                roll_elemental(w, t, u, record, k, lvl);
                if etype == 4 && n == 3 && p5 != 0 {
                    record.freeze_len = record.freeze_len.wrapping_add(record.cold_len / p5);
                }
                // Unconditional, as for prgdam 3.
                record.result |= 0x4000;
                let mut pc = eval(w, t, u, calc1, k, lvl);
                if pc > 0 {
                    pc = pc.min(100);
                    let ph = record.physical;
                    let c = pct(ph, pc, 100).min(ph);
                    record.physical = ph.wrapping_sub(c);
                    add_element(w, u, record, i32::from(etype), c, 0);
                }
            }
        }
    }
}

/// Finisher.
pub fn finisher<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    record: &DamageRecord,
) {
    if record.result & 1 == 0 {
        return;
    }
    let Some(tg) = target(w, u) else {
        return;
    };
    if !w.combat().in_melee_range(u, tg, 0) {
        return;
    }
    flag_40(w, u);
    for s in 0..w.state_count() {
        if !w.state_flag(s, group::PGSV) || !w.has_state(u, s as u16) {
            continue;
        }
        let Some(p) = w.state_list(u, s) else {
            continue;
        };
        finish_one(w, t, ct, u, tg, p);
        w.detach_free(u, p);
    }
    w.clear_group_states(u, group::PGSV);
}

/// Steps 1–5 of the finisher for one list P.
fn finish_one<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    tg: W::Unit,
    p: W::List,
) {
    let k = w.list_get(p, stat::MODIFIERLIST_SKILL);
    let Some(r) = rec(t, k) else {
        return;
    };
    let a1 = s16(r.aurastat1);
    let (stack, funcs) = (r.prgstack, [r.srvprgfunc1, r.srvprgfunc2, r.srvprgfunc3]);
    let list = w.skill_list(u);
    let e = highest_entry(&list, k);
    let lvl =
        w.list_get(p, stat::MODIFIERLIST_LEVEL)
            .max(skill_level(w, t, Some(u), e.as_ref(), true));
    if lvl <= 0 || !stat_ok(t, a1) {
        return;
    }
    let n = w.list_get(p, a1).clamp(1, 3);
    let roll = {
        let mut c = *w.seed(u);
        c.step()
    };
    let call = |w: &mut W, f: u16| {
        if (1..=190).contains(&f) && table::lookup(Kind::Do, f).is_some() {
            do_slot(w, t, ct, f, u, k, lvl);
        }
    };
    if stack {
        for i in 1..n {
            w.list_set(p, a1, i);
            call(w, funcs[(i - 1) as usize]);
        }
        w.list_set(p, a1, n);
    }
    call(w, funcs[(n - 1) as usize]);
    w.queue_progressive(
        u,
        ProgressiveMsg {
            charges: n as u8,
            skill: k as u16,
            level: lvl as u16,
            unit: u,
            target: tg,
            roll,
        },
    );
}

// ------------------------------------------------------------------

/// `shape_start(game, unit, L)`.
pub fn shape_start<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    lvl: i32,
) -> i32 {
    let Some(tg) = target(w, u) else {
        return 0;
    };
    let (mut bonus, mut s) = (0i32, 128);
    for st in 0..w.state_count() {
        if !w.state_flag(st, group::DISGUISE) || !w.has_state(u, st as u16) {
            continue;
        }
        let Some(l) = w.state_list(u, st) else {
            continue;
        };
        let k = w.list_get(l, stat::MODIFIERLIST_SKILL);
        let Some(r) = rec(t, k) else {
            continue;
        };
        let src = i32::from(r.srcdam);
        bonus = bonus.wrapping_add(crate::skills::to_hit(w, t, Some(u), k, lvl));
        if src != 0 {
            s = src;
        }
    }
    let mut record = DamageRecord {
        result: crate::combat::melee_result(w.combat(), t, ct, Some(u), Some(tg), bonus, 0),
        hit_class: 1,
        ..DamageRecord::default()
    };
    crate::combat::start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, s);
    1
}

// ------------------------------------------------------------------

/// Pet-maximum resync: the largest
/// `petmax` of the player's skills per pet type, then `basemax` for every
/// pet type no skill raised. `set_max(w, t, v)` is the pet maximum set (the pet
/// lists' maximum, `player::pets::set_max`: only v = 1 for type 1, never
/// trims type 7); `basemax(t)` is `pettype` `basemax` of row `t`, `None`
/// without a row. The caller has checked the player's pet lists; an empty
/// skill list stands for the missing list (nothing happens).
pub fn pet_resync<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    basemax: &dyn Fn(i32) -> Option<i32>,
    set_max: &mut dyn FnMut(&mut W, i32, i32),
) {
    if w.unit_type(u) != UnitType::Player {
        return;
    }
    let list = w.skill_list(u);
    if list.is_empty() {
        return;
    }
    let count = w.pettype_count();
    let mut m = vec![0i32; usize::try_from(count).unwrap_or(0)];
    for e in &list {
        let lvl = skill_level(w, t, Some(u), Some(e), true);
        let Some(r) = rec(t, e.skill) else { continue };
        let (pt, petmax) = (i32::from(r.pettype as i8), r.petmax);
        if pt <= 0 || pt >= count {
            continue;
        }
        let v = eval(w, t, u, petmax, e.skill, lvl).max(1);
        if v > m[pt as usize] {
            m[pt as usize] = v;
            set_max(w, pt, v);
        }
    }
    for (pt, &have) in m.iter().enumerate() {
        if have <= 0 {
            if let Some(b) = basemax(pt as i32) {
                set_max(w, pt as i32, b);
            }
        }
    }
}

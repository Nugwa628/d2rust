//! The unit event functions: the table of functions 1–31 that skills and item properties
//! register on a unit and that run when something happens to it (it hits, is hit, kills,
//! dies). Functions 15 and 16 are [`super::damage::open_wounds`] and
//! [`super::damage::crushing_blow`]; the rest are here, together with the item cast
//! (`cast`, `cast_point`, `core`), Reanimate's raise, and `run`, which walks a unit's
//! handlers for an event.
//!
//! In the function docs, H is the unit holding the handler, O is the other unit in the
//! event, `k` is the registered key (a stat id and layer, or a skill) and `l` the
//! registered level.
//!
//! The functions run on a [`BodyWorld`] (units, stats, state lists, combat, skills,
//! paths); what no body seam covers is [`EventWorld`].
//!
//! A mod game's event functions (`docs/plans/mods/seams.md` MS7b,
//! [`crate::items::provider::ItemProvider`]) are asked first through
//! [`EventWorld::mod_event`]; see [`call`].

use super::damage::{apply, crushing_blow, open_wounds, DamageRecord};
use super::{pct, CombatTables, CombatWorld};
use crate::skills::levels::eval_skill;
use crate::skills::levels::{dm, roll_elemental};
use crate::skills::use_::bodies::dos::{curse_unit, events as aura_events, CurseCtx};
use crate::skills::use_::bodies::effects::PathOp;
use crate::skills::use_::bodies::{
    apply_state, b4_helpers::dir8, param, rec, s16, stat_ok, state_ok, BodyWorld, Handler,
    MissileRequest, StateRequest,
};
use crate::skills::use_::{do_core, start_core_no_mana, UseWorld};
use crate::skills::SkillTables;
use crate::units::UnitType;

/// Stats read and written here (`itemstatcost` rows).
pub mod stat {
    /// Current life.
    pub const LIFE: u16 = 6;
    /// Current mana.
    pub const MANA: u16 = 8;
    /// Unit level.
    pub const LEVEL: u16 = 12;
    /// Velocity percent (movement speed).
    pub const VELOCITYPERCENT: i32 = 67;
    /// Attack rate.
    pub const ATTACKRATE: i32 = 68;
    /// Other animation rate.
    pub const OTHER_ANIMRATE: i32 = 69;
    /// Stat list value last sent to the client for a state's display.
    pub const UNSENTPARAM1: i32 = 84;
    /// Item slow percent.
    pub const ITEM_SLOW: u16 = 150;
}

/// States set here.
pub mod state {
    /// Slowed.
    pub const SLOWED: i32 = 24;
    /// Cannot be interrupted.
    pub const UNINTERRUPTABLE: u16 = 54;
    /// Revived (raised by Reanimate).
    pub const REVIVE: i32 = 96;
    /// The corpse cannot be selected.
    pub const CORPSE_NOSELECT: i32 = 118;
    /// Rest in peace: the victim leaves no usable corpse.
    pub const RESTINPEACE: i32 = 172;
    /// The corpse is not drawn.
    pub const CORPSE_NODRAW: i32 = 173;
}

/// Event 6 (`events.txt` index): the unit deals missile damage. Several functions use
/// different formulas for it.
pub const EV_DOMISSILEDAMAGE: i32 = 6;
/// Event 13: the death animation ended (Reanimate's handler).
pub const EV_DEATH_END: u8 = 13;
/// The function key that Reanimate stores in a handler record to mean "raise the dead
/// unit". It is not an index into the 1–31 table; `call` dispatches it to `raise`.
pub const RAISE_FUNC: i32 = 0x005C_07A0;
/// Life leech overlay.
pub const OVERLAY_LIFE: i32 = 151;
/// Mana leech overlay.
pub const OVERLAY_MANA: i32 = 152;
/// Skill id of Dim Vision.
pub const SKILL_DIM_VISION: i32 = 71;
/// Skill id of Howl.
pub const SKILL_HOWL: i32 = 130;
/// Monster class `bloodgolem`.
pub const CLASS_BLOODGOLEM: i32 = 290;
/// The elemental record's result flags: hit, no events, soft hit.
pub const ELEMENTAL_RESULT: u16 = 0x4021;

/// An item-cast queue entry on the unit's message list, sent to the client as packet
/// 0x99 (unit target) or 0x9A (point target).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemCastMsg {
    /// 0x99: skill, level, target type and GUID, aim.
    Unit {
        /// Skill id.
        skill: i32,
        level: i32,
        /// Target unit type and GUID.
        target: (u32, u32),
        /// Whether the cast aims at the target.
        aim: bool,
    },
    /// 0x9A: skill, level, point, aim.
    Point {
        /// Skill id.
        skill: i32,
        level: i32,
        /// Target point.
        at: (i32, i32),
        /// Whether the cast aims at the point.
        aim: bool,
    },
}

/// A step of Reanimate's raise on the new monster N that `d2-sim` has no body for here,
/// handed to the world in the raise's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaiseStep<U> {
    /// Re-install the AI on a fresh monster.
    AiReinstall,
    /// Restart the monster's think step.
    ThinkRestart,
    /// Set the owner data to the player P.
    OwnerData(U),
    /// Leash N to the player P.
    Leash(U),
    /// Set alignment to 2 (good).
    AlignmentGood,
    /// Apply unique modifier 21.
    Umod21,
    /// Insert N into the player's minion list.
    NodeInsert(U),
    /// `ResurrectMode` 14: mode request 14 with N's used skill set to its native entry of
    /// skill 158, step count 1, target point (x, y), flag 0.
    Resurrect(i32, i32),
}

/// The world calls the event functions need beyond [`BodyWorld`].
pub trait EventWorld: BodyWorld {
    /// The global layer split: (shift, mask) that separates a registered layer into a
    /// skill id and a skill level.
    fn layer_split(&self) -> (u32, u32);
    /// The owner of a stat list, resolved to a unit.
    fn list_owner(&self, l: Self::List) -> Option<Self::Unit>;
    /// The unit's GUID.
    fn guid(&self, u: Self::Unit) -> u32;
    /// Installs terror (flee) on `unit` from `source`, with `skill` and two parameters.
    fn terror(&mut self, source: Self::Unit, unit: Self::Unit, skill: i32, a: i32, b: i32);
    /// The point is free of collisions for the unit (its room, pattern and collision mask).
    fn point_free(&self, u: Self::Unit, at: (i32, i32)) -> bool;
    /// The corpse find of item target 3: the first unit of the unit find around `t0`,
    /// radius 10, flags 0x1002, filtered by the corpse test, in `t0`'s room.
    fn corpse_near(&mut self, t0: Self::Unit) -> Option<Self::Unit>;
    /// Queue an item-cast message on the unit's message list and queue the unit for
    /// update.
    fn queue_item_cast(&mut self, u: Self::Unit, msg: ItemCastMsg);
    /// The raise test: the dead unit `v` can be raised.
    fn raise_test(&self, v: Self::Unit) -> bool;
    /// Clear V's pattern from its room (so the raised monster can stand there).
    fn clear_pattern(&mut self, v: Self::Unit);
    /// A raise step on N ([`RaiseStep`]).
    fn raise_step(&mut self, n: Self::Unit, step: RaiseStep<Self::Unit>);
    /// The unit's handler records in list order.
    fn handlers_of(&self, u: Self::Unit) -> Vec<Handler>;
    /// Unlink and free one handler record (the first equal one).
    fn remove_handler(&mut self, u: Self::Unit, h: &Handler);
    /// The path record's target point as stored, (0, 0) when unset, for the casts with no
    /// O in skill-on-attack and skill-on-get-hit. Default: [`BodyWorld::path_target_point`].
    fn path_point(&self, u: Self::Unit) -> (i32, i32) {
        self.path_target_point(u)
    }
    /// A mod game's event function `func` for this call (MS7b): `Some`
    /// when the game's rules have one and it answered, `None` to run
    /// D2RUST's. Default: D2RUST's own rules (none).
    fn mod_event(
        &mut self,
        func: i32,
        c: EventCall<Self::Unit>,
        r: Option<&mut DamageRecord>,
    ) -> Option<i32> {
        let _ = (func, c, r);
        None
    }
}

/// The tables the functions read.
#[derive(Clone, Copy)]
pub struct EventTables<'a> {
    /// Skill tables.
    pub skills: &'a SkillTables,
    /// Combat tables.
    pub combat: &'a CombatTables,
}

/// The arguments of one event function call.
#[derive(Debug, Clone, Copy)]
pub struct EventCall<U> {
    /// The event (`events.txt` index).
    pub event: i32,
    /// H, the unit holding the handler.
    pub h: Option<U>,
    /// O, the other unit in the event.
    pub o: Option<U>,
    /// The registered key (stat id << 16 | layer, or a skill).
    pub k: i32,
    /// The registered level.
    pub l: i32,
}

// ---------------------------------------------------------------- helpers

/// v = H's item/skill value of stat (k >> 16) at layer (k & 0xFFFF).
fn item_value<W: EventWorld>(w: &W, h: W::Unit, k: i32) -> i32 {
    w.item_stat(h, ((k as u32) >> 16) as u16, k as u16)
}

/// The (skill, level) pair of the registration's layer, split by the world's layer split.
fn split<W: EventWorld>(w: &W, k: i32) -> (i32, i32) {
    let layer = (k as u32) & 0xFFFF;
    let (shift, mask) = w.layer_split();
    ((layer >> (shift & 31)) as i32, (layer & mask) as i32)
}

/// The unit's level stat.
/// The unit's level stat.
fn level<W: EventWorld>(w: &W, u: W::Unit) -> i32 {
    w.stat(u, stat::LEVEL, 0)
}

/// True for a player or monster.
/// True for a player or monster.
fn player_or_monster<W: EventWorld>(w: &W, u: W::Unit) -> bool {
    matches!(w.unit_type(u), UnitType::Player | UnitType::Monster)
}

/// Heals the unit by `x` up to its maximum life and returns the amount applied.
pub fn heal<W: EventWorld>(w: &mut W, u: W::Unit, x: i32) -> i32 {
    if x <= 0 {
        return 0;
    }
    let mut x = x;
    let mut life = w.stat(u, stat::LIFE, 0).wrapping_add(x);
    let max = w.stat_max(u, stat::LIFE);
    if life > max {
        x = x.wrapping_sub(life.wrapping_sub(max));
        life = max;
    }
    w.combat().set_stat(u, stat::LIFE, life);
    x
}

/// The overlay id when valid (0 ≤ id < overlay count).
fn overlay_if_valid<W: EventWorld>(w: &mut W, u: W::Unit, id: i32) {
    if (0..w.overlay_count()).contains(&id) {
        w.combat().overlay(u, id);
    }
}

/// The owner C of the holder's state list for `auratargetstate` (Iron Maiden, Life Tap),
/// or `None` without that state.
fn aura_target_owner<W: EventWorld>(w: &W, h: W::Unit, ats: i32) -> Option<W::Unit> {
    if !state_ok(w, ats) || !w.has_state(h, ats as u16) {
        return None;
    }
    let l = w.state_list(h, ats)?;
    w.list_owner(l)
}

/// Evaluates a skill formula with `u` as the caster.
/// Evaluates a skill formula with `u` as the caster.
fn eval_on<W: EventWorld>(w: &mut W, t: &SkillTables, u: W::Unit, c: u32, k: i32, l: i32) -> i32 {
    eval_skill(w, t, Some(u), c, k, l)
}

/// The skill-event level rule: L := 1 when L < 0.
fn skill_level(l: i32) -> i32 {
    if l < 0 {
        1
    } else {
        l
    }
}

// ---------------------------------------------------------------- functions

/// Runs event function `func` (1–31, or `RAISE_FUNC`) once and returns its result: 1 when
/// it acted, 0 when not. `r` is the event's damage record. A mod game's function for `func`
/// ([`EventWorld::mod_event`]) runs first, also for ids past D2RUST's; when it doesn't
/// answer, D2RUST's runs (an unknown id does nothing).
pub fn call<W: EventWorld>(
    w: &mut W,
    tb: EventTables<'_>,
    func: i32,
    c: EventCall<W::Unit>,
    mut r: Option<&mut DamageRecord>,
) -> i32 {
    if let Some(v) = w.mod_event(func, c, r.as_deref_mut()) {
        return v;
    }
    match func {
        1 => chilling_armor(w, tb, c),
        2 => frozen_armor(w, tb, c, r),
        3 => shiver_armor(w, tb, c),
        4 => iron_maiden(w, tb, c, r),
        5 => life_tap(w, tb, c, r),
        6 | 10 | 11 | 12 => attacker_takes(w, tb, func, c),
        7 => knockback(w, tb, c, r),
        8 => howl(w, c),
        9 => stupidity(w, tb, c),
        13 => damage_to_mana(w, c, r),
        14 => freeze(w, tb, c),
        15 => match (c.h, c.o) {
            (Some(h), Some(o)) => {
                i32::from(open_wounds(w.combat(), c.event as u8, h, o, c.k as u32).is_some())
            }
            _ => 0,
        },
        16 => match (c.h, c.o, r) {
            (Some(h), Some(o), Some(r)) => {
                crushing_blow(w.combat(), c.event as u8, h, o, r, c.k as u32)
            }
            _ => 0,
        },
        17 | 18 | 28 => after_kill(w, func, c),
        19 | 27 => slow(w, tb, func, c),
        20 | 30 => skill_on(w, tb, func, c, r.as_deref()),
        21 => skill_on_get_hit(w, tb, c, r.as_deref()),
        22 | 25 => absorb(w, tb, func, c, r),
        23 => blood_golem_done(w, tb, c, r.as_deref()),
        24 => energy_shield(w, tb, c, r),
        26 => blood_golem_taken(w, tb, c, r),
        29 => match (c.h, c.o) {
            (Some(_), Some(o)) => {
                w.state_on(o, state::RESTINPEACE, true);
                1
            }
            _ => 0,
        },
        31 => reanimate(w, c),
        RAISE_FUNC => match c.h {
            Some(v) => raise(w, tb, v, c.k, c.l),
            None => 0,
        },
        _ => 0,
    }
}

/// Function 1, Chilling Armor: when the armor's holder is hit by a missile with return
/// fire (O is the missile), fires a missile back at the missile's owner.
fn chilling_armor<W: EventWorld>(w: &mut W, tb: EventTables<'_>, c: EventCall<W::Unit>) -> i32 {
    let (Some(h), Some(o)) = (c.h, c.o) else {
        return 0;
    };
    let Some(rs) = rec(tb.skills, c.k) else {
        return 0;
    };
    let l = skill_level(c.l);
    let Some(p) = w.missile_owner(o) else {
        return 0;
    };
    if !w.combat().hostile(h, p) {
        return 0;
    }
    let m = s16(rs.srvmissilea);
    if tb.skills.missile(m).is_none() {
        return 0;
    }
    if !tb
        .skills
        .missile(w.class_id(o))
        .is_some_and(|row| row.returnfire)
    {
        return 0;
    }
    let (px, py) = w.position(p);
    let mut q = MissileRequest::new(h, m);
    q.flags = 0x20;
    q.origin = Some(h);
    q.target_x = px;
    q.target_y = py;
    q.skill = c.k;
    q.level = l;
    w.spawn_missile(q);
    1
}

/// Function 2, Frozen Armor: freezes O (a player or monster) for the skill's `calc1` length
/// when the event's record carries physical damage.
fn frozen_armor<W: EventWorld>(
    w: &mut W,
    tb: EventTables<'_>,
    c: EventCall<W::Unit>,
    r: Option<&mut DamageRecord>,
) -> i32 {
    let (Some(h), Some(o)) = (c.h, c.o) else {
        return 0;
    };
    let Some(rs) = rec(tb.skills, c.k) else {
        return 0;
    };
    let (calc1, ov) = (rs.calc1, s16(rs.cltoverlaya));
    let l = skill_level(c.l);
    if !player_or_monster(w, o) || r.is_some_and(|r| r.physical <= 0) {
        return 0;
    }
    let mut r2 = DamageRecord {
        result: 0x20,
        ..DamageRecord::default()
    };
    r2.freeze_len = eval_on(w, tb.skills, h, calc1, c.k, l);
    apply(w.combat(), tb.combat, h, o, true, &mut r2);
    overlay_if_valid(w, o, ov);
    1
}

/// Function 3, Shiver Armor: hits O with an elemental record rolled from the skill.
fn shiver_armor<W: EventWorld>(w: &mut W, tb: EventTables<'_>, c: EventCall<W::Unit>) -> i32 {
    let (Some(h), Some(o)) = (c.h, c.o) else {
        return 0;
    };
    let Some(rs) = rec(tb.skills, c.k) else {
        return 0;
    };
    let ov = s16(rs.cltoverlaya);
    let l = skill_level(c.l);
    if !player_or_monster(w, o) {
        return 0;
    }
    let mut r2 = DamageRecord::default();
    roll_elemental(w, tb.skills, h, &mut r2, c.k, l);
    r2.hit_class |= 0xD;
    r2.hit_class_fixed = 1;
    r2.result = ELEMENTAL_RESULT;
    apply(w.combat(), tb.combat, h, o, true, &mut r2);
    w.combat().reaction(h, o, &mut r2);
    overlay_if_valid(w, o, ov);
    1
}

/// Function 4, Iron Maiden: a percent of the physical damage goes back to H, with O as the
/// attacker; then the blood golem branch.
fn iron_maiden<W: EventWorld>(
    w: &mut W,
    tb: EventTables<'_>,
    c: EventCall<W::Unit>,
    r: Option<&mut DamageRecord>,
) -> i32 {
    let (Some(h), Some(o), Some(r)) = (c.h, c.o, r) else {
        return 0;
    };
    let Some(rs) = rec(tb.skills, c.k) else {
        return 0;
    };
    let l = skill_level(c.l);
    if !player_or_monster(w, h) || !w.is_alive(h) || r.physical <= 0 {
        return 0;
    }
    let Some(cu) = aura_target_owner(w, h, s16(rs.auratargetstate)) else {
        return 0;
    };
    let calc = if w.unit_type(h) == UnitType::Player && !w.combat().is_hireling(h) {
        rs.calc1
    } else if w.unit_type(o) == UnitType::Player {
        rs.calc2
    } else {
        rs.calc3
    };
    let (rf, hf, hc) = (rs.resultflags, rs.hitflags, rs.hitclass);
    let p = eval_on(w, tb.skills, cu, calc, c.k, l);
    let mut r2 = DamageRecord {
        physical: pct(r.physical, p, 100),
        ..DamageRecord::default()
    };
    r2.result |= rf | 0x20;
    r2.hit_flags |= hf;
    r2.hit_class = hc;
    apply(w.combat(), tb.combat, o, h, true, &mut r2);
    w.combat().reaction(o, h, &mut r2);
    if w.unit_type(o) != UnitType::Monster || w.class_id(o) != CLASS_BLOODGOLEM {
        return 1;
    }
    // The blood golem branch: when O is a blood golem, H and H's owner are healed.
    let d = drain(w, tb.combat, o);
    if d <= 0 || w.combat().alignment(o) != 0 {
        return 1;
    }
    let mut x = r2.physical;
    if d != 100 {
        x = pct(x, d, 100);
    }
    let q = w.minion_owner(h);
    if q.is_some_and(|q| w.combat().is_dead(q)) || w.combat().is_dead(h) {
        return 1;
    }
    x = pct(x, 20, 100);
    if x <= 0 {
        return 1;
    }
    share_heal(w, h, q, x, 50);
    w.combat().overlay(h, OVERLAY_LIFE);
    if let Some(q) = q {
        w.combat().overlay(q, OVERLAY_LIFE);
    }
    1
}

/// The monster's `Drain` for the game's difficulty.
fn drain<W: EventWorld>(w: &mut W, ct: &CombatTables, u: W::Unit) -> i32 {
    let d = w.combat().difficulty();
    ct.monstats(w.class_id(u)).map_or(0, |m| {
        i32::from(match d {
            0 => m.drain,
            1 => m.drain_n,
            _ => m.drain_h,
        })
    })
}

/// Shares a heal of `x` between the golem H and its owner Q: Q first takes `g` percent,
/// H takes what it can, and any remainder goes to Q.
fn share_heal<W: EventWorld>(w: &mut W, h: W::Unit, q: Option<W::Unit>, x: i32, g: i32) {
    let mut x = x;
    if let Some(q) = q {
        if g > 0 {
            x = x.wrapping_sub(heal(w, q, pct(x, g, 100)));
        }
    }
    x = x.wrapping_sub(heal(w, h, x));
    if let Some(q) = q {
        if x > 0 {
            heal(w, q, x);
        }
    }
}

/// Function 5, Life Tap: heals O by a percent of the physical damage in the record.
fn life_tap<W: EventWorld>(
    w: &mut W,
    tb: EventTables<'_>,
    c: EventCall<W::Unit>,
    r: Option<&mut DamageRecord>,
) -> i32 {
    let (Some(h), Some(o), Some(r)) = (c.h, c.o, r) else {
        return 0;
    };
    let Some(rs) = rec(tb.skills, c.k) else {
        return 0;
    };
    let (calc1, prg) = (rs.calc1, s16(rs.prgoverlay));
    let l = skill_level(c.l);
    if !player_or_monster(w, h) || !player_or_monster(w, o) || !w.is_alive(o) || r.physical <= 0 {
        return 0;
    }
    let Some(cu) = aura_target_owner(w, h, s16(rs.auratargetstate)) else {
        return 0;
    };
    let x = pct(r.physical, eval_on(w, tb.skills, cu, calc1, c.k, l), 100);
    let life = w.stat(o, stat::LIFE, 0);
    let max = w.stat_max(o, stat::LIFE);
    if life <= 0 || max <= 0 {
        return 0;
    }
    w.combat()
        .set_stat(o, stat::LIFE, life.wrapping_add(x).clamp(1, max));
    if prg > 0 {
        overlay_if_valid(w, o, prg);
    }
    1
}

/// Functions 6, 10, 11 and 12, attacker takes damage: physical, lightning, fire and cold
/// damage dealt back to O.
fn attacker_takes<W: EventWorld>(
    w: &mut W,
    tb: EventTables<'_>,
    func: i32,
    c: EventCall<W::Unit>,
) -> i32 {
    let (Some(h), Some(o)) = (c.h, c.o) else {
        return 0;
    };
    if w.unit_flags(o) & 0x4 == 0 {
        return 0;
    }
    let v = item_value(w, h, c.k);
    if v <= 0 {
        return 0;
    }
    let rec = attacker_takes_record(func, v, level(w, h), level(w, o));
    let mut r = rec;
    apply(w.combat(), tb.combat, h, o, true, &mut r);
    w.combat().reaction(h, o, &mut r);
    1
}

/// The damage record that functions 6, 10, 11 and 12 deal for value v and the levels of H
/// and O. Cold lengthens with the level difference.
pub fn attacker_takes_record(func: i32, v: i32, hl: i32, ol: i32) -> DamageRecord {
    let mut r = DamageRecord {
        result: ELEMENTAL_RESULT,
        ..DamageRecord::default()
    };
    let x = v.wrapping_shl(8);
    match func {
        6 => (r.physical, r.hit_class) = (x, 0x8D),
        10 => (r.lightning, r.hit_class) = (x, 0x4D),
        11 => (r.fire, r.hit_class) = (x, 0x2D),
        _ => {
            r.cold = x;
            r.cold_len = if hl > ol {
                10i32.wrapping_mul(hl.wrapping_sub(ol)).wrapping_add(25)
            } else {
                25
            };
            r.hit_class = 0x3D;
        }
    }
    r
}

/// Function 7, Knockback: sets the knockback result flag on the record, with a chance that
/// depends on O's size (large 1/4, small 1/1, otherwise 1/2 of the 128 draw).
fn knockback<W: EventWorld>(
    w: &mut W,
    tb: EventTables<'_>,
    c: EventCall<W::Unit>,
    r: Option<&mut DamageRecord>,
) -> i32 {
    let (Some(h), Some(o), Some(r)) = (c.h, c.o, r) else {
        return 0;
    };
    if item_value(w, h, c.k) <= 0 {
        return 0;
    }
    let mut t = 0x40;
    if w.unit_type(o) == UnitType::Monster {
        if let Some(m2) = tb.combat.monstats2(w.class_id(o)) {
            if m2.large {
                t = 0x20;
            } else if m2.small {
                t = 0x80;
            }
        }
    }
    if w.seed(h).mask(128) < t {
        r.result |= 8;
        return 1;
    }
    0
}

/// Function 8, Howl: a chance (v out of 128) to terrify a plain monster O.
fn howl<W: EventWorld>(w: &mut W, c: EventCall<W::Unit>) -> i32 {
    let (Some(h), Some(o)) = (c.h, c.o) else {
        return 0;
    };
    if w.unit_type(o) != UnitType::Monster || w.combat().monster_flag(o, 12) {
        return 0;
    }
    let v = item_value(w, h, c.k);
    if v <= 0 {
        return 0;
    }
    if (w.seed(h).mask(128) as i32) < v {
        w.terror(h, o, SKILL_HOWL, 20, 20);
    }
    1
}

/// Function 9, Stupidity: a Dim Vision curse on O alone.
fn stupidity<W: EventWorld>(w: &mut W, tb: EventTables<'_>, c: EventCall<W::Unit>) -> i32 {
    let (Some(h), Some(o)) = (c.h, c.o) else {
        return 0;
    };
    if w.unit_type(o) != UnitType::Monster {
        return 0;
    }
    let v = item_value(w, h, c.k);
    if v == 0 {
        return 0;
    }
    let cv = stupidity_chance(level(w, h), level(w, o), v, c.event == EV_DOMISSILEDAMAGE);
    let r = w.seed(h).roll(100) as i32;
    if r >= cv {
        return 0;
    }
    dim_vision(w, tb, h, o, stupidity_level(cv, r));
    1
}

/// The Stupidity chance: c = 5 × (H level + 4v − O level + 6), divided by 3 for a missile
/// when positive, clamped to 1…99.
pub fn stupidity_chance(hl: i32, ol: i32, v: i32, missile: bool) -> i32 {
    let mut c = 5i32.wrapping_mul(
        hl.wrapping_add(v.wrapping_mul(4))
            .wrapping_sub(ol)
            .wrapping_add(6),
    );
    if missile && c > 0 {
        c /= 3;
    }
    c.clamp(1, 99)
}

/// The Dim Vision level for Stupidity: d = (c − r) / 5 + 1, clamped to 1…20.
pub fn stupidity_level(c: i32, r: i32) -> i32 {
    (c.wrapping_sub(r) / 5 + 1).clamp(1, 20)
}

/// Casts Dim Vision at level `d` from H on O alone: builds the curse context from the
/// skill's aura stats (ai curse, skill 71) and applies it to O.
fn dim_vision<W: EventWorld>(w: &mut W, tb: EventTables<'_>, h: W::Unit, o: W::Unit, d: i32) {
    let t = tb.skills;
    let Some(rs) = rec(t, SKILL_DIM_VISION) else {
        return;
    };
    let pairs = [
        (rs.aurastat1, rs.aurastatcalc1),
        (rs.aurastat2, rs.aurastatcalc2),
        (rs.aurastat3, rs.aurastatcalc3),
        (rs.aurastat4, rs.aurastatcalc4),
        (rs.aurastat5, rs.aurastatcalc5),
        (rs.aurastat6, rs.aurastatcalc6),
    ];
    let (lencalc, ts, ev) = (rs.auralencalc, s16(rs.auratargetstate), aura_events(rs));
    let mut dur = eval_on(w, t, h, lencalc, SKILL_DIM_VISION, d);
    let div = tb
        .combat
        .difficulty(w.combat().difficulty())
        .map_or(0, |x| x.aicursedivisor as i32);
    if div != 0 {
        dur /= div;
    }
    let mut cx = CurseCtx {
        ai: true,
        upd: false,
        skill: SKILL_DIM_VISION,
        lvl: d,
        duration: dur,
        stats: [-1; 6],
        values: [0; 6],
        state: ts,
        events: ev,
    };
    for (i, &(s, calc)) in pairs.iter().enumerate() {
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
        cx.values[i] = eval_on(w, t, h, calc, SKILL_DIM_VISION, d);
    }
    curse_unit(w, t, tb.combat, h, &cx, o);
}

/// Function 13, Damage to mana: gives a player H mana equal to v percent of the damage
/// taken.
fn damage_to_mana<W: EventWorld>(
    w: &mut W,
    c: EventCall<W::Unit>,
    r: Option<&mut DamageRecord>,
) -> i32 {
    let (Some(h), Some(r)) = (c.h, r) else {
        return 0;
    };
    if w.unit_type(h) != UnitType::Player || r.total <= 0 {
        return 0;
    }
    let v = item_value(w, h, c.k);
    if v <= 0 {
        return 0;
    }
    let max = w.stat_max(h, stat::MANA);
    let m = w.stat(h, stat::MANA, 0);
    if m >= max {
        return 0;
    }
    let mana = m.wrapping_add(pct(r.total, v, 100)).max(0).min(max);
    w.combat().set_stat(h, stat::MANA, mana);
    w.combat().overlay(h, OVERLAY_MANA);
    1
}

/// Function 14, Freeze: a chance to freeze O (no hit reaction).
fn freeze<W: EventWorld>(w: &mut W, tb: EventTables<'_>, c: EventCall<W::Unit>) -> i32 {
    let (Some(h), Some(o)) = (c.h, c.o) else {
        return 0;
    };
    let v = item_value(w, h, c.k);
    if v <= 0 {
        return 0;
    }
    let cv = freeze_chance(v, level(w, h), level(w, o), c.event == EV_DOMISSILEDAMAGE);
    let r = (w.seed(h).step() % 100) as i32;
    if r >= cv {
        return 0;
    }
    let len = freeze_length(cv, r);
    let mut r2 = DamageRecord {
        freeze_len: len,
        ..DamageRecord::default()
    };
    apply(w.combat(), tb.combat, h, o, true, &mut r2);
    1
}

/// The Freeze chance: c = 5 × (4·max(v − 1, 0) − O level + H level (− 6 for a missile)
/// + 10), divided by 3 for a missile, clamped to 0…100.
pub fn freeze_chance(v: i32, hl: i32, ol: i32, missile: bool) -> i32 {
    let hl = if missile { hl.wrapping_sub(6) } else { hl };
    let mut c = 5i32.wrapping_mul(
        4i32.wrapping_mul(v.wrapping_sub(1).max(0))
            .wrapping_sub(ol)
            .wrapping_add(hl)
            .wrapping_add(10),
    );
    if missile {
        c /= 3;
    }
    c.clamp(0, 100)
}

/// The Freeze length in frames: clamp(2(c − r) + 25, 25, 250).
pub fn freeze_length(c: i32, r: i32) -> i32 {
    c.wrapping_sub(r)
        .wrapping_mul(2)
        .wrapping_add(25)
        .clamp(25, 250)
}

/// Mana after kill (17), heal after demon kill (18) and heal after kill (28): adds v points
/// of mana or life to H, up to its maximum.
fn after_kill<W: EventWorld>(w: &mut W, func: i32, c: EventCall<W::Unit>) -> i32 {
    let (Some(h), Some(o)) = (c.h, c.o) else {
        return 0;
    };
    let (s, ov) = if func == 17 {
        if w.unit_type(h) != UnitType::Player {
            return 0;
        }
        (stat::MANA, OVERLAY_MANA)
    } else {
        if func == 18 && !w.combat().is_demon(o) {
            return 0;
        }
        (stat::LIFE, OVERLAY_LIFE)
    };
    let v = item_value(w, h, c.k);
    if v == 0 {
        return 0;
    }
    let max = w.stat_max(h, s);
    let m = w.stat(h, s, 0);
    if m >= max {
        return 0;
    }
    let n = m.wrapping_add(v << 8).max(0).min(max);
    w.combat().set_stat(h, s, n);
    w.combat().overlay(h, ov);
    1
}

/// Slow (19) and Clay Golem (27): slows O by v percent, capped by what O is: 50 for
/// players, champions, uniques, bosses and hirelings; 75 for super uniques; 90 otherwise
/// (Clay Golem uses 90 for any monster that is not a champion or unique).
fn slow<W: EventWorld>(w: &mut W, tb: EventTables<'_>, func: i32, c: EventCall<W::Unit>) -> i32 {
    let (Some(h), Some(o)) = (c.h, c.o) else {
        return 0;
    };
    let v = if func == 19 {
        item_value(w, h, c.k)
    } else {
        w.item_stat(h, stat::ITEM_SLOW, 0)
    };
    if v == 0 {
        return 0;
    }
    let cap = match w.unit_type(o) {
        UnitType::Player => 50,
        UnitType::Monster => {
            if w.combat().monster_flag(o, 12) {
                50
            } else if func == 27 {
                90
            } else if w.combat().is_boss(o) || w.combat().is_hireling(o) {
                50
            } else if w.combat().monster_flag(o, 2) {
                75
            } else {
                90
            }
        }
        _ => return 0,
    };
    let v = v.min(cap);
    let Some(l) = apply_state(
        w,
        tb.combat,
        StateRequest {
            source: h,
            target: o,
            skill: 0,
            level: 1,
            duration: 750,
            stat: stat::VELOCITYPERCENT,
            value: v.wrapping_neg(),
            state: state::SLOWED,
            callback: 0,
        },
    ) else {
        return 0;
    };
    w.list_set(l, stat::ATTACKRATE, v.wrapping_neg());
    w.list_set(l, stat::OTHER_ANIMRATE, v.wrapping_neg());
    w.combat().refresh_anim_rate(o);
    1
}

/// Skill on attack, kill and hit (20) and on death and level-up (30): a chance to cast a
/// skill through the item cast.
fn skill_on<W: EventWorld>(
    w: &mut W,
    tb: EventTables<'_>,
    func: i32,
    c: EventCall<W::Unit>,
    r: Option<&DamageRecord>,
) -> i32 {
    let Some(h) = c.h else {
        return 0;
    };
    let v = item_value(w, h, c.k);
    if v <= 0 {
        return 0;
    }
    if func == 20 && r.is_some_and(|r| r.hit_flags & 0x20 == 0) {
        return 0;
    }
    let (s, l) = split(w, c.k);
    if (w.seed(h).step() % 100) as i32 >= v {
        return 0;
    }
    let Some(rs) = rec(tb.skills, s) else {
        return 0;
    };
    let tgt_do = rs.itemtgtdo;
    if func == 20 && tgt_do {
        // The skill's `ItemTgtDo` makes O cast on itself.
        if let Some(o) = c.o {
            cast(w, tb, Some(o), s, l, Some(o), false);
        }
        return 1;
    }
    let aim = func == 20;
    match c.o {
        Some(o) => {
            cast(w, tb, Some(h), s, l, Some(o), aim);
        }
        None => {
            let at = w.path_point(h);
            cast_point(w, tb, Some(h), s, l, at, aim);
        }
    }
    1
}

/// Skill on get-hit (21): a chance to cast a skill, only when the record carries a
/// get-hit result.
fn skill_on_get_hit<W: EventWorld>(
    w: &mut W,
    tb: EventTables<'_>,
    c: EventCall<W::Unit>,
    r: Option<&DamageRecord>,
) -> i32 {
    let (Some(h), Some(r)) = (c.h, r) else {
        return 0;
    };
    let v = item_value(w, h, c.k);
    if v <= 0 || r.result & 4 == 0 {
        return 0;
    }
    let (s, l) = split(w, c.k);
    if (w.seed(h).roll(100) as i32) >= v || rec(tb.skills, s).is_none() {
        return 0;
    }
    match c.o {
        Some(o) => {
            cast(w, tb, Some(h), s, l, Some(o), false);
        }
        None => {
            let at = w.path_point(h);
            cast_point(w, tb, Some(h), s, l, at, false);
        }
    }
    1
}

/// Absorbs as much of `field` as the pool `a` allows, reducing both.
fn absorb_field(a: &mut i32, field: &mut i32) {
    if *a >= *field {
        *a -= *field;
        *field = 0;
    } else {
        *field -= *a;
        *a = 0;
    }
}

/// Bone Armor (22) and Cyclone Armor (25): the armor's pool absorbs physical damage (Bone
/// Armor) or fire, cold and lightning damage (Cyclone Armor). The state ends when the pool
/// is spent.
fn absorb<W: EventWorld>(
    w: &mut W,
    tb: EventTables<'_>,
    func: i32,
    c: EventCall<W::Unit>,
    r: Option<&mut DamageRecord>,
) -> i32 {
    let (Some(h), Some(r)) = (c.h, r) else {
        return 0;
    };
    let Some(rs) = rec(tb.skills, c.k) else {
        return 0;
    };
    let (st, a1, a2) = (s16(rs.aurastate), s16(rs.aurastat1), s16(rs.aurastat2));
    if !state_ok(w, st) {
        return 0;
    }
    let Some(list) = w.state_list(h, st) else {
        return 0;
    };
    let cmax = if stat_ok(tb.skills, a2) {
        w.list_get(list, a2)
    } else {
        0
    };
    if cmax <= 0 || !stat_ok(tb.skills, a1) {
        return 0;
    }
    let mut a = w.list_get(list, a1);
    if func == 22 {
        if a > 0 {
            absorb_field(&mut a, &mut r.physical);
            w.list_set(list, a1, a);
        }
    } else if a != 0 {
        for f in [&mut r.fire, &mut r.cold, &mut r.lightning] {
            if *f > 0 {
                absorb_field(&mut a, f);
            }
        }
        w.list_set(list, a1, a);
    }
    let gone = if func == 22 { a == 0 } else { a <= 0 };
    if gone {
        w.state_on(h, st, false);
        w.detach_free(h, list);
        return 1;
    }
    let q = pct(a, 100, cmax);
    let old = w.list_get(list, stat::UNSENTPARAM1);
    let d = q.wrapping_sub(old);
    // Cyclone Armor compares the signed difference; Bone Armor uses its absolute value.
    let mark = if func == 22 { d.abs() >= 5 } else { d >= 5 };
    if mark {
        w.mark_state_changed(h, st);
        w.list_set(list, stat::UNSENTPARAM1, q);
    }
    1
}

/// The golem drain percent: `DM(L, Param1, Param2)` of skill k for a valid k and L > 0,
/// else 0.
pub fn golem_drain_pct(t: &SkillTables, l: i32, k: i32) -> i32 {
    if rec(t, k).is_none() || l <= 0 {
        return 0;
    }
    dm(l, param(t, k, 1), param(t, k, 2))
}

/// Blood Golem, damage done (23): heals the golem and its owner by a percent of the
/// physical damage dealt.
fn blood_golem_done<W: EventWorld>(
    w: &mut W,
    tb: EventTables<'_>,
    c: EventCall<W::Unit>,
    r: Option<&DamageRecord>,
) -> i32 {
    let (Some(h), Some(o), Some(r)) = (c.h, c.o, r) else {
        return 0;
    };
    if !player_or_monster(w, o) {
        return 0;
    }
    let l = skill_level(c.l);
    let mut d = 100;
    if w.unit_type(o) == UnitType::Monster {
        d = drain(w, tb.combat, o);
        if d <= 0 {
            return 0;
        }
    }
    if w.combat().alignment(o) != 0 {
        return 0;
    }
    let mut x = r.physical;
    if d != 100 {
        x = pct(x, d, 100);
    }
    x = x.min(w.stat(o, stat::LIFE, 0));
    let q = w.minion_owner(h);
    if q.is_some_and(|q| w.combat().is_dead(q)) || w.combat().is_dead(h) {
        return 0;
    }
    let f = golem_drain_pct(tb.skills, l, c.k);
    if f <= 0 {
        return 0;
    }
    x = pct(x, f, 100);
    if x <= 0 {
        return 0;
    }
    let g = param(tb.skills, c.k, 3);
    share_heal(w, h, q, x, g);
    w.combat().overlay(h, OVERLAY_LIFE);
    if let Some(q) = q {
        if g > 0 {
            w.combat().overlay(q, OVERLAY_LIFE);
        }
    }
    1
}

/// Energy Shield (24): damage is absorbed through the holder's mana.
fn energy_shield<W: EventWorld>(
    w: &mut W,
    tb: EventTables<'_>,
    c: EventCall<W::Unit>,
    r: Option<&mut DamageRecord>,
) -> i32 {
    let (Some(h), Some(r)) = (c.h, r) else {
        return 0;
    };
    let Some(rs) = rec(tb.skills, c.k) else {
        return 0;
    };
    let (st, calc1, calc2, prg) = (s16(rs.aurastate), rs.calc1, rs.calc2, s16(rs.prgoverlay));
    let l = skill_level(c.l);
    if !state_ok(w, st) {
        return 0;
    }
    let Some(list) = w.state_list(h, st) else {
        return 0;
    };
    let p = eval_on(w, tb.skills, h, calc1, c.k, l);
    if p <= 0 {
        return 0;
    }
    let mut m = w.stat(h, stat::MANA, 0);
    let q = eval_on(w, tb.skills, h, calc2, c.k, l).max(1);
    let all = match c.o {
        None => true,
        Some(o) => w.unit_type(o) == UnitType::Monster && !w.combat().is_hireling(o),
    };
    // The record fields in absorb order: (field, leech-only).
    let fields: [(&mut i32, bool); 8] = [
        (&mut r.physical, false),
        (&mut r.fire, false),
        (&mut r.lightning, false),
        (&mut r.cold, false),
        (&mut r.magic, false),
        (&mut r.life_leech, true),
        (&mut r.mana_leech, true),
        (&mut r.stamina_leech, true),
    ];
    let mut absorbed = 0i32;
    for (f, leech) in fields {
        if leech && !all {
            continue;
        }
        let v = *f;
        if v <= 0 {
            continue;
        }
        let (x, left) = shield_row(v, p, m, q);
        absorbed = absorbed.wrapping_add(x);
        *f = v.wrapping_sub(x);
        m = left;
    }
    w.combat().set_stat(h, stat::MANA, m);
    if let Some(o) = c.o {
        // An overlay index equal to the overlay count is accepted.
        if absorbed > 0 && prg >= 0 && prg <= w.overlay_count() {
            let at = w.position(o);
            let d = dir8(w.dir64(h, at));
            w.combat().overlay(h, prg.wrapping_add(d));
        }
    }
    if m <= 0 {
        w.detach_free(h, list);
    }
    1
}

/// One Energy Shield row: the absorbed amount x = min(pct(w, p, 100), pct(m, 16, q)) and
/// the mana left, max(m − pct(x, q, 16), 0).
pub fn shield_row(w: i32, p: i32, m: i32, q: i32) -> (i32, i32) {
    let x = pct(w, p, 100).min(pct(m, 16, q));
    (x, m.wrapping_sub(pct(x, q, 16)).max(0))
}

/// Blood Golem, damage taken (26): the owner takes a percent of the golem's damage, and
/// the golem's total is reduced by that amount.
fn blood_golem_taken<W: EventWorld>(
    w: &mut W,
    tb: EventTables<'_>,
    c: EventCall<W::Unit>,
    r: Option<&mut DamageRecord>,
) -> i32 {
    let (Some(h), Some(r)) = (c.h, r) else {
        return 0;
    };
    if w.unit_type(h) != UnitType::Monster || r.total <= 0 {
        return 0;
    }
    let Some(q) = w.minion_owner(h) else {
        return 0;
    };
    if w.combat().is_dead(q) {
        return 0;
    }
    let life = w.stat(q, stat::LIFE, 0);
    if life < 256 {
        return 0;
    }
    let skill1 = tb
        .combat
        .monstats(w.class_id(h))
        .map_or(-1, |m| s16(m.skill1));
    if skill1 >= 0 {
        let p = param(tb.skills, skill1, 5);
        let x = pct(r.total, p, 100);
        w.combat()
            .set_stat(q, stat::LIFE, life.wrapping_sub(x).max(256));
        r.total = r.total.wrapping_sub(x).max(0);
    }
    1
}

/// Reanimate (31): on a successful chance, registers a run-once handler on O that raises
/// it when its death animation ends.
fn reanimate<W: EventWorld>(w: &mut W, c: EventCall<W::Unit>) -> i32 {
    let (Some(h), Some(o)) = (c.h, c.o) else {
        return 0;
    };
    if w.unit_type(o) != UnitType::Monster || w.combat().monster_flag(o, 12) {
        return 0;
    }
    let v = item_value(w, h, c.k);
    if v <= 0 {
        return 0;
    }
    if (w.seed(h).roll(100) as i32) >= v {
        return 0;
    }
    // P is H, or the first player up H's owner chain.
    let mut p = Some(h);
    let mut guard = 0;
    while let Some(u) = p {
        if w.unit_type(u) == UnitType::Player {
            break;
        }
        p = w.minion_owner(u).filter(|&n| n != u);
        guard += 1;
        if guard > 64 {
            p = None;
        }
    }
    let Some(p) = p else {
        return 0;
    };
    let g = w.guid(p) as i32;
    let class = (c.k as u32 & 0xFFFF) as i32;
    w.add_handler(
        o,
        Handler {
            event: EV_DEATH_END,
            key_type: 0,
            key: g,
            skill: class,
            level: g,
            func: RAISE_FUNC,
        },
    );
    1
}

/// Reanimate's raise of the dead unit `v` as a monster of class `c` owned by the player with
/// GUID `g`.
fn raise<W: EventWorld>(w: &mut W, tb: EventTables<'_>, v: W::Unit, c: i32, g: i32) -> i32 {
    if w.unit_type(v) != UnitType::Monster {
        return 0;
    }
    let Some(p) = w.find_unit(0, g as u32) else {
        return 0;
    };
    if !w.raise_test(v) || tb.combat.monstats(c).is_none() {
        return 0;
    }
    let at = w.position(v);
    let Some(room) = w.unit_room(v).and_then(|r| w.room_at(r, at.0, at.1)) else {
        return 0;
    };
    let Some(m2) = tb.combat.monstats2(c) else {
        return 0;
    };
    let resurrect = m2.resurrectmode;
    w.clear_pattern(v);
    let Some(n) = w.spawn_monster(crate::skills::use_::bodies::MonsterSpawn::At {
        room,
        x: at.0,
        y: at.1,
        class: c,
        mode: 1,
        spread: -1,
        flags: 0x4A,
    }) else {
        return 0;
    };
    let f = w.unit_flags(n);
    w.set_unit_flags(n, f | 0x0402_000E);
    w.raise_step(n, RaiseStep::AiReinstall);
    w.raise_step(n, RaiseStep::ThinkRestart);
    w.raise_step(n, RaiseStep::OwnerData(p));
    w.raise_step(n, RaiseStep::Leash(p));
    let frame = UseWorld::frame(w);
    w.combat().cancel_timers(n, 2);
    // The raised monster thinks again 25 base ticks (1 s) later.
    let rate = UseWorld::tick_rate(w);
    w.combat()
        .schedule_timer(n, 2, frame.wrapping_add(rate.ticks(25)));
    w.raise_step(n, RaiseStep::AlignmentGood);
    let f = w.unit_flags(n);
    w.set_unit_flags(n, f | 0x8000_0000);
    w.state_on(n, state::REVIVE, true);
    w.raise_step(n, RaiseStep::Umod21);
    // The revive state is removed 1500 base ticks (60 s) later.
    w.combat()
        .schedule_timer(n, 7, frame.wrapping_add(rate.ticks(1500)));
    w.raise_step(n, RaiseStep::NodeInsert(p));
    w.path_op(n, PathOp::Reset);
    if resurrect == 14 {
        w.raise_step(n, RaiseStep::Resurrect(at.0, at.1));
    }
    w.state_on(n, state::CORPSE_NOSELECT, true);
    w.state_on(v, state::CORPSE_NODRAW, true);
    1
}

// ---------------------------------------------------------------- item cast

/// The item-cast core's outputs: chosen unit (type, GUID) and point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CastOut {
    /// The unit the core chose, or `NO_UNIT`.
    pub unit: (u32, u32),
    /// The point the core chose, (0, 0) when none.
    pub at: (i32, i32),
}

/// "No unit chosen": type 6, GUID −1.
pub const NO_UNIT: (u32, u32) = (6, u32::MAX);

/// Item cast of skill `s` at level `l` by U at the target unit T: runs the core and queues
/// a unit message for the client. Returns 1 when it cast.
pub fn cast<W: EventWorld>(
    w: &mut W,
    tb: EventTables<'_>,
    u: Option<W::Unit>,
    s: i32,
    l: i32,
    t: Option<W::Unit>,
    aim: bool,
) -> i32 {
    let (Some(u), Some(t)) = (u, t) else {
        return 0;
    };
    if w.has_state(u, state::UNINTERRUPTABLE) {
        return 0;
    }
    let (res, out) = core(w, tb, u, s, l, Some(t), (0, 0), aim);
    if res == 0 {
        return 0;
    }
    let target = if out.unit == NO_UNIT {
        ident(w, t)
    } else {
        out.unit
    };
    w.queue_item_cast(
        u,
        ItemCastMsg::Unit {
            skill: s,
            level: l,
            target,
            aim,
        },
    );
    1
}

/// Item cast of skill `s` at level `l` by U at a point: runs the core and queues a point
/// message for the client. Returns 1 when it cast.
pub fn cast_point<W: EventWorld>(
    w: &mut W,
    tb: EventTables<'_>,
    u: Option<W::Unit>,
    s: i32,
    l: i32,
    at: (i32, i32),
    aim: bool,
) -> i32 {
    let Some(u) = u else {
        return 0;
    };
    if w.has_state(u, state::UNINTERRUPTABLE) {
        return 0;
    }
    let (res, out) = core(w, tb, u, s, l, None, at, aim);
    if res == 0 {
        return 0;
    }
    let p = if out.at.0 == 0 { at } else { out.at };
    w.queue_item_cast(
        u,
        ItemCastMsg::Point {
            skill: s,
            level: l,
            at: p,
            aim,
        },
    );
    1
}

/// A unit's (type, GUID) pair.
/// A unit's (type, GUID) pair.
fn ident<W: EventWorld>(w: &W, u: W::Unit) -> (u32, u32) {
    (w.unit_type(u) as u32, w.guid(u))
}

/// The item-cast core: checks the skill's `ItemEffect`, aims U at the target (or lets the
/// skill's `ItemTarget` kind pick one), runs the start and do cores, and restores U's
/// target. Returns the result and the outputs.
#[allow(clippy::too_many_arguments)]
pub fn core<W: EventWorld>(
    w: &mut W,
    tb: EventTables<'_>,
    u: W::Unit,
    s: i32,
    l: i32,
    t: Option<W::Unit>,
    at: (i32, i32),
    aim: bool,
) -> (i32, CastOut) {
    let mut out = CastOut {
        unit: NO_UNIT,
        at: (0, 0),
    };
    // The skill must exist and have an item effect.
    let Some(rs) = rec(tb.skills, s).filter(|_| s != 0) else {
        return (0, out);
    };
    if rs.itemeffect == 0 {
        return (0, out);
    }
    let (kind, check_start) = (rs.itemtarget, rs.itemcheckstart);
    // Save U's target, target point and unit flag 0x40.
    let saved_unit = UseWorld::target(w, u);
    let saved_point = w.path_target_point(u);
    let saved_40 = w.unit_flags(u) & 0x40;
    // Aim at the given target unit or point.
    match t {
        Some(t) => {
            w.path_op(u, PathOp::TargetUnit(Some(t)));
        }
        None => {
            w.path_op(u, PathOp::TargetPoint(at.0, at.1));
        }
    }
    // Item target kinds: 1 self, 2 a random free point near U, 3 a corpse near the target,
    // 4 U's killer when U has no target.
    let mut ok = true;
    match kind {
        1 => {
            w.path_op(u, PathOp::TargetUnit(Some(u)));
            out.unit = ident(w, u);
        }
        2 => {
            let (ux, uy) = w.position(u);
            let mut found = None;
            for _ in 0..10 {
                let x = ux
                    .wrapping_add((w.seed(u).step() % 40) as i32)
                    .wrapping_sub(20);
                let y = uy
                    .wrapping_add((w.seed(u).step() % 40) as i32)
                    .wrapping_sub(20);
                if w.point_free(u, (x, y)) {
                    found = Some((x, y));
                    break;
                }
            }
            match found {
                Some(p) => {
                    w.path_op(u, PathOp::TargetPoint(p.0, p.1));
                    out.at = p;
                }
                None => ok = false,
            }
        }
        3 => {
            let t0 = UseWorld::target(w, u);
            let c = match t0 {
                Some(t0) if l > 0 => w.corpse_near(t0),
                _ => None,
            };
            match c {
                Some(c) => {
                    w.path_op(u, PathOp::TargetUnit(Some(c)));
                    out.unit = ident(w, c);
                }
                None => ok = false,
            }
        }
        4 if UseWorld::target(w, u).is_none() => {
            let k = w.killer_of(u);
            w.path_op(u, PathOp::TargetUnit(k));
            out.unit = k.map_or(NO_UNIT, |k| ident(w, k));
        }
        _ => {}
    }
    // The start core (when `ItemCheckStart` is set) runs only after a target was found; the
    // do core runs after it.
    let started = ok && (!check_start || start_core_no_mana(w, tb.skills, u, s, l) != 0);
    let res = if !started {
        0
    } else {
        do_core(w, tb.skills, u, s, l, false, true, aim)
    };
    // Restore U's saved target and flag 0x40.
    match saved_unit {
        Some(t) => {
            w.path_op(u, PathOp::TargetUnit(Some(t)));
        }
        None => {
            w.path_op(u, PathOp::TargetPoint(saved_point.0, saved_point.1));
        }
    }
    let f = w.unit_flags(u);
    w.set_unit_flags(u, (f & !0x40) | saved_40);
    (res, out)
}

// ---------------------------------------------------------- iteration

/// Runs the unit's handlers for `event` in list order: each record of `event` runs once;
/// key-type-0 records are removed after their run. Returns the last matching result, 0
/// when none matched.
///
/// The next record is read **after** each call: a record unlinked by an earlier call in the
/// same run is not reached, and a record added during a call (registration prepends) lies
/// behind the walk. The host's list keeps no record identity, so after each call the walk
/// continues with [`surviving_tail`] of the live list. A record unregistered while running
/// (flags bit 2) is unlinked by the host at once, which is the same as freeing it after
/// the call.
pub fn run<W: EventWorld>(
    w: &mut W,
    tb: EventTables<'_>,
    event: u8,
    unit: Option<W::Unit>,
    other: Option<W::Unit>,
    mut r: Option<&mut DamageRecord>,
) -> i32 {
    let Some(u) = unit else {
        return 0;
    };
    let mut last = 0;
    let mut tail = w.handlers_of(u);
    while !tail.is_empty() {
        let h = tail.remove(0);
        if h.event != event {
            continue;
        }
        last = call(
            w,
            tb,
            h.func,
            EventCall {
                event: i32::from(event),
                h: Some(u),
                o: other,
                k: h.skill,
                l: h.level,
            },
            r.as_deref_mut(),
        );
        // The next record is read after the call.
        let live = w.handlers_of(u);
        let k = surviving_tail(&live, &tail);
        let head = &live[..live.len() - k];
        tail = live[live.len() - k..].to_vec();
        // Key type 0: unlinked after its run (when still linked: a record
        // unregistered during its own call is already gone).
        if h.key_type == 0 && head.contains(&h) {
            w.remove_handler(u, &h);
        }
    }
    last
}

/// The length of the records still ahead of the walk after a call: the
/// longest suffix of the live list (at most `tail.len()` records) that is
/// a subsequence of `tail`, the records that were ahead before the call.
/// Calls only unlink records behind the cursor and prepend new ones, so the records ahead
/// form that suffix.
pub fn surviving_tail(live: &[Handler], tail: &[Handler]) -> usize {
    let is_subsequence = |sub: &[Handler]| {
        let mut it = tail.iter();
        sub.iter().all(|x| it.any(|y| y == x))
    };
    (0..=live.len().min(tail.len()))
        .rev()
        .find(|&k| is_subsequence(&live[live.len() - k..]))
        .unwrap_or(0)
}

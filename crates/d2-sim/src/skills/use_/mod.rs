//! The skill use pipeline: the skill messages, use at a point / on a
//! unit, the mode change gates, start and do, the
//! shared cooldown, periodic skills and auras and the two
//! function tables ([`table`]).
//!
//! Status: implemented, not yet checked against a playthrough.
//!
//! Everything other specs own goes through seams: unit state, modes,
//! timers, rooms and messages through [`UseWorld`] (units/stats, tick,
//! `d2-server`); missile creation through [`UseMissiles`] (missiles, wired
//! by the action wiring still in progress on `claude/wire-action`); the
//! per-skill start / do bodies through [`SkillFunctions`]: a provider
//! runs the bodies of `functions.tsv` status `spec'd-here` from
//! [`bodies`] (over [`bodies::BodyWorld`]) and the
//! rest (status `mapped`) from its own seam; the
//! bodies that read nothing ([`bodies::start`]) run in the start core.
//! Mana formulas come from [`super::levels`].

pub mod bodies;
pub mod item_type;
pub mod table;

#[cfg(test)]
pub(crate) mod tests;

use super::{consume_mana, eval_skill, skill_level, ManaUnits, SkillEntry, SkillTables};
use crate::combat::RoomKind;
use crate::stats::stat;
use crate::tick::events::event;
use crate::units::UnitType;
use d2_data::tables::Skills;

/// Player modes (`plrmode.txt` rows) the pipeline names.
pub mod mode {
    pub const DT: u32 = 0;
    pub const NU: u32 = 1;
    pub const WL: u32 = 2;
    pub const RN: u32 = 3;
    pub const GH: u32 = 4;
    pub const TN: u32 = 5;
    pub const TW: u32 = 6;
    pub const A1: u32 = 7;
    pub const A2: u32 = 8;
    pub const BL: u32 = 9;
    pub const SC: u32 = 10;
    pub const TH: u32 = 11;
    pub const KK: u32 = 12;
    pub const S1: u32 = 13;
    pub const S2: u32 = 14;
    pub const S3: u32 = 15;
    pub const S4: u32 = 16;
    pub const DD: u32 = 17;
    pub const SQ: u32 = 18;
    pub const KB: u32 = 19;
}

/// Stat 328 `pierce_idx`.
pub const PIERCE_IDX: u16 = 328;
/// Stat 164 in state 42's list (step 4 of the interrupt gate).
pub const CONCENTRATION_STAT: u16 = 164;
/// Stats 350 / 351: skill and level in an aura state's list.
pub const AURA_SKILL: u16 = 350;
pub const AURA_LEVEL: u16 = 351;

/// States the pipeline reads.
pub mod state {
    /// Concentration (state 42, the interrupt roll).
    pub const CONCENTRATION: u16 = 42;
    /// Concentrate (state 15).
    pub const CONCENTRATE: u16 = 15;
    /// Uninterruptable (state 54).
    pub const UNINTERRUPTABLE: u16 = 54;
    /// `skilldelay` (state 121, the shared cooldown).
    pub const SKILL_DELAY: u16 = 121;
    /// `attached` (state 143, checked in the use gate).
    pub const ATTACHED: u16 = 143;
}

/// Unit flag 0x40 (+0xC4): later code-1/2 frame events of this
/// animation skip the do.
pub const FLAG_MISSILE_FIRED: u32 = 0x40;
/// Unit flag 2 (+0xC4): a valid target.
pub const FLAG_TARGETABLE: u32 = 2;
/// Used-skill flags bit 0: moving skills.
pub const SKILL_MOVING: u32 = 1;
/// Used-skill flags bit 1: the path finished.
pub const SKILL_ARRIVED: u32 = 2;

/// `use_state` codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseState {
    Usable = 0,
    NoMana = 1,
    NoQuantity = 2,
    Disabled = 3,
    Shape = 4,
    Passive = 5,
    Aura = 6,
    NoLevel = 7,
    Cooldown = 8,
}

/// Server messages the pipeline sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerMsg {
    /// 0x15 resync.
    Resync,
    /// 0x5A "can't do that" (bytes `5A 0E 01 …`).
    // TODO: the rest of the 0x5A layout; the provider builds it.
    CantDo,
}

/// What a mode start aims at (a point or a unit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeTarget<U> {
    Point(i32, i32),
    Unit(U),
}

/// The missile position of a start (its step 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissileAim {
    /// Position 0.
    None,
    /// `item` and `aim`: offset = target − unit, aim = 2 × target − unit.
    At { offset: (i32, i32), aim: (i32, i32) },
}

/// The per-skill bodies of the two function tables (a seam). A provider
/// answers the slots of [`bodies::START_BODIES`] / [`bodies::DO_BODIES`]
/// with [`bodies::run_start`] / [`bodies::run_do`]; the other slots'
/// bodies are not specified (`functions.tsv` status `mapped`, Open
/// question 10). Only filled slots ([`table::lookup`]) are called.
pub trait SkillFunctions: ManaUnits {
    /// `srvst[index](game, unit, skill, level)`; 0 refuses.
    fn srvst(&mut self, index: u16, u: Self::Unit, skill: i32, lvl: i32) -> i32;
    /// `srvdo[index](game, unit, skill, level, …)` with the core's
    /// arguments; 0 = nothing happened.
    #[allow(clippy::too_many_arguments)]
    fn srvdo(
        &mut self,
        index: u16,
        u: Self::Unit,
        skill: i32,
        lvl: i32,
        charge: bool,
        item: bool,
        aim: bool,
    ) -> i32;
    /// A mod game fills slot `index` of the `kind` table
    /// ([`crate::rules::Ruleset::has_skill_slot`],
    /// MS1): the cores call it even where the
    /// table has no function, and past the table's end. D2RUST's own
    /// rules fill none.
    fn mod_slot(&self, kind: table::Kind, index: u16) -> bool {
        let _ = (kind, index);
        false
    }
}

/// Skill missile creation (`lob` → the lobbed form, else;
/// monsters branch). A narrow seam until the missile action wiring
/// (`claude/wire-action`) provides it from `crate::missiles`.
pub trait UseMissiles: ManaUnits {
    fn create_skill_missile(
        &mut self,
        u: Self::Unit,
        skill: i32,
        lvl: i32,
        missile: u16,
        lob: bool,
        aim: MissileAim,
    );
}

/// The unit, mode, timer, room and message effects the pipeline needs (a
/// seam). Expected providers: units and stat lists, tick (timers),
/// `d2-server` (messages), items
/// (inventory, equippable).
pub trait UseWorld: UseMissiles + SkillFunctions {
    /// Diagnostic notification after a successful skill start.
    fn record_cast(&mut self, _skill: i32) {}
    // ---- game, messages
    /// Game frame (game +0xA8).
    fn frame(&self) -> i32;
    /// The game's tick rate: durations stated in base ticks are
    /// converted through it.
    /// No default, so no world runs at the base rate by accident.
    fn tick_rate(&self) -> crate::tick::TickRate;
    /// Sends a server message to the unit's client.
    fn send(&mut self, u: Self::Unit, msg: ServerMsg);

    // ---- player data, position, lookups
    /// The unit has player data.
    fn has_player_data(&self, u: Self::Unit) -> bool;
    /// Player data +0x168: frame of the last valid point message.
    fn last_point_frame(&self, u: Self::Unit) -> i32;
    fn set_last_point_frame(&mut self, u: Self::Unit, frame: i32);
    /// Unit position (path x, y).
    fn position(&self, u: Self::Unit) -> (i32, i32);
    /// Unit of `ty` with `guid` in the game.
    fn find_unit(&self, ty: u32, guid: u32) -> Option<Self::Unit>;
    /// The item is in `u`'s own inventory.
    fn in_own_inventory(&self, u: Self::Unit, item: Self::Unit) -> bool;
    /// Both units are in the same act.
    fn same_act(&self, a: Self::Unit, b: Self::Unit) -> bool;
    /// Distance test (50).
    fn within_reach(&self, a: Self::Unit, b: Self::Unit) -> bool;
    /// The unit's owner (pets, attached units).
    fn owner(&self, u: Self::Unit) -> Option<Self::Unit>;

    // ---- skills of the unit
    /// Left / right skill.
    fn left_skill(&self, u: Self::Unit) -> Option<SkillEntry>;
    fn right_skill(&self, u: Self::Unit) -> Option<SkillEntry>;
    fn set_left_skill(&mut self, u: Self::Unit, e: SkillEntry);
    fn set_right_skill(&mut self, u: Self::Unit, e: SkillEntry);
    /// The unit's entry of `skill`.
    // TODO: the lookup function is not settled
    // (D2MOO `SKILLS_GetSkillById`); the provider decides.
    fn find_entry(&self, u: Self::Unit, skill: i32) -> Option<SkillEntry>;
    /// The unit's entry of `skill` with owner GUID `owner` (0x3C).
    fn find_entry_owned(&self, u: Self::Unit, skill: i32, owner: i32) -> Option<SkillEntry>;
    /// The unit owns the skill.
    fn owns_skill(&self, u: Self::Unit, skill: i32) -> bool;
    /// Set / clear the used skill.
    fn set_used_skill(&mut self, u: Self::Unit, e: Option<SkillEntry>);
    /// Used-skill flags (bit 0 moving, bit 1 arrived).
    fn used_skill_flags(&self, u: Self::Unit) -> u32;
    fn set_used_skill_flags(&mut self, u: Self::Unit, f: u32);
    /// Skill entry +8: the mode fixed at creation ([`skill_mode`]).
    fn entry_mode(&self, u: Self::Unit, e: &SkillEntry) -> u32;
    /// Param4 of the unit's Attack entry (dual wield).
    fn attack_param4(&self, u: Self::Unit) -> i32;
    fn set_attack_param4(&mut self, u: Self::Unit, v: i32);
    /// `use_state(unit, skill)` (its parts are known, not their
    /// order; [`cooldown_blocks`] is the shared cooldown's).
    // TODO: the order of the use_state checks is unspecified.
    fn use_state(&mut self, u: Self::Unit, e: &SkillEntry) -> UseState;
    /// Decrease quantity (`decquant`).
    fn dec_quantity(&mut self, u: Self::Unit, skill: i32);

    // ---- equipment (dual wield, range)
    /// The unit can dual-wield.
    fn can_dual_wield(&self, u: Self::Unit) -> bool;
    /// The item is equippable.
    fn equippable(&self, item: Self::Item) -> bool;
    /// A bow or crossbow is equipped (`range` "both").
    fn bow_equipped(&self, u: Self::Unit) -> bool;
    /// The unit's state mask test (`range` rule, mask 0x26).
    fn state_mask(&self, u: Self::Unit, mask: u32) -> bool;
    /// Melee range.
    fn in_melee_range(&self, u: Self::Unit, target: Self::Unit) -> bool;

    // ---- modes
    fn mode(&self, u: Self::Unit) -> u32;
    /// A cursor item is held.
    fn cursor_item(&self, u: Self::Unit) -> bool;
    /// Smallest positive expire frame of the unit's type-1
    /// timers, 0 if none.
    fn endanim_expire(&self, u: Self::Unit) -> i32;
    /// A plain mode set (reenter 1, no gates).
    fn set_mode(&mut self, u: Self::Unit, mode: u32);
    /// The mode's start: set mode, target, the unit's update,
    /// delete type-0/1 timers, schedule frame events, clear flag
    /// 0x40.
    fn start_mode(&mut self, u: Self::Unit, mode: u32, target: ModeTarget<Self::Unit>);
    /// Run (mode 3) to the target remembering the skill.
    fn run_to(&mut self, u: Self::Unit, target: Self::Unit, e: SkillEntry);
    /// Current target unit.
    fn target(&self, u: Self::Unit) -> Option<Self::Unit>;
    /// Drops the target.
    fn clear_target(&mut self, u: Self::Unit);
    /// Unit flags (+0xC4).
    fn unit_flags(&self, u: Self::Unit) -> u32;
    fn set_unit_flags(&mut self, u: Self::Unit, f: u32);
    /// Unit +0x38 bits 8+: the frame event's arg2.
    fn event_arg(&self, u: Self::Unit) -> i32;
    fn set_event_arg(&mut self, u: Self::Unit, a: i32);
    /// Step the path; 2 = finished.
    fn step_path(&mut self, u: Self::Unit) -> i32;
    /// The unit is alive.
    fn is_alive(&self, u: Self::Unit) -> bool;

    // ---- relations, rooms, line of sight
    fn is_hostile(&self, a: Self::Unit, b: Self::Unit) -> bool;
    fn is_pet(&self, a: Self::Unit, b: Self::Unit) -> bool;
    fn is_ally(&self, a: Self::Unit, b: Self::Unit) -> bool;
    /// The unit's room kind.
    fn room(&self, u: Self::Unit) -> RoomKind;
    /// Target position.
    fn target_position(&self, u: Self::Unit) -> Option<(i32, i32)>;
    /// Line test with a collision mask.
    fn line_clear(&self, u: Self::Unit, to: (i32, i32), mask: u32) -> bool;

    // ---- timers
    fn schedule(&mut self, u: Self::Unit, kind: u8, frame: i32, arg1: i32, arg2: i32);
    /// Deletes the unit's timers of `kind` whose arg1 is `arg1`.
    fn delete_timers(&mut self, u: Self::Unit, kind: u8, arg1: i32);

    // ---- stat lists
    /// The unit has a stat list for `state`.
    fn has_state_list(&self, u: Self::Unit, state: u16) -> bool;
    /// Allocate a list (flags 2, expire, owner unit), state 121,
    /// remove callback, attach, switch state 121 on.
    fn create_delay_list(&mut self, u: Self::Unit, expire: i32);
    fn set_state_list_expiry(&mut self, u: Self::Unit, state: u16, expire: i32);
    /// Frees the aura state's list and switches the state off (0x3C).
    fn free_aura_state(&mut self, u: Self::Unit, state: u16);
    /// Switches the aura state on with stats 350 / 351 (0x3C).
    fn set_aura_state(&mut self, u: Self::Unit, state: u16, skill: i32, lvl: i32);
}

fn rec(t: &SkillTables, skill: i32) -> Option<&Skills> {
    t.skill(skill)
}

/// i32 view of an i16 record field stored as u16.
fn s16(v: u16) -> i32 {
    i32::from(v as i16)
}

// ---------------------------------------------------------------

/// Message ids of the skill-use messages.
pub mod msg {
    pub const LEFT_POINT: u8 = 0x05;
    pub const LEFT_UNIT: u8 = 0x06;
    pub const LEFT_UNIT_SHIFT: u8 = 0x07;
    pub const LEFT_POINT_HOLD: u8 = 0x08;
    pub const LEFT_UNIT_HOLD: u8 = 0x09;
    pub const LEFT_UNIT_SHIFT_HOLD: u8 = 0x0A;
    pub const RIGHT_POINT: u8 = 0x0C;
    pub const RIGHT_UNIT: u8 = 0x0D;
    pub const RIGHT_UNIT_SHIFT: u8 = 0x0E;
    pub const RIGHT_POINT_HOLD: u8 = 0x0F;
    pub const RIGHT_UNIT_HOLD: u8 = 0x10;
    pub const RIGHT_UNIT_SHIFT_HOLD: u8 = 0x11;
    pub const SELECT_SKILL: u8 = 0x3C;
}

/// Unit validator failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetError {
    /// Size ≠ 9 (result 3).
    Size,
    /// Type ≥ 6.
    BadType,
    /// Unit not found (result 1).
    NotFound,
    /// Other act (result 2).
    OtherAct,
    /// Distance test failed.
    Far,
}

impl TargetError {
    /// The handler result; `None` where it is not known.
    // TODO: the results of a bad type and of a failed
    // distance test are not specified.
    pub fn code(self) -> Option<i32> {
        match self {
            TargetError::Size => Some(3),
            TargetError::NotFound => Some(1),
            TargetError::OtherAct => Some(2),
            TargetError::BadType | TargetError::Far => None,
        }
    }
}

/// A skill message handler's result: the handler's return value, or a
/// validator failure with no defined value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgResult {
    Code(i32),
    Unspecified(TargetError),
}

/// Point validator: `Ok((x, y))` or the result
/// 1 (far), 2 (no player data), 3 (size). On failure, a resync (0x15) is
/// sent when more than 25 frames passed since the last success.
pub fn validate_point<W: UseWorld>(w: &mut W, u: W::Unit, m: &[u8]) -> Result<(i32, i32), i32> {
    let r = if m.len() != 5 {
        Err(3)
    } else if !w.has_player_data(u) {
        Err(2)
    } else {
        let x = i32::from(u16::from_le_bytes([m[1], m[2]]));
        let y = i32::from(u16::from_le_bytes([m[3], m[4]]));
        let (ux, uy) = w.position(u);
        if (x - ux).abs() <= 50 && (y - uy).abs() <= 50 {
            Ok((x, y))
        } else {
            Err(1)
        }
    };
    let f = w.frame();
    match r {
        Ok(_) => w.set_last_point_frame(u, f),
        Err(_) => {
            // No success for over 25 base ticks (1 s): resync.
            let window = w.tick_rate().ticks(25);
            if w.has_player_data(u) && f.wrapping_sub(w.last_point_frame(u)) > window {
                // TODO: without player data there is no
                // last-success frame; D2RUST sends no resync then.
                w.send(u, ServerMsg::Resync);
            }
        }
    }
    r
}

/// Unit validator: `(type,
/// guid)` of a valid target.
pub fn validate_unit<W: UseWorld>(
    w: &W,
    u: W::Unit,
    m: &[u8],
) -> Result<(u32, u32, W::Unit), TargetError> {
    if m.len() != 9 {
        return Err(TargetError::Size);
    }
    let ty = u32::from_le_bytes([m[1], m[2], m[3], m[4]]);
    let guid = u32::from_le_bytes([m[5], m[6], m[7], m[8]]);
    if ty >= 6 {
        return Err(TargetError::BadType);
    }
    let target = w.find_unit(ty, guid).ok_or(TargetError::NotFound)?;
    if ty == UnitType::Item as u32 && w.in_own_inventory(u, target) {
        return Ok((ty, guid, target));
    }
    if !w.same_act(u, target) {
        return Err(TargetError::OtherAct);
    }
    if !w.within_reach(u, target) {
        return Err(TargetError::Far);
    }
    Ok((ty, guid, target))
}

/// The plain skill message handlers 0x05, 0x06, 0x07, 0x0C, 0x0D, 0x0E
/// (rules 1–5). Other ids are not skill-use messages (`None`).
pub fn handle_message<W: UseWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    m: &[u8],
) -> Option<MsgResult> {
    let id = *m.first()?;
    let (left, point, run) = match id {
        msg::LEFT_POINT => (true, true, false),
        msg::LEFT_UNIT => (true, false, true),
        msg::LEFT_UNIT_SHIFT => (true, false, false),
        msg::RIGHT_POINT => (false, true, false),
        msg::RIGHT_UNIT => (false, false, true),
        msg::RIGHT_UNIT_SHIFT => (false, false, false),
        msg::LEFT_POINT_HOLD
        | msg::LEFT_UNIT_HOLD
        | msg::LEFT_UNIT_SHIFT_HOLD
        | msg::RIGHT_POINT_HOLD
        | msg::RIGHT_UNIT_HOLD
        | msg::RIGHT_UNIT_SHIFT_HOLD => return Some(handle_hold(w, t, u, m)),
        _ => return None,
    };
    let target = if point {
        match validate_point(w, u, m) {
            Ok((x, y)) => Err((x, y)),
            Err(c) => return Some(MsgResult::Code(c)),
        }
    } else {
        match validate_unit(w, u, m) {
            Ok((ty, guid, _)) => Ok((ty, guid)),
            Err(e) => {
                return Some(match e.code() {
                    Some(c) => MsgResult::Code(c),
                    None => MsgResult::Unspecified(e),
                })
            }
        }
    };
    let skill = if left {
        w.left_skill(u)
    } else {
        w.right_skill(u)
    };
    let Some(skill) = skill else {
        return Some(MsgResult::Code(3));
    };
    let p = w.base_stat(u, PIERCE_IDX, 0);
    w.set_stat(u, PIERCE_IDX, p.wrapping_add(1));
    match target {
        Err((x, y)) => {
            use_at_point(w, t, u, skill.skill, x, y);
        }
        Ok((ty, guid)) => {
            use_on_unit(w, t, u, skill.skill, ty, guid, run);
        }
    }
    Some(MsgResult::Code(0))
}

/// The hold handlers 0x08–0x0A, 0x0F–0x11: no left / right
/// skill → 3; else the plain handler, result 0 whatever it returned.
pub fn handle_hold<W: UseWorld>(w: &mut W, t: &SkillTables, u: W::Unit, m: &[u8]) -> MsgResult {
    let Some(&id) = m.first() else {
        return MsgResult::Code(3);
    };
    let (left, plain) = match id {
        msg::LEFT_POINT_HOLD => (true, msg::LEFT_POINT),
        msg::LEFT_UNIT_HOLD => (true, msg::LEFT_UNIT),
        msg::LEFT_UNIT_SHIFT_HOLD => (true, msg::LEFT_UNIT_SHIFT),
        msg::RIGHT_POINT_HOLD => (false, msg::RIGHT_POINT),
        msg::RIGHT_UNIT_HOLD => (false, msg::RIGHT_UNIT),
        msg::RIGHT_UNIT_SHIFT_HOLD => (false, msg::RIGHT_UNIT_SHIFT),
        _ => return MsgResult::Code(3),
    };
    let has = if left {
        w.left_skill(u)
    } else {
        w.right_skill(u)
    };
    if has.is_none() {
        return MsgResult::Code(3);
    }
    let mut plain_msg = m.to_vec();
    plain_msg[0] = plain;
    handle_message(w, t, u, &plain_msg);
    MsgResult::Code(0)
}

// ---------------------------------------------------------------

/// Dual wield: the skill to use instead of
/// `skill`.
pub fn dual_wield<W: UseWorld>(w: &mut W, u: W::Unit, skill: i32) -> i32 {
    if skill != 0 || !w.can_dual_wield(u) {
        return skill;
    }
    let weapon = |w: &W, loc| {
        w.item_at(u, loc)
            .is_some_and(|i| w.equippable(i) && w.item_is(i, 45) && !w.item_is(i, 38))
    };
    if !(weapon(w, 4) && weapon(w, 5)) {
        return skill;
    }
    if w.attack_param4(u) != 0 {
        w.set_attack_param4(u, 0);
        0
    } else {
        w.set_attack_param4(u, 5);
        5
    }
}

/// The entry and `use_state` with the `AttackNoMana` fallback (use
/// steps 3–4, start step 1). `None`: no entry or no record (result 2).
fn entry_and_state<W: UseWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
) -> Option<(SkillEntry, UseState)> {
    let e = w.find_entry(u, skill)?;
    let r = rec(t, e.skill)?;
    let st = w.use_state(u, &e);
    if matches!(
        st,
        UseState::NoMana | UseState::NoQuantity | UseState::Shape
    ) && r.attacknomana
    {
        let e0 = w.find_entry(u, 0)?;
        rec(t, 0)?;
        let st0 = w.use_state(u, &e0);
        return Some((e0, st0));
    }
    Some((e, st))
}

/// `use_at_point(game, unit, skill, x, y)`: 0 when a
/// mode change was requested, else 2.
pub fn use_at_point<W: UseWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    x: i32,
    y: i32,
) -> i32 {
    if matches!(w.mode(u), mode::DT | mode::DD) {
        return 2;
    }
    let skill = dual_wield(w, u, skill);
    let Some((e, st)) = entry_and_state(w, t, u, skill) else {
        return 2;
    };
    let m = w.entry_mode(u, &e);
    if m == 0 {
        return 2;
    }
    match st {
        UseState::Usable => {
            set_mode_with_skill(w, t, u, e, m, ModeTarget::Point(x, y), false);
            0
        }
        UseState::NoMana => {
            w.send(u, ServerMsg::CantDo);
            2
        }
        _ => 2,
    }
}

// ---------------------------------------------------------------

/// `range(skill, unit)`: 0 none, 1 h2h, 2 rng,
/// 4 loc ("both" resolved).
pub fn range<W: UseWorld>(w: &W, r: &Skills, u: W::Unit) -> u8 {
    let mut v = r.range;
    if v == 3 {
        v = if w.bow_equipped(u) { 2 } else { 1 };
    }
    if v == 2 && w.unit_type(u) == UnitType::Player && w.state_mask(u, 0x26) {
        v = 1;
    }
    v
}

/// `use_on_unit(game, unit, skill, type, guid, run)`:
/// 0 or 2.
pub fn use_on_unit<W: UseWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    ty: u32,
    guid: u32,
    run: bool,
) -> i32 {
    let skill = dual_wield(w, u, skill);
    let Some((e, st)) = entry_and_state(w, t, u, skill) else {
        return 2;
    };
    if st != UseState::Usable {
        return 2;
    }
    let m = w.entry_mode(u, &e);
    if m == 0 {
        return 2;
    }
    let Some(r) = rec(t, e.skill) else {
        return 2;
    };
    if !r.targetitem && !matches!(ty, 0 | 1 | 3) {
        return 2;
    }
    let mut target = w.find_unit(ty, guid);
    if let Some(tg) = target {
        if w.has_state(tg, state::ATTACHED) {
            if let Some(o) = w.owner(tg) {
                target = Some(o);
            }
        }
    }
    let Some(tg) = target else {
        // A missing target changes nothing.
        return 0;
    };
    if !run {
        set_mode_with_skill(w, t, u, e, m, ModeTarget::Unit(tg), false);
        return 0;
    }
    match range(w, r, u) {
        1 if w.in_melee_range(u, tg) => {
            set_mode_with_skill(w, t, u, e, m, ModeTarget::Unit(tg), false);
        }
        1 => w.run_to(u, tg, e),
        _ => {
            set_mode_with_skill(w, t, u, e, m, ModeTarget::Unit(tg), false);
        }
    }
    0
}

// ---------------------------------------------------------------

/// `can_change_mode`.
pub fn can_change_mode<W: UseWorld>(w: &W, t: &SkillTables, u: W::Unit, new: u32) -> bool {
    if w.cursor_item(u) && !matches!(new, mode::DT | mode::DD) {
        return false;
    }
    if matches!(new, mode::DT | mode::NU | mode::TN | mode::DD) {
        return true;
    }
    // Within 5 base ticks of the animation's end.
    let early = || w.frame() <= w.endanim_expire(u).wrapping_add(w.tick_rate().ticks(5));
    match w.mode(u) {
        mode::NU | mode::WL | mode::RN | mode::TN | mode::TW | mode::S2 | mode::S4 | mode::KB => {
            true
        }
        mode::DT | mode::GH | mode::BL | mode::DD => false,
        mode::A1 | mode::A2 | mode::SC | mode::TH => early() || matches!(new, mode::GH | mode::BL),
        mode::KK => early(),
        mode::S1 => w.class_id(u) != 0,
        mode::S3 => w.class_id(u) != 5,
        mode::SQ => {
            let seq = w
                .used_skill(u)
                .and_then(|e| rec(t, e.skill))
                .map_or(0, |r| r.seqinput);
            seq > 0 || early()
        }
        // Any current mode above 18 (past the jump table) → yes.
        _ => true,
    }
}

/// `interrupt_gate`. `requested` is the skill entry
/// of the new mode. Draws `roll(100)` on the unit's seed in step 4 only.
pub fn interrupt_gate<W: UseWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    new: u32,
    requested: &SkillEntry,
) -> bool {
    let cur = w.mode(u);
    if w.has_state(u, state::UNINTERRUPTABLE) || matches!(cur, mode::DT | mode::DD) {
        return false;
    }
    if new == mode::DT {
        return true;
    }
    let Some(used) = w.used_skill(u) else {
        return true;
    };
    let Some(r) = rec(t, used.skill) else {
        return true;
    };
    let same = used.skill == requested.skill && used.owner_guid == requested.owner_guid;
    if same && matches!(r.srvdofunc, 67 | 76) {
        return true;
    }
    if !r.interrupt {
        if cur == mode::NU {
            w.set_mode(u, mode::NU);
            return true;
        }
        return matches!(
            new,
            mode::A1 | mode::A2 | mode::SC | mode::TH | mode::S1 | mode::SQ
        ) && w.frame() <= w.endanim_expire(u).wrapping_add(w.tick_rate().ticks(5));
    }
    let mut blocked = false;
    if w.has_state(u, state::CONCENTRATION) {
        // The list's stat 164, read before the draw; a missing list gives
        // 0, so the draw is still made and never blocks.
        let limit = w
            .state_stat(u, state::CONCENTRATION, CONCENTRATION_STAT)
            .unwrap_or(0);
        let roll = w.seed(u).roll(100) as i32;
        blocked = roll < limit;
    }
    if !blocked && w.has_state(u, state::CONCENTRATE) {
        blocked = true;
    }
    if !blocked {
        return true;
    }
    if cur == mode::NU {
        w.set_mode(u, mode::NU);
        true
    } else {
        false
    }
}

/// `set_mode_xy` / `set_mode_target` for a
/// player: with `reenter = false` the two gates first; then the used
/// skill, the mode start and **start** in the same tick. Returns
/// whether the mode changed.
// TODO: monsters use their own start (monsters branch).
pub fn set_mode_with_skill<W: UseWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    e: SkillEntry,
    new: u32,
    target: ModeTarget<W::Unit>,
    reenter: bool,
) -> bool {
    if !reenter && (!can_change_mode(w, t, u, new) || !interrupt_gate(w, t, u, new, &e)) {
        return false;
    }
    w.set_used_skill(u, Some(e));
    w.start_mode(u, new, target);
    start(w, t, u);
    true
}

// -----------------------------------------------------------------

/// The mode of a skill entry, fixed at creation. `from_item`: an
/// item-charge skill.
pub fn skill_mode(ty: UnitType, class: i32, r: &Skills, skill: i32, from_item: bool) -> u32 {
    let anim = u32::from(r.anim);
    if from_item {
        return if matches!(anim, mode::A1 | mode::A2 | mode::SC | mode::TH | mode::SQ) {
            anim
        } else {
            mode::SC
        };
    }
    match ty {
        // The Assassin's (class 6) Left Hand Swing.
        UnitType::Player if class == 6 && skill == 5 => mode::S4,
        UnitType::Player => anim,
        UnitType::Monster => u32::from(r.monanim),
        // Every other unit type takes `anim`.
        _ => anim,
    }
}

// -----------------------------------------------------------------

/// The type-0 events of a mode start from an animation's frame
/// codes: `(frame, arg1, arg2)` for each frame with code 1–4; arg2 is a
/// running index for codes 1, 2, 4 and 0 for 3. The ENDANIM timer
/// follows.
// The index is one counter shared by codes 1, 2 and 4, from 0.
pub fn frame_events(codes: &[u8]) -> Vec<(usize, i32, i32)> {
    let mut n = 0;
    let mut out = Vec::new();
    for (f, &c) in codes.iter().enumerate() {
        match c {
            1 | 2 | 4 => {
                out.push((f, i32::from(c), n));
                n += 1;
            }
            3 => out.push((f, 3, 0)),
            _ => {}
        }
    }
    out
}

/// The player type-0 handler of attack-type modes:
/// 1, or 2 when the unit died.
pub fn attack_frame_event<W: UseWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    arg1: i32,
    arg2: i32,
) -> i32 {
    w.set_event_arg(u, arg2);
    // Step 2: a moving skill steps its path on every event; finished →
    // the do at once. Any other result falls through to step 3, so the
    // do also runs on the way.
    let arrived = w.used_skill_flags(u) & SKILL_MOVING != 0 && w.step_path(u) == 2;
    if arrived {
        let f = w.used_skill_flags(u);
        w.set_used_skill_flags(u, f | SKILL_ARRIVED);
    }
    let run_do = arrived || (w.unit_flags(u) & FLAG_MISSILE_FIRED == 0 && matches!(arg1, 1 | 2));
    if run_do {
        if let Some(e) = w.used_skill(u) {
            let l = skill_level(w, t, Some(u), Some(&e), true);
            do_skill(w, t, u, e.skill, l);
        }
    }
    if w.is_alive(u) {
        1
    } else {
        2
    }
}

// -----------------------------------------------------------------

/// Start for the used skill; its result.
pub fn start<W: UseWorld>(w: &mut W, t: &SkillTables, u: W::Unit) -> i32 {
    let Some(e) = w.used_skill(u) else {
        // No used skill: 0, no neutral reset.
        return 0;
    };
    let Some(r) = rec(t, e.skill) else {
        return 0;
    };
    let player = w.unit_type(u) == UnitType::Player;
    if !target_checks(w, r, u) {
        if player {
            player_neutral(w, t, u);
        }
        return 0;
    }
    let l = skill_level(w, t, Some(u), Some(&e), true);
    if !e.is_native() && e.charges == 0 {
        return 0;
    }
    if let Some(tg) = w.target(u) {
        if w.is_ally(u, tg) && !r.targetally {
            return 0;
        }
    }
    if !r.intown && w.room(u) == RoomKind::Town {
        w.set_used_skill(u, None);
        if player {
            player_neutral(w, t, u);
        }
        return 0;
    }
    let res = start_core(w, t, u, &e, l);
    if player && res == 0 {
        player_neutral(w, t, u);
    }
    res
}

/// The player's neutral request of a failed start (start steps 2, 5
/// and 7): mode 5, forced. The mode check
/// ([`can_change_mode`]) runs first (a cursor item refuses
/// mode 5: the mode stays); the mode start then sets NU (1) in a
/// field room and TN (5) in a town room or with no room, after
/// cancelling the unit's type-0 and type-1 timers of any arg
/// (the type-0 and type-1 cancels), and clears the
/// used skill, in that order.
fn player_neutral<W: UseWorld>(w: &mut W, t: &SkillTables, u: W::Unit) {
    if !can_change_mode(w, t, u, mode::TN) {
        return;
    }
    let n = if w.room(u) == RoomKind::Field {
        mode::NU
    } else {
        mode::TN
    };
    w.delete_timers(u, event::MODE_CHANGE, 0);
    w.delete_timers(u, event::END_ANIM, 0);
    w.set_mode(u, n);
    w.set_used_skill(u, None);
}

/// Target checks; false = fail.
fn target_checks<W: UseWorld>(w: &mut W, r: &Skills, u: W::Unit) -> bool {
    let Some(tg) = w.target(u) else {
        return true;
    };
    if r.targetableonly && !(w.is_hostile(u, tg) || w.is_pet(u, tg) || w.is_ally(u, tg)) {
        return false;
    }
    if w.unit_flags(tg) & FLAG_TARGETABLE == 0 {
        w.clear_target(u);
    } else if w.unit_type(tg) == UnitType::Monster {
        let dead = w.mode(tg) == crate::units::modes::monster_mode::DD;
        if dead != r.targetcorpse {
            w.clear_target(u);
        }
    }
    true
}

/// Mana check.
pub fn mana_check<W: UseWorld>(w: &W, t: &SkillTables, u: W::Unit, e: &SkillEntry, l: i32) -> bool {
    if w.unit_type(u) != UnitType::Player {
        return true;
    }
    if !e.is_native() {
        return e.charges > 0;
    }
    let Some(r) = rec(t, e.skill) else {
        return false;
    };
    if r.srvdofunc == 116 && w.shapeshifted(u) {
        return true;
    }
    let c = s16(r.mana)
        .wrapping_add(l.wrapping_sub(1).max(0).wrapping_mul(s16(r.lvlmana)))
        .wrapping_shl(u32::from(r.manashift as u8));
    c <= w.stat(u, stat::MANA, 0)
}

/// Collision masks of `lineofsight` 1–5.
pub const LOS_MASKS: [u32; 5] = [4, 0x1C09, 0x180, 0x804, 0x805];

/// Start core (no-mana-check 0) called from a body: the unit's used
/// entry when it is of `skill`, else
/// its entry of `skill`; none → 0.
pub(crate) fn start_core_of<W: UseWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    l: i32,
) -> i32 {
    let e = match w.used_skill(u) {
        Some(e) if e.skill == skill => Some(e),
        _ => w.find_entry(u, skill),
    };
    match e {
        Some(e) => start_core(w, t, u, &e, l),
        None => 0,
    }
}

/// Start core of the item cast (no-mana-check 1), so the mana
/// check is skipped and no entry is needed.
pub(crate) fn start_core_no_mana<W: UseWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    l: i32,
) -> i32 {
    start_core_with(w, t, u, skill, None, l)
}

/// Start core (`noManaCheck = 0`).
fn start_core<W: UseWorld>(w: &mut W, t: &SkillTables, u: W::Unit, e: &SkillEntry, l: i32) -> i32 {
    start_core_with(w, t, u, e.skill, Some(e), l)
}

/// The start core; `mana` = the entry for the mana check (step 6.2),
/// `None` with `noManaCheck = 1`.
fn start_core_with<W: UseWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    mana: Option<&SkillEntry>,
    l: i32,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let room = w.room(u);
    if room == RoomKind::None {
        return 0;
    }
    if mana.is_some_and(|e| !mana_check(w, t, u, e, l)) {
        return 0;
    }
    if !r.intown && room == RoomKind::Town {
        return 0;
    }
    match r.lineofsight {
        0 => {}
        v @ 1..=5 => {
            // No target position: the test is skipped.
            if let Some(pos) = w.target_position(u) {
                if !w.line_clear(u, pos, LOS_MASKS[usize::from(v - 1)]) {
                    return 0;
                }
            }
        }
        _ => return 0,
    }
    let modded = w.mod_slot(table::Kind::Start, r.srvstfunc);
    if r.srvstfunc >= table::START_SLOTS && !modded {
        return 0;
    }
    if table::lookup(table::Kind::Start, r.srvstfunc).is_none() && !modded {
        w.record_cast(skill);
        return 1;
    }
    crate::cov!(Skill, skill, 0);
    let res = match bodies::start(r.srvstfunc) {
        Some(v) if !modded => v,
        _ => w.srvst(r.srvstfunc, u, skill, l),
    };
    if res != 0 {
        w.record_cast(skill);
    }
    if res != 0 && !r.usemanaondo {
        consume_mana(w, t, Some(u), skill, l);
    }
    if res != 0 && r.periodic {
        w.delete_timers(u, event::PERIODIC_SKILLS, 0);
    }
    res
}

// -----------------------------------------------------------------

/// Do wrapper: the core with `charge = 1`, then
/// `schedule_periodic(…, aura = 0)`.
pub fn do_skill<W: UseWorld>(w: &mut W, t: &SkillTables, u: W::Unit, skill: i32, l: i32) -> i32 {
    let r = do_core(w, t, u, skill, l, true, false, false);
    schedule_periodic(w, t, u, skill, l, false);
    r
}

/// Do core.
#[allow(clippy::too_many_arguments)]
pub fn do_core<W: UseWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    l: i32,
    charge: bool,
    item: bool,
    aim: bool,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    crate::cov!(Skill, skill, 1);
    let ty = w.unit_type(u);
    let living = w.is_alive(u);
    let used = w.used_skill(u);
    if living
        && matches!(ty, UnitType::Player | UnitType::Monster)
        && used.is_some()
        && w.used_skill_flags(u) & SKILL_MOVING == 0
        && !r.intown
        && matches!(w.room(u), RoomKind::None | RoomKind::Town)
    {
        w.set_used_skill(u, None);
        if ty == UnitType::Player {
            w.set_mode(u, mode::TN);
        }
        return 0;
    }
    // The used entry's, else the highest entry's, `skill_level(…, 1)`.
    if !item
        && !used.is_some_and(|e| e.skill == skill && skill_level(w, t, Some(u), Some(&e), true) > 0)
    {
        let list = w.skill_list(u);
        let e = super::highest_entry(&list, skill);
        if skill_level(w, t, Some(u), e.as_ref(), true) <= 0 {
            return 0;
        }
    }
    if r.srvdofunc >= table::DO_SLOTS && !w.mod_slot(table::Kind::Do, r.srvdofunc) {
        return 0;
    }
    if living && r.usemanaondo {
        let e = used.filter(|e| e.skill == skill).unwrap_or(SkillEntry {
            skill,
            owner_guid: -1,
            ..SkillEntry::default()
        });
        if !mana_check(w, t, u, &e, l) {
            return 0;
        }
    }
    let mut index = r.srvdofunc;
    if aim && item && s16(r.itemeffect) > 1 {
        index = r.itemeffect;
    }
    let mut res = 0;
    if table::lookup(table::Kind::Do, index).is_some() || w.mod_slot(table::Kind::Do, index) {
        res = w.srvdo(index, u, skill, l, charge, item, aim);
    }
    if r.srvmissile != 0xFFFF && t.missile(i32::from(r.srvmissile)).is_some() {
        let f = w.unit_flags(u);
        w.set_unit_flags(u, f | FLAG_MISSILE_FIRED);
        let at = match (item && aim, w.target_position(u)) {
            // A position fails when either coordinate is 0 (both
            // must be non-zero).
            (true, Some((tx, ty))) if tx != 0 && ty != 0 => {
                let (ux, uy) = w.position(u);
                MissileAim::At {
                    offset: (tx - ux, ty - uy),
                    aim: (2 * tx - ux, 2 * ty - uy),
                }
            }
            _ => MissileAim::None,
        };
        w.create_skill_missile(u, skill, l, r.srvmissile, r.lob, at);
        res = 1;
    }
    if res == 0 {
        return 0;
    }
    if charge {
        if living {
            let no_start = table::lookup(table::Kind::Start, r.srvstfunc).is_none()
                && !w.mod_slot(table::Kind::Start, r.srvstfunc);
            if no_start || r.usemanaondo {
                consume_mana(w, t, Some(u), skill, l);
            }
            if r.decquant {
                w.dec_quantity(u, skill);
            }
        }
        let d = eval_skill(w, t, Some(u), r.delay, skill, l);
        if d > 0 && ty == UnitType::Player && (w.mode(u) != mode::SQ || w.event_arg(u) == 0) {
            set_delay(w, u, d);
        }
    }
    res
}

// ---------------------------------------------------------------

/// The skill cooldown: players only, `d ≠ 0` base ticks.
pub fn set_delay<W: UseWorld>(w: &mut W, u: W::Unit, d: i32) {
    if w.unit_type(u) != UnitType::Player || d == 0 {
        return;
    }
    let e = w.frame().wrapping_add(w.tick_rate().ticks(d));
    if !w.has_state_list(u, state::SKILL_DELAY) {
        w.create_delay_list(u, e);
    }
    w.set_state_list_expiry(u, state::SKILL_DELAY, e);
    w.schedule(u, event::REMOVE_STATE, e, 0, 0);
}

/// The cooldown part of `use_state`: a player with
/// state 121 and the skill's `delay` > 0 at level `l`.
pub fn cooldown_blocks<W: UseWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    l: i32,
) -> bool {
    if w.unit_type(u) != UnitType::Player || !w.has_state(u, state::SKILL_DELAY) {
        return false;
    }
    let Some(r) = rec(t, skill) else {
        return false;
    };
    eval_skill(w, t, Some(u), r.delay, skill, l) > 0
}

// ---------------------------------------------------------------

/// The next frame a periodic skill (an aura) acts on: frames are aligned
/// to its period, `perdelay` base ticks (at least 5, 200 ms), converted
/// through the game's tick rate.
pub fn period<W: UseWorld>(w: &mut W, t: &SkillTables, u: W::Unit, skill: i32, l: i32) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let mut d = eval_skill(w, t, Some(u), r.perdelay, skill, l);
    if d <= 5 {
        d = 5;
    }
    let d = w.tick_rate().ticks(d);
    let f = w.frame();
    (f.wrapping_add(d).wrapping_sub(1) / d)
        .wrapping_mul(d)
        .wrapping_add(1)
}

/// `schedule_periodic(game, unit, skill, L, aura)`.
pub fn schedule_periodic<W: UseWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    l: i32,
    aura: bool,
) {
    let Some(r) = rec(t, skill) else {
        return;
    };
    if !r.periodic && !r.aura {
        return;
    }
    let at = period(w, t, u, skill, l);
    if aura {
        w.delete_timers(u, event::PERIODIC_SKILLS, -1);
        w.schedule(u, event::PERIODIC_SKILLS, at, -1, 0);
    } else {
        w.delete_timers(u, event::PERIODIC_SKILLS, skill);
        w.schedule(u, event::PERIODIC_SKILLS, at, skill, l);
    }
}

/// Type-8 handler. "Delete" = the timer is not
/// rescheduled (it has run).
pub fn periodic_event<W: UseWorld>(w: &mut W, t: &SkillTables, u: W::Unit, arg1: i32) {
    if arg1 == -1 {
        let Some(e) = w.right_skill(u) else {
            return;
        };
        let aura = rec(t, e.skill).is_some_and(|r| r.aura);
        if aura && w.is_alive(u) {
            let l = skill_level(w, t, Some(u), Some(&e), true);
            do_core(w, t, u, e.skill, l, true, false, false);
            schedule_periodic(w, t, u, e.skill, l, true);
        }
        return;
    }
    // TODO: arg1 = 0 (and other values < −1) is not described;
    // D2RUST treats it like any other skill id.
    let skill = arg1;
    let Some(r) = rec(t, skill) else {
        return;
    };
    let st = r.aurastate;
    if !w.has_state(u, st) || w.state_stat(u, st, AURA_SKILL) != Some(skill) {
        return;
    }
    let l = w.state_stat(u, st, AURA_LEVEL).unwrap_or(0);
    if !w.owns_skill(u, skill) {
        return;
    }
    do_core(w, t, u, skill, l, true, false, false);
    schedule_periodic(w, t, u, skill, l, false);
}

/// Type-9 handler for one item aura of stat 151: do
/// core (…, 1, 1, 0).
// TODO: how the skill and level are read from stat 151 is not
// specified; the caller passes them.
pub fn item_aura_event<W: UseWorld>(w: &mut W, t: &SkillTables, u: W::Unit, skill: i32, l: i32) {
    do_core(w, t, u, skill, l, true, true, false);
}

/// Type-5 handler: `srvdo[srvactivefunc]` of the
/// skill's aura state (`states.srvactivefunc`, passed in). The do
/// function's other arguments are not specified; D2RUST passes the core's
/// defaults (charge 1, item 0, aim 0).
pub fn active_state_event<W: UseWorld>(
    w: &mut W,
    u: W::Unit,
    srvactivefunc: u16,
    skill: i32,
    l: i32,
) -> i32 {
    if table::lookup(table::Kind::Do, srvactivefunc).is_none()
        && !w.mod_slot(table::Kind::Do, srvactivefunc)
    {
        return 0;
    }
    w.srvdo(srvactivefunc, u, skill, l, true, false, false)
}

/// 0x3C SelectSkill → `assign(…)`: 0, or 3.
pub fn select_skill<W: UseWorld>(w: &mut W, t: &SkillTables, u: W::Unit, m: &[u8]) -> i32 {
    if m.len() != 9 {
        return 3;
    }
    let v = u32::from_le_bytes([m[1], m[2], m[3], m[4]]);
    let skill = (v & 0x7FFF_FFFF) as i32;
    let left = v & 0x8000_0000 != 0;
    let owner = i32::from_le_bytes([m[5], m[6], m[7], m[8]]);
    let Some(e) = w.find_entry_owned(u, skill, owner) else {
        return 3;
    };
    let l = skill_level(w, t, Some(u), Some(&e), true);
    if l == 0 {
        return 3;
    }
    if left {
        w.set_left_skill(u, e);
        return 0;
    }
    if let Some(old) = w.right_skill(u) {
        if let Some(or) = rec(t, old.skill).filter(|r| r.aura) {
            w.free_aura_state(u, or.aurastate);
            w.delete_timers(u, event::PERIODIC_SKILLS, -1);
        }
    }
    w.set_right_skill(u, e);
    right_aura_start(w, t, u, &e, l);
    0
}

/// The new right skill's aura start of `assign`:
/// an `aura` skill runs its do core once when `immediate`, else switches
/// its aura state on; then the aura-form schedule. Also run by the save
/// load's mouse-skill selection.
pub fn right_aura_start<W: UseWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    e: &SkillEntry,
    l: i32,
) {
    if let Some(r) = rec(t, e.skill).filter(|r| r.aura) {
        if r.immediate {
            do_core(w, t, u, e.skill, l, true, false, false);
        } else {
            w.set_aura_state(u, r.aurastate, e.skill, l);
        }
        schedule_periodic(w, t, u, e.skill, l, true);
    }
}

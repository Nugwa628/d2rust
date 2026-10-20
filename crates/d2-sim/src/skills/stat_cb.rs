//! The skill handlers of the server stat callback
//! (oskill entries, class and tab bonuses, item states,
//! pet maximum, item auras, charged skills) and the skill reset,
//! over the [`StatCbWorld`] seam on top of
//! [`BodyWorld`].
//!
//! Status: implemented, unverified (no measured game covers these handlers).

use super::use_::bodies::helpers::{clear_group, eval, rec, s16};
use super::use_::bodies::BodyWorld;
use super::use_::{do_core, period};
use super::{highest_entry, skill_level, SkillEntry, SkillTables, SkillUnits};
use crate::units::UnitType;

/// Stat ids the handlers are keyed by.
pub mod stat_id {
    pub const ITEM_ADDCLASSSKILLS: u16 = 83;
    pub const ITEM_NONCLASSSKILL: u16 = 97;
    pub const STATE: u16 = 98;
    pub const ITEM_SINGLESKILL: u16 = 107;
    pub const ITEM_ELEMSKILL: u16 = 126;
    pub const ITEM_ALLSKILLS: u16 = 127;
    pub const ITEM_AURA: u16 = 151;
    pub const ITEM_ADDSKILL_TAB: u16 = 188;
    pub const ITEM_CHARGED_SKILL: u16 = 204;
}

/// Timer type 9: the item aura re-application.
const AURA_TIMER: u8 = 9;

/// What the handlers need beyond [`BodyWorld`] (a seam). Each method
/// names its the base game address.
pub trait StatCbWorld: BodyWorld {
    /// Add a skill-list entry (a new native entry, base 1).
    fn add_entry(&mut self, u: Self::Unit, skill: i32);
    /// Assign the native base
    /// level, or delete the entry (`remove`).
    fn assign_entry(&mut self, u: Self::Unit, skill: i32, level: i32, remove: bool);
    /// Assign with remove, then the list
    /// refresh and, for a player, the player refresh.
    fn remove_skill(&mut self, u: Self::Unit, skill: i32);
    /// Message 0x21 to the client of `to`.
    fn send_skill_msg(&mut self, to: Self::Unit, skill: i32, base: i32, remove: bool);
    fn set_pet_max(&mut self, u: Self::Unit, pet_type: i32, max: i32);
    /// The skill ids of pettype row `t` (built by [`pettype_skill_lists`]).
    fn pettype_skills(&self, pet_type: i32) -> Vec<i32>;
    /// The unit is a hireling.
    fn is_hireling(&self, u: Self::Unit) -> bool;
    /// The draw identity (type, class) of the unit.
    fn draw_identity(&self, u: Self::Unit) -> (UnitType, i32);
    /// Select Attack (skill 0, owner −1) on the left (EDX 1)
    /// or right (EDX 0) hand.
    fn select_attack(&mut self, u: Self::Unit, left: bool);
    /// The skill selected on the left / right hand.
    fn selected_skill(&self, u: Self::Unit, left: bool) -> Option<SkillEntry>;
    /// Set path: update the first entry of (skill, owner)
    /// (base, charges, has-charges 1); false when there is none.
    fn entry_update(&mut self, u: Self::Unit, skill: i32, owner: i32, base: i32, ch: i32) -> bool;
    /// Set path: append a new entry with this mode.
    fn entry_append(&mut self, u: Self::Unit, e: SkillEntry, mode: u32);
    /// Remove path: unlink and free the first entry of
    /// (skill, owner).
    fn entry_unlink(&mut self, u: Self::Unit, skill: i32, owner: i32);
    /// The item's own total.
    fn item_charges(&self, item: Self::Item, layer: u16) -> i32;
    /// The item's stats are attached to the unit.
    fn item_attached(&self, item: Self::Item, u: Self::Unit) -> bool;
    /// Client updates after a skill reset.
    fn client_skill_reset_updates(&mut self, u: Self::Unit);
}

/// The skill lists of the pettype rows (built at table load):
/// every `skills` row in id order whose `pettype` p is in
/// 0…count − 1 is appended to row p's list while it holds fewer than 15.
pub fn pettype_skill_lists(t: &SkillTables, count: usize) -> Vec<Vec<i32>> {
    let mut lists = vec![Vec::new(); count];
    for (id, r) in t.skills.iter().enumerate() {
        let p = usize::from(r.pettype);
        if p < count && lists[p].len() < 15 {
            lists[p].push(id as i32);
        }
    }
    lists
}

fn native<W: SkillUnits>(w: &W, u: W::Unit, skill: i32) -> Option<SkillEntry> {
    w.skill_list(u)
        .into_iter()
        .find(|e| e.skill == skill && e.is_native())
}

/// The dispatcher: the skill handler of `stat` (table). `g` = the
/// propagation unit's GUID (0 for none), `item` the propagation unit,
/// `new` the new value; `shift_mask` the charged-skill layer split (data
/// +0xC6C / +0xC70). Stats without a skill handler do nothing.
#[allow(clippy::too_many_arguments)]
pub fn skill_stat_callback<W: StatCbWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    item: Option<W::Item>,
    g: i32,
    stat: u16,
    layer: u16,
    new: i32,
    shift_mask: (u32, u32),
) {
    use stat_id::*;
    match stat {
        ITEM_NONCLASSSKILL | ITEM_SINGLESKILL => oskill(w, t, u, stat, i32::from(layer), new),
        ITEM_ADDCLASSSKILLS => class_bonus(w, t, u, i32::from(layer)),
        ITEM_ADDSKILL_TAB => class_bonus(w, t, u, i32::from(layer >> 3)),
        ITEM_ELEMSKILL | ITEM_ALLSKILLS => refresh_all(w, t, u),
        STATE => item_state(w, u, i32::from(layer), new),
        ITEM_AURA => {
            if new != 0 {
                aura_on(w, t, u, g, i32::from(layer), new);
            } else {
                aura_off(w, t, u, g, i32::from(layer));
            }
        }
        ITEM_CHARGED_SKILL => charged(w, t, u, item, g, layer, shift_mask),
        _ => {}
    }
}

// -----------------------------------------------------------------

/// Oskill entries (stats 97, 107).
pub fn oskill<W: StatCbWorld>(w: &mut W, t: &SkillTables, u: W::Unit, stat: u16, s: i32, new: i32) {
    if s == 0 {
        return;
    }
    if stat == stat_id::ITEM_SINGLESKILL {
        let class = rec(t, s).map(|r| i32::from(r.charclass));
        if w.unit_type(u) != UnitType::Player || class != Some(w.class_id(u)) {
            return;
        }
    }
    let e = match native(w, u, s) {
        Some(e) => e,
        None => {
            w.add_entry(u, s);
            w.assign_entry(u, s, 0, false);
            let to = match w.unit_type(u) {
                UnitType::Player => Some(u),
                UnitType::Monster => w.minion_owner(u),
                _ => None,
            };
            if let Some(to) = to {
                w.send_skill_msg(to, s, 0, false);
            }
            match native(w, u, s) {
                Some(e) => e,
                None => return,
            }
        }
    };
    w.passive_state_apply(u, &e);
    pet_max(w, t, u, s);
    if new > 0 {
        return;
    }
    let mut remove = true;
    if skill_level(w, t, Some(u), Some(&e), false) != 0 {
        remove = false;
        if skill_level(w, t, Some(u), Some(&e), true) != 0 {
            return;
        }
    }
    for left in [true, false] {
        if w.selected_skill(u, left)
            .is_some_and(|x| x.skill == s && x.is_native())
        {
            w.select_attack(u, left);
        }
    }
    if remove {
        w.assign_entry(u, s, 0, true);
    }
}

// -----------------------------------------------------------------

/// Class and tab bonuses (stats 83, 188).
pub fn class_bonus<W: StatCbWorld>(w: &mut W, t: &SkillTables, u: W::Unit, c: i32) {
    let player = w.unit_type(u) == UnitType::Player && w.class_id(u) == c;
    let draw = {
        let (ty, class) = w.draw_identity(u);
        ty == UnitType::Player && class == c
    };
    if player || w.is_hireling(u) || draw {
        refresh_all(w, t, u);
    }
}

/// Refresh all: the passive states of every entry back on.
pub fn refresh_all<W: StatCbWorld>(w: &mut W, t: &SkillTables, u: W::Unit) {
    for e in w.skill_list(u) {
        let p = rec(t, e.skill).map_or(-1, |r| s16(r.passivestate));
        if p > 0 {
            w.state_on(u, p, true);
            w.passive_state_apply(u, &e);
        }
    }
}

// -----------------------------------------------------------------

/// Item states (stat 98).
pub fn item_state<W: StatCbWorld>(w: &mut W, u: W::Unit, s: i32, new: i32) {
    if !(0..w.state_count()).contains(&s) {
        return;
    }
    let has = w.has_state(u, s as u16);
    if new != 0 {
        if has {
            return;
        }
        clear_group(w, u, s, false);
        w.state_on(u, s, true);
    } else {
        if !has {
            return;
        }
        clear_group(w, u, s, true);
        w.state_on(u, s, false);
    }
    w.mark_state_changed(u, s);
}

// -----------------------------------------------------------------

/// Pet maximum of one skill.
pub fn pet_max<W: StatCbWorld>(w: &mut W, t: &SkillTables, u: W::Unit, s: i32) {
    if w.unit_type(u) != UnitType::Player {
        return;
    }
    let Some(r) = rec(t, s) else {
        return;
    };
    let pt = i32::from(r.pettype as i8);
    if !(0 < pt && pt < w.pettype_count()) {
        return;
    }
    let mut m = 0;
    for k in w.pettype_skills(pt) {
        let Some(kr) = rec(t, k) else {
            continue;
        };
        let petmax = kr.petmax;
        let Some(e) = highest_entry(&w.skill_list(u), k) else {
            continue;
        };
        let l = skill_level(w, t, Some(u), Some(&e), true);
        let v = if l > 0 {
            eval(w, t, u, petmax, k, l).max(1)
        } else {
            0
        };
        m = m.max(v);
    }
    w.set_pet_max(u, pt, m);
}

// -----------------------------------------------------------------

/// The shared gate of aura on / off.
fn aura_ok<W: StatCbWorld>(w: &W, t: &SkillTables, s: i32) -> bool {
    let Some(r) = rec(t, s) else {
        return false;
    };
    r.aura && (0..w.state_count()).contains(&s16(r.aurastate))
}

/// Aura on `(game, unit, G, s, L)`.
pub fn aura_on<W: StatCbWorld>(w: &mut W, t: &SkillTables, u: W::Unit, g: i32, s: i32, l: i32) {
    if !aura_ok(w, t, s) {
        return;
    }
    w.delete_timers(u, AURA_TIMER, g);
    let at = period(w, t, u, s, l);
    w.schedule(u, AURA_TIMER, at, g, s);
    if rec(t, s).is_some_and(|r| r.immediate) {
        do_core(w, t, u, s, l, true, true, false);
    }
}

/// Aura off.
pub fn aura_off<W: StatCbWorld>(w: &mut W, t: &SkillTables, u: W::Unit, g: i32, s: i32) {
    if !aura_ok(w, t, s) {
        return;
    }
    let state = rec(t, s).map_or(0, |r| s16(r.aurastate));
    w.state_on(u, state, false);
    if let Some(l) = w.state_list(u, state) {
        w.detach_free(u, l);
    }
    w.delete_timers(u, AURA_TIMER, g);
}

// -----------------------------------------------------------------

/// The mode of a new charge entry (jump table).
fn charge_mode(anim: u8) -> u32 {
    match anim {
        7 | 8 | 10 | 11 | 18 => u32::from(anim),
        _ => 10,
    }
}

/// Item charged skills (stat 204).
pub fn charged<W: StatCbWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    item: Option<W::Item>,
    g: i32,
    layer: u16,
    (shift, mask): (u32, u32),
) {
    let Some(item) = item else {
        return;
    };
    let s = i32::from(layer) >> shift.min(31);
    let l = i32::from(layer) & mask as i32;
    let c = w.item_charges(item, layer) & 0xFF;
    if c > 0 && w.item_attached(item, u) {
        charge_entry(w, t, u, g, s, l, c, false);
    } else {
        charge_entry(w, t, u, g, s, l, c, true);
        pet_max(w, t, u, s);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn charge_entry<W: StatCbWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    g: i32,
    s: i32,
    l: i32,
    c: i32,
    remove: bool,
) {
    if w.unit_type(u) != UnitType::Player || l <= 0 {
        return;
    }
    let Some(r) = rec(t, s) else {
        return;
    };
    if !remove {
        if !w.entry_update(u, s, g, l, c) {
            let e = SkillEntry {
                skill: s,
                base: l,
                level_bonus: 0,
                owner_guid: g,
                charges: c,
                has_charges: true,
            };
            w.entry_append(u, e, charge_mode(r.anim));
        }
        return;
    }
    for left in [true, false] {
        if w.selected_skill(u, left)
            .is_some_and(|x| x.skill == s && x.owner_guid == g)
        {
            w.select_attack(u, left);
        }
    }
    if w.used_skill(u).is_some_and(|x| x.skill == s) {
        w.set_used_skill(u, None);
    }
    w.entry_unlink(u, s, g);
}

// -----------------------------------------------------------------

/// Skill reset: `class_skills` is the player's class skill
/// list.
pub fn reset_skills<W: StatCbWorld>(w: &mut W, t: &SkillTables, u: W::Unit, class_skills: &[i32]) {
    if w.unit_type(u) != UnitType::Player {
        return;
    }
    let mut sum = 0;
    for &s in class_skills {
        let Some(e) = native(w, u, s) else {
            continue;
        };
        sum += skill_level(w, t, Some(u), Some(&e), false);
        w.remove_skill(u, s);
        w.send_skill_msg(u, s, 0, true);
    }
    w.add_stat(u, 5, sum);
    w.client_skill_reset_updates(u);
}

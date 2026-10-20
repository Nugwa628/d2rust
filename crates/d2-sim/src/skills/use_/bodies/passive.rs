//! The passive-state refresh: the stat list of a
//! passive skill's state (Critical Strike, Dodge, Avoid, Evade, Penetrate,
//! the masteries ...) holds the skill's `passivestat1…5` set to
//! `eval(passivecalc_i, skill, L)`, plus the markers 350 (skill) and
//! 351 (level).
//!
//! Each stat goes on the layer `passiveitype` when it is > 0 (the weapon
//! masteries: the item type the bonus applies to), else on layer 0; the
//! two markers are on layer 0 (read; the layer is the
//! 0xA8 entry's param, measured at frame 2).

use super::helpers::{eval, rec, s16, stat_ok, state_ok};
use super::BodyWorld;
use crate::skills::levels::{highest_entry, skill_level};
use crate::skills::SkillTables;

/// Stat 350: the passive list's skill marker.
pub const PASSIVE_SKILL: i32 = 350;
/// Stat 351: the passive list's level marker.
pub const PASSIVE_LEVEL: i32 = 351;

pub fn refresh<W: BodyWorld>(w: &mut W, t: &SkillTables, u: W::Unit, skill: i32) {
    let Some(r) = rec(t, skill) else {
        return;
    };
    let p = s16(r.passivestate);
    if p <= 0 || !state_ok(w, p) {
        return;
    }
    let entry = highest_entry(&w.skill_list(u), skill);
    let aura = s16(r.aurastate);
    let level = entry
        .filter(|_| aura <= 0 || !w.has_state(u, aura as u16))
        .map_or(0, |e| skill_level(w, t, Some(u), Some(&e), true));
    refresh_at(w, t, u, skill, level);
}

/// From the point where the level is known (`level` = the
/// entry's level): the passive state's stat
/// list is built or detached. A monster's entries are not in a
/// [`BodyWorld::skill_list`], so its caller passes the base level
/// (the AI init does).
pub fn refresh_at<W: BodyWorld>(w: &mut W, t: &SkillTables, u: W::Unit, skill: i32, level: i32) {
    let Some(r) = rec(t, skill) else {
        return;
    };
    let p = s16(r.passivestate);
    if p <= 0 || !state_ok(w, p) {
        return;
    }
    let found = w.state_list(u, p);
    // The passive's state list: L = 0 detaches and frees it.
    if level <= 0 {
        if let Some(l) = found {
            w.detach_free(u, l);
        }
        return;
    }
    let l = match found {
        Some(l) => l,
        None => {
            let Some(l) = w.alloc_list(0, 0, Some(u)) else {
                return;
            };
            w.set_list_state(l, p);
            w.attach(u, l);
            l
        }
    };
    if w.list_get(l, PASSIVE_LEVEL) == level {
        return;
    }
    let pairs = [
        (r.passivestat1, r.passivecalc1),
        (r.passivestat2, r.passivecalc2),
        (r.passivestat3, r.passivecalc3),
        (r.passivestat4, r.passivecalc4),
        (r.passivestat5, r.passivecalc5),
    ];
    let layer = match s16(r.passiveitype) {
        k @ 1.. => k as u16,
        _ => 0,
    };
    for (stat, calc) in pairs {
        let s = s16(stat);
        if !stat_ok(t, s) {
            break;
        }
        let v = eval(w, t, u, calc, skill, level);
        w.list_set_layer(l, s, v, layer);
    }
    w.list_set(l, PASSIVE_SKILL, skill);
    w.list_set(l, PASSIVE_LEVEL, level);
    // At the end (the changed bit and
    // the update queue): a level change sends the state again.
    w.mark_state_changed(u, p);
    w.queue_update(u);
}

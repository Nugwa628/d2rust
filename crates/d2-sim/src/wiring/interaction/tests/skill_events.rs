//! Tests for the skill timer events through the combined dispatcher: a timer of
//! type 5, 8 or 9 on the real queue runs the unit dispatch, whose hooks hand it
//! to the skill use pipeline ([`crate::wiring::interaction::skill_events::route`]);
//! the pipeline's do core calls the skill's server-do function (the fake logs
//! it) and the periodic form reschedules itself on the same queue.

use std::sync::Arc;

use crate::skills::fake::skill_rec;
use crate::skills::{SkillEntry, SkillTables};
use crate::stats::stat as sst;
use crate::stats::StatData;
use crate::tick::events::event;
use crate::units::{UnitId, UnitType};

use super::skill_use::{skills, Fx};

/// The aura skill: `aura`, `srvdofunc` 53 (a filled slot without a body,
/// so the fake logs it; the basic aura do 65 runs its body, see
/// `skill_bodies`), no missile, no mana, no formulas.
const AURA: i32 = 1;
/// Its do function (srvdo 53; no body, so the fake logs it).
const AURA_DO: u16 = 53;
/// Its aura state.
const AURA_STATE: u16 = 40;
/// `srvactivefunc` of the aura state: srvdo 138, a filled
/// slot without a body (hurricane, 145, has one).
const ACTIVE_DO: u16 = 138;

fn aura_skills() -> SkillTables {
    let mut t = skills();
    let mut a = skill_rec();
    a.aura = true;
    a.srvdofunc = AURA_DO;
    a.srvmissile = 0xFFFF;
    a.aurastate = AURA_STATE;
    a.perdelay = 0xFFFF_FFFF;
    a.delay = 0xFFFF_FFFF;
    t.skills.push(a);
    t
}

/// The interaction stat data with the aura skill's `aurastate` and the
/// state's `srvactivefunc`.
fn aura_stats() -> Arc<StatData> {
    let mut d = (*super::stat_data()).clone();
    d.aurastate[AURA as usize] = AURA_STATE;
    d.states.set_srvactivefunc(u32::from(AURA_STATE), ACTIVE_DO);
    Arc::new(d)
}

fn fx() -> (Fx, UnitId) {
    let mut fx = Fx::with(aura_stats(), aura_skills());
    let p = fx.spawn(UnitType::Player, 10, 10);
    (fx, p)
}

fn entry(skill: i32) -> SkillEntry {
    SkillEntry {
        skill,
        base: 1,
        owner_guid: -1,
        ..SkillEntry::default()
    }
}

/// The unit's pending timers of `kind`: (expire, arg1, arg2).
fn timers(fx: &Fx, u: UnitId, kind: u8) -> Vec<(i32, u32, u32)> {
    let t = &fx.game.timers;
    let mut v: Vec<_> = t
        .unit_timers(u)
        .into_iter()
        .filter_map(|id| {
            let (k, a1, a2) = t.event(id)?;
            (k == kind).then_some((t.expire(id)?, a1, a2))
        })
        .collect();
    v.sort();
    v
}

fn srvdo_count(fx: &Fx, f: u16) -> usize {
    let want = format!("srvdo {f}");
    fx.sim
        .sys
        .hooks
        .x
        .log
        .iter()
        .filter(|l| **l == want)
        .count()
}

#[test]
fn periodic_aura_event_runs_the_do_core_and_reschedules_on_the_queue() {
    let (mut fx, p) = fx();
    fx.sim.sys.hooks.x.right.insert(p, entry(AURA));
    fx.sim.sys.hooks.x.skills.insert(p, vec![entry(AURA)]);
    // The aura form (−1, 0), due at frame 1.
    fx.game
        .schedule_event(p, u32::from(event::PERIODIC_SKILLS), 1, None, u32::MAX, 0)
        .unwrap();
    fx.frame();
    // The do core ran the aura's do function once and rescheduled the
    // aura form at `period` (perdelay ≤ 5 → 5: frames ≡ 1 mod 5).
    assert_eq!(srvdo_count(&fx, AURA_DO), 1);
    assert_eq!(timers(&fx, p, event::PERIODIC_SKILLS), [(6, u32::MAX, 0)]);
    for _ in 1..11 {
        fx.frame();
    }
    // Frames 6 and 11: two more runs, the next one at 16.
    assert_eq!(srvdo_count(&fx, AURA_DO), 3);
    assert_eq!(timers(&fx, p, event::PERIODIC_SKILLS), [(16, u32::MAX, 0)]);
    fx.assert_clean();
}

#[test]
fn periodic_aura_event_without_an_aura_right_skill_is_not_rescheduled() {
    let (mut fx, p) = fx();
    // The right skill is the missile skill (no `aura`): the event is
    // deleted (it has run, nothing reschedules it).
    fx.sim.sys.hooks.x.right.insert(p, entry(0));
    fx.game
        .schedule_event(p, u32::from(event::PERIODIC_SKILLS), 1, None, u32::MAX, 0)
        .unwrap();
    fx.frame();
    assert_eq!(srvdo_count(&fx, AURA_DO), 0);
    assert!(timers(&fx, p, event::PERIODIC_SKILLS).is_empty());
    fx.assert_clean();
}

// (0x3C SelectSkill: a new `aura` right skill schedules the aura form; the
// save load's assign does the same)
#[test]
fn the_loaded_right_aura_schedules_the_aura_form_and_a_plain_skill_does_not() {
    let (mut fx, p) = fx();
    fx.sim.sys.hooks.x.right.insert(p, entry(AURA));
    fx.sim.sys.hooks.x.skills.insert(p, vec![entry(AURA)]);
    fx.sim.assign_right_aura(&mut fx.game, p);
    // The aura form (−1, 0) at `period` (perdelay ≤ 5 → 5: frames ≡ 1 mod 5).
    assert_eq!(timers(&fx, p, event::PERIODIC_SKILLS), [(1, u32::MAX, 0)]);
    fx.assert_clean();
    let (mut plain, q) = self::fx();
    plain.sim.sys.hooks.x.right.insert(q, entry(0));
    plain.sim.assign_right_aura(&mut plain.game, q);
    assert!(timers(&plain, q, event::PERIODIC_SKILLS).is_empty());
    plain.assert_clean();
}

fn monster_right_aura(fx: &mut Fx, m: UnitId, skill: i32, level: i32) {
    let s = &mut fx.sim.sys;
    let mut sim = crate::units::hooks::Sim {
        game: &mut fx.game,
        units: &mut s.units,
        stats: &mut s.stats,
        data: &s.data,
    };
    crate::wiring::action::Pending::monster_right_aura(&mut s.hooks, &mut sim, m, skill, level);
}

// (a monster's aura: the skill given at the level, the right hand
// assigned, the aura form scheduled; a plain skill and a player do nothing)
#[test]
fn a_monster_aura_gives_the_skill_and_schedules_the_aura_form() {
    let (mut fx, _) = fx();
    let m = fx.spawn(UnitType::Monster, 12, 12);
    monster_right_aura(&mut fx, m, AURA, 3);
    assert_eq!(timers(&fx, m, event::PERIODIC_SKILLS), [(1, u32::MAX, 0)]);
    let h = &fx.sim.sys.hooks;
    assert_eq!(h.monster_skills[&m].get(&AURA), Some(&3));
    let l = &h.skill_lists[&m];
    let right = l.right.and_then(|i| l.view().get(i).copied()).unwrap();
    assert_eq!((right.skill, right.base), (AURA, 3));
    fx.assert_clean();
    let (mut plain, p) = self::fx();
    let q = plain.spawn(UnitType::Monster, 12, 12);
    monster_right_aura(&mut plain, q, 0, 3);
    monster_right_aura(&mut plain, p, AURA, 3);
    assert!(timers(&plain, q, event::PERIODIC_SKILLS).is_empty());
    assert!(timers(&plain, p, event::PERIODIC_SKILLS).is_empty());
    assert!(!plain.sim.sys.hooks.skill_lists.contains_key(&q));
    plain.assert_clean();
}

#[test]
fn item_aura_event_reads_stat_151_and_runs_the_do_core() {
    let (mut fx, p) = fx();
    // Item aura of skill 1 at level 3 (stat 151, layer = skill).
    fx.sim.with(&mut fx.game, |_, v| {
        v.stats
            .unit_set(&mut *v.h, p, sst::ITEM_AURA, 3, AURA as u16);
    });
    fx.game
        .schedule_event(p, u32::from(event::PERIODIC_STATS), 1, None, 0, AURA as u32)
        .unwrap();
    fx.frame();
    assert_eq!(srvdo_count(&fx, AURA_DO), 1);
    // No item aura (stat 151 of the skill ≤ 0): the dispatch cancels the
    // unit's type-9 events and nothing reaches the skills.
    fx.sim.with(&mut fx.game, |_, v| {
        v.stats
            .unit_set(&mut *v.h, p, sst::ITEM_AURA, 0, AURA as u16);
    });
    fx.game
        .schedule_event(p, u32::from(event::PERIODIC_STATS), 2, None, 0, AURA as u32)
        .unwrap();
    fx.game
        .schedule_event(p, u32::from(event::PERIODIC_STATS), 9, None, 0, AURA as u32)
        .unwrap();
    fx.frame();
    assert_eq!(srvdo_count(&fx, AURA_DO), 1);
    assert!(timers(&fx, p, event::PERIODIC_STATS).is_empty());
    fx.assert_clean();
}

#[test]
fn active_state_event_calls_the_aura_states_server_do_function() {
    let (mut fx, p) = fx();
    // a1 = the skill: its aura state's `srvactivefunc` (145) is the do
    // function the skill pipeline calls.
    fx.game
        .schedule_event(p, u32::from(event::ACTIVE_STATE), 1, None, AURA as u32, 4)
        .unwrap();
    // a1 = 0 is out of range: nothing.
    fx.game
        .schedule_event(p, u32::from(event::ACTIVE_STATE), 1, None, 0, 4)
        .unwrap();
    fx.frame();
    assert_eq!(srvdo_count(&fx, ACTIVE_DO), 1);
    assert_eq!(fx.sim.sys.hooks.x.log.len(), 1);
    fx.assert_clean();
}

#[test]
fn monster_attack_event0_runs_the_used_skills_do_on_every_event() {
    // A monster in attack mode 2 (A2) with a used (not moving) skill runs
    // the skill's do on each attack-family event 0, whatever the frame code
    // (`action_frame` 3, 2, and unchanged by a code-0 event): three events,
    // three server-do calls.
    let mut fx = Fx::with(aura_stats(), aura_skills());
    let m = fx.spawn(UnitType::Monster, 10, 10);
    fx.sim.sys.units.get_mut(m).unwrap().mode = 5;
    fx.sim.sys.hooks.x.used.insert(m, entry(AURA));
    fx.sim.sys.hooks.x.skills.insert(m, vec![entry(AURA)]);
    {
        // The codes come from the animation record through the frame
        // advance (frames 1, 2, 3).
        let r = fx.sim.sys.units.get_mut(m).unwrap();
        let mut events = [0u8; crate::units::record::ANIM_EVENTS];
        events[1] = 3;
        events[2] = 2;
        r.anim.record = Some(crate::units::record::AnimRecord {
            frames: 1 << 8,
            byte_0f: 0,
            events,
        });
        r.anim.speed = 256;
        r.anim.frame = 0;
        r.anim.frame_count = 1 << 16;
    }
    for (i, code) in [3u32, 2, 0].into_iter().enumerate() {
        fx.game
            .schedule_event(
                m,
                u32::from(event::MODE_CHANGE),
                1 + i as i32,
                None,
                code,
                0,
            )
            .unwrap();
    }
    for _ in 0..3 {
        fx.frame();
    }
    assert_eq!(srvdo_count(&fx, AURA_DO), 3);
    // Each advance rewrites `action_frame`: frame 3 holds no code.
    assert_eq!(fx.sim.sys.units.get(m).unwrap().anim.action_frame, 0);
    fx.assert_clean();
}

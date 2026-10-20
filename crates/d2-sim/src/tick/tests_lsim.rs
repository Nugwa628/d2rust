//! Coverage tests for the timer queue, the event run and the frame loop.

use super::tests::{id, setup};
use super::timer::{flags, needs_uninterruptable_check, TimerClass};
use super::*;
use crate::units::UnitType;

#[test]
fn state54_check_only_for_timed_monster_ai_think() {
    assert!(needs_uninterruptable_check(UnitType::Monster, 2, 40));
    assert!(!needs_uninterruptable_check(UnitType::Monster, 2, -1));
    assert!(!needs_uninterruptable_check(UnitType::Monster, 1, 40));
    assert!(!needs_uninterruptable_check(UnitType::Player, 2, 40));
}

#[test]
fn every_tick_flags_expire_and_prepend() {
    let (mut g, rec, _) = setup(&[("P", UnitType::Player)]);
    let p = id(&rec, "P");
    let a = g.schedule_event(p, 0, -1, None, 0, 0).unwrap().unwrap();
    let b = g.schedule_event(p, 0, -1, None, 0, 0).unwrap().unwrap();
    assert_eq!(g.timers.expire(a), Some(-1));
    assert_ne!(g.timers.flags(a).unwrap() & flags::EVERY_TICK, 0);
    assert_eq!(
        g.timers
            .every_tick(TimerClass::of(UnitType::Player).unwrap()),
        [b, a]
    );
    assert_eq!(g.timers.unit_timers(p), [b, a]);
    // Type >= 15 is ignored.
    assert!(g.schedule_event(p, 15, -1, None, 0, 0).unwrap().is_none());
}

#[test]
fn periodic_steps_due_frames() {
    assert_eq!(period::FREE_INACTIVE_ROOMS, 11);
    assert_eq!(period::ROOM_DEACTIVATION, 12);
    assert_eq!(period::QUESTS, 20);
    assert_eq!(period::EXPIRED_ITEMS, 1500);
    assert!(is_due(660, 11) && is_due(660, 12) && is_due(660, 20));
    assert!(!is_due(661, 11));
    assert!(is_due(1500, period::EXPIRED_ITEMS));
}

use super::tests::{sched, setup, tick_to};
use super::timer::needs_uninterruptable_check;
use super::*;
use crate::units::UnitType;

#[test]
fn classes_run_missile_player_monster_object_item() {
    // Classes never interleave; run order missile, player, monster,
    // object, item, whatever the scheduling order.
    let (mut g, mut rec, _) = setup(&[
        ("I", UnitType::Item),
        ("O", UnitType::Object),
        ("M", UnitType::Monster),
        ("P", UnitType::Player),
        ("X", UnitType::Missile),
    ]);
    for n in ["I", "O", "M", "P", "X"] {
        sched(&mut g, &rec, n, 3).unwrap();
    }
    let runs = tick_to(&mut g, &mut rec, 3);
    assert_eq!(runs, ["X", "P", "M", "O", "I"]);
}

#[test]
fn uninterruptable_precondition_only_for_timed_monster_ai() {
    assert!(needs_uninterruptable_check(UnitType::Monster, 2, 50));
    assert!(!needs_uninterruptable_check(UnitType::Monster, 2, -1));
    assert!(!needs_uninterruptable_check(UnitType::Monster, 3, 50));
    assert!(!needs_uninterruptable_check(UnitType::Player, 2, 50));
}

#[test]
fn periodic_table_periods() {
    assert_eq!(period::FREE_INACTIVE_ROOMS, 11);
    assert_eq!(period::ROOM_DEACTIVATION, 12);
    assert_eq!(period::QUESTS, 20);
    assert_eq!(period::EXPIRED_ITEMS, 1500);
    assert!(is_due(20, 20) && !is_due(21, 20));
    assert!(is_due(1500, 1500) && !is_due(1499, 1500));
}

// Spec: specs/world/quests.md (§1.8 completed quests and the save)
#![allow(unused_imports)]
use super::*;

fn completed(r: &QuestFlags, q: u8) -> bool {
    r.get(q, bit::REWARD_GRANTED) || r.get(q, bit::COMPLETED_BEFORE)
}

// Covers: specs/world/quests.md §1.8 r1
#[test]
fn completion_test_bit0_or_bit15() {
    let mut r = QuestFlags::default();
    assert!(!completed(&r, 3));
    r.set(3, bit::COMPLETED_BEFORE);
    assert!(completed(&r, 3));
    let mut r = QuestFlags::default();
    r.set(3, bit::REWARD_GRANTED);
    assert!(completed(&r, 3));
    // Gates test bit 0 alone.
    let mut r = QuestFlags::default();
    r.set(7, bit::COMPLETED_BEFORE);
    assert!(!r.get(7, bit::REWARD_GRANTED));
}

// Covers: specs/world/quests.md §1.8 r2
#[test]
fn bit1_without_bit0_load_sets_bit15() {
    let mut r = QuestFlags::default();
    r.set(5, bit::REWARD_PENDING);
    let l = QuestFlags::copy_in(&r.copy_out(), true).unwrap();
    assert!(l.get(5, bit::REWARD_PENDING));
    assert!(!l.get(5, bit::REWARD_GRANTED));
    assert!(l.get(5, bit::COMPLETED_BEFORE));
    assert!(completed(&l, 5));
}

// Covers: specs/world/quests.md §1.8 r3
#[test]
fn played_completion_bits_save_and_load() {
    let mut r = QuestFlags::default();
    r.set(2, bit::REWARD_GRANTED);
    r.set(2, bit::PRIMARY_GOAL_DONE);
    r.set(2, bit::COMPLETED_NOW);
    r.set(2, 7);
    // The writer copies out unchanged.
    let out = r.copy_out();
    assert_eq!(QuestFlags(out), r);
    // The next load clears 13 and 14 and keeps the path bits.
    let l = QuestFlags::copy_in(&out, true).unwrap();
    assert!(!l.get(2, bit::PRIMARY_GOAL_DONE) && !l.get(2, bit::COMPLETED_NOW));
    assert!(l.get(2, 7) && l.get(2, bit::REWARD_GRANTED));
    // reset_progress clears bits 2-11 only.
    let mut p = QuestFlags::default();
    for b in 0..16 {
        p.set(9, b);
    }
    p.reset_progress(9);
    assert_eq!(p.word(9), 0b1111_0000_0000_0011);
}

// Covers: specs/world/quests.md §1.8 r4
#[test]
fn smallest_completion_record_and_non_quest_slots() {
    let mut r = QuestFlags::default();
    r.set(41, bit::REWARD_GRANTED);
    r.set(41, bit::REWARD_PENDING);
    r.set(4, 10);
    assert_eq!(r.word(41), 0b11);
    assert_eq!(r.word(4), 1 << 10);
    assert!(completed(&r, 41));
    // Slot 34 stays unused: untouched by the above.
    assert_eq!(r.word(34), 0);
}

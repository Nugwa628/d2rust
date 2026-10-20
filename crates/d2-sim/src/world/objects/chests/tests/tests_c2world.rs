// Spec: specs/world/objects.md §8.3 (edge cases)
//! Gap test: the trap monster is the family base.

use super::*;

// Covers: specs/world/objects.md §edge-cases-original-bugs r25
#[test]
fn trap_monster_is_the_family_base_and_cached() {
    // A level whose first matching entry is zombie3 (class 7, act range
    // 5–9) spawns zombie1 (5), cached per level for the game.
    let (mut c, mut h) = setup(CHEST, 8, Seed::init_low(1));
    h.trap_monster = Some(7);
    assert_eq!(trap_monster_id(&mut c, &h, OBJ), Some(5));
    // The cache answers on the next call, whatever the region holds now.
    h.trap_monster = Some(9);
    assert_eq!(trap_monster_id(&mut c, &h, OBJ), Some(5));
}

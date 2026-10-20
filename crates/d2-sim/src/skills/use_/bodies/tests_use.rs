//! Tests of the skill missile helpers on [`BodyFake`].

use super::tests2::{monster, world};
use super::*;

#[test]
fn skill_missile_helpers_fill_the_record() {
    // Straight: flags 0x21, no origin unit, start = position + (dx, dy),
    // explicit aim used as given.
    let (mut f, u) = world();
    f.c.units[u].skills.clear();
    f.pos.insert(u, (100, 200));
    assert!(skill_missile(
        &mut f,
        7,
        u,
        1,
        3,
        (5, -6),
        (40, 50),
        false,
        false
    ));
    let r = f.missiles[0];
    assert_eq!((r.flags, r.origin), (0x21, None));
    assert_eq!((r.x, r.y, r.target_x, r.target_y), (105, 194, 40, 50));
    assert_eq!((r.class, r.skill, r.level), (7, 1, 3));
    assert_eq!(r.owner, u);

    // Lob: flags 0x420, origin = the unit.
    assert!(skill_missile(
        &mut f,
        7,
        u,
        1,
        3,
        (5, -6),
        (40, 50),
        false,
        true
    ));
    let r = f.missiles[1];
    assert_eq!((r.flags, r.origin), (0x420, Some(u)));
}

#[test]
fn skill_missile_helpers_target_fallback_and_null() {
    let (mut f, u) = world();
    // No aim and no target position: null, no missile.
    assert!(!skill_missile(
        &mut f,
        7,
        u,
        1,
        1,
        (0, 0),
        (0, 0),
        false,
        false
    ));
    assert!(f.missiles.is_empty());
    // A zero coordinate falls back to the unit's target position.
    f.tpos.insert(u, (33, 44));
    assert!(skill_missile(
        &mut f,
        7,
        u,
        1,
        1,
        (0, 0),
        (9, 0),
        false,
        false
    ));
    assert_eq!((f.missiles[0].target_x, f.missiles[0].target_y), (33, 44));
    // Target position with a 0 coordinate: null.
    f.tpos.insert(u, (33, 0));
    assert!(!skill_missile(
        &mut f,
        7,
        u,
        1,
        1,
        (0, 0),
        (0, 0),
        false,
        false
    ));
    // Take ammo, player with nothing to take: null.
    f.tpos.insert(u, (33, 44));
    assert!(!skill_missile(
        &mut f,
        7,
        u,
        1,
        1,
        (0, 0),
        (5, 5),
        true,
        false
    ));
    assert_eq!(f.missiles.len(), 1);
}

#[test]
fn straight_missile_of_a_monster_carries_its_tohit() {
    let (mut f, _) = world();
    let m = monster(&mut f, (10, 10));
    f.c.units[m].stats.insert((19, 0), 77);
    assert!(skill_missile(
        &mut f,
        7,
        m,
        1,
        1,
        (0, 0),
        (5, 5),
        false,
        false
    ));
    let r = f.missiles[0];
    assert_eq!((r.flags, r.attack_bonus), (0x21 | 0x1000, 77));
    // The lob form never sets the attack bonus.
    assert!(skill_missile(
        &mut f,
        7,
        m,
        1,
        1,
        (0, 0),
        (5, 5),
        false,
        true
    ));
    let r = f.missiles[1];
    assert_eq!((r.flags, r.attack_bonus), (0x420, 0));
    // A monster with no to-hit stat: no bonus flag.
    let n = monster(&mut f, (0, 0));
    assert!(skill_missile(
        &mut f,
        7,
        n,
        1,
        1,
        (0, 0),
        (5, 5),
        false,
        false
    ));
    assert_eq!(f.missiles[2].flags, 0x21);
}

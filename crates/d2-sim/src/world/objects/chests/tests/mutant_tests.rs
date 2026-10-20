// Spec: specs/world/objects.md §8
//! Mutation-testing kills for the chests (METHODS M08), on the chest
//! harness of the parent test module.

use super::*;

// Covers: specs/world/objects.md §8.1 r5
#[test]
fn chest_empty_quarter_is_below_25_only() {
    // r = 25 is not "r < 25": the unlocked plain chest drops once.
    let t = tables();
    let seed = find_seed(|s| s.roll(100) == 25);
    let (mut c, mut h) = setup(CHEST, 0, seed);
    assert_eq!(operate(&mut c, &t, &mut h, &op(CHEST, 4)).unwrap(), 1);
    assert_eq!(h.drop_qs(), s(&["drop 0 UnitId(10)"]));
}

// Covers: specs/world/objects.md §8.1 r3
#[test]
fn chest_sparkle_q6_is_below_5_only() {
    // Sparkling: Q := 6 if roll(100) < 5, else 4; r = 5 gives 4.
    let t = tables();
    let seed = find_seed(|s| s.roll(100) == 5);
    let (mut c, mut h) = setup(CHEST, 0, seed);
    c.get_mut(OBJ).unwrap().spark = 1;
    h.drops.push_back(Some(5));
    assert_eq!(operate(&mut c, &t, &mut h, &op(CHEST, 4)).unwrap(), 1);
    assert_eq!(h.drop_qs(), s(&["drop 4 UnitId(10)"]));
}

// Covers: specs/world/objects.md §8.1 r4
#[test]
fn class_397_band_edges_belong_to_the_upper_band() {
    // Each band is "r < bound": r equal to a bound runs the next band.
    // One scripted magic drop first, then nothing.
    let t = tables();
    let d = |q: u8, n: usize| vec![format!("drop {q} UnitId(10)"); n];
    let gold7 = vec!["code gld  UnitId(10)".to_string(); 7];
    let cases: [(u32, Vec<String>); 5] = [
        (200, d(5, 1)),
        (600, d(6, 1)),
        (1200, d(4, 10)),
        (3200, [d(4, 10), d(0, 1), gold7].concat()),
        (6200, [d(4, 1), tail(0)].concat()),
    ];
    for (r, want) in cases {
        let seed = find_seed(|s| s.roll(10000) == r);
        let (mut c, mut h) = setup(SPECIAL_CHEST, 0, seed);
        h.drops.push_back(Some(5));
        operate(&mut c, &t, &mut h, &op(SPECIAL_CHEST, 4)).unwrap();
        assert_eq!(h.drop_qs(), want, "r = {r}");
    }
}

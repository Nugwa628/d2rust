// Spec: specs/world/vendors.md §5.1
//! Mutation-testing kills (METHODS M08): each test pins an outcome the
//! spec decides that no earlier test checked. A child of `gamble` to call
//! the private upgrade step.

use super::super::tests::{index, p, record, tables, Fake, PLAYER};
use super::*;
use crate::rng::Seed;

/// hax (uber 9ha level 31, ultra 7ha level 54).
fn hax(t: &VendorTables) -> usize {
    index(t, "hax")
}

// From specs/world/vendors.md §5.1 r5: w_u = (L_g − uber.level)·uber + 1;
// roll(10000) < w_u → uber.
#[test]
fn upgrade_uber_boundary() {
    let t = tables();
    let (id, u) = (hax(&t), index(&t, "9ha"));
    // L_g = 31: w_u = 1. Roll 0 < 1 → uber; roll 1 → not (ultra w_x < 0).
    assert_eq!(upgrade(&t, &mut Seed::new(0, 0), id, 31, 100, 100), u);
    assert_eq!(upgrade(&t, &mut Seed::new(0, 1), id, 31, 100, 100), id);
}

/// A seed whose first roll(10000) is ≥ 1 and whose second is `second`.
fn seed_second(second: u32) -> Seed {
    (1..)
        .map(|x| Seed::new(0, x))
        .find(|s| {
            let mut c = *s;
            c.roll(10_000) >= 1 && c.roll(10_000) == second
        })
        .unwrap()
}

// From specs/world/vendors.md §5.1 r5: w_x = (L_g − ultra.level)·ultra +
// 1; w_x > 0 and roll(10000) < w_x → ultra.
#[test]
fn upgrade_ultra_boundary() {
    let t = tables();
    let (id, x) = (hax(&t), index(&t, "7ha"));
    // L_g = 54, uber mult 0: w_u = 1 (the first roll misses), w_x = 1.
    assert_eq!(upgrade(&t, &mut seed_second(0), id, 54, 0, 100), x);
    assert_eq!(upgrade(&t, &mut seed_second(1), id, 54, 0, 100), id);
    // L_g = 53, ultra mult 1: w_x = 0 → no second draw.
    let mut s = seed_second(0);
    let mut after = s;
    after.roll(10_000);
    assert_eq!(upgrade(&t, &mut s, id, 53, 0, 1), id);
    assert_eq!(s, after);
}

fn list(t: &VendorTables, w: &mut Fake, seed: &mut Seed) -> VendorRecord {
    let mut rec = record(t, super::super::class::GHEED, 0);
    make_list(&mut StoreCtx { tables: t, seed }, &mut rec, w, p());
    rec
}

// From specs/world/vendors.md §5.1 r2: T[L_g] < 1 → idx 0 without a draw;
// T = 1 draws roll(1).
#[test]
fn threshold_1_draws() {
    let run = |th: u32| {
        let mut t = tables();
        t.gamble_index = Some(vec![index(&t, "lax") as u32]);
        t.gamble_thresholds = vec![th; 100];
        let mut w = Fake::new(super::super::class::GHEED);
        let mut seed = Seed::new(7, 666);
        list(&t, &mut w, &mut seed);
        seed
    };
    assert_ne!(run(0), run(1));
}

// From specs/world/vendors.md §5.1 r7: only flag 0x10 is cleared.
#[test]
fn list_items_lose_only_the_identified_flag() {
    let mut t = tables();
    t.gamble_index = Some(vec![index(&t, "lax") as u32]);
    t.gamble_thresholds = vec![1; 100];
    let mut w = Fake::new(super::super::class::GHEED);
    w.create_queue.push_back(Some(true)); // created identified
    let rec = list(&t, &mut w, &mut Seed::new(7, 666));
    let items = &rec.gamble_lists[0].items;
    assert_eq!(rec.gamble_lists[0].player, PLAYER);
    assert!(items.iter().all(|u| w.unit(u.0).flags == 0));
}

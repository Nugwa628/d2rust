//! Gap tests of rules no other test pinned, on the fake world of `tests.rs` (a child
//! module so the fake and the synthetic tables are shared).

use super::*;

#[test]
fn cursor_holds_one_item_outside_every_grid() {
    let t = tables();
    let mut w = Fake::new();
    let mut inv = player_inv();
    let a = w.add(10, R_RING, mode::CURSOR);
    let b = w.add(11, R_RING, mode::CURSOR);
    let c = w.add(12, R_RING, mode::CURSOR);
    assert!(place_at_page(&mut inv, &mut w, &t, a, 0, 0, 0));
    assert_eq!(inv.cursor(), None);
    inv.set_cursor(Some(b));
    assert_eq!(inv.cursor(), Some(b));
    // Setting another item replaces it: one item at a time.
    inv.set_cursor(Some(c));
    assert_eq!(inv.cursor(), Some(c));
    for g in 0..inv.grid_count() {
        if let Some(grid) = inv.grid(g) {
            assert!(!grid.cells.contains(&Some(c)), "grid {g}");
            assert!(!grid.items.contains(&c), "grid {g}");
        }
    }
    assert_eq!(inv.items(), &[a]);
}

#[test]
fn place_in_page_link_check_kind_one_after_the_put() {
    let t = tables();
    let mut w = Fake::new();
    let mut inv = player_inv();
    let a = w.add(10, R_RING, mode::CURSOR);
    w.items.get_mut(&a).unwrap().page = 0;
    inv.set_cursor(Some(a));
    w.log.clear();
    assert!(place_in_page(
        &mut inv,
        &mut w,
        &t,
        Some(a),
        4,
        2,
        false,
        false
    ));
    // Step 4 put the item in its grid; step 5 is the first owner call,
    // with kind 1, before step 6's charm re-link.
    assert_eq!(inv.item_at(2, 4, 2), Some(a));
    assert_eq!(w.log[..2], ["link_check 10 1", "charm 10"]);
}

#[test]
fn equip_check_hand_table_every_row() {
    for (l, o) in [(4u8, 5u8), (5, 4), (11, 12), (12, 11)] {
        // N absent, T present → 3.
        let mut h = hands();
        h.equip(10, R_SWORD, l);
        assert_eq!(h.check(l, None), res::REMOVABLE, "L {l}");
        // N absent, T absent, X two-handed → 4; X other or absent → 0.
        let mut h = hands();
        assert_eq!(h.check(l, None), res::NO, "L {l}");
        let x = h.equip(10, R_BOW, o);
        assert_eq!(h.check(l, None), res::NO, "L {l}");
        h.w.p(x).two_handed = true;
        assert_eq!(h.check(l, None), res::OTHER_HAND_TWO_HANDED, "L {l}");
        // N present, T present: 6 stack, else 5 compatible, else 7 if X
        // fits page 0, else 0.
        let mut h = hands();
        h.equip(10, R_ARROWS, l);
        let n = h.w.add(11, R_ARROWS, mode::CURSOR);
        assert_eq!(h.check(l, Some(n)), res::STACK, "L {l}");
        let mut h = hands();
        h.equip(10, R_SWORD, l);
        let n = h.w.add(11, R_SWORD, mode::CURSOR);
        assert_eq!(h.check(l, Some(n)), res::SWAP, "L {l}");
        let x = h.equip(12, R_SHIELD, o);
        let n2 = h.w.add(13, R_2HSWORD, mode::CURSOR);
        h.w.p(n2).two_handed = true;
        assert_eq!(h.check(l, Some(n2)), res::SWAP_OTHER_TO_PAGE, "L {l}");
        h.w.no_free_page0 = true;
        assert_eq!(h.check(l, Some(n2)), res::NO, "L {l}");
        let _ = x;
        // N present, T absent, X present: 1 compatible, else 2.
        let mut h = hands();
        h.equip(10, R_SHIELD, o);
        let n = h.w.add(11, R_SWORD, mode::CURSOR);
        assert_eq!(h.check(l, Some(n)), res::FREE, "L {l}");
        let n2 = h.w.add(12, R_2HSWORD, mode::CURSOR);
        h.w.p(n2).two_handed = true;
        assert_eq!(h.check(l, Some(n2)), res::OTHER_HAND_BLOCKS, "L {l}");
        // N present, T absent, X absent → 1.
        let mut h = hands();
        let n = h.w.add(11, R_2HSWORD, mode::CURSOR);
        h.w.p(n).two_handed = true;
        assert_eq!(h.check(l, Some(n)), res::FREE, "L {l}");
    }
}

#[test]
fn equip_from_cursor_requirements_only_without_skip() {
    let o = |ok, out| EquipOutcome { ok, out };
    // Strength 0 fails in either mode.
    let mut h = hands();
    h.w.unit_stats.insert((PLAYER, stat::STRENGTH), 0);
    let n = h.w.add(10, R_SWORD, mode::CURSOR);
    h.inv.set_cursor(Some(n));
    // skip = 0: already refuses with out 0, so step 3's own
    // refusal (out 1) needs to pass equipping and fail not
    // equipping, which a consistent unit never does.
    assert_eq!(
        equip_from_cursor(&mut h.inv, &mut h.w, &h.t, Some(n), 4, false),
        o(false, false)
    );
    // skip = 1: neither step tests requirements; the item is equipped.
    assert_eq!(
        equip_from_cursor(&mut h.inv, &mut h.w, &h.t, Some(n), 4, true),
        o(true, false)
    );
    assert_eq!(h.inv.body_item(4), Some(n));
}

#[test]
fn weighted_search_finds_a_spot_whenever_one_fits() {
    // Every occupancy of a 3 × 3 grid, every item size up to 3 × 3: some
    // fitting spot touches an edge or an item (weight > 0), so the player
    // search fails exactly when nothing fits.
    for bits in 0u32..512 {
        let mut g = Grid::new(3, 3);
        for i in 0..9 {
            if bits & (1 << i) != 0 {
                g.cells[i] = Some(UnitId(900));
            }
        }
        for iw in 1..=3u8 {
            for ih in 1..=3u8 {
                let mut best = None;
                for y in 0..3 {
                    for x in 0..3 {
                        if in_bounds(&g, x, y, iw, ih) && fits(&g, x, y, iw, ih) {
                            best = best.max(Some(weight(&g, x, y, iw, ih)));
                        }
                    }
                }
                assert_ne!(best, Some(0), "{bits:#b} {iw}x{ih}");
                assert_eq!(
                    search(&g, iw, ih, true).is_some(),
                    best.is_some(),
                    "{bits:#b} {iw}x{ih}"
                );
            }
        }
    }
}

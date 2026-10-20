//! Tests added from a `cargo mutants` pass over `items::inventory`:
//! each one kills a
//! surviving mutant with an outcome the rules state. Mutants that no rule
//! outcome decides are listed in the note, not tested here.
//!
//! Each test checks one clause of a rule whose unit other tests already
//! cover, so none claims a rule of its own;
//! the `Rule` comment names the clause's unit.

use super::*;

// ---------------------------------------------------------------

// Rule (one clause; no claim):
#[test]
fn unlink_leaves_the_grid_list_and_handles_gridless_items() {
    let t = tables();
    let mut w = Fake::new();
    let mut inv = player_inv();
    let a = w.add(10, R_RING, mode::CURSOR);
    let b = w.add(11, R_RING, mode::CURSOR);
    assert!(place_at_page(&mut inv, &mut w, &t, a, 0, 0, 0));
    assert!(place_at_page(&mut inv, &mut w, &t, b, 0, 1, 0));
    assert!(inv.contains(a) && inv.contains(b));
    // Unlinking zeroes the node fields: the item leaves its grid's list,
    // the other item stays.
    assert!(inv.unlink(&mut w, a));
    assert_eq!(inv.grid(2).unwrap().items, vec![b]);
    assert!(!inv.contains(a));
    assert!(inv.contains(b));
    // An item linked without a grid (node grid 0): no cells to clear.
    let c = w.add(12, R_RING, mode::CURSOR);
    inv.link(&mut w, c, None);
    assert!(inv.contains(c));
    assert!(inv.unlink(&mut w, c));
    assert!(!inv.contains(c));
    assert_eq!(inv.grid(2).unwrap().items, vec![b]);
    assert_eq!(inv.item_at(2, 1, 0), Some(b));
}

// Not a game rule: the accessor contract of `Grid::cell` ("out of bounds
// → none"), which every caller relies on.
#[test]
fn grid_cell_out_of_bounds_is_none() {
    let g = grid_with(4, 3, &[(0, 0, 4, 3)]);
    for (x, y) in [
        (-1, 0),
        (0, -1),
        (4, 0),
        (0, 3),
        (-1, 2),
        (3, -1),
        (4, 2),
        (3, 3),
    ] {
        assert_eq!(g.cell(x, y), None, "({x}, {y})");
    }
    assert!(g.cell(3, 2).is_some());
}

// ---------------------------------------------------------------

/// The synthetic tables with `hp2` made 1 × 2 (still beltable) and
/// autobelt.
fn tall_potion_tables() -> InvTables {
    let mut t = tables();
    t.items[R_HP2].invheight = 2;
    t.items[R_HP2].autobelt = 1;
    t
}

// Rule (one clause; no claim):
#[test]
fn free_slot_needs_one_by_one() {
    let t = tall_potion_tables();
    let mut w = Fake::new();
    let inv = player_inv();
    // Beltable and autobelt, but 1 × 2: no slot (an empty belt would
    // otherwise give slot 0).
    let it = w.add(11, R_HP2, mode::CURSOR);
    assert!(beltable(&t, R_HP2));
    assert_eq!(free_belt_slot(&inv, &w, &t, it), None);
}

// Rule (one clause; no claim):
#[test]
fn place_in_slot_needs_one_by_one() {
    let t = tall_potion_tables();
    let mut w = Fake::new();
    let mut inv = player_inv();
    let it = w.add(11, R_HP2, mode::CURSOR);
    assert!(!place_in_belt_slot(&mut inv, &mut w, &t, it, 0));
    assert_eq!(inv.belt_item(0), None);
    assert!(!inv.contains(it));
}

// ---------------------------------------------------------------

// Rule (one clause; no claim):
#[test]
fn swap_locations_need_only_one_hand_location() {
    let mut t = tables();
    // A type allowed at the right hand and the head only.
    t.itemtypes[T_HELM as usize].bodyloc1 = 1;
    t.itemtypes[T_HELM as usize].bodyloc2 = 4;
    assert!(body_location_allowed(&t, R_HELM, 11));
    assert!(body_location_allowed(&t, R_HELM, 12));
    t.itemtypes[T_HELM as usize].bodyloc1 = 5;
    t.itemtypes[T_HELM as usize].bodyloc2 = 1;
    assert!(body_location_allowed(&t, R_HELM, 11));
    assert!(body_location_allowed(&t, R_HELM, 12));
}

// ---------------------------------------------------------------

// Rule (one clause; no claim):
#[test]
fn item_checks_owner_conditions() {
    let mut w = Fake::new();
    let mut inv = player_inv();
    let g = |w: &Fake, u: UnitId| w.d(u).guid;
    // Cursor: a mode-4 item that is not the player's cursor item → 1.
    let cur = w.add(10, R_RING, mode::CURSOR);
    let stray = w.add(11, R_RING, mode::CURSOR);
    inv.set_cursor(Some(cur));
    assert_eq!(cursor_item_check(&inv, &w, g(&w, stray)), 1);
    assert_eq!(cursor_item_check(&inv, &w, g(&w, cur)), 0);
    // Belt: mode 2 in the player's inventory passes.
    let belt = w.add(12, R_HP1, mode::BELT);
    w.items.get_mut(&belt).unwrap().inv = Some(PLAYER);
    assert_eq!(belt_item_check(&inv, &w, g(&w, belt)), 0);
    // Ground or owned: mode 4 goes to the owned check (the cursor item
    // passes, another mode-4 item fails).
    assert_eq!(ground_or_owned_check(&inv, &w, g(&w, cur)), 0);
    assert_eq!(ground_or_owned_check(&inv, &w, g(&w, stray)), 1);
}

// ------------------------------------------------------------------------

// Rule (one clause; no claim):
#[test]
fn free_position_searches_the_page_grid() {
    let t = tables();
    let mut w = Fake::new();
    let mut inv = player_inv();
    // Cube (page 3, grid 5, 3 × 4): fill every cell but (1, 2).
    let mut id = 10;
    for y in 0..4 {
        for x in 0..3 {
            if (x, y) == (1, 2) {
                continue;
            }
            let r = w.add(id, R_RING, mode::CURSOR);
            w.items.get_mut(&r).unwrap().page = page::CUBE;
            assert!(place_at_page(&mut inv, &mut w, &t, r, page::CUBE, x, y));
            id += 1;
        }
    }
    let r = w.add(id, R_RING, mode::CURSOR);
    assert_eq!(
        find_free_position(&mut inv, &w, &t, r, page::CUBE),
        Some((1, 2))
    );
}

// ---------------------------------------------------------------

// Rule (one clause; no claim):
#[test]
fn requirement_bounds_are_inclusive() {
    let (t, mut w, s) = req_setup(); // reqstr 25, reqdex 10
                                     // Dexterity: no percent, not ethereal → requirement 10 exactly.
    w.unit_stats.insert((PLAYER, stat::DEXTERITY), 9);
    assert!(!requirements_met(&w, &t, Some(s), PLAYER, false));
    w.unit_stats.insert((PLAYER, stat::DEXTERITY), 10);
    assert!(requirements_met(&w, &t, Some(s), PLAYER, false));
    // Equipping an active item: the stat less its own contribution equal
    // to the requirement passes, one below fails.
    w.unit_stats.insert((PLAYER, stat::STRENGTH), 40);
    w.p(s).active = true;
    w.p(s).contribution.insert(stat::STRENGTH, 15);
    assert!(requirements_met(&w, &t, Some(s), PLAYER, true));
    w.p(s).contribution.insert(stat::STRENGTH, 16);
    assert!(!requirements_met(&w, &t, Some(s), PLAYER, true));
}

// Rule (one clause; no claim):
#[test]
fn equip_check_left_and_swap_right_see_the_other_hand() {
    // N absent, L empty, the other hand two-handed → 4 (step 4), for
    // L = 5 (other 4) and L = 11 (other 12).
    let mut h = hands();
    let b = h.equip(10, R_BOW, 4);
    h.w.p(b).two_handed = true;
    assert_eq!(h.check(5, None), res::OTHER_HAND_TWO_HANDED);
    let mut h = hands();
    let b = h.equip(10, R_BOW, 12);
    h.w.p(b).two_handed = true;
    assert_eq!(h.check(11, None), res::OTHER_HAND_TWO_HANDED);
}

// Rule (one clause; no claim):
#[test]
fn equip_from_cursor_skip_bypasses_requirements() {
    let mut h = hands();
    let n = h.w.add(10, R_HELM, mode::CURSOR);
    h.inv.set_cursor(Some(n));
    // Not identified: fails; with skip = 1 neither step 2 nor step 3
    // tests it.
    h.w.items.get_mut(&n).unwrap().flags = 0;
    assert_eq!(
        equip_from_cursor(&mut h.inv, &mut h.w, &h.t, Some(n), 1, true),
        EquipOutcome {
            ok: true,
            out: false
        }
    );
    assert_eq!(h.inv.body_item(1), Some(n));
}

// Rule (one clause; no claim):
#[test]
fn auto_equip_needs_a_body_type() {
    let t = tables();
    let mut w = Fake::new();
    let inv = player_inv();
    // hp1: itemtypes `body` 0 → no location.
    let p = w.add(10, R_HP1, mode::GROUND);
    assert_eq!(t.itype_of(R_HP1).unwrap().body, 0);
    assert_eq!(auto_equip_location(&inv, &w, &t, p, false), None);
    assert_eq!(auto_equip_location(&inv, &w, &t, p, true), None);
}

// ---------------------------------------------------------------- tables

// The "item is type T" bullet of item generation (`type2`
// counts only when > 0). No claim: is one unit holding several
// bullets, and this checks one.
#[test]
fn is_type_reads_type2_only_when_positive() {
    let mut t = tables();
    // Ring with type2 = helm: a ring, a helm, not a book.
    t.items[R_RING].type2 = T_HELM;
    assert!(t.is_type(R_RING, T_RING));
    assert!(t.is_type(R_RING, T_HELM));
    assert!(!t.is_type(R_RING, T_BOOK));
    // type2 = 0 is "none": row 0 is not a type of the ring.
    t.items[R_RING].type2 = 0;
    assert!(!t.is_type(R_RING, 0));
    assert!(t.is_type(R_RING, T_RING));
}

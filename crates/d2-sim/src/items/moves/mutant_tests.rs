//! Tests added from a `cargo mutants` pass over `items::moves`:
//! each one kills a surviving
//! mutant with an outcome the rules state, on the shared fake world of
//! `tests/mod.rs`. Mutants that no rule outcome decides are listed in the
//! note, not tested here.
//!
//! Each test checks one clause of a rule whose unit other tests already
//! cover, so none claims a rule of its own;
//! the `Rule` comment names the clause's unit.

use super::{me, Fake, P};
use crate::items::moves::deferred::{category, mark, player_update};
use crate::items::moves::ground::{
    can_pick, gold_pickup, pickup_auto, pickup_to_cursor, refused_pickup,
};
use crate::items::moves::handlers::{insert_item, to_belt};
use crate::items::moves::{handle, iflag, layouts, mode, res, ty, MoveUnits, Outcome, Owner};

fn m32(id: u8, fields: &[u32]) -> Vec<u8> {
    let mut b = vec![id];
    for v in fields {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}

fn mloc(id: u8, item: u32, loc: u8) -> Vec<u8> {
    let mut b = m32(id, &[item]);
    b.extend_from_slice(&[loc, 0, 0, 0]);
    b
}

fn run(f: &mut Fake, msg: &[u8]) -> u32 {
    handle(f, P, msg).expect("handled id").expect("no fatal")
}

// ---------------------------------------------------------------

// Rule (one clause; no claim):
#[test]
fn mark_keeps_a_flag_already_set() {
    let mut f = Fake::new();
    f.item(10, mode::STORED).cmd = 0x80;
    mark(&mut f, me(), 10, 0x80);
    assert_eq!(f.it(10).cmd, 0x80);
}

// Rule (one clause; no claim):
#[test]
fn item_update_list_needs_its_bit_0() {
    let mut f = Fake::new();
    f.item(11, mode::EQUIPPED).cmd = 0x8;
    f.item(12, mode::SOCKETED).cmd = 0x10;
    f.inv_mut().update = vec![11];
    f.invs.entry(Owner::item(11)).or_default().update = vec![12];
    f.set_update_bits(me(), 1);
    // Item 11's +0xC8 bit 0 clear (bit 1 set): its list is not walked.
    f.set_update_bits(Owner::item(11), 2);
    let heads: Vec<[u8; 2]> = player_update(&mut f, P, P)
        .unwrap()
        .iter()
        .map(|b| [b[0], b[1]])
        .collect();
    assert_eq!(heads, vec![[0x9D, 6], [0x47, 0], [0x48, 0]]);
}

// Rule (one clause; no claim):
#[test]
fn category_monster_classes() {
    let mut f = Fake::new();
    let m = Owner::monster(50);
    for g in [10, 11] {
        let it = f.item(g, mode::EQUIPPED);
        it.component = 5;
        it.owner = Some(m);
    }
    f.items.get_mut(&10).unwrap().body_loc = 4;
    f.items.get_mut(&11).unwrap().body_loc = 5;
    let inv = f.invs.entry(m).or_default();
    inv.body.insert(4, 10);
    inv.body.insert(5, 11);
    f.unit(m).class = 0x1A1;
    assert_eq!(category(&f, 10), 6);
    f.unit(m).class = 0x100;
    assert_eq!(category(&f, 10), 5);
}

// ---------------------------------------------------------------

// Rule (one clause; no claim):
#[test]
fn insert_item_missing_player_or_existing_non_player() {
    // Player missing: page ≤ 4 goes on to step 4 (page 1 too).
    let mut f = Fake::new();
    f.item(10, mode::CURSOR);
    f.units.remove(&me());
    assert_eq!(run(&mut f, &m32(0x18, &[10, 0, 0, 1])), res::OK);
    // An existing non-player owner: pages 1 and 4 go on to step 4.
    for pg in [1, 4] {
        let mut f = Fake::new();
        let m = Owner::monster(50);
        f.unit(m);
        f.item(10, mode::CURSOR);
        f.inv_mut().cursor = None;
        f.invs.entry(m).or_default().cursor = Some(10);
        f.k.in_town = false;
        assert_eq!(insert_item(&mut f, m, 10, 0, 0, pg), res::OK, "page {pg}");
    }
}

// Rule (one clause; no claim):
#[test]
fn swap_2handed_left_hand() {
    let mut f = Fake::new();
    f.item(10, mode::CURSOR);
    f.item(11, mode::EQUIPPED).body_loc = 4;
    f.inv_mut().body.insert(4, 11);
    f.unit(Owner::item(11)).uflags = 0x4 | 0x2;
    f.k.equip_check = 2;
    assert_eq!(run(&mut f, &mloc(0x1B, 10, 5)), res::OK);
    assert_eq!(f.inv().body.get(&5), Some(&10));
    assert_eq!(f.it(11).mode, mode::CURSOR);
    // X: unit flag 0x2 cleared, other unit flags kept.
    assert_eq!(f.unit_flags(Owner::item(11)), 0x4);
}

// Rule (one clause; no claim):
#[test]
fn swap_cursor_with_body_belt_needs_the_belt_check() {
    let mut f = Fake::new();
    f.item(10, mode::CURSOR);
    let b = f.item(11, mode::EQUIPPED);
    b.body_loc = 8;
    b.types = vec![ty::BELT];
    f.inv_mut().body.insert(8, 11);
    f.k.equip_check = 5;
    // Refuses (the seam's reading): result 0, nothing moves.
    assert_eq!(run(&mut f, &mloc(0x1D, 10, 8)), res::OK);
    assert_eq!(f.inv().cursor, Some(10));
    assert_eq!(f.it(11).mode, mode::EQUIPPED);
}

// Rule (one clause; no claim):
#[test]
fn swap_cursor_with_body_sets_flags_on_set_flags() {
    let mut f = Fake::new();
    f.item(10, mode::CURSOR).iflags = iflag::CHANGED;
    f.item(11, mode::EQUIPPED).body_loc = 1;
    f.inv_mut().body.insert(1, 11);
    f.k.equip_check = 5;
    assert_eq!(run(&mut f, &mloc(0x1D, 10, 1)), res::OK);
    // N: item flags 0x40 and 0x1 (0x1 already set stays set).
    assert_eq!(f.it(10).iflags, 0x41);
}

// Rule (one clause; no claim):
#[test]
fn stack_items_exact_fit_merges() {
    let mut f = Fake::new();
    f.item(10, mode::CURSOR).types = vec![ty::BOOK];
    f.item(11, mode::STORED).max_stack = 20;
    f.set_stat(Owner::item(10), 70, 10);
    f.set_stat(Owner::item(11), 70, 10);
    f.set_stat(Owner::item(10), 72, 3);
    f.set_stat(Owner::item(11), 72, 3);
    // q_s + q_d = m: not above m → merge.
    assert_eq!(run(&mut f, &m32(0x21, &[10, 11])), res::OK);
    assert_eq!(f.stat(Owner::item(11), 70), 20);
    assert!(f.logged("free 10"));
    assert_eq!(f.it(11).iflags & iflag::STACK_FULL, 0);
    assert_eq!(f.inv().cursor, None);
    // Equal stat 72: not lowered, no 0x3E for it.
    assert!(!f.logged("3E 11 72"));
    // Only src is a book: no book count change.
    assert!(f.log.iter().all(|l| !l.starts_with("book")));
}

// Rule (one clause; no claim):
#[test]
fn to_belt_needs_the_cursor_mode() {
    let mut f = Fake::new();
    f.item(10, mode::STORED).beltable = true;
    f.k.belt_slot = Some(0);
    assert_eq!(to_belt(&mut f, me(), 10, 0), Outcome::REFUSED);
    assert_eq!(f.it(10).mode, mode::STORED);
}

fn socket_setup(target_mode: u8, target_flags: u32, filler: bool, filler_flags: u32) -> Fake {
    let mut f = Fake::new();
    let fl = f.item(10, mode::CURSOR);
    fl.filler = filler;
    fl.iflags = filler_flags;
    let t = f.item(11, target_mode);
    t.iflags = target_flags;
    t.sockets = 1;
    f
}

// Rule (one clause; no claim):
#[test]
fn socket_item_conditions() {
    let ok = iflag::IDENTIFIED | iflag::SOCKETED;
    // Equipped target (mode 1) is accepted.
    let mut f = socket_setup(mode::EQUIPPED, ok, true, iflag::IDENTIFIED);
    assert_eq!(run(&mut f, &m32(0x28, &[10, 11])), res::OK);
    assert_eq!(f.it(10).mode, mode::SOCKETED);
    // Target missing → out 1.
    let mut f = socket_setup(mode::STORED, ok, true, iflag::IDENTIFIED);
    f.items.remove(&11);
    assert_eq!(run(&mut f, &m32(0x28, &[10, 11])), res::REFUSED);
    // Target not identified → 0, nothing linked.
    let mut f = socket_setup(mode::STORED, iflag::SOCKETED, true, iflag::IDENTIFIED);
    assert_eq!(run(&mut f, &m32(0x28, &[10, 11])), res::OK);
    assert_eq!(f.it(10).mode, mode::CURSOR);
    assert!(f.it(11).fillers.is_empty());
    // Filler identified but not a socket filler → nothing.
    let mut f = socket_setup(mode::STORED, ok, false, iflag::IDENTIFIED);
    assert_eq!(run(&mut f, &m32(0x28, &[10, 11])), res::OK);
    assert_eq!(f.it(10).mode, mode::CURSOR);
    // Socket filler not identified → nothing.
    let mut f = socket_setup(mode::STORED, ok, true, 0);
    assert_eq!(run(&mut f, &m32(0x28, &[10, 11])), res::OK);
    assert_eq!(f.it(10).mode, mode::CURSOR);
    assert!(f.it(11).fillers.is_empty());
}

// Rule (one clause; no claim):
#[test]
fn scroll_to_book_ground_scroll_and_book_type() {
    let mut f = Fake::new();
    f.item(10, mode::GROUND).types = vec![ty::SCRO];
    let b = f.item(11, mode::STORED);
    b.types = vec![ty::BOOK];
    b.max_stack = 20;
    f.set_stat(Owner::item(11), 70, 5);
    // A scroll on the ground (mode 3) is taken.
    assert_eq!(run(&mut f, &m32(0x29, &[10, 11])), res::OK);
    assert_eq!(f.stat(Owner::item(11), 70), 6);
    // A stored item that is not a book → out 1.
    let mut f = Fake::new();
    f.item(10, mode::CURSOR).types = vec![ty::SCRO];
    f.item(11, mode::STORED).max_stack = 20;
    assert_eq!(run(&mut f, &m32(0x29, &[10, 11])), res::REFUSED);
    assert_eq!(f.stat(Owner::item(11), 70), 0);
}

// Rule (one clause; no claim):
#[test]
fn drop_gold_bounds_are_inclusive() {
    // amount = gold.
    let mut f = Fake::new();
    f.set_stat(me(), 14, 700);
    assert_eq!(run(&mut f, &m32(0x50, &[P, 700])), res::OK);
    assert_eq!(f.stat(me(), 14), 0);
    // amount = gold = limit (level 1 → 10000).
    let mut f = Fake::new();
    f.set_stat(me(), 14, 10_000);
    assert_eq!(run(&mut f, &m32(0x50, &[P, 10_000])), res::OK);
    assert_eq!(f.stat(me(), 14), 0);
}

// Rule (one clause; no claim):
#[test]
fn merc_give_act5_axe_must_be_one_handed() {
    use crate::items::moves::handlers::merc_give;
    let m = Owner::monster(60);
    let gives = |types: Vec<u16>, two: bool| {
        let mut f = Fake::new();
        f.k.hireling = Some(m);
        f.unit(m).class = 0x230;
        let c = f.item(10, mode::CURSOR);
        c.types = types;
        c.iflags = iflag::IDENTIFIED;
        c.two_handed = two;
        merc_give(&mut f, me(), Some(m), 10);
        f.logged("equip_on_merc 10")
    };
    assert!(gives(vec![ty::AXE], false));
    assert!(!gives(vec![ty::AXE], true));
    assert!(!gives(vec![ty::SWOR], false));
}

// ---------------------------------------------------------------

fn ground(f: &mut Fake, g: u32) -> &mut super::FItem {
    f.item(g, mode::GROUND)
}

// Rule (one clause; no claim):
#[test]
fn pickup_refused_while_busy_without_cursor() {
    let mut f = Fake::new();
    ground(&mut f, 10);
    f.k.busy = true;
    assert_eq!(pickup_auto(&mut f, me(), 10), Ok(Outcome::NOTHING));
    assert_eq!(pickup_to_cursor(&mut f, me(), 10), Outcome::NOTHING);
    assert_eq!(f.it(10).mode, mode::GROUND);
    assert_eq!(f.inv().cursor, None);
}

// Rule (one clause; no claim):
#[test]
fn auto_pickup_page_path_leaves_the_room_and_skips_the_belt() {
    let mut f = Fake::new();
    ground(&mut f, 10); // not beltable
    f.k.belt_slot = Some(1);
    assert_eq!(pickup_auto(&mut f, me(), 10), Ok(Outcome::DONE));
    assert!(!f.log.iter().any(|l| l.starts_with("belt_place")));
    assert_eq!(f.it(10).cmd, 0x80);
    for s in ["room_delete 10", "free_collision 10", "remove_from_room 10"] {
        assert!(f.logged(s), "{s}");
    }
}

// Rule (one clause; no claim):
#[test]
fn pickup_to_cursor_flag_arithmetic() {
    let mut f = Fake::new();
    let it = ground(&mut f, 10);
    it.cmd = 0x40;
    f.unit(Owner::item(10)).uflags = 0x4 | 0x2 | 0x200_0000;
    assert_eq!(pickup_to_cursor(&mut f, me(), 10), Outcome::DONE);
    // Command flag 0x40 set (already set: stays); unit flags 0x2 and
    // 0x2000000 cleared, others kept.
    assert_eq!(f.it(10).cmd, 0x40);
    assert_eq!(f.unit_flags(Owner::item(10)), 0x4);
}

// Rule (one clause; no claim):
#[test]
fn refused_pickup_keeps_unit_flag_0x1000() {
    let mut f = Fake::new();
    ground(&mut f, 10);
    refused_pickup(&mut f, me(), 10, 0x13);
    refused_pickup(&mut f, me(), 10, 0x13);
    assert_eq!(f.unit_flags(Owner::item(10)), 0x1000);
}

/// The player holds a quest item `held` (quest 7); the ground item has
/// code `code`, quest 7.
fn quest_pick(held: &[u8; 4], code: &[u8; 4]) -> bool {
    let mut f = Fake::new();
    let h = f.item(11, mode::STORED);
    h.quest = 7;
    h.code = *held;
    let g = ground(&mut f, 10);
    g.quest = 7;
    g.code = *code;
    can_pick(&f, me(), 10)
}

// Rule (one clause; no claim):
#[test]
fn held_test_codes_and_pairs() {
    assert!(quest_pick(b"xxx ", b"yyy "));
    assert!(quest_pick(b"j34 ", b"bkd "));
    assert!(quest_pick(b"g34 ", b"bkd "));
    assert!(quest_pick(b"bks ", b"g34 "));
    assert!(!quest_pick(b"xxx ", b"xxx "));
    assert!(!quest_pick(b"j34 ", b"g34 "));
    assert!(!quest_pick(b"g34 ", b"j34 "));
    assert!(!quest_pick(b"msf ", b"hst "));
}

// Rule (one clause; no claim):
#[test]
fn carry_one_needs_a_unique() {
    // Magic (quality 4) items with a file index and the carry-one bit are
    // not carry-one uniques: holding one does not block another.
    let mut f = Fake::new();
    for (g, m) in [(11, mode::STORED), (10, mode::GROUND)] {
        let it = f.item(g, m);
        it.quality = 4;
        it.file_index = 4;
        it.carry_one = true;
    }
    assert!(can_pick(&f, me(), 10));
}

// Rule (one clause; no claim):
#[test]
fn g33_needs_both_flags_clear() {
    let mut f = Fake::new();
    let g = ground(&mut f, 10);
    g.quest = 1;
    g.code = *b"g33 ";
    assert!(can_pick(&f, me(), 10));
    f.k.quest_flags.insert((19, 7));
    assert!(!can_pick(&f, me(), 10));
    f.k.quest_flags.clear();
    f.k.quest_flags.insert((19, 8));
    assert!(!can_pick(&f, me(), 10));
}

// ----------------------------------------------------------------

// Rule (one clause; no claim):
#[test]
fn gold_pickup_sums_and_owner_kinds() {
    // g + p within the limit: add.
    let mut f = Fake::new();
    f.set_stat(me(), 14, 100);
    ground(&mut f, 10).types = vec![ty::GOLD];
    f.set_stat(Owner::item(10), 14, 300);
    gold_pickup(&mut f, me(), 10);
    assert_eq!(f.stat(me(), 14), 400);
    // A pile owned by a non-player is added like an unowned one.
    let mut f = Fake::new();
    ground(&mut f, 10).types = vec![ty::GOLD];
    f.set_stat(Owner::item(10), 14, 300);
    f.k.pile_owner = Some(Owner::monster(5));
    gold_pickup(&mut f, me(), 10);
    assert_eq!(f.stat(me(), 14), 300);
    // A negative sum sets the stat to 0.
    let mut f = Fake::new();
    f.set_stat(me(), 14, -500);
    ground(&mut f, 10).types = vec![ty::GOLD];
    f.set_stat(Owner::item(10), 14, 100);
    gold_pickup(&mut f, me(), 10);
    assert_eq!(f.stat(me(), 14), 0);
}

// Rule (one clause; no claim):
#[test]
fn gold_message_boundary_0xff() {
    // new = 0xFF is not < 0xFF: 0x1E with a u16.
    assert_eq!(
        layouts::gold(0xFF, 0x200),
        Some(vec![0x1E, 0x0E, 0xFF, 0x00])
    );
    assert_eq!(layouts::gold(0xFE, 0x200), Some(vec![0x1D, 0x0E, 0xFE]));
}

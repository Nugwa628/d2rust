//! Tests of the item-move edge cases settled after the first pass: the dispatcher
//! walk, the fillers' 0x9D, the update-list reset, the belt change, the
//! bodies of 0x1E / 0x20 / 0x27, the pickup specials, the held test and
//! the ground expiry reader.

use super::{me, Fake, P};
use crate::items::moves::deferred::{
    dispatch, item_reset, room_cleanup, send_item_page, update_list_reset, NO_FILLERS,
};
use crate::items::moves::ground::{can_pick, expired_items, held, pickup_auto, tome_for};
use crate::items::moves::handlers::belt_change;
use crate::items::moves::{
    handle, layouts, mode, res, ty, InventoryOps, MoveFatal, MoveUnits, Outcome, Owner,
};

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

fn run(f: &mut Fake, msg: &[u8]) -> Result<u32, MoveFatal> {
    handle(f, P, msg).expect("handled id")
}

// ---------------------------------------------------------------

#[test]
fn excluded_row_lets_the_walk_go_on() {
    // Row 3 (0x2, owner only) matches by flags; for another client the
    // walk goes on to row 5 (0x8, all).
    let mut f = Fake::new();
    f.item(10, mode::STORED).cmd = 0x2 | 0x8;
    let m = dispatch(&mut f, 2, me(), 10).unwrap();
    assert_eq!(m.len(), 1);
    assert_eq!(&m[0][..2], &[0x9D, 0x06]);
    let m = dispatch(&mut f, P, me(), 10).unwrap();
    assert_eq!(&m[0][..2], &[0x9C, 0x04]);
}

#[test]
fn item_flag_rows_18_19_end_the_walk() {
    // Broken (row 18) and changed (row 20), mode 0, another client: the
    // walk ends at row 18 with nothing sent; row 20 is not tried.
    let mut f = Fake::new();
    f.item(10, mode::STORED).iflags = 0x100 | 0x1;
    assert!(dispatch(&mut f, 2, me(), 10).unwrap().is_empty());
    // Repaired (row 19) the same.
    f.items.get_mut(&10).unwrap().iflags = 0x200 | 0x1;
    assert!(dispatch(&mut f, 2, me(), 10).unwrap().is_empty());
    // The owner gets the 0x7D with state = item flags & 0x200.
    let m = dispatch(&mut f, P, me(), 10).unwrap();
    assert_eq!(m, vec![layouts::item_state(0, P, 10, 0x200, 0x200)]);
}

#[test]
fn no_fillers_with_flag_0x20() {
    let mut f = Fake::new();
    f.k.bits = vec![0xEE];
    let it = f.item(10, mode::STORED);
    it.iflags = 0x800;
    it.fillers = vec![11];
    f.item(11, mode::SOCKETED);
    send_item_page(&mut f, me(), 10, NO_FILLERS, 3).unwrap();
    assert_eq!(f.sent.len(), 1);
    // Without 0x20 the filler follows, owned by the parent item, with the
    // flag argument | 0x8.
    send_item_page(&mut f, me(), 10, 0x4, 3).unwrap();
    assert_eq!(f.sent.len(), 3);
    assert_eq!(f.sent[2][..2], [0x9D, 0x13]);
    assert_eq!(f.sent[2][8..13], [4, 10, 0, 0, 0]);
    assert_eq!(f.sent[2][13..], [0xEE, 0xC, 0xFF]);
}

#[test]
fn update_list_reset_clears_by_the_tables() {
    let mut f = Fake::new();
    let a = f.item(10, mode::CURSOR);
    a.cmd = 0x10 | 0x1 | 0x400000;
    a.iflags = 0x20 | 0x2 | 0x8 | 0x80 | 0x40 | 0x1 | 0x200 | 0x40000 | 0x10;
    a.body_loc = 4;
    let b = f.item(11, mode::CURSOR);
    b.cmd = 0x20;
    b.iflags = 0x80;
    b.body_loc = 5;
    let c = f.item(12, mode::STORED);
    c.cmd = 0x20;
    c.body_loc = 3;
    // Item 12 has its own changed list with item 13.
    f.item(13, mode::SOCKETED).cmd = 0x8;
    f.invs.entry(Owner::item(12)).or_default().update = vec![13];
    f.set_update_bits(Owner::item(12), 1 | 0x4 | 0x10);
    f.inv_mut().update = vec![10, 99, 11, 12];
    f.set_update_bits(me(), 3);
    update_list_reset(&mut f, me());
    // Bit 0 cleared; bit 1 ("save pending") stays (IS1).
    assert_eq!(f.update_bits(me()), 2);
    // Body location 0 for 0x10, and for 0x20 with item flag 0x80 only.
    assert_eq!(
        (f.it(10).body_loc, f.it(11).body_loc, f.it(12).body_loc),
        (0, 0, 3)
    );
    // Command flags of the table cleared; 0x1 and bits outside it stay.
    assert_eq!(f.it(10).cmd, 0x1 | 0x400000);
    assert_eq!(f.it(10).iflags, 0x10);
    // Command flag 0x1 → removal.
    assert!(f.logged("free 10"));
    assert!(!f.logged("free 11"));
    // The item's own list: reset, freed, its bit 0 and bits 4 / 0x10.
    assert_eq!(f.it(13).cmd, 0);
    assert_eq!(f.update_bits(Owner::item(12)), 0);
    assert!(f.invs[&Owner::item(12)].update.is_empty());
    assert!(f.inv().update.is_empty());
    // No inventory → nothing.
    let m = Owner::monster(9);
    f.set_update_bits(m, 1);
    update_list_reset(&mut f, m);
    assert_eq!(f.update_bits(m), 1);
}

#[test]
fn room_cleanup_clears_unit_flags_and_bits() {
    let mut f = Fake::new();
    f.set_unit_flags(me(), 0x1 | 0x2 | 0x10 | 0x400 | 0x8000);
    f.set_update_bits(me(), 0x1 | 0x2 | 0x800 | 0x1000 | 0x10000 | 0x200000 | 0x4);
    room_cleanup(&mut f, me());
    assert_eq!(f.unit_flags(me()), 0x2);
    assert_eq!(f.update_bits(me()), 0x2 | 0x4);
    let mut f = Fake::new();
    f.item(10, mode::STORED).cmd = 0x2;
    item_reset(&mut f, 10);
    assert_eq!(f.it(10).cmd, 0);
}

// ------------------------------------------------------------------

fn belt_item(f: &mut Fake, g: u32, slot: u8) {
    f.item(g, mode::BELT).beltable = true;
    assert!(f.belt_place(me(), g, u32::from(slot)));
}

#[test]
fn belt_change_moves_items_beyond_the_new_boxes() {
    let mut f = Fake::new();
    f.k.bits = vec![0xEE];
    f.k.boxes = 4;
    belt_item(&mut f, 10, 2);
    belt_item(&mut f, 11, 4);
    belt_item(&mut f, 12, 9);
    belt_change(&mut f, me(), None).unwrap();
    // Slots ≥ 4, in slot order: 0x9C action 0xF (flag 0x20) sent now,
    // out of grid 1, item-skill unlink, page 0, then find-free.
    assert_eq!(f.sent.len(), 2);
    assert_eq!(f.sent[0][..2], [0x9C, 0x0F]);
    assert_eq!(f.sent[0][8..], [0xEE, 0x20, 0xFF]);
    assert_eq!(f.sent[1][4..8], 12u32.to_le_bytes());
    assert!(f.logged("place_in_page 11 0,0 find=true send=true"));
    assert!(f.logged("charm_unlink 11"));
    assert_eq!(f.it(11).mode, mode::STORED);
    assert_eq!(f.belt_item(me(), 2), Some(10));
    assert_eq!(f.belt_item(me(), 4), None);
    // No room on page 0: dropped at the unit's position.
    let mut f = Fake::new();
    f.k.boxes = 4;
    f.k.place_ok = true;
    belt_item(&mut f, 11, 5);
    f.k.place_ok = false;
    belt_change(&mut f, me(), None).unwrap();
    assert_eq!(f.it(11).mode, mode::GROUND);
    assert_eq!(f.spot_calls.borrow()[0], ((100, 100), (100, 100), 1));
    // No spot either: refuse the belt change and keep its contents linked.
    let mut f = Fake::new();
    f.k.boxes = 4;
    belt_item(&mut f, 11, 5);
    f.k.place_ok = false;
    f.k.spot = None;
    assert_eq!(belt_change(&mut f, me(), None), Err(MoveFatal::Link));
    assert_eq!(f.it(11).mode, mode::BELT);
    assert_eq!(f.inv().cursor, None);
    assert!(f.inv().list.contains(&11));
    assert_eq!(f.belt_item(me(), 5), Some(11));
    assert!(f.sent.is_empty());
}

// ---------------------------------------------------------------

#[test]
fn swap_cursor_with_body_failures_and_belt() {
    let setup = || {
        let mut f = Fake::new();
        f.item(10, mode::CURSOR);
        f.item(11, mode::EQUIPPED).body_loc = 1;
        f.inv_mut().body.insert(1, 11);
        f.k.equip_check = 5;
        f
    };
    // E not in mode 1 → out 1.
    let mut f = setup();
    f.items.get_mut(&11).unwrap().mode = mode::STORED;
    assert_eq!(run(&mut f, &mloc(0x1D, 10, 1)), Ok(res::REFUSED));
    // N's link failing restores the original cursor and body item.
    let mut f = setup();
    f.k.link_ok = false;
    assert_eq!(run(&mut f, &mloc(0x1D, 10, 1)), Ok(res::REFUSED));
    assert_eq!(f.inv().cursor, Some(10));
    assert_eq!(f.inv().body.get(&1), Some(&11));
    // N a belt: the belt change runs with N (its boxes).
    let mut f = setup();
    f.items.get_mut(&10).unwrap().types = vec![ty::BELT];
    f.k.boxes = 4;
    belt_item(&mut f, 20, 6);
    assert_eq!(run(&mut f, &mloc(0x1D, 10, 1)), Ok(res::OK));
    assert!(f.logged("place_in_page 20 0,0 find=true send=true"));
}

#[test]
fn swap_1h_with_2h_body() {
    let setup = || {
        let mut f = Fake::new();
        f.item(10, mode::CURSOR);
        f.item(11, mode::EQUIPPED).body_loc = 4;
        f.item(12, mode::EQUIPPED).body_loc = 5;
        f.inv_mut().body.insert(4, 11);
        f.inv_mut().body.insert(5, 12);
        f.k.equip_check = 7;
        f
    };
    let mut f = setup();
    assert_eq!(run(&mut f, &mloc(0x1E, 10, 4)), Ok(res::OK));
    // X (other hand) to page 0 with command flag 0x4000.
    let x = f.it(12);
    assert_eq!(
        (x.mode, x.page, x.cmd, x.iflags),
        (mode::STORED, 0, 0x4000, 0x1)
    );
    // T to the cursor (command flag 0x10).
    let t = f.it(11);
    assert_eq!((t.mode, t.cmd), (mode::CURSOR, 0x10));
    assert_eq!(f.inv().cursor, Some(11));
    // N at L: item flags 0x8 | 0x1, command flag 0x8.
    let n = f.it(10);
    assert_eq!(
        (n.mode, n.body_loc, n.page, n.cmd, n.iflags),
        (mode::EQUIPPED, 4, 0xFF, 0x8, 0x9)
    );
    assert_eq!(f.inv().update, vec![12, 11, 10]);
    // ≠ 7 → nothing; failing → out 1.
    let mut f = setup();
    f.k.equip_check = 5;
    assert_eq!(run(&mut f, &mloc(0x1E, 10, 4)), Ok(res::OK));
    assert_eq!(f.it(12).mode, mode::EQUIPPED);
    let mut f = setup();
    f.k.requirements = false;
    assert_eq!(run(&mut f, &mloc(0x1E, 10, 4)), Ok(res::REFUSED));
    // No free position for X → out 1.
    let mut f = setup();
    f.k.free = None;
    assert_eq!(run(&mut f, &mloc(0x1E, 10, 4)), Ok(res::REFUSED));
    // A failed placement restores both body items and the incoming cursor.
    let mut f = setup();
    f.k.place_ok = false;
    assert_eq!(run(&mut f, &mloc(0x1E, 10, 4)), Ok(res::REFUSED));
    assert_eq!(f.it(12).mode, mode::EQUIPPED);
    assert!(f.inv().list.contains(&12));
    assert_eq!(f.inv().body.get(&5), Some(&12));
    assert_eq!(f.inv().cursor, Some(10));
}

#[test]
fn swap_cursor_buffer_link_failure() {
    let mut f = Fake::new();
    f.item(10, mode::CURSOR);
    f.item(11, mode::STORED);
    f.k.link_ok = false;
    assert_eq!(run(&mut f, &m32(0x1F, &[10, 11, 2, 3])), Ok(res::REFUSED));
}

#[test]
fn use_grid_item_body_use() {
    let book = |f: &mut Fake, q: i32| {
        let it = f.item(10, mode::STORED);
        it.useable = true;
        it.types = vec![ty::BOOK];
        f.set_stat(Owner::item(10), 70, q);
    };
    // A tome with no charges → nothing.
    let mut f = Fake::new();
    book(&mut f, 0);
    assert_eq!(run(&mut f, &m32(0x20, &[10, 100, 100])), Ok(res::OK));
    assert!(!f.logged("use_at 10 100,100"));
    // A used tome with a skill: stat 70 − 1 (0x3E), 0x7C, decrement.
    let mut f = Fake::new();
    book(&mut f, 3);
    f.k.item_skill = 220;
    assert_eq!(run(&mut f, &m32(0x20, &[10, 100, 100])), Ok(res::OK));
    assert_eq!(f.stat(Owner::item(10), 70), 2);
    assert!(f.logged("3E 10 70") && f.logged("skill_dec 220"));
    assert_eq!(f.sent, vec![layouts::item_used(4, 10)]);
    // A used scroll: decrement when the player has the skill; consumed.
    let mut f = Fake::new();
    let it = f.item(10, mode::STORED);
    it.useable = true;
    it.types = vec![ty::SCRO];
    f.k.item_skill = 220;
    assert_eq!(run(&mut f, &m32(0x20, &[10, 100, 100])), Ok(res::OK));
    assert!(!f.logged("skill_dec 220") && f.logged("consume_item 10"));
    // Not useable → out 1; a cursor item → nothing.
    let mut f = Fake::new();
    f.item(10, mode::STORED);
    assert_eq!(run(&mut f, &m32(0x20, &[10, 100, 100])), Ok(res::REFUSED));
    f.items.get_mut(&10).unwrap().useable = true;
    f.item(11, mode::CURSOR);
    assert_eq!(run(&mut f, &m32(0x20, &[10, 100, 100])), Ok(res::OK));
    assert!(!f.logged("use_at 10 100,100"));
}

#[test]
fn use_grid_item_quest_items() {
    let quest_item = |code: &[u8; 4]| {
        let mut f = Fake::new();
        let it = f.item(10, mode::STORED);
        it.useable = true;
        it.code = *code;
        f.k.use_ok = false;
        f
    };
    // `ass`: flag (9, 5) set → cleared, newskills + 1, consumed.
    let mut f = quest_item(b"ass ");
    f.k.quest_flags.insert((9, 5));
    assert_eq!(run(&mut f, &m32(0x20, &[10, 100, 100])), Ok(res::OK));
    assert_eq!(f.stat(me(), 5), 1);
    assert!(f.logged("quest_flag 9 5 false") && f.logged("consume_item 10"));
    assert!(f.logged("quest_used 8"));
    // Flag clear → sound event 0x13 only.
    let mut f = quest_item(b"ass ");
    assert_eq!(run(&mut f, &m32(0x20, &[10, 100, 100])), Ok(res::OK));
    assert!(!f.logged("consume_item 10") && f.logged("sound 0x13"));
    assert!(!f.logged("pickup_sound"));
    // `xyz`: 20 life (8.8) on maxhp.
    let mut f = quest_item(b"xyz ");
    f.k.quest_flags.insert((20, 5));
    assert_eq!(run(&mut f, &m32(0x20, &[10, 100, 100])), Ok(res::OK));
    assert_eq!(f.stat(me(), 7), 0x1400);
    assert!(f.logged("quest_used 18"));
    // `tr2`: (37, 8) set and (37, 7) clear → (37, 7) set.
    let mut f = quest_item(b"tr2 ");
    f.k.quest_flags.insert((37, 8));
    assert_eq!(run(&mut f, &m32(0x20, &[10, 100, 100])), Ok(res::OK));
    assert!(f.logged("quest_flag 37 7 true") && f.logged("tr2"));
    assert!(f.logged("quest_used 33"));
    // `toa`: skills and stats reset.
    let mut f = quest_item(b"toa ");
    assert_eq!(run(&mut f, &m32(0x20, &[10, 100, 100])), Ok(res::OK));
    assert!(f.logged("reset_skills_stats") && f.logged("consume_item 10"));
    // Sound event 2.
    assert!(f.logged("sound 0x2") && !f.logged("pickup_sound"));
    // Other codes → nothing.
    let mut f = quest_item(b"key ");
    assert_eq!(run(&mut f, &m32(0x20, &[10, 100, 100])), Ok(res::OK));
    assert!(!f.logged("consume_item 10"));
}

#[test]
fn switch_belt_item_link_failure_is_fatal() {
    let mut f = Fake::new();
    f.item(10, mode::CURSOR).beltable = true;
    f.item(11, mode::BELT);
    f.k.link_ok = false;
    assert_eq!(run(&mut f, &m32(0x25, &[10, 11])), Err(MoveFatal::Link));
}

fn tome(f: &mut Fake, g: u32, q: i32) {
    let it = f.item(g, mode::STORED);
    it.types = vec![ty::BOOK];
    it.max_stack = 20;
    f.set_stat(Owner::item(g), 70, q);
}

#[test]
fn use_item_action_gates() {
    // T = U → nothing.
    let mut f = Fake::new();
    f.item(11, mode::STORED);
    assert_eq!(run(&mut f, &m32(0x27, &[11, 11])), Ok(res::OK));
    // A belt U that is not a scroll → out 1.
    let mut f = Fake::new();
    f.item(10, mode::STORED);
    f.item(11, mode::BELT);
    assert_eq!(run(&mut f, &m32(0x27, &[10, 11])), Ok(res::REFUSED));
    // T not in mode 0 / 1, U a stored tome: a charge spent, nothing used.
    let mut f = Fake::new();
    f.item(10, mode::BELT);
    tome(&mut f, 11, 3);
    assert_eq!(run(&mut f, &m32(0x27, &[10, 11])), Ok(res::OK));
    assert_eq!(f.stat(Owner::item(11), 70), 2);
    // An empty tome → 0x7C, nothing used.
    let mut f = Fake::new();
    f.item(10, mode::STORED);
    tome(&mut f, 11, 0);
    assert_eq!(run(&mut f, &m32(0x27, &[10, 11])), Ok(res::OK));
    assert_eq!(f.sent, vec![layouts::item_used(4, 11)]);
    assert!(!f.logged("use 11 on 4:10"));
}

#[test]
fn use_item_action_effects() {
    // Not used → result 1 (handler 0), nothing spent.
    let mut f = Fake::new();
    f.item(10, mode::STORED);
    tome(&mut f, 11, 3);
    f.k.use_ok = false;
    assert_eq!(run(&mut f, &m32(0x27, &[10, 11])), Ok(res::OK));
    assert_eq!(f.stat(Owner::item(11), 70), 3);
    // A used tome with a skill: − 1, decrement, 0x7C.
    let mut f = Fake::new();
    f.item(10, mode::STORED);
    tome(&mut f, 11, 3);
    f.k.item_skill = 218;
    assert_eq!(run(&mut f, &m32(0x27, &[10, 11])), Ok(res::OK));
    assert_eq!(f.stat(Owner::item(11), 70), 2);
    assert!(f.logged("skill_dec 218"));
    assert_eq!(f.sent, vec![layouts::item_used(4, 11)]);
    // A used scroll: decrement and consume.
    let mut f = Fake::new();
    f.item(10, mode::STORED);
    f.item(11, mode::STORED).types = vec![ty::SCRO];
    f.k.item_skill = 218;
    assert_eq!(run(&mut f, &m32(0x27, &[10, 11])), Ok(res::OK));
    assert!(f.logged("skill_dec 218") && f.logged("consume_item 11"));
    // A used belt scroll: decrement and removal from the belt.
    let mut f = Fake::new();
    f.item(10, mode::STORED);
    f.item(11, mode::BELT).types = vec![ty::SCRO];
    f.k.item_skill = 218;
    assert_eq!(run(&mut f, &m32(0x27, &[10, 11])), Ok(res::OK));
    assert!(f.logged("remove_used 11"));
}

// ---------------------------------------------------------------

#[test]
fn pickup_scroll_and_book_into_a_tome() {
    // A scroll goes into the first tome of page 0 with its spell and room.
    let mut f = Fake::new();
    tome(&mut f, 11, 20);
    tome(&mut f, 12, 4);
    let s = f.item(10, mode::GROUND);
    s.types = vec![ty::SCRO];
    assert_eq!(tome_for(&f, me(), 10), Some(12));
    assert_eq!(pickup_auto(&mut f, me(), 10), Ok(Outcome::DONE));
    assert_eq!(f.stat(Owner::item(12), 70), 5);
    assert!(f.logged("free 10") && f.logged("book 1"));
    // B027: the ground scroll leaves its room with the delete notice (the
    // client's 0x0A) before it is freed.
    let mut f = Fake::new();
    tome(&mut f, 12, 4);
    f.item(10, mode::GROUND).types = vec![ty::SCRO];
    f.k.in_room = true;
    assert_eq!(pickup_auto(&mut f, me(), 10), Ok(Outcome::DONE));
    assert!(f.logged("room_delete 10") && f.logged("free_collision 10"));
    assert!(f.logged("remove_from_room 10") && f.logged("free 10"));
    // Another spell: no tome → not handled (placed on page 0).
    let mut f = Fake::new();
    tome(&mut f, 12, 4);
    f.items.get_mut(&12).unwrap().spell = 2;
    f.item(10, mode::GROUND).types = vec![ty::SCRO];
    assert_eq!(tome_for(&f, me(), 10), None);
    // A book onto a tome over the max: T := m, P keeps the rest, handled.
    let mut f = Fake::new();
    tome(&mut f, 12, 15);
    let b = f.item(10, mode::GROUND);
    b.types = vec![ty::BOOK];
    f.set_stat(Owner::item(10), 70, 8);
    assert_eq!(pickup_auto(&mut f, me(), 10), Ok(Outcome::DONE));
    assert_eq!(f.stat(Owner::item(12), 70), 20);
    assert_eq!(f.stat(Owner::item(10), 70), 3);
    assert!(f.logged("book 5") && !f.logged("free 10"));
    assert_eq!(f.it(10).mode, mode::GROUND);
    // A negative quantity → fatal.
    let mut f = Fake::new();
    tome(&mut f, 12, 4);
    f.item(10, mode::GROUND).types = vec![ty::BOOK];
    f.set_stat(Owner::item(10), 70, -1);
    assert_eq!(
        pickup_auto(&mut f, me(), 10),
        Err(MoveFatal::NegativeQuantity)
    );
}

#[test]
fn pickup_auto_stack_fills_in_order() {
    let mut f = Fake::new();
    for g in [11, 12] {
        let it = f.item(g, mode::STORED);
        it.max_stack = 10;
        f.set_stat(Owner::item(g), 70, 8);
    }
    let p = f.item(10, mode::GROUND);
    (p.stackable, p.autostack, p.max_stack) = (true, true, 10);
    f.set_stat(Owner::item(10), 70, 3);
    assert_eq!(pickup_auto(&mut f, me(), 10), Ok(Outcome::DONE));
    // 11 filled to 10, the last 1 onto 12; P freed.
    assert_eq!(f.stat(Owner::item(11), 70), 10);
    assert_eq!(f.stat(Owner::item(12), 70), 9);
    assert_eq!(f.stat(Owner::item(10), 70), 0);
    assert!(f.logged("free 10"));
    // No candidate left: not handled, earlier partial merges stay.
    let mut f = Fake::new();
    let it = f.item(11, mode::STORED);
    it.max_stack = 10;
    f.set_stat(Owner::item(11), 70, 8);
    let p = f.item(10, mode::GROUND);
    (p.stackable, p.autostack, p.max_stack) = (true, true, 10);
    f.set_stat(Owner::item(10), 70, 5);
    f.k.free = None;
    assert_eq!(pickup_auto(&mut f, me(), 10), Ok(Outcome::NOTHING));
    assert_eq!(f.stat(Owner::item(11), 70), 10);
    assert_eq!(f.stat(Owner::item(10), 70), 3);
}

#[test]
fn held_pairs_corpses_and_the_stop_at_p() {
    let quest_item = |f: &mut Fake, g: u32, m: u8, code: &[u8; 4]| {
        let it = f.item(g, m);
        it.quest = 3;
        it.code = *code;
    };
    // `hst` / `vip` and `qf2` / `qbr` are pairs.
    for (a, b) in [(b"hst ", b"vip "), (b"qbr ", b"qf2 ")] {
        let mut f = Fake::new();
        quest_item(&mut f, 11, mode::STORED, a);
        quest_item(&mut f, 10, mode::GROUND, b);
        assert!(held(&f, me(), 10));
        assert!(!can_pick(&f, me(), 10));
    }
    // `qf1` / `qhr` are no pair (both pair with `qf2` only).
    let mut f = Fake::new();
    quest_item(&mut f, 11, mode::STORED, b"qf1 ");
    quest_item(&mut f, 10, mode::GROUND, b"qhr ");
    assert!(!held(&f, me(), 10));
    // The walk stops at P itself.
    let mut f = Fake::new();
    quest_item(&mut f, 10, mode::STORED, b"hst ");
    quest_item(&mut f, 11, mode::STORED, b"msf ");
    assert!(!held(&f, me(), 10));
    assert!(held(&f, me(), 11));
}

// ---------------------------------------------------------------

#[test]
fn expiry_reader_takes_due_items_only() {
    let units = [(10, 0), (11, 500), (12, 1000), (13, 1001)];
    assert_eq!(expired_items(&units, 1000), vec![11, 12]);
    // Quest items (expiry 0) never expire.
    assert!(expired_items(&[(10, 0)], i32::MAX).is_empty());
}

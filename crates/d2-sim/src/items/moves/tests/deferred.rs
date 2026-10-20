//! Deferred messages, category and sizes.

use super::{me, Fake, P};
use crate::items::moves::deferred::{
    announce_item, category, dispatch, ground_update, item_unit_update, mark, owner_refresh,
    player_update,
};
use crate::items::moves::{layouts, mode, MoveFatal, MoveUnits, Owner};

fn one(f: &mut Fake, client: u32, g: u32) -> Vec<Vec<u8>> {
    dispatch(f, client, me(), g).unwrap()
}

#[test]
fn mark_sets_flags_list_and_refresh_bits() {
    let mut f = Fake::new();
    f.item(10, mode::STORED);
    mark(&mut f, me(), 10, 0x80);
    mark(&mut f, me(), 10, 0x2);
    assert_eq!(f.it(10).cmd, 0x82);
    assert_eq!(f.inv().update, vec![10]);
    assert_eq!(f.update_bits(me()), 3);
    assert!(f.logged("queue_update 0:1"));
    // A non-player owner gets bit 0 only.
    let m = Owner::monster(9);
    owner_refresh(&mut f, m);
    assert_eq!(f.update_bits(m), 1);
}

#[test]
fn first_matching_row_wins() {
    let mut f = Fake::new();
    f.item(10, mode::EQUIPPED).cmd = 0x40 | 0x8;
    f.unit(Owner::item(10)).x = 5;
    let m = one(&mut f, P, 10);
    assert_eq!(m.len(), 1);
    assert_eq!(&m[0][..2], &[0x9C, 0x01]);
    // Row 1 sets the item's x, y to 0 first.
    assert_eq!(f.pos(Owner::item(10)), (0, 0));

    // Row 5 (Equip, all): 0x9D action 6 with the owner fields.
    f.items.get_mut(&10).unwrap().cmd = 0x8;
    let m = one(&mut f, P, 10);
    assert_eq!(m, vec![layouts::item_owned(6, 0, 10, 0, P, &[]).unwrap()]);
    // Another client also gets it.
    assert_eq!(one(&mut f, 2, 10).len(), 1);
}

#[test]
fn owner_rows_skip_other_clients_and_end_the_walk() {
    let mut f = Fake::new();
    // Row 3 (owner) matches; row 20 would also match (item flag 1, mode 0).
    let it = f.item(10, mode::STORED);
    it.cmd = 0x2;
    it.iflags = 0x1;
    assert_eq!(one(&mut f, P, 10)[0][..2], [0x9C, 0x04]);
    assert!(one(&mut f, 2, 10).is_empty());
}

#[test]
fn item_flag_rows() {
    let mut f = Fake::new();
    f.item(10, mode::EQUIPPED).iflags = 0x100 | 0x200;
    // Row 18: 0x7D flag 0x100, state = the flag test's value; mode 1 → any client.
    assert_eq!(
        one(&mut f, 2, 10),
        vec![layouts::item_state(0, P, 10, 0x100, 0x100)]
    );
    f.items.get_mut(&10).unwrap().iflags = 0x200;
    assert_eq!(
        one(&mut f, 2, 10),
        vec![layouts::item_state(0, P, 10, 0x200, 0x200)]
    );
    // Mode 0: only the owner.
    f.items.get_mut(&10).unwrap().mode = mode::STORED;
    assert!(one(&mut f, 2, 10).is_empty());
    assert_eq!(one(&mut f, P, 10).len(), 1);
    // Row 20: 0x9D action 0x15 for item flag 1 in modes 0..2.
    f.items.get_mut(&10).unwrap().iflags = 0x1;
    assert_eq!(one(&mut f, P, 10)[0][..2], [0x9D, 0x15]);
    // Mode 3 → nothing; owner with 0x40000 → nothing; another client of
    // a mode-1 item still gets it.
    f.items.get_mut(&10).unwrap().mode = mode::GROUND;
    assert!(one(&mut f, P, 10).is_empty());
    let it = f.items.get_mut(&10).unwrap();
    it.mode = mode::EQUIPPED;
    it.iflags = 0x1 | 0x40000;
    assert!(one(&mut f, P, 10).is_empty());
    assert_eq!(one(&mut f, 2, 10).len(), 1);
}

#[test]
fn fillers_follow_their_parent() {
    let mut f = Fake::new();
    f.k.bits = vec![0xEE];
    let it = f.item(10, mode::STORED);
    it.cmd = 0x80;
    // Socketed: the fillers follow with flag argument 0x8.
    it.iflags = 0x800;
    it.fillers = vec![11, 12];
    f.item(11, mode::SOCKETED).page = 0xFF;
    f.item(12, mode::SOCKETED).page = 0xFF;
    let m = one(&mut f, P, 10);
    assert_eq!(m.len(), 3);
    assert_eq!(m[0], layouts::item_world(4, 0, 10, &[0xEE, 0, 0]).unwrap());
    assert_eq!(
        m[1],
        layouts::item_owned(0x13, 0, 11, 4, 10, &[0xEE, 0x8, 0xFF]).unwrap()
    );
    assert_eq!(m[2][..5], [0x9D, 0x13, 16, 0, 12]);
}

#[test]
fn player_update_walks_lists_then_relators() {
    let mut f = Fake::new();
    f.item(10, mode::STORED).cmd = 0x80;
    f.item(11, mode::EQUIPPED).cmd = 0x8;
    f.item(12, mode::SOCKETED).cmd = 0x10;
    f.inv_mut().update = vec![10, 99, 11];
    // Item 11 has its own update list with item 12.
    f.invs.entry(Owner::item(11)).or_default().update = vec![12];
    f.set_update_bits(Owner::item(11), 1);
    // Bit 0 clear: nothing.
    assert!(player_update(&mut f, P, P).unwrap().is_empty());
    f.set_update_bits(me(), 3);
    let m = player_update(&mut f, P, P).unwrap();
    let heads: Vec<[u8; 2]> = m.iter().map(|b| [b[0], b[1]]).collect();
    assert_eq!(
        heads,
        vec![[0x9C, 4], [0x9D, 6], [0x9D, 8], [0x47, 0], [0x48, 0]]
    );
    // Item 12's owner fields name item 11.
    assert_eq!(m[2][8], 4);
    assert_eq!(m[3], layouts::relator1(0, P));
    assert_eq!(m[4], layouts::relator2(0, 0, P));
}

#[test]
fn ground_items() {
    let mut f = Fake::new();
    f.item(10, mode::GROUND);
    assert_eq!(ground_update(&f, 10).unwrap().unwrap()[..2], [0x9C, 3]);
    f.unit(Owner::item(10)).uflags = 0x1000;
    assert_eq!(ground_update(&f, 10).unwrap().unwrap()[..2], [0x9C, 2]);
    f.unit(Owner::item(10)).uflags = 0x1010;
    assert_eq!(ground_update(&f, 10).unwrap(), None);
    f.item(11, mode::STORED);
    assert_eq!(ground_update(&f, 11).unwrap(), None);
}

#[test]
fn item_unit_update_announces_once_then_updates() {
    let mut f = Fake::new();
    f.item(10, mode::GROUND);
    // Part 1: flag 0x10 (not yet announced) → unit-add message; mode 3
    // with 0x1000 → action 2, otherwise action 0 (also without bit 0).
    f.unit(Owner::item(10)).uflags = 0x10;
    assert_eq!(item_unit_update(&f, 10).unwrap().unwrap()[..2], [0x9C, 0]);
    f.unit(Owner::item(10)).uflags = 0x1011;
    assert_eq!(item_unit_update(&f, 10).unwrap().unwrap()[..2], [0x9C, 2]);
    assert_eq!(announce_item(&f, 10).unwrap()[..2], [0x9C, 2]);
    f.item(11, mode::STORED);
    f.unit(Owner::item(11)).uflags = 0x1010;
    assert_eq!(item_unit_update(&f, 11).unwrap().unwrap()[..2], [0x9C, 0]);
    // Part 2 (flag 0x10 cleared by the room clean-up): only when changed
    // (bit 0), then 0x9C action 2 or 3; a stored item sends nothing.
    f.unit(Owner::item(10)).uflags = 0x1000;
    assert_eq!(item_unit_update(&f, 10).unwrap(), None);
    f.unit(Owner::item(10)).uflags = 0x1001;
    assert_eq!(item_unit_update(&f, 10).unwrap().unwrap()[..2], [0x9C, 2]);
    f.unit(Owner::item(10)).uflags = 0x1;
    assert_eq!(item_unit_update(&f, 10).unwrap().unwrap()[..2], [0x9C, 3]);
    f.unit(Owner::item(11)).uflags = 0x1;
    assert_eq!(item_unit_update(&f, 11).unwrap(), None);
}

#[test]
fn category_rule() {
    let mut f = Fake::new();
    f.unit(me()).class = 4; // barbarian
    f.item(10, mode::EQUIPPED).component = 5;
    f.item(11, mode::EQUIPPED).component = 5;
    f.items.get_mut(&10).unwrap().body_loc = 4;
    f.items.get_mut(&11).unwrap().body_loc = 5;
    f.inv_mut().body.insert(4, 10);
    f.inv_mut().body.insert(5, 11);
    f.inv_mut().weapon = Some(11);
    assert_eq!(category(&f, 10), 6);
    // The weapon in use keeps its component.
    assert_eq!(category(&f, 11), 5);
    // Other classes, a broken hand, a two-handed hand: component.
    f.unit(me()).class = 1;
    assert_eq!(category(&f, 10), 5);
    f.unit(me()).class = 6;
    assert_eq!(category(&f, 10), 6);
    f.items.get_mut(&11).unwrap().iflags = 0x100;
    assert_eq!(category(&f, 10), 5);
    f.items.get_mut(&11).unwrap().iflags = 0;
    f.items.get_mut(&11).unwrap().two_handed = true;
    assert_eq!(category(&f, 10), 5);
    f.items.get_mut(&11).unwrap().two_handed = false;
    f.items.get_mut(&10).unwrap().body_loc = 3;
    assert_eq!(category(&f, 10), 5);
    // Monster classes 0x1A1 / 0x1A2.
    let m = Owner::monster(50);
    f.unit(m).class = 0x1A2;
    f.invs.entry(m).or_default().body.insert(4, 10);
    f.invs.entry(m).or_default().body.insert(5, 11);
    let it = f.items.get_mut(&10).unwrap();
    it.body_loc = 4;
    it.owner = Some(m);
    assert_eq!(category(&f, 10), 6);
}

#[test]
fn item_message_size_limit() {
    assert!(layouts::item_world(1, 0, 1, &[0; 0xF4]).is_ok());
    assert_eq!(
        layouts::item_world(1, 0, 1, &[0; 0xF5]),
        Err(MoveFatal::MessageSize(0xFD))
    );
    assert_eq!(
        layouts::item_owned(1, 0, 1, 0, 1, &[0; 0xF0]),
        Err(MoveFatal::MessageSize(0xFD))
    );
}

//! Pickup, drop, gold.

use super::{me, FItem, Fake};
use crate::items::moves::ground::{
    can_pick, corpse_fill, cube_spill, drop_cursor_item, drop_spot, gold_pickup, gold_piles,
    ground_expiry, pickup_auto, refused_pickup,
};
use crate::items::moves::{layouts, mode, ty, InventoryOps, MoveUnits, Outcome, Owner};

fn ground(f: &mut Fake, g: u32) -> &mut FItem {
    f.item(g, mode::GROUND)
}

#[test]
fn auto_pickup_gates_and_refusal() {
    let mut f = Fake::new();
    f.item(9, mode::CURSOR);
    ground(&mut f, 10);
    assert_eq!(pickup_auto(&mut f, me(), 10), Ok(Outcome::NOTHING));
    f.inv_mut().cursor = None;
    f.items.remove(&9);
    assert_eq!(pickup_auto(&mut f, me(), 11), Ok(Outcome::REFUSED));
    f.item(12, mode::STORED);
    assert_eq!(pickup_auto(&mut f, me(), 12), Ok(Outcome::REFUSED));
    // Can-pick fails (a quest item already held): refused pickup, sound 0x13.
    f.items.get_mut(&12).unwrap().quest = 7;
    f.items.get_mut(&10).unwrap().quest = 7;
    f.items.get_mut(&12).unwrap().code = *b"xxx ";
    assert_eq!(pickup_auto(&mut f, me(), 10), Ok(Outcome::NOTHING));
    let it = f.it(10);
    assert_eq!((it.mode, it.page), (mode::GROUND, 0xFF));
    assert_eq!(f.unit_flags(Owner::item(10)), 0x1000);
    assert!(f.logged("room_delete 10") && f.logged("sound 0x13"));
    // Direct refused pickup with another sound.
    refused_pickup(&mut f, me(), 10, 0x17);
    assert!(f.logged("sound 0x17"));
}

#[test]
fn auto_pickup_gold_specials_equip() {
    let mut f = Fake::new();
    ground(&mut f, 10).types = vec![ty::GOLD];
    f.set_stat(Owner::item(10), 14, 300);
    assert_eq!(pickup_auto(&mut f, me(), 10), Ok(Outcome::DONE));
    assert_eq!(f.stat(me(), 14), 300);
    assert!(f.logged("pickup_sound") && f.logged("free 10"));

    // An auto-stack item merges into a page-0 stack: handled.
    let mut f = Fake::new();
    let p = ground(&mut f, 10);
    (p.stackable, p.autostack, p.max_stack) = (true, true, 20);
    f.item(11, mode::STORED).max_stack = 20;
    f.set_stat(Owner::item(10), 70, 5);
    f.set_stat(Owner::item(11), 70, 3);
    assert_eq!(pickup_auto(&mut f, me(), 10), Ok(Outcome::DONE));
    assert_eq!(f.it(10).mode, mode::GROUND);
    assert_eq!(f.stat(Owner::item(11), 70), 8);
    assert!(f.logged("free 10"));

    let mut f = Fake::new();
    ground(&mut f, 10);
    f.k.auto_equip = Some(3);
    f.k.equip_check = 5;
    assert_eq!(pickup_auto(&mut f, me(), 10), Ok(Outcome::REFUSED));
    f.k.equip_check = 1;
    assert_eq!(pickup_auto(&mut f, me(), 10), Ok(Outcome::DONE));
    assert!(f.logged("equip_picked 10") && f.logged("quest_picked 10"));
}

#[test]
fn auto_pickup_to_belt() {
    let mut f = Fake::new();
    ground(&mut f, 10).beltable = true;
    f.unit(Owner::item(10)).uflags = 0x200_0002;
    f.k.belt_slot = Some(1);
    assert_eq!(pickup_auto(&mut f, me(), 10), Ok(Outcome::DONE));
    let it = f.it(10);
    assert_eq!((it.mode, it.page, it.cmd), (mode::BELT, 0xFF, 0x2000));
    assert_eq!(f.unit_flags(Owner::item(10)), 0);
    assert!(f.logged("belt_place 10 1") && f.logged("link 10 kind 2"));
    assert_eq!(f.inv().update, vec![10]);
    // Belt placement fails → page 0.
    let mut f = Fake::new();
    ground(&mut f, 10).beltable = true;
    f.k.belt_slot = None;
    assert_eq!(pickup_auto(&mut f, me(), 10), Ok(Outcome::DONE));
    assert_eq!(f.it(10).cmd, 0x80);
}

#[test]
fn auto_pickup_no_room() {
    let mut f = Fake::new();
    ground(&mut f, 10);
    f.k.free = None;
    assert_eq!(pickup_auto(&mut f, me(), 10), Ok(Outcome::NOTHING));
    assert!(f.logged("sound 0x17"));
    assert_eq!(f.it(10).mode, mode::GROUND);
    assert_eq!(f.unit_flags(Owner::item(10)), 0x1000);
}

#[test]
fn can_pick_uniques_and_inventory() {
    let mut f = Fake::new();
    let it = ground(&mut f, 10);
    it.quality = 7;
    it.file_index = 4;
    it.carry_one = true;
    assert!(can_pick(&f, me(), 10));
    let h = f.item(11, mode::STORED);
    h.quality = 7;
    h.file_index = 4;
    h.carry_one = true;
    assert!(!can_pick(&f, me(), 10));
    // Held on page 1 does not count.
    f.items.get_mut(&11).unwrap().page = 1;
    assert!(can_pick(&f, me(), 10));
    f.items.get_mut(&11).unwrap().page = 0;
    f.items.get_mut(&11).unwrap().file_index = 5;
    assert!(can_pick(&f, me(), 10));
    // No inventory → no.
    f.invs.remove(&me());
    assert!(!can_pick(&f, me(), 10));
}

#[test]
fn can_pick_quest_items() {
    let check = |code: &[u8; 4], quest: u8, flags: &[(u8, u8)]| {
        let mut f = Fake::new();
        let it = ground(&mut f, 10);
        it.code = *code;
        it.quest = quest;
        f.k.quest_flags = flags.iter().copied().collect();
        can_pick(&f, me(), 10)
    };
    assert!(check(b"ass ", 1, &[]));
    assert!(!check(b"ass ", 1, &[(9, 5)]));
    assert!(!check(b"j34 ", 1, &[(20, 0)]));
    assert!(!check(b"xyz ", 1, &[(20, 5)]));
    assert!(!check(b"g33 ", 1, &[(19, 8)]));
    assert!(!check(b"tr2 ", 1, &[]));
    assert!(check(b"tr2 ", 1, &[(37, 8)]));
    assert!(!check(b"tr2 ", 1, &[(37, 8), (37, 7)]));
    assert!(!check(b"hdm ", 10, &[(10, 0)]));
    assert!(check(b"box ", 10, &[(10, 0)]));
    assert!(!check(b"xxx ", 5, &[(4, 0)]));
    assert!(check(b"leg ", 5, &[(4, 0)]));
    assert!(!check(b"xxx ", 4, &[(3, 0)]));
    assert!(!check(b"xxx ", 0x11, &[(0x12, 0)]));
    assert!(!check(b"xxx ", 0x12, &[(0x13, 0)]));
    assert!(!check(b"xxx ", 0x19, &[(0x1B, 0)]));
    assert!(check(b"xxx ", 0x19, &[(0x1A, 0)]));
    // Held equivalent pair: a `g34` held blocks a `j34`.
    let mut f = Fake::new();
    let it = ground(&mut f, 10);
    it.code = *b"j34 ";
    it.quest = 3;
    let h = f.item(11, mode::STORED);
    h.code = *b"g34 ";
    h.quest = 3;
    assert!(!can_pick(&f, me(), 10));
    f.items.get_mut(&11).unwrap().quest = 2;
    assert!(can_pick(&f, me(), 10));
}

#[test]
fn drop_places_on_ground() {
    let mut f = Fake::new();
    let it = f.item(10, mode::CURSOR);
    it.filled = true;
    it.iflags = 0x4000;
    it.quest = 0;
    f.k.room_at = true;
    drop_cursor_item(&mut f, me(), 10).unwrap();
    // Start (x + 2, y + 3) when a room exists there; last argument 1.
    assert_eq!(f.spot_calls.borrow()[0], ((102, 103), (100, 100), 1));
    let it = f.it(10);
    assert_eq!((it.mode, it.page, it.iflags), (mode::GROUND, 0xFF, 0x1));
    assert_eq!(it.expiry, 1000 + 15000);
    assert_eq!(f.pos(Owner::item(10)), (10, 20));
    assert_eq!(f.unit_flags(Owner::item(10)), 0x1002 | 0x200_0000);
    assert_eq!(f.inv().cursor, None);
    assert!(f.logged("add_to_room 10 7") && f.logged("quest_dropped 10"));
    // The ground update then announces it as "dropped" (action 2).
    let m = crate::items::moves::ground_update(&f, 10).unwrap().unwrap();
    assert_eq!(m, layouts::item_world(2, 0, 10, &[]).unwrap());
    // Not on the cursor: nothing.
    let mut f = Fake::new();
    f.item(10, mode::STORED);
    drop_cursor_item(&mut f, me(), 10).unwrap();
    assert_eq!(f.it(10).mode, mode::STORED);
    assert!(f.spot_calls.borrow().is_empty());
    // No room at the start: (x, y).
    let f = Fake::new();
    drop_spot(&f, me(), 0);
    assert_eq!(f.spot_calls.borrow()[0], ((100, 100), (100, 100), 0));
}

#[test]
fn ground_expiry_values() {
    let exp = |q: u8, quest: u8, types: Vec<u16>, gold: i32, filler: bool| {
        let mut f = Fake::new();
        let it = ground(&mut f, 10);
        it.quality = q;
        it.quest = quest;
        it.types = types;
        it.filler = filler;
        f.set_stat(Owner::item(10), 14, gold);
        ground_expiry(&f, 10)
    };
    assert_eq!(exp(2, 0, vec![], 0, false), 16000);
    assert_eq!(exp(2, 1, vec![], 0, false), 0);
    assert_eq!(exp(7, 1, vec![], 0, false), 0);
    assert_eq!(exp(4, 0, vec![], 0, false), 31000);
    for q in 5..=9 {
        assert_eq!(exp(q, 0, vec![], 0, false), 46000);
    }
    assert_eq!(exp(2, 0, vec![ty::GOLD], 10000, false), 16000);
    assert_eq!(exp(2, 0, vec![ty::GOLD], 10001, false), 46000);
    assert_eq!(exp(2, 0, vec![], 0, true), 31000);
    assert_eq!(exp(4, 0, vec![], 0, true), 31000);
}

#[test]
fn cube_spill_moves_contents() {
    let mut f = Fake::new();
    f.k.bits = vec![0xEE];
    f.item(10, mode::CURSOR).code = *b"box ";
    f.item(11, mode::STORED).page = 3;
    f.item(12, mode::STORED).page = 0;
    f.item(13, mode::STORED).page = 3;
    f.unit(Owner::item(11)).x = 1;
    drop_cursor_item(&mut f, me(), 10).unwrap();
    // One 0x9D action 5 per cube item, flag 0x20, page shown 3.
    assert_eq!(f.sent.len(), 2);
    assert_eq!(f.sent[0][..8], [0x9D, 5, 16, 0, 11, 0, 0, 0]);
    assert_eq!(f.sent[0][13..], [0xEE, 0x20, 3]);
    assert_eq!(f.sent[1][4], 13);
    assert!(f.logged("room_change 11 1,0"));
    assert!(f.logged("place_in_page 11 0,0 find=true send=true"));
    for g in [11, 13] {
        assert_eq!((f.it(g).mode, f.it(g).page), (mode::STORED, 0));
    }
    // Placement fails: page 0xFF, dropped next to the player.
    let mut f = Fake::new();
    f.item(11, mode::STORED).page = 3;
    f.k.place_ok = false;
    cube_spill(&mut f, me()).unwrap();
    assert_eq!((f.it(11).mode, f.it(11).page), (mode::GROUND, 0xFF));
    assert!(!f.inv().list.contains(&11));
}

#[test]
fn gold_pickup_g2() {
    let mut f = Fake::new();
    f.set_stat(me(), 14, 9500);
    ground(&mut f, 10).types = vec![ty::GOLD];
    f.set_stat(Owner::item(10), 14, 1000);
    gold_pickup(&mut f, me(), 10);
    assert_eq!(f.stat(me(), 14), 10000);
    assert!(f.logged("rest_pile 500") && f.logged("free 10"));
    // A pile owned by a player goes through.
    let mut f = Fake::new();
    ground(&mut f, 10);
    f.set_stat(Owner::item(10), 14, 100);
    f.k.pile_owner = Some(me());
    gold_pickup(&mut f, me(), 10);
    assert_eq!(f.stat(me(), 14), 0);
    assert!(!f.logged("rest_pile 0"));
}

#[test]
fn gold_piles_caps() {
    let mut f = Fake::new();
    let piles = gold_piles(&mut f, me(), 5, 32);
    assert_eq!(piles, vec![500]);
    assert_eq!(f.stat(Owner::item(500), 14), 5);
    assert_eq!(f.it(500).mode, mode::GROUND);
    assert_eq!(f.spot_calls.borrow()[0].2, 0);
    // Above 2,000,000,000: two piles; the max count stops it.
    let mut f = Fake::new();
    let piles = gold_piles(&mut f, me(), i32::MAX, 32);
    assert_eq!(piles.len(), 2);
    assert_eq!(f.stat(Owner::item(500), 14), 2_000_000_000);
    assert_eq!(f.stat(Owner::item(501), 14), i32::MAX - 2_000_000_000);
    let mut f = Fake::new();
    assert_eq!(gold_piles(&mut f, me(), i32::MAX, 1).len(), 1);
    // No spot: no pile.
    let mut f = Fake::new();
    f.k.spot = None;
    assert!(gold_piles(&mut f, me(), 5, 32).is_empty());
    assert!(InventoryOps::items(&f, me()).is_empty());
}

#[test]
fn gold_messages_g3() {
    assert_eq!(layouts::gold(120, 100), Some(vec![0x19, 0x14]));
    assert_eq!(layouts::gold(400, 100), Some(vec![0x1E, 0x0E, 0x90, 0x01]));
    assert_eq!(
        layouts::gold(70000, 100),
        Some(vec![0x1F, 0x0E, 0x70, 0x11, 0x01, 0x00])
    );
    assert_eq!(layouts::gold(354, 100), Some(vec![0x19, 254]));
    assert_eq!(layouts::gold(100, 120), Some(vec![0x1D, 0x0E, 100]));
    assert_eq!(
        layouts::gold(0xFFFF, 0),
        Some(vec![0x1F, 0x0E, 0xFF, 0xFF, 0, 0])
    );
    assert_eq!(layouts::gold(5, 5), None);
}

#[test]
fn corpse_fill_drops_an_unplaced_copy_near_the_player() {
    let mut f = Fake::new();
    f.item(10, mode::CURSOR);
    f.item(11, mode::EQUIPPED);
    f.inv_mut().body.insert(4, 11);
    let c = Owner::player(2);
    f.invs.insert(c, Default::default());
    f.k.room_at = true;
    f.k.place_ok = false;
    assert!(!corpse_fill(&mut f, me(), c).unwrap());
    // Both copies (500 cursor, 501 body) go to the ground near P with the
    // cursor drop's search, never onto C's update list.
    assert!(f.logged("copy 10 -> 500") && f.logged("copy 11 -> 501"));
    assert_eq!(
        f.spot_calls.borrow().as_slice(),
        &[((102, 103), (100, 100), 1); 2]
    );
    for k in [500, 501] {
        assert_eq!(f.it(k).mode, mode::GROUND);
        assert!(f.logged(&format!("add_to_room {k} 7")));
    }
    assert!(f.invs[&c].update.is_empty());
    // Placed on C: no ground search.
    let mut f = Fake::new();
    f.item(10, mode::CURSOR);
    f.invs.insert(c, Default::default());
    assert!(corpse_fill(&mut f, me(), c).unwrap());
    assert!(f.spot_calls.borrow().is_empty());
    assert_eq!(f.invs[&c].update, vec![500]);
}

#[test]
fn failed_auto_equip_keeps_the_pickup_in_its_room() {
    let mut f = Fake::new();
    ground(&mut f, 10);
    f.k.auto_equip = Some(4);
    f.k.equip_check = 1;
    f.k.equip_picked = false;
    f.k.in_room = true;
    assert_eq!(pickup_auto(&mut f, me(), 10), Ok(Outcome::NOTHING));
    assert_eq!(f.it(10).mode, mode::GROUND);
    assert_eq!(f.inv().cursor, None);
    assert!(!f.logged("room_delete 10"));
    assert!(!f.logged("remove_from_room 10"));
    assert!(f.items.contains_key(&10));
}

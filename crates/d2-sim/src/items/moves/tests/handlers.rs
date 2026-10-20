//! Intent handlers: validation order and result codes.

use super::{me, Fake, P};
use crate::items::moves::{
    handle, iflag, mode, res, ty, InventoryOps, MoveFatal, MoveUnits, Owner, HANDLED,
};

/// Message `id` followed by u32 fields.
fn m32(id: u8, fields: &[u32]) -> Vec<u8> {
    let mut b = vec![id];
    for v in fields {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}

/// Message `id` with an item u32 and a body location u8 (+3 padding).
fn mloc(id: u8, item: u32, loc: u8) -> Vec<u8> {
    let mut b = m32(id, &[item]);
    b.extend_from_slice(&[loc, 0, 0, 0]);
    b
}

fn m16(id: u8, v: u16) -> Vec<u8> {
    let mut b = vec![id];
    b.extend_from_slice(&v.to_le_bytes());
    b
}

fn run(f: &mut Fake, msg: &[u8]) -> u32 {
    handle(f, P, msg).expect("handled id").expect("no fatal")
}

#[test]
fn size_check_first_and_foreign_ids() {
    let mut f = Fake::new();
    for &(id, size) in &HANDLED {
        let mut b = vec![id; size + 1];
        assert_eq!(run(&mut f, &b), res::REFUSED, "id {id:#x} long");
        b.truncate(size - 1);
        assert_eq!(run(&mut f, &b), res::REFUSED, "id {id:#x} short");
    }
    assert!(handle(&mut f, P, &[0x4C, 0, 0, 0, 0]).is_none());
    assert!(handle(&mut f, P, &[]).is_none());
}

// ---- 0x16

#[test]
fn pick_item_validation() {
    let mut f = Fake::new();
    assert_eq!(run(&mut f, &m32(0x16, &[6, 10, 0])), res::BAD);
    assert_eq!(run(&mut f, &m32(0x16, &[0, P, 0])), res::REFUSED);
    // Missing item, item not on the ground, too far.
    assert_eq!(run(&mut f, &m32(0x16, &[4, 10, 0])), res::RANGE);
    f.item(10, mode::STORED);
    assert_eq!(run(&mut f, &m32(0x16, &[4, 10, 0])), res::RANGE);
    f.item(11, mode::GROUND);
    f.k.distance = 51;
    assert_eq!(run(&mut f, &m32(0x16, &[4, 11, 0])), res::RANGE);
    // Distance 5..50 or a collision: walk to it, 0.
    f.k.distance = 50;
    assert_eq!(run(&mut f, &m32(0x16, &[4, 11, 1])), res::OK);
    assert!(f.logged("walk 11 true"));
    f.k.distance = 4;
    f.k.collides = true;
    assert_eq!(run(&mut f, &m32(0x16, &[4, 11, 0])), res::OK);
    assert!(f.logged("walk 11 false"));
    assert_eq!(f.it(11).mode, mode::GROUND);
}

#[test]
fn pick_item_to_cursor_only_when_asked() {
    let mut f = Fake::new();
    f.item(11, mode::GROUND);
    f.unit(Owner::item(11)).uflags = 0x2 | 0x200_0000;
    assert_eq!(run(&mut f, &m32(0x16, &[4, 11, 1])), res::OK);
    assert_eq!(f.inv().cursor, Some(11));
    let it = f.it(11);
    assert_eq!((it.mode, it.page, it.cmd), (mode::CURSOR, 0xFF, 0x40));
    assert_eq!(f.unit_flags(Owner::item(11)), 0);
    assert_eq!(f.inv().update, vec![11]);
    let order: Vec<&str> = f
        .log
        .iter()
        .map(String::as_str)
        .filter(|l| {
            [
                "room_delete",
                "free_collision",
                "remove_from_room",
                "quest_picked",
                "pickup_sound",
            ]
            .iter()
            .any(|p| l.starts_with(p))
        })
        .collect();
    assert_eq!(
        order,
        vec![
            "room_delete 11",
            "free_collision 11",
            "remove_from_room 11",
            "quest_picked 11",
            "pickup_sound"
        ]
    );
    // Cursor flag 0 takes the auto path: page 0.
    let mut f = Fake::new();
    f.item(11, mode::GROUND);
    f.k.free = Some((3, 2));
    assert_eq!(run(&mut f, &m32(0x16, &[4, 11, 0])), res::OK);
    assert_eq!(
        (f.it(11).mode, f.it(11).page, f.it(11).cmd),
        (mode::STORED, 0, 0x80)
    );
    assert!(f.logged("place_at 11 p0 3,2"));
    assert_eq!(f.inv().cursor, None);
}

#[test]
fn pick_other_unit_types() {
    let mut f = Fake::new();
    // NPC, object: the pending seams answer 0. Type 3 (missile) → 1.
    for t in [1, 2] {
        assert_eq!(run(&mut f, &m32(0x16, &[t, 77, 0])), res::OK);
    }
    assert_eq!(run(&mut f, &m32(0x16, &[3, 77, 0])), res::RANGE);
    // Types 0 and 5: a missing unit → 1.
    assert_eq!(run(&mut f, &m32(0x16, &[0, 77, 0])), res::RANGE);
    assert_eq!(run(&mut f, &m32(0x16, &[5, 77, 0])), res::RANGE);
}

#[test]
fn pick_player_and_tile() {
    let other = Owner::player(77);
    let tile = Owner { ty: 5, guid: 78 };
    let mut f = Fake::new();
    f.unit(other);
    f.unit(tile);
    // Player: > 50 → 1; > 8 → walk; dead and not trading → corpse
    // pickup; else the player interaction.
    f.k.distance = 51;
    assert_eq!(run(&mut f, &m32(0x16, &[0, 77, 1])), res::RANGE);
    f.k.distance = 9;
    assert_eq!(run(&mut f, &m32(0x16, &[0, 77, 1])), res::OK);
    assert!(f.logged("walk_unit 0:77 true"));
    f.k.distance = 8;
    assert_eq!(run(&mut f, &m32(0x16, &[0, 77, 0])), res::OK);
    assert!(f.logged("interact 77"));
    f.unit(other).mode = 17;
    f.k.trading = true;
    assert_eq!(run(&mut f, &m32(0x16, &[0, 77, 0])), res::OK);
    assert!(!f.logged("corpse 77"));
    f.k.trading = false;
    assert_eq!(run(&mut f, &m32(0x16, &[0, 77, 0])), res::OK);
    assert!(f.logged("corpse 77"));
    // Tile: > 50 → 1; < 5 → warp; else walk.
    f.k.distance = 51;
    assert_eq!(run(&mut f, &m32(0x16, &[5, 78, 0])), res::RANGE);
    f.k.distance = 4;
    assert_eq!(run(&mut f, &m32(0x16, &[5, 78, 0])), res::OK);
    assert!(f.logged("warp 78"));
    f.k.distance = 5;
    assert_eq!(run(&mut f, &m32(0x16, &[5, 78, 0])), res::OK);
    assert!(f.logged("walk_unit 5:78 false"));
}

// ---- 0x17

#[test]
fn drop_item() {
    let mut f = Fake::new();
    assert_eq!(run(&mut f, &m32(0x17, &[10])), 1);
    f.item(10, mode::CURSOR);
    f.k.busy = true;
    f.k.trading = true;
    assert_eq!(run(&mut f, &m32(0x17, &[10])), res::REFUSED);
    f.k.trading = false;
    // No ground spot: result 0, the item stays on the cursor.
    f.k.spot = None;
    assert_eq!(run(&mut f, &m32(0x17, &[10])), res::OK);
    assert_eq!(f.inv().cursor, Some(10));
    f.k.spot = super::Knobs::default().spot;
    assert_eq!(run(&mut f, &m32(0x17, &[10])), res::OK);
    assert_eq!(f.inv().cursor, None);
    assert_eq!(f.it(10).mode, mode::GROUND);
}

// ---- 0x18

fn insert(f: &mut Fake, page: u32) -> u32 {
    run(f, &m32(0x18, &[10, 4, 1, page]))
}

#[test]
fn insert_item_pages() {
    let mut f = Fake::new();
    assert_eq!(insert(&mut f, 0), 1);
    f.item(10, mode::CURSOR);
    assert_eq!(insert(&mut f, 1), res::BAD);
    assert_eq!(insert(&mut f, 5), res::BAD);
    f.k.in_town = false;
    assert_eq!(insert(&mut f, 4), res::REFUSED);
    assert_eq!(insert(&mut f, 2), res::REFUSED);
    f.k.place_ok = false;
    assert_eq!(insert(&mut f, 0), res::REFUSED);
    assert_eq!(f.it(10).page, 0);
    f.k.place_ok = true;
    // The cube page needs no open cube; the item is placed at (x, y), sent.
    assert_eq!(insert(&mut f, 3), res::OK);
    assert_eq!(f.it(10).page, 3);
    assert!(f.logged("place_in_page 10 4,1 find=false send=true"));
    assert!(f.logged("targeting_reset"));
    let mut f = Fake::new();
    f.item(10, mode::CURSOR);
    assert_eq!(insert(&mut f, 4), res::OK);
    let mut f = Fake::new();
    f.item(10, mode::CURSOR);
    f.k.trading = true;
    assert_eq!(insert(&mut f, 2), res::OK);
}

#[test]
fn insert_item_non_player() {
    use crate::items::moves::handlers::insert_item;
    let mut f = Fake::new();
    let m = Owner::monster(50);
    f.invs.entry(m).or_default().cursor = Some(10);
    f.item(10, mode::CURSOR);
    f.inv_mut().cursor = None;
    f.invs.get_mut(&m).unwrap().cursor = Some(10);
    assert_eq!(insert_item(&mut f, m, 10, 0, 0, 5), res::BAD);
    // Page 1 is allowed for a non-player owner.
    assert_eq!(insert_item(&mut f, m, 10, 0, 0, 1), res::OK);
}

// ---- 0x19

#[test]
fn remove_from_buffer() {
    let mut f = Fake::new();
    assert_eq!(run(&mut f, &m32(0x19, &[10])), 1);
    f.item(10, mode::STORED);
    f.item(11, mode::CURSOR);
    assert_eq!(run(&mut f, &m32(0x19, &[10])), res::BAD);
    // "Can't do that": 0x5A, 40 bytes.
    assert_eq!(
        f.sent.last().map(|m| (m[0], m[1], m[2], m.len())),
        Some((0x5A, 0x0E, 1, 40))
    );
    f.inv_mut().cursor = None;
    f.items.get_mut(&10).unwrap().page = 1;
    assert_eq!(run(&mut f, &m32(0x19, &[10])), res::REFUSED);
    f.items.get_mut(&10).unwrap().page = 4;
    f.k.gate = false;
    assert_eq!(run(&mut f, &m32(0x19, &[10])), res::OK);
    f.k.gate = true;
    // Not busy and page ≠ 0 → refused.
    assert_eq!(run(&mut f, &m32(0x19, &[10])), res::REFUSED);
    // Busy (an interaction): lifted.
    f.k.busy = true;
    f.items.get_mut(&10).unwrap().filled = true;
    f.items.get_mut(&10).unwrap().iflags = 0x4000;
    f.unit(Owner::item(10)).x = 3;
    assert_eq!(run(&mut f, &m32(0x19, &[10])), res::OK);
    let it = f.it(10);
    assert_eq!(
        (it.mode, it.page, it.stored_page, it.cmd, it.iflags),
        (4, 0xFF, 4, 0x4, 0x1)
    );
    assert_eq!(f.inv().cursor, Some(10));
    assert!(f.logged("room_change 10 3,0"));
    assert!(f.logged("stat_refresh_unlink 0:1 1"));
}

// ---- 0x1A

#[test]
fn equip_item() {
    let mut f = Fake::new();
    f.item(10, mode::CURSOR);
    assert_eq!(run(&mut f, &mloc(0x1A, 10, 0)), res::BAD);
    assert_eq!(run(&mut f, &mloc(0x1A, 10, 11)), res::BAD);
    assert_eq!(run(&mut f, &mloc(0x1A, 10, 3)), res::OK);
    assert!(f.logged("equip_from_cursor 10 3 false"));
    f.k.equip_from_cursor = (false, true);
    assert_eq!(run(&mut f, &mloc(0x1A, 10, 3)), res::REFUSED);
    f.k.equip_from_cursor = (false, false);
    assert_eq!(run(&mut f, &mloc(0x1A, 10, 3)), res::OK);
}

// ---- 0x1B

#[test]
fn swap_two_handed() {
    let mut f = Fake::new();
    f.item(10, mode::CURSOR);
    assert_eq!(run(&mut f, &mloc(0x1B, 10, 11)), res::BAD);
    assert_eq!(run(&mut f, &mloc(0x1B, 10, 3)), res::REFUSED);
    // Other hand empty → 3.
    assert_eq!(run(&mut f, &mloc(0x1B, 10, 4)), res::REFUSED);
    f.item(11, mode::EQUIPPED).body_loc = 5;
    f.inv_mut().body.insert(5, 11);
    f.k.equip_check = 1;
    assert_eq!(run(&mut f, &mloc(0x1B, 10, 4)), res::REFUSED);
    f.k.equip_check = 2;
    f.k.requirements = false;
    assert_eq!(run(&mut f, &mloc(0x1B, 10, 4)), res::OK);
    assert!(f.logged("requirement_sound"));
    assert_eq!(f.inv().cursor, Some(10));
    f.k.requirements = true;
    assert_eq!(run(&mut f, &mloc(0x1B, 10, 4)), res::OK);
    // X stays the cursor item (the handler has no "cursor := none").
    assert_eq!(f.inv().cursor, Some(11));
    assert_eq!(f.it(11).mode, mode::CURSOR);
    let n = f.it(10);
    assert_eq!(
        (n.mode, n.body_loc, n.page, n.cmd, n.iflags),
        (1, 4, 0xFF, 0x10000, 0x1)
    );
    assert_eq!(f.inv().body.get(&4), Some(&10));
    assert_eq!(f.inv().body.get(&5), None);
    assert!(f.logged("link 10 kind 3"));
    assert!(f.logged("stat_link 10"));
}

// ---- 0x1C

#[test]
fn remove_body_item() {
    let mut f = Fake::new();
    assert_eq!(run(&mut f, &m16(0x1C, 11)), res::BAD);
    assert_eq!(run(&mut f, &m16(0x1C, 0)), res::BAD);
    // Empty location → 0.
    assert_eq!(run(&mut f, &m16(0x1C, 1)), res::OK);
    let it = f.item(10, mode::EQUIPPED);
    it.body_loc = 8;
    it.types = vec![ty::BELT];
    f.inv_mut().body.insert(8, 10);
    // The belt needs (default: no).
    assert_eq!(run(&mut f, &m16(0x1C, 8)), res::OK);
    assert_eq!(f.it(10).mode, mode::EQUIPPED);
    f.items.get_mut(&10).unwrap().body_loc = 1;
    f.inv_mut().body.clear();
    f.inv_mut().body.insert(1, 10);
    f.k.gate = false;
    assert_eq!(run(&mut f, &m16(0x1C, 1)), res::OK);
    f.k.gate = true;
    f.k.equip_check = 5;
    assert_eq!(run(&mut f, &m16(0x1C, 1)), res::REFUSED);
    f.k.equip_check = 3;
    assert_eq!(run(&mut f, &m16(0x1C, 1)), res::OK);
    let it = f.it(10);
    assert_eq!((it.mode, it.cmd), (mode::CURSOR, 0x10));
    assert_eq!(f.inv().cursor, Some(10));
    // A belt leaving the body runs the belt change: no belt
    // items, nothing sent.
    assert!(f.sent.is_empty());
    assert!(f.logged("clear_slot 1"));
}

// ---- 0x1D

#[test]
fn swap_cursor_with_body() {
    let mut f = Fake::new();
    f.item(10, mode::CURSOR);
    assert_eq!(run(&mut f, &mloc(0x1D, 10, 0)), res::BAD);
    assert_eq!(run(&mut f, &mloc(0x1D, 10, 1)), res::RANGE);
    f.item(11, mode::EQUIPPED).body_loc = 1;
    f.inv_mut().body.insert(1, 11);
    f.k.equip_check = 1;
    assert_eq!(run(&mut f, &mloc(0x1D, 10, 1)), res::OK);
    assert_eq!(f.inv().cursor, Some(10));
    f.k.equip_check = 5;
    assert_eq!(run(&mut f, &mloc(0x1D, 10, 1)), res::OK);
    assert_eq!(f.inv().cursor, Some(11));
    let e = f.it(11);
    assert_eq!((e.mode, e.cmd, e.iflags), (mode::CURSOR, 0x20, 0x81));
    let n = f.it(10);
    assert_eq!(
        (n.mode, n.cmd, n.iflags, n.body_loc, n.page),
        (1, 0x20, 0x41, 1, 0xFF)
    );
    assert_eq!(f.inv().update, vec![11, 10]);
}

// ---- 0x1E

#[test]
fn swap_1h_with_2h() {
    let mut f = Fake::new();
    f.item(10, mode::CURSOR);
    assert_eq!(run(&mut f, &mloc(0x1E, 10, 0)), res::BAD);
    assert_eq!(run(&mut f, &mloc(0x1E, 10, 3)), res::REFUSED);
    assert_eq!(run(&mut f, &mloc(0x1E, 10, 4)), res::RANGE);
    f.item(11, mode::EQUIPPED);
    f.inv_mut().body.insert(4, 11);
    // Does not give 7: nothing.
    assert_eq!(run(&mut f, &mloc(0x1E, 10, 4)), res::OK);
    assert_eq!(f.inv().cursor, Some(10));
}

/// With the equip check = 7 and no item X in the other hand, the
/// routine's fatal (not a refusal).
#[test]
fn swap_1h_with_2h_without_the_other_hand_is_fatal() {
    use crate::items::moves::{handle, MoveFatal};
    let mut f = Fake::new();
    f.item(10, mode::CURSOR);
    f.item(11, mode::EQUIPPED);
    f.inv_mut().body.insert(4, 11);
    f.k.equip_check = 7;
    assert_eq!(
        handle(&mut f, P, &mloc(0x1E, 10, 4)),
        Some(Err(MoveFatal::Missing))
    );
}

// ---- 0x1F

#[test]
fn swap_cursor_buffer() {
    let mut f = Fake::new();
    f.item(10, mode::CURSOR);
    assert_eq!(run(&mut f, &m32(0x1F, &[10, 11, 2, 3])), 1);
    f.item(11, mode::STORED).page = 1;
    assert_eq!(run(&mut f, &m32(0x1F, &[10, 11, 2, 3])), res::REFUSED);
    // No cube in a cube.
    f.items.get_mut(&11).unwrap().page = 3;
    f.items.get_mut(&10).unwrap().code = *b"box ";
    assert_eq!(run(&mut f, &m32(0x1F, &[10, 11, 2, 3])), res::OK);
    assert_eq!(f.inv().cursor, Some(10));
    f.items.get_mut(&10).unwrap().code = *b"rin ";
    f.k.gate = false;
    assert_eq!(run(&mut f, &m32(0x1F, &[10, 11, 2, 3])), res::OK);
    f.k.gate = true;
    assert_eq!(run(&mut f, &m32(0x1F, &[10, 11, 2, 3])), res::OK);
    assert_eq!(f.inv().cursor, Some(11));
    let t = f.it(11);
    assert_eq!(
        (t.mode, t.page, t.stored_page, t.cmd),
        (mode::CURSOR, 0xFF, 3, 0x40000)
    );
    let c = f.it(10);
    assert_eq!(
        (c.mode, c.page, c.stored_page, c.cmd),
        (mode::STORED, 3, 0, 0x40000)
    );
    assert_eq!(f.pos(Owner::item(10)), (2, 3));
    assert_eq!(f.inv().update, vec![11, 10]);
}

#[test]
fn swap_cursor_buffer_placement_fails() {
    let mut f = Fake::new();
    f.item(10, mode::CURSOR);
    f.item(11, mode::STORED);
    f.k.place_ok = false;
    assert_eq!(run(&mut f, &m32(0x1F, &[10, 11, 2, 3])), res::REFUSED);
    // The target already went to the cursor (no rollback).
    assert_eq!(f.inv().cursor, Some(11));
    // C stays in mode 4, in no grid and not the cursor.
    assert_eq!(f.it(10).mode, mode::CURSOR);
    assert_eq!(f.it(11).mode, mode::CURSOR);
}

#[test]
fn swap_cursor_buffer_clears_4000_on_both() {
    let mut f = Fake::new();
    f.item(10, mode::CURSOR).iflags = iflag::NOEQUIP;
    f.items.get_mut(&10).unwrap().filled = true;
    f.item(11, mode::STORED).iflags = iflag::NOEQUIP;
    assert_eq!(run(&mut f, &m32(0x1F, &[10, 11, 2, 3])), res::OK);
    // Item flag 0x4000 cleared on T and on C; C (socket-filled) gets 0x1.
    assert_eq!(f.it(11).iflags & iflag::NOEQUIP, 0);
    assert_eq!(f.it(10).iflags & iflag::NOEQUIP, 0);
    assert_ne!(f.it(10).iflags & iflag::CHANGED, 0);
}

// ---- 0x20

#[test]
fn use_grid_item_range() {
    let mut f = Fake::new();
    f.item(10, mode::STORED).useable = true;
    assert_eq!(run(&mut f, &m32(0x20, &[10, 151, 100])), res::RANGE);
    assert_eq!(run(&mut f, &m32(0x20, &[10, 100, 49])), res::RANGE);
    assert_eq!(run(&mut f, &m32(0x20, &[10, 150, 50])), res::OK);
    assert_eq!(run(&mut f, &m32(0x20, &[11, 100, 100])), 1);
}

// ---- 0x21

#[test]
fn stack_items() {
    let mut f = Fake::new();
    f.item(10, mode::CURSOR);
    f.item(11, mode::STORED);
    assert_eq!(run(&mut f, &m32(0x21, &[10, 10])), res::REFUSED);
    assert_eq!(run(&mut f, &m32(0x21, &[10, 12])), 1);
    f.items.get_mut(&11).unwrap().page = 2;
    assert_eq!(run(&mut f, &m32(0x21, &[10, 11])), res::REFUSED);
    f.items.get_mut(&11).unwrap().page = 0;
    f.k.stack = false;
    assert_eq!(run(&mut f, &m32(0x21, &[10, 11])), res::OK);
    assert_eq!(f.it(11).cmd, 0);
    f.k.stack = true;
    // Overflow: dst := max, src := rest.
    f.items.get_mut(&11).unwrap().max_stack = 20;
    f.set_stat(Owner::item(10), 70, 15);
    f.set_stat(Owner::item(11), 70, 10);
    assert_eq!(run(&mut f, &m32(0x21, &[10, 11])), res::OK);
    assert_eq!(f.stat(Owner::item(11), 70), 20);
    assert_eq!(f.stat(Owner::item(10), 70), 5);
    assert_eq!(f.it(11).iflags, 0x8);
    assert_eq!(f.it(11).cmd, 0x100);
    assert!(f.logged("3E 11 70") && f.logged("3E 10 70"));
    // Merge: dst := sum, 0x42 for the player's cursor (measured `42 00
    // <player GUID>`), src freed.
    f.set_stat(Owner::item(11), 70, 10);
    f.set_stat(Owner::item(10), 72, 3);
    f.set_stat(Owner::item(11), 72, 9);
    assert_eq!(run(&mut f, &m32(0x21, &[10, 11])), res::OK);
    assert_eq!(f.stat(Owner::item(11), 70), 15);
    assert_eq!(f.stat(Owner::item(11), 72), 3);
    assert_eq!(f.sent, vec![vec![0x42, 0, P as u8, 0, 0, 0]]);
    assert!(f.logged("free 10"));
    assert_eq!(f.inv().cursor, None);
}

/// Two arrow quivers (no durability: = 0) whose total fits
/// merge; only the stat-72 step needs durability (measured: 30 + 40
/// arrows merge in the base game).
#[test]
fn stack_items_without_durability_merge() {
    let mut f = Fake::new();
    f.item(10, mode::CURSOR);
    f.item(11, mode::STORED).max_stack = 500;
    f.k.stack = true;
    f.k.merge = false;
    f.set_stat(Owner::item(10), 70, 100);
    f.set_stat(Owner::item(11), 70, 250);
    f.set_stat(Owner::item(10), 72, 3);
    f.set_stat(Owner::item(11), 72, 9);
    assert_eq!(run(&mut f, &m32(0x21, &[10, 11])), res::OK);
    assert_eq!(f.stat(Owner::item(11), 70), 350);
    assert_eq!(f.stat(Owner::item(11), 72), 9, "no durability step");
    assert!(f.logged("free 10"));
    assert_eq!(f.inv().cursor, None);
}

#[test]
fn stack_books() {
    let mut f = Fake::new();
    f.item(10, mode::CURSOR).types = vec![ty::BOOK];
    let d = f.item(11, mode::STORED);
    d.types = vec![ty::BOOK];
    d.max_stack = 20;
    f.set_stat(Owner::item(10), 70, 15);
    f.set_stat(Owner::item(11), 70, 10);
    run(&mut f, &m32(0x21, &[10, 11]));
    assert!(f.logged("book 10"));
}

// ---- 0x22

#[test]
fn unstack_items_x1() {
    let mut f = Fake::new();
    assert_eq!(run(&mut f, &m32(0x22, &[10])), 1);
    f.item(10, mode::STORED);
    assert_eq!(run(&mut f, &m32(0x22, &[10])), res::REFUSED);
}

// ---- 0x23

#[test]
fn item_to_belt() {
    let mut f = Fake::new();
    f.item(10, mode::CURSOR);
    assert_eq!(run(&mut f, &m32(0x23, &[10, 2])), res::OK);
    assert_eq!(f.it(10).mode, mode::CURSOR);
    f.items.get_mut(&10).unwrap().beltable = true;
    f.k.place_ok = false;
    f.items.get_mut(&10).unwrap().page = 0;
    assert_eq!(run(&mut f, &m32(0x23, &[10, 2])), res::OK);
    assert_eq!(f.it(10).page, 0xFF);
    f.k.place_ok = true;
    f.k.link_ok = false;
    assert_eq!(run(&mut f, &m32(0x23, &[10, 2])), res::REFUSED);
    f.k.link_ok = true;
    // No `numboxes` check: slot 15 is passed to the placement as is.
    assert_eq!(run(&mut f, &m32(0x23, &[10, 15])), res::OK);
    assert!(f.logged("belt_place 10 15"));
    let it = f.it(10);
    assert_eq!((it.mode, it.page, it.cmd), (mode::BELT, 0xFF, 0x400));
    assert_eq!(f.inv().cursor, None);
}

// ---- 0x24

#[test]
fn item_from_belt() {
    let mut f = Fake::new();
    f.item(10, mode::BELT);
    f.unit(Owner::item(10)).x = 5;
    f.item(11, mode::CURSOR);
    assert_eq!(run(&mut f, &m32(0x24, &[10])), res::BAD);
    f.inv_mut().cursor = None;
    f.k.gate = false;
    assert_eq!(run(&mut f, &m32(0x24, &[10])), res::OK);
    f.k.gate = true;
    // Missing item passes the belt check, then refused.
    assert_eq!(run(&mut f, &m32(0x24, &[12])), res::REFUSED);
    assert_eq!(run(&mut f, &m32(0x24, &[10])), res::OK);
    assert_eq!((f.it(10).mode, f.it(10).cmd), (mode::CURSOR, 0x800));
    assert_eq!(f.inv().cursor, Some(10));
    assert_eq!(f.log.last().unwrap(), "compact 5");
}

// ---- 0x25

#[test]
fn switch_belt_item() {
    let mut f = Fake::new();
    f.item(10, mode::CURSOR);
    f.item(11, mode::BELT);
    f.unit(Owner::item(11)).x = 6;
    assert_eq!(run(&mut f, &m32(0x25, &[10, 11])), res::OK);
    assert_eq!(f.it(11).mode, mode::BELT);
    f.items.get_mut(&10).unwrap().beltable = true;
    assert_eq!(run(&mut f, &m32(0x25, &[10, 12])), res::REFUSED);
    assert_eq!(run(&mut f, &m32(0x25, &[10, 11])), res::OK);
    assert_eq!(f.inv().cursor, Some(11));
    assert_eq!((f.it(11).mode, f.it(11).cmd), (mode::CURSOR, 0x1000));
    assert_eq!(
        (f.it(10).mode, f.it(10).cmd, f.it(10).page),
        (mode::BELT, 0x1000, 0xFF)
    );
    assert_eq!(f.pos(Owner::item(10)), (6, 0));
    assert!(f.logged("belt_place 10 6"));
    // A failed placement is fatal.
    let mut f = Fake::new();
    f.item(10, mode::CURSOR).beltable = true;
    f.item(11, mode::BELT);
    f.k.place_ok = false;
    assert_eq!(
        handle(&mut f, P, &m32(0x25, &[10, 11])),
        Some(Err(MoveFatal::BeltSwitch))
    );
}

// ---- 0x26

#[test]
fn use_belt_item() {
    let mut f = Fake::new();
    assert_eq!(run(&mut f, &m32(0x26, &[10, 0, 0])), res::REFUSED);
    f.item(10, mode::BELT);
    f.unit(Owner::item(10)).x = 9;
    assert_eq!(run(&mut f, &m32(0x26, &[10, 0, 0])), res::REFUSED);
    f.items.get_mut(&10).unwrap().useable = true;
    f.k.trading = true;
    assert_eq!(run(&mut f, &m32(0x26, &[10, 0, 0])), res::OK);
    assert!(!f.logged("use 10 on 0:1"));
    f.k.trading = false;
    // On the hireling: potions only.
    f.k.hireling = Some(Owner::monster(60));
    assert_eq!(run(&mut f, &m32(0x26, &[10, 1, 0])), res::OK);
    assert!(f.log.iter().all(|l| !l.starts_with("use ")));
    f.items.get_mut(&10).unwrap().types = vec![ty::HPOT];
    assert_eq!(run(&mut f, &m32(0x26, &[10, 1, 0])), res::OK);
    assert!(f.logged("use 10 on 1:60"));
    assert!(f.logged("remove_used 10"));
    assert_eq!(f.log.last().unwrap(), "compact 9");
    // Classic: on_merc ignored.
    f.expansion = false;
    f.log.clear();
    run(&mut f, &m32(0x26, &[10, 1, 0]));
    assert!(f.logged("use 10 on 0:1"));
}

// ---- 0x27

#[test]
fn use_item_action() {
    let mut f = Fake::new();
    f.item(10, mode::STORED);
    assert_eq!(run(&mut f, &m32(0x27, &[10, 11])), 1);
    f.item(11, mode::CURSOR);
    assert_eq!(run(&mut f, &m32(0x27, &[10, 11])), res::OK);
}

// ---- 0x28

#[test]
fn socket_item() {
    let mut f = Fake::new();
    f.item(10, mode::CURSOR);
    let t = f.item(11, mode::STORED);
    t.sockets = 1;
    assert_eq!(run(&mut f, &m32(0x28, &[10, 11])), res::OK);
    assert_eq!(f.it(10).mode, mode::CURSOR);
    f.items.get_mut(&11).unwrap().iflags = 0x10;
    assert_eq!(run(&mut f, &m32(0x28, &[10, 11])), res::OK);
    assert_eq!(f.it(10).mode, mode::CURSOR);
    let fl = f.items.get_mut(&10).unwrap();
    fl.filler = true;
    fl.iflags = 0x10;
    assert_eq!(run(&mut f, &m32(0x28, &[10, 11])), res::OK);
    assert_eq!(f.it(10).mode, mode::CURSOR);
    f.items.get_mut(&11).unwrap().iflags = 0x10 | 0x800 | 0x4000;
    f.k.trading = true;
    assert_eq!(run(&mut f, &m32(0x28, &[10, 11])), res::OK);
    assert_eq!(f.it(10).mode, mode::CURSOR);
    f.k.trading = false;
    assert_eq!(run(&mut f, &m32(0x28, &[10, 11])), res::OK);
    assert_eq!(f.it(10).mode, mode::SOCKETED);
    assert_eq!(f.it(11).fillers, vec![10]);
    assert_eq!(f.it(11).iflags, 0x10 | 0x800 | 0x1);
    assert_eq!(f.inv().cursor, None);
    assert_eq!(f.inv().update, vec![11]);
    assert!(f.logged("runeword 11"));
    // Sockets full now.
    f.item(12, mode::CURSOR).filler = true;
    f.items.get_mut(&12).unwrap().iflags = 0x10;
    assert_eq!(run(&mut f, &m32(0x28, &[12, 11])), res::OK);
    assert_eq!(f.it(12).mode, mode::CURSOR);
}

// ---- 0x29

#[test]
fn scroll_to_book() {
    let mut f = Fake::new();
    f.item(10, mode::CURSOR).types = vec![ty::SCRO];
    let b = f.item(11, mode::STORED);
    b.types = vec![ty::BOOK];
    b.max_stack = 20;
    b.spell = 1;
    f.items.get_mut(&10).unwrap().spell = 2;
    assert_eq!(
        handle(&mut f, P, &m32(0x29, &[10, 11])),
        Some(Err(MoveFatal::SpellMismatch))
    );
    f.items.get_mut(&10).unwrap().spell = 1;
    f.set_stat(Owner::item(11), 70, 20);
    assert_eq!(run(&mut f, &m32(0x29, &[10, 11])), res::OK);
    assert_eq!(f.inv().cursor, Some(10));
    // A full tome: nothing sent (measured with a full `tbk`).
    assert!(f.sent.is_empty());
    f.set_stat(Owner::item(11), 70, 5);
    assert_eq!(run(&mut f, &m32(0x29, &[10, 11])), res::OK);
    assert_eq!(f.stat(Owner::item(11), 70), 6);
    assert_eq!(f.inv().cursor, None);
    // The cursor clear names the player (measured `42 00 <player>`).
    assert_eq!(f.sent, vec![vec![0x42, 0, P as u8, 0, 0, 0]]);
    assert!(f.logged("free 10") && f.logged("3E 11 70") && f.logged("book 1"));
    // A wrong type is refused.
    f.item(12, mode::CURSOR);
    assert_eq!(run(&mut f, &m32(0x29, &[12, 11])), res::REFUSED);
}

// ---- 0x50

#[test]
fn drop_gold_g1_and_checks() {
    let mut f = Fake::new();
    f.set_stat(me(), 14, 5000);
    // G1: amount 0 → 0, no pile, no creation.
    assert_eq!(run(&mut f, &m32(0x50, &[P, 0])), res::OK);
    assert!(f.log.iter().all(|l| !l.starts_with("create_gold")));
    assert_eq!(run(&mut f, &m32(0x50, &[2, 10])), res::REFUSED);
    assert_eq!(run(&mut f, &m32(0x50, &[P, 5001])), res::REFUSED);
    assert_eq!(run(&mut f, &m32(0x50, &[P, u32::MAX])), res::REFUSED);
    // Above the gold limit (level 1 → 10000).
    f.set_stat(me(), 14, 20000);
    assert_eq!(run(&mut f, &m32(0x50, &[P, 10001])), res::REFUSED);
    f.k.busy = true;
    f.k.trading = true;
    assert_eq!(run(&mut f, &m32(0x50, &[P, 10])), res::REFUSED);
    f.k.busy = false;
    assert_eq!(run(&mut f, &m32(0x50, &[P, 700])), res::OK);
    assert_eq!(f.stat(me(), 14), 19300);
    assert!(f.logged("create_gold 500") && f.logged("set_owner 500"));
    assert_eq!(f.stat(Owner::item(500), 14), 700);
    // ≠ 0: no owner.
    f.k.q44 = true;
    run(&mut f, &m32(0x50, &[P, 1]));
    assert!(!f.logged("set_owner 501"));
}

// ---- 0x61

#[test]
fn merc_item_gates() {
    let mut f = Fake::new();
    f.expansion = false;
    assert_eq!(run(&mut f, &m16(0x61, 1)), res::REFUSED);
    f.expansion = true;
    // No hireling → 0.
    assert_eq!(run(&mut f, &m16(0x61, 1)), res::OK);
    f.k.hireling = Some(Owner::monster(60));
    f.k.used_skill = true;
    assert_eq!(run(&mut f, &m16(0x61, 1)), res::OK);
    f.k.used_skill = false;
    f.k.owns = false;
    assert_eq!(run(&mut f, &m16(0x61, 1)), res::OK);
    f.k.owns = true;
    // Location 0 with no cursor item → 0. A hireling without an
    // inventory → 3.
    assert_eq!(run(&mut f, &m16(0x61, 0)), res::OK);
    assert_eq!(run(&mut f, &m16(0x61, 1)), res::REFUSED);
    f.invs.entry(Owner::monster(60)).or_default();
    // A location outside 1..10 → 3.
    assert_eq!(run(&mut f, &m16(0x61, 11)), res::REFUSED);
    // Empty hireling slot → 2.
    assert_eq!(run(&mut f, &m16(0x61, 1)), res::BAD);
    f.invs.remove(&me());
    assert_eq!(run(&mut f, &m16(0x61, 1)), res::REFUSED);
}

#[test]
fn merc_take_copies_to_cursor() {
    let mut f = Fake::new();
    let m = Owner::monster(60);
    f.k.hireling = Some(m);
    f.units.insert(Owner::item(20), Default::default());
    f.items.insert(
        20,
        super::FItem {
            mode: mode::EQUIPPED,
            body_loc: 1,
            owner: Some(m),
            ..Default::default()
        },
    );
    let inv = f.invs.entry(m).or_default();
    inv.list.push(20);
    inv.body.insert(1, 20);
    assert_eq!(run(&mut f, &m16(0x61, 1)), res::OK);
    assert!(f.logged("copy 20 -> 500"));
    assert_eq!(f.inv().cursor, Some(500));
    assert_eq!(f.it(20).iflags, 0x20);
    assert_eq!(f.it(20).cmd, 0x10);
    assert_eq!(f.invs[&m].update, vec![20]);
    assert!(f.invs[&m].body.is_empty());
    assert!(f.logged("stat_refresh_unlink 1:60 0"));
}

/// An item at the location that the unlink does not
/// return → 2, nothing taken (not fatal).
#[test]
fn merc_take_failed_unlink_is_2() {
    let mut f = Fake::new();
    let m = Owner::monster(60);
    f.k.hireling = Some(m);
    f.units.insert(Owner::item(20), Default::default());
    f.items.insert(
        20,
        super::FItem {
            mode: mode::EQUIPPED,
            body_loc: 1,
            owner: Some(m),
            ..Default::default()
        },
    );
    // In the slot but not in the hireling's item list.
    f.invs.entry(m).or_default().body.insert(1, 20);
    assert_eq!(run(&mut f, &m16(0x61, 1)), res::BAD);
    assert_eq!(f.inv().cursor, None);
    assert!(!f.logged("copy 20 -> 500"));
}

#[test]
fn merc_give_rules() {
    use crate::items::moves::handlers::merc_give;
    let m = Owner::monster(60);
    let setup = |class: u32, types: Vec<u16>, two: bool| {
        let mut f = Fake::new();
        f.k.hireling = Some(m);
        f.unit(m).class = class;
        let c = f.item(10, mode::CURSOR);
        c.types = types;
        c.iflags = 0x10;
        c.two_handed = two;
        f
    };
    let gives = |f: &mut Fake| {
        merc_give(f, me(), Some(m), 10);
        f.logged("equip_on_merc 10")
    };
    assert!(gives(&mut setup(0x10F, vec![ty::BOW], true)));
    assert!(!gives(&mut setup(0x10F, vec![ty::SPEA], true)));
    assert!(gives(&mut setup(0x152, vec![ty::POLE], true)));
    assert!(gives(&mut setup(0x167, vec![ty::SWOR], false)));
    assert!(!gives(&mut setup(0x167, vec![ty::SWOR], true)));
    assert!(gives(&mut setup(0x230, vec![ty::AXE], false)));
    assert!(gives(&mut setup(0x230, vec![ty::PHLM], false)));
    assert!(gives(&mut setup(0x231, vec![ty::SWOR], true)));
    assert!(gives(&mut setup(0x231, vec![ty::HELM], false)));
    // Not identified / broken: nothing, no sound.
    let mut f = setup(0x10F, vec![ty::BOW], true);
    f.items.get_mut(&10).unwrap().iflags = 0;
    assert_eq!(merc_give(&mut f, me(), Some(m), 10), 0);
    assert!(!f.logged("merc_sound"));
    // Potions: used on the hireling, consumed, result 1.
    let mut f = setup(0x10F, vec![ty::APOT], false);
    assert_eq!(merc_give(&mut f, me(), Some(m), 10), 1);
    assert!(f.logged("use 10 on 1:60") && f.logged("consume 10"));
    assert_eq!(f.inv().cursor, None);
    // Requirements fail: sound, no equip.
    let mut f = setup(0x10F, vec![ty::TORS], false);
    f.k.requirements = false;
    assert!(!gives(&mut f));
    assert!(f.logged("merc_sound"));
    // No hireling: only body armor and helms are allowed (nothing to equip).
    let mut f = setup(0x10F, vec![ty::BOW], true);
    assert_eq!(merc_give(&mut f, me(), None, 10), 0);
    // Through the handler: a quest item is not given.
    let mut f = setup(0x10F, vec![ty::BOW], true);
    f.items.get_mut(&10).unwrap().quest = 1;
    assert_eq!(run(&mut f, &m16(0x61, 0)), res::OK);
    assert!(!f.logged("equip_on_merc 10"));
    let mut f = setup(0x10F, vec![ty::BOW], true);
    assert_eq!(run(&mut f, &m16(0x61, 0)), res::OK);
    assert!(f.logged("equip_on_merc 10"));
}

// ---- 0x63

#[test]
fn item_to_belt_shift() {
    let mut f = Fake::new();
    f.k.bits = vec![0xEE];
    f.item(10, mode::STORED);
    f.unit(Owner::item(10)).x = 3;
    assert_eq!(run(&mut f, &m32(0x63, &[10])), res::BAD);
    f.items.get_mut(&10).unwrap().beltable = true;
    // No slot → 0.
    assert_eq!(run(&mut f, &m32(0x63, &[10])), res::OK);
    f.k.belt_slot = Some(2);
    f.items.get_mut(&10).unwrap().page = 3;
    assert_eq!(run(&mut f, &m32(0x63, &[10])), res::REFUSED);
    f.items.get_mut(&10).unwrap().page = 0;
    f.item(11, mode::CURSOR);
    assert_eq!(run(&mut f, &m32(0x63, &[10])), res::BAD);
    // "Can't do that": 0x5A.
    assert_eq!(f.sent.remove(0)[..3], [0x5A, 0x0E, 0x01]);
    f.inv_mut().cursor = None;
    assert_eq!(run(&mut f, &m32(0x63, &[10])), res::OK);
    let it = f.it(10);
    assert_eq!((it.mode, it.page, it.stored_page), (mode::BELT, 0xFF, 0));
    // Direct sends: 0x9D action 5 (page shown 0), then 0x9C action 0xE.
    assert_eq!(f.sent.len(), 2);
    assert_eq!(f.sent[0][..2], [0x9D, 0x05]);
    assert_eq!(f.sent[0][13..], [0xEE, 0, 0]);
    assert_eq!(f.sent[1][..2], [0x9C, 0x0E]);
    // No update list (owner refresh only).
    assert!(f.inv().update.is_empty());
    assert_eq!(f.update_bits(me()), 3);
    assert!(f.logged("room_change 10 3,0"));
}

#[test]
fn item_to_belt_shift_limbo() {
    let mut f = Fake::new();
    f.item(10, mode::STORED).beltable = true;
    f.k.belt_slot = Some(0);
    f.k.place_ok = false;
    assert_eq!(run(&mut f, &m32(0x63, &[10])), res::OK);
    assert_eq!(f.it(10).mode, mode::CURSOR);
    assert_eq!(f.inv().cursor, None);
    assert!(!f.inv().list.contains(&10));
    assert_eq!(f.sent.len(), 1);
    // `has_inventory` seam reached through the trait object.
    assert!(InventoryOps::has_inventory(&f, me()));
}

/// Placement and link failures must leave the same cursor and body identities.
#[test]
fn failed_equipment_swaps_restore_all_items_and_updates() {
    for (opcode, check, loc, other) in [(0x1B, 2, 4, 5), (0x1D, 5, 4, 4), (0x1E, 7, 4, 5)] {
        for failure in 0..5 {
            let mut f = Fake::new();
            f.item(10, mode::CURSOR);
            f.item(11, mode::EQUIPPED).body_loc = other;
            f.inv_mut().body.insert(other, 11);
            if opcode == 0x1E {
                f.item(12, mode::EQUIPPED).body_loc = loc;
                f.inv_mut().body.insert(loc, 12);
            }
            f.k.equip_check = check;
            f.k.place_ok = failure != 0;
            f.k.link_ok = failure != 1;
            f.k.fail_body_place = failure == 2;
            f.k.fail_link_kind = (failure == 3).then_some(3);
            f.k.fail_unlink = failure == 4;
            let body = f.inv().body.clone();
            let items = f.inv().list.clone();
            let result = handle(&mut f, P, &mloc(opcode, 10, loc));
            if failure == 4 {
                assert_eq!(result, Some(Err(MoveFatal::Unlink)));
            } else {
                assert_eq!(result, Some(Ok(res::REFUSED)));
            }
            assert_eq!(f.inv().cursor, Some(10), "opcode {opcode:#x}");
            assert_eq!(f.inv().body, body);
            assert_eq!(f.inv().list, items);
            assert!(f.inv().update.is_empty());
            assert_eq!(f.it(10).mode, mode::CURSOR);
            for item in body.values() {
                assert_eq!(f.it(*item).mode, mode::EQUIPPED);
            }
            // A retry can complete: there is no orphaned item or stale cursor.
            f.k.place_ok = true;
            f.k.link_ok = true;
            f.k.fail_body_place = false;
            f.k.fail_link_kind = None;
            f.k.fail_unlink = false;
            assert_eq!(run(&mut f, &mloc(opcode, 10, loc)), res::OK);
            assert_eq!(f.inv().cursor, Some(if opcode == 0x1E { 12 } else { 11 }));
            assert_eq!(f.inv().body.get(&loc), Some(&10));
        }
    }
}

#[test]
fn failed_unequip_keeps_the_body_item_and_empty_cursor() {
    for detached in [false, true] {
        let mut f = Fake::new();
        f.item(10, mode::EQUIPPED).body_loc = 4;
        f.inv_mut().body.insert(4, 10);
        f.k.equip_check = 3;
        f.k.fail_unlink = true;
        let result = if detached {
            crate::items::moves::handlers::unequip_detached(&mut f, me(), 4).map(u32::from)
        } else {
            crate::items::moves::handlers::remove_body_item(&mut f, me(), 4)
        };
        assert_eq!(result, Err(MoveFatal::Unlink));
        assert_eq!(f.inv().cursor, None);
        assert_eq!(f.inv().body.get(&4), Some(&10));
        assert_eq!(f.it(10).mode, mode::EQUIPPED);
        assert!(f.inv().update.is_empty());
    }
}

#[test]
fn belt_removal_without_room_for_contents_restores_the_equipped_belt() {
    let mut f = Fake::new();
    f.item(10, mode::EQUIPPED).body_loc = 8;
    f.items.get_mut(&10).unwrap().types = vec![ty::BELT];
    f.inv_mut().body.insert(8, 10);
    f.item(11, mode::BELT).beltable = true;
    assert!(f.belt_place(me(), 11, 5));
    f.k.equip_check = 3;
    f.k.boxes = 4;
    f.k.place_ok = false;
    f.k.spot = None;
    assert_eq!(
        crate::items::moves::handlers::remove_from_body(&mut f, me(), 10),
        Err(MoveFatal::Link)
    );
    assert_eq!(f.inv().body.get(&8), Some(&10));
    assert_eq!(f.it(10).mode, mode::EQUIPPED);
    assert_eq!(f.belt_item(me(), 5), Some(11));
    assert_eq!(f.it(11).mode, mode::BELT);
    assert_eq!(f.inv().cursor, None);
    assert!(f.inv().update.is_empty());
    assert!(f.sent.is_empty());
}

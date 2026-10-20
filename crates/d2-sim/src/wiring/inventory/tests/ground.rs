//! Tests for 0x16 PickItem and 0x17 DropItem on real units, unit lists,
//! item data and the inventory model.

use super::*;
use crate::items::moves::{ground_update, item_unit_update, MoveUnits};

/// 0x16: a ground key to the cursor. Mode 4, the cursor,
/// out of the room list, unit flags 0x2 / 0x2000000 cleared, command flag
/// 0x40; the update pass sends 0x9C action 1, which sets x, y to
/// 0; the pickup sound follows.
#[test]
fn pick_to_cursor_moves_a_ground_item_to_the_cursor() {
    let mut w = World::new();
    let k = w.ground_item(KEY, 12, 11);
    assert!(w.in_room(k));
    let u = w.unit(k).unwrap();
    w.units.get_mut(u).unwrap().flags |= 0x2 | 0x200_0000;

    assert_eq!(w.handle(&pick(k, 1)), Ok(0));

    assert_eq!(w.mode(k), 4);
    assert_eq!(w.inventory().cursor(), Some(u));
    assert!(!w.in_room(k), "ground item removed from room list");
    assert_eq!(w.units.get(u).unwrap().flags & (0x2 | 0x200_0000), 0);
    let d = w.data(k);
    assert_eq!((d.cmd_flags, d.page), (0x40, 0xFF));
    assert_eq!(w.inventory().update_list(), &[k]);
    // Owner refresh: bits 0 and 1 of the player's unit flags.
    assert_eq!(w.units.get(w.player).unwrap().flags2 & 3, 3);
    assert_eq!(
        w.rest.log,
        [
            format!("room_delete_notice {k}"),
            format!("free_collision {k}"),
            format!("quest_item_picked {k}"),
            format!("pickup_sound {} {k}", w.pguid()),
        ]
    );

    let out = w.drain();
    assert_eq!(item_msgs(&out), [(0x9C, 0x01, k)]);
    assert_eq!(out.len(), 3, "then 0x47, 0x48");
    assert_eq!((out[1][0], out[2][0]), (0x47, 0x48));
    assert_eq!((w.data(k).x, w.data(k).y), (0, 0));
    assert_eq!(w.data(k).cmd_flags, 0, "clean-up");
    assert!(w.inventory().update_list().is_empty());
}

/// 0x16: distance > 50 → 1; distance ≥ 5 → walk, 0; the item
/// stays on the ground.
#[test]
fn pick_out_of_range_or_far_walks_or_fails() {
    let mut w = World::new();
    let k = w.ground_item(KEY, 40, 40);
    w.rest.distance = 51;
    assert_eq!(w.handle(&pick(k, 0)), Ok(1));
    w.rest.distance = 5;
    assert_eq!(w.handle(&pick(k, 0)), Ok(0));
    assert_eq!(
        w.rest.log,
        [format!("walk_to_item {} {k} false", w.pguid())]
    );
    assert_eq!(w.mode(k), 3);
    assert!(w.in_room(k));
    // Not an item / type > 5 / own player.
    assert_eq!(w.handle(&msg(0x16, &[6, k, 0])), Ok(2));
    assert_eq!(w.handle(&msg(0x16, &[0, w.pguid(), 0])), Ok(3));
    // The size check comes first.
    assert_eq!(w.handle(&pick(k, 0)[..12]), Ok(3));
}

/// Auto pickup: a key (no body location, not beltable) goes to the first
/// free page-0 position of the player's 10 × 4 grid ((9, 3)): mode
/// 0, page 0, linked, command flag 0x80 → 0x9C action 4.
#[test]
fn auto_pickup_places_into_the_inventory_grid() {
    let mut w = World::new();
    let k = w.ground_item(KEY, 12, 11);
    assert_eq!(w.handle(&pick(k, 0)), Ok(0));
    let u = w.unit(k).unwrap();
    assert_eq!(w.mode(k), 0);
    let d = w.data(k);
    assert_eq!((d.page, d.x, d.y, d.node_grid), (0, 9, 3, 3));
    assert_eq!(d.owner_guid, w.pguid());
    assert_eq!(w.inventory().items(), &[u]);
    assert_eq!(w.inventory().item_at(2, 9, 3), Some(u));
    assert_eq!(w.inventory().cursor(), None);
    assert!(!w.in_room(k));
    assert_eq!(w.items.get(u).unwrap().inv_page, 0, "page written back");
    assert_eq!(item_msgs(&w.drain()), [(0x9C, 0x04, k)]);
}

/// Auto pickup with a full page 0 (ten 2 × 2 caps fill 10 × 4): refused
/// pickup with sound 0x17: mode 3 again, page 0xFF, unit flag
/// 0x1000; result 0; ground update (see [`assert_ground_update`]).
#[test]
fn auto_pickup_with_a_full_inventory_is_refused() {
    let mut w = World::new();
    let me = w.me();
    for i in 0..10 {
        let c = w.ground_item(CAP, 0, 0);
        let (x, y) = ((i % 5) * 2, (i / 5) * 2);
        assert!(w.desk(|d| moves::InventoryOps::place_at(d, me, c, 0, x, y)));
    }
    let k = w.ground_item(KEY, 12, 11);
    assert_eq!(w.handle(&pick(k, 0)), Ok(0));
    assert_eq!(w.mode(k), 3);
    let u = w.unit(k).unwrap();
    assert_eq!(w.units.get(u).unwrap().flags & 0x1000, 0x1000);
    assert_eq!(w.data(k).page, 0xFF);
    assert!(w.in_room(k), "never left the room");
    assert_eq!(w.rest.called(&format!("sound {} 0x17", w.pguid())), 1);
    assert_eq!(w.inventory().items().len(), 10);
    assert_ground_update(&mut w, k);
}

/// The ground update on a real item unit: the allocator sets unit flag
/// 0x10 ("not yet announced") on every unit, so the
/// first update is part 1 (the unit-add message, 0x9C action 2 for a
/// dropped ground item) and part 2 (`ground_update`) builds nothing; once
/// the room clean-up has cleared 0x10 and 0x1, a later change (bit 0)
/// sends part 2's 0x9C action 2.
fn assert_ground_update(w: &mut World, k: Guid) {
    let u = w.unit(k).unwrap();
    assert_ne!(w.units.get(u).unwrap().flags & 0x10, 0);
    assert_eq!(w.desk(|d| ground_update(d, k)), Ok(None));
    let m = w.desk(|d| item_unit_update(d, k)).unwrap().unwrap();
    assert_eq!(head(&m), (0x9C, 0x02, k));
    w.units.get_mut(u).unwrap().flags &= !0x11;
    assert_eq!(w.desk(|d| item_unit_update(d, k)), Ok(None));
    w.units.get_mut(u).unwrap().flags |= 0x1;
    let m = w.desk(|d| item_unit_update(d, k)).unwrap().unwrap();
    assert_eq!(head(&m), (0x9C, 0x02, k));
}

/// 0x17: the cursor item dropped at the free spot found from (x +
/// 2, y + 3): position, room list, unit flags 0x1002 and 0x2000000, mode
/// 3, page 0xFF, expiry frame + 15000 (a normal item, not a quest item),
/// the ITEMDROPPED hook, cursor cleared; ground update (see
/// [`assert_ground_update`]).
#[test]
fn drop_places_the_cursor_item_on_the_ground() {
    let mut w = World::new();
    let k = w.cursor_item(KEY);
    let u = w.unit(k).unwrap();
    w.game.frame = 100;
    w.rest.room_at = true;
    w.rest.spot = Some(Spot {
        room: w.room,
        x: 13,
        y: 12,
    });
    w.rest.log.clear();

    assert_eq!(w.handle(&drop_msg(k)), Ok(0));

    assert_eq!(
        w.rest.queries.borrow().last().unwrap(),
        "free_spot (12, 13) (10, 10) 1 0x3e01 0x801 1"
    );
    assert_eq!(w.mode(k), 3);
    assert!(w.in_room(k));
    assert_eq!(w.desk(|d| d.pos(Owner::item(k))), (13, 12));
    assert_eq!(w.units.get(u).unwrap().flags & 0x200_1002, 0x200_1002);
    assert_eq!(w.data(k).page, 0xFF);
    assert_eq!(w.state.expiry[&u], 100 + 15000);
    assert_eq!(w.inventory().cursor(), None);
    assert_eq!(w.rest.log, [format!("quest_item_dropped {k}")]);
    assert_ground_update(&mut w, k);
}

/// 0x17: no free spot → nothing, the item stays on the cursor; an item
/// that is not the cursor item → 1.
#[test]
fn drop_without_a_spot_keeps_the_cursor_item() {
    let mut w = World::new();
    let k = w.cursor_item(KEY);
    assert_eq!(w.handle(&drop_msg(k)), Ok(0));
    assert_eq!(w.mode(k), 4);
    assert_eq!(w.inventory().cursor(), w.unit(k));
    assert_eq!(w.handle(&drop_msg(k + 77)), Ok(1));
    assert_eq!(
        w.rest.queries.borrow().last().unwrap(),
        "free_spot (10, 10) (10, 10) 1 0x3e01 0x801 1",
        "no room at (x + 2, y + 3): start at the player"
    );
}

/// The pickup position comes from the real grid record. With the
/// barbarian's record (4) cut to 9 × 4 the same pickup lands at (8, 3),
/// and nothing else differs.
#[test]
fn pickup_position_follows_the_grid_record() {
    let run = |t: InvTables| {
        let mut w = World::with_tables(t);
        let k = w.ground_item(KEY, 12, 11);
        assert_eq!(w.handle(&pick(k, 0)), Ok(0));
        let mut d = w.data(k);
        let pos = (d.x, d.y);
        d.x = 0;
        (pos, d, w.drain())
    };
    let (base, d0, m0) = run(inv_tables());
    let mut t = inv_tables();
    t.grids[CLASS as usize].grid_x = 9;
    let (cut, d1, m1) = run(t);
    assert_eq!(base, (9, 3));
    assert_eq!(cut, (8, 3));
    assert_eq!(d0, d1);
    // The 0x9C item stream carries the grid x (4 bits at stream bit 49,
    // message bit 113): 9 → 8 flips its bit
    // 0, byte 14 bit 1; nothing else differs.
    let mut m1 = m1;
    m1[0][14] ^= 0x02;
    assert_eq!(m0, m1);
}

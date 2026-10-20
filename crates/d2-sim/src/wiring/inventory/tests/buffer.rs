//! Tests for 0x18 InsertItemInBuffer and 0x19 RemoveItemFromBuffer on the
//! real inventory grids.

use super::*;

/// A key auto-picked into page 0 at (9, 3), messages drained.
fn stored_key(w: &mut World) -> Guid {
    let k = w.ground_item(KEY, 12, 11);
    assert_eq!(w.handle(&pick(k, 0)), Ok(0));
    w.drain();
    k
}

/// 0x19: lift a page-0 key to the cursor: unlinked (cells cleared), mode
/// 4, stored page 0, page 0xFF, command flag 0x4 → 0x9D action 5 with
/// the player as owner; the room-change notice carries the old
/// cell. 0x18: put it back at (0, 0) of page 0 → mode 0, linked, command
/// flag 0x2 → 0x9C action 4.
#[test]
fn lift_and_insert_move_an_item_inside_the_grid() {
    let mut w = World::new();
    let k = stored_key(&mut w);
    let u = w.unit(k).unwrap();
    w.rest.log.clear();

    assert_eq!(w.handle(&lift(k)), Ok(0));
    assert_eq!(w.mode(k), 4);
    assert_eq!(w.inventory().cursor(), Some(u));
    assert!(w.inventory().items().is_empty());
    assert_eq!(w.inventory().item_at(2, 9, 3), None);
    let d = w.data(k);
    assert_eq!((d.stored_page, d.page, d.cmd_flags), (0, 0xFF, 0x4));
    assert_eq!(
        w.rest.log,
        [format!("room_change_notice {k} 9 3")],
        "old cell; inventory stat refresh uses the seam (default no-op)"
    );
    let out = w.drain();
    assert_eq!(item_msgs(&out), [(0x9D, 0x05, k)]);
    assert_eq!(out[0][8], 0, "owner type player");
    assert_eq!(&out[0][9..13], &w.pguid().to_le_bytes());

    // The unit flags are cleared by the room clean-up; on this path only
    // the owner refresh of the insert sets them again.
    let p = w.player;
    w.units.get_mut(p).unwrap().flags2 &= !3;
    assert_eq!(w.handle(&insert(k, 0, 0, 0)), Ok(0));
    assert_eq!(w.units.get(p).unwrap().flags2 & 3, 3, "owner refresh");
    assert_eq!(w.mode(k), 0);
    let d = w.data(k);
    assert_eq!((d.page, d.x, d.y, d.cmd_flags), (0, 0, 0, 0x2));
    assert_eq!(w.inventory().item_at(2, 0, 0), Some(u));
    assert_eq!(w.inventory().cursor(), None);
    assert_eq!(item_msgs(&w.drain()), [(0x9C, 0x04, k)]);
}

/// 0x18: the cube page (3, 3 × 4) needs no open cube; the
/// stash (page 4) only in a town level, else 3; page 1 → 2; a cell
/// outside the grid → 3 (the placement fails, nothing changed).
#[test]
fn insert_checks_the_page_then_places() {
    let mut w = World::new();
    let k = stored_key(&mut w);
    assert_eq!(w.handle(&lift(k)), Ok(0));
    w.drain();

    assert_eq!(w.handle(&insert(k, 0, 0, 1)), Ok(2));
    assert_eq!(w.handle(&insert(k, 0, 0, 4)), Ok(3), "not in town");
    assert_eq!(w.handle(&insert(k, 3, 0, 3)), Ok(3), "x 3 outside 3 × 4");
    assert_eq!(w.mode(k), 4, "nothing changed");

    assert_eq!(w.handle(&insert(k, 2, 3, 3)), Ok(0));
    let d = w.data(k);
    assert_eq!((d.page, d.x, d.y, d.node_grid), (3, 2, 3, 2 + 3 + 1));
    assert_eq!(item_msgs(&w.drain()), [(0x9C, 0x04, k)]);

    assert_eq!(w.handle(&lift(k)), Ok(3), "players lift only page 0 (§7.4)");
    w.rest.in_town = true;
    let s = w.cursor_item(KEY);
    assert_eq!(w.handle(&insert(s, 5, 7, 4)), Ok(0), "stash 6 × 8");
    assert_eq!((w.data(s).page, w.data(s).x, w.data(s).y), (4, 5, 7));
}

/// 0x19: an item that is not stored in the player's inventory
/// → 1; with a cursor item → resync (seam) and 2.
#[test]
fn lift_checks() {
    let mut w = World::new();
    let k = stored_key(&mut w);
    let g = w.ground_item(KEY, 12, 12);
    assert_eq!(w.handle(&lift(g)), Ok(1));
    let _c = w.cursor_item(CAP);
    assert_eq!(w.handle(&lift(k)), Ok(2));
    assert_eq!(w.mode(k), 0);
}

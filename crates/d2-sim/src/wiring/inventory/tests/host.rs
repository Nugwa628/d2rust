//! Tests for the model's entry points for the other item systems
//! (`host.rs`): the reads on `InvState` follow what the item moves did,
//! and placement, removal, the free, the checks, the targeting reset and
//! the direct 0x9D run the inventory rules on the same inventory.

use super::*;
use crate::items::inventory::InvItem;
use crate::items::moves::MoveUnits;
use crate::wiring::inventory::InvError;

/// The model's reads after the moves: cursor (0x16 to the cursor), the
/// item list and the grid (0x18), a body location (0x1A).
#[test]
fn reads_follow_the_item_moves() {
    let mut w = World::new();
    let p = w.player;
    let k = w.cursor_item(CAP);
    let u = w.unit(k).unwrap();
    assert_eq!(w.state.cursor_of(p), Some(u));
    assert!(w.state.holds(p, u));
    assert!(w.state.items_of(p).is_empty());
    assert!(w.state.body_items(p).is_empty());

    assert_eq!(w.handle(&insert(k, 0, 0, 0)), Ok(0));
    w.drain();
    assert_eq!(w.state.cursor_of(p), None);
    assert_eq!(w.state.items_of(p), [u]);
    assert!(w.state.holds(p, u));

    assert_eq!(w.handle(&lift(k)), Ok(0));
    assert_eq!(w.handle(&body(0x1A, k, 1)), Ok(0));
    w.drain();
    assert_eq!(w.state.body_items(p), [u]);
    assert!(w.state.holds(p, u));
    assert!(w.state.fillers(u).is_empty());
    // Another unit holds nothing.
    assert!(!w.state.holds(u, u));
    assert!(w.state.items_of(UnitId(9999)).is_empty());
}

/// Placing a cursor item (page 0 set first, as every caller does: the
/// pickup left page 0xFF): placed at a free position (the weighted search
/// from the bottom-right corner for a 2 × 2 item of a player: (0, 0)
/// first, `grid.rs`), mode 0, command flag 0x2 with "send", owner
/// refreshed: the next pass sends 0x9C action 4. Without "send": no
/// command flag, nothing in the pass. A ground item is refused (the
/// item must be on the cursor).
#[test]
fn place_runs_section_2_4() {
    let mut w = World::new();
    let p = w.player;
    let k = w.cursor_item(CAP);
    let u = w.unit(k).unwrap();
    w.items.get_mut(u).unwrap().inv_page = 0;
    assert!(w.desk(|d| d.place(p, u, (0, 0), true, true)));
    assert_eq!(w.mode(k), 0);
    let d = w.data(k);
    assert_eq!((d.page, d.x, d.y, d.cmd_flags), (0, 0, 0, 0x2));
    assert_eq!(w.inventory().item_at(2, 0, 0), Some(u));
    assert_eq!(w.inventory().cursor(), None);
    assert_eq!(item_msgs(&w.drain()), [(0x9C, 0x04, k)]);

    let k2 = w.cursor_item(CAP);
    let u2 = w.unit(k2).unwrap();
    w.items.get_mut(u2).unwrap().inv_page = 0;
    assert!(w.desk(|d| d.place(p, u2, (0, 0), true, false)));
    assert_eq!(w.mode(k2), 0);
    assert_eq!(w.data(k2).cmd_flags, 0);
    assert!(item_msgs(&w.drain()).is_empty());
    assert_eq!(w.state.items_of(p), [u, u2]);

    let g = w.ground_item(KEY, 11, 11);
    let gu = w.unit(g).unwrap();
    assert!(!w.desk(|d| d.place(p, gu, (0, 0), true, true)));
    assert_eq!(w.mode(g), 3);
    assert!(!w.state.holds(p, gu));
    assert_eq!(w.state.errors, Vec::new());
}

/// An item in cursor mode that is still in a room (only an
/// item copy from a ground source could be one) is a caller error:
/// recorded, not placed, left in its room.
#[test]
fn placing_an_item_with_a_room_is_a_caller_error() {
    let mut w = World::new();
    let p = w.player;
    let g = w.ground_item(CAP, 11, 11);
    let gu = w.unit(g).unwrap();
    w.units.get_mut(gu).unwrap().mode = 4;
    w.items.get_mut(gu).unwrap().inv_page = 0;
    assert!(!w.desk(|d| d.place(p, gu, (0, 0), true, true)));
    assert!(w.in_room(g));
    assert!(!w.state.holds(p, gu));
    assert_eq!(w.state.errors, [InvError::PlacedWithRoom(gu)]);
}

/// Unlink by unit, then the free: the cells are cleared, the unit
/// and its item data are gone, nothing logged. A second removal finds
/// nothing. Freeing an item still linked unlinks it from the player's
/// inventory first.
#[test]
fn remove_then_free() {
    let mut w = World::new();
    let p = w.player;
    let k = w.cursor_item(CAP);
    assert_eq!(w.handle(&insert(k, 0, 0, 0)), Ok(0));
    w.drain();
    let u = w.unit(k).unwrap();
    assert!(w.desk(|d| d.remove(p, u)));
    assert!(!w.desk(|d| d.remove(p, u)));
    assert_eq!(w.inventory().item_at(2, 0, 0), None);
    assert!(w.state.items_of(p).is_empty());
    w.desk(|d| d.free(u));
    assert!(w.unit(k).is_none());
    assert!(!w.items.contains(u));
    w.desk(|_| ());
    assert!(!w.state.items.contains_key(&u));
    assert_eq!(w.state.errors, Vec::new());

    let k2 = w.cursor_item(CAP);
    assert_eq!(w.handle(&insert(k2, 0, 0, 0)), Ok(0));
    let u2 = w.unit(k2).unwrap();
    w.desk(|d| d.free(u2));
    assert_eq!(w.state.errors, Vec::new());
    assert!(w.state.items_of(p).is_empty());
    assert!(w.unit(k2).is_none());
}

/// The checks by unit: the stored check passes a stored item (0) and refuses a
/// cursor item (1); ground or owned: a ground item within 10 subtiles of
/// the player per axis passes, 11 away is refused, another act is 2; a
/// cursor item passes (owned). The targeting reset clears flag 0x4 on the list's
/// items.
#[test]
fn checks_and_targeting_reset() {
    let mut w = World::new();
    let p = w.player;
    let k = w.cursor_item(CAP);
    assert_eq!(w.desk(|d| d.check_stored(p, k)), 1);
    assert_eq!(w.desk(|d| d.check_ground_or_owned(p, k)), 0);
    assert_eq!(w.handle(&insert(k, 0, 0, 0)), Ok(0));
    w.drain();
    assert_eq!(w.desk(|d| d.check_stored(p, k)), 0);
    assert_eq!(w.desk(|d| d.check_stored(p, 0xDEAD)), 1);

    // The player stands at (10, 10).
    let near = w.ground_item(KEY, 20, 0);
    let far = w.ground_item(KEY, 21, 10);
    assert_eq!(w.desk(|d| d.check_ground_or_owned(p, near)), 0);
    assert_eq!(w.desk(|d| d.check_ground_or_owned(p, far)), 1);
    let nu = w.unit(near).unwrap();
    w.units.get_mut(nu).unwrap().act = 1;
    assert_eq!(w.desk(|d| d.check_ground_or_owned(p, near)), 2);

    let u = w.unit(k).unwrap();
    w.items.get_mut(u).unwrap().flags |= 0x4;
    w.rest.sent.clear();
    w.desk(|d| d.reset_targeting(p));
    assert_eq!(w.items.get(u).unwrap().flags & 0x4, 0);
    // The probed unit is the owner, a player: 0x3F (code 0xFF, the GUID,
    // 0xFFFF) to it.
    let mut want = vec![0x3F, 0xFF];
    want.extend_from_slice(&k.to_le_bytes());
    want.extend_from_slice(&[0xFF, 0xFF]);
    let me = w.desk(|d| d.owner_of(p)).unwrap();
    assert_eq!(w.rest.sent, [(me, want)]);
}

/// The direct 0x9D action 5 of the cube's removal: the stored
/// page set to the shown page first, queued now through
/// `MovePending::send` to the owner: [0x9D, 5, 13, category 0, GUID,
/// owner type 0, the player's GUID, the item bit stream with the flag
/// argument 0x20 and page 3].
#[test]
fn send_item_page_queues_0x9d_now() {
    let mut w = World::new();
    let p = w.player;
    let k = w.cursor_item(CAP);
    assert_eq!(w.handle(&insert(k, 0, 0, 0)), Ok(0));
    w.drain();
    let u = w.unit(k).unwrap();
    let stream = w.desk(|d| d.item_stream(k, 0x20, 3));
    assert!(!stream.is_empty());
    // Page 3 is sent as page + 1 = 4 (3 bits at stream bit 57).
    let page1 = (u32::from_le_bytes(stream[4..8].try_into().unwrap()) >> 25) & 7;
    assert_eq!(page1, 4);
    assert_eq!(w.desk(|d| d.send_item_page(p, u, 0x20, 3)), Ok(()));
    let mut want = vec![0x9D, 5, 13 + stream.len() as u8, 0];
    want.extend_from_slice(&k.to_le_bytes());
    want.push(0);
    want.extend_from_slice(&w.pguid().to_le_bytes());
    want.extend_from_slice(&stream);
    assert_eq!(w.rest.sent, [(w.me(), want)]);
    assert_eq!(w.desk(|d| d.stored_page(k)), 3);
    let _: InvItem = w.data(k);
}

/// S→C 0x22: the desk's
/// `send_skill_quantity` builds the 12 bytes from the unit's type and
/// GUID (quantity's low byte, flag 0 without state 7) and sends them
/// through the rest's transport; a unit without a record sends nothing.
#[test]
fn skill_quantity_sends_0x22() {
    use crate::items::inventory::bookkeeping::EquipWorld;
    let mut w = World::new();
    let (player, me, g) = (w.player, w.me(), w.pguid());
    w.rest.sent.clear();
    w.desk(|d| {
        d.send_skill_quantity(player, 220, 300);
        d.send_skill_quantity(UnitId(9999), 220, 1);
    });
    assert_eq!(
        w.rest.sent,
        [(
            me,
            crate::units::messages::update_item_skill(0, g, 220, 44, false).to_vec()
        )]
    );
}

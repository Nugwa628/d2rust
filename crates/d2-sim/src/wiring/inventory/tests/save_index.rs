//! Tests for the item indices written to the save header, on the inventory
//! state: `save_item_index` and `item_at_save_index`.

use super::*;

/// `save_item_index` is the 1-based list position by GUID; GUID −1, no
/// inventory or not found → 0. `item_at_save_index` maps 0 → −1, a position
/// → its GUID, past the end → −1.
#[test]
fn indices_follow_the_item_list() {
    let mut w = World::new();
    let p = w.player;
    let a = w.cursor_item(CAP);
    assert_eq!(w.handle(&insert(a, 0, 0, 0)), Ok(0));
    w.drain();
    let b = w.cursor_item(KEY);
    assert_eq!(w.handle(&insert(b, 4, 0, 0)), Ok(0));
    w.drain();
    let s = &w.state;
    assert_eq!(s.save_item_index(p, a), Ok(1));
    assert_eq!(s.save_item_index(p, b), Ok(2));
    assert_eq!(s.save_item_index(p, u32::MAX), Ok(0));
    assert_eq!(s.save_item_index(p, 0xDEAD), Ok(0));
    assert_eq!(s.save_item_index(UnitId(9999), a), Ok(0));
    assert_eq!(s.item_at_save_index(p, 0), u32::MAX);
    assert_eq!(s.item_at_save_index(p, 1), a);
    assert_eq!(s.item_at_save_index(p, 2), b);
    assert_eq!(s.item_at_save_index(p, 3), u32::MAX);
    assert_eq!(s.item_at_save_index(UnitId(9999), 1), u32::MAX);
}

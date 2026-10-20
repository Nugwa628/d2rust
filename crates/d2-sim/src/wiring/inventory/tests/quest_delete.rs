//! Tests for `delete_held_item`, which removes a quest item from a player
//! according to the item's mode: stored is freed, equipped is detached (not
//! freed), a belt item is untouched. The cursor case is consumed.

use super::*;

#[test]
fn a_stored_quest_item_is_freed() {
    let mut w = World::new();
    let p = w.player;
    let k = w.cursor_item(CAP);
    let u = w.unit(k).unwrap();
    w.items.get_mut(u).unwrap().inv_page = 0;
    assert!(w.desk(|d| d.place(p, u, (4, 2), false, true)));
    w.drain();
    w.desk(|d| d.delete_held_item(p, u));
    assert!(w.unit(k).is_none());
    assert!(w.state.items_of(p).is_empty());
}

#[test]
fn an_equipped_quest_item_stays_allocated_and_detached() {
    let mut w = World::new();
    let p = w.player;
    let k = w.cursor_item(CAP);
    assert_eq!(w.handle(&body(0x1A, k, 1)), Ok(0));
    w.drain();
    let u = w.unit(k).unwrap();
    assert_eq!(w.state.body_items(p), [u]);
    w.desk(|d| d.delete_held_item(p, u));
    // Not freed; taken off the body, mode 4, not the cursor.
    w.unit(k).expect("still allocated");
    assert!(w.state.body_items(p).is_empty());
    assert_eq!(w.mode(k), 4);
    assert_eq!(w.state.cursor_of(p), None);
    assert!(w.state.items_of(p).is_empty());
}

#[test]
fn a_cursor_quest_item_is_consumed() {
    let mut w = World::new();
    let p = w.player;
    let k = w.cursor_item(CAP);
    let u = w.unit(k).unwrap();
    w.desk(|d| d.delete_held_item(p, u));
    assert!(w.unit(k).is_none());
    assert_eq!(w.state.cursor_of(p), None);
}

#[test]
fn a_belt_quest_item_is_untouched() {
    let mut w = World::new();
    let p = w.player;
    let g = w.ground_item(HP1, 12, 11);
    assert_eq!(w.handle(&pick(g, 0)), Ok(0));
    w.drain();
    assert_eq!(w.mode(g), 2);
    let u = w.unit(g).unwrap();
    w.desk(|d| d.delete_held_item(p, u));
    assert!(w.unit(g).is_some());
    assert_eq!(w.mode(g), 2);
    assert!(w.drain().is_empty());
}

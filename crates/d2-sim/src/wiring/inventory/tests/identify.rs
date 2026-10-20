//! Tests for 0x27 UseItemAction with an identify scroll (provisional
//! effect), and for Cain's identify entry points.

use super::*;
use crate::items::moves::iflag;

/// `record` stored at the cell (x, y) of page 0; drained.
fn stored(w: &mut World, record: usize, x: u32, y: u32) -> Guid {
    let g = w.cursor_item(record);
    assert_eq!(w.handle(&insert(g, x, y, 0)), Ok(0));
    w.drain();
    g
}

fn clear_identified(w: &mut World, g: Guid) {
    let u = w.unit(g).unwrap();
    w.items.get_mut(u).unwrap().flags &= !iflag::IDENTIFIED;
}

/// The first use has armed the item (flag 0x4).
fn arm(w: &mut World, g: Guid) {
    let u = w.unit(g).unwrap();
    w.items.get_mut(u).unwrap().flags |= iflag::TARGETING;
}

/// The scroll identifies an unidentified stored item: flag 0x10 set, the
/// item is sent again (0x9D action 0x15), the scroll is consumed.
#[test]
fn scroll_identifies_a_stored_item() {
    let mut w = World::new();
    let target = stored(&mut w, CAP, 0, 0);
    let scroll = stored(&mut w, ISC, 4, 0);
    clear_identified(&mut w, target);
    arm(&mut w, scroll);
    assert_eq!(w.handle(&msg(0x27, &[target, scroll])), Ok(0));
    assert_ne!(w.data(target).flags & iflag::IDENTIFIED, 0);
    assert!(w.unit(scroll).is_none(), "the scroll is used up");
    let sent = item_msgs(&w.drain());
    assert!(sent.contains(&(0x9D, 0x15, target)), "{sent:?}");
}

/// An item that is already identified is not used on: the scroll stays.
#[test]
fn identified_target_keeps_the_scroll() {
    let mut w = World::new();
    let target = stored(&mut w, CAP, 0, 0);
    let scroll = stored(&mut w, ISC, 4, 0);
    let _ = w.handle(&msg(0x27, &[target, scroll]));
    assert!(w.unit(scroll).is_some());
    assert!(!item_msgs(&w.drain()).contains(&(0x9D, 0x15, target)));
}

/// Another item (a key) used on an item identifies nothing.
#[test]
fn only_identify_items_identify() {
    let mut w = World::new();
    let target = stored(&mut w, CAP, 0, 0);
    let other = stored(&mut w, KEY, 4, 0);
    let scroll = stored(&mut w, ISC, 5, 0);
    clear_identified(&mut w, target);
    let p = Owner::player(w.pguid());
    assert!(!w.desk(|d| d.use_identify(p, Owner::item(target), other)));
    assert_eq!(w.data(target).flags & iflag::IDENTIFIED, 0);
    // A player target is not an item to identify.
    assert!(!w.desk(|d| d.use_identify(p, p, scroll)));
    assert!(w.desk(|d| d.use_identify(p, Owner::item(target), scroll)));
}

/// Cain's walk: the entries list the stored item with its page and
/// flags; `identify_unit` sets flag 0x10 once and queues the update.
#[test]
fn cain_entries_and_identify_unit() {
    let mut w = World::new();
    let target = stored(&mut w, CAP, 0, 0);
    clear_identified(&mut w, target);
    let tu = w.unit(target).unwrap();
    let pu = w.player;
    let entries = w.desk(|d| d.npc_entries(pu));
    let e = entries.iter().find(|e| e.item == tu).expect("listed");
    assert_eq!(e.place, crate::world::npc::Place::Grid(0));
    assert_eq!(e.flags & iflag::IDENTIFIED, 0);
    assert!(w.desk(|d| d.identify_unit(pu, tu)));
    assert!(!w.desk(|d| d.identify_unit(pu, tu)), "already identified");
    assert_ne!(w.data(target).flags & iflag::IDENTIFIED, 0);
    assert!(item_msgs(&w.drain()).contains(&(0x9D, 0x15, target)));
}

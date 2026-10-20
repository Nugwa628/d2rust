//! Tests for the item-use dispatcher on the desk (`inventory/town_portal.rs`):
//! the Town Portal entry records a request outside a town and is refused
//! in one (failure reset, S→C 0x7C, nothing spent); items without an
//! entry are not taken.

use super::*;
use crate::items::moves::{MovePending, MoveUnits};

const QUANTITY: u16 = 70;
/// Item flag 0x4: the "armed" flag that using the item sets.
const ARMED: u32 = 0x4;

/// A scroll used from the inventory: entry 2, the request names the
/// player, the item is armed and not touched otherwise (its cost is the
/// move handler's).
#[test]
fn scroll_use_records_a_portal_request() {
    let mut w = World::new();
    let s = w.cursor_item(TSC);
    let (me, pu) = (w.me(), w.player);
    assert_eq!(w.desk(|d| d.use_entry(s)), Some((2, -1)));
    assert!(w.desk(|d| d.use_item_at(me, s, 0, 0)));
    assert_eq!(w.desk(|d| d.take_portal_requests()), [pu]);
    assert!(w.desk(|d| d.take_portal_requests()).is_empty());
    assert_ne!(w.desk(|d| d.item_flags(s)) & ARMED, 0);
}

/// The tome's use leaves its charge alone: what a 1 costs is the
/// caller's.
#[test]
fn tome_use_does_not_spend_its_charge() {
    let mut w = World::new();
    let t = w.cursor_item(TBK);
    let tu = w.unit(t).unwrap();
    w.set_stat(tu, QUANTITY, 2);
    let me = w.me();
    let it = crate::items::moves::Owner::item(t);
    assert!(w.desk(|d| d.use_item_at(me, t, 0, 0)));
    assert_eq!(w.desk(|d| d.stat(it, QUANTITY)), 2);
    assert_eq!(w.desk(|d| d.take_portal_requests()).len(), 1);
}

/// In a town the cast refuses: no request,
/// the failure reset clears the armed flag with S→C 0x3F (`3F FF`, GUID,
/// `FF FF`), then S→C 0x7C for the item; the use reports 0.
#[test]
fn a_town_refuses_the_cast_and_resets() {
    let mut w = World::new();
    w.rest.in_town = true;
    let s = w.cursor_item(TSC);
    // Into the backpack (C→S 0x18 at (0, 0), page 0): the reset walks
    // the inventory's item list, which never holds the cursor item.
    let mut put = vec![0x18];
    for v in [s, 0, 0, 0] {
        put.extend_from_slice(&v.to_le_bytes());
    }
    assert_eq!(w.handle(&put), Ok(0));
    let me = w.me();
    w.rest.sent.clear();
    assert!(!w.desk(|d| d.use_item_at(me, s, 0, 0)));
    assert!(w.desk(|d| d.take_portal_requests()).is_empty());
    assert_eq!(w.desk(|d| d.item_flags(s)) & ARMED, 0);
    let mut reset = vec![0x3F, 0xFF];
    reset.extend_from_slice(&s.to_le_bytes());
    reset.extend_from_slice(&[0xFF, 0xFF]);
    let mut used = vec![0x7C, 4];
    used.extend_from_slice(&s.to_le_bytes());
    assert_eq!(w.rest.sent, [(me, reset), (me, used)]);
}

/// Another item has no use entry and is not taken.
#[test]
fn other_items_are_left_alone() {
    let mut w = World::new();
    let k = w.cursor_item(KEY);
    let me = w.me();
    assert_eq!(w.desk(|d| d.use_entry(k)), None);
    assert!(!w.desk(|d| d.use_item_at(me, k, 0, 0)));
    assert!(w.desk(|d| d.take_portal_requests()).is_empty());
}

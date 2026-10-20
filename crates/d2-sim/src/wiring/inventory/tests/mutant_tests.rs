//! Mutation-testing additions for the model's host API (`host.rs`): checks
//! that fail when a read returns a fixed answer.

use super::*;
use crate::items::inventory::UnitKind;

/// A socketed item owns an inventory, and its fillers are that inventory's
/// item list in link order. `fillers` reads it back (the cube's `socketed`),
/// and the fillers are not the player's items.
#[test]
fn fillers_are_the_items_own_inventory_list() {
    let mut w = World::new();
    let p = w.player;
    let k = w.cursor_item(CAP);
    assert_eq!(w.handle(&insert(k, 0, 0, 0)), Ok(0));
    w.drain();
    let target = w.unit(k).unwrap();
    let g1 = w.ground_item(KEY, 11, 11);
    let g2 = w.ground_item(KEY, 12, 12);
    let (f1, f2) = (w.unit(g1).unwrap(), w.unit(g2).unwrap());
    let tg = w.units.get(target).unwrap().guid;
    // The target's inventory, created on the first filler.
    w.state.add_inventory(target, UnitKind::Item, tg);
    w.desk(|d| {
        let o = d.owner_of(target).unwrap();
        d.with_inv(o, |inv, d| {
            inv.link(d, f1, None);
            inv.link(d, f2, None);
        })
    })
    .expect("the target's inventory");
    assert_eq!(w.state.fillers(target), [f1, f2]);
    assert_eq!(w.state.items_of(p), [target]);
    assert!(!w.state.holds(p, f1));
}

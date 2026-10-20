//! Tests for the inactive item record with socketed children: the children are
//! stored after their parent and restored into it.

use super::objects::{create, drop_fx, CHEST};
use super::*;
use crate::items::bitstream::read;
use crate::world::objects::Operate;

fn chest_item(fx: &mut Fx, x: i32) -> UnitId {
    let o = create(fx, CHEST, x);
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 10, 10);
    let op = Operate {
        object: o,
        operator: Some(p),
        class: CHEST as u16,
        operate_fn: 4,
    };
    fx.sim
        .objects(&mut fx.game, |_, _, w| {
            crate::world::objects::ChestWorld::chest_drop(w, &op, 0)
        })
        .flatten()
        .expect("an item")
}

#[test]
fn a_socketed_child_is_stored_after_its_parent_and_restored_into_it() {
    let mut fx = drop_fx();
    let parent = chest_item(&mut fx, 20);
    let child = chest_item(&mut fx, 30);
    fx.sim.sys.hooks.x.sockets.insert(parent, vec![child]);
    let s = &mut fx.sim.sys;
    let rec = View::of(&mut s.units, &mut s.stats, &s.data, &mut s.hooks)
        .item_record_of(parent)
        .expect("a record");
    let t = s.hooks.object_drops.as_ref().unwrap().tables.clone();
    let entry = read::read_save_entry(&rec.bytes, &t.items).expect("reads");
    // A compact record has no filled count: then no child follows.
    let n = usize::from(!(entry.item.item.compact || entry.item.item.alt));
    assert_eq!(entry.children.len(), n);
    assert_eq!(entry.len, rec.bytes.len());
    if n == 0 {
        return;
    }
    let room = fx.a;
    let s = &mut fx.sim.sys;
    View::of(&mut s.units, &mut s.stats, &s.data, &mut s.hooks).restore_item_record(
        &mut fx.game,
        room,
        &rec,
        0,
    );
    let restored = &fx.sim.sys.hooks.x.socket_restored;
    assert_eq!(restored.len(), 1);
    let (p2, c2) = restored[0];
    assert!(p2 != parent && c2 != child && p2 != c2);
    assert_eq!(fx.game.lists.unit(p2).unwrap().room(), Some(room));
    assert_eq!(fx.game.lists.unit(c2).unwrap().room(), None);
}

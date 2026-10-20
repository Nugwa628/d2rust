//! Tests for 0x21 StackItems and 0x22 UnstackItems on real stats.

use super::*;
use crate::items::moves::stat::{DURABILITY, QUANTITY};
use crate::units::messages::update_item_stat;

/// `dst` stored in page 0 with quantity `qd`, `src` on the cursor with
/// quantity `qs`; drained.
fn pair(w: &mut World, record: usize, qd: i32, qs: i32) -> (Guid, Guid) {
    let dst = w.cursor_item(record);
    assert_eq!(w.handle(&insert(dst, 0, 0, 0)), Ok(0));
    w.drain();
    let src = w.cursor_item(record);
    for (g, q) in [(dst, qd), (src, qs)] {
        let u = w.unit(g).unwrap();
        w.set_stat(u, QUANTITY, q);
    }
    (dst, src)
}

fn qty(w: &World, g: Guid) -> i32 {
    w.stats.unit_total(w.unit(g).unwrap(), QUANTITY, 0)
}

/// A sum over the max stack (keys: `maxstack` 12 + stat 254 0): dst
/// := 12, src := the rest, each announced (0x3E, seam), dst item flag
/// 0x8, command flag 0x100 → 0x9C action 0xA. Both stay.
#[test]
fn stack_over_the_max_fills_dst() {
    let mut w = World::new();
    let (dst, src) = pair(&mut w, KEY, 8, 7);
    w.rest.log.clear();
    assert_eq!(w.handle(&msg(0x21, &[src, dst])), Ok(0));
    assert_eq!((qty(&w, dst), qty(&w, src)), (12, 3));
    // S→C 0x3E built by the desk (`units::messages::update_item_stat`)
    // from the new base quantities, through the rest's transport.
    let me = w.me();
    assert_eq!(
        w.rest.sent,
        [
            (me, update_item_stat(dst, QUANTITY, 12, 0)),
            (me, update_item_stat(src, QUANTITY, 3, 0)),
        ]
    );
    assert_eq!(w.rest.log, Vec::<String>::new());
    assert_eq!(w.data(dst).flags & 0x8, 0x8);
    assert_eq!(w.inventory().cursor(), w.unit(src));
    assert_eq!(item_msgs(&w.drain()), [(0x9C, 0x0A, dst)]);
}

/// Merge branch: throwing knives have durability (durability 20, a stat
/// list, stat 152 < 1): src's lower stat 72 lowers dst's (stat 72
/// `durability`), dst := q_s + q_d, cursor cleared, S→C 0x42 naming the
/// player (`42 00 <player>`), src freed (its unit is gone); dst 0x9C
/// action 0xA.
#[test]
fn stack_merge_frees_the_source() {
    let mut w = World::new();
    let (dst, src) = pair(&mut w, KNIFE, 10, 5);
    let (du, su) = (w.unit(dst).unwrap(), w.unit(src).unwrap());
    w.set_stat(du, DURABILITY, 20);
    w.set_stat(su, DURABILITY, 7);
    assert_eq!(w.handle(&msg(0x21, &[src, dst])), Ok(0));
    assert_eq!(qty(&w, dst), 15);
    assert_eq!(w.stats.unit_total(du, DURABILITY, 0), 7);
    assert_eq!(w.inventory().cursor(), None);
    assert_eq!(w.unit(src), None, "freed");
    assert!(!w.state.items.contains_key(&su));
    let me = w.me();
    let mut clear = vec![0x42, me.ty];
    clear.extend_from_slice(&me.guid.to_le_bytes());
    assert_eq!(
        w.rest.sent,
        [
            (me, update_item_stat(dst, DURABILITY, 7, 0)),
            (me, update_item_stat(dst, QUANTITY, 15, 0)),
            (me, clear)
        ]
    );
    assert_eq!(item_msgs(&w.drain()), [(0x9C, 0x0A, dst)]);
    assert!(w.state.errors.is_empty());
}

/// Keys have no durability; durability gates only the stat-72 step, so they
/// merge (keys 3 + 4): dst := 7, src freed, dst marked (0x9C action 0xA).
/// src = dst → 3; items of different classes fail the same-class check
/// → 0. 0x22 on an owned item → 3, on another → 1.
#[test]
fn stack_without_durability_and_unstack() {
    let mut w = World::new();
    let (dst, src) = pair(&mut w, KEY, 3, 4);
    assert_eq!(w.handle(&msg(0x21, &[src, src])), Ok(3));
    assert_eq!(w.handle(&msg(0x21, &[src, dst])), Ok(0));
    assert_eq!(qty(&w, dst), 7);
    assert_eq!(w.unit(src), None, "the source is freed");
    assert_eq!(item_msgs(&w.drain()), [(0x9C, 0x0A, dst)]);
    let src = w.cursor_item(KEY);
    let other = w.ground_item(KNIFE, 12, 12);
    assert_eq!(w.handle(&msg(0x21, &[src, other])), Ok(1), "not owned");
    assert_eq!(w.handle(&msg(0x22, &[dst])), Ok(3));
    assert_eq!(w.handle(&msg(0x22, &[other])), Ok(1));
}

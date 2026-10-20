//! Tests for C→S 0x28 SocketItem on the real units:
//! the filler joins the target's own inventory in mode 6, the target is
//! marked changed and listed for the client.

use super::*;

const SOCKETED: u32 = 0x800;

/// A stored sword with `sockets` sockets, identified, and a gem on the
/// cursor; returns (sword, gem).
fn setup(sockets: i32) -> (World, Guid, Guid) {
    let mut w = World::new();
    let sword = w.cursor_item(SWORD);
    assert_eq!(w.handle(&insert(sword, 0, 0, 0)), Ok(0));
    w.drain();
    let u = w.unit(sword).unwrap();
    w.items.get_mut(u).unwrap().flags |= SOCKETED;
    w.set_stat(u, super::super::inv_world::STAT_SOCKETS, sockets);
    let gem = w.cursor_item(GEM);
    (w, sword, gem)
}

#[test]
fn gem_goes_into_a_socketed_item() {
    let (mut w, sword, gem) = setup(1);
    assert_eq!(w.handle(&msg(0x28, &[gem, sword])), Ok(0));
    assert_eq!(w.mode(gem), 6, "socketed");
    let su = w.unit(sword).unwrap();
    let gu = w.unit(gem).unwrap();
    assert_eq!(w.state.inventories[&su].items(), [gu]);
    assert_eq!(w.inventory().cursor(), None);
    let out = w.drain();
    assert!(
        item_msgs(&out).iter().any(|&(_, _, g)| g == sword),
        "the target is sent to the client"
    );
}

#[test]
fn full_or_plain_target_refuses() {
    let (mut w, sword, gem) = setup(0);
    assert_eq!(w.handle(&msg(0x28, &[gem, sword])), Ok(0));
    assert_eq!(
        w.mode(gem),
        4,
        "no free socket: the gem stays on the cursor"
    );
    let (mut w, sword, gem) = setup(1);
    let g2 = {
        // Fill the socket, then try a second gem.
        assert_eq!(w.handle(&msg(0x28, &[gem, sword])), Ok(0));
        w.drain();
        w.cursor_item(GEM)
    };
    assert_eq!(w.handle(&msg(0x28, &[g2, sword])), Ok(0));
    assert_eq!(w.mode(g2), 4);
}

#[test]
fn gem_properties_reach_the_target() {
    use crate::items::tables::{GemRec, PropRec, PropSlot, PropertyRec};
    const STAT: u16 = 31;
    let (mut w, sword, gem) = setup(1);
    let mut p = PropertyRec::default();
    p.slots[0] = PropSlot {
        func: 1,
        stat: STAT,
        set: 0,
        val: 0,
    };
    w.tables.properties = vec![p];
    let block = [
        PropRec {
            code: 0,
            param: 0,
            min: 5,
            max: 5,
        },
        PropRec::NONE,
        PropRec::NONE,
    ];
    w.tables.gems = vec![GemRec { mods: [block; 3] }];
    let su = w.unit(sword).unwrap();
    let before = w.stats.unit_total(su, STAT, 0);
    assert_eq!(w.handle(&msg(0x28, &[gem, sword])), Ok(0));
    // The gem keeps its property list; its root is linked under the sword,
    // and the sword's own main list lookup (the stream writer's) does not
    // find it.
    let gu = w.unit(gem).unwrap();
    let l = crate::wiring::economy::find_list(&w.stats, gu, crate::items::ListKey::ITEM)
        .expect("the gem's list stays on the gem");
    assert_eq!(w.stats.base(l, STAT, 0), 5);
    assert_eq!(w.stats.unit_total(su, STAT, 0), before + 5);
}

#[test]
fn runeword_completes_when_the_last_rune_goes_in() {
    use crate::items::tables::{PropRec, PropSlot, PropertyRec, RuneRec};
    use crate::items::ListKey;
    const STAT: u16 = 52;
    let (mut w, sword, g1) = setup(2);
    let mut p = PropertyRec::default();
    p.slots[0] = PropSlot {
        func: 1,
        stat: STAT,
        set: 0,
        val: 0,
    };
    w.tables.properties = vec![p];
    w.tables.items[SWORD].hasinv = 1;
    let mut props = [PropRec::NONE; 7];
    props[0] = PropRec {
        code: 0,
        param: 0,
        min: 9,
        max: 9,
    };
    w.tables.runes = vec![RuneRec {
        complete: 1,
        itype: [T_WEAP as i16, 0, 0, 0, 0, 0],
        runes: [GEM as i32, GEM as i32, 0, 0, 0, 0],
        name_id: 0x0777,
        props,
        ..RuneRec::default()
    }];
    let su = w.unit(sword).unwrap();
    assert_eq!(w.handle(&msg(0x28, &[g1, sword])), Ok(0));
    assert_eq!(w.desk(|d| d.runeword_name(su)), 0xFFFF, "one rune: no word");
    let g2 = w.cursor_item(GEM);
    assert_eq!(w.handle(&msg(0x28, &[g2, sword])), Ok(0));
    assert_eq!(w.desk(|d| d.runeword_name(su)), 0x0777);
    let k = ListKey {
        state: 171,
        flags: 0x40,
    };
    let l = crate::wiring::economy::find_list(&w.stats, su, k).expect("runeword list");
    assert_eq!(w.stats.base(l, STAT, 0), 9);
}

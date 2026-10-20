//! Tests for 0x1A EquipItem, 0x1B Swap2HandedItem, 0x1C RemoveBodyItem
//! and 0x1D SwapCursorWithBody on the real body grid, plus the stat
//! effects of wearing items and of the weapon switch.

use super::*;

/// Equips `record` from the cursor at `loc` through 0x1A; drained.
fn equipped(w: &mut World, record: usize, loc: u32) -> Guid {
    let g = w.cursor_item(record);
    assert_eq!(w.handle(&body(0x1A, g, loc)), Ok(0));
    w.drain();
    g
}

/// 0x1A: a cap to the head: grid 0 cell 1, mode 1, body location
/// 1, page 0xFF, command flag 0x8 and item flag 0x1 → 0x9D action 6 to
/// all. 0x1C: off again: cursor, mode 4, command flag 0x10 → 0x9D
/// action 8.
#[test]
fn equip_and_unequip_a_helm() {
    let mut w = World::new();
    let c = w.cursor_item(CAP);
    let u = w.unit(c).unwrap();
    assert_eq!(w.handle(&body(0x1A, c, 11)), Ok(2), "location ∉ 1..10");
    assert_eq!(w.handle(&body(0x1A, c, 1)), Ok(0));
    assert_eq!(w.mode(c), 1);
    assert_eq!(w.inventory().body_item(1), Some(u));
    assert_eq!(w.inventory().cursor(), None);
    let d = w.data(c);
    assert_eq!(
        (d.body_loc, d.page, d.cmd_flags, d.node_kind),
        (1, 0xFF, 0x8, 3)
    );
    assert_eq!(d.flags & 0x1, 0x1);
    assert_eq!(item_msgs(&w.drain()), [(0x9D, 0x06, c)]);

    assert_eq!(w.handle(&unequip(1)), Ok(0));
    assert_eq!(w.mode(c), 4);
    assert_eq!(w.inventory().body_item(1), None);
    assert_eq!(w.inventory().cursor(), Some(u));
    assert_eq!(w.data(c).cmd_flags, 0x10);
    assert_eq!(item_msgs(&w.drain()), [(0x9D, 0x08, c)]);
    assert_eq!(w.handle(&unequip(1)), Ok(0), "cursor present: nothing");
    assert!(w.state.errors.is_empty());
}

/// With requirements: strength 10 < reqstr 50 → the equip check gives 0,
/// the equip refuses without output → 0; the item stays on the cursor.
#[test]
fn equip_refused_by_strength() {
    let mut w = World::new();
    let h = w.cursor_item(HEAVY_CAP);
    assert_eq!(w.handle(&body(0x1A, h, 1)), Ok(0));
    assert_eq!(w.mode(h), 4);
    assert_eq!(w.inventory().body_item(1), None);
    let p = w.player;
    w.set_stat(p, 0, 50);
    assert_eq!(w.handle(&body(0x1A, h, 1)), Ok(0));
    assert_eq!(w.mode(h), 1, "strength 50 meets reqstr 50");
}

/// 0x1B: a two-handed sword onto the right hand while the left holds a
/// shield: the equip check gives 2; the shield leaves the body (mode 4,
/// not listed), the sword goes to location 4 with command flag 0x10000 → 0x9D
/// action 7. 0x1C takes it off again (0x9D action 8).
#[test]
fn two_handed_swap_and_removal_from_the_other_hand() {
    let mut w = World::new();
    let s = equipped(&mut w, SHIELD, 5);
    let t = w.cursor_item(TWO_HANDER);
    w.inv.items[TWO_HANDER].twohanded = 1;
    let (su, tu) = (w.unit(s).unwrap(), w.unit(t).unwrap());

    assert_eq!(w.handle(&body(0x1A, t, 4)), Ok(0), "§4.3 gives 2 ≠ 1");
    assert_eq!(w.mode(t), 4);
    assert_eq!(w.handle(&body(0x1B, t, 3)), Ok(3), "location ∉ {{4, 5}}");
    assert_eq!(w.handle(&body(0x1B, t, 4)), Ok(0));
    assert_eq!(w.inventory().body_item(4), Some(tu));
    assert_eq!(w.inventory().body_item(5), None);
    // The shield stays the cursor item: 0x1B does not clear the cursor.
    assert_eq!(w.inventory().cursor(), Some(su));
    assert_eq!(w.mode(s), 4);
    assert!(!w.inventory().contains(su));
    assert_eq!(w.mode(t), 1);
    assert_eq!(w.data(t).cmd_flags, 0x10000);
    assert_eq!(w.data(s).node_grid, 0, "unlinked");
    assert_eq!(item_msgs(&w.drain()), [(0x9D, 0x07, t)]);

    // The equip check with no new item on the empty left hand: 4, and
    // `item_to_remove` picks the two-handed sword of the other hand.
    let me = w.me();
    use crate::items::moves::InventoryOps;
    // 0x1C needs an empty cursor: the shield leaves it.
    w.desk(|d| d.set_cursor(me, None));
    assert_eq!(w.inventory().cursor(), None);
    assert_eq!(w.desk(|d| d.equip_check(me, 5, None, false)), 4);
    assert_eq!(w.desk(|d| d.item_to_remove(me, 5)), Some(t));
    // 0x1C refuses the empty left hand before the equip check ("empty
    // location → 0"), so the check's result 4 is not reached from it; the
    // right hand gives 3.
    assert_eq!(w.handle(&unequip(5)), Ok(0));
    assert_eq!(w.inventory().body_item(4), Some(tu));
    assert_eq!(w.handle(&unequip(4)), Ok(0));
    assert_eq!(w.inventory().body_item(4), None);
    assert_eq!(w.inventory().cursor(), Some(tu));
    assert_eq!(item_msgs(&w.drain()), [(0x9D, 0x08, t)]);
    assert!(w.state.errors.is_empty());
}

/// 0x1B: with a one-handed sword in the left hand nothing blocks (the
/// equip check gives 1, not 2) → 3; with an empty other hand → 3.
#[test]
fn two_handed_swap_needs_a_blocking_other_hand() {
    let mut w = World::new();
    let t = w.cursor_item(TWO_HANDER);
    w.inv.items[TWO_HANDER].twohanded = 1;
    assert_eq!(w.handle(&body(0x1B, t, 4)), Ok(3), "other hand empty");
    let mut w = World::new();
    let _s = equipped(&mut w, SWORD, 5);
    let n = w.cursor_item(SWORD);
    assert_eq!(w.handle(&body(0x1B, n, 4)), Ok(3), "barbarian: compatible");
    assert_eq!(w.mode(n), 4);
}

/// 0x1D: a cap on the cursor over an equipped cap: the equip check gives
/// 5; the old cap to the cursor (item flags 0x80 | 0x1, command flag
/// 0x20), the new one to the head (item
/// flags 0x40 | 0x1, command flag 0x20); both 0x9D action 9 in update-list
/// order. Empty location → 1.
#[test]
fn swap_cursor_with_body() {
    let mut w = World::new();
    let e = equipped(&mut w, CAP, 1);
    let n = w.cursor_item(CAP);
    assert_eq!(w.handle(&body(0x1D, n, 9)), Ok(1), "feet empty");
    assert_eq!(w.handle(&body(0x1D, n, 1)), Ok(0));
    assert_eq!(w.inventory().body_item(1), w.unit(n));
    assert_eq!(w.inventory().cursor(), w.unit(e));
    assert_eq!((w.mode(e), w.mode(n)), (4, 1));
    assert_eq!(w.data(e).flags & 0x81, 0x81);
    assert_eq!(w.data(n).flags & 0x41, 0x41);
    assert_eq!((w.data(e).cmd_flags, w.data(n).cmd_flags), (0x20, 0x20));
    assert_eq!(item_msgs(&w.drain()), [(0x9D, 0x09, e), (0x9D, 0x09, n)]);
}

/// D2RUST's own rule, not yet checked in play: a worn item's stat list is attached to
/// the wearer, so its base damage reaches the wearer's stats (21 / 22),
/// and goes with the item when it comes off.
#[test]
fn a_worn_weapon_gives_its_damage_to_the_wearer() {
    const MIN: u16 = 21;
    const MAX: u16 = 22;
    let mut w = World::new();
    w.state.link_item_stats = true;
    let p = w.player;
    let c = w.cursor_item(SWORD);
    let u = w.unit(c).unwrap();
    w.set_stat(u, MIN, 3);
    w.set_stat(u, MAX, 9);
    assert_eq!(w.stats.unit_total(p, MAX, 0), 0);
    assert_eq!(w.handle(&body(0x1A, c, 4)), Ok(0));
    assert_eq!(w.mode(c), 1);
    assert_eq!(w.stats.unit_total(p, MIN, 0), 3);
    assert_eq!(w.stats.unit_total(p, MAX, 0), 9);
    assert_eq!(w.handle(&unequip(4)), Ok(0));
    assert_eq!(w.stats.unit_total(p, MIN, 0), 0);
    assert_eq!(w.stats.unit_total(p, MAX, 0), 0);
}

/// A gem socketed into a worn item reaches the wearer (the filler's list
/// hangs on the item's list, which hangs on the wearer's).
#[test]
fn a_socketed_gem_reaches_the_wearer() {
    use crate::items::tables::{GemRec, PropRec, PropSlot, PropertyRec};
    const STAT: u16 = 31;
    let mut w = World::new();
    w.state.link_item_stats = true;
    let p = w.player;
    let sword = equipped(&mut w, SWORD, 4);
    let su = w.unit(sword).unwrap();
    w.items.get_mut(su).unwrap().flags |= 0x800;
    w.set_stat(su, super::super::inv_world::STAT_SOCKETS, 1);
    let mut pr = PropertyRec::default();
    pr.slots[0] = PropSlot {
        func: 1,
        stat: STAT,
        set: 0,
        val: 0,
    };
    w.tables.properties = vec![pr];
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
    let gem = w.cursor_item(GEM);
    let before = w.stats.unit_total(p, STAT, 0);
    assert_eq!(w.handle(&msg(0x28, &[gem, sword])), Ok(0));
    assert_eq!(w.mode(gem), 6);
    assert_eq!(w.stats.unit_total(p, STAT, 0), before + 5);
    assert_eq!(w.handle(&unequip(4)), Ok(0));
    assert_eq!(w.stats.unit_total(p, STAT, 0), before);
}

/// Defense (stat 31) of the player's totals.
fn defense(w: &World) -> i32 {
    w.stats.unit_total(w.player, 31, 0)
}

// A charm that left page 0 is unlinked and stops counting.
#[test]
fn a_charm_in_the_inventory_counts_and_stops_when_picked_up() {
    let mut w = World::new();
    w.state.link_item_stats = true;
    // Created as a box (a magic charm needs the affix tables), then
    // turned into the charm record.
    let c = w.ground_item(BOX, 11, 11);
    let cu = w.unit(c).unwrap();
    w.items.get_mut(cu).unwrap().record = CHARM;
    assert_eq!(w.handle(&pick(c, 1)), Ok(0));
    w.drain();
    w.stats.unit_add(&mut w.hooks, cu, 31, 5, 0);
    let before = defense(&w);
    assert_eq!(w.handle(&insert(c, 0, 0, 0)), Ok(0));
    w.drain();
    assert_eq!(w.mode(c), 0);
    assert_eq!(defense(&w), before + 5, "the charm's stats count");
    assert_eq!(w.handle(&lift(c)), Ok(0));
    w.drain();
    assert_eq!(defense(&w), before, "off again on the cursor");
    assert_eq!(w.handle(&insert(c, 1, 0, 0)), Ok(0));
    w.drain();
    assert_eq!(defense(&w), before + 5);
    assert!(w.state.errors.is_empty(), "{:?}", w.state.errors);
}

/// The weapon switch (0x60) trades the
/// hands with body locations 11 / 12: the swap set's damage reaches the
/// wearer, the other set's goes, and every moved item is marked for the
/// next update (command flag 0x200000, item flag 0x40 into the hands,
/// 0x80 out). S→C 0x97 is the host's.
#[test]
fn the_weapon_switch_trades_the_hands_with_the_swap_set() {
    use crate::items::inventory::cmd;
    const MAX: u16 = 22;
    let mut w = World::new();
    w.state.link_item_stats = true;
    let p = w.player;
    let owner = Owner::player(w.pguid());
    let switch = |w: &mut World| {
        w.desk(|d| d.swap_weapon_sets(owner))
            .is_some_and(|s| s.ok && s.saved == [(0, -1); 2])
    };
    // Set 1 is in the hands (max damage 9); set 2 (max 20) was put on
    // the swap slots by an earlier switch.
    let second = equipped(&mut w, SWORD, 4);
    let su = w.unit(second).unwrap();
    w.set_stat(su, MAX, 20);
    assert!(switch(&mut w));
    assert_eq!(w.data(second).body_loc, 11);
    assert_eq!(w.data(second).flags & 0x80, 0x80, "item flag 0x80 out");
    assert_eq!(w.stats.unit_total(p, MAX, 0), 0, "off the hands, no effect");
    let first = equipped(&mut w, SWORD, 4);
    let fu = w.unit(first).unwrap();
    w.set_stat(fu, MAX, 9);
    w.drain();
    assert_eq!(w.stats.unit_total(p, MAX, 0), 9);
    // The switch: the second set takes the hands.
    assert!(switch(&mut w));
    assert_eq!(w.data(second).body_loc, 4);
    assert_eq!(w.data(second).flags & 0x40, 0x40, "item flag 0x40 in");
    assert_eq!(w.data(first).body_loc, 11);
    for i in [first, second] {
        assert_ne!(w.data(i).cmd_flags & cmd::WEAPON_SWITCH, 0);
        assert_eq!(w.data(i).mode, crate::items::moves::mode::EQUIPPED);
    }
    assert_eq!(w.stats.unit_total(p, MAX, 0), 20);
    assert!(
        !w.rest.sent.iter().any(|(_, b)| b == &[0x97]),
        "0x97 is the host's"
    );
    // And back.
    assert!(switch(&mut w));
    assert_eq!(w.data(first).body_loc, 4);
    assert_eq!(w.stats.unit_total(p, MAX, 0), 9);
    assert!(w.state.errors.is_empty(), "{:?}", w.state.errors);
}

/// An item of the swap table not in mode
/// 1 makes out 1 with result 0 before anything moves; the skill sets are
/// already traded.
#[test]
fn a_switch_item_not_in_mode_1_fails_before_any_move() {
    let mut w = World::new();
    let owner = Owner::player(w.pguid());
    let sword = equipped(&mut w, SWORD, 4);
    w.desk(|d| {
        crate::items::moves::MoveUnits::set_mode(d, sword, crate::items::moves::mode::CURSOR)
    });
    let s = w.desk(|d| d.swap_weapon_sets(owner)).expect("a player");
    assert!(!s.ok);
    assert_eq!(w.data(sword).body_loc, 4, "nothing moved");
}

/// With the equipment rules the desk answers the hands-compatible and
/// quiver-kind questions from the tables: a barbarian holds a
/// 1-or-2-handed sword beside a shield, and a quiver-type item is not
/// auto-equipped without a bow. Without the rules both stay the rest's
/// (false).
#[test]
fn the_rules_answer_one_or_two_handed_and_quivers_from_the_tables() {
    use crate::items::inventory::{equip, InvWorld};
    let mut t = inv_tables();
    // `2hs `: two-handed and `1or2handed` (record 3); `tkf ` a quiver type.
    t.items[3].twohanded = 1;
    t.items[3].onetwohanded = 1;
    let tkni = &mut t.itemtypes[usize::from(T_TKNI)];
    tkni.quiver = 1;
    tkni.body = 1;
    tkni.bodyloc1 = 4;
    tkni.bodyloc2 = 5;
    let mut w = World::with_tables(t);
    let sword = w.ground_item(3, 12, 12);
    let shield = w.ground_item(4, 12, 13);
    let knives = w.ground_item(8, 12, 14);
    let (s, b, k) = (
        w.unit(sword).unwrap(),
        w.unit(shield).unwrap(),
        w.unit(knives).unwrap(),
    );
    let p = w.player;
    for rules in [false, true] {
        w.state.equip_rules = rules;
        let (pair, quiver) = w.desk(|d| {
            let tables = d.tables;
            (
                equip::hands_compatible(d, tables, p, Some(s), Some(b)),
                InvWorld::quiver_kind(d, k),
            )
        });
        assert_eq!((pair, quiver), (rules, rules), "rules {rules}");
    }
    // No bow in either hand → no location.
    let me = w.me();
    let l = w.desk(|d| {
        let tables = d.tables;
        d.with_inv(me, |inv, d| {
            equip::auto_equip_location(inv, d, tables, k, false)
        })
    });
    assert_eq!(l.flatten(), None);
}

/// Every body slot and item family preserves both identities, including stacks.
#[test]
fn different_equipment_swaps_and_the_cursor_can_be_placed() {
    for loc in 1..=10 {
        for record in [CAP, SWORD, TWO_HANDER, SHIELD, KNIFE, HP1] {
            let mut w = World::new();
            // Give each synthetic family the slot under test. HP1 models a
            // beltable throwing potion that must follow the body-swap path.
            let ty = w.inv.items[record].type_ as usize;
            w.inv.itemtypes[ty].body = 1;
            w.inv.itemtypes[ty].bodyloc1 = loc;
            w.inv.itemtypes[ty].bodyloc2 = loc;
            let old_record = if record == CAP { SWORD } else { CAP };
            let old_ty = w.inv.items[old_record].type_ as usize;
            w.inv.itemtypes[old_ty].body = 1;
            w.inv.itemtypes[old_ty].bodyloc1 = loc;
            w.inv.itemtypes[old_ty].bodyloc2 = loc;
            let old = equipped(&mut w, old_record, u32::from(loc));
            let new = w.cursor_item(record);
            let before = [w.unit(old).unwrap(), w.unit(new).unwrap()];
            assert_eq!(w.handle(&body(0x1D, new, u32::from(loc))), Ok(0));
            assert_eq!(w.inventory().body_item(loc), Some(before[1]));
            assert_eq!(w.inventory().cursor(), Some(before[0]));
            assert_eq!((w.mode(old), w.mode(new)), (4, 1));
            assert_eq!([w.unit(old).unwrap(), w.unit(new).unwrap()], before);
            assert_eq!(w.inventory().items(), &[before[1]]);
            w.drain();
            assert_eq!(w.handle(&insert(old, 0, 0, 0)), Ok(0));
            assert_eq!(w.inventory().cursor(), None);
            assert_eq!((w.mode(old), w.mode(new)), (0, 1));
            assert!(w.state.errors.is_empty(), "slot {loc}, record {record}");
        }
    }
}

#[test]
fn fulminating_potion_swaps_with_sword_and_displaced_sword_can_drop() {
    let mut w = World::new();
    let sword = equipped(&mut w, SWORD, 4);
    // Reuse the stackable item fixture with the throwing-potion body type.
    w.inv.items[KNIFE].type_ = 38;
    w.inv.items[KNIFE].autobelt = 1;
    w.inv.itemtypes[38].bodyloc1 = 4;
    w.inv.itemtypes[38].bodyloc2 = 5;
    let potion = w.cursor_item(KNIFE);
    assert_eq!(w.handle(&body(0x1D, potion, 4)), Ok(0));
    assert_eq!(w.inventory().cursor(), w.unit(sword));
    assert_eq!(w.inventory().body_item(4), w.unit(potion));
    assert_eq!(item_msgs(&w.drain()), [(0x9D, 9, sword), (0x9D, 9, potion)]);
    w.rest.spot = Some(Spot {
        room: w.room,
        x: 12,
        y: 12,
    });
    assert_eq!(w.handle(&drop_msg(sword)), Ok(0));
    assert_eq!(w.inventory().cursor(), None);
    assert!(matches!(w.mode(sword), 3 | 5));
    assert_eq!(w.mode(potion), 1);
    assert!(w.state.errors.is_empty());
}

#[test]
fn matching_equipped_stacks_merge_and_leave_overflow_on_cursor() {
    use crate::items::moves::stat::QUANTITY;
    for potion in [false, true] {
        for (dest_qty, src_qty, expected_dest, expected_cursor) in
            [(10, 5, 15, None), (45, 12, 50, Some(7))]
        {
            let mut w = World::new();
            if potion {
                w.inv.items[KNIFE].type_ = 38;
            }
            let ty = w.inv.items[KNIFE].type_ as usize;
            w.inv.itemtypes[ty].bodyloc1 = 4;
            w.inv.itemtypes[ty].bodyloc2 = 5;
            let dst = equipped(&mut w, KNIFE, 4);
            let src = w.cursor_item(KNIFE);
            for (g, q) in [(dst, dest_qty), (src, src_qty)] {
                w.set_stat(w.unit(g).unwrap(), QUANTITY, q);
            }
            let owner = w.me();
            assert_eq!(
                w.desk(|d| moves::InventoryOps::equip_check(d, owner, 4, Some(src), false)),
                6,
                "matching stacks must dispatch to StackItems"
            );
            assert_eq!(w.handle(&msg(0x21, &[src, dst])), Ok(0));
            assert_eq!(
                w.stats.unit_total(w.unit(dst).unwrap(), QUANTITY, 0),
                expected_dest
            );
            assert_eq!(w.inventory().body_item(4), w.unit(dst));
            if let Some(q) = expected_cursor {
                assert_eq!(w.inventory().cursor(), w.unit(src));
                assert_eq!(w.stats.unit_total(w.unit(src).unwrap(), QUANTITY, 0), q);
            } else {
                assert_eq!(w.inventory().cursor(), None);
                assert_eq!(w.unit(src), None);
            }
            assert_eq!(
                dest_qty + src_qty,
                expected_dest + expected_cursor.unwrap_or(0)
            );
            assert!(w.state.errors.is_empty());
        }
    }
}

#[test]
fn failed_body_swap_restores_worn_stats_and_weapon_selection() {
    use crate::items::inventory::{grid_id, BODY_GRID};
    let mut w = World::new();
    w.state.link_item_stats = true;
    let sword = equipped(&mut w, SWORD, 4);
    w.set_stat(w.unit(sword).unwrap(), 21, 9);
    let player = w.player;
    assert_eq!(w.stats.unit_total(player, 21, 0), 9);
    let incoming = w.cursor_item(SHIELD);
    let weapon = w.inventory().weapon_guid;
    w.state
        .inventories
        .get_mut(&player)
        .unwrap()
        .grid_or_create(grid_id::BODY, BODY_GRID.0, BODY_GRID.1)
        .unwrap()
        .width += 1;
    assert_eq!(w.handle(&body(0x1D, incoming, 4)), Ok(3));
    assert_eq!(w.inventory().body_item(4), w.unit(sword));
    assert_eq!(w.inventory().cursor(), w.unit(incoming));
    assert_eq!(w.inventory().weapon_guid, weapon);
    assert_eq!(w.stats.unit_total(player, 21, 0), 9);
    assert!(w.state.errors.is_empty());
}

#[test]
fn shrinking_a_belt_during_a_swap_keeps_the_displaced_cursor() {
    let mut w = World::new();
    for record in [CAP, SWORD] {
        w.inv.items[record].type_ = 19;
    }
    w.inv.items[CAP].belt = 0;
    w.inv.items[SWORD].belt = 1;
    w.inv.belts[0] = 16;
    w.inv.belts[1] = 4;
    w.inv.itemtypes[19].bodyloc1 = 8;
    w.inv.itemtypes[19].bodyloc2 = 8;
    let old = equipped(&mut w, CAP, 8);
    let potion = w.cursor_item(HP1);
    assert_eq!(w.handle(&msg(0x23, &[potion, 8])), Ok(0));
    w.drain();
    let incoming = w.cursor_item(SWORD);
    let owner = w.me();
    w.desk(|d| moves::InventoryOps::update_list_add(d, owner, old));
    assert_eq!(w.handle(&body(0x1D, incoming, 8)), Ok(0));
    assert_eq!(w.inventory().cursor(), w.unit(old));
    assert_eq!(w.inventory().body_item(8), w.unit(incoming));
    assert_eq!(w.mode(potion), 0);
    assert!(w.inventory().contains(w.unit(potion).unwrap()));
    let messages = item_msgs(&w.drain());
    let stored = messages
        .iter()
        .position(|&m| m == (0x9C, 4, potion))
        .unwrap();
    let held = messages.iter().position(|&m| m == (0x9D, 9, old)).unwrap();
    assert!(
        stored < held,
        "container updates must precede the new cursor"
    );
    assert_eq!(w.handle(&insert(old, 0, 2, 0)), Ok(0));
    assert_eq!(w.inventory().cursor(), None);
    assert!(w.state.errors.is_empty(), "{:?}", w.state.errors);
}

#[test]
fn exhausting_an_equipped_throw_stack_clears_the_slot_without_taking_the_cursor() {
    use super::T_TKNI;
    use crate::items::moves::stat::QUANTITY;
    let mut w = World::new();
    // The throwing-knife family must be wearable in the right hand.
    w.inv.itemtypes[usize::from(T_TKNI)].body = 1;
    w.inv.itemtypes[usize::from(T_TKNI)].bodyloc1 = 4;
    w.inv.itemtypes[usize::from(T_TKNI)].bodyloc2 = 5;
    let thrown = equipped(&mut w, KNIFE, 4);
    assert_eq!(w.inventory().body_item(4), w.unit(thrown));
    let held = w.cursor_item(CAP);
    w.set_stat(w.unit(thrown).unwrap(), QUANTITY, 0);
    let owner = w.me();
    assert_eq!(
        w.desk(|d| moves::handlers::exhaust_stack(d, owner, thrown)),
        Ok(())
    );
    assert_eq!(w.inventory().body_item(4), None);
    assert_eq!(w.inventory().cursor(), w.unit(held));
    let messages = item_msgs(&w.drain());
    assert!(messages.contains(&(0x9D, 8, thrown)));
    assert_eq!(w.unit(thrown), None);
    assert_eq!(w.inventory().cursor(), w.unit(held));
    assert!(w.state.errors.is_empty());
}

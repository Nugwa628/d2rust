//! Tests for an equipped item's stats reaching the wearer (`item_link`):
//! attach on 0x1A, detach on 0x1C, set bonuses and the
//! static / dynamic flag of the item's stat list.

use super::*;

const STR: u16 = 0;

#[test]
fn equipped_item_stats_reach_the_wearer_and_leave_with_it() {
    let mut w = World::new();
    w.state.link_item_stats = true;
    let c = w.cursor_item(CAP);
    let cu = w.unit(c).unwrap();
    w.set_stat(cu, STR, 5);
    let p = w.player;
    assert_eq!(w.stats.unit_total(p, STR, 0), 10);
    assert_eq!(w.handle(&body(0x1A, c, 1)), Ok(0));
    assert_eq!(w.mode(c), 1);
    assert_eq!(w.stats.unit_total(p, STR, 0), 15, "worn: +5 strength");
    assert_eq!(w.handle(&unequip(1)), Ok(0));
    assert_eq!(w.stats.unit_total(p, STR, 0), 10, "off again");
}

#[test]
fn switch_off_leaves_the_player_alone() {
    let mut w = World::new();
    let c = w.cursor_item(CAP);
    let cu = w.unit(c).unwrap();
    w.set_stat(cu, STR, 5);
    assert_eq!(w.handle(&body(0x1A, c, 1)), Ok(0));
    assert_eq!(w.stats.unit_total(w.player, STR, 0), 10);
}

use crate::items::q;
use crate::items::tables::{PropRec, PropSlot, PropertyRec, SetItemRec, SetRec};

const DEX: u16 = 2;
const SET_ID: u16 = 71;

/// The errors other than the weapon bookkeeping's `NoMouseSkill` (the
/// test rest answers no mouse skills when the equipment rules run).
fn real_errors(w: &World) -> Vec<String> {
    w.state
        .errors
        .iter()
        .map(|e| format!("{e:?}"))
        .filter(|e| !e.contains("NoMouseSkill"))
        .collect()
}

/// A set of three with a cap (slot 0) and a sword (slot 1) in the
/// tables; the two-piece bonus is +5 dexterity (two pieces
/// worn give the first two partial records).
fn set_world() -> World {
    let mut w = World::new();
    w.state.link_item_stats = true;
    let mut p = PropertyRec::default();
    p.slots[0] = PropSlot {
        func: 1,
        stat: DEX,
        set: 0,
        val: 0,
    };
    w.tables.properties = vec![p];
    let mut partial = [PropRec::NONE; 8];
    partial[0] = PropRec {
        code: 0,
        param: 0,
        min: 5,
        max: 5,
    };
    w.tables.sets = vec![SetRec {
        count: 3,
        version: 0,
        partial,
        full: [PropRec::NONE; 8],
    }];
    w.tables.setitems = (0..2)
        .map(|slot| SetItemRec {
            set: 0,
            slot,
            add_func: 2,
            props: [PropRec::NONE; 9],
            aprops: [PropRec::NONE; 10],
            ..Default::default()
        })
        .collect();
    w
}

fn set_piece(w: &mut World, record: usize, index: i32) -> Guid {
    let g = w.cursor_item(record);
    let u = w.unit(g).unwrap();
    let it = w.items.get_mut(u).unwrap();
    it.quality = q::SET;
    it.file_index = index;
    g
}

#[test]
fn set_bonus_applies_with_two_pieces_and_leaves_with_one() {
    let mut w = set_world();
    // The inventory pass re-updates the pieces still worn after a body
    // removal.
    w.state.equip_rules = true;
    let p = w.player;
    let cap = set_piece(&mut w, CAP, 0);
    assert_eq!(w.handle(&body(0x1A, cap, 1)), Ok(0));
    assert_eq!(w.stats.unit_total(p, DEX, 0), 10, "one piece: no bonus");
    let sword = set_piece(&mut w, SWORD, 1);
    assert_eq!(w.handle(&body(0x1A, sword, 4)), Ok(0));
    assert!(real_errors(&w).is_empty(), "{:?}", w.state.errors);
    assert_eq!(w.stats.unit_total(p, DEX, 0), 15, "two pieces: +5");
    assert_eq!(w.handle(&unequip(4)), Ok(0));
    assert_eq!(w.stats.unit_total(p, DEX, 0), 10, "bonus gone");
    assert!(real_errors(&w).is_empty(), "{:?}", w.state.errors);
}

#[test]
fn a_dual_wielders_weapon_in_use_is_static_and_the_second_weapon_dynamic() {
    use crate::stats::lists::flag;
    let mut w = World::new();
    w.state.link_item_stats = true;
    // The only weapon becomes the weapon in use before the stat refresh:
    // reset 1 (its damage-related stats reach the owner).
    let sword = w.cursor_item(SWORD);
    assert_eq!(w.handle(&body(0x1A, sword, 4)), Ok(0));
    let l = w.stats.unit_list(w.unit(sword).unwrap()).unwrap();
    assert_eq!(
        w.stats.flags(l) & flag::DYNAMIC,
        0,
        "weapon in use: reset 1"
    );
    // A second weapon in the other hand is not the weapon in use: reset 0.
    let second = w.cursor_item(SWORD);
    assert_eq!(w.handle(&body(0x1A, second, 5)), Ok(0));
    let l2 = w.stats.unit_list(w.unit(second).unwrap()).unwrap();
    assert_ne!(
        w.stats.flags(l2) & flag::DYNAMIC,
        0,
        "second weapon: reset 0"
    );
    assert_eq!(w.stats.flags(l) & flag::DYNAMIC, 0, "first stays static");
    let cap = w.cursor_item(CAP);
    assert_eq!(w.handle(&body(0x1A, cap, 1)), Ok(0));
    let l = w.stats.unit_list(w.unit(cap).unwrap()).unwrap();
    assert_eq!(w.stats.flags(l) & flag::DYNAMIC, 0, "armour: reset 1");
}

#[test]
fn the_last_set_piece_off_leaves_the_set_list_tag_on_the_owner() {
    let mut w = set_world();
    w.state.equip_rules = true;
    // Set 1, so the tag (stat 71 = set id) is not 0.
    let s0 = w.tables.sets[0].clone();
    w.tables.sets.push(s0);
    for r in &mut w.tables.setitems {
        r.set = 1;
    }
    let p = w.player;
    let cap = set_piece(&mut w, CAP, 0);
    assert_eq!(w.handle(&body(0x1A, cap, 1)), Ok(0));
    let sword = set_piece(&mut w, SWORD, 1);
    assert_eq!(w.handle(&body(0x1A, sword, 4)), Ok(0));
    assert_eq!(w.stats.unit_total(p, DEX, 0), 15, "two pieces: +5");
    assert_eq!(w.stats.unit_total(p, SET_ID, 0), 1);
    assert_eq!(w.handle(&unequip(4)), Ok(0));
    assert_eq!(w.stats.unit_total(p, DEX, 0), 10, "bonus gone");
    assert_eq!(w.stats.unit_total(p, SET_ID, 0), 1, "the cap's list");
    // The body removal only detaches the cap's list; the deactivation
    // then skips its set update, so the owner list stays (the game keeps
    // the set tag, stat 71, here).
    assert_eq!(w.handle(&unequip(1)), Ok(0));
    assert_eq!(w.stats.unit_total(p, SET_ID, 0), 1, "tag stays");
    assert!(real_errors(&w).is_empty(), "{:?}", w.state.errors);
}

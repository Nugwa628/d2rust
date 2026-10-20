//! Tests of the inventory edge cases settled after the first pass, on the fake
//! world of `tests.rs` (a child module: the fake and the synthetic tables
//! are shared).

use super::*;
use crate::items::inventory::checks::{active_inventory_item, usable, TYPE_CHARM};
use crate::items::inventory::levelreq::{
    affix_value, level_requirement, AffixReq, LevelReqItem, LevelReqUnit, CRAFTED_CAP,
};

// ---------------------------------------------------------------

#[test]
fn put_cursor_links_nothing_and_none_unlinks_the_cursor_item() {
    let mut w = Fake::new();
    let mut inv = player_inv();
    let a = w.add(10, R_RING, mode::CURSOR);
    inv.weapon_guid = w.d(a).guid;
    inv.put_cursor(&mut w, Some(a));
    // +0x20 := item, +0x5C := inventory; no list link, no count change.
    assert_eq!(inv.cursor(), Some(a));
    assert_eq!(w.d(a).inv, Some(PLAYER));
    assert!(inv.items().is_empty());
    assert_eq!(inv.count, 0);
    // None: the cursor item is unlinked (only the cursor field of the
    // inventory changes; the item's node fields, owning inventory and a
    // matching weapon GUID are cleared).
    inv.put_cursor(&mut w, None);
    assert_eq!(inv.cursor(), None);
    assert_eq!(inv.count, 0);
    let d = w.d(a);
    assert_eq!((d.inv, d.node_grid, d.node_kind), (None, 0, node::NONE));
    assert_eq!(inv.weapon_guid, NO_GUID);
    // A placement of the cursor item unlinks it from the cursor.
    let t = tables();
    let b = w.add(11, R_RING, mode::CURSOR);
    inv.put_cursor(&mut w, Some(b));
    assert!(place_at_page(&mut inv, &mut w, &t, b, 0, 0, 0));
    assert_eq!(inv.cursor(), None);
    assert_eq!((inv.items(), inv.count), (&[b][..], 1));
}

#[test]
fn grid_record_other_pages_and_owner_types() {
    let p = UnitKind::Player { class: 5 };
    // Pages 0 and 5–255 take the class record (`page − 1 ≤ 3` unsigned).
    for pg in [0, 5, 0x7F, 0xFF] {
        assert_eq!(grid_record(p, pg, true), Some(14), "page {pg}");
    }
    // Missiles, items, tiles: no record.
    assert_eq!(grid_record(UnitKind::Item, 0, true), None);
    assert_eq!(grid_record(UnitKind::Other, 3, true), None);
}

// ---------------------------------------------------------------

#[test]
fn signed_wrap_places_without_cells() {
    let t = tables();
    let mut w = Fake::new();
    let mut inv = player_inv();
    let g = Grid::new(10, 4);
    // x + w wraps past 2^31: the bound test passes, the fit loop runs zero
    // times.
    assert!(in_bounds(&g, i32::MAX, 0, 1, 1));
    assert!(fits(&g, i32::MAX, 0, 1, 1));
    assert!(!in_bounds(&g, 9, 0, 2, 1));
    let a = w.add(10, R_RING, mode::CURSOR);
    assert!(place_at_page(&mut inv, &mut w, &t, a, 0, i32::MAX, 0));
    let d = w.d(a);
    assert_eq!((d.x, d.y, d.node_kind), (i32::MAX, 0, node::PAGE));
    assert_eq!(inv.count, 1);
    assert!(inv.items().contains(&a));
    let grid = inv.grid(grid_id::PAGE).unwrap();
    assert!(grid.cells.iter().all(Option::is_none), "no cell occupied");
    // y + h wraps the same way.
    let b = w.add(11, R_SWORD, mode::CURSOR);
    assert!(place_at_page(&mut inv, &mut w, &t, b, 0, 0, i32::MAX - 1));
}

#[test]
fn failed_link_check_leaves_the_item_placed_in_mode_4() {
    let t = tables();
    let mut w = Fake::new();
    w.link_ok = false;
    let mut inv = player_inv();
    let a = w.add(10, R_RING, mode::CURSOR);
    w.items.get_mut(&a).unwrap().page = 0;
    inv.put_cursor(&mut w, Some(a));
    assert!(!place_in_page(
        &mut inv,
        &mut w,
        &t,
        Some(a),
        2,
        1,
        false,
        true
    ));
    // Nothing undone: placed (cells, list, count), cursor cleared by the
    // placement's unlink, still in mode 4.
    assert_eq!(inv.item_at(grid_id::PAGE, 2, 1), Some(a));
    assert_eq!((inv.items(), inv.count), (&[a][..], 1));
    assert_eq!(inv.cursor(), None);
    assert_eq!(w.d(a).mode, mode::CURSOR);
}

// ---------------------------------------------------------------

fn potion(w: &mut Fake, id: u32, r: usize) -> UnitId {
    w.add(id, r, mode::CURSOR)
}

#[test]
fn full_similar_column_falls_through_to_the_next() {
    let t = tables();
    let mut w = Fake::new();
    let mut inv = player_inv();
    // No belt: 4 boxes; column 0 holds hp1 (full: only slot 0 < 4),
    // column 1 holds hp2 (full too), column 2 empty.
    let a = potion(&mut w, 10, R_HP1);
    let b = potion(&mut w, 11, R_HP2);
    assert!(place_in_belt_slot(&mut inv, &mut w, &t, a, 0));
    assert!(place_in_belt_slot(&mut inv, &mut w, &t, b, 1));
    let rv = potion(&mut w, 12, R_RVS);
    assert!(place_in_belt_slot(&mut inv, &mut w, &t, rv, 3));
    // An hp4: columns 0 and 1 are similar but full → none; autobelt 0.
    let n = potion(&mut w, 13, R_HP4);
    assert_eq!(free_belt_slot(&inv, &w, &t, n), None);
    // A rejuvenation: column 3 similar and full; autobelt → slot 2.
    let r2 = potion(&mut w, 14, R_RVS);
    assert_eq!(free_belt_slot(&inv, &w, &t, r2), Some(2));
}

#[test]
fn belt_slot_bound_is_fifteen_without_numboxes() {
    let t = tables();
    let mut w = Fake::new();
    let mut inv = player_inv();
    // No belt (4 boxes): slot 15 still places; 16 does not.
    let a = potion(&mut w, 10, R_HP1);
    assert!(place_in_belt_slot(&mut inv, &mut w, &t, a, 15));
    let b = potion(&mut w, 11, R_HP1);
    assert!(!place_in_belt_slot(&mut inv, &mut w, &t, b, 16));
}

#[test]
fn compaction_flags_only_items_that_move() {
    let t = tables();
    let mut w = Fake::new();
    let mut inv = player_inv();
    // Column 1: rows 0 and 2 hold items; row 1 is empty.
    let a = potion(&mut w, 10, R_HP1);
    let b = potion(&mut w, 11, R_HP1);
    assert!(place_in_belt_slot(&mut inv, &mut w, &t, a, 1));
    assert!(place_in_belt_slot(&mut inv, &mut w, &t, b, 9));
    let moves = compact_belt(&mut inv, &mut w, &t, 5);
    assert_eq!(moves, vec![(9, 5)]);
    // The moved item gets item flags 0x400 | 0x1 (item flags, not command
    // flags); the one that stays gets nothing.
    assert_eq!(w.d(b).flags & (iflag::F400 | iflag::CHANGED), 0x401);
    assert_eq!(w.d(b).cmd_flags, 0);
    assert_eq!(w.d(a).flags & (iflag::F400 | iflag::CHANGED), 0);
    assert_eq!(inv.update_list(), &[w.d(b).guid]);
}

#[test]
fn belt_boxes_of_a_belt_or_none() {
    let t = tables();
    let mut w = Fake::new();
    let sash = w.add(10, R_SASH, mode::EQUIPPED);
    let girdle = w.add(11, R_GIRDLE, mode::EQUIPPED);
    assert_eq!(belt_boxes_of(&w, &t, None), 4);
    assert_eq!(belt_boxes_of(&w, &t, Some(sash)), 8);
    assert_eq!(belt_boxes_of(&w, &t, Some(girdle)), 16);
}

#[test]
fn belt_removal_gate_refuses_only_a_trade_with_belt_items() {
    let t = tables();
    let mut w = Fake::new();
    let mut inv = player_inv();
    let trade = InteractionTarget::Unit {
        ty: 0,
        unit: UnitId(5),
    };
    w.interaction = Some(trade);
    // Trade, empty belt → allowed.
    assert!(belt_removal_allowed(&inv, &w));
    let a = potion(&mut w, 10, R_HP1);
    assert!(place_in_belt_slot(&mut inv, &mut w, &t, a, 0));
    assert!(!belt_removal_allowed(&inv, &w));
    // An NPC interaction (type 1) does not refuse.
    w.interaction = Some(InteractionTarget::Unit {
        ty: 1,
        unit: UnitId(5),
    });
    assert!(belt_removal_allowed(&inv, &w));
    w.interaction = None;
    assert!(belt_removal_allowed(&inv, &w));
}

// ---------------------------------------------------------------

#[test]
fn requirement_percent_is_signed_and_truncating() {
    let t = tables();
    let mut w = Fake::new();
    let s = w.add(10, R_SWORD, mode::CURSOR);
    // reqstr 25, p = −50: pct(25, −50, 100) = −12 (toward zero), so the
    // strength requirement is 13, not 12.
    w.p(s).req_percent = -50;
    w.unit_stats.insert((PLAYER, stat::STRENGTH), 13);
    w.unit_stats.insert((PLAYER, stat::DEXTERITY), 10);
    assert!(requirements_met(&w, &t, Some(s), PLAYER, false));
    w.unit_stats.insert((PLAYER, stat::STRENGTH), 12);
    assert!(!requirements_met(&w, &t, Some(s), PLAYER, false));
    // p = 33: 25 × 33 / 100 = 8.25 → 8; strength 33 passes.
    w.p(s).req_percent = 33;
    w.unit_stats.insert((PLAYER, stat::STRENGTH), 33);
    w.unit_stats.insert((PLAYER, stat::DEXTERITY), 13);
    assert!(requirements_met(&w, &t, Some(s), PLAYER, false));
}

#[test]
fn hands_of_an_object_owner_never_dual() {
    let t = tables();
    let mut w = Fake::new();
    let obj = UnitId(50);
    w.kinds.insert(obj, UnitKind::Object { class: 0x152 });
    let a = w.add(10, R_SWORD, mode::CURSOR);
    let b = w.add(11, R_SWORD, mode::CURSOR);
    assert!(!hands_compatible(&w, &t, obj, Some(a), Some(b)));
}

#[test]
fn stack_test_ethereal_and_quality() {
    let t = tables();
    let mut w = Fake::new();
    let a = w.add(10, R_ARROWS, mode::CURSOR);
    let b = w.add(11, R_ARROWS, mode::CURSOR);
    assert!(stack_test(&w, &t, a, b));
    // Ethereal bits must be equal.
    w.items.get_mut(&a).unwrap().flags |= iflag::ETHEREAL;
    assert!(!stack_test(&w, &t, a, b));
    w.items.get_mut(&b).unwrap().flags |= iflag::ETHEREAL;
    assert!(stack_test(&w, &t, a, b));
    // Qualities 1–3 only: equal magic (4) or none (0) fail.
    for q in [0, 4, 7, 9] {
        w.p(a).quality = q;
        w.p(b).quality = q;
        assert!(!stack_test(&w, &t, a, b), "quality {q}");
    }
    for q in 1..=3 {
        assert!(stack_quality_ok(q));
    }
}

#[test]
fn auto_equip_primary_tpot_and_quiver_hands() {
    let mut t = tables();
    // A helm whose type2 is `tpot`: only the primary type is tested.
    let mut r = t.items[R_HELM];
    r.type2 = T_TPOT;
    t.items.push(r);
    let helm2 = t.items.len() - 1;
    let mut h = hands();
    h.t = t;
    let x = h.w.add(10, helm2, mode::GROUND);
    assert_eq!(auto_equip_location(&h.inv, &h.w, &h.t, x, true), Some(1));
    // Quiver: needs the hand weapon's `shoots`; right hand first, then left.
    let q = h.w.add(11, R_ARROWS, mode::GROUND);
    assert_eq!(auto_equip_location(&h.inv, &h.w, &h.t, q, true), None);
    let bow = h.equip(12, R_BOW, 4);
    h.w.p(bow).ammo = Some(T_BOWQ);
    h.w.p(bow).two_handed = true;
    assert_eq!(auto_equip_location(&h.inv, &h.w, &h.t, q, true), Some(5));
    let mut h = hands();
    let bow = h.equip(12, R_BOW, 5);
    h.w.p(bow).ammo = Some(T_BOWQ);
    let q = h.w.add(11, R_ARROWS, mode::GROUND);
    // The bow in the left hand feeds it; the quiver goes to the free 4.
    assert_eq!(auto_equip_location(&h.inv, &h.w, &h.t, q, true), Some(4));
}

#[test]
fn auto_equip_compatibility_profile() {
    let mut t = tables();
    // An `xbow` (35) and an `xboq` (6), not in the base tables.
    t.items.push(item_rec(b"lxb ", 35, 2, 3));
    let xbow = t.items.len() - 1;
    t.items.push(item_rec(b"cqv ", 6, 1, 3));
    let xboq = t.items.len() - 1;
    let mut w = Fake::new();
    let bow = w.add(10, R_BOW, mode::CURSOR);
    let bq = w.add(11, R_ARROWS, mode::CURSOR);
    let xb = w.add(12, xbow, mode::CURSOR);
    let xq = w.add(13, xboq, mode::CURSOR);
    let ring = w.add(14, R_RING, mode::CURSOR);
    let ring2 = w.add(15, R_RING, mode::CURSOR);
    let sword = w.add(16, R_SWORD, mode::CURSOR);
    let shield = w.add(17, R_SHIELD, mode::CURSOR);
    let claw = w.add(18, R_CLAW, mode::CURSOR);
    let claw2 = w.add(19, R_CLAW, mode::CURSOR);
    let c = |w: &Fake, n, e| auto_equip_compatible(w, &t, PLAYER, n, e);
    assert!(c(&w, bow, bq) && c(&w, bq, bow));
    assert!(c(&w, xb, xq) && c(&w, xq, xb));
    assert!(!c(&w, bow, xq) && !c(&w, xb, bq));
    assert!(c(&w, ring, ring2));
    assert!(c(&w, sword, shield) && c(&w, shield, sword));
    // A two-handed weapon is no one-hand weapon.
    w.p(sword).two_handed = true;
    assert!(!c(&w, sword, shield));
    // Both one-hand weapons and both dual: an assassin needs `h2h` on
    // both; a barbarian dual-wields any.
    assert!(!c(&w, claw, claw2));
    w.kinds.insert(PLAYER, UnitKind::Player { class: 6 });
    assert!(c(&w, claw, claw2));
    w.p(sword).two_handed = false;
    assert!(!c(&w, claw, sword));
    w.kinds.insert(PLAYER, UnitKind::Player { class: 4 });
    assert!(c(&w, claw, sword));
    let p = equip_profile(&w, &t, PLAYER, bq);
    assert!(p.bowq && !p.weapon && !p.ring);
}

#[test]
fn level_requirement_by_quality() {
    let a = |levelreq, class, classlevelreq| {
        Some(AffixReq {
            levelreq,
            class,
            classlevelreq,
        })
    };
    let sorc = LevelReqUnit {
        class: 1,
        player: true,
        expansion: true,
    };
    // Affix value: the class value only for the matching class.
    let ar = AffixReq {
        levelreq: 20,
        class: 1,
        classlevelreq: 15,
    };
    assert_eq!(affix_value(&ar, Some(&sorc)), 15);
    assert_eq!(affix_value(&ar, None), 20);
    // Magic: prefix 0, suffix 0, automagic only.
    let mut m = LevelReqItem {
        quality: 4,
        prefixes: [a(5, 0xFF, 0), a(40, 0xFF, 0), None],
        suffixes: [a(9, 0xFF, 0), None, None],
        automagic: a(7, 0xFF, 0),
        ..Default::default()
    };
    assert_eq!(level_requirement(&m, None), 9);
    // Rare: all six and the automagic.
    m.quality = 6;
    assert_eq!(level_requirement(&m, None), 40);
    // Crafted: max of the six + 10 + 3 × present, capped at 98.
    m.quality = 8;
    assert_eq!(level_requirement(&m, None), 40 + 10 + 3 * 3);
    m.prefixes[1] = a(90, 0xFF, 0);
    assert_eq!(level_requirement(&m, None), CRAFTED_CAP);
    // Set: negative → 0; unique: classic unit on a version-0 item → 0.
    let s = LevelReqItem {
        quality: 5,
        set_lvlreq: -3,
        ..Default::default()
    };
    assert_eq!(level_requirement(&s, None), 0);
    let mut u = LevelReqItem {
        quality: 7,
        unique_lvlreq: Some(29),
        ..Default::default()
    };
    assert_eq!(level_requirement(&u, Some(&sorc)), 29);
    let classic = LevelReqUnit {
        expansion: false,
        ..sorc
    };
    assert_eq!(level_requirement(&u, Some(&classic)), 0);
    u.version = 100;
    assert_eq!(level_requirement(&u, Some(&classic)), 29);
    // Then the class levelreq, fillers, skills, stat 92.
    let mut n = LevelReqItem {
        quality: 2,
        class_levelreq: 12,
        fillers: vec![LevelReqItem {
            quality: 2,
            class_levelreq: 18,
            ..Default::default()
        }],
        single_skills: vec![6],
        nonclass_skills: vec![(1, 1)],
        ..Default::default()
    };
    assert_eq!(level_requirement(&n, None), 18);
    // Non-class skill: reqlevel + 6, or alone for the player of its class.
    n.nonclass_skills = vec![(30, 1)];
    assert_eq!(level_requirement(&n, None), 36);
    assert_eq!(level_requirement(&n, Some(&sorc)), 30);
    let monster = LevelReqUnit {
        player: false,
        ..sorc
    };
    assert_eq!(level_requirement(&n, Some(&monster)), 36);
    // Stat 92 is added last; a sum below 1 returns 0.
    n.stat_levelreq = 4;
    assert_eq!(level_requirement(&n, None), 40);
    let z = LevelReqItem {
        stat_levelreq: -5,
        ..Default::default()
    };
    assert_eq!(level_requirement(&z, None), 0);
}

// ---------------------------------------------------------------

#[test]
fn active_inventory_item_and_usable() {
    let mut t = tables();
    t.items.push(item_rec(b"cm1 ", TYPE_CHARM, 1, 1));
    let charm = t.items.len() - 1;
    let mut w = Fake::new();
    let c = w.add(10, charm, mode::STORED);
    w.items.get_mut(&c).unwrap().page = 0;
    assert!(active_inventory_item(&w, &t, c, PLAYER));
    for f in [iflag::BROKEN, iflag::F4000] {
        w.items.get_mut(&c).unwrap().flags |= f;
        assert!(!active_inventory_item(&w, &t, c, PLAYER));
        w.items.get_mut(&c).unwrap().flags &= !f;
    }
    w.items.get_mut(&c).unwrap().page = page::STASH;
    assert!(!active_inventory_item(&w, &t, c, PLAYER));
    w.items.get_mut(&c).unwrap().page = 0;
    w.p(c).level_req = 99;
    assert!(!active_inventory_item(&w, &t, c, PLAYER));
    let ring = w.add(11, R_RING, mode::STORED);
    w.items.get_mut(&ring).unwrap().page = 0;
    assert!(!active_inventory_item(&w, &t, ring, PLAYER), "not a charm");
    // Usable: a quiver-type item needs the other hand to hold its type.
    let mut h = hands();
    let q = h.equip(20, R_ARROWS, 5);
    assert!(!usable(&h.inv, &h.w, &h.t, q));
    h.equip(21, R_BOW, 4);
    // Itemtypes `quiver` of `bowq` is 1 in the synthetic tables: the bow
    // is not of type 1.
    assert!(!usable(&h.inv, &h.w, &h.t, q));
    h.t.itemtypes[T_BOWQ as usize].quiver = T_BOW as u16;
    assert!(usable(&h.inv, &h.w, &h.t, q));
    let s = h.w.add(22, R_SWORD, mode::EQUIPPED);
    assert!(usable(&h.inv, &h.w, &h.t, s));
}

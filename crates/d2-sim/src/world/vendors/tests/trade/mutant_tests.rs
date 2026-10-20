// Spec: specs/world/vendors.md §4, §6, §7, §8, §9.1
//! Mutation-testing kills (METHODS M08): each test pins an outcome the
//! spec decides that no earlier test checked. A child of
//! `vendors::tests::trade` for its trading setup.

use super::*;
use crate::rng::Seed;
use crate::world::vendors::store::{open, refresh_act, town_act, StoreCtx};
use crate::world::vendors::trade::price_ctx;

// From specs/world/vendors.md §9.2 rule 9: only quest flags f ≠ 0 read a
// quest slot.
#[test]
fn price_ctx_reads_only_nonzero_quest_flags() {
    let (t, _, mut w) = setup(class::GHEED, 0);
    w.quest.insert(4, 1);
    w.quest.insert(0, 7);
    let c = price_ctx(&t, &w, p(), class::GHEED);
    assert_eq!(c.quest_slots, [1, 0, 0]);
}

// From specs/world/vendors.md §9.1: s := 0 only when the stash cap is
// *below* it; receive drops only above the cap.
#[test]
fn gold_cap_boundaries() {
    let mut w = Fake::new(class::CHARSI);
    w.set(PLAYER, stat::GOLD, 10);
    w.set(PLAYER, stat::GOLD_BANK, 100);
    w.stash_cap = 90;
    assert!(pay(&mut w, p(), 20));
    assert_eq!((w.gold(), w.get(PLAYER, stat::GOLD_BANK)), (0, 90));
    w.gold_cap = 1000;
    w.set(PLAYER, stat::GOLD, 900);
    receive(&mut w, p(), 100);
    assert_eq!((w.gold(), w.dropped_gold), (1000, 0));
}

// From specs/world/vendors.md §8.2: quantity := max stack only for a
// throwable **and** stackable item; durability only when max > 0.
#[test]
fn repair_item_quantity_and_durability_conditions() {
    let (mut t, _, mut w) = setup(class::CHARSI, 0);
    let lax = index(&t, "lax");
    t.items[lax].stackable = 1;
    t.items[lax].maxstack = 10;
    let tkn = index(&t, "tkn");
    t.items[tkn].stackable = 0;
    for c in ["lax", "tkn"] {
        let g = add(&mut w, &t, c, 0x20);
        w.set(g, stat::DURABILITY, 5);
        repair_item(&t, &mut w, UnitId(g), Some(p()));
        assert_eq!(w.get(g, stat::QUANTITY), 0, "{c}");
        assert!(!w.stat_msgs.contains(&(g, stat::QUANTITY)), "{c}");
    }
    // Max durability 0 (repairable through its charges): no stat 72.
    let skc = add(&mut w, &t, "skc", 0x30);
    w.set(skc, stat::MAXDURABILITY, 0);
    w.set(skc, stat::DURABILITY, 0);
    w.units.get_mut(&skc).unwrap().extra.charges = vec![(0, (5 << 8) | 3)];
    w.stat_msgs.clear();
    repair_item(&t, &mut w, UnitId(skc), Some(p()));
    assert!(w.stat_msgs.is_empty());
}

// From specs/world/vendors.md §7.1 r12 and edge case 3: only t = 2 takes
// the item out of the gamble list.
#[test]
fn other_transactions_keep_the_gamble_list() {
    let (t, mut rec, mut w) = setup(class::GHEED, 0);
    let hax = add(&mut w, &t, "hax", 0x40);
    rec.gamble_lists.push(GambleList {
        player: PLAYER,
        items: vec![UnitId(hax)],
    });
    assert!(rec.has_gamble);
    w.set(PLAYER, stat::GOLD, 10_000);
    assert_eq!(
        buy(&t, &mut rec, &mut w, p(), &buy_msg(hax, 1, false)),
        Ok(0)
    );
    assert_eq!(rec.gamble_lists[0].items, vec![UnitId(hax)]);
}

/// Charsi with a permanent `aqv` stack of 500 whose unit price is 1.
fn arrows() -> (VendorTables, VendorRecord, Fake, u32) {
    let (mut t, mut rec, mut w) = setup(class::CHARSI, 0);
    let aqv = index(&t, "aqv");
    t.items[aqv].cost = 2048;
    let a = add(&mut w, &t, "aqv", 0x30);
    w.set(a, stat::QUANTITY, 500);
    w.set(a, stat::MAXDURABILITY, 0);
    rec.store.push(UnitId(a));
    (t, rec, w, a)
}

// From specs/world/vendors.md §7.1 r8, r9: the stack rule needs fill;
// k = 1 fills one; a new copy gets n = min(f, a).
#[test]
fn fill_stack_rules() {
    // Without fill: one copy of the whole stack at cost(0) = 937.
    let (t, mut rec, mut w, a) = arrows();
    w.set(PLAYER, stat::GOLD, 1000);
    assert_eq!(buy(&t, &mut rec, &mut w, p(), &buy_msg(a, 0, false)), Ok(0));
    assert_eq!(w.gold(), 1000 - 937);
    // A partial stack with room for 1: k = 1 is bought.
    let (t, mut rec, mut w, a) = arrows();
    let stack = add(&mut w, &t, "aqv", 0x31);
    w.set(stack, stat::QUANTITY, 499);
    w.stack = Some((stack, 1));
    w.set(PLAYER, stat::GOLD, 1000);
    assert_eq!(buy(&t, &mut rec, &mut w, p(), &buy_msg(a, 0, true)), Ok(0));
    assert_eq!(w.get(stack, stat::QUANTITY), 500);
    assert_eq!(w.gold(), 999);
    // No stack, a store stack of 200 (price 375): a = 375 → the copy holds
    // min(500, 375), not the store stack's 200.
    let (t, mut rec, mut w, a) = arrows();
    w.set(a, stat::QUANTITY, 200);
    w.set(PLAYER, stat::GOLD, 375);
    assert_eq!(buy(&t, &mut rec, &mut w, p(), &buy_msg(a, 0, true)), Ok(0));
    let copy = *w.backpack.last().unwrap();
    assert_eq!(w.get(copy, stat::QUANTITY), 375);
    assert_eq!(w.gold(), 0);
}

// From specs/world/vendors.md §7.1 r9.8: copy flag 2 := 1.
#[test]
fn bought_copy_flag_2_is_set() {
    let (t, mut rec, mut w) = setup(class::CHARSI, 0);
    let dgr = add(&mut w, &t, "dgr", 0x12);
    w.units.get_mut(&dgr).unwrap().flags |= flag::TARGET;
    rec.store.push(UnitId(dgr));
    w.set(PLAYER, stat::GOLD, 500);
    assert_eq!(
        buy(&t, &mut rec, &mut w, p(), &buy_msg(dgr, 0, false)),
        Ok(0)
    );
    let copy = w.backpack[0];
    assert!(w.unit(copy).flags & flag::TARGET != 0);
}

fn sell_msg(item: u32, mode: u16) -> SellMsg {
    SellMsg {
        npc: NPC6,
        item,
        mode,
        client_price: 0,
    }
}

// From specs/world/vendors.md §7.2 r7: only a unique whose row has the
// no-sell bit is not re-sold.
#[test]
fn unique_nosell_bit() {
    let mut t = tables();
    t.unique_nosell_mask = 0x04;
    t.uniques[0].flags = 0x04;
    t.uniques.push(UniqueCost {
        cost: t.uniques[0].cost,
        flags: 0x02,
    });
    for (quality, index_, resold) in [
        (q::UNIQUE, 0, false),
        (q::UNIQUE, 1, true),
        (q::RARE, 0, true),
    ] {
        let (_, mut rec, mut w) = setup(class::CHARSI, 0);
        let skc = add(&mut w, &t, "skc", 0x20);
        let u = w.units.get_mut(&skc).unwrap();
        u.quality = quality;
        u.file_index = index_;
        sell(&t, &mut rec, &mut w, p(), &sell_msg(skc, 0)).unwrap();
        assert_eq!(!rec.store.is_empty(), resold, "{quality} {index_}");
    }
}

// From specs/world/vendors.md §7.2 r5: a quest item is refused even when
// not of type 39.
#[test]
fn quest_item_is_not_sold() {
    let (mut t, mut rec, mut w) = setup(class::CHARSI, 0);
    let skc_i = index(&t, "skc");
    t.items[skc_i].quest = 1;
    let skc = add(&mut w, &t, "skc", 0x20);
    assert_eq!(sell(&t, &mut rec, &mut w, p(), &sell_msg(skc, 0)), Ok(3));
    assert_eq!(w.sent[0].code, 9);
}

// From specs/world/vendors.md §7.2 r9: a failed unequip destroys the copy
// (only the copy leaves the store).
#[test]
fn failed_unequip_removes_only_the_copy() {
    let (t, mut rec, mut w) = setup(class::CHARSI, 0);
    let other = add(&mut w, &t, "dgr", 0x10);
    rec.store.push(UnitId(other));
    let e = add(&mut w, &t, "skc", 0x23);
    w.units.get_mut(&e).unwrap().mode = 1;
    w.unequip_ok = false;
    assert_eq!(sell(&t, &mut rec, &mut w, p(), &sell_msg(e, 1)), Ok(1));
    assert_eq!(rec.store, vec![UnitId(other)]);
}

// From specs/world/vendors.md §8.1 r3: a full throwing stack needs no
// repair; one below max does; charges alone do.
#[test]
fn repair_all_needs() {
    let (t, _, mut w) = setup(class::CHARSI, 0);
    let tkn = add(&mut w, &t, "tkn", 0x20);
    w.set(tkn, stat::QUANTITY, 60);
    w.equipped = vec![tkn];
    w.set(PLAYER, stat::GOLD, 10_000);
    repair(&t, &mut w, p(), &rmsg(0, true)).unwrap();
    assert_eq!(w.gold(), 10_000);
    w.set(tkn, stat::QUANTITY, 3);
    repair(&t, &mut w, p(), &rmsg(0, true)).unwrap();
    assert!(w.gold() < 10_000);
    assert_eq!(w.get(tkn, stat::QUANTITY), 60);
    // Charges not full, full durability.
    let (_, _, mut w) = setup(class::CHARSI, 0);
    let skc = add(&mut w, &t, "skc", 0x30);
    w.units.get_mut(&skc).unwrap().extra.charges = vec![(0, (5 << 8) | 3)];
    w.equipped = vec![skc];
    w.set(PLAYER, stat::GOLD, 100_000);
    repair(&t, &mut w, p(), &rmsg(0, true)).unwrap();
    assert!(w.gold() < 100_000);
}

// From specs/world/vendors.md §8.1 r4: a non-ethereal throwable stack
// below max goes on; a full one or a non-throwable stack does not.
#[test]
fn repair_one_throwing_stack_rule() {
    let (t, _, mut w) = setup(class::CHARSI, 0);
    let tkn = add(&mut w, &t, "tkn", 0x20);
    w.inventory.push(tkn);
    w.set(PLAYER, stat::GOLD, 1000);
    w.set(tkn, stat::QUANTITY, 3);
    repair(&t, &mut w, p(), &rmsg(tkn, false)).unwrap();
    assert_eq!(w.sent.last().unwrap().code, 2);
    w.set(tkn, stat::QUANTITY, 60);
    repair(&t, &mut w, p(), &rmsg(tkn, false)).unwrap();
    assert_eq!(w.sent.last().unwrap().code, 9);
    // A stackable but not throwable item at full durability.
    let (mut t2, _, mut w) = setup(class::CHARSI, 0);
    let lax_i = index(&t2, "lax");
    t2.items[lax_i].stackable = 1;
    t2.items[lax_i].maxstack = 10;
    let lax = add(&mut w, &t2, "lax", 0x21);
    w.set(lax, stat::QUANTITY, 3);
    w.inventory.push(lax);
    w.set(PLAYER, stat::GOLD, 1000);
    repair(&t2, &mut w, p(), &rmsg(lax, false)).unwrap();
    assert_eq!(w.sent.last().unwrap().code, 9);
    // Not stackable, maxstack 10, quantity 3: no stack either.
    t2.items[lax_i].stackable = 0;
    repair(&t2, &mut w, p(), &rmsg(lax, false)).unwrap();
    assert_eq!(w.sent.last().unwrap().code, 9);
}

// From specs/world/vendors.md §8.1 r6: a stack at full durability without
// gold → code 12 (no partial step).
#[test]
fn repair_without_gold_at_full_durability() {
    let (t, _, mut w) = setup(class::CHARSI, 0);
    let tkn = add(&mut w, &t, "tkn", 0x20);
    w.inventory.push(tkn);
    w.set(tkn, stat::QUANTITY, 3);
    repair(&t, &mut w, p(), &rmsg(tkn, false)).unwrap();
    assert_eq!(w.sent.last().unwrap().code, 12);
}

// From specs/world/vendors.md §8.1 r6: per = c·1024/(max − durability);
// needs per < g·1024.
#[test]
fn partial_repair_per_and_bound() {
    // lax durability 5 of 20: c = (15·100/20)·128/1024 = 9.
    let (t, _, mut w) = setup(class::CHARSI, 0);
    let lax = add(&mut w, &t, "lax", 0x20);
    w.set(lax, stat::DURABILITY, 5);
    w.inventory.push(lax);
    w.set(PLAYER, stat::GOLD, 5);
    repair(&t, &mut w, p(), &rmsg(lax, false)).unwrap();
    let per = 9 * 1024 / 15;
    assert_eq!(w.get(lax, stat::DURABILITY), 5 + 5 * 1024 / per);
    // c = 10 at durability 18 of 20 (cost 800): per = 5120 = g·1024 for
    // g = 5 → code 12.
    let (mut t, _, mut w) = setup(class::CHARSI, 0);
    t.items.push(item("big", 800, T_WEAP));
    let big = add(&mut w, &t, "big", 0x21);
    w.set(big, stat::DURABILITY, 18);
    w.inventory.push(big);
    w.set(PLAYER, stat::GOLD, 5);
    repair(&t, &mut w, p(), &rmsg(big, false)).unwrap();
    assert_eq!(w.sent.last().unwrap().code, 12);
    assert_eq!(w.get(big, stat::DURABILITY), 18);
}

// From specs/world/vendors.md §4 r3: shown store items with filled sockets
// get flag 1 added.
#[test]
fn shown_socketed_items_keep_their_flags() {
    let (t, mut rec, mut w) = setup(class::CHARSI, 0);
    let dgr = add(&mut w, &t, "dgr", 0x12);
    w.units.get_mut(&dgr).unwrap().sockets = true;
    rec.store.push(UnitId(dgr));
    // One already flagged: the flag stays set.
    let sbw = add(&mut w, &t, "sbw", 0x13);
    w.units.get_mut(&sbw).unwrap().sockets = true;
    w.units.get_mut(&sbw).unwrap().flags |= flag::NEW;
    rec.store.push(UnitId(sbw));
    rec.store_generated = true;
    let mut seed = Seed::new(1, 666);
    let mut c = StoreCtx {
        tables: &t,
        seed: &mut seed,
    };
    open(&mut c, &mut rec, &mut w, npc(), p(), true, false, 0).unwrap();
    assert_eq!(w.unit(dgr).flags, flag::IDENTIFIED | flag::NEW);
    assert_eq!(w.unit(sbw).flags, flag::IDENTIFIED | flag::NEW);
}

// From specs/world/vendors.md §6 r3: a record needs its act, trader, has
// traded and store generated.
#[test]
fn refresh_needs_all_four() {
    let t = tables();
    let fresh = || {
        let mut r = record(&t, class::CHARSI, 0);
        (r.trader, r.has_traded, r.store_generated) = (true, true, true);
        r
    };
    let edits: [&dyn Fn(&mut VendorRecord); 4] = [
        &|r| r.act = 1,
        &|r| r.trader = false,
        &|r| r.has_traded = false,
        &|r| r.store_generated = false,
    ];
    for (k, e) in edits.iter().enumerate() {
        let mut recs = [fresh()];
        e(&mut recs[0]);
        let mut w = Fake::new(class::CHARSI);
        refresh_act(&mut recs, &mut w, 0, false, REFRESH_MS + 1);
        assert!(!recs[0].refresh_pending, "edit {k}");
    }
    let mut recs = [fresh()];
    let mut w = Fake::new(class::CHARSI);
    refresh_act(&mut recs, &mut w, 0, false, REFRESH_MS + 1);
    assert!(recs[0].refresh_pending);
}

// From specs/world/vendors.md §6 r1: the act of a town level.
#[test]
fn town_acts() {
    assert_eq!(town_act(1), Some(0));
    assert_eq!(town_act(40), Some(1));
    assert_eq!(town_act(109), Some(4));
    assert_eq!(town_act(2), None);
}

// From specs/world/vendors.md §8.1 r6: the partial step needs 0 < per.
#[test]
fn partial_repair_needs_positive_per() {
    // cost 20, durability 0 of 3000: c = 20·128/1024 = 2; per =
    // 2·1024/3000 = 0; gold 1 < c → code 12.
    let (mut t, _, mut w) = setup(class::CHARSI, 0);
    t.items.push(item("chp", 20, T_WEAP));
    let g = add(&mut w, &t, "chp", 0x20);
    w.set(g, stat::MAXDURABILITY, 3000);
    w.set(g, stat::DURABILITY, 0);
    w.inventory.push(g);
    w.set(PLAYER, stat::GOLD, 1);
    repair(&t, &mut w, p(), &rmsg(g, false)).unwrap();
    assert_eq!(w.sent.last().unwrap().code, 12);
    assert_eq!((w.gold(), w.get(g, stat::DURABILITY)), (1, 0));
}

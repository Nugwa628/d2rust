// Spec: specs/world/vendors.md §7, §8, §9.1 (Test vectors, edge cases 3,
// 4, 11, 12)
use super::*;
use crate::items::q;
use crate::world::vendors::trade::{
    buy, pay, receive, repair, repair_item, sell, BuyMsg, RepairMsg, SellMsg,
};

mod mutant_tests;

/// The NPC GUID of the recording.
const NPC6: u32 = 6;

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// A world trading with `class` (NPC GUID 6) and its record.
fn setup(class: u16, act: u8) -> (VendorTables, VendorRecord, Fake) {
    let t = tables();
    let rec = record(&t, class, act);
    let mut w = Fake::new(class);
    w.npcs.insert(NPC6, (class, false));
    w.interact.insert(PLAYER, NPC6);
    (t, rec, w)
}

/// An identified item at full durability 20.
fn add(w: &mut Fake, t: &VendorTables, c: &str, guid: u32) -> u32 {
    w.next = guid;
    let g = w.add_item(index(t, c), q::NORMAL, flag::IDENTIFIED);
    w.set(g, stat::DURABILITY, 20);
    w.set(g, stat::MAXDURABILITY, 20);
    g
}

fn buy_msg(item: u32, txn: u32, fill: bool) -> BuyMsg {
    BuyMsg {
        npc: NPC6,
        item,
        txn,
        fill,
        client_price: 0,
    }
}

fn tr(kind: u8, code: u8, guid: u32, gold: i32) -> Transaction {
    Transaction {
        kind,
        code,
        guid,
        gold,
    }
}

// Covers: specs/world/vendors.md §7.1 text, §7.1 r3, §7.1 r9, §7.1 r10, §7.1 r11, §7.1 r12
#[test]
fn recorded_buy() {
    let (t, mut rec, mut w) = setup(class::CHARSI, 0);
    let dgr = add(&mut w, &t, "dgr", 0x12);
    rec.store.push(UnitId(dgr));
    w.set(PLAYER, stat::GOLD, 500);
    let m = BuyMsg::parse(&hex("32 06000000 12000000 00000000 38000000")).unwrap();
    assert_eq!(
        (m.npc, m.item, m.txn, m.fill, m.client_price),
        (6, 0x12, 0, false, 56)
    );
    assert_eq!(buy(&t, &mut rec, &mut w, p(), &m), Ok(0));
    let copy = w.backpack[0];
    assert_eq!(w.sent, vec![tr(4, 0, copy, 444)]);
    assert_eq!(w.gold(), 444);
    assert_eq!(w.last_bought, copy);
    assert_eq!(w.unit(copy).mode, mode::CURSOR);
    assert!(w.unit(copy).flags & flag::TARGET != 0);
    // The store item is taken out (the client drops it).
    assert_eq!(w.taken, vec![dgr]);
    assert!(w.unit(dgr).uflags & unit_flag::TAKEN != 0);
    assert!(rec.store.is_empty());
    // The client price is never read.
    let mut m2 = m;
    m2.client_price = 1;
    let d2 = add(&mut w, &t, "dgr", 0x40);
    rec.store.push(UnitId(d2));
    m2.item = d2;
    assert_eq!(buy(&t, &mut rec, &mut w, p(), &m2), Ok(0));
    assert_eq!(w.gold(), 388);
}

// Covers: specs/world/vendors.md §7.1 r6, §7.1 r12
#[test]
fn recorded_permanent_buy() {
    let (t, mut rec, mut w) = setup(class::AKARA, 0);
    let yps = add(&mut w, &t, "yps", 0x30);
    rec.store.push(UnitId(yps));
    w.set(PLAYER, stat::GOLD, 444);
    assert_eq!(
        buy(&t, &mut rec, &mut w, p(), &buy_msg(yps, 0, false)),
        Ok(0)
    );
    assert_eq!(w.gold(), 404);
    assert_eq!(rec.store, vec![UnitId(yps)], "store copy stays");
    assert!(w.taken.is_empty());
}

// Covers: specs/world/vendors.md §7.1 text, §7.1 r1, §7.1 r2, §7.1 r4, §7.1 r5
#[test]
fn buy_refusals() {
    let (t, mut rec, mut w) = setup(class::CHARSI, 0);
    let dgr = add(&mut w, &t, "dgr", 0x12);
    let other = add(&mut w, &t, "dgr", 0x13);
    rec.store.push(UnitId(dgr));
    w.set(PLAYER, stat::GOLD, 10);
    let mut run = |w: &mut Fake, m: BuyMsg| {
        w.sent.clear();
        let r = buy(&t, &mut rec, w, p(), &m).unwrap();
        (r, w.sent[0])
    };
    let mut wrong_npc = buy_msg(dgr, 0, false);
    wrong_npc.npc = 99;
    assert_eq!(run(&mut w, wrong_npc), (1, tr(0, 9, NO_GUID, 10)));
    assert_eq!(
        run(&mut w, buy_msg(0x99, 0, false)),
        (1, tr(0, 7, 0x99, 10))
    );
    assert_eq!(
        run(&mut w, buy_msg(other, 0, false)),
        (1, tr(0, 7, other, 10))
    );
    assert_eq!(run(&mut w, buy_msg(dgr, 2, false)), (1, tr(0, 7, dgr, 10)));
    assert_eq!(
        run(&mut w, buy_msg(dgr, 0, false)),
        (0, tr(0, 12, NO_GUID, 10))
    );
    w.set(PLAYER, stat::GOLD, 100);
    w.cursor = true;
    assert_eq!(
        run(&mut w, buy_msg(dgr, 0, false)),
        (1, tr(0, 7, NO_GUID, 100))
    );
    // Stash gold counts.
    w.cursor = false;
    w.set(PLAYER, stat::GOLD, 6);
    w.set(PLAYER, stat::GOLD_BANK, 50);
    assert_eq!(run(&mut w, buy_msg(dgr, 0, false)).1.code, 0);
    assert_eq!(w.get(PLAYER, stat::GOLD_BANK), 0);
}

// Covers: specs/world/vendors.md §edge-cases-original-bugs r3
#[test]
fn buy_with_other_transactions() {
    let (t, mut rec, mut w) = setup(class::CHARSI, 0);
    // Not offered, but t = 1 skips the test and pays the sell price.
    let skc = add(&mut w, &t, "skc", 0x20);
    w.set(PLAYER, stat::GOLD, 1000);
    assert_eq!(
        buy(&t, &mut rec, &mut w, p(), &buy_msg(skc, 1, false)),
        Ok(0)
    );
    assert_eq!(w.gold(), 500);
    assert_eq!(w.sent[0].code, 0);
}

// Covers: specs/world/vendors.md §7.1 r7
#[test]
fn scroll_into_tome() {
    let (t, mut rec, mut w) = setup(class::AKARA, 0);
    let isc = add(&mut w, &t, "isc", 0x20);
    let tome = add(&mut w, &t, "ibk", 0x21);
    rec.store.push(UnitId(isc));
    w.tome = Some((tome, 5));
    w.set(PLAYER, stat::GOLD, 100);
    assert_eq!(
        buy(&t, &mut rec, &mut w, p(), &buy_msg(isc, 0, true)),
        Ok(0)
    );
    // Not permanent: fill cleared, k = 1.
    assert_eq!((w.gold(), w.get(tome, stat::QUANTITY)), (84, 1));
    assert_eq!(w.sent, vec![tr(5, 0, tome, 84)]);
    assert_eq!(w.taken, vec![isc], "a store scroll is taken");
    // Permanent with fill: k = min(gold / price, free).
    rec.perm.push(code("isc"));
    rec.store.push(UnitId(isc));
    assert_eq!(
        buy(&t, &mut rec, &mut w, p(), &buy_msg(isc, 0, true)),
        Ok(0)
    );
    assert_eq!((w.gold(), w.get(tome, stat::QUANTITY)), (84 - 5 * 16, 6));
    w.tome = Some((tome, 10));
    assert_eq!(
        buy(&t, &mut rec, &mut w, p(), &buy_msg(isc, 0, true)),
        Ok(0)
    );
    // Gold 4 is below the price 16: refused by rule 4.
    assert_eq!(w.sent.last().unwrap().code, 12);
    assert_eq!(w.get(tome, stat::QUANTITY), 6);
}

// Covers: specs/world/vendors.md §7.1 r8
#[test]
fn fill_a_stack() {
    let (t, mut rec, mut w) = setup(class::CHARSI, 0);
    // A permanent stack whose unit price is 2 (cost 2048 · 1/1024).
    let mut t = t;
    let aqv = index(&t, "aqv");
    t.items[aqv].cost = 2048;
    let a = add(&mut w, &t, "aqv", 0x30);
    w.set(a, stat::QUANTITY, 500);
    w.set(a, stat::MAXDURABILITY, 0);
    rec.store.push(UnitId(a));
    let stack = add(&mut w, &t, "aqv", 0x31);
    w.set(stack, stat::QUANTITY, 100);
    w.stack = Some((stack, 400));
    // Rule 4 prices the whole store stack: 2048·500/1024 = 1000 → 937.
    w.set(PLAYER, stat::GOLD, 936);
    assert_eq!(buy(&t, &mut rec, &mut w, p(), &buy_msg(a, 0, true)), Ok(0));
    assert_eq!(w.sent, vec![tr(0, 12, NO_GUID, 936)]);
    // u = 2·960/1024 = 1; a = 1000; k = min(400, 1000).
    w.sent.clear();
    w.set(PLAYER, stat::GOLD, 1000);
    assert_eq!(buy(&t, &mut rec, &mut w, p(), &buy_msg(a, 0, true)), Ok(0));
    assert_eq!(w.get(stack, stat::QUANTITY), 500);
    assert_eq!(w.gold(), 600);
    assert_eq!(w.stat_msgs, vec![(stack, stat::QUANTITY)]);
    assert_eq!(w.sent, vec![tr(5, 0, stack, 600)]);
    assert_eq!(rec.store, vec![UnitId(a)], "permanent stays");
    // A full stack: k = 0 → code 12.
    w.stack = Some((stack, 0));
    w.set(PLAYER, stat::GOLD, 1000);
    assert_eq!(buy(&t, &mut rec, &mut w, p(), &buy_msg(a, 0, true)), Ok(0));
    assert_eq!(w.sent.last().unwrap().code, 12);
    // No stack: one copy of n = min(max stack, a) at n·u.
    w.stack = None;
    assert_eq!(buy(&t, &mut rec, &mut w, p(), &buy_msg(a, 0, true)), Ok(0));
    let copy = *w.backpack.last().unwrap();
    assert_eq!(w.get(copy, stat::QUANTITY), 500);
    assert_eq!(w.gold(), 500);
}

// Covers: specs/world/vendors.md §7.1 r9, §edge-cases-original-bugs r4
#[test]
fn fill_into_the_belt() {
    let (t, mut rec, mut w) = setup(class::AKARA, 0);
    w.difficulty = 1;
    let hp4 = add(&mut w, &t, "hp4", 0x30);
    w.set(hp4, stat::MAXDURABILITY, 0);
    rec.store.push(UnitId(hp4));
    w.belt = true;
    w.set(PLAYER, stat::GOLD, 100);
    // Nightmare hp4 fills: 3 buys of 30, then payment fails.
    assert_eq!(
        buy(&t, &mut rec, &mut w, p(), &buy_msg(hp4, 0, true)),
        Ok(0)
    );
    let codes: Vec<u8> = w.sent.iter().map(|s| s.code).collect();
    assert_eq!(codes, vec![0, 0, 0, 12]);
    assert_eq!(w.gold(), 10);
    assert!(w.destroyed.is_empty(), "the unpaid copy is not destroyed");
    // Belt full after 2: the third copy is undone silently.
    w.sent.clear();
    w.belt_room = 2;
    w.set(PLAYER, stat::GOLD, 100);
    assert_eq!(
        buy(&t, &mut rec, &mut w, p(), &buy_msg(hp4, 0, true)),
        Ok(0)
    );
    assert_eq!(w.sent.len(), 2);
    assert_eq!(w.gold(), 40);
    assert_eq!(w.destroyed.len(), 1);
    // Normal: no fill, one buy.
    w.difficulty = 0;
    w.sent.clear();
    w.belt_room = 10;
    assert_eq!(
        buy(&t, &mut rec, &mut w, p(), &buy_msg(hp4, 0, true)),
        Ok(0)
    );
    assert_eq!(w.sent.len(), 1);
}

// Covers: specs/world/vendors.md §7.1 r9
#[test]
fn no_room() {
    let (t, mut rec, mut w) = setup(class::CHARSI, 0);
    let dgr = add(&mut w, &t, "dgr", 0x12);
    rec.store.push(UnitId(dgr));
    w.set(PLAYER, stat::GOLD, 100);
    w.backpack_ok = false;
    assert_eq!(
        buy(&t, &mut rec, &mut w, p(), &buy_msg(dgr, 0, false)),
        Ok(0)
    );
    assert_eq!(w.sent, vec![tr(0, 10, NO_GUID, 100)], "gold restored");
    assert_eq!(w.destroyed.len(), 1);
    assert_eq!(rec.store, vec![UnitId(dgr)]);
    // Arrows the player can equip need no room.
    w.ammo = true;
    w.sent.clear();
    assert_eq!(
        buy(&t, &mut rec, &mut w, p(), &buy_msg(dgr, 0, false)),
        Ok(0)
    );
    assert_eq!(w.sent[0].code, 0);
}

// Covers: specs/world/vendors.md §5.2, §5.3, §7.1 r12
#[test]
fn buy_a_gambled_item() {
    let (t, mut rec, mut w) = setup(class::GHEED, 0);
    let hax = add(&mut w, &t, "hax", 0x40);
    w.units.get_mut(&hax).unwrap().flags = 0;
    w.units.get_mut(&hax).unwrap().extra.format = 101;
    rec.gamble_lists.push(GambleList {
        player: PLAYER,
        items: vec![UnitId(hax)],
    });
    w.set(PLAYER, stat::GOLD, 1000);
    assert_eq!(
        buy(&t, &mut rec, &mut w, p(), &buy_msg(hax, 2, false)),
        Ok(0)
    );
    assert_eq!(w.gold(), 1000 - 771);
    assert!(rec.gamble_lists[0].items.is_empty());
    assert_eq!(w.removed, vec![hax]);
    let copy = w.backpack[0];
    assert_eq!(
        w.unit(copy).flags & flag::IDENTIFIED,
        0,
        "stays unidentified"
    );
}

// Covers: specs/world/vendors.md §7.2 text, §7.2 r6, §7.2 r8, §7.2 r9, §7.2 r10
#[test]
fn recorded_sell() {
    let (t, mut rec, mut w) = setup(class::CHARSI, 0);
    let skc = add(&mut w, &t, "skc", 7);
    w.units.get_mut(&skc).unwrap().mode = mode::CURSOR;
    let m = SellMsg::parse(&hex("33 06000000 07000000 0400 0000 f4010000")).unwrap();
    assert_eq!((m.npc, m.item, m.mode, m.client_price), (6, 7, 4, 500));
    assert_eq!(sell(&t, &mut rec, &mut w, p(), &m), Ok(0));
    assert_eq!(w.gold(), 500);
    assert_eq!(w.sent, vec![tr(3, 1, 7, 500)]);
    // The restored copy is in the store, marked and shown.
    let copy = rec.store[0];
    assert_ne!(copy.0, skc);
    assert!(w.trade_inv.contains(&copy.0));
    assert!(w.unit(copy.0).uflags & unit_flag::VENDOR != 0);
    assert_eq!(w.get(copy.0, stat::DURABILITY), 20);
}

// Covers: specs/world/vendors.md §7.2 r1, §7.2 r2, §7.2 r3, §7.2 r4, §7.2 r5
#[test]
fn sell_refusals() {
    let (t, mut rec, mut w) = setup(class::CHARSI, 0);
    let skc = add(&mut w, &t, "skc", 0x20);
    let m = |item| SellMsg {
        npc: NPC6,
        item,
        mode: 0,
        client_price: 0,
    };
    let mut run = |w: &mut Fake, m: SellMsg| {
        w.sent.clear();
        let r = sell(&t, &mut rec, w, p(), &m).unwrap();
        (r, w.sent.first().map(|s| s.code))
    };
    assert_eq!(run(&mut w, m(0x99)), (1, None));
    let mut wrong = m(skc);
    wrong.npc = 99;
    assert_eq!(run(&mut w, wrong), (1, Some(9)));
    w.owned = false;
    assert_eq!(run(&mut w, m(skc)), (3, Some(11)));
    w.owned = true;
    let mut wrong_mode = m(skc);
    wrong_mode.mode = 4;
    assert_eq!(run(&mut w, wrong_mode), (3, Some(9)));
    w.units.get_mut(&skc).unwrap().flags |= flag::NOSELL;
    assert_eq!(run(&mut w, m(skc)), (3, Some(9)));
    let hdm = add(&mut w, &t, "hdm", 0x21);
    assert_eq!(run(&mut w, m(hdm)), (3, Some(9)), "quest item");
    assert_eq!(w.gold(), 0);
}

// Covers: specs/world/vendors.md §7.2 r7, §7.2 r8
#[test]
fn resell_rules() {
    let t = tables();
    type Edit<'a> = &'a dyn Fn(&mut Fake, u32);
    let cases: [(&str, Edit); 7] = [
        ("cracked", &|w, g| {
            let u = w.units.get_mut(&g).unwrap();
            u.quality = 1;
            u.file_index = 0;
        }),
        ("broken", &|w, g| {
            w.units.get_mut(&g).unwrap().flags |= flag::BROKEN
        }),
        ("personalized", &|w, g| {
            w.units.get_mut(&g).unwrap().flags |= flag::PERSONALIZED
        }),
        ("ethereal", &|w, g| {
            w.units.get_mut(&g).unwrap().flags |= flag::ETHEREAL
        }),
        ("sockets", &|w, g| {
            w.units.get_mut(&g).unwrap().sockets = true
        }),
        ("crude is fine", &|w, g| {
            let u = w.units.get_mut(&g).unwrap();
            u.quality = 1;
            u.file_index = 1;
        }),
        ("plain", &|_, _| {}),
    ];
    for (name, f) in cases {
        let (_, mut rec, mut w) = setup(class::CHARSI, 0);
        let skc = add(&mut w, &t, "skc", 0x20);
        f(&mut w, skc);
        let m = SellMsg {
            npc: NPC6,
            item: skc,
            mode: 0,
            client_price: 0,
        };
        assert_eq!(sell(&t, &mut rec, &mut w, p(), &m), Ok(0), "{name}");
        let copied = !rec.store.is_empty();
        assert_eq!(copied, name == "plain" || name == "crude is fine", "{name}");
    }
    // An ear and a gamble-mode node: no copy; a permanent: no copy.
    let (_, mut rec, mut w) = setup(class::CHARSI, 0);
    let ear = add(&mut w, &t, "ear", 0x20);
    let skc = add(&mut w, &t, "skc", 0x21);
    let aqv = add(&mut w, &t, "aqv", 0x22);
    let m = |item| SellMsg {
        npc: NPC6,
        item,
        mode: 0,
        client_price: 0,
    };
    sell(&t, &mut rec, &mut w, p(), &m(ear)).unwrap();
    sell(&t, &mut rec, &mut w, p(), &m(aqv)).unwrap();
    rec.chain_node_mut(PLAYER).gamble_mode = true;
    sell(&t, &mut rec, &mut w, p(), &m(skc)).unwrap();
    assert!(rec.store.is_empty());
    assert_eq!(w.sent.len(), 3);
    // No store page: the copy is destroyed; no room: destroyed.
    let (mut t2, mut rec, mut w) = setup(class::CHARSI, 0);
    let skc_i = index(&t2, "skc");
    t2.items[skc_i].type_ = T_NOPAGE;
    let skc = add(&mut w, &t2, "skc", 0x20);
    sell(&t2, &mut rec, &mut w, p(), &m(skc)).unwrap();
    assert_eq!((rec.store.len(), w.destroyed.len()), (0, 1));
    let (_, mut rec, mut w) = setup(class::CHARSI, 0);
    w.store_room = [0; 4];
    let skc = add(&mut w, &t, "skc", 0x20);
    sell(&t, &mut rec, &mut w, p(), &m(skc)).unwrap();
    assert_eq!((rec.store.len(), w.destroyed.len()), (0, 1));
    assert_eq!(w.gold(), 500, "still paid");
}

// Covers: specs/world/vendors.md §7.2 r8, §edge-cases-original-bugs r11
#[test]
fn sell_pays_the_restored_price() {
    let (t, mut rec, mut w) = setup(class::CHARSI, 0);
    let skc = add(&mut w, &t, "skc", 0x20);
    // A non-stack carrying quantity 5: B ×= 5 = 2500 at sale; the copy's
    // quantity becomes max stack 0 → 1 → 500.
    w.set(skc, stat::QUANTITY, 5);
    let m = SellMsg {
        npc: NPC6,
        item: skc,
        mode: 0,
        client_price: 0,
    };
    assert_eq!(sell(&t, &mut rec, &mut w, p(), &m), Ok(0));
    assert_eq!(w.gold(), 500);
}

// Covers: specs/world/vendors.md §7.2 r9
#[test]
fn sell_by_mode() {
    let (t, mut rec, mut w) = setup(class::AKARA, 0);
    let m = |item, mode| SellMsg {
        npc: NPC6,
        item,
        mode,
        client_price: 0,
    };
    let isc = add(&mut w, &t, "isc", 0x20);
    let ibk = add(&mut w, &t, "ibk", 0x21);
    w.set(ibk, stat::QUANTITY, 7);
    w.units.get_mut(&ibk).unwrap().extra.suffix[0] = 1;
    sell(&t, &mut rec, &mut w, p(), &m(isc, 0)).unwrap();
    sell(&t, &mut rec, &mut w, p(), &m(ibk, 0)).unwrap();
    assert_eq!(w.book_lowered, vec![1, 7]);
    assert!(w.removed.contains(&isc) && w.removed.contains(&ibk));
    // Cursor take fails: code 9, result 1.
    let c = add(&mut w, &t, "skc", 0x22);
    w.units.get_mut(&c).unwrap().mode = 4;
    w.cursor_ok = false;
    w.sent.clear();
    assert_eq!(sell(&t, &mut rec, &mut w, p(), &m(c, 4)), Ok(1));
    assert_eq!(w.sent[0].code, 9);
    // Unequip fails: the copy is destroyed, no message.
    let e = add(&mut w, &t, "skc", 0x23);
    w.units.get_mut(&e).unwrap().mode = 1;
    w.unequip_ok = false;
    w.sent.clear();
    let n = rec.store.len();
    assert_eq!(sell(&t, &mut rec, &mut w, p(), &m(e, 1)), Ok(1));
    assert!(w.sent.is_empty());
    assert_eq!(rec.store.len(), n);
    assert_eq!(w.destroyed.len(), 1);
}

// Covers: specs/world/vendors.md §9.1
#[test]
fn gold_transfers() {
    let mut w = Fake::new(class::CHARSI);
    w.set(PLAYER, stat::GOLD, 10);
    w.set(PLAYER, stat::GOLD_BANK, 100);
    assert!(!pay(&mut w, p(), 111));
    assert!(pay(&mut w, p(), 10));
    assert_eq!((w.gold(), w.get(PLAYER, stat::GOLD_BANK)), (0, 100));
    w.set(PLAYER, stat::GOLD, 10);
    assert!(pay(&mut w, p(), 50));
    assert_eq!((w.gold(), w.get(PLAYER, stat::GOLD_BANK)), (0, 60));
    w.set(PLAYER, stat::GOLD, 10);
    w.stash_cap = 10;
    assert!(pay(&mut w, p(), 20));
    assert_eq!(w.get(PLAYER, stat::GOLD_BANK), 0, "above the stash cap");
    // Receive: cap 1000.
    w.gold_cap = 1000;
    w.set(PLAYER, stat::GOLD, 900);
    receive(&mut w, p(), 50);
    assert_eq!(w.gold(), 950);
    receive(&mut w, p(), 80);
    assert_eq!((w.gold(), w.dropped_gold), (1000, 30));
    receive(&mut w, p(), 5);
    assert_eq!((w.gold(), w.dropped_gold), (1000, 35));
}

fn rmsg(item: u32, all: bool) -> RepairMsg {
    RepairMsg {
        npc: NPC6,
        item,
        all,
    }
}

// Covers: specs/world/vendors.md §8.1 text, §8.1 r1, §8.1 r2, §8.1 r4, §8.1 r5
#[test]
fn repair_one() {
    let (t, _, mut w) = setup(class::CHARSI, 0);
    let lax = add(&mut w, &t, "lax", 0x20);
    w.set(lax, stat::DURABILITY, 0);
    w.set(PLAYER, stat::GOLD, 100);
    let m = RepairMsg::parse(&hex("35 06000000 20000000 0000 0000 00000000")).unwrap();
    assert_eq!(m, rmsg(0x20, false));
    assert_eq!(repair(&t, &mut w, p(), &m), Ok(3), "not in the inventory");
    assert_eq!(w.sent[0].code, 9);
    w.inventory.push(lax);
    w.sent.clear();
    // R = 100 → ·128/1024 = 12.
    assert_eq!(repair(&t, &mut w, p(), &m), Ok(0));
    assert_eq!(w.sent, vec![tr(1, 2, NO_GUID, 88)]);
    assert_eq!(w.get(lax, stat::DURABILITY), 20);
    assert_eq!(w.stat_msgs, vec![(lax, stat::DURABILITY)]);
    // Full: code 9, routine result 1 (rule 7). Unidentified: code 9.
    w.sent.clear();
    assert_eq!(repair(&t, &mut w, p(), &m), Ok(1));
    assert_eq!(w.sent[0].code, 9);
    w.set(lax, stat::DURABILITY, 3);
    w.units.get_mut(&lax).unwrap().flags = 0;
    w.sent.clear();
    repair(&t, &mut w, p(), &m).unwrap();
    assert_eq!(w.sent[0].code, 9);
    // Not a repairer; not the interact unit.
    let (_, _, mut w) = setup(class::AKARA, 0);
    repair(&t, &mut w, p(), &m).unwrap();
    assert_eq!(w.sent[0].code, 9);
    let (_, _, mut w) = setup(class::CHARSI, 0);
    w.interact.clear();
    repair(&t, &mut w, p(), &m).unwrap();
    assert_eq!(w.sent[0].code, 9);
}

// Covers: specs/world/vendors.md §8.1 r6, §edge-cases-original-bugs r12
#[test]
fn repair_partial() {
    let (t, _, mut w) = setup(class::CHARSI, 0);
    let lax = add(&mut w, &t, "lax", 0x20);
    w.set(lax, stat::DURABILITY, 0);
    w.inventory.push(lax);
    // c = 12; gold 5 + stash 3 < 12: per = 12·1024/20 = 614.
    w.set(PLAYER, stat::GOLD, 5);
    w.set(PLAYER, stat::GOLD_BANK, 3);
    assert_eq!(repair(&t, &mut w, p(), &rmsg(lax, false)), Ok(0));
    assert_eq!(w.get(lax, stat::DURABILITY), 5 * 1024 / 614);
    assert_eq!((w.gold(), w.get(PLAYER, stat::GOLD_BANK)), (0, 3));
    assert_eq!(w.sent, vec![tr(1, 2, NO_GUID, 0)]);
    // No gold: code 12.
    w.sent.clear();
    repair(&t, &mut w, p(), &rmsg(lax, false)).unwrap();
    assert_eq!(w.sent, vec![tr(0, 12, NO_GUID, 0)]);
    // Broken: code 12.
    w.set(PLAYER, stat::GOLD, 5);
    w.set(lax, stat::DURABILITY, 0);
    w.units.get_mut(&lax).unwrap().flags |= flag::BROKEN;
    w.sent.clear();
    repair(&t, &mut w, p(), &rmsg(lax, false)).unwrap();
    assert_eq!(w.sent[0].code, 12);
}

// Covers: specs/world/vendors.md §8.1 r3
#[test]
fn repair_all() {
    let (t, _, mut w) = setup(class::CHARSI, 0);
    let a = add(&mut w, &t, "lax", 0x20);
    let b = add(&mut w, &t, "dgr", 0x21);
    let c = add(&mut w, &t, "skc", 0x22);
    w.set(a, stat::DURABILITY, 10);
    w.set(b, stat::DURABILITY, 0);
    w.equipped = vec![a, b, c];
    w.set(PLAYER, stat::GOLD, 100);
    // Charsi rep 128: lax 50·128/1024 = 6; dgr 60·128/1024 = 7.
    assert_eq!(repair(&t, &mut w, p(), &rmsg(0, true)), Ok(0));
    assert_eq!(w.gold(), 100 - 13);
    assert_eq!(
        (w.get(a, stat::DURABILITY), w.get(b, stat::DURABILITY)),
        (20, 20)
    );
    assert_eq!(w.sent, vec![tr(1, 2, NO_GUID, 87)]);
    // Nothing to repair: code 2, kind 1, free.
    w.sent.clear();
    repair(&t, &mut w, p(), &rmsg(0, true)).unwrap();
    assert_eq!(w.sent, vec![tr(1, 2, NO_GUID, 87)]);
    // Not enough: code 12.
    w.set(a, stat::DURABILITY, 0);
    w.set(PLAYER, stat::GOLD, 1);
    w.sent.clear();
    repair(&t, &mut w, p(), &rmsg(0, true)).unwrap();
    assert_eq!(w.sent, vec![tr(0, 12, NO_GUID, 1)]);
}

// Covers: specs/world/vendors.md §8.2
#[test]
fn repairing_an_item() {
    let (t, _, mut w) = setup(class::CHARSI, 0);
    // A throwing stack: quantity := max stack, recharge, durability.
    let tkn = add(&mut w, &t, "tkn", 0x20);
    w.set(tkn, stat::QUANTITY, 3);
    w.set(tkn, stat::DURABILITY, 4);
    repair_item(&t, &mut w, UnitId(tkn), Some(p()));
    assert_eq!(w.get(tkn, stat::QUANTITY), 60);
    assert_eq!(w.get(tkn, stat::DURABILITY), 20);
    assert_eq!(
        w.stat_msgs,
        vec![(tkn, stat::QUANTITY), (tkn, stat::DURABILITY)]
    );
    assert!(w.log.contains(&format!("recharge {tkn}")));
    // Broken: the broken-item repair, then step 3 of `0x0055F900`
    // (`generation.md` §12.1; recorded, `items-vendor-charsi-repair`):
    // durability := max, the 0x3E to the player when one is given.
    let lax = add(&mut w, &t, "lax", 0x21);
    let max = w.get(lax, stat::MAXDURABILITY);
    assert!(max > 0);
    w.set(lax, stat::DURABILITY, 0);
    w.units.get_mut(&lax).unwrap().flags |= flag::BROKEN;
    repair_item(&t, &mut w, UnitId(lax), Some(p()));
    assert!(w.log.contains(&format!("repair broken {lax}")));
    assert_eq!(w.get(lax, stat::DURABILITY), max);
    assert_eq!(w.stat_msgs.last(), Some(&(lax, stat::DURABILITY)));
    // Not repairable: nothing.
    let n = w.log.len();
    let yps = add(&mut w, &t, "yps", 0x22);
    repair_item(&t, &mut w, UnitId(yps), None);
    assert_eq!(w.log.len(), n);
}

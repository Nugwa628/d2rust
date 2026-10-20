//! Tests for NPC ↔ vendors on the real providers: the NPC menu opens a real store
//! (items created by the economy wiring on the game seed, stats on the
//! real lists), a buy pays from real gold; the trade open lends the
//! NPC-control seed to the hire list.

use super::*;
use crate::items::q;
use crate::world::npc::{code as npc_code, NpcVendors};
use crate::world::vendors::price::{cost, PriceCtx, PriceItem};
use crate::world::vendors::{flag, store, tx, unit_flag, Transaction};

/// Item flag 0x2 set on a bought copy.
const FLAG_TARGET: u32 = 0x2;

#[test]
fn npc_menu_opens_a_real_store_and_buy_pays_real_gold() {
    let mut w = World::new(false);
    let player = w.spawn(UnitType::Player, 0);
    let npc = w.npc(class::AKARA);
    w.set(player, &[(st::GOLD, PLAYER_GOLD), (st::LEVEL, 1)]);
    let m = msg(0x13, &[1, w.guid(npc)]);
    assert_eq!(w.desk(|d, ctl| ctl.interact(d, player, &m)), Ok(Some(0)));
    // C→S 0x38 action 1 (trade) opens the store and generates its items.
    let before = w.fields.seed;
    let m = msg(0x38, &[1, w.guid(npc), 0]);
    assert_eq!(w.desk(|d, ctl| ctl.menu_action(d, player, &m)), Ok(0));
    let i = w.state.vendor_index(class::AKARA).unwrap();
    let rec = &w.state.vendors[i];
    assert!(rec.has_traded && rec.store_generated);
    assert_eq!(rec.last_npc, w.guid(npc));
    // One permanent cap, made by the real item creation: two game-seed
    // steps (allocation + creation), none of the
    // NPC-control seed.
    assert_eq!(rec.store.len(), 1, "{:?}", w.state.errors);
    let item = rec.store[0];
    let mut seed = before;
    seed.step();
    seed.step();
    assert_eq!(w.fields.seed, seed);
    let it = w.items.get(item).unwrap();
    assert_eq!((it.record, it.quality), (CAP, q::NORMAL));
    assert_eq!(it.inv_page, 0, "helm store page 0 (§3.1 rule 3)");
    assert_ne!(it.flags & flag::IDENTIFIED, 0);
    assert_ne!(w.units.get(item).unwrap().flags2 & unit_flag::VENDOR, 0);
    assert_eq!(w.rest.store, [(class::AKARA, item)]);
    // The store's repair step needs an identified item: the item creation
    // identifies it. The new cap is already at full durability.
    let max = w.stat(item, st::MAXDURABILITY);
    assert_eq!(max, 12);
    // The price from the item's real fields and stats.
    let it = w.items.get(item).unwrap();
    let pi = PriceItem {
        record: it.record,
        quality: it.quality,
        flags: it.flags,
        file_index: it.file_index,
        format: it.format,
        armor_base: w.stats.unit_base(item, 31, 0),
        durability: w.stat(item, st::DURABILITY),
        max_durability: max,
        ..PriceItem::default()
    };
    let ctx = PriceCtx {
        difficulty: 0,
        npc_class: class::AKARA,
        reduced_prices: 0,
        player_level: 1,
        quest_slots: [0; 3],
    };
    let price = cost(&w.vendor_tables, &ctx, Some(&pi), tx::BUY).unwrap();
    assert!(price > 0);
    // C→S 0x32: buy it.
    let (ng, ig) = (w.guid(npc), w.guid(item));
    let m = msg(0x32, &[ng, ig, tx::BUY, 0]);
    assert_eq!(m.len(), 17);
    let r = w.desk(|d, ctl| d.vendors(Some(ctl)).buy(player, &m));
    assert_eq!(r, Some(0));
    assert_eq!(w.stat(player, st::GOLD), PLAYER_GOLD - price);
    assert_eq!(w.rest.last_bought[&player], ig);
    assert_eq!(w.units.get(item).unwrap().mode, 4);
    assert_ne!(w.items.get(item).unwrap().flags & FLAG_TARGET, 0);
    assert_eq!(
        w.rest.transactions,
        [(
            player,
            Transaction {
                kind: 4,
                code: 0,
                guid: ig,
                gold: PLAYER_GOLD - price,
            }
        )]
    );
    // Not enough gold for a second one: code 12, nothing paid.
    w.set(player, &[(st::GOLD, price - 1)]);
    w.desk(|d, ctl| d.vendors(Some(ctl)).buy(player, &m));
    assert_eq!(w.stat(player, st::GOLD), price - 1);
    assert_eq!(
        w.rest.transactions.last().unwrap().1.code,
        npc_code::NO_GOLD
    );
    w.assert_clean();
}

#[test]
fn store_item_creation_is_never_ethereal_and_identified() {
    use crate::world::vendors::VendorWorld;
    // An expansion game: the ethereal roll runs for item format >= 100.
    let mut w = World::new(true);
    w.rest.item_format = 101;
    let npc = w.npc(class::AKARA);
    // Item creation is called with never-ethereal set (request flags2
    // 0x02): no store or gamble item is ethereal; on success item flag
    // 0x10.
    for _ in 0..200 {
        let item = w
            .desk(|d, _| {
                let mut v = d.vendors(None);
                v.npc = Some(npc);
                VendorWorld::create_item(&mut v, class::AKARA, CAP, q::NORMAL, 6)
            })
            .expect("cap created");
        let f = w.items.get(item).unwrap().flags;
        assert_eq!(f & flag::ETHEREAL, 0, "store item ethereal");
        assert_ne!(f & flag::IDENTIFIED, 0, "store item not identified");
    }
    w.assert_clean();
}

#[test]
fn trade_open_lends_the_npc_seed_to_the_hire_list() {
    let mut w = World::new(false);
    let player = w.spawn(UnitType::Player, 0);
    let npc = w.npc(class::ASHEARA);
    w.set(player, &[(st::LEVEL, 1)]);
    // Asheara's list: gold with Min 0, Max 1 (one range draw on the
    // NPC-control seed), then the permanent cap.
    w.vendor_tables.items[GOLD].columns[10] = [0, 1, 0, 0, 0];
    w.vendor_tables.items[GOLD].spawnable = 1;
    w.state = InteractionState::new(&w.ctl, &GlobalLists::build(&w.vendor_tables));
    w.state.add_npc(npc);
    // A seed whose draw gives n = 0 (no gold item is made).
    let start = (1u32..)
        .map(Seed::init_low)
        .find(|s| store::range(&mut s.clone(), 0, 2) == 0)
        .unwrap();
    w.ctl.seed = start;
    let mut after_store = start;
    store::range(&mut after_store, 0, 2);
    let mut want = w.ctl.clone();
    want.seed = after_store;
    want.make_hire_list(class::ASHEARA).unwrap();
    let r = w.desk(|d, ctl| d.open_trade(ctl, player, npc, true, false));
    assert_eq!(r, Ok(()));
    // The hire list was drawn after the store, on the same seed.
    assert_eq!(w.ctl.seed, want.seed);
    let got = w.ctl.record(class::ASHEARA).unwrap();
    let exp = want.record(class::ASHEARA).unwrap();
    assert!(got.hire_made);
    assert_eq!(got.hire, exp.hire);
    // Drawn from the seed before the store's draw, the list differs.
    let mut wrong = w.ctl.clone();
    wrong.seed = start;
    wrong.records.iter_mut().for_each(|r| {
        r.hire = None;
        r.hire_made = false;
    });
    wrong.make_hire_list(class::ASHEARA).unwrap();
    assert_ne!(wrong.record(class::ASHEARA).unwrap().hire, exp.hire);
    let i = w.state.vendor_index(class::ASHEARA).unwrap();
    assert_eq!(w.state.vendors[i].store.len(), 1);
    w.assert_clean();
}

//! The shop's display prices from the client's own game data: the price
//! inputs of a store item (`PriceItem`) and of the player at the NPC
//! (`PriceCtx`) built from the client model, priced with the vendor code
//! the server runs (`d2_sim::world::vendors::price::cost`) and written
//! into the [`ShopPrices`] board the shop panel reads.
//!
//! An online game has no server in the process to publish the board;
//! the client fills it itself (contracts 3.7 v2: the client computes
//! the prices it shows, the server prices every transaction on its
//! own). Single player keeps the server's board; the regression test
//! (`tests/app_shop_prices.rs`) checks this builder against it.
//!
//! The item's stats are rebuilt as the server holds them: an item stat
//! list with the item's base array (the record's own values, as the
//! client's 0x3E updates keep it) and one attached property list (the
//! stream's main list and its runeword list), on the same stat engine
//! (`d2_sim::stats::StatLists`), so the totals, the base values and the
//! (total − base) bonuses include the item-base ops (the enhanced
//! defense and durability percents) as the server's do.
//!
//! Gaps (D2RUST's own, unverified):
//! - the set lists of a set item are left out (a set item's price reads
//!   its set record, not its bonuses);
//! - the stat-204 charges follow the client's 0x3E updates of the
//!   item's lists; other list values stay as the last record sent them;
//! - the quest slots come from the player's quest record the last NPC
//!   dialog (S→C 0x28) carried; without one they are 0.

use std::sync::Arc;

use d2_proto::item_bits::{decode, hflag, ItemBits};
use d2_server::adapters::item_bits::TablesLookup;
use d2_server::world_data::game::GameTables;
use d2_sim::items::{ItemTables, LIST_FLAGS};
use d2_sim::stats::{key_layer, key_stat, StatData, StatHost, StatLists};
use d2_sim::units::{UnitId, UnitType};
use d2_sim::world::vendors::price::{self, Bonus, PriceCtx, PriceItem};
use d2_sim::world::vendors::{stat, trade, tx, StoreRoles, VendorTables};

use crate::bridge::item_lists::{ItemFields, ItemProp};
use crate::bridge::items::{self, mode};
use crate::bridge::world::{ClientWorld, ItemData, KindData, UnitKey};
use crate::ui::original::ShopPrices;
use d2_server::rest::REPAIR_ALL_KEY;

/// The tables a price reads: the vendor tables, the item tables (the
/// stream decode) and the stat data (the item's stat list).
#[derive(Clone)]
pub struct PriceTables {
    pub vendors: VendorTables,
    pub items: Arc<ItemTables>,
    pub stats: Arc<StatData>,
}

impl PriceTables {
    /// The tables of the user's files.
    pub fn live(t: &GameTables) -> anyhow::Result<Self> {
        Ok(Self {
            vendors: t.vendor_tables()?,
            items: Arc::new(t.item_tables()?),
            stats: Arc::new(t.stat_data()?),
        })
    }
}

/// The open shop: the NPC class and whether it is the gamble window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShopView {
    pub npc_class: u16,
    pub gamble: bool,
}

/// A stat host that answers nothing: an item's own list needs no world.
struct NoHost;

impl StatHost for NoHost {}

/// One item's stat list, built as the server holds it (module docs).
struct ItemList {
    lists: StatLists,
    unit: UnitId,
}

impl ItemList {
    fn new(
        stats: &Arc<StatData>,
        guid: u32,
        class: u32,
        base: &[ItemProp],
        props: &[ItemProp],
    ) -> Self {
        let mut h = NoHost;
        let mut lists = StatLists::new(stats.clone());
        let unit = UnitId(0);
        let l = lists.alloc_extended(&mut h, unit, UnitType::Item, guid, class, 0, None);
        for p in base {
            lists.unit_set(&mut h, unit, p.stat, p.value, p.layer);
        }
        if !props.is_empty() {
            // The property list (state 0, flags 0x40), attached with
            // reset as the item stat writes attach it.
            let pl = lists.alloc(LIST_FLAGS, 0, lists.owner_type(l), guid);
            lists.attach(&mut h, unit, pl, true);
            for p in props {
                lists.add(&mut h, pl, p.stat, p.value, p.layer);
            }
        }
        Self { lists, unit }
    }

    fn total(&self, s: u16) -> i32 {
        self.lists.unit_total(self.unit, s, 0)
    }

    /// The entries of `s` as (layer, value) in key order (the full array).
    fn entries(&self, s: u16) -> Vec<(u16, i32)> {
        self.full()
            .into_iter()
            .filter(|&(k, _)| key_stat(k) == s)
            .map(|(k, v)| (key_layer(k), v))
            .collect()
    }

    /// Every full-array entry whose (total − base) is not 0.
    fn bonuses(&self) -> Vec<Bonus> {
        self.full()
            .into_iter()
            .filter_map(|(k, _)| {
                let (stat, layer) = (key_stat(k), key_layer(k));
                let value = self.lists.unit_bonus(self.unit, stat, layer);
                (value != 0).then_some(Bonus { stat, layer, value })
            })
            .collect()
    }

    fn full(&self) -> Vec<(i32, i32)> {
        self.lists
            .unit_list(self.unit)
            .map(|l| self.lists.full_entries(l))
            .unwrap_or_default()
    }
}

/// The values of the item's property lists: the main list and the
/// runeword list (not the set lists), each value as the item's list holds
/// it; a stat-204 entry takes the client's current value (`props`, which
/// the 0x3E charge updates write).
fn list_props(t: &ItemTables, b: &ItemBits, d: &ItemData) -> Vec<ItemProp> {
    let shift = |s: u16| t.valshift.get(usize::from(s)).copied().unwrap_or(0);
    let main = b.lists.first().and_then(Option::as_ref);
    let runeword = if b.flags & hflag::RUNEWORD != 0 && b.lists.len() > 1 {
        b.lists.last().and_then(Option::as_ref)
    } else {
        None
    };
    main.into_iter()
        .chain(runeword)
        .flat_map(|list| crate::ui::item_tip_props::stream_values(list, shift))
        .map(|(stat, layer, value)| {
            let layer = layer as u16;
            let value = if stat == stat::CHARGED_SKILL {
                d.props
                    .iter()
                    .find(|p| p.stat == stat && p.layer == layer)
                    .map_or(value, |p| p.value)
            } else {
                value
            };
            ItemProp { stat, layer, value }
        })
        .collect()
}

/// The price inputs of item `key` from the client model; `None` when it
/// is not an item with a readable record.
pub fn price_item(t: &PriceTables, w: &ClientWorld, key: UnitKey) -> Option<PriceItem> {
    let KindData::Item(d) = &w.units.get(&key)?.kind else {
        return None;
    };
    let stream = &d.last.as_ref()?.stream;
    let b = decode(stream, &TablesLookup(&t.items))
        .ok()
        .filter(|b| !b.failed)?;
    let fields: ItemFields = match &d.fields {
        Some(f) => f.clone(),
        None => super::items::client_item_fields(&t.items, stream)?,
    };
    let props = list_props(&t.items, &b, d);
    let l = ItemList::new(&t.stats, key.guid, fields.class, &fields.base, &props);
    // The items in the item's sockets, as the model places them (the
    // record counts its filled sockets).
    let sockets = if b.filled == 0 {
        Vec::new()
    } else {
        items::items(w)
            .into_iter()
            .filter(|i| i.mode == mode::SOCKETED && i.owner == Some(key))
            .filter_map(|i| match &w.units.get(&i.key)?.kind {
                KindData::Item(s) => s.fields.as_ref().map(|f| f.class as usize),
                _ => None,
            })
            .collect()
    };
    Some(PriceItem {
        record: fields.class as usize,
        quality: fields.quality as u8,
        // Item data +0x18 as the model keeps it (flag 4 apart).
        flags: d.flags | if d.flags4 { 4 } else { 0 },
        file_index: fields.file_index,
        prefix: fields.prefix,
        suffix: fields.suffix,
        auto_affix: fields.auto,
        ear_level: b.ear.as_ref().map_or(0, |e| i32::from(e.level)),
        // Item data +0x30: the record's version field.
        format: b.version,
        quantity: l.total(stat::QUANTITY),
        armor_base: l.lists.unit_base(l.unit, stat::ARMORCLASS, 0),
        indestructible: l.total(stat::INDESTRUCTIBLE),
        durability: l.total(stat::DURABILITY),
        max_durability: l.total(stat::MAXDURABILITY),
        replenish_durability: l.total(stat::REPLENISH_DURABILITY),
        replenish_quantity: l.total(stat::REPLENISH_QUANTITY),
        extra_stack: l.total(stat::EXTRA_STACK),
        item_skills: l.entries(stat::ITEM_SINGLESKILL),
        charges: l.entries(stat::CHARGED_SKILL),
        bonuses: l.bonuses(),
        sockets,
    })
}

/// The price context of the local player at an NPC of `npc_class`; `quest`
/// is the player's quest record of the game's difficulty (S→C 0x28).
/// `None` without a local player.
pub fn price_ctx(
    t: &PriceTables,
    w: &ClientWorld,
    npc_class: u16,
    quest: Option<&[u8; 96]>,
) -> Option<PriceCtx> {
    let p = w.local_player?;
    let mut quest_slots = [0; 3];
    if let (Some(row), Some(q)) = (t.vendors.npc_row(npc_class), quest) {
        for (k, &(f, ..)) in row.quests.iter().enumerate() {
            let s = f as usize;
            if f != 0 {
                quest_slots[k] = q
                    .get(2 * s..2 * s + 2)
                    .map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
            }
        }
    }
    Some(PriceCtx {
        difficulty: w.difficulty,
        npc_class,
        reduced_prices: w.total(p, stat::REDUCED_PRICES, 0),
        player_level: w.total(p, stat::LEVEL, 0),
        quest_slots,
    })
}

/// The prices the shop shows: the buy price (the gamble price in a gamble
/// window) of every store item of the model by GUID, and for a repairer
/// the repair-all total of the player's equipped items that need repair
/// (under [`REPAIR_ALL_KEY`]), as the server computes them.
pub fn shop_prices(
    t: &PriceTables,
    w: &ClientWorld,
    shop: ShopView,
    quest: Option<&[u8; 96]>,
) -> Vec<(u32, u32)> {
    let Some(ctx) = price_ctx(t, w, shop.npc_class, quest) else {
        return Vec::new();
    };
    let kind = if shop.gamble { tx::GAMBLE } else { tx::BUY };
    let mut out: Vec<(u32, u32)> = items::store_items(w)
        .into_iter()
        .filter_map(|i| {
            let it = price_item(t, w, i.key)?;
            let c = price::cost(&t.vendors, &ctx, Some(&it), kind).ok()?;
            Some((i.key.guid, u32::try_from(c).ok()?))
        })
        .collect();
    if StoreRoles::of(&t.items.rules, shop.npc_class).repairs {
        let worn = items::local_items(w)
            .into_iter()
            .filter(|i| i.mode == mode::BODY)
            .filter_map(|i| price_item(t, w, i.key));
        let mut total = 0i32;
        let mut ok = true;
        for it in worn.filter(|it| trade::needs_repair(&t.vendors, it)) {
            match price::cost(&t.vendors, &ctx, Some(&it), tx::REPAIR) {
                Ok(c) => total = total.wrapping_add(c),
                Err(_) => ok = false,
            }
        }
        if ok {
            out.push((REPAIR_ALL_KEY, u32::try_from(total).unwrap_or(0)));
        }
    }
    out
}

/// [`shop_prices`] written into `board`.
pub fn publish(
    t: &PriceTables,
    w: &ClientWorld,
    shop: ShopView,
    quest: Option<&[u8; 96]>,
    board: &ShopPrices,
) {
    for (guid, price) in shop_prices(t, w, shop, quest) {
        board.set(guid, price);
    }
}

//! Buy (C→S 0x32), sell (0x33), repair (0x35), item repair and gold
//! transfers.

use super::price::{
    charges_not_full, cost, max_stack, repairable, replenishable_stack, PriceCtx, PriceFatal,
    PriceItem,
};
use super::store::{is_cracked, mark, place_store_page};
use super::{
    flag, mode, stat, tx, ty, unit_flag, StoreRoles, Transaction, VendorRecord, VendorTables,
    VendorWorld, HP4, HP5, MP4, MP5, NO_GUID,
};
use crate::units::UnitId;

fn le32(m: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([m[o], m[o + 1], m[o + 2], m[o + 3]])
}

/// The price context of a player at an NPC (the price inputs).
pub fn price_ctx<W: VendorWorld>(
    t: &VendorTables,
    w: &W,
    player: UnitId,
    npc_class: u16,
) -> PriceCtx {
    let d = w.difficulty();
    let mut quest_slots = [0; 3];
    if let Some(row) = t.npc_row(npc_class) {
        for (k, &(f, ..)) in row.quests.iter().enumerate() {
            if f != 0 {
                quest_slots[k] = w.quest_slot(player, d, f);
            }
        }
    }
    PriceCtx {
        difficulty: d,
        npc_class,
        reduced_prices: w.stat(player, stat::REDUCED_PRICES, 0),
        player_level: w.stat(player, stat::LEVEL, 0),
        quest_slots,
    }
}

/// Pay `c` gold from the player. False: not enough gold.
pub fn pay<W: VendorWorld>(w: &mut W, player: UnitId, c: i32) -> bool {
    let g = w.stat(player, stat::GOLD, 0);
    let s = w.stat(player, stat::GOLD_BANK, 0);
    if g.wrapping_add(s) < c {
        return false;
    }
    if c <= g {
        w.set_stat(player, stat::GOLD, 0, g - c);
    } else {
        w.set_stat(player, stat::GOLD, 0, 0);
        let mut s2 = s.wrapping_add(g.wrapping_sub(c));
        if s2 < 0 || w.stash_cap(player) < s2 {
            s2 = 0;
        }
        w.set_stat(player, stat::GOLD_BANK, 0, s2);
    }
    true
}

/// Receive `a` gold.
pub fn receive<W: VendorWorld>(w: &mut W, player: UnitId, a: i32) {
    let cap = w.gold_cap(player);
    let g = w.stat(player, stat::GOLD, 0);
    if g == cap {
        w.drop_gold(player, a);
    } else if (g as u32).wrapping_add(a as u32) > cap as u32 {
        w.set_stat(player, stat::GOLD, 0, cap);
        w.drop_gold(player, g.wrapping_add(a).wrapping_sub(cap));
    } else {
        w.set_stat(player, stat::GOLD, 0, g.wrapping_add(a));
    }
}

/// Repairing an item for a player.
pub fn repair_item<W: VendorWorld>(
    t: &VendorTables,
    w: &mut W,
    item: UnitId,
    player: Option<UnitId>,
) {
    let Some(it) = w.price_item(item) else { return };
    if !repairable(t, &it) {
        return;
    }
    let y = t.type_of(it.record);
    let stackable = t.item(it.record).is_some_and(|r| r.stackable != 0);
    if y.is_some_and(|y| y.throwable != 0) && stackable {
        w.set_stat(item, stat::QUANTITY, 0, max_stack(t, &it));
        if let Some(p) = player {
            w.send_item_stat(p, item, stat::QUANTITY);
        }
    }
    w.recharge(item);
    let broken = w.item_flags(item) & flag::BROKEN != 0;
    if broken {
        // The host's flags and update
        // list, then base stat 72 := max durability, 0x3E to the
        // owner.
        w.repair_broken(item);
    }
    let max = w.stat(item, stat::MAXDURABILITY, 0);
    if max > 0 {
        w.set_stat(item, stat::DURABILITY, 0, max);
        if let Some(p) = player {
            w.send_item_stat(p, item, stat::DURABILITY);
        }
    }
}

/// "Permanent item of this NPC, or in Nightmare / Hell hp4, hp5, mp4,
/// mp5" (buy rule 6).
pub fn is_permanent<W: VendorWorld>(
    t: &VendorTables,
    rec: &VendorRecord,
    w: &W,
    item: UnitId,
) -> bool {
    let Some(code) = t.item(w.item_record(item)).map(|r| r.code) else {
        return false;
    };
    rec.perm.contains(&code) || (w.difficulty() > 0 && [HP4, HP5, MP4, MP5].contains(&code))
}

fn send<W: VendorWorld>(w: &mut W, player: UnitId, kind: u8, code: u8, guid: u32) {
    let gold = w.stat(player, stat::GOLD, 0);
    w.send_transaction(
        player,
        Transaction {
            kind,
            code,
            guid,
            gold,
        },
    );
}

/// On-buy hook of a bought item (buy rule 12).
fn on_buy<W: VendorWorld>(
    t: &VendorTables,
    rec: &mut VendorRecord,
    w: &mut W,
    player: u32,
    item: UnitId,
    txn: u32,
) {
    if txn == tx::GAMBLE && rec.has_gamble {
        if let Some(g) = rec.gamble_lists.iter_mut().find(|g| g.player == player) {
            if let Some(i) = g.items.iter().position(|&x| x == item) {
                g.items.remove(i);
                w.remove_gamble_item(rec.class, player, item);
                return;
            }
        }
    }
    if is_permanent(t, rec, w, item) {
        return;
    }
    if let Some(i) = rec.store.iter().position(|&x| x == item) {
        rec.store.remove(i);
        w.or_unit_flags(item, unit_flag::TAKEN);
        w.take_from_store(rec.class, item);
    }
}

/// C→S 0x32 BuyItem (17 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuyMsg {
    pub npc: u32,
    pub item: u32,
    /// Bits 0–15.
    pub txn: u32,
    /// Bit 31.
    pub fill: bool,
    /// u32 @13, never read (buy rule 11).
    pub client_price: u32,
}

impl BuyMsg {
    pub fn parse(m: &[u8]) -> Option<Self> {
        if m.len() != 17 {
            return None;
        }
        let w = le32(m, 9);
        Some(BuyMsg {
            npc: le32(m, 1),
            item: le32(m, 5),
            txn: w & 0xFFFF,
            fill: w & 0x8000_0000 != 0,
            client_price: le32(m, 13),
        })
    }
}

/// Buy (C→S 0x32). `rec` is the
/// record of the message NPC's class. Returns the handler result.
pub fn buy<W: VendorWorld>(
    t: &VendorTables,
    rec: &mut VendorRecord,
    w: &mut W,
    player: UnitId,
    m: &BuyMsg,
) -> Result<u32, PriceFatal> {
    let pg = w.guid(player);
    let npc = w.npc_by_guid(m.npc);
    if !npc.is_some_and(|n| w.is_interact_unit(player, n)) {
        send(w, player, 0, 9, NO_GUID);
        return Ok(1);
    }
    // Rule 1.
    let Some(item) = w.item_by_guid(m.item) else {
        send(w, player, 0, 7, m.item);
        return Ok(1);
    };
    // Rule 2.
    let offered = match m.txn {
        tx::BUY => Some(rec.store.contains(&item)),
        tx::GAMBLE => Some(rec.gamble_list(pg).is_some_and(|g| g.items.contains(&item))),
        _ => None,
    };
    if offered == Some(false) {
        // Rules 1–2 carry the requested item GUID.
        send(w, player, 0, 7, m.item);
        return Ok(1);
    }
    // Rule 3.
    let ctx = price_ctx(t, w, player, rec.class);
    let it = w.price_item(item);
    let mut price = cost(t, &ctx, it.as_ref(), m.txn)?;
    let gold = |w: &W| {
        w.stat(player, stat::GOLD, 0)
            .wrapping_add(w.stat(player, stat::GOLD_BANK, 0))
    };
    // Rule 4.
    if gold(w) < price {
        send(w, player, 0, 12, NO_GUID);
        return Ok(0);
    }
    // Rule 5.
    if w.has_cursor_item(player) {
        send(w, player, 0, 7, NO_GUID);
        return Ok(1);
    }
    // Rule 6.
    let mut fill = m.fill && is_permanent(t, rec, w, item);
    let record = w.item_record(item);
    // Rule 7.
    if t.is_type(record, ty::SCRO) {
        if let Some((tome, f)) = w.find_tome(player, item) {
            let k = if fill {
                // Unsigned division (buy rule 7); a zero price reads as 0.
                let a = (gold(w) as u32).checked_div(price as u32).unwrap_or(0);
                (a as i64).min(i64::from(f)) as i32
            } else {
                1
            };
            if !pay(w, player, k.wrapping_mul(price)) {
                send(w, player, 0, 12, NO_GUID);
                return Ok(0);
            }
            on_buy(t, rec, w, pg, item, m.txn);
            w.add_to_tome(tome, k);
            let tg = w.guid(tome);
            send(w, player, 5, 0, tg);
            return Ok(0);
        }
    }
    // Rule 8.
    let mut n_stack = None;
    if fill && t.type_of(record).is_some_and(|y| y.autostack != 0) {
        let mut one = it.clone().unwrap_or_default();
        one.quantity = 1;
        let u = cost(t, &ctx, it.as_ref().map(|_| &one), tx::BUY)?;
        let a = (gold(w) as u32).checked_div(u as u32).unwrap_or(0);
        let found = w.find_partial_stack(player, item);
        let f = match found {
            Some((_, f)) => f,
            None => it.as_ref().map_or(0, |it| max_stack(t, it)),
        };
        let k = (i64::from(f)).min(i64::from(a)) as i32;
        match found {
            Some((stack, _)) => {
                if k < 1 || !pay(w, player, k.wrapping_mul(u)) {
                    send(w, player, 0, 12, NO_GUID);
                    return Ok(0);
                }
                on_buy(t, rec, w, pg, item, m.txn);
                let q = w.stat(stack, stat::QUANTITY, 0);
                w.set_stat(stack, stat::QUANTITY, 0, q.wrapping_add(k));
                w.send_item_stat(player, stack, stat::QUANTITY);
                let sg = w.guid(stack);
                send(w, player, 5, 0, sg);
                return Ok(0);
            }
            None => {
                let n = k.max(1);
                price = n.wrapping_mul(u);
                n_stack = Some(n);
            }
        }
    }
    // Rule 9.
    fill = fill && w.can_belt(player, item);
    let stackable = t.item(record).is_some_and(|r| r.stackable != 0);
    let mut bought = 0;
    loop {
        // 9.1
        if bought > 0 && !fill {
            return Ok(0);
        }
        // 9.2
        let Some(copy) = w.copy_item(item) else {
            send(w, player, 0, 9, NO_GUID);
            return Ok(1);
        };
        // 9.3
        if let (Some(n), true) = (n_stack, stackable) {
            w.set_stat(copy, stat::QUANTITY, 0, n);
        }
        // 9.4
        let before = (
            w.stat(player, stat::GOLD, 0),
            w.stat(player, stat::GOLD_BANK, 0),
        );
        if !pay(w, player, price) {
            send(w, player, 0, 12, NO_GUID);
            return Ok(0);
        }
        // 9.5
        let cg = w.guid(copy);
        w.set_last_bought(player, cg);
        w.set_item_mode(copy, mode::CURSOR);
        // 9.6
        let mut placed = false;
        if w.can_belt(player, copy) {
            placed = w.put_in_belt(player, copy);
            if !placed {
                fill = false;
            }
        }
        // 9.7
        if !placed && !w.equip_ammo(player, copy) {
            let undo = |w: &mut W| {
                w.set_stat(player, stat::GOLD, 0, before.0);
                w.set_stat(player, stat::GOLD_BANK, 0, before.1);
            };
            if bought > 0 {
                undo(w);
                w.destroy_item(copy);
                return Ok(0);
            }
            w.set_item_page(copy, 0);
            if !w.place_in_backpack(player, copy) {
                undo(w);
                w.destroy_item(copy);
                send(w, player, 0, 10, NO_GUID);
                return Ok(0);
            }
        }
        // 9.8
        on_buy(t, rec, w, pg, item, m.txn);
        let f = w.item_flags(copy);
        w.set_item_flags(copy, f | flag::TARGET);
        send(w, player, 4, 0, cg);
        bought += 1;
    }
}

/// C→S 0x33 SellItem (17 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SellMsg {
    pub npc: u32,
    pub item: u32,
    pub mode: u16,
    pub client_price: u32,
}

impl SellMsg {
    pub fn parse(m: &[u8]) -> Option<Self> {
        if m.len() != 17 {
            return None;
        }
        Some(SellMsg {
            npc: le32(m, 1),
            item: le32(m, 5),
            mode: u16::from_le_bytes([m[9], m[10]]),
            client_price: le32(m, 13),
        })
    }
}

/// Re-sellable (sell rule 7).
fn resellable<W: VendorWorld>(
    t: &VendorTables,
    rec: &VendorRecord,
    w: &W,
    player: u32,
    item: UnitId,
    it: &PriceItem,
) -> bool {
    let f = it.flags;
    let unique_blocked = it.quality == crate::items::q::UNIQUE
        && usize::try_from(it.file_index)
            .ok()
            .and_then(|i| t.uniques.get(i))
            .is_some_and(|u| u.flags & t.unique_nosell_mask != 0);
    !(is_cracked(t, w, item)
        || f & flag::BROKEN != 0
        || t.is_type(it.record, ty::PLAY)
        || f & flag::PERSONALIZED != 0
        || f & flag::ETHEREAL != 0
        || w.has_filled_sockets(item)
        || unique_blocked
        || rec.chain_node(player).is_some_and(|n| n.gamble_mode))
}

/// Sell (C→S 0x33). Returns the handler result.
pub fn sell<W: VendorWorld>(
    t: &VendorTables,
    rec: &mut VendorRecord,
    w: &mut W,
    player: UnitId,
    m: &SellMsg,
) -> Result<u32, PriceFatal> {
    let pg = w.guid(player);
    // Rule 1.
    let Some(item) = w.item_by_guid(m.item) else {
        return Ok(1);
    };
    // Rule 2.
    let npc = w.npc_by_guid(m.npc);
    if !npc.is_some_and(|n| w.is_interact_unit(player, n)) {
        send(w, player, 0, 9, NO_GUID);
        return Ok(1);
    }
    // Rule 3.
    if !w.owns_item(player, item) {
        send(w, player, 0, 11, NO_GUID);
        return Ok(3);
    }
    // Rule 4.
    let item_mode = w.item_mode(item);
    if item_mode != u32::from(m.mode) {
        send(w, player, 0, 9, NO_GUID);
        return Ok(3);
    }
    // Rule 5.
    let record = w.item_record(item);
    let quest = t.item(record).is_some_and(|r| r.quest != 0);
    if w.item_flags(item) & flag::NOSELL != 0 || quest || t.is_type(record, ty::QUEST) {
        send(w, player, 0, 9, NO_GUID);
        return Ok(3);
    }
    // Rule 6.
    let ctx = price_ctx(t, w, player, rec.class);
    let it = w.price_item(item);
    let mut price = cost(t, &ctx, it.as_ref(), tx::SELL)?;
    // Rules 7–8.
    let mut copy = None;
    let resell = it
        .as_ref()
        .is_some_and(|it| resellable(t, rec, w, pg, item, it));
    if resell && !is_permanent(t, rec, w, item) {
        let Some(c) = w.copy_item(item) else {
            send(w, player, 0, 9, NO_GUID);
            return Ok(3);
        };
        w.set_item_mode(c, mode::CURSOR);
        match place_store_page(t, w, rec.class, c) {
            None => {}
            Some(false) => w.destroy_item(c),
            Some(true) => {
                mark(w, rec.class, c);
                rec.store.push(c);
                let max = w.stat(c, stat::MAXDURABILITY, 0);
                w.set_stat(c, stat::DURABILITY, 0, max);
                // Sell rule 8: "quantity := max stack" for every
                // placed copy, stackable or not.
                if let Some(ci) = w.price_item(c) {
                    let ms = max_stack(t, &ci);
                    w.set_stat(c, stat::QUANTITY, 0, ms);
                }
                let restored = w.price_item(c);
                price = price.min(cost(t, &ctx, restored.as_ref(), tx::SELL)?);
                copy = Some(c);
            }
        }
    }
    // Rule 9.
    match item_mode {
        mode::CURSOR => {
            if !w.take_from_cursor(player, item) {
                send(w, player, 0, 9, NO_GUID);
                return Ok(1);
            }
        }
        mode::STORED => {
            if t.is_type(record, ty::SCRO) {
                w.lower_book_skill(player, item, 1);
            } else if t.is_type(record, ty::BOOK) {
                let qn = w.stat(item, stat::QUANTITY, 0);
                if qn >= 0 {
                    w.lower_book_skill(player, item, qn);
                }
            }
            w.remove_stored(player, item);
        }
        _ => {
            if !w.unequip(player, item) {
                if let Some(c) = copy {
                    if let Some(i) = rec.store.iter().position(|&x| x == c) {
                        rec.store.remove(i);
                    }
                    w.destroy_item(c);
                }
                return Ok(1);
            }
        }
    }
    // Rule 10.
    receive(w, player, price);
    send(w, player, 3, 1, m.item);
    Ok(0)
}

/// C→S 0x35 Repair (17 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RepairMsg {
    pub npc: u32,
    pub item: u32,
    pub all: bool,
}

impl RepairMsg {
    pub fn parse(m: &[u8]) -> Option<Self> {
        if m.len() != 17 {
            return None;
        }
        Some(RepairMsg {
            npc: le32(m, 1),
            item: le32(m, 5),
            all: le32(m, 13) & 0x8000_0000 != 0,
        })
    }
}

/// "Needs repair" of repair all (repair rule 3).
pub fn needs_repair(t: &VendorTables, it: &PriceItem) -> bool {
    (it.max_durability != 0 && it.durability != it.max_durability)
        || (replenishable_stack(t, it) && it.quantity < max_stack(t, it))
        || charges_not_full(it)
}

/// The repair-all quote of rule 3: the summed repair cost of the equipped
/// items that need repair, and those items.
pub fn repair_all_quote<W: VendorWorld>(
    t: &VendorTables,
    ctx: &PriceCtx,
    w: &W,
    player: UnitId,
) -> Result<(i32, Vec<UnitId>), PriceFatal> {
    let mut total = 0i32;
    let mut todo = Vec::new();
    for item in w.equipped_items(player) {
        let Some(it) = w.price_item(item) else {
            continue;
        };
        if needs_repair(t, &it) {
            total = total.wrapping_add(cost(t, ctx, Some(&it), tx::REPAIR)?);
            todo.push(item);
        }
    }
    Ok((total, todo))
}

/// Repair (C→S 0x35). Returns the routine's own result (rule 7):
/// 1 for rules 1, 2, rule 4's "not repairable" and "nothing to
/// repair" and repair-all's failed payment; 3 for rule 4's "missing or
/// not in the inventory"; 0 otherwise. The message handler drops it
/// (0 for every 17-byte message).
pub fn repair<W: VendorWorld>(
    t: &VendorTables,
    w: &mut W,
    player: UnitId,
    m: &RepairMsg,
) -> Result<u32, PriceFatal> {
    // Rules 1–2.
    let npc = w.npc_by_guid(m.npc);
    let Some(npc) = npc.filter(|&n| w.is_interact_unit(player, n)) else {
        send(w, player, 0, 9, NO_GUID);
        return Ok(1);
    };
    let class = w.npc_class(npc);
    if !StoreRoles::of(w.rules(), class).repairs {
        send(w, player, 0, 9, NO_GUID);
        return Ok(1);
    }
    let ctx = price_ctx(t, w, player, class);
    // Rule 3.
    if m.all {
        let (total, todo) = repair_all_quote(t, &ctx, w, player)?;
        if total == 0 {
            send(w, player, 1, 2, NO_GUID);
            return Ok(0);
        }
        if !pay(w, player, total) {
            send(w, player, 0, 12, NO_GUID);
            return Ok(1);
        }
        for item in todo {
            repair_item(t, w, item, Some(player));
        }
        send(w, player, 1, 2, NO_GUID);
        return Ok(0);
    }
    // Rule 4.
    let item = w
        .item_by_guid(m.item)
        .filter(|&i| w.in_inventory(player, i));
    let Some(item) = item else {
        send(w, player, 0, 9, NO_GUID);
        return Ok(3);
    };
    let Some(it) = w.price_item(item).filter(|it| repairable(t, it)) else {
        send(w, player, 0, 9, NO_GUID);
        return Ok(1);
    };
    let ms = max_stack(t, &it);
    let throw_stack = it.flags & flag::ETHEREAL == 0
        && t.type_of(it.record).is_some_and(|y| y.throwable != 0)
        && t.item(it.record).is_some_and(|r| r.stackable != 0)
        && it.quantity < ms;
    if !throw_stack
        && !charges_not_full(&it)
        && !(it.max_durability != 0 && it.durability < it.max_durability)
    {
        send(w, player, 0, 9, NO_GUID);
        return Ok(1);
    }
    // Rule 5.
    let c = cost(t, &ctx, Some(&it), tx::REPAIR)?;
    if pay(w, player, c) {
        repair_item(t, w, item, Some(player));
        send(w, player, 1, 2, NO_GUID);
        return Ok(0);
    }
    // Rule 6.
    let g = w.stat(player, stat::GOLD, 0);
    let (max, dur) = (it.max_durability, it.durability);
    if dur < max {
        let per = (c as u32).wrapping_mul(1024) / (max - dur) as u32;
        let g1024 = (g as u32).wrapping_mul(1024);
        if g > 0 && 0 < per && per < g1024 && it.flags & flag::BROKEN == 0 {
            w.set_stat(player, stat::GOLD, 0, 0);
            let add = (g1024 / per) as i32;
            w.set_stat(item, stat::DURABILITY, 0, max.min(dur.wrapping_add(add)));
            w.send_item_stat(player, item, stat::DURABILITY);
            send(w, player, 1, 2, NO_GUID);
            return Ok(0);
        }
    }
    send(w, player, 0, 12, NO_GUID);
    Ok(0)
}

//! Store generation, one store item, trade and gamble open and the
//! refresh rules.

use super::price::PriceFatal;
use super::trade::repair_item;
use super::{
    flag, gamble, stat, store_level, unit_flag, EventNode, StoreRoles, VendorRecord, VendorTables,
    VendorWorld, AQV, CQV, CRACKED, FAIL_LIMIT, NO_CODE, REFRESH_MS, TOWNS, XXX,
};
use crate::items::q;
use crate::rng::Seed;
use crate::units::UnitId;
use crate::world::npc::NpcRoles;

/// The quality draw of a store item: one step, r = lo' mod 100.
pub fn quality_draw(seed: &mut Seed, ilvl: i32) -> u8 {
    let r = seed.step() % 100;
    match ilvl {
        i if i < 5 => {
            if r > 90 {
                q::LOW
            } else {
                q::NORMAL
            }
        }
        i if i < 10 => {
            if r > 85 {
                q::SUPERIOR
            } else {
                q::NORMAL
            }
        }
        _ => {
            if r > 74 {
                q::SUPERIOR
            } else {
                q::NORMAL
            }
        }
    }
}

/// range(min, max): `min` with no draw if max ≤ min
/// (signed), else roll(max − min) + min.
pub fn range(seed: &mut Seed, min: i32, max: i32) -> i32 {
    seed.roll_range(min, max.wrapping_sub(min))
}

/// Whether a created item is an inferior `Cracked`.
pub fn is_cracked<W: VendorWorld>(t: &VendorTables, w: &W, item: UnitId) -> bool {
    w.item_quality(item) == q::LOW
        && usize::try_from(w.item_file_index(item))
            .ok()
            .and_then(|i| t.lowquality.get(i))
            .is_some_and(|n| n.as_slice() == CRACKED)
}

/// The NPC side of one store generation.
pub struct StoreCtx<'a> {
    pub tables: &'a VendorTables,
    /// The NPC-control seed.
    pub seed: &'a mut Seed,
}

/// Upgrade code choice: one roll(100000) in Nightmare or
/// Hell when L_p > 25. Returns the chosen record and whether the chosen
/// code is in the code map: a code the map lacks is class index 0 (`hax`:
/// the code lookup gives 0), whose creation never matches the chosen code
/// (a null item).
fn upgrade<W: VendorWorld>(
    t: &VendorTables,
    seed: &mut Seed,
    w: &W,
    record: usize,
    ilvl: i32,
    player_level: i32,
) -> (usize, bool) {
    let d = w.difficulty();
    let Some(rec) = t.item(record) else {
        return (record, true);
    };
    if d == 0 || player_level <= 25 {
        return (record, true);
    }
    // Rule 1 tests `ubercode` / `ultracode` only for ≠ 0 and ≠ spaces;
    // the lookup comes after the choice.
    let set = |c: [u8; 4]| c != [0; 4] && c != NO_CODE;
    let find = |c: [u8; 4]| t.find_code(c).map_or((0, false), |i| (i, true));
    let r = seed.roll(100_000) as i32;
    let mut code = (record, true);
    if d == 1 {
        if r < ilvl * 64 + 4000 && set(rec.ubercode) {
            code = find(rec.ubercode);
        } else if rec.nightmare_upgrade != XXX {
            code = find(rec.nightmare_upgrade);
        }
    } else {
        if w.expansion() && r < ilvl * 16 + 1000 && set(rec.ultracode) {
            code = find(rec.ultracode);
        } else if r < ilvl * 128 + 5000 && set(rec.ubercode) {
            code = find(rec.ubercode);
        }
        if rec.hell_upgrade != XXX {
            code = find(rec.hell_upgrade);
        }
    }
    code
}

/// Mark a store item.
pub fn mark<W: VendorWorld>(w: &mut W, class: u16, item: UnitId) {
    let mut f = w.item_flags(item) | flag::IDENTIFIED;
    if w.has_filled_sockets(item) {
        f |= flag::NEW;
    }
    w.set_item_flags(item, f);
    w.or_unit_flags(item, unit_flag::VENDOR);
    w.add_trade_inventory(class, item);
}

/// Places an item in the store on its store page; page 1 retries on page
/// 2. Returns false (item not destroyed) when there is no
/// room or no store page; `None` store page → the item is destroyed.
pub(crate) fn place_store_page<W: VendorWorld>(
    t: &VendorTables,
    w: &mut W,
    class: u16,
    item: UnitId,
) -> Option<bool> {
    let page = t.type_of(w.item_record(item)).map_or(0xFF, |y| y.storepage);
    if page == 0xFF {
        w.destroy_item(item);
        return None;
    }
    w.set_item_page(item, page);
    if w.place_in_store(class, item) {
        return Some(true);
    }
    if page == 1 {
        w.set_item_page(item, 2);
        if w.place_in_store(class, item) {
            return Some(true);
        }
    }
    Some(false)
}

/// One store item. Returns the placed item, or null;
/// a null creation call is a fatal assert.
pub fn make_store_item<W: VendorWorld>(
    c: &mut StoreCtx,
    rec: &mut VendorRecord,
    w: &mut W,
    record: usize,
    quality: u8,
    ilvl: i32,
    player_level: i32,
) -> Result<Option<UnitId>, PriceFatal> {
    let t = c.tables;
    let (chosen, found) = upgrade(t, c.seed, w, record, ilvl, player_level);
    let mut q = quality;
    let mut made = None;
    'rounds: for _ in 0..2 {
        let mut item = None;
        for _ in 0..5 {
            // A null creation ends the try loop and the code read of none
            // is a fatal assert (rule 2, line 0x61A).
            match w.create_item(rec.class, chosen, q, ilvl) {
                Some(i) if is_cracked(t, w, i) => w.destroy_item(i),
                Some(i) => {
                    item = Some(i);
                    break;
                }
                None => return Err(PriceFatal::NullStoreItem),
            }
        }
        let Some(i) = item else {
            return Ok(None);
        };
        // A code mismatch destroys the item; in round 2 the result is
        // null (rule 2, V7). An unfound code made class 0, which never
        // equals the chosen code.
        if !found || w.item_record(i) != chosen {
            w.destroy_item(i);
            q = q::NORMAL;
            continue 'rounds;
        }
        made = Some(i);
        break;
    }
    let Some(item) = made else {
        return Ok(None);
    };
    // Rule 3–4.
    let page = t.type_of(w.item_record(item)).map_or(0xFF, |y| y.storepage);
    if page == 0xFF {
        w.destroy_item(item);
        return Ok(None);
    }
    w.set_item_page(item, page);
    repair_item(t, w, item, None);
    let mut placed = w.place_in_store(rec.class, item);
    if !placed && page == 1 {
        w.set_item_page(item, 2);
        placed = w.place_in_store(rec.class, item);
    }
    if !placed {
        rec.events.push(EventNode {
            unit: item,
            arg: 0,
            kind: 0,
            deferred: true,
        });
        return Ok(None);
    }
    // Rule 5.
    mark(w, rec.class, item);
    rec.store.push(item);
    Ok(Some(item))
}

/// Store generation. `now` is the host's
/// millisecond clock.
pub fn generate<W: VendorWorld>(
    c: &mut StoreCtx,
    rec: &mut VendorRecord,
    w: &mut W,
    player: UnitId,
    now: u32,
) -> Result<(), PriceFatal> {
    let t = c.tables;
    rec.store_time = now;
    let lp = w.stat(player, stat::LEVEL, 0);
    let ilvl = store_level(lp, w.difficulty(), rec.act);
    let mut fails = 0u32;
    let entries = rec.items.clone();
    for e in &entries {
        // Every list code is an item record's code, so the map
        // finds it; a missing one (unreachable) is skipped.
        let Some(record) = t.find_code(e.code) else {
            continue;
        };
        let r = &t.items[record];
        // Step 1.
        if i32::from(r.level) > ilvl {
            continue;
        }
        // Step 2.
        let n_norm = if ilvl < 25 {
            range(c.seed, i32::from(e.min), i32::from(e.max) + 1)
        } else {
            0
        };
        // Step 3.
        if r.version >= 100 && w.item_format() < 100 {
            continue;
        }
        // Step 4.
        for _ in 0..n_norm {
            let q = quality_draw(c.seed, ilvl);
            if make_store_item(c, rec, w, record, q, ilvl, lp)?.is_none() {
                fails += 1;
                if fails > FAIL_LIMIT {
                    return Ok(());
                }
            }
        }
        // Step 5.
        if r.bitfield1 & 1 != 0 && i32::from(e.magic_lvl) <= ilvl {
            let k = if ilvl >= 25 {
                range(c.seed, 1, 3) + 1
            } else {
                1
            };
            let n_mag = range(c.seed, i32::from(e.magic_min), i32::from(e.magic_max) + k);
            for _ in 0..n_mag {
                if make_store_item(c, rec, w, record, q::MAGIC, ilvl, lp)?.is_none() {
                    fails += 1;
                }
            }
        }
    }
    let perm = rec.perm.clone();
    for code in perm {
        // As above: every permanent code is in the map (unreachable skip).
        if let Some(record) = t.find_code(code) {
            match make_store_item(c, rec, w, record, q::NORMAL, ilvl, lp)? {
                None => fails += 1,
                Some(item) => {
                    if code == CQV || code == AQV {
                        if let Some(it) = w.price_item(item) {
                            let m = super::price::max_stack(t, &it);
                            w.set_stat(item, stat::QUANTITY, 0, m);
                        }
                    }
                }
            }
        }
        if fails > FAIL_LIMIT {
            return Ok(());
        }
    }
    Ok(())
}

/// Clearing a record's data.
pub fn clear_record<W: VendorWorld>(rec: &mut VendorRecord, w: &mut W) {
    // Every gamble list: items removed and destroyed, as a gamble refresh.
    for g in std::mem::take(&mut rec.gamble_lists) {
        for item in g.items {
            w.remove_gamble_item(rec.class, g.player, item);
            w.destroy_item(item);
        }
    }
    // Each store item out of the NPC's inventory, then freed.
    for item in std::mem::take(&mut rec.store) {
        w.remove_store_item(rec.class, item);
        w.destroy_item(item);
    }
    for e in std::mem::take(&mut rec.events) {
        if e.deferred {
            w.destroy_item(e.unit);
        }
    }
}

/// Adds every store or gamble-list item to the NPC's trade inventory
/// (trade open rule 3).
fn show_items<W: VendorWorld>(rec: &VendorRecord, w: &mut W, player: u32, gamble: bool) {
    let items: Vec<UnitId> = if gamble {
        rec.gamble_list(player)
            .map(|g| g.items.clone())
            .unwrap_or_default()
    } else {
        rec.store.clone()
    };
    for item in items {
        w.or_unit_flags(item, unit_flag::VENDOR);
        if w.has_filled_sockets(item) {
            let f = w.item_flags(item);
            w.set_item_flags(item, f | flag::NEW);
        }
        w.add_trade_inventory(rec.class, item);
    }
}

/// Trade or gamble open at `npc` (`is_gamble`: the gamble window).
#[allow(clippy::too_many_arguments)]
pub fn open<W: VendorWorld>(
    c: &mut StoreCtx,
    rec: &mut VendorRecord,
    w: &mut W,
    npc: UnitId,
    player: UnitId,
    single: bool,
    is_gamble: bool,
    now: u32,
) -> Result<(), PriceFatal> {
    let pg = w.guid(player);
    // Rule 1.
    rec.has_traded = true;
    rec.last_npc = w.guid(npc);
    // Rule 2: a trade open.
    if !is_gamble {
        if rec.trader {
            rec.chain_node_mut(pg).gamble_mode = false;
            if !rec.store_generated {
                generate(c, rec, w, player, now)?;
                rec.store_generated = true;
            }
            if !w.hire_list_made(rec.class) {
                if NpcRoles::of(w.rules(), rec.class).hires {
                    w.make_hire_list(rec.class, c.seed);
                }
                w.set_hire_list_made(rec.class);
            }
        }
    } else if rec.has_gamble {
        rec.chain_node_mut(pg).gamble_mode = true;
        if rec.gamble_list(pg).is_none() {
            gamble::make_list(c, rec, w, player);
        }
    }
    // Rule 3.
    if StoreRoles::of(w.rules(), rec.class).no_refresh {
        return Ok(());
    }
    if !is_gamble && single && rec.refresh_pending {
        rec.refresh_pending = false;
        clear_record(rec, w);
        w.new_store_inventory(rec.class, None);
        generate(c, rec, w, player, now)?;
        rec.store_generated = true;
    }
    w.refresh_npc_inventory(npc);
    show_items(rec, w, pg, is_gamble);
    Ok(())
}

/// Refresh rule 3 (`act`, `empty`) over the game's records.
pub fn refresh_act<W: VendorWorld>(
    records: &mut [VendorRecord],
    w: &mut W,
    act: u8,
    empty: bool,
    now: u32,
) {
    for rec in records.iter_mut() {
        if rec.act != act || !rec.trader || !rec.has_traded || !rec.store_generated {
            continue;
        }
        if !empty {
            if rec.store_time.wrapping_add(REFRESH_MS) < now {
                rec.refresh_pending = true;
                rec.store_time = now;
            }
            continue;
        }
        let npc = w
            .npc_by_guid(rec.last_npc)
            .filter(|&n| w.npc_class(n) == rec.class);
        let reset = match npc {
            None => Some(None),
            Some(n) if w.interaction_empty(n) => Some(Some(n)),
            Some(_) => None,
        };
        match reset {
            Some(assign) => {
                clear_record(rec, w);
                rec.store_generated = false;
                rec.store_time = 0;
                w.new_store_inventory(rec.class, assign);
            }
            None => rec.refresh_pending = true,
        }
    }
}

/// The act of a town level.
pub fn town_act(level: u16) -> Option<u8> {
    TOWNS.iter().position(|&l| l == level).map(|a| a as u8)
}

/// Leaving a level (`from`, `to`).
pub fn level_changed<W: VendorWorld>(
    records: &mut [VendorRecord],
    w: &mut W,
    player: UnitId,
    from: u16,
    to: u16,
    now: u32,
) {
    if let Some(act) = town_act(from) {
        let empty = w.players_in_level(from) == 0;
        refresh_act(records, w, act, empty, now);
    }
    if matches!(to, 1 | 40 | 75 | 109) {
        w.town_entered(player, to);
    }
}

/// A client leaving the game.
pub fn client_left<W: VendorWorld>(
    records: &mut [VendorRecord],
    w: &mut W,
    player: UnitId,
    now: u32,
) {
    if w.game_type() == 3 {
        return;
    }
    let level = w.player_level_id(player);
    if !matches!(level, 1 | 40 | 75 | 103) {
        return;
    }
    let Some(act) = town_act(level) else { return };
    let empty = w.players_in_level(level).wrapping_sub(1) == 0;
    refresh_act(records, w, act, empty, now);
}

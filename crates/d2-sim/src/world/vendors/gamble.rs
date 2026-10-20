// Spec: specs/world/vendors.md §5
//! Gamble lists: making a player's list (`0x00578790`), the upgrade
//! (`0x005786A0`), dropping a list (`0x00537190`) and C→S 0x37
//! (`0x0054BC30`).

use super::store::StoreCtx;
use super::trade::repair_item;
use super::{
    flag, stat, GambleList, VendorRecord, VendorTables, VendorWorld, AMU, GAMBLE_ITEMS, RIN,
};
use crate::items::q;
use crate::units::UnitId;

/// Gamble level draw (§5.1 step 1): one step, L_g = (lo' mod 10) − 5 +
/// L_p, clamped to 5..=99.
pub fn level_draw(seed: &mut crate::rng::Seed, player_level: i32) -> i32 {
    let l = (seed.step() % 10) as i32 - 5 + player_level;
    l.clamp(5, 99)
}

/// Upgrade `0x005786A0` (§5.1 step 5, expansion only).
fn upgrade(
    t: &VendorTables,
    seed: &mut crate::rng::Seed,
    id: usize,
    lg: i32,
    uber: i32,
    ultra: i32,
) -> usize {
    let Some(rec) = t.item(id) else { return id };
    let Some(u) = t.valid_code(rec.ubercode) else {
        return id;
    };
    let w_u = (lg - i32::from(t.items[u].level)).wrapping_mul(uber) + 1;
    if w_u <= 0 {
        return id;
    }
    if (seed.roll(10_000) as i32) < w_u {
        return u;
    }
    if let Some(x) = t.valid_code(rec.ultracode) {
        let w_x = (lg - i32::from(t.items[x].level)).wrapping_mul(ultra) + 1;
        if w_x > 0 && (seed.roll(10_000) as i32) < w_x {
            return x;
        }
    }
    id
}

/// Quality draw (§5.1 step 6): magic, or one step when H > 0.
pub fn quality_draw(seed: &mut crate::rng::Seed, rare: u32, set: u32, unique: u32) -> u8 {
    let h = rare.wrapping_add(set).wrapping_add(unique);
    if h == 0 {
        return q::MAGIC;
    }
    let r = seed.step() % 100_000;
    if r < unique {
        q::UNIQUE
    } else if r < unique.wrapping_add(set) {
        q::SET
    } else if r < h {
        q::RARE
    } else {
        q::MAGIC
    }
}

/// Makes the player's gamble list `0x00578790` (§5.1).
pub fn make_list<W: VendorWorld>(
    c: &mut StoreCtx,
    rec: &mut VendorRecord,
    w: &mut W,
    player: UnitId,
) {
    let t = c.tables;
    let Some(index) = t.gamble_index.as_ref() else {
        // The gamble index is never none in 1.14d (V9); a D2RUST table set
        // without one makes no list.
        return;
    };
    let Some(odds) = t.difficulty.get(usize::from(w.difficulty())).copied() else {
        return;
    };
    let pg = w.guid(player);
    rec.gamble_lists.insert(
        0,
        GambleList {
            player: pg,
            items: Vec::new(),
        },
    );
    let rin = t.find_code(RIN);
    let amu = t.find_code(AMU);
    let lp = w.stat(player, stat::LEVEL, 0);
    let expansion = w.expansion();
    let mut c_n = 0u32;
    loop {
        // Steps 1–2.
        let lg = level_draw(c.seed, lp);
        let th = t.gamble_thresholds.get(lg as usize).copied().unwrap_or(0) as i32;
        let idx = if th < 1 { 0 } else { c.seed.roll(th) as usize };
        let Some(&id) = index.get(idx) else {
            // idx < T[L_g] ≤ count always (V9): unreachable.
            break;
        };
        let mut id = id as usize;
        // Step 3.
        if !expansion {
            match t.item(id) {
                None => break,
                Some(r) if r.version >= 100 => continue,
                Some(_) => {}
            }
        }
        // Step 4.
        // A missing `rin` / `amu` is cached as item 0 (`hax`, V9).
        if c_n == 0 {
            id = rin.unwrap_or(0);
        } else if c_n == 1 {
            id = amu.unwrap_or(0);
        }
        // Step 5.
        if expansion {
            id = upgrade(t, c.seed, id, lg, odds.uber, odds.ultra);
        }
        // Step 6.
        let quality = quality_draw(c.seed, odds.rare, odds.set, odds.unique);
        // Step 7.
        c_n += 1;
        if let Some(item) = w.create_item(rec.class, id, quality, lg) {
            w.set_item_page(item, 0);
            repair_item(t, w, item, None);
            let f = w.item_flags(item);
            w.set_item_flags(item, f & !flag::IDENTIFIED);
            if !w.place_in_gamble(rec.class, pg, item) {
                w.destroy_item(item);
                break;
            }
            rec.gamble_lists[0].items.push(item);
        }
        // Step 8.
        if c_n >= GAMBLE_ITEMS {
            break;
        }
    }
}

/// Drops the player's list here `0x00537190` (§5.4): the node is
/// unlinked, its items removed and destroyed. Recorded:
/// `a2-npc-elzix-gamble` frame 21, the 14 list items are gone after the
/// 0x30.
pub fn drop_list<W: VendorWorld>(rec: &mut VendorRecord, w: &mut W, player: u32) {
    if let Some(i) = rec.gamble_lists.iter().position(|g| g.player == player) {
        let g = rec.gamble_lists.remove(i);
        for item in g.items {
            w.remove_gamble_item(rec.class, player, item);
            w.destroy_item(item);
        }
    }
}

/// C→S 0x37 IdentifyGamble `0x0054BC30` (§5.5). Returns the result.
pub fn identify_gamble<W: VendorWorld>(w: &mut W, player: UnitId, msg: &[u8]) -> u32 {
    if msg.len() != 5 {
        return 3;
    }
    let guid = u32::from_le_bytes([msg[1], msg[2], msg[3], msg[4]]);
    let Some(item) = w.item_by_guid(guid) else {
        return 2;
    };
    if w.last_bought(player) != guid {
        return 3;
    }
    if w.item_flags(item) & flag::IDENTIFIED == 0 {
        w.identify(item);
    }
    0
}

#[cfg(test)]
mod mutant_tests;

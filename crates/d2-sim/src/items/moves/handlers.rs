//! The item-move intents: C→S 0x16–0x29, 0x50, 0x61 (expansion only)
//! and 0x63, each in its fixed validation order with its result codes.
//! 0x4C belongs to the cube (`world::cube`). Layouts: `sim/client-messages.tsv`.

use super::deferred::{mark, owner_refresh, send_item_page, send_to_belt, NO_FILLERS};
use super::ground::{
    corpse_pickup_rest, drop_cursor_item, gold_limit, gold_piles, ground_place, pickup_auto,
    pickup_to_cursor,
};
use super::layouts;
use super::seams::MoveWorld;
use super::{
    add_cmd, add_iflags, changed_if_filled, clear_iflags, clear_uflags, cmd, exists, iflag, mode,
    page, res, sound, stat, ty, uflag, Guid, MoveFatal, Outcome, Owner, CUBE_CODE, DROP_MASK,
    DROP_MASK2, MAX_PILES, PICK_COLLISION_MASK, PICK_RANGE, PILE_CAP, USE_RANGE, WALK_RANGE,
};

/// The ids handled here with their exact handler size (`handler_size`
/// column of `client-messages.tsv`).
pub const HANDLED: [(u8, usize); 23] = [
    (0x16, 13),
    (0x17, 5),
    (0x18, 17),
    (0x19, 5),
    (0x1A, 9),
    (0x1B, 9),
    (0x1C, 3),
    (0x1D, 9),
    (0x1E, 9),
    (0x1F, 17),
    (0x20, 13),
    (0x21, 9),
    (0x22, 5),
    (0x23, 9),
    (0x24, 5),
    (0x25, 9),
    (0x26, 13),
    (0x27, 9),
    (0x28, 9),
    (0x29, 9),
    (0x50, 9),
    (0x61, 3),
    (0x63, 5),
];

fn u32_at(m: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([m[o], m[o + 1], m[o + 2], m[o + 3]])
}

fn u16_at(m: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([m[o], m[o + 1]])
}

/// Dispatches one C→S item message for `player`. `None` when the id is
/// not one of [`HANDLED`]. The size check (→ 3) comes first (text).
pub fn handle<W: MoveWorld>(w: &mut W, player: Guid, msg: &[u8]) -> Option<Result<u32, MoveFatal>> {
    let id = *msg.first()?;
    let &(_, size) = HANDLED.iter().find(|&&(i, _)| i == id)?;
    if msg.len() != size {
        return Some(Ok(res::REFUSED));
    }
    let p = Owner::player(player);
    let m = msg;
    Some(match id {
        0x16 => pick_item(w, p, u32_at(m, 1), u32_at(m, 5), u32_at(m, 9)),
        0x17 => drop_item(w, p, u32_at(m, 1)),
        0x18 => Ok(insert_item(
            w,
            p,
            u32_at(m, 1),
            u32_at(m, 5),
            u32_at(m, 9),
            u32_at(m, 13),
        )),
        0x19 => remove_from_buffer(w, p, u32_at(m, 1)),
        0x1A => Ok(equip_item(w, p, u32_at(m, 1), m[5])),
        0x1B => swap_2handed(w, p, u32_at(m, 1), m[5]),
        0x1C => remove_body_item(w, p, u16_at(m, 1)),
        0x1D => swap_cursor_with_body(w, p, u32_at(m, 1), m[5]),
        0x1E => swap_1h_with_2h(w, p, u32_at(m, 1), m[5]),
        0x1F => swap_cursor_buffer(
            w,
            p,
            u32_at(m, 1),
            u32_at(m, 5),
            u32_at(m, 9),
            u32_at(m, 13),
        ),
        0x20 => Ok(use_grid_item(
            w,
            p,
            u32_at(m, 1),
            u32_at(m, 5),
            u32_at(m, 9),
        )),
        0x21 => Ok(stack_items(w, p, u32_at(m, 1), u32_at(m, 5))),
        0x22 => Ok(unstack_items(w, p, u32_at(m, 1))),
        0x23 => Ok(item_to_belt(w, p, u32_at(m, 1), u32_at(m, 5))),
        0x24 => item_from_belt(w, p, u32_at(m, 1)),
        0x25 => switch_belt_item(w, p, u32_at(m, 1), u32_at(m, 5)),
        0x26 => Ok(use_belt_item(w, p, u32_at(m, 1), u32_at(m, 5))),
        0x27 => Ok(use_item_action(w, p, u32_at(m, 1), u32_at(m, 5))),
        0x28 => socket_item(w, p, u32_at(m, 1), u32_at(m, 5)),
        0x29 => scroll_to_book(w, p, u32_at(m, 1), u32_at(m, 5)),
        0x50 => Ok(drop_gold(w, p, u32_at(m, 1), u32_at(m, 5))),
        0x61 => merc_item(w, p, u16_at(m, 1)),
        0x63 => item_to_belt_shift(w, p, u32_at(m, 1)),
        _ => unreachable!("HANDLED lists only the ids above"),
    })
}

fn valid_loc(loc: u32) -> bool {
    (1..=10).contains(&loc)
}

/// Chebyshev range test around the player.
fn within<W: MoveWorld>(w: &W, player: Owner, x: i32, y: i32, r: i32) -> bool {
    let (px, py) = w.pos(player);
    (i64::from(x) - i64::from(px)).abs() <= i64::from(r)
        && (i64::from(y) - i64::from(py)).abs() <= i64::from(r)
}

// ------------------------------------------------------------------ 0x16

/// 0x16 PickItem.
pub fn pick_item<W: MoveWorld>(
    w: &mut W,
    p: Owner,
    unit_type: u32,
    guid: Guid,
    cursor: u32,
) -> Result<u32, MoveFatal> {
    if unit_type > 5 {
        return Ok(res::BAD);
    }
    if unit_type == 0 && guid == p.guid {
        return Ok(res::REFUSED);
    }
    match unit_type {
        0 => pick_player(w, p, guid, cursor),
        1 => Ok(w.pick_npc(p, guid, cursor)),
        2 => Ok(w.pick_object(p, guid, cursor)),
        3 => Ok(res::RANGE),
        5 => Ok(pick_tile(w, p, guid, cursor)),
        4 => {
            let it = Owner::item(guid);
            if !exists(w, guid) || w.mode(guid) != mode::GROUND || w.distance(p, it) > PICK_RANGE {
                return Ok(res::RANGE);
            }
            if w.distance(p, it) >= WALK_RANGE || w.collides(p, it, PICK_COLLISION_MASK) {
                w.walk_to_item(p, guid, cursor != 0);
                return Ok(res::OK);
            }
            let o = if cursor != 0 {
                pickup_to_cursor(w, p, guid)
            } else {
                pickup_auto(w, p, guid)?
            };
            Ok(o.result())
        }
        _ => unreachable!("types above 5 return 2 first"),
    }
}

/// 0x16 type 0: another player P.
fn pick_player<W: MoveWorld>(
    w: &mut W,
    p: Owner,
    guid: Guid,
    cursor: u32,
) -> Result<u32, MoveFatal> {
    let o = Owner::player(guid);
    if !w.unit_exists(o) || w.distance(p, o) > PICK_RANGE {
        return Ok(res::RANGE);
    }
    if w.distance(p, o) > PLAYER_WALK_RANGE {
        w.walk_to_unit(p, o, cursor != 0);
        return Ok(res::OK);
    }
    // Busy test with (1) = the trading test.
    if w.unit_mode(o) == MODE_DEAD && !w.trading(p) {
        // (the rest's), then steps 3–5.
        if w.corpse_pickup(p, o) {
            corpse_pickup_rest(w, p, o)?;
        }
    } else {
        w.player_interact(p, o);
    }
    Ok(res::OK)
}

/// 0x16 type 5: a tile.
fn pick_tile<W: MoveWorld>(w: &mut W, p: Owner, guid: Guid, cursor: u32) -> u32 {
    let o = Owner { ty: TILE, guid };
    if !w.unit_exists(o) || w.distance(p, o) > PICK_RANGE {
        return res::RANGE;
    }
    if w.distance(p, o) < WALK_RANGE {
        w.tile_warp(p, o);
    } else {
        w.walk_to_unit(p, o, cursor != 0);
    }
    res::OK
}

/// Unit type of a tile.
const TILE: u8 = 5;
/// Player mode "dead" (type 0).
const MODE_DEAD: u32 = 17;
/// Walk range of a player target (type 0: distance > 8 → walk).
const PLAYER_WALK_RANGE: i32 = 8;

// ------------------------------------------------------------------ 0x17

/// 0x17 DropItem.
pub fn drop_item<W: MoveWorld>(w: &mut W, p: Owner, item: Guid) -> Result<u32, MoveFatal> {
    let r = w.check_cursor_item(p, item);
    if r != 0 {
        return Ok(r);
    }
    if w.busy(p) && w.trading(p) {
        return Ok(res::REFUSED);
    }
    drop_cursor_item(w, p, item)?;
    Ok(res::OK)
}

// ------------------------------------------------------------------ 0x18

/// 0x18 InsertItemInBuffer.
pub fn insert_item<W: MoveWorld>(w: &mut W, p: Owner, item: Guid, x: u32, y: u32, pg: u32) -> u32 {
    let r = w.check_cursor_item(p, item);
    if r != 0 {
        return r;
    }
    if !w.unit_exists(p) || !p.is_player() {
        if pg > 4 {
            return res::BAD;
        }
    } else {
        // Step 3: "busy = 0 and page ≠ 0 → 2" is unreachable (the cursor
        // item makes the player busy); kept in its place.
        if !w.busy(p) && pg != 0 {
            return res::BAD;
        }
        match pg {
            4 if !w.in_town(p) => return res::REFUSED,
            2 if !w.trading(p) => return res::REFUSED,
            1 => return res::BAD,
            n if n >= 5 => return res::BAD,
            _ => {}
        }
    }
    if !exists(w, item) {
        return res::BAD;
    }
    w.set_page(item, pg as u8);
    // (owner present; targeting reset), then steps 2–9.
    w.targeting_reset(p);
    if w.place_in_page(p, item, x as i32, y as i32, false, true) {
        res::OK
    } else {
        res::REFUSED
    }
}

// ------------------------------------------------------------------ 0x19

/// 0x19 RemoveItemFromBuffer.
pub fn remove_from_buffer<W: MoveWorld>(w: &mut W, p: Owner, item: Guid) -> Result<u32, MoveFatal> {
    let r = w.check_stored(p, item);
    if r != 0 {
        return Ok(r);
    }
    if w.cursor(p).is_some() {
        w.send(p, layouts::cant_do_that());
        return Ok(res::BAD);
    }
    if !exists(w, item) {
        return Ok(res::BAD);
    }
    if w.page(item) == page::TRADE1 {
        return Ok(res::REFUSED);
    }
    if !w.item_move_gate(p, Some(item)) {
        return Ok(res::OK);
    }
    Ok(to_cursor(w, p, item)?.result())
}

/// To cursor.
pub fn to_cursor<W: MoveWorld>(w: &mut W, p: Owner, item: Guid) -> Result<Outcome, MoveFatal> {
    if w.cursor(p).is_some() {
        return Ok(Outcome::NOTHING);
    }
    if !exists(w, item) {
        return Ok(Outcome::REFUSED);
    }
    let pg = w.page(item);
    // Only an idle player is held to page 0; a busy one (open stash or
    // cube) passes on any page.
    if p.is_player() && !w.busy(p) && pg != page::INVENTORY {
        return Ok(Outcome::REFUSED);
    }
    if w.mode(item) != mode::STORED {
        return Ok(Outcome::REFUSED);
    }
    w.targeting_reset(p);
    clear_uflags(w, item, uflag::TARGETABLE);
    let (x, y) = w.pos(Owner::item(item));
    if !w.unlink(p, item) {
        return Err(MoveFatal::Unlink);
    }
    w.stat_refresh_unlink(p, 1);
    if w.is_active(p, item) {
        w.inventory_pass(p);
    }
    w.charm_unlink(p, item);
    w.room_change_notice(item, x, y);
    w.set_stored_page(item, pg);
    w.set_page(item, page::NONE);
    if !(p.is_player() && pg == page::TRADE1) {
        w.set_cursor(p, Some(item));
    }
    w.set_mode(item, mode::CURSOR);
    add_cmd(w, item, cmd::FROM_PAGE);
    changed_if_filled(w, item);
    clear_iflags(w, item, iflag::NOEQUIP);
    w.update_list_add(p, item);
    owner_refresh(w, p);
    Ok(Outcome::DONE)
}

/// The save load's cursor placement (mode 4:
/// the owner's cursor := the item, then: mode 4,
/// command flag 0x100000 = the 0x9C action 0x12, update list, owner
/// refresh). No cell is looked up, nothing is unlinked (the item was never
/// in a list) and the saved page and cell stay on the unit, so the next
/// save writes them back (stored page 4 stays 4).
pub fn load_to_cursor<W: MoveWorld>(w: &mut W, p: Owner, item: Guid) {
    w.set_cursor(p, Some(item));
    w.set_mode(item, mode::CURSOR);
    add_cmd(w, item, cmd::TO_CURSOR);
    w.update_list_add(p, item);
    owner_refresh(w, p);
}

// ------------------------------------------------------------------ 0x1A

/// 0x1A EquipItem.
pub fn equip_item<W: MoveWorld>(w: &mut W, p: Owner, item: Guid, loc: u8) -> u32 {
    let r = w.check_cursor_item(p, item);
    if r != 0 {
        return r;
    }
    if !valid_loc(u32::from(loc)) {
        return res::BAD;
    }
    let saved = w.equipment_checkpoint(p);
    let (ok, out) = w.equip_from_cursor(p, item, loc, false);
    if !ok {
        w.restore_equipment(p, saved);
    }
    Outcome { ok, out }.result()
}

// ------------------------------------------------------------------ 0x1B

fn other_hand(loc: u8) -> u8 {
    if loc == 4 {
        5
    } else {
        4
    }
}

/// Unlink a worn item, detach its active effects and clear its body slot.
/// Removing a belt also evacuates slots beyond the default capacity.
pub(super) fn remove_from_body<W: MoveWorld>(
    w: &mut W,
    owner: Owner,
    item: Guid,
) -> Result<(), MoveFatal> {
    let saved = w.equipment_checkpoint(owner);
    let result = remove_from_body_attempt(w, owner, item);
    if result.is_err() {
        w.restore_equipment(owner, saved);
    }
    result
}

fn remove_from_body_attempt<W: MoveWorld>(
    w: &mut W,
    owner: Owner,
    item: Guid,
) -> Result<(), MoveFatal> {
    let loc = w.body_loc(item);
    if !w.unlink(owner, item) {
        return Err(MoveFatal::Unlink);
    }
    w.body_leave_effects(owner, item);
    w.clear_body_slot(owner, loc);
    if w.primary_type(item) == ty::BELT {
        belt_change(w, owner, None)?;
    }
    Ok(())
}

/// Move contents beyond the new belt's capacity to inventory or the ground.
/// Each moved item gets a direct 0x9C action 0xF removal packet. If neither
/// destination is available, abort the belt change and restore its contents.
pub fn belt_change<W: MoveWorld>(w: &mut W, u: Owner, new: Option<Guid>) -> Result<(), MoveFatal> {
    let n = w.belt_boxes(new);
    // Prepare every fallible packet before changing inventory or rooms.
    let mut moves = Vec::new();
    for s in n..BELT_SLOTS {
        if let Some(item) = w.belt_item(u, s) {
            let removal = layouts::item_world(
                0x0F,
                super::deferred::category(w, item),
                item,
                &w.item_bits(item, NO_FILLERS, w.page(item)),
            )?;
            moves.push((item, removal));
        }
    }
    let saved = w.equipment_checkpoint(u);
    let cursor = w.cursor(u);
    for &(item, _) in &moves {
        if !w.unlink(u, item) {
            w.restore_equipment(u, saved);
            return Err(MoveFatal::Unlink);
        }
        w.set_mode(item, mode::CURSOR);
        w.charm_unlink(u, item);
        w.set_page(item, page::INVENTORY);
    }
    let mut drops = Vec::new();
    for &(item, _) in &moves {
        let placed = w.place_in_page(u, item, 0, 0, true, true);
        // Page placement normally clears a cursor. Belt evacuation must keep
        // the displaced equipment item held by the enclosing swap.
        if cursor.is_some() {
            w.set_cursor(u, cursor);
        }
        if placed {
            continue;
        }
        // A provider may fail its link check after placing the item.
        if w.items(u).contains(&item) && !w.unlink(u, item) {
            w.restore_equipment(u, saved);
            return Err(MoveFatal::Unlink);
        }
        let (x, y) = w.pos(u);
        let Some(spot) = w.free_spot((x, y), (x, y), 1, DROP_MASK, DROP_MASK2, 1) else {
            w.restore_equipment(u, saved);
            return Err(MoveFatal::Link);
        };
        drops.push((item, spot));
    }
    // No fallible work remains: publish the moves and their removal packets.
    for (item, spot) in drops {
        w.stat_refresh_unlink(u, 1);
        ground_place(w, item, spot);
    }
    for (_, removal) in moves {
        w.send(u, removal);
    }
    Ok(())
}

/// Belt slots (grid 1).
const BELT_SLOTS: u8 = 16;

/// 0x1B Swap2HandedItem.
pub fn swap_2handed<W: MoveWorld>(w: &mut W, p: Owner, n: Guid, loc: u8) -> Result<u32, MoveFatal> {
    let saved = w.equipment_checkpoint(p);
    let result = swap_2handed_attempt(w, p, n, loc);
    if !(matches!(result, Ok(res::OK)) && w.mode(n) == mode::EQUIPPED) {
        w.restore_equipment(p, saved);
    }
    result
}

fn swap_2handed_attempt<W: MoveWorld>(
    w: &mut W,
    p: Owner,
    n: Guid,
    loc: u8,
) -> Result<u32, MoveFatal> {
    let r = w.check_cursor_item(p, n);
    if r != 0 {
        return Ok(r);
    }
    if !valid_loc(u32::from(loc)) {
        return Ok(res::BAD);
    }
    if loc != 4 && loc != 5 {
        return Ok(res::REFUSED);
    }
    let Some(x) = w.body_item(p, other_hand(loc)) else {
        return Ok(res::REFUSED);
    };
    if w.equip_check(p, loc, Some(n), false) != 2 {
        return Ok(res::REFUSED);
    }
    if !w.requirements(n, p, false) {
        w.stat_refresh(p);
        w.requirement_sound(p);
        return Ok(res::OK);
    }
    remove_from_body(w, p, x)?;
    // X becomes the cursor item: no command flag, no update-list entry.
    // The client learns the move from N's 0x9D action 7 (indirect swap).
    w.set_cursor(p, Some(x));
    w.set_mode(x, mode::CURSOR);
    clear_uflags(w, x, uflag::TARGETABLE);
    // Any failed placement or link restores both items and the cursor.
    if !w.place_body(p, n, loc) || !w.link_check(p, n, 3) {
        return Ok(res::REFUSED);
    }
    w.set_body_loc(n, loc);
    w.stat_link(p, n);
    w.stat_refresh(p);
    clear_uflags(w, n, uflag::TARGETABLE);
    w.set_mode(n, mode::EQUIPPED);
    w.set_page(n, page::NONE);
    add_cmd(w, n, cmd::INDIRECT_SWAP);
    add_iflags(w, n, iflag::CHANGED);
    clear_iflags(w, n, iflag::NOEQUIP);
    w.update_list_add(p, n);
    w.weapon_bookkeeping(p);
    w.inventory_pass(p);
    owner_refresh(w, p);
    Ok(res::OK)
}

// ------------------------------------------------------------------ 0x1C

/// 0x1C RemoveBodyItem. The empty
/// location is tested before, so result 4 (the two-handed item in
/// the other hand) never occurs here.
pub fn remove_body_item<W: MoveWorld>(w: &mut W, p: Owner, loc: u16) -> Result<u32, MoveFatal> {
    if !valid_loc(u32::from(loc)) {
        return Ok(res::BAD);
    }
    let loc = loc as u8;
    let at = w.body_item(p, loc);
    if !w.item_move_gate(p, at) {
        return Ok(res::OK);
    }
    if loc == 8 && !w.belt_remove_allowed(p) {
        return Ok(res::OK);
    }
    if w.cursor(p).is_some() || at.is_none() {
        return Ok(res::OK);
    }
    let r = w.equip_check(p, loc, None, false);
    if r != 3 && r != 4 {
        return Ok(res::REFUSED);
    }
    // Without an item → out 1 (MV4).
    let Some(it) = w.item_to_remove(p, loc) else {
        return Ok(res::REFUSED);
    };
    remove_from_body(w, p, it)?;
    w.set_cursor(p, Some(it));
    w.stat_refresh(p);
    clear_uflags(w, it, uflag::TARGETABLE);
    w.set_mode(it, mode::CURSOR);
    add_cmd(w, it, cmd::UNEQUIP);
    changed_if_filled(w, it);
    clear_iflags(w, it, iflag::NOEQUIP);
    w.update_list_add(p, it);
    owner_refresh(w, p);
    w.weapon_bookkeeping(p);
    w.inventory_pass(p);
    Ok(res::OK)
}

/// Take-off to the ground side (mode 1): [`remove_body_item`]'s take-off with
/// the fourth argument set: the item is not made the cursor item but gets
/// item flag 0x20; mode 4, command flag 0x10, update entry, stats refresh,
/// weapon bookkeeping and inventory pass. End state: detached, not freed.
/// Nothing changes unless the cursor is empty and the body slot check
/// gives 3 or 4. Returns whether the item was taken off.
pub fn unequip_detached<W: MoveWorld>(w: &mut W, p: Owner, loc: u8) -> Result<bool, MoveFatal> {
    if !valid_loc(u32::from(loc)) || w.cursor(p).is_some() {
        return Ok(false);
    }
    let r = w.equip_check(p, loc, None, false);
    if r != 3 && r != 4 {
        return Ok(false);
    }
    let Some(it) = w.item_to_remove(p, loc) else {
        return Ok(false);
    };
    remove_from_body(w, p, it)?;
    w.stat_refresh(p);
    add_iflags(w, it, iflag::COPIED);
    clear_uflags(w, it, uflag::TARGETABLE);
    w.set_mode(it, mode::CURSOR);
    add_cmd(w, it, cmd::UNEQUIP);
    clear_iflags(w, it, iflag::NOEQUIP);
    w.update_list_add(p, it);
    owner_refresh(w, p);
    w.weapon_bookkeeping(p);
    w.inventory_pass(p);
    Ok(true)
}

// ------------------------------------------------------------------ 0x1D

/// Remove an exhausted equipped stack without changing an unrelated cursor.
/// The normal equipment update announces the removal before freeing the item.
pub fn exhaust_stack<W: MoveWorld>(w: &mut W, owner: Owner, item: Guid) -> Result<(), MoveFatal> {
    if !exists(w, item)
        || w.mode(item) != mode::EQUIPPED
        || w.stat(Owner::item(item), stat::QUANTITY) != 0
        || w.body_item(owner, w.body_loc(item)) != Some(item)
    {
        return Ok(());
    }
    remove_from_body(w, owner, item)?;
    w.set_mode(item, mode::CURSOR);
    add_iflags(w, item, iflag::COPIED);
    add_cmd(w, item, cmd::UNEQUIP | super::deferred::CMD_REMOVE);
    add_iflags(w, item, iflag::CHANGED);
    w.update_list_add(owner, item);
    owner_refresh(w, owner);
    w.weapon_bookkeeping(owner);
    w.inventory_pass(owner);
    Ok(())
}

/// 0x1D SwapCursorWithBody.
pub fn swap_cursor_with_body<W: MoveWorld>(
    w: &mut W,
    p: Owner,
    n: Guid,
    loc: u8,
) -> Result<u32, MoveFatal> {
    let saved = w.equipment_checkpoint(p);
    let result = swap_cursor_with_body_attempt(w, p, n, loc);
    if !(matches!(result, Ok(res::OK)) && w.mode(n) == mode::EQUIPPED) {
        w.restore_equipment(p, saved);
    }
    result
}

fn swap_cursor_with_body_attempt<W: MoveWorld>(
    w: &mut W,
    p: Owner,
    n: Guid,
    loc: u8,
) -> Result<u32, MoveFatal> {
    let r = w.check_cursor_item(p, n);
    if r != 0 {
        return Ok(r);
    }
    if !valid_loc(u32::from(loc)) {
        return Ok(res::BAD);
    }
    let Some(at) = w.body_item(p, loc) else {
        return Ok(res::RANGE);
    };
    if loc == 8 && !w.belt_remove_allowed(p) {
        return Ok(res::OK);
    }
    if !w.item_move_gate(p, Some(at)) {
        return Ok(res::OK);
    }
    if w.equip_check(p, loc, Some(n), false) != 5 {
        return Ok(res::OK);
    }
    w.weapon_in_use_update(p);
    // E missing or not in mode 1 → out 1 (MV4).
    let Some(e) = w
        .item_to_remove(p, loc)
        .filter(|&e| w.mode(e) == mode::EQUIPPED)
    else {
        return Ok(res::REFUSED);
    };
    w.stat_refresh(p);
    if !w.requirements(n, p, false) {
        w.stat_refresh(p);
        w.requirement_sound(p);
        return Ok(res::OK);
    }
    // E leaves the body (as in [`remove_body_item`], without its own belt step).
    let eloc = w.body_loc(e);
    // A failed unlink must not detach the equipped item's stats.
    if !w.unlink(p, e) {
        return Err(MoveFatal::Unlink);
    }
    w.body_leave_effects(p, e);
    w.clear_body_slot(p, eloc);
    w.set_cursor(p, Some(e));
    w.set_mode(e, mode::CURSOR);
    add_iflags(w, e, iflag::SWAP_OUT);
    add_cmd(w, e, cmd::SWAP_BODY);
    add_iflags(w, e, iflag::CHANGED);
    // N goes to the location; a failed put or link → out 1 (MV4).
    if !w.place_body(p, n, loc) || !w.link_check(p, n, 3) {
        return Ok(res::REFUSED);
    }
    if w.primary_type(n) == ty::BELT {
        belt_change(w, p, Some(n))?;
        // The item may already be queued by an earlier move in this tick.
        // Requeue it after all evacuated contents even in that case.
        let updates = w.update_list(p);
        w.update_list_free(p);
        for item in updates.into_iter().filter(|&item| item != e) {
            w.update_list_add(p, item);
        }
    }
    // Evacuated belt contents send container updates that clear a cursor.
    // Publish the displaced equipment after those updates set their locations.
    w.update_list_add(p, e);
    w.set_body_loc(n, loc);
    w.stat_link(p, n);
    w.set_mode(n, mode::EQUIPPED);
    w.set_page(n, page::NONE);
    add_iflags(w, n, iflag::SWAP_IN | iflag::CHANGED);
    add_cmd(w, n, cmd::SWAP_BODY);
    w.update_list_add(p, n);
    owner_refresh(w, p);
    w.weapon_bookkeeping(p);
    w.inventory_pass(p);
    Ok(res::OK)
}

// ------------------------------------------------------------------ 0x1E

/// 0x1E Swap1HWith2H.
pub fn swap_1h_with_2h<W: MoveWorld>(
    w: &mut W,
    p: Owner,
    n: Guid,
    loc: u8,
) -> Result<u32, MoveFatal> {
    let r = w.check_cursor_item(p, n);
    if r != 0 {
        return Ok(r);
    }
    if !valid_loc(u32::from(loc)) {
        return Ok(res::BAD);
    }
    if loc != 4 && loc != 5 {
        return Ok(res::REFUSED);
    }
    let Some(at) = w.body_item(p, loc) else {
        return Ok(res::RANGE);
    };
    if !w.item_move_gate(p, Some(at)) {
        return Ok(res::OK);
    }
    Ok(swap_1h_2h_body(w, p, n, loc)?.result())
}

/// Move the cursor item to a hand, the displaced item to the cursor and
/// the other hand's item to the inventory. Restore all three on failure.
pub fn swap_1h_2h_body<W: MoveWorld>(
    w: &mut W,
    p: Owner,
    n: Guid,
    loc: u8,
) -> Result<Outcome, MoveFatal> {
    let saved = w.equipment_checkpoint(p);
    let result = swap_1h_2h_body_attempt(w, p, n, loc);
    if !matches!(result, Ok(Outcome::DONE)) {
        w.restore_equipment(p, saved);
    }
    result
}

fn swap_1h_2h_body_attempt<W: MoveWorld>(
    w: &mut W,
    p: Owner,
    n: Guid,
    loc: u8,
) -> Result<Outcome, MoveFatal> {
    // Step 1.
    if !exists(w, n) || w.mode(n) != mode::CURSOR {
        return Ok(Outcome::NOTHING);
    }
    // Step 2.
    if w.equip_check(p, loc, Some(n), false) != 7 {
        return Ok(Outcome::NOTHING);
    }
    if !w.requirements(n, p, false) {
        return Ok(Outcome::REFUSED);
    }
    // Step 3: = a free position of page 0 for X.
    // X missing is fatal (unreachable after = 7, which needs X).
    let o = other_hand(loc);
    let Some(x) = w.body_item(p, o) else {
        return Err(MoveFatal::Missing);
    };
    if w.find_free(p, x, page::INVENTORY).is_none() {
        return Ok(Outcome::REFUSED);
    }
    // Step 4.
    if w.mode(x) == mode::EQUIPPED {
        leave_body_quiet(w, p, x)?;
        let placed = match w.find_free(p, x, page::INVENTORY) {
            Some((fx, fy)) => w.place_at(p, x, page::INVENTORY, fx, fy),
            None => false,
        };
        if !placed {
            return Ok(Outcome::REFUSED);
        }
        if !w.link_check(p, x, 1) {
            return Ok(Outcome::REFUSED);
        }
        w.set_page(x, page::INVENTORY);
        clear_uflags(w, x, uflag::TARGETABLE);
        w.set_mode(x, mode::STORED);
        changed_if_filled(w, x);
        clear_iflags(w, x, iflag::NOEQUIP);
        add_cmd(w, x, cmd::AUTO_UNEQUIP);
        add_iflags(w, x, iflag::CHANGED);
        w.update_list_add(p, x);
    }
    // Step 5.
    let Some(t) = w.body_item(p, loc) else {
        return Err(MoveFatal::Missing);
    };
    if w.mode(t) != mode::EQUIPPED {
        return Ok(Outcome::REFUSED);
    }
    w.stat_refresh_unlink(p, 1);
    if !w.requirements(n, p, false) {
        w.stat_refresh(p);
        w.set_cursor(p, Some(n));
        w.requirement_sound(p);
        owner_refresh(w, p);
        return Ok(Outcome::NOTHING);
    }
    // Step 6.
    leave_body_quiet(w, p, t)?;
    w.set_cursor(p, Some(t));
    clear_uflags(w, t, uflag::TARGETABLE);
    w.set_mode(t, mode::CURSOR);
    add_cmd(w, t, cmd::UNEQUIP);
    add_iflags(w, t, iflag::CHANGED);
    clear_iflags(w, t, iflag::NOEQUIP);
    w.update_list_add(p, t);
    // Step 7.
    if !w.place_body(p, n, loc) || !w.link_check(p, n, 3) {
        return Ok(Outcome::REFUSED);
    }
    w.set_body_loc(n, loc);
    w.stat_link(p, n);
    w.stat_refresh(p);
    clear_uflags(w, n, uflag::TARGETABLE);
    w.set_mode(n, mode::EQUIPPED);
    w.set_page(n, page::NONE);
    add_iflags(w, n, iflag::STACK_FULL);
    add_cmd(w, n, cmd::EQUIP);
    add_iflags(w, n, iflag::CHANGED);
    clear_iflags(w, n, iflag::NOEQUIP);
    w.update_list_add(p, n);
    w.weapon_bookkeeping(p);
    w.inventory_pass(p);
    // The inventory pass ends with the owner refresh; the
    // pass is a seam, so the refresh is run here.
    owner_refresh(w, p);
    Ok(Outcome::DONE)
}

/// Remove a swap participant from its body slot and detach its active effects.
fn leave_body_quiet<W: MoveWorld>(w: &mut W, p: Owner, x: Guid) -> Result<(), MoveFatal> {
    let loc = w.body_loc(x);
    if !w.unlink(p, x) {
        return Err(MoveFatal::Unlink);
    }
    w.body_leave_effects(p, x);
    w.clear_body_slot(p, loc);
    w.stat_refresh_unlink(p, 1);
    Ok(())
}

// ------------------------------------------------------------------ 0x1F

/// 0x1F SwapCursorBufferItem.
pub fn swap_cursor_buffer<W: MoveWorld>(
    w: &mut W,
    p: Owner,
    c: Guid,
    t: Guid,
    x: u32,
    y: u32,
) -> Result<u32, MoveFatal> {
    let r = w.check_cursor_item(p, c);
    if r != 0 {
        return Ok(r);
    }
    let r = w.check_stored(p, t);
    if r != 0 {
        return Ok(r);
    }
    if !exists(w, t) {
        return Ok(res::BAD);
    }
    let pg = w.page(t);
    if pg == page::TRADE1 || pg == page::TRADE2 {
        return Ok(res::REFUSED);
    }
    if !w.item_move_gate(p, Some(t)) {
        return Ok(res::OK);
    }
    if pg == page::CUBE && w.code(c) == CUBE_CODE {
        return Ok(res::OK);
    }
    if pg == page::TRADE2 {
        return Ok(res::REFUSED);
    }
    // A target not in mode 0 → nothing, out 0 (MV4).
    if w.mode(t) != mode::STORED {
        return Ok(res::OK);
    }
    let t_active = w.is_active(p, t);
    let (tx, ty_) = w.pos(Owner::item(t));
    if !w.unlink(p, t) {
        return Err(MoveFatal::Unlink);
    }
    w.stat_refresh(p);
    w.room_change_notice(t, tx, ty_);
    w.set_page(t, page::NONE);
    w.set_cursor(p, Some(t));
    w.charm_unlink(p, t);
    w.set_stored_page(t, pg);
    clear_uflags(w, t, uflag::TARGETABLE);
    w.set_mode(t, mode::CURSOR);
    add_cmd(w, t, cmd::SWAP_IN_PAGE);
    changed_if_filled(w, t);
    w.update_list_add(p, t);
    // Step 5: item flag 0x4000 is cleared on T before C's placement.
    clear_iflags(w, t, iflag::NOEQUIP);
    // The cursor item C.
    if !w.place_at(p, c, pg, x as i32, y as i32) {
        return Ok(res::REFUSED);
    }
    w.set_stored_page(c, 0);
    w.set_page(c, pg);
    w.set_pos(Owner::item(c), x as i32, y as i32);
    // C's link failing → out 1 (MV4).
    if !w.link_check(p, c, 1) {
        return Ok(res::REFUSED);
    }
    w.charm_relink(p, c);
    if w.is_active(p, c) {
        w.stat_refresh(p);
    }
    // Step 5: C's item flag 0x1 test (socket-filled), then its 0x4000
    // clear.
    changed_if_filled(w, c);
    clear_iflags(w, c, iflag::NOEQUIP);
    w.set_mode(c, mode::STORED);
    add_cmd(w, c, cmd::SWAP_IN_PAGE);
    w.update_list_add(p, c);
    owner_refresh(w, p);
    if t_active {
        w.inventory_pass(p);
    }
    Ok(res::OK)
}

// ------------------------------------------------------------------ 0x20

/// 0x20 UseGridItem.
pub fn use_grid_item<W: MoveWorld>(w: &mut W, p: Owner, item: Guid, x: u32, y: u32) -> u32 {
    let r = w.check_stored(p, item);
    if r != 0 {
        return r;
    }
    if !within(w, p, x as i32, y as i32, USE_RANGE) {
        return res::RANGE;
    }
    use_grid_body(w, p, item, x as i32, y as i32).result()
}

/// The grid use of item I at (x, y); the use
/// effects behind it belong to item use.
pub fn use_grid_body<W: MoveWorld>(w: &mut W, p: Owner, i: Guid, x: i32, y: i32) -> Outcome {
    // Step 1.
    w.targeting_reset(p);
    if !exists(w, i) {
        return Outcome::REFUSED;
    }
    if w.cursor(p).is_some() {
        return Outcome::NOTHING;
    }
    if w.mode(i) != mode::STORED || !w.useable(i) {
        return Outcome::REFUSED;
    }
    // Step 2.
    let it = Owner::item(i);
    let book = w.primary_type(i) == ty::BOOK;
    if book && w.stat(it, stat::QUANTITY) < 1 {
        return Outcome::NOTHING;
    }
    if w.trading(p) {
        return Outcome::NOTHING;
    }
    // The cube is opened, not used up (use entry 7, returns 0; in the
    // base game `misc.txt`
    // `pSpell` 7 is `box ` only, so the code test picks the same item).
    // Step 4 has no `box ` branch: the handler ends with 0.
    if &w.code(i) == b"box " {
        // The use function returns 0, which ends step 4 with 0.
        w.open_cube(p, i);
        return Outcome::NOTHING;
    }
    // Step 3.
    if w.use_item_at(p, i, x, y) {
        let s = w.item_skill(i);
        if book {
            let q = w.stat(it, stat::QUANTITY);
            if s != -1 && q > 0 {
                w.set_stat(it, stat::QUANTITY, q - 1);
                w.send_item_stat(p, i, stat::QUANTITY);
                w.send(p, layouts::item_used(Owner::ITEM, i));
                w.skill_decrement(p, s);
            }
            return Outcome::DONE;
        }
        if s != -1 && w.has_skill(p, s) {
            w.skill_decrement(p, s);
        }
        w.targeting_reset(p);
        w.consume_item(p, i);
        return Outcome::DONE;
    }
    // Step 4: quest items, with the player's quest record.
    let quest = |w: &mut W, q: u8, f: u8| w.quest_flag(p, q, f);
    let used = match &w.code(i) {
        b"ass " => {
            w.targeting_reset(p);
            if quest(w, 9, 5) {
                w.set_quest_flag(p, 9, 5, false);
                let v = w.stat(p, STAT_NEWSKILLS);
                w.set_stat(p, STAT_NEWSKILLS, v.wrapping_add(1));
                true
            } else {
                false
            }
        }
        b"xyz " => {
            w.targeting_reset(p);
            if quest(w, 20, 5) {
                w.set_quest_flag(p, 20, 5, false);
                let v = w.stat(p, STAT_MAXHP);
                w.set_stat(p, STAT_MAXHP, v.wrapping_add(XYZ_LIFE));
                true
            } else {
                false
            }
        }
        b"tr2 " => {
            w.targeting_reset(p);
            if quest(w, 37, 8) && !quest(w, 37, 7) {
                w.set_quest_flag(p, 37, 7, true);
                w.quest_tr2_used(p);
                true
            } else {
                false
            }
        }
        b"toa " => {
            w.targeting_reset(p);
            w.reset_skills_stats(p);
            w.consume_item(p, i);
            //–: event 2 on the player.
            w.sound(p, sound::TOA_USED);
            return Outcome::DONE;
        }
        _ => return Outcome::NOTHING,
    };
    if used {
        // The chain is fixed per code.
        let chain = match &w.code(i) {
            b"ass " => 8,
            b"xyz " => 18,
            _ => 33,
        };
        w.quest_item_used(p, chain);
        w.consume_item(p, i);
    } else {
        // A failed flag test: event 0x13 on the player.
        w.sound(p, sound::REFUSED_PICKUP);
    }
    Outcome::DONE
}

/// The key item's code.
pub const KEY_CODE: [u8; 4] = *b"key ";

/// The player's first key stack in the inventory page, in grid list
/// order (a key in the stash or the cube does not open anything).
pub fn first_key<W: MoveWorld>(w: &W, p: Owner) -> Option<Guid> {
    w.page_items(p, page::INVENTORY).into_iter().find(|&i| {
        exists(w, i) && w.code(i) == KEY_CODE && w.stat(Owner::item(i), stat::QUANTITY) > 0
    })
}

/// A locked chest or door took a key: the first key stack in the player's
/// inventory loses one, and the last key of a stack is used up. `false`:
/// the player has no key.
pub fn use_key<W: MoveWorld>(w: &mut W, p: Owner) -> bool {
    let Some(k) = first_key(w, p) else {
        return false;
    };
    if !w.consume_one(k) {
        w.consume_item(p, k);
    }
    true
}

/// Stat 5 `newskills`, stat 7 `maxhp`.
const STAT_NEWSKILLS: u16 = 5;
const STAT_MAXHP: u16 = 7;
/// `xyz`: 20 life in 8.8 fixed point.
const XYZ_LIFE: i32 = 0x1400;

// ------------------------------------------------------------------ 0x21

/// 0x21 StackItems.
pub fn stack_items<W: MoveWorld>(w: &mut W, p: Owner, src: Guid, dst: Guid) -> u32 {
    for g in [src, dst] {
        let r = w.check_owned(p, g);
        if r != 0 {
            return r;
        }
    }
    if src == dst {
        return res::REFUSED;
    }
    if !exists(w, src) || !exists(w, dst) || w.page(dst) == page::TRADE2 {
        return res::REFUSED;
    }
    if !w.stack_test(src, dst) {
        return res::OK;
    }
    let (s, d) = (Owner::item(src), Owner::item(dst));
    let qs = w.stat(s, stat::QUANTITY);
    let qd = w.stat(d, stat::QUANTITY);
    let m = w.max_stack(dst);
    // Primary type 18 (MV5).
    let books = w.primary_type(src) == ty::BOOK && w.primary_type(dst) == ty::BOOK;
    if i64::from(qs) + i64::from(qd) > i64::from(m) {
        w.set_stat(d, stat::QUANTITY, m);
        w.send_item_stat(p, dst, stat::QUANTITY);
        w.set_stat(s, stat::QUANTITY, qs.wrapping_add(qd).wrapping_sub(m));
        w.send_item_stat(p, src, stat::QUANTITY);
        if books {
            w.book_count_changed(p, dst, m.wrapping_sub(qd));
        }
        add_iflags(w, dst, iflag::STACK_FULL);
    } else {
        // ("has durability") gates only the
        // stat-72 step of the auto-stack: keys (3 + 4) and
        // arrow quivers (30 + 40) merge in the base game (measured).
        if w.merge_allowed(src) {
            let ds = w.stat(s, stat::DURABILITY);
            if ds < w.stat(d, stat::DURABILITY) {
                // Stat 72 `durability`: throwing weapons keep the worse one.
                w.set_stat(d, stat::DURABILITY, ds);
                w.send_item_stat(p, dst, stat::DURABILITY);
            }
        }
        w.set_stat(d, stat::QUANTITY, qs.wrapping_add(qd));
        w.send_item_stat(p, dst, stat::QUANTITY);
        if books {
            w.book_count_changed(p, dst, qs);
        }
        w.set_cursor(p, None);
        // S→C 0x42 names the player whose cursor clears (measured:
        // `42 00 <player GUID>`;
        // acts only on the local
        // player).
        w.send(p, layouts::clear_cursor(p.ty, p.guid));
        w.free_item(src);
    }
    mark(w, p, dst, cmd::ADD_QUANTITY);
    res::OK
}

// ------------------------------------------------------------------ 0x22

/// 0x22 UnstackItems: 3 for every owned item
/// (returns 0 and leaves the size in the refusal flag).
pub fn unstack_items<W: MoveWorld>(w: &mut W, p: Owner, item: Guid) -> u32 {
    let r = w.check_owned(p, item);
    if r != 0 {
        return r;
    }
    res::REFUSED
}

// ------------------------------------------------------------------ 0x23

/// 0x23 ItemToBelt.
pub fn item_to_belt<W: MoveWorld>(w: &mut W, p: Owner, item: Guid, slot: u32) -> u32 {
    let r = w.check_cursor_item(p, item);
    if r != 0 {
        return r;
    }
    to_belt(w, p, item, slot).result()
}

pub fn to_belt<W: MoveWorld>(w: &mut W, p: Owner, item: Guid, slot: u32) -> Outcome {
    w.targeting_reset(p);
    if !exists(w, item) || w.mode(item) != mode::CURSOR {
        return Outcome::REFUSED;
    }
    if !w.beltable(item) {
        return Outcome::NOTHING;
    }
    if !w.belt_place(p, item, slot) {
        w.set_page(item, page::NONE);
        return Outcome::NOTHING;
    }
    if !w.link_check(p, item, 2) {
        return Outcome::REFUSED;
    }
    w.set_cursor(p, None);
    w.charm_relink(p, item);
    clear_uflags(w, item, uflag::TARGETABLE);
    if w.is_active(p, item) {
        w.stat_refresh(p);
    }
    w.set_mode(item, mode::BELT);
    w.set_page(item, page::NONE);
    add_cmd(w, item, cmd::TO_BELT);
    owner_refresh(w, p);
    w.update_list_add(p, item);
    Outcome::DONE
}

// ------------------------------------------------------------------ 0x24

/// 0x24 ItemFromBelt.
pub fn item_from_belt<W: MoveWorld>(w: &mut W, p: Owner, item: Guid) -> Result<u32, MoveFatal> {
    let r = w.check_belt(p, item);
    if r != 0 {
        return Ok(r);
    }
    if !w.item_move_gate(p, None) {
        return Ok(res::OK);
    }
    if w.cursor(p).is_some() {
        return Ok(res::BAD);
    }
    if !exists(w, item) || w.mode(item) != mode::BELT {
        return Ok(res::REFUSED);
    }
    w.targeting_reset(p);
    let slot = w.pos(Owner::item(item)).0;
    if !w.unlink(p, item) {
        return Err(MoveFatal::Unlink);
    }
    w.set_cursor(p, Some(item));
    w.charm_unlink(p, item);
    clear_uflags(w, item, uflag::TARGETABLE);
    w.stat_refresh(p);
    w.set_mode(item, mode::CURSOR);
    add_cmd(w, item, cmd::FROM_BELT);
    owner_refresh(w, p);
    w.update_list_add(p, item);
    w.belt_compact(p, slot as u8);
    Ok(res::OK)
}

// ------------------------------------------------------------------ 0x25

/// 0x25 SwitchBeltItem.
pub fn switch_belt_item<W: MoveWorld>(
    w: &mut W,
    p: Owner,
    c: Guid,
    b: Guid,
) -> Result<u32, MoveFatal> {
    let r = w.check_cursor_item(p, c);
    if r != 0 {
        return Ok(r);
    }
    let r = w.check_belt(p, b);
    if r != 0 {
        return Ok(r);
    }
    // The cursor item check guarantees mode 4.
    if !w.beltable(c) || w.mode(c) != mode::CURSOR {
        return Ok(res::OK);
    }
    if !exists(w, b) || w.mode(b) != mode::BELT {
        return Ok(res::REFUSED);
    }
    let slot = w.pos(Owner::item(b)).0;
    if !w.unlink(p, b) {
        return Err(MoveFatal::Unlink);
    }
    w.set_cursor(p, Some(b));
    w.set_mode(b, mode::CURSOR);
    add_cmd(w, b, cmd::SWAP_BELT);
    w.update_list_add(p, b);
    if !w.belt_place(p, c, slot as u32) {
        return Err(MoveFatal::BeltSwitch);
    }
    w.set_pos(Owner::item(c), slot, 0);
    // C's link failing → fatal assert (line 0x12D4, MV4).
    if !w.link_check(p, c, 2) {
        return Err(MoveFatal::Link);
    }
    w.set_mode(c, mode::BELT);
    w.set_page(c, page::NONE);
    add_cmd(w, c, cmd::SWAP_BELT);
    w.update_list_add(p, c);
    owner_refresh(w, p);
    w.inventory_pass(p);
    Ok(res::OK)
}

// ------------------------------------------------------------------ 0x26

/// 0x26 UseBeltItem.
pub fn use_belt_item<W: MoveWorld>(w: &mut W, p: Owner, item: Guid, on_merc: u32) -> u32 {
    let r = w.check_belt(p, item);
    if r != 0 {
        return r;
    }
    // The player's own position passes the 50-subtile test.
    if !exists(w, item) {
        return res::REFUSED;
    }
    if w.cursor(p).is_some() {
        return res::OK;
    }
    if w.mode(item) != mode::BELT || !w.useable(item) {
        return res::REFUSED;
    }
    if w.trading(p) {
        return res::OK;
    }
    let mut target = p;
    if on_merc != 0 && p.is_player() && w.expansion() {
        if ![ty::HPOT, ty::APOT, ty::WPOT]
            .iter()
            .any(|&t| w.is_type(item, t))
        {
            return res::OK;
        }
        // No hireling: the target stays the player, so the potion is used
        // on the player.
        if let Some(merc) = w.hireling(p) {
            target = merc;
        }
    }
    let slot = w.pos(Owner::item(item)).0;
    if w.use_item(p, target, item) {
        w.charge_update(p, item);
        w.targeting_reset(p);
        w.remove_used(p, item);
        w.belt_compact(p, slot as u8);
    }
    res::OK
}

// ------------------------------------------------------------------ 0x27

/// 0x27 UseItemAction.
pub fn use_item_action<W: MoveWorld>(w: &mut W, p: Owner, target: Guid, used: Guid) -> u32 {
    for g in [target, used] {
        let r = w.check_owned(p, g);
        if r != 0 {
            return r;
        }
    }
    use_item_action_body(w, p, target, used).result()
}

/// U (scroll or tome)
/// used on item T; the effect belongs to item use.
pub fn use_item_action_body<W: MoveWorld>(w: &mut W, p: Owner, t: Guid, u: Guid) -> Outcome {
    // Step 1.
    if !exists(w, u) {
        return Outcome::REFUSED;
    }
    if !exists(w, t) || t == u {
        w.targeting_reset(p);
        return Outcome::NOTHING;
    }
    // Step 2.
    let um = w.mode(u);
    if um == mode::BELT && !w.is_type(u, ty::SCRO) {
        return Outcome::REFUSED;
    }
    if w.trading(p) {
        return Outcome::NOTHING;
    }
    if w.cursor(p).is_some() {
        return Outcome::NOTHING;
    }
    let uo = Owner::item(u);
    let book = w.primary_type(u) == ty::BOOK;
    // Step 3.
    let tm = w.mode(t);
    if tm != mode::STORED && tm != mode::EQUIPPED {
        if um == mode::BELT {
            w.targeting_reset(p);
            w.remove_used(p, u);
            return Outcome::NOTHING;
        }
        if um == mode::STORED {
            if book {
                let q = w.stat(uo, stat::QUANTITY);
                w.set_stat(uo, stat::QUANTITY, (q - 1).max(0));
                w.send_item_stat(p, u, stat::QUANTITY);
            }
            return Outcome::NOTHING;
        }
        return Outcome::REFUSED;
    }
    // Step 4.
    if book && w.stat(uo, stat::QUANTITY) < 1 {
        w.targeting_reset(p);
        w.send(p, layouts::item_used(Owner::ITEM, u));
        return Outcome::NOTHING;
    }
    // Step 5.
    if !w.use_item(p, Owner::item(t), u) {
        return Outcome::DONE;
    }
    // Step 6.
    let s = w.item_skill(u);
    // Step 7.
    if um == mode::BELT {
        if s == -1 {
            w.send(p, layouts::item_used(Owner::ITEM, u));
        } else {
            w.skill_decrement(p, s);
            w.remove_used(p, u);
        }
        w.targeting_reset(p);
        return Outcome::DONE;
    }
    // Step 8.
    if um != mode::STORED {
        return Outcome::REFUSED;
    }
    // Step 9.
    if book {
        let q = w.stat(uo, stat::QUANTITY);
        if s != -1 && q >= 1 {
            w.set_stat(uo, stat::QUANTITY, q - 1);
            w.send_item_stat(p, u, stat::QUANTITY);
            w.skill_decrement(p, s);
            w.send(p, layouts::item_used(Owner::ITEM, u));
        } else {
            w.consume_item(p, u);
        }
    } else {
        if s != -1 {
            w.skill_decrement(p, s);
        }
        w.consume_item(p, u);
    }
    w.targeting_reset(p);
    Outcome::DONE
}

// ------------------------------------------------------------------ 0x28

/// 0x28 SocketItem.
pub fn socket_item<W: MoveWorld>(
    w: &mut W,
    p: Owner,
    filler: Guid,
    target: Guid,
) -> Result<u32, MoveFatal> {
    let r = w.check_cursor_item(p, filler);
    if r != 0 {
        return Ok(r);
    }
    let r = w.check_stored_or_equipped(p, target);
    if r != 0 {
        return Ok(r);
    }
    if w.trading(p) {
        return Ok(res::OK);
    }
    w.targeting_reset(p);
    if !exists(w, filler) {
        return Ok(res::REFUSED);
    }
    if w.mode(filler) != mode::CURSOR || !exists(w, target) {
        return Ok(res::REFUSED);
    }
    if w.item_flags(target) & iflag::IDENTIFIED == 0 {
        return Ok(res::OK);
    }
    // Only "filler missing / not in mode 4 / target missing" set out; the
    // other checks → 0 with out 0 (MV4).
    if w.mode(target) > mode::EQUIPPED {
        return Ok(res::OK);
    }
    if !w.socket_filler(filler) || w.item_flags(filler) & iflag::IDENTIFIED == 0 {
        return Ok(res::OK);
    }
    if w.item_flags(target) & iflag::SOCKETED == 0
        || w.fillers(target).len() as i64 >= i64::from(w.sockets(target))
    {
        return Ok(res::OK);
    }
    if !w.link_into_item(target, filler) {
        return Err(MoveFatal::Link);
    }
    w.set_cursor(p, None);
    clear_uflags(w, filler, uflag::TARGETABLE);
    w.filler_linked(filler, target);
    w.set_mode(filler, mode::SOCKETED);
    w.runeword(p, target);
    add_iflags(w, target, iflag::CHANGED);
    clear_iflags(w, target, iflag::NOEQUIP);
    owner_refresh(w, p);
    w.update_list_add(p, target);
    Ok(res::OK)
}

// ------------------------------------------------------------------ 0x29

/// 0x29 ScrollToBook.
pub fn scroll_to_book<W: MoveWorld>(
    w: &mut W,
    p: Owner,
    scroll: Guid,
    book: Guid,
) -> Result<u32, MoveFatal> {
    let r = w.check_ground_or_owned(p, scroll);
    if r != 0 {
        return Ok(r);
    }
    let r = w.check_stored(p, book);
    if r != 0 {
        return Ok(r);
    }
    Ok(scroll_into_book(w, p, scroll, book)?.result())
}

/// (also the scroll pickup of
/// Step 4).
pub fn scroll_into_book<W: MoveWorld>(
    w: &mut W,
    p: Owner,
    scroll: Guid,
    book: Guid,
) -> Result<Outcome, MoveFatal> {
    if !exists(w, scroll) {
        return Ok(Outcome::REFUSED);
    }
    let sm = w.mode(scroll);
    if (sm != mode::GROUND && sm != mode::CURSOR) || !w.is_type(scroll, ty::SCRO) {
        return Ok(Outcome::REFUSED);
    }
    if !exists(w, book) {
        return Ok(Outcome::REFUSED);
    }
    // The book type is the primary type (MV5).
    if w.mode(book) != mode::STORED || w.primary_type(book) != ty::BOOK {
        return Ok(Outcome::REFUSED);
    }
    if w.spell(book) != w.spell(scroll) {
        return Err(MoveFatal::SpellMismatch);
    }
    let b = Owner::item(book);
    let q = w.stat(b, stat::QUANTITY);
    if q >= w.max_stack(book) {
        return Ok(Outcome::NOTHING);
    }
    if !w.consume_one(scroll) {
        // A scroll in a room (picked from the ground) leaves
        // it with the room delete notice, the collision
        // and the list; unit flag 0x2 cleared.
        // Without the notice the client kept drawing the
        // freed scroll on the ground (B027).
        super::ground::leave_room(w, scroll);
        clear_uflags(w, scroll, uflag::TARGETABLE);
        w.free_item(scroll);
        w.set_cursor(p, None);
        if sm == mode::CURSOR {
            // Recorded (`facts/items/a1-town-item-moves.tsv`): a cursor
            // scroll put into its tome clears the player's cursor with
            // S→C 0x42 before the tome's 0x3E.
            w.send(p, layouts::clear_cursor(p.ty, p.guid));
        }
    }
    w.set_stat(b, stat::QUANTITY, q.wrapping_add(1));
    w.send_item_stat(p, book, stat::QUANTITY);
    w.book_count_changed(p, book, 1);
    Ok(Outcome::DONE)
}

// ------------------------------------------------------------------ 0x50

/// 0x50 DropGold.
pub fn drop_gold<W: MoveWorld>(w: &mut W, p: Owner, unit: Guid, amount: u32) -> u32 {
    if w.busy(p) && w.trading(p) {
        return res::REFUSED;
    }
    if unit != p.guid {
        return res::REFUSED;
    }
    let amount = amount as i32;
    let gold = w.stat(p, stat::GOLD);
    if amount < 0 || amount > gold || amount > gold_limit(w, p) {
        return res::REFUSED;
    }
    if amount == 0 {
        return res::OK;
    }
    let amount = amount.min(PILE_CAP);
    for pile in gold_piles(w, p, amount, MAX_PILES) {
        if !w.gold_drop_has_no_owner() {
            w.set_owner(pile, p);
        }
        let g = w.stat(p, stat::GOLD) - w.stat(Owner::item(pile), stat::GOLD);
        // Setting gold: a negative value stores 0.
        w.set_stat(p, stat::GOLD, g.max(0));
    }
    res::OK
}

// ------------------------------------------------------------------ 0x61

const MERC_A1: u32 = 0x10F;
const MERC_A2: u32 = 0x152;
const MERC_A3: u32 = 0x167;
const MERC_A5A: u32 = 0x230;
const MERC_A5B: u32 = 0x231;

/// 0x61 MercItem, expansion only.
pub fn merc_item<W: MoveWorld>(w: &mut W, p: Owner, loc: u16) -> Result<u32, MoveFatal> {
    if !w.expansion() {
        return Ok(res::REFUSED);
    }
    if w.busy(p) && w.trading(p) {
        return Ok(res::REFUSED);
    }
    if w.has_used_skill(p) || !w.alive(p) {
        return Ok(res::OK);
    }
    let Some(merc) = w.hireling(p) else {
        return Ok(res::OK);
    };
    if !w.alive(merc) || !w.owns_hireling(p, merc) {
        return Ok(res::OK);
    }
    if !w.has_inventory(p) {
        return Ok(res::REFUSED);
    }
    if let Some(c) = w.cursor(p) {
        if w.quest(c) == 0 {
            merc_give(w, p, Some(merc), c);
        }
        return Ok(res::OK);
    }
    if loc != 0 {
        return merc_take(w, p, merc, loc);
    }
    Ok(res::OK)
}

/// Take from the hireling.
pub fn merc_take<W: MoveWorld>(
    w: &mut W,
    p: Owner,
    merc: Owner,
    loc: u16,
) -> Result<u32, MoveFatal> {
    //–: classic, a hireling without an
    // inventory, or a location outside 1..10 → 3.
    if !w.expansion() || !w.has_inventory(merc) || !valid_loc(u32::from(loc)) {
        return Ok(res::REFUSED);
    }
    let loc = loc as u8;
    let Some(it) = w
        .body_item(merc, loc)
        .filter(|&i| w.mode(i) == mode::EQUIPPED)
    else {
        return Ok(res::BAD);
    };
    // The unlink must return the item, else 2.
    if !w.unlink(merc, it) {
        return Ok(res::BAD);
    }
    w.clear_body_slot(merc, loc);
    w.stat_refresh_unlink(merc, 0);
    mark(w, merc, it, cmd::UNEQUIP);
    // A failed copy is fatal (the cursor is set to none, then the base
    // game asserts), after the item already left the hireling.
    let Some(copy) = w.copy_item(it) else {
        return Err(MoveFatal::Create);
    };
    w.give_cursor_item(p, copy);
    add_iflags(w, it, iflag::COPIED);
    w.merc_after_take(merc);
    Ok(res::OK)
}

/// Give to the hireling; returns its result (the
/// caller ignores it).
pub fn merc_give<W: MoveWorld>(w: &mut W, p: Owner, merc: Option<Owner>, c: Guid) -> u32 {
    if !w.expansion() {
        return res::REFUSED;
    }
    let f = w.item_flags(c);
    if f & iflag::IDENTIFIED == 0 || f & iflag::BROKEN != 0 {
        return 0;
    }
    if [ty::HPOT, ty::WPOT, ty::APOT]
        .iter()
        .any(|&t| w.is_type(c, t))
    {
        let target = merc.unwrap_or(p);
        w.use_item(p, target, c);
        w.consume_one(c);
        w.set_cursor(p, None);
        return 1;
    }
    let one_handed = !w.two_handed(c);
    let allowed = w.is_type(c, ty::TORS)
        || w.is_type(c, ty::HELM)
        || match merc.map(|m| w.unit_class(m)) {
            Some(MERC_A1) => w.is_type(c, ty::BOW),
            Some(MERC_A2) => w.is_type(c, ty::SPEA) || w.is_type(c, ty::POLE),
            Some(MERC_A3) => w.is_type(c, ty::SHIE) || (w.is_type(c, ty::SWOR) && one_handed),
            Some(MERC_A5A) => (w.is_type(c, ty::AXE) && one_handed) || w.is_type(c, ty::PHLM),
            Some(MERC_A5B) => w.is_type(c, ty::SWOR) || w.is_type(c, ty::PHLM),
            _ => false,
        };
    if let Some(m) = merc {
        if allowed && w.requirements(c, m, false) {
            w.equip_on_merc(m, c);
        }
    }
    w.merc_sound(p);
    0
}

// ------------------------------------------------------------------ 0x63

/// 0x63 ItemToBeltShift.
pub fn item_to_belt_shift<W: MoveWorld>(w: &mut W, p: Owner, item: Guid) -> Result<u32, MoveFatal> {
    let r = w.check_stored(p, item);
    if r != 0 {
        return Ok(r);
    }
    if w.cursor(p).is_some() {
        w.send(p, layouts::cant_do_that());
        return Ok(res::BAD);
    }
    if !exists(w, item) || !w.beltable(item) {
        return Ok(res::BAD);
    }
    if w.page(item) != page::INVENTORY {
        return Ok(res::REFUSED);
    }
    let Some(slot) = w.belt_free_slot(p, item) else {
        return Ok(res::OK);
    };
    if !w.item_move_gate(p, Some(item)) {
        return Ok(res::OK);
    }
    if !w.has_inventory(p) {
        return Err(MoveFatal::NoInventory);
    }
    let pg = w.page(item);
    if pg != page::INVENTORY || w.mode(item) != mode::STORED {
        return Ok(res::BAD);
    }
    let (x, y) = w.pos(Owner::item(item));
    if !w.unlink(p, item) {
        return Err(MoveFatal::Unlink);
    }
    w.room_change_notice(item, x, y);
    w.set_stored_page(item, pg);
    w.set_page(item, page::NONE);
    send_item_page(w, p, item, 0, pg)?;
    w.set_mode(item, mode::CURSOR);
    clear_uflags(w, item, uflag::TARGETABLE);
    if w.belt_place(p, item, u32::from(slot)) {
        w.set_page(item, page::NONE);
        w.link_check(p, item, 2);
        w.set_mode(item, mode::BELT);
        send_to_belt(w, p, item)?;
    }
    owner_refresh(w, p);
    Ok(res::OK)
}

//! The hireling item swap (game, player, merc, item C), reached from the
//! 0x61 give ([`crate::items::moves::handlers`]) when C is allowed. Item
//! placement, requirements, the cursor equip, duplication and messages
//! belong to other modules and are reached
//! through [`HirelingItems`]. Units and items are the 0x61 path's
//! [`Owner`] / [`Guid`] so that path's `equip_on_merc` seam can be served
//! by [`swap`].
//!
//! Caller: the 0x61 give's `equip_on_merc` seam
//! on the inventory wiring (`wiring::inventory::merc`), with the host's
//! hireling lists lent to the inventory state; its [`HirelingItems`]
//! provider is `wiring::inventory::merc::MercItems` (the duplicate is
//! `InvDesk::copy_of`).

use crate::items::moves::{Guid, Owner};

use super::class;

/// Results of [`swap`].
pub mod res {
    /// Step 1: classic game.
    pub const CLASSIC: u32 = 3;
    /// Step 1 (no player inventory) and step 4 fail.
    pub const NONE: u32 = 0;
    /// Steps 3 and 4 pass.
    pub const SWAPPED: u32 = 1;
}

/// Item type `shie` (2): the Act 3 hireling's left-hand rule.
pub const TYPE_SHIELD: u32 = 2;
/// Mode of the duplicate before the cursor equip.
pub const MODE_CURSOR: u8 = 4;
/// Mode of an item put back at its body location (step 4 fail).
pub const MODE_EQUIPPED: u8 = 1;
/// Page of an item put back at its body location (step 4 fail).
pub const PAGE_BODY: u8 = 3;
/// Item flag 0x10, set on the replaced item (step 4).
pub const FLAG_10: u32 = 0x10;
/// Item flag 0x20, set on the replaced item (step 4).
pub const FLAG_20: u32 = 0x20;
/// Timer type 9 (periodic stats) cancelled with C's GUID on the merc and
/// on the player.
pub const TIMER_STATS: u8 = 9;
/// Timer type 3 (regeneration) cancelled on the merc, any argument, then
/// scheduled again at frame + 1.
pub const TIMER_REGEN: u8 = 3;
/// Event id queued at frame + 1.
pub const EVENT_3: u32 = 3;

/// Everything the swap reaches outside this module. `player` and `merc` are the
/// swap's units; each call takes the unit the base game passes it (step 4 "Order
/// and units").
pub trait HirelingItems {
    /// The unit has an inventory.
    fn has_inventory(&self, unit: Owner) -> bool;
    /// Give the unit an inventory.
    fn create_inventory(&mut self, unit: Owner);
    /// C's two body locations: itemtypes `BodyLoc1` / `BodyLoc2`.
    fn body_locs(&self, item: Guid) -> (u8, u8);
    /// Monstats class of the unit.
    fn class(&self, unit: Owner) -> u32;
    /// The item is of item type `ty` (the 0x61 give's type test).
    fn is_type(&self, item: Guid, ty: u32) -> bool;
    /// The unit's item at a body location.
    fn body_item(&self, unit: Owner, loc: u8) -> Option<Guid>;
    /// A
    /// duplicate of `item` owned by `owner` (new GUID; socketed children
    /// recreated in mode 4 and inserted; flags 0x80000 set, 0x2000
    /// cleared; the source gets 0x8000000; replenish timers). `None`: the
    /// creation failed (also
    /// a socketed child's).
    fn duplicate(&mut self, owner: Owner, item: Guid) -> Option<Guid>;
    /// The item's mode.
    fn set_mode(&mut self, item: Guid, mode: u8);
    /// Cancel the unit's pending timer events of `ty` whose first
    /// argument equals `a` (`a` = 0: any). Step 3: `(9, C's GUID)` on the
    /// merc then on the player, `(3, 0)` on the merc. Nothing is sent.
    fn cancel_timers(&mut self, unit: Owner, ty: u8, a: Guid);
    /// The cursor equip of `item` at `loc`; the swap does not read its
    /// result.
    fn equip_from_cursor(&mut self, unit: Owner, item: Guid, loc: u8, skip: bool);
    /// Consume the item.
    fn consume(&mut self, player: Owner, item: Guid);
    /// The player's cursor item := none.
    fn clear_cursor(&mut self, player: Owner);
    /// The inventory pass; the swap calls it on the merc.
    fn inventory_pass(&mut self, unit: Owner);
    /// An inventory refresh; the swap calls it on the merc.
    fn refresh_0055f4f0(&mut self, unit: Owner);
    /// Event `id` at the current frame + 1; the swap queues it on the
    /// merc.
    fn event_next_frame(&mut self, unit: Owner, id: u32);
    /// Unlink `item` from the unit's inventory; it must be the item at
    /// the slot.
    fn unlink(&mut self, unit: Owner, item: Guid);
    /// Clear the unit's body slot `loc`.
    fn clear_slot(&mut self, unit: Owner, loc: u8);
    /// The unit's stat refresh after an unlink.
    fn stat_refresh_unlink(&mut self, unit: Owner);
    /// Whether `unit` meets `item`'s requirements (`equipping`: for an
    /// equip).
    fn requirements(&self, item: Guid, unit: Owner, equipping: bool) -> bool;
    /// Set item flag `flag` (step 4: 0x10, then 0x20).
    fn item_flag_on(&mut self, item: Guid, flag: u32);
    /// The item leaves the unit's inventory (step 4 pass).
    fn leave_inventory(&mut self, unit: Owner, item: Guid);
    /// A unit call of the step 4 pass, with `arg` (unnamed in D2RUST).
    fn call_00621000(&mut self, unit: Owner, arg: u32);
    /// The player's cursor := `item`, then the cursor update with it;
    /// `None` (a failed duplicate of old): cursor := none, the cursor
    /// update with none.
    fn become_cursor(&mut self, player: Owner, item: Option<Guid>);
    /// Link `item` back into the unit's inventory at `page` and body
    /// location `loc` (step 4 fail).
    fn put_back(&mut self, unit: Owner, item: Guid, page: u8, loc: u8);
    /// An item call of the step 4 fail, with `arg` (unnamed in D2RUST).
    fn call_00628280(&mut self, item: Guid, arg: u8);
    /// The merc refresh.
    fn refresh_0055c460(&mut self, merc: Owner);
}

/// The swap of item C onto the merc: 3 classic, 0
/// no player inventory or C fails the requirements, 1 swapped.
pub fn swap<W: HirelingItems>(
    w: &mut W,
    expansion: bool,
    player: Owner,
    merc: Owner,
    item: Guid,
) -> u32 {
    // Step 1.
    if !expansion {
        return res::CLASSIC;
    }
    if !w.has_inventory(player) {
        return res::NONE;
    }
    if !w.has_inventory(merc) {
        w.create_inventory(merc);
    }
    // Step 2.
    let (loc1, loc2) = w.body_locs(item);
    let target = if w.class(merc) == class::ACT3 && w.is_type(item, TYPE_SHIELD) {
        loc2
    } else {
        loc1
    };
    let Some(old) = w.body_item(merc, target) else {
        // Step 3.
        equip_copy(w, player, merc, item, target, None);
        return res::SWAPPED;
    };
    // Step 4: unlink old, the stat refresh, the inventory pass, then the
    // requirement check.
    w.unlink(merc, old);
    w.clear_slot(merc, target);
    w.stat_refresh_unlink(merc);
    w.inventory_pass(merc);
    if w.requirements(item, merc, false) {
        w.item_flag_on(old, FLAG_10);
        w.item_flag_on(old, FLAG_20);
        w.leave_inventory(merc, old);
        w.call_00621000(merc, 1);
        equip_copy(w, player, merc, item, target, Some(old));
        res::SWAPPED
    } else {
        w.put_back(merc, old, PAGE_BODY, target);
        w.set_mode(old, MODE_EQUIPPED);
        w.call_00628280(old, 0xFF);
        w.refresh_0055c460(merc);
        w.refresh_0055f4f0(merc);
        res::NONE
    }
}

/// Step 3, with step 4's "duplicate of old to the player" when `old` is
/// given.
///
/// A failed duplicate of C: the mode set is skipped
/// (a null unit is ignored), the two type-9 cancels run, the
/// equip runs with GUID −1 and equips nothing (it finds no
/// unit: the seam is not called), C is still consumed and the cursor
/// cleared: the item is lost and the merc's slot stays empty. The tail
/// and result 1 are unchanged.
fn equip_copy<W: HirelingItems>(
    w: &mut W,
    player: Owner,
    merc: Owner,
    item: Guid,
    target: u8,
    old: Option<Guid>,
) {
    let copy = w.duplicate(merc, item);
    if let Some(copy) = copy {
        w.set_mode(copy, MODE_CURSOR);
    }
    w.cancel_timers(merc, TIMER_STATS, item);
    w.cancel_timers(player, TIMER_STATS, item);
    if let Some(copy) = copy {
        w.equip_from_cursor(merc, copy, target, true);
    }
    w.consume(player, item);
    w.clear_cursor(player);
    // Step 4: the old item's duplicate after "cursor := none", before
    // the refresh calls (all four on the merc).
    if let Some(old) = old {
        let back = w.duplicate(player, old);
        w.become_cursor(player, back);
    }
    w.inventory_pass(merc);
    w.refresh_0055f4f0(merc);
    w.cancel_timers(merc, TIMER_REGEN, 0);
    w.event_next_frame(merc, EVENT_3);
}

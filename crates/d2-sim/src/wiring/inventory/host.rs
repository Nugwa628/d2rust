//! The inventory model for the other item systems (vendors, the cube):
//! one inventory per owner unit, read and written by every system that
//! touches a player's items. The item-move code (`items::moves`) runs on
//! [`InvDesk`] through its seams; the vendor and cube adapters of a host
//! call the entry points here, keyed by unit, instead of keeping lists of
//! their own.
//!
//! Reads that need no unit record ([`InvState::holds`],
//! [`InvState::items_of`], [`InvState::cursor_of`],
//! [`InvState::body_items`], [`InvState::fillers`]) are on the state;
//! the rules (placement, unlink, the item checks, the targeting
//! reset, the direct 0x9D) run on the desk, each the
//! `items::inventory` / `items::moves` function of its rule.

use super::{InvDesk, InvError, InvRest, InvState};
use crate::items::inventory::{
    grid_id, ground_or_owned_check, place_in_page, stored_item_check, targeting_reset, Inventory,
    BODY_GRID,
};
use crate::items::moves::{deferred, mode, MoveFatal, MovePending, MoveUnits};
use crate::units::lifecycle::LifecycleHooks;
use crate::units::UnitId;

impl InvState {
    /// The owner's inventory.
    pub fn of(&self, owner: UnitId) -> Option<&Inventory> {
        self.inventories.get(&owner)
    }

    /// The owner's item list in link order; empty without an
    /// inventory.
    pub fn items_of(&self, owner: UnitId) -> Vec<UnitId> {
        self.of(owner).map_or_else(Vec::new, |i| i.items().to_vec())
    }

    /// The owner's cursor item.
    pub fn cursor_of(&self, owner: UnitId) -> Option<UnitId> {
        self.of(owner)?.cursor()
    }

    /// The item is the owner's: in its item list or its cursor item (the
    /// owned-item test without the item lookup).
    pub fn holds(&self, owner: UnitId, item: UnitId) -> bool {
        self.of(owner)
            .is_some_and(|i| i.contains(item) || i.cursor() == Some(item))
    }

    /// The items at the body locations (grid 0), in location order.
    pub fn body_items(&self, owner: UnitId) -> Vec<UnitId> {
        let Some(inv) = self.of(owner) else {
            return Vec::new();
        };
        (0..BODY_GRID.0)
            .filter_map(|loc| inv.item_at(grid_id::BODY, i32::from(loc), 0))
            .collect()
    }

    /// The items socketed into `item`: its own inventory's item list, in
    /// link order (the inventory an item with fillers owns).
    pub fn fillers(&self, item: UnitId) -> Vec<UnitId> {
        self.items_of(item)
    }
}

impl<H: LifecycleHooks, R: InvRest + ?Sized> InvDesk<'_, '_, H, R> {
    /// Item placement into a page (the targeting
    /// reset, then the cursor item `item` into the page its item
    /// data names, at (x, y) or a free position). False when the owner has
    /// no inventory or the placement fails. A cursor-mode item still in a
    /// room is a caller error (the mode test does not check
    /// the room): recorded, not placed.
    pub fn place(
        &mut self,
        owner: UnitId,
        item: UnitId,
        (x, y): (i32, i32),
        find_free: bool,
        send: bool,
    ) -> bool {
        let in_room = self
            .econ
            .game
            .lists
            .unit(item)
            .and_then(|e| e.room())
            .is_some();
        if in_room && self.econ.units.get(item).is_some_and(|r| r.mode == 4) {
            self.state.errors.push(InvError::PlacedWithRoom(item));
            return false;
        }
        let Some(o) = self.owner_of(owner) else {
            return false;
        };
        let t = self.tables;
        let item = self.state.items.contains_key(&item).then_some(item);
        self.with_inv(o, |inv, d| {
            place_in_page(inv, d, t, item, x, y, find_free, send)
        })
        .unwrap_or(false)
    }

    /// The NPC grid placement of a store item: the NPC's inventory (made
    /// on first use, the monster record by page) takes the item on
    /// its store page at the first free position; no send (the store
    /// stream is the trade open's 0x9C action 11).
    /// False when the page has no room.
    pub fn store_place(&mut self, npc: UnitId, item: UnitId) -> bool {
        if let (Some(kind), Some(r)) = (self.kind_of(npc), self.econ.units.get(npc)) {
            self.state.add_inventory(npc, kind, r.guid);
        }
        self.place(npc, item, (0, 0), true, false)
    }

    /// Runs `f` with the (NPC, player) gamble node's inventory standing in
    /// for the NPC's own (made on first use), then puts both back.
    fn with_gamble<T>(&mut self, npc: UnitId, player: u32, f: impl FnOnce(&mut Self) -> T) -> T {
        if let (Some(kind), Some(r)) = (self.kind_of(npc), self.econ.units.get(npc)) {
            self.state.add_inventory(npc, kind, r.guid);
        }
        let node = match self.state.gamble.remove(&(npc, player)) {
            Some(n) => n,
            None => {
                let own = self.state.inventories.get(&npc);
                crate::items::inventory::Inventory::new(
                    npc,
                    own.map(|i| i.owner_kind)
                        .unwrap_or(crate::items::inventory::UnitKind::Other),
                    self.guid_of(npc),
                )
            }
        };
        let real = self.state.inventories.insert(npc, node);
        let out = f(self);
        if let Some(node) = self.state.inventories.remove(&npc) {
            self.state.gamble.insert((npc, player), node);
        }
        if let Some(real) = real {
            self.state.inventories.insert(npc, real);
        }
        out
    }

    /// Places `item` into the (NPC, player) gamble node's inventory:
    /// mode 0 on its page-0 grid.
    pub fn gamble_place(&mut self, npc: UnitId, player: u32, item: UnitId) -> bool {
        self.with_gamble(npc, player, |d| d.place(npc, item, (0, 0), true, false))
    }

    /// Unlinks a gamble item from the player's gamble node that holds it.
    /// The node is found by its contents, not by the NPC unit: a list is
    /// dropped on a chat close or a town refresh, where the caller knows
    /// the vendor record but not the NPC unit the list was made at. A
    /// node left empty goes away, so the next list starts on an empty
    /// grid. False when no node of the player holds the item.
    pub fn gamble_unlink(&mut self, player: u32, item: UnitId) -> bool {
        let npc = self
            .state
            .gamble
            .iter()
            .find(|((_, p), node)| *p == player && node.items().contains(&item))
            .map(|(&(npc, _), _)| npc);
        let Some(npc) = npc else {
            return false;
        };
        let removed = self.with_gamble(npc, player, |d| d.store_unlink(item));
        if self
            .state
            .gamble
            .get(&(npc, player))
            .is_some_and(|node| node.items().is_empty())
        {
            self.state.gamble.remove(&(npc, player));
        }
        removed
    }

    /// Unlinks a store item from the NPC inventory that holds it (a
    /// monster-owned inventory), for the take
    /// of a purchase and the removal of a store clear.
    /// False when no NPC inventory holds it.
    pub fn store_unlink(&mut self, item: UnitId) -> bool {
        let holder = self
            .state
            .inventories
            .iter()
            .find(|(_, i)| {
                matches!(
                    i.owner_kind,
                    crate::items::inventory::UnitKind::Monster { .. }
                ) && i.items().contains(&item)
            })
            .map(|(&u, _)| u);
        let Some(npc) = holder else {
            return false;
        };
        let removed = self.remove(npc, item);
        if removed {
            // The taken store item reaches the client in mode 4 (a
            // purchase shows it in the 0x9C action 12 record).
            let g = self.guid_of(item);
            self.set_mode(g, mode::CURSOR);
        }
        removed
    }

    /// Unlinks `item` from the owner's inventory.
    /// False when it is not there.
    pub fn remove(&mut self, owner: UnitId, item: UnitId) -> bool {
        let Some(o) = self.owner_of(owner) else {
            return false;
        };
        self.with_inv(o, |inv, d| inv.unlink(d, item))
            .unwrap_or(false)
    }

    /// Takes `item` off the owner's cursor and consumes it, in mode 4.
    /// False when it is not the owner's cursor item.
    ///
    /// When the item is the owner's cursor item, S→C 0x42 names the owner
    /// (type, GUID), the cursor is cleared and the item freed; the
    /// sale from the cursor sends 0x42 before the 0x2A.
    pub fn take_cursor(&mut self, owner: UnitId, item: UnitId) -> bool {
        if self.state.cursor_of(owner) != Some(item) {
            return false;
        }
        let Some(o) = self.owner_of(owner) else {
            return false;
        };
        // S→C 0x42 names the player whose cursor clears.
        let pg = self.guid_of(owner);
        MovePending::send(self, o, crate::items::moves::layouts::clear_cursor(0, pg));
        self.free(item);
        true
    }

    /// The cursor removal of [`Self::take_cursor`] without the free: the
    /// item leaves the owner's cursor and S→C 0x42 names the owner; the
    /// caller frees it once it has read it (the NPC services, whose drop
    /// request was filled from the input before the removal). False when
    /// it is not the owner's cursor item. The client gets
    /// `42 00 01000000` before the 0x58.
    pub fn take_cursor_keep(&mut self, owner: UnitId, item: UnitId) -> bool {
        if self.state.cursor_of(owner) != Some(item) {
            return false;
        }
        let Some(o) = self.owner_of(owner) else {
            return false;
        };
        if !self
            .with_inv(o, |inv, d| inv.unlink(d, item))
            .unwrap_or(false)
        {
            return false;
        }
        let pg = self.guid_of(owner);
        MovePending::send(self, o, crate::items::moves::layouts::clear_cursor(0, pg));
        true
    }

    /// Frees an item unit: the unit is removed and its item data dropped.
    pub fn free(&mut self, item: UnitId) {
        let g = self.guid_of(item);
        MovePending::free_item(self, g);
    }

    /// The targeting reset of the owner's item list.
    pub fn reset_targeting(&mut self, owner: UnitId) {
        let Some(o) = self.owner_of(owner) else {
            return;
        };
        self.with_inv(o, |inv, d| targeting_reset(inv, d));
    }

    /// Stored item check of the GUID for `player` (a
    /// player without an inventory is checked against an empty one).
    pub fn check_stored(&self, player: UnitId, guid: u32) -> u8 {
        self.unit_check(player, guid, stored_item_check)
    }

    /// Ground or owned check of the GUID for `player`.
    pub fn check_ground_or_owned(&self, player: UnitId, guid: u32) -> u8 {
        self.unit_check(player, guid, ground_or_owned_check)
    }

    fn unit_check(&self, player: UnitId, guid: u32, f: fn(&Inventory, &Self, u32) -> u8) -> u8 {
        match self.owner_of(player) {
            Some(o) => f(&self.inv_or_empty(o), self, guid),
            None => 1,
        }
    }

    /// Direct S→C 0x9C with `action` for `item`, queued to the owner now
    /// (`deferred::send_item_world`; used for the store item's action 11).
    pub fn send_item_world(
        &mut self,
        owner: UnitId,
        item: UnitId,
        action: u8,
        flags: u32,
    ) -> Result<(), MoveFatal> {
        let (Some(o), g) = (self.owner_of(owner), self.guid_of(item)) else {
            return Ok(());
        };
        deferred::send_item_world(self, o, g, action, flags)
    }

    /// The shown store item to the trading
    /// player: S→C 0x9C action 0x0B with the store stream
    /// (`deferred::store_item_message`), queued now.
    pub fn send_store_item(&mut self, player: UnitId, item: UnitId) -> Result<(), MoveFatal> {
        let (Some(o), g) = (self.owner_of(player), self.guid_of(item)) else {
            return Ok(());
        };
        for m in deferred::store_item_message(self, g)? {
            MovePending::send(self, o, m);
        }
        Ok(())
    }

    /// Direct S→C 0x9D action 5 for an item of the
    /// owner, queued now through `MovePending::send`: the stored page set
    /// to `shown` first, item flags OR-ed with `flags` in
    /// the message.
    pub fn send_item_page(
        &mut self,
        owner: UnitId,
        item: UnitId,
        flags: u32,
        shown: u8,
    ) -> Result<(), MoveFatal> {
        let (Some(o), g) = (self.owner_of(owner), self.guid_of(item)) else {
            return Ok(());
        };
        self.set_stored_page(g, shown);
        deferred::send_item_page(self, o, g, flags, shown)
    }
}

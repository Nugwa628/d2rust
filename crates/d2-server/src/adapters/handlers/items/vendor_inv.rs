//! The vendor world of the wired host on the game's one inventory model:
//! [`InvVendors`] wraps the interaction wiring's vendor world
//! ([`VendorDesk`] over the economy) and answers the player-inventory calls of
//! the vendor trade from the inventory parts the item-move handlers use
//! ([`InvParts`], `d2_sim::wiring::inventory`): an item placed by C→S 0x18
//! is the player's at a vendor, and an item sold leaves that inventory.
//! Every other call goes to the wrapped world unchanged. It is the vendor
//! side of the server's item handling, next to the item moves (`super::moves`)
//! and the cube (`super::cube_world`).
//!
//! Wired to the model: `has_cursor_item`, `owns_item`, `in_inventory`,
//! `equipped_items` (grid 0), `place_in_backpack`, `remove_stored`
//! (unlink, then the free), `copy_item` (`InvDesk::copy_of`),
//! `take_from_cursor` (`InvDesk::take_cursor`), and the buy / sell
//! seams: `find_tome`, `add_to_tome`, `find_partial_stack`, `can_belt`,
//! `put_in_belt`, `equip_ammo` (the equip try), `lower_book_skill`,
//! `unequip` (by item), `has_filled_sockets` and the price's socket
//! list.
//!
//! A host without inventory parts sees an empty inventory: nothing is
//! owned, placement fails (the inventory wiring's reading for a unit
//! without an inventory).

use d2_sim::items::inventory::{ty::WEAP, InvWorld};
use d2_sim::items::moves::{
    deferred, ground, handlers, stat as istat, InventoryOps, MovePending, MoveUnits, Owner,
};
use d2_sim::rng::Seed;
use d2_sim::units::lifecycle::LifecycleHooks;
use d2_sim::units::UnitId;
use d2_sim::wiring::economy::QuestRest;
use d2_sim::wiring::interaction::{NpcRest, PlayerQuestsRef, VendorDesk, VendorRest};
use d2_sim::wiring::inventory::{InvDesk, InvError, InvState};
use d2_sim::world::vendors::price::PriceItem;
use d2_sim::world::vendors::{NpcLink, Transaction, VendorWorld};

use super::moves::{take_sent, InvParts, MoveRest};

/// The interaction wiring's vendor world with the player inventories of
/// `inv`. Messages the inventory rules queue (`MovePending::send`) are
/// collected in `sent` (receiving unit, bytes), in call order.
pub struct InvVendors<'i, 'v, 'd, 'a, H, R> {
    pub inner: VendorDesk<'v, 'd, 'a, H, R>,
    pub inv: Option<&'i mut InvParts>,
    pub sent: Vec<(UnitId, Vec<u8>)>,
}

impl<'i, 'v, 'd, 'a, H, R> InvVendors<'i, 'v, 'd, 'a, H, R>
where
    H: LifecycleHooks,
    R: NpcRest + VendorRest + QuestRest + PlayerQuestsRef,
{
    pub fn new(inner: VendorDesk<'v, 'd, 'a, H, R>, inv: Option<&'i mut InvParts>) -> Self {
        Self {
            inner,
            inv,
            sent: Vec::new(),
        }
    }

    fn state(&self) -> Option<&InvState> {
        self.inv.as_deref().map(|p| &p.state)
    }

    /// Runs `f` on the inventory desk over the wrapped world's economy;
    /// what it sent is collected. `None` without inventory parts.
    fn with_desk<T>(
        &mut self,
        f: impl FnOnce(&mut InvDesk<'_, 'a, H, dyn MoveRest + Send + Sync>) -> T,
    ) -> Option<T> {
        let parts = self.inv.as_deref_mut()?;
        let mut d = parts.desk(&mut *self.inner.desk.econ);
        let out = f(&mut d);
        let sent = take_sent(&mut d);
        self.sent
            .extend(sent.into_iter().filter_map(|(u, b)| Some((u?, b))));
        Some(out)
    }
}

/// A character without start items has no inventory yet (`world.rs` adds
/// it with the first item, D2RUST's own): the buy's equip and auto-place need it.
fn ensure_inventory<H: LifecycleHooks>(
    d: &mut InvDesk<'_, '_, H, dyn MoveRest + Send + Sync>,
    player: UnitId,
) {
    if let Some(kind) = d.kind_of(player) {
        let guid = d.guid_of(player);
        d.state.add_inventory(player, kind, guid);
    }
}

impl<H, R> NpcLink for InvVendors<'_, '_, '_, '_, H, R>
where
    H: LifecycleHooks,
    R: NpcRest + VendorRest + QuestRest + PlayerQuestsRef,
{
    fn npc_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.inner.npc_by_guid(guid)
    }
    fn npc_class(&self, npc: UnitId) -> u16 {
        self.inner.npc_class(npc)
    }
    fn is_interact_unit(&self, player: UnitId, npc: UnitId) -> bool {
        self.inner.is_interact_unit(player, npc)
    }
    fn interaction_empty(&self, npc: UnitId) -> bool {
        self.inner.interaction_empty(npc)
    }
    fn hire_list_made(&self, class: u16) -> bool {
        self.inner.hire_list_made(class)
    }
    fn set_hire_list_made(&mut self, class: u16) {
        self.inner.set_hire_list_made(class)
    }
    fn make_hire_list(&mut self, class: u16, seed: &mut Seed) {
        self.inner.make_hire_list(class, seed)
    }
}

impl<H, R> VendorWorld for InvVendors<'_, '_, '_, '_, H, R>
where
    H: LifecycleHooks,
    R: NpcRest + VendorRest + QuestRest + PlayerQuestsRef,
{
    fn difficulty(&self) -> u8 {
        self.inner.difficulty()
    }
    fn expansion(&self) -> bool {
        self.inner.expansion()
    }
    fn item_format(&self) -> u16 {
        self.inner.item_format()
    }
    fn rules(&self) -> &d2_sim::rules::Ruleset {
        self.inner.rules()
    }
    fn game_type(&self) -> u8 {
        self.inner.game_type()
    }
    fn guid(&self, unit: UnitId) -> u32 {
        self.inner.guid(unit)
    }
    fn player_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.inner.player_by_guid(guid)
    }
    fn item_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.inner.item_by_guid(guid)
    }
    fn stat(&self, unit: UnitId, id: u16, layer: u16) -> i32 {
        self.inner.stat(unit, id, layer)
    }
    fn base_stat(&self, unit: UnitId, id: u16, layer: u16) -> i32 {
        self.inner.base_stat(unit, id, layer)
    }
    fn set_stat(&mut self, unit: UnitId, id: u16, layer: u16, value: i32) {
        self.inner.set_stat(unit, id, layer, value)
    }
    fn quest_slot(&self, player: UnitId, difficulty: u8, slot: u32) -> u16 {
        self.inner.quest_slot(player, difficulty, slot)
    }
    fn players_in_level(&self, level: u16) -> i32 {
        self.inner.players_in_level(level)
    }
    fn player_level_id(&self, player: UnitId) -> u16 {
        self.inner.player_level_id(player)
    }
    fn town_entered(&mut self, player: UnitId, level: u16) {
        self.inner.town_entered(player, level)
    }
    fn gold_cap(&self, player: UnitId) -> i32 {
        self.inner.gold_cap(player)
    }
    fn stash_cap(&self, player: UnitId) -> i32 {
        self.inner.stash_cap(player)
    }
    fn drop_gold(&mut self, player: UnitId, amount: i32) {
        self.inner.drop_gold(player, amount)
    }
    fn last_bought(&self, player: UnitId) -> u32 {
        self.inner.last_bought(player)
    }
    fn set_last_bought(&mut self, player: UnitId, guid: u32) {
        self.inner.set_last_bought(player, guid)
    }
    /// The inventory's cursor item.
    fn has_cursor_item(&self, player: UnitId) -> bool {
        self.state().is_some_and(|s| s.cursor_of(player).is_some())
    }
    fn create_item(
        &mut self,
        npc_class: u16,
        record: usize,
        quality: u8,
        ilvl: i32,
    ) -> Option<UnitId> {
        self.inner.create_item(npc_class, record, quality, ilvl)
    }
    /// Copy the item on the inventory model: buy and sell pass fillers
    /// 1 (the socket fillers are copied too). Without inventory parts:
    /// the wrapped world's.
    fn copy_item(&mut self, item: UnitId) -> Option<UnitId> {
        if self.inv.is_none() {
            return self.inner.copy_item(item);
        }
        self.with_desk(|d| d.copy_of(item, true)).flatten()
    }
    fn destroy_item(&mut self, item: UnitId) {
        self.inner.destroy_item(item)
    }
    fn item_record(&self, item: UnitId) -> usize {
        self.inner.item_record(item)
    }
    fn item_quality(&self, item: UnitId) -> u8 {
        self.inner.item_quality(item)
    }
    fn item_file_index(&self, item: UnitId) -> i32 {
        self.inner.item_file_index(item)
    }
    fn item_flags(&self, item: UnitId) -> u32 {
        self.inner.item_flags(item)
    }
    fn set_item_flags(&mut self, item: UnitId, flags: u32) {
        self.inner.set_item_flags(item, flags)
    }
    fn or_unit_flags(&mut self, item: UnitId, bits: u32) {
        self.inner.or_unit_flags(item, bits)
    }
    fn item_mode(&self, item: UnitId) -> u32 {
        self.inner.item_mode(item)
    }
    fn set_item_mode(&mut self, item: UnitId, mode: u32) {
        self.inner.set_item_mode(item, mode)
    }
    fn set_item_page(&mut self, item: UnitId, page: u8) {
        self.inner.set_item_page(item, page)
    }
    fn recharge(&mut self, item: UnitId) {
        self.inner.recharge(item)
    }
    fn repair_broken(&mut self, item: UnitId) {
        self.with_desk(|d| d.repair_broken_item(item));
        self.inner.repair_broken(item)
    }
    fn identify(&mut self, item: UnitId) {
        self.inner.identify(item)
    }
    fn send_item_stat(&mut self, player: UnitId, item: UnitId, stat: u16) {
        self.inner.send_item_stat(player, item, stat)
    }
    fn send_transaction(&mut self, player: UnitId, t: Transaction) {
        self.inner.send_transaction(player, t)
    }
    fn new_store_inventory(&mut self, npc_class: u16, npc: Option<UnitId>) {
        self.inner.new_store_inventory(npc_class, npc)
    }
    /// Place the item on the NPC's inventory (`InvDesk::store_place`) when
    /// the call has one; else the wrapped world's.
    fn place_in_store(&mut self, npc_class: u16, item: UnitId) -> bool {
        let Some(npc) = self
            .inner
            .record_npc(npc_class)
            .filter(|_| self.inv.is_some())
        else {
            return self.inner.place_in_store(npc_class, item);
        };
        self.with_desk(|d| d.store_place(npc, item))
            .unwrap_or(false)
    }
    /// The store item leaves the NPC grid, then the wrapped
    /// world's removal.
    fn remove_store_item(&mut self, npc_class: u16, item: UnitId) {
        self.with_desk(|d| d.store_unlink(item));
        self.inner.remove_store_item(npc_class, item)
    }
    /// A purchase takes the store item out of the NPC grid, then the
    /// wrapped world's take (the action 12 queue).
    fn take_from_store(&mut self, npc_class: u16, item: UnitId) {
        self.with_desk(|d| d.store_unlink(item));
        self.inner.take_from_store(npc_class, item)
    }
    /// A gamble list item is placed the same way as a store item: mode
    /// 0 on the NPC's page-0 grid.
    fn place_in_gamble(&mut self, npc_class: u16, player: u32, item: UnitId) -> bool {
        let Some(npc) = self
            .inner
            .record_npc(npc_class)
            .filter(|_| self.inv.is_some())
        else {
            return self.inner.place_in_gamble(npc_class, player, item);
        };
        self.with_desk(|d| d.gamble_place(npc, player, item))
            .unwrap_or(false)
    }
    /// The item leaves the player's gamble node (found by its contents:
    /// a town refresh drops lists without the NPC unit), then the wrapped
    /// world's removal.
    fn remove_gamble_item(&mut self, npc_class: u16, player: u32, item: UnitId) {
        self.with_desk(|d| d.gamble_unlink(player, item));
        self.inner.remove_gamble_item(npc_class, player, item)
    }
    fn refresh_npc_inventory(&mut self, npc: UnitId) {
        self.inner.refresh_npc_inventory(npc)
    }
    fn add_trade_inventory(&mut self, npc_class: u16, item: UnitId) {
        self.inner.add_trade_inventory(npc_class, item)
    }

    // ---- player inventories on the model ----------------------------

    /// The player's cursor item or an item of its item list (any page,
    /// body location or belt). A ground item is in neither, so its sale
    /// is refused here (code 11, result 3) before any copy.
    fn owns_item(&self, player: UnitId, item: UnitId) -> bool {
        self.state().is_some_and(|s| s.holds(player, item))
    }
    /// The "in the player's inventory" test of the sell checks: the
    /// same reading as [`Self::owns_item`].
    fn in_inventory(&self, player: UnitId, item: UnitId) -> bool {
        self.state().is_some_and(|s| s.holds(player, item))
    }
    /// The items of grid 0 over the 13 body locations, in location
    /// order.
    fn equipped_items(&self, player: UnitId) -> Vec<UnitId> {
        self.state().map_or_else(Vec::new, |s| s.body_items(player))
    }
    /// The first page-0 tome (type 18) whose spell is the scroll's and
    /// whose quantity is below its max stack, with its free space (the
    /// "tome for P" test of the ground pick-up, [`ground::tome_for`]).
    fn find_tome(&mut self, player: UnitId, scroll: UnitId) -> Option<(UnitId, i32)> {
        if self.inv.is_none() {
            return self.inner.find_tome(player, scroll);
        }
        self.with_desk(|d| {
            let p = d.owner_of(player)?;
            let t = ground::tome_for(d, p, d.guid_of(scroll))?;
            let free = d.max_stack(t) - d.stat(Owner::item(t), istat::QUANTITY);
            Some((d.item_unit(t)?, free))
        })
        .flatten()
    }
    /// Tome quantity += k.
    ///
    /// The announcement is S→C 0x3E for stat 70 to the buyer, then the
    /// tome's skill count changes by `k`.
    fn add_to_tome(&mut self, tome: UnitId, k: i32) {
        if self.inv.is_none() {
            return self.inner.add_to_tome(tome, k);
        }
        self.with_desk(|d| {
            let g = d.guid_of(tome);
            let o = Owner::item(g);
            let q = d.stat(o, istat::QUANTITY);
            d.set_stat(o, istat::QUANTITY, q.wrapping_add(k));
            let Some(player) = d.item_owner(g) else {
                return;
            };
            d.send_item_stat(player, g, istat::QUANTITY);
            d.book_count_changed(player, g, k);
        });
    }
    /// A stack of the same item on the player below its max stack, the
    /// equipped items first (location order), then page 0 (grid list
    /// order), with its free space.
    fn find_partial_stack(&mut self, player: UnitId, item: UnitId) -> Option<(UnitId, i32)> {
        if self.inv.is_none() {
            return self.inner.find_partial_stack(player, item);
        }
        self.with_desk(|d| {
            let p = d.owner_of(player)?;
            let g = d.guid_of(item);
            let mut cands = d.body_items(p);
            cands.extend(d.page_items(p, 0));
            cands.into_iter().find_map(|c| {
                let free = d.max_stack(c) - d.stat(Owner::item(c), istat::QUANTITY);
                (c != g && d.stack_test(g, c) && free > 0).then(|| Some((d.item_unit(c)?, free)))?
            })
        })
        .flatten()
    }
    /// The auto-belt gate: true for every item (the belt slot search
    /// decides whether the item fits).
    fn can_belt(&mut self, player: UnitId, item: UnitId) -> bool {
        if self.inv.is_none() {
            return self.inner.can_belt(player, item);
        }
        true
    }
    /// Put the item in the belt (the 0x23 body): the free belt slot
    /// is picked first, then the item moves there.
    fn put_in_belt(&mut self, player: UnitId, item: UnitId) -> bool {
        if self.inv.is_none() {
            return self.inner.put_in_belt(player, item);
        }
        self.with_desk(|d| {
            let Some(p) = d.owner_of(player) else {
                return false;
            };
            let g = d.guid_of(item);
            let Some(slot) = d.belt_free_slot(p, g) else {
                return false;
            };
            handlers::to_belt(d, p, g, u32::from(slot)) == d2_sim::items::moves::Outcome::DONE
        })
        .unwrap_or(false)
    }
    /// The equip try for a bought ammo item: by the weapon in use and
    /// its class, then the equip rules (skip 0).
    fn equip_ammo(&mut self, player: UnitId, item: UnitId) -> bool {
        if self.inv.is_none() {
            return self.inner.equip_ammo(player, item);
        }
        self.with_desk(|d| {
            ensure_inventory(d, player);
            let Some(p) = d.owner_of(player) else {
                return false;
            };
            let g = d.guid_of(item);
            let code = d.code(g);
            let quiver = |c: &[u8; 4]| c == b"cqv " || c == b"aqv ";
            let wclass = |w: u32| {
                d.item(d.item_unit(w)?)
                    .and_then(|r| d.tables.item(r.record))
                    .map(|r| r.wclass)
            };
            let try_it = match d.weapon_in_use(p) {
                None => !quiver(&code),
                Some(w) => {
                    let h = wclass(w);
                    if d.is_type(g, WEAP as u16) {
                        false
                    } else if &code == b"cqv " {
                        h == Some(*b"xbw ")
                    } else if &code == b"aqv " {
                        h == Some(*b"bow ")
                    } else {
                        true
                    }
                }
            };
            try_it && d.equip_picked(p, g)
        })
        .unwrap_or(false)
    }
    /// Auto-place on the player's inventory: placement with a free
    /// position and "send" (the purchased copy appears as 0x9C action 4
    /// the next frame).
    fn place_in_backpack(&mut self, player: UnitId, item: UnitId) -> bool {
        // "page := 0 and auto-place": the store copy still
        // names its store page.
        self.with_desk(|d| {
            let g = d.guid_of(item);
            MoveUnits::set_page(d, g, d2_sim::items::inventory::page::INVENTORY);
            ensure_inventory(d, player);
            d.place(player, item, (0, 0), true, true)
        })
        .unwrap_or(false)
    }
    /// Take the item off the cursor on the inventory model (`InvDesk::take_cursor`).
    /// Without inventory parts: the wrapped world's.
    fn take_from_cursor(&mut self, player: UnitId, item: UnitId) -> bool {
        if self.inv.is_none() {
            return self.inner.take_from_cursor(player, item);
        }
        self.with_desk(|d| d.take_cursor(player, item))
            .unwrap_or(false)
    }
    /// Lower the book skill on the equipment rules
    /// ([`InvDesk::run_item_skill_remove`]).
    fn lower_book_skill(&mut self, player: UnitId, item: UnitId, n: i32) {
        if self.inv.is_none() {
            return self.inner.lower_book_skill(player, item, n);
        }
        self.with_desk(|d| d.run_item_skill_remove(player, item, n));
    }
    /// Sell path, mode 0: the item's stored page := its page, S→C 0x9D
    /// action 5 with command flags 0x20, then the item is unlinked from the
    /// player's inventory and freed.
    fn remove_stored(&mut self, player: UnitId, item: UnitId) {
        self.with_desk(|d| {
            let g = d.guid_of(item);
            if let Some(o) = d.owner_of(player) {
                let page = MoveUnits::page(d, g);
                MoveUnits::set_stored_page(d, g, page);
                if let Err(e) = deferred::send_item_page(d, o, g, 0x20, page) {
                    d.state.errors.push(InvError::Move(e));
                }
            }
            d.remove(player, item);
            d.free(item);
        });
    }
    /// Unequip by item (other sell modes): the 0x1C body at the item's
    /// body location; true when the item left the body.
    fn unequip(&mut self, player: UnitId, item: UnitId) -> bool {
        if self.inv.is_none() {
            return self.inner.unequip(player, item);
        }
        self.with_desk(|d| {
            let Some(p) = d.owner_of(player) else {
                return false;
            };
            let g = d.guid_of(item);
            if d.mode(g) != 1 {
                return false;
            }
            let loc = d.body_loc(g);
            let ok = matches!(handlers::remove_body_item(d, p, u16::from(loc)), Ok(0));
            ok && d.mode(g) != 1
        })
        .unwrap_or(false)
    }
    /// The item's own inventory holds a filler.
    fn has_filled_sockets(&self, item: UnitId) -> bool {
        match self.state() {
            Some(s) => s
                .inventories
                .get(&item)
                .is_some_and(|i| !i.items().is_empty()),
            None => self.inner.has_filled_sockets(item),
        }
    }
    /// The sockets of the price: the records of the items of the item's
    /// own inventory, list order, from the inventory model.
    fn price_item(&self, item: UnitId) -> Option<PriceItem> {
        let mut p = self.inner.price_item(item)?;
        if let Some(s) = self.state() {
            p.sockets = s
                .inventories
                .get(&item)
                .map(|i| {
                    i.items()
                        .iter()
                        .filter_map(|f| s.items.get(f).map(|d| d.record))
                        .collect()
                })
                .unwrap_or_default();
        }
        Some(p)
    }
}

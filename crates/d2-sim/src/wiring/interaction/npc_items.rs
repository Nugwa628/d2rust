//! The item calls of the NPC services (`NpcWorld::cursor_item`,
//! `item_facts`, `remove_cursor_item`, `create_imbued`, `place_or_drop`)
//! on the game's one inventory model and the item store, instead of the
//! rest's refusals. [`NpcInventory`] is the host's inventory as a desk
//! call sees it (the pattern of
//! [`crate::wiring::economy::QuestInventory`]); [`NpcInv`] is it on an
//! [`InvDesk`]. Without one the desk keeps answering from the rest.
//!
//! D2RUST's own, unverified: `put_back` leaves the refused item on the
//! cursor (the put-back routine is not settled);
//! `nameable` is never set.

use super::Desk;
use crate::items::inventory::InvTables;
use crate::items::req;
use crate::items::{flag, ItemRequest};
use crate::units::lifecycle::LifecycleHooks;
use crate::units::UnitId;
use crate::wiring::economy::{Economy, ItemSpawn};
use crate::wiring::inventory::{InvDesk, InvRest, InvState};
use crate::world::npc::{ImbueMods, ItemFacts};

/// The host's inventory model for an NPC service call.
pub trait NpcInventory<H> {
    /// The player's cursor item.
    fn cursor_item(&self, player: UnitId) -> Option<UnitId>;
    /// The player's item list, for the quests' held-item tests of the NPC
    /// chat.
    fn items_of(&self, player: UnitId) -> Vec<UnitId>;
    /// The item owns socketed items (its own inventory is not empty).
    fn has_fillers(&self, item: UnitId) -> bool;
    /// Takes `item` off the player's cursor; false when it is not there.
    fn remove_cursor_item(
        &mut self,
        econ: &mut Economy<'_, H>,
        player: UnitId,
        item: UnitId,
    ) -> bool;
    /// Places `item` in the player's backpack at a free spot; false when
    /// there is none.
    fn place(&mut self, econ: &mut Economy<'_, H>, player: UnitId, item: UnitId) -> bool;
    /// Places a store item in the NPC's grid; false when its page has no
    /// room.
    fn store_place(&mut self, econ: &mut Economy<'_, H>, npc: UnitId, item: UnitId) -> bool;
    /// Unlinks a store item from the NPC grid that holds it.
    fn store_unlink(&mut self, econ: &mut Economy<'_, H>, item: UnitId) -> bool;
    /// Places a gamble list item in the (NPC, player) node's inventory.
    fn gamble_place(
        &mut self,
        econ: &mut Economy<'_, H>,
        npc: UnitId,
        player: u32,
        item: UnitId,
    ) -> bool;
    /// Unlinks a gamble list item from the player's node that holds it.
    fn gamble_unlink(&mut self, econ: &mut Economy<'_, H>, player: u32, item: UnitId) -> bool;
}

/// [`NpcInventory`] on an [`InvDesk`] for each call.
pub struct NpcInv<'d, R: ?Sized> {
    pub tables: &'d InvTables,
    pub state: &'d mut InvState,
    pub rest: &'d mut R,
}

impl<H: LifecycleHooks, R: InvRest + ?Sized> NpcInventory<H> for NpcInv<'_, R> {
    fn store_place(&mut self, econ: &mut Economy<'_, H>, npc: UnitId, item: UnitId) -> bool {
        InvDesk::new(econ, self.tables, self.state, self.rest).store_place(npc, item)
    }
    fn store_unlink(&mut self, econ: &mut Economy<'_, H>, item: UnitId) -> bool {
        InvDesk::new(econ, self.tables, self.state, self.rest).store_unlink(item)
    }
    fn gamble_place(
        &mut self,
        econ: &mut Economy<'_, H>,
        npc: UnitId,
        player: u32,
        item: UnitId,
    ) -> bool {
        InvDesk::new(econ, self.tables, self.state, self.rest).gamble_place(npc, player, item)
    }
    fn gamble_unlink(&mut self, econ: &mut Economy<'_, H>, player: u32, item: UnitId) -> bool {
        InvDesk::new(econ, self.tables, self.state, self.rest).gamble_unlink(player, item)
    }
    fn cursor_item(&self, player: UnitId) -> Option<UnitId> {
        self.state.cursor_of(player)
    }
    fn items_of(&self, player: UnitId) -> Vec<UnitId> {
        self.state.items_of(player)
    }
    fn has_fillers(&self, item: UnitId) -> bool {
        !self.state.fillers(item).is_empty()
    }
    fn remove_cursor_item(
        &mut self,
        econ: &mut Economy<'_, H>,
        player: UnitId,
        item: UnitId,
    ) -> bool {
        InvDesk::new(econ, self.tables, self.state, self.rest).take_cursor_keep(player, item)
    }
    fn place(&mut self, econ: &mut Economy<'_, H>, player: UnitId, item: UnitId) -> bool {
        InvDesk::new(econ, self.tables, self.state, self.rest).place(
            player,
            item,
            (0, 0),
            true,
            true,
        )
    }
}

impl<H: LifecycleHooks, R> Desk<'_, '_, H, R> {
    /// The item facts the NPC services' predicates read, from the item
    /// store and the item tables.
    pub(super) fn facts_of(&self, item: UnitId) -> Option<ItemFacts> {
        let i = self.econ.items.get(item)?;
        let t = self.econ.tables;
        let rec = t.item(i.record)?;
        let ty = t.itemtype(rec.type_);
        Some(ItemFacts {
            gold: rec.type_ == crate::items::ty::GOLD as i16,
            flags: i.flags,
            bitfield1: rec.bitfield1,
            throwable: ty.is_some_and(|t| t.throwable != 0),
            unit_flags: self.econ.units.get(item).map_or(0, |r| r.flags2),
            quest: rec.quest != 0,
            code: rec.code,
            has_socketed: self.inv.as_ref().is_some_and(|v| v.has_fillers(item)),
            quality: i.quality,
            item_type: rec.type_.max(0) as u16,
            max_sockets: 0,
            stat194: self.econ.stats.unit_total(item, 194, 0),
            nameable: false,
            ethereal: i.flags & flag::ETHEREAL != 0,
        })
    }

    /// Creates the imbue result: a request from the input's record with
    /// the mods, created in the cursor mode.
    pub(super) fn imbue_from(&mut self, input: UnitId, mods: &ImbueMods) -> Option<UnitId> {
        let i = self.econ.items.get(input)?;
        let mut rq = ItemRequest {
            item: i.record as i32,
            ilvl: mods.level as i32,
            format: mods.format,
            quality: mods.quality,
            quantity: 1,
            flags2: mods.flags & (req::ALWAYS_ETHEREAL | req::NEVER_ETHEREAL | req::STAFFMODS_ILVL),
            ..ItemRequest::default()
        };
        let mode = self.econ.units.get(input).map_or(4, |r| r.mode);
        match self.econ.create_item(
            &mut rq,
            false,
            ItemSpawn {
                room: None,
                mode,
                init_flags: 1,
            },
        ) {
            Ok(u) => Some(u),
            Err(e) => {
                self.state.errors.push(super::InteractionError::Economy(e));
                None
            }
        }
    }
}

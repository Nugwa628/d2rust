//! [`MoveUnits`] on [`InvDesk`]: unit record fields (class, flags, update
//! bits, mode), stats from the stat lists, game fields, the item store (flags,
//! page, quality, file index), the inventory tables (code, type,
//! `quest`, `useable`, `component`, `maxstack`) and the state's item data
//! (body location, stored page, command flags, position, owner, expiry,
//! fillers). Positions of non-item units, the socket-filler test and
//! spells are [`InvRest`]'s.

use super::inv_world::STAT_SOCKETS;
use super::{InvDesk, InvRest};
use crate::items::moves::{Guid, MoveUnits, Owner};
use crate::units::lifecycle::LifecycleHooks;
use crate::units::UnitId;

/// Stat 254 `item_extra_stack`: added to `maxstack` for the total max stack.
const STAT_EXTRA_STACK: u16 = 254;
/// Cap of the total max stack.
const MAX_STACK_CAP: i32 = 511;

impl<H: LifecycleHooks, R: InvRest + ?Sized> InvDesk<'_, '_, H, R> {
    fn rec_of(&self, u: Owner) -> Option<&crate::units::record::UnitRecord> {
        self.econ.units.get(self.unit_of(u)?)
    }

    pub(super) fn item_rec(
        &self,
        item: Guid,
    ) -> Option<&crate::items::inventory::tables::InvItemRec> {
        let u = self.item_unit(item)?;
        self.tables.item(self.state.items.get(&u)?.record)
    }

    /// Edits the state's item data and, for the copied fields, their
    /// owner (write-through).
    fn edit(&mut self, item: Guid, f: impl FnOnce(&mut crate::items::inventory::InvItem)) {
        let Some(u) = self.item_unit(item) else {
            return;
        };
        if let Some(d) = self.state.items.get_mut(&u) {
            f(d);
            let (mode, flags, page) = (d.mode, d.flags, d.page);
            if let Some(r) = self.econ.units.get_mut(u) {
                r.mode = u32::from(mode);
            }
            if let Some(it) = self.econ.items.get_mut(u) {
                it.flags = flags;
                it.inv_page = page;
            }
        }
    }

    fn data(&self, item: Guid) -> Option<&crate::items::inventory::InvItem> {
        self.state.items.get(&self.item_unit(item)?)
    }

    fn set_rec(&mut self, u: Owner, f: impl FnOnce(&mut crate::units::record::UnitRecord)) {
        if let Some(id) = self.unit_of(u) {
            if let Some(r) = self.econ.units.get_mut(id) {
                f(r);
            }
        }
    }

    fn stat_unit(&self, u: Owner) -> Option<UnitId> {
        self.unit_of(u)
    }
}

impl<H: LifecycleHooks, R: InvRest + ?Sized> MoveUnits for InvDesk<'_, '_, H, R> {
    fn unit_exists(&self, u: Owner) -> bool {
        self.unit_of(u).is_some()
    }
    fn unit_class(&self, u: Owner) -> u32 {
        self.rec_of(u).map_or(0, |r| r.class)
    }
    fn unit_mode(&self, u: Owner) -> u32 {
        self.rec_of(u).map_or(0, |r| r.mode)
    }
    /// Items: the item data x, y; other units: seam.
    fn pos(&self, u: Owner) -> (i32, i32) {
        if u.ty == Owner::ITEM {
            return self.data(u.guid).map_or((0, 0), |d| (d.x, d.y));
        }
        self.rest.pos(u)
    }
    fn set_pos(&mut self, u: Owner, x: i32, y: i32) {
        if u.ty == Owner::ITEM {
            self.edit(u.guid, |d| {
                d.x = x;
                d.y = y;
            });
            return;
        }
        self.rest.set_pos(u, x, y)
    }
    /// The unit record's flags.
    fn unit_flags(&self, u: Owner) -> u32 {
        self.rec_of(u).map_or(0, |r| r.flags)
    }
    fn set_unit_flags(&mut self, u: Owner, v: u32) {
        self.set_rec(u, |r| r.flags = v);
    }
    /// The unit record's update bits (`flags2`).
    fn update_bits(&self, u: Owner) -> u32 {
        self.rec_of(u).map_or(0, |r| r.flags2)
    }
    fn set_update_bits(&mut self, u: Owner, v: u32) {
        self.set_rec(u, |r| r.flags2 = v);
    }
    fn stat(&self, u: Owner, id: u16) -> i32 {
        self.stat_unit(u)
            .map_or(0, |s| self.econ.stats.unit_total(s, id, 0))
    }
    fn set_stat(&mut self, u: Owner, id: u16, v: i32) {
        if let Some(s) = self.stat_unit(u) {
            self.econ.stats.unit_set(&mut *self.econ.hooks, s, id, v, 0);
        }
    }
    fn expansion(&self) -> bool {
        self.econ.fields.expansion
    }
    fn frame(&self) -> i32 {
        self.econ.game.frame
    }
    fn tick_rate(&self) -> crate::tick::TickRate {
        self.econ.game.tick_rate
    }

    // ---- item data -------------------------------------------------------

    fn mode(&self, item: Guid) -> u8 {
        self.data(item).map_or(0, |d| d.mode)
    }
    fn set_mode(&mut self, item: Guid, m: u8) {
        self.edit(item, |d| d.mode = m);
    }
    fn page(&self, item: Guid) -> u8 {
        self.data(item).map_or(0xFF, |d| d.page)
    }
    fn set_page(&mut self, item: Guid, p: u8) {
        self.edit(item, |d| d.page = p);
    }
    fn stored_page(&self, item: Guid) -> u8 {
        self.data(item).map_or(0, |d| d.stored_page)
    }
    fn set_stored_page(&mut self, item: Guid, p: u8) {
        self.edit(item, |d| d.stored_page = p);
    }
    fn body_loc(&self, item: Guid) -> u8 {
        self.data(item).map_or(0, |d| d.body_loc)
    }
    fn set_body_loc(&mut self, item: Guid, loc: u8) {
        self.edit(item, |d| d.body_loc = loc);
    }
    fn cmd_flags(&self, item: Guid) -> u32 {
        self.data(item).map_or(0, |d| d.cmd_flags)
    }
    fn set_cmd_flags(&mut self, item: Guid, v: u32) {
        self.edit(item, |d| d.cmd_flags = v);
    }
    fn item_flags(&self, item: Guid) -> u32 {
        self.data(item).map_or(0, |d| d.flags)
    }
    fn set_item_flags(&mut self, item: Guid, v: u32) {
        self.edit(item, |d| d.flags = v);
    }
    fn set_expiry(&mut self, item: Guid, frame: i32) {
        if let Some(u) = self.item_unit(item) {
            self.state.expiry.insert(u, frame);
        }
    }
    /// The unit owning the item's inventory.
    fn item_owner(&self, item: Guid) -> Option<Owner> {
        self.owner_of(self.data(item)?.inv?)
    }
    fn is_type(&self, item: Guid, ty: u16) -> bool {
        self.data(item)
            .is_some_and(|d| self.tables.is_type(d.record, ty as i16))
    }
    fn primary_type(&self, item: Guid) -> u16 {
        self.item_rec(item).map_or(0, |r| r.type_ as u16)
    }
    fn stackable(&self, item: Guid) -> bool {
        self.item_rec(item).is_some_and(|r| r.stackable != 0)
    }
    /// Itemtypes `autostack` of the primary type.
    fn autostack(&self, item: Guid) -> bool {
        self.data(item)
            .and_then(|d| self.econ.tables.itype_of(d.record))
            .is_some_and(|t| t.autostack != 0)
    }
    fn quiver(&self, item: Guid) -> bool {
        self.data(item)
            .and_then(|d| self.tables.itype_of(d.record))
            .is_some_and(|t| t.quiver != 0)
    }
    fn code(&self, item: Guid) -> [u8; 4] {
        self.item_rec(item).map_or([0; 4], |r| r.code)
    }
    fn quality(&self, item: Guid) -> u8 {
        self.item_unit(item)
            .and_then(|u| self.econ.items.get(u))
            .map_or(0, |i| i.quality)
    }
    fn file_index(&self, item: Guid) -> i32 {
        self.item_unit(item)
            .and_then(|u| self.econ.items.get(u))
            .map_or(-1, |i| i.file_index)
    }
    fn quest(&self, item: Guid) -> u8 {
        self.item_rec(item).map_or(0, |r| r.quest)
    }
    fn useable(&self, item: Guid) -> bool {
        self.item_rec(item).is_some_and(|r| r.useable != 0)
    }
    fn component(&self, item: Guid) -> u8 {
        self.item_rec(item).map_or(0, |r| r.component)
    }
    /// `maxstack` + stat 254, capped at 511.
    fn max_stack(&self, item: Guid) -> i32 {
        let Some(r) = self.item_rec(item) else {
            return 0;
        };
        let extra = self.stat(Owner::item(item), STAT_EXTRA_STACK);
        (r.maxstack as i32).wrapping_add(extra).min(MAX_STACK_CAP)
    }
    fn socket_filled(&self, item: Guid) -> bool {
        self.rest.socket_filled(item)
    }
    /// A gem (20), rune (74) or jewel (58) is a filler. A chipped ruby, El
    /// and a magic jewel each go into a 3-socket shield with C→S 0x28; a
    /// ring is not offered (the client, which runs the same test, sends a
    /// swap 0x1F instead, also onto free sockets).
    fn socket_filler(&self, item: Guid) -> bool {
        use crate::items::ty;
        [ty::GEM, ty::RUNE, ty::JEWL]
            .into_iter()
            .any(|t| self.is_type(item, t))
            || self.rest.socket_filler(item)
    }
    /// The socket count: stat 194.
    fn sockets(&self, item: Guid) -> i32 {
        self.stat(Owner::item(item), STAT_SOCKETS)
    }
    /// The item's own inventory, in link order.
    fn fillers(&self, item: Guid) -> Vec<Guid> {
        self.item_unit(item)
            .and_then(|u| self.state.inventories.get(&u))
            .map(|i| i.items().iter().map(|&u| self.guid_of(u)).collect())
            .unwrap_or_default()
    }
    /// The item's spell ([`InvDesk::spell_of`]).
    fn spell(&self, item: Guid) -> i32 {
        self.item_unit(item).map_or(0, |u| self.spell_of(u))
    }
}

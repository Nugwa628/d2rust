//! [`InvWorld`] on [`InvDesk`]: item data from the state's copies, unit
//! kinds and acts from the unit records, stats from the stat lists, room
//! removal from the unit lists, item getters from the item store, the
//! interaction from the unit record's interact info; the
//! rest through [`InvRest`] / [`MovePending`].

use super::equip_rules::EquipCall;
use super::{InvDesk, InvRest};
use crate::items::inventory::{
    active_inventory_item, find_free_position, InteractionTarget, InvItem, InvWorld, Inventory,
    UnitKind, UNIT_FLAG_TARGETABLE,
};
use crate::items::moves::{layouts, Guid, Owner};
use crate::units::lifecycle::LifecycleHooks;
use crate::units::{UnitId, UnitType};

/// Stat 91 `item_req_percent`.
const STAT_REQ_PERCENT: u16 = 91;
/// Stat 194 `item_numsockets`.
pub const STAT_SOCKETS: u16 = 194;
/// Ethereal item flag (ethereal items cannot be repaired and take
/// more damage; cube recipes test it).
pub const ETHEREAL: u32 = 0x40_0000;

impl<H: LifecycleHooks, R: InvRest + ?Sized> InvDesk<'_, '_, H, R> {
    fn g(&self, u: UnitId) -> Guid {
        self.guid_of(u)
    }

    fn o(&self, u: UnitId) -> Owner {
        self.owner_of(u).unwrap_or(Owner {
            ty: Owner::NONE,
            guid: u32::MAX,
        })
    }

    /// Item position (x, y): the item data's.
    pub(crate) fn item_pos(&self, u: UnitId) -> (i32, i32) {
        self.state.items.get(&u).map_or((0, 0), |d| (d.x, d.y))
    }
}

impl<H: LifecycleHooks, R: InvRest + ?Sized> InvWorld for InvDesk<'_, '_, H, R> {
    fn expansion(&self) -> bool {
        self.econ.fields.expansion
    }

    fn item(&self, item: UnitId) -> Option<&InvItem> {
        self.state.items.get(&item)
    }
    fn item_mut(&mut self, item: UnitId) -> Option<&mut InvItem> {
        self.state.items.get_mut(&item)
    }
    fn item_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.item_unit(guid)
    }
    fn item_stat(&self, item: UnitId, stat: u16) -> i32 {
        self.econ.stats.unit_total(item, stat, 0)
    }

    /// Unlink on the other inventory (lent out for the call).
    fn unlink_from(&mut self, owner: UnitId, item: UnitId) {
        if let Some(mut inv) = self.state.inventories.remove(&owner) {
            inv.unlink(self, item);
            self.state.inventories.insert(owner, inv);
        }
    }

    /// Room delete notice and collision freed
    /// (seams), then the room list removal.
    fn remove_from_room(&mut self, item: UnitId) {
        let g = self.g(item);
        self.rest.room_delete_notice(g);
        self.econ.hooks.ground_item_lifted(item);
        self.rest.free_collision(g);
        let room = self.econ.game.lists.unit(item).and_then(|e| e.room());
        if let Some(room) = room {
            self.econ.hooks.room_delete_record(self.econ.game, g, room);
        }
        let r = self.econ.game.lists.room_remove(item);
        self.note_list(r);
    }

    fn clear_targetable(&mut self, item: UnitId) {
        if let Some(r) = self.econ.units.get_mut(item) {
            r.flags &= !UNIT_FLAG_TARGETABLE;
        }
    }

    /// Link check: sockets the item when the inventory belongs to an
    /// item (seam), else succeeds.
    fn link_check(&mut self, owner: UnitId, item: UnitId, kind: u8) -> bool {
        match self.econ.units.get(owner).map(|r| r.ty) {
            Some(UnitType::Item) => {
                let (o, i) = (self.g(owner), self.g(item));
                self.rest.socket_link(o, i, kind)
            }
            _ => true,
        }
    }
    fn charm_relink(&mut self, owner: UnitId, item: UnitId) {
        if self.state.equip_rules {
            return self.queue_equip(EquipCall::SkillLink(owner, item));
        }
        let (o, i) = (self.o(owner), self.g(item));
        self.rest.charm_relink(o, i)
    }
    /// Whether the item is active (counts for the owner's stats).
    fn active_item(&self, owner: UnitId, item: UnitId) -> bool {
        active_inventory_item(self, self.tables, item, owner)
    }
    fn stat_refresh(&mut self, owner: UnitId) {
        let o = self.o(owner);
        self.rest.stat_refresh(o)
    }
    /// Whether the item's own inventory holds a filler.
    fn socket_filled(&self, item: UnitId) -> bool {
        self.state
            .inventories
            .get(&item)
            .is_some_and(|inv| !inv.items().is_empty())
            || self.rest.socket_filled(self.g(item))
    }
    /// Run after the call (`items::moves::owner_refresh`).
    fn owner_refresh(&mut self, owner: UnitId) {
        self.state.refresh.push(owner);
    }
    fn inventory_pass(&mut self, owner: UnitId) {
        if self.state.equip_rules {
            return self.queue_equip(EquipCall::Pass(owner));
        }
        let o = self.o(owner);
        self.rest.inventory_pass(o);
        self.link_charms(owner)
    }
    fn trade_hook(&mut self, owner: UnitId, item: UnitId) {
        let (o, i) = (self.o(owner), self.g(item));
        self.rest.trade_hook(o, i)
    }
    fn weapon_in_use_update(&mut self, unit: UnitId) {
        let o = self.o(unit);
        self.rest.weapon_in_use_update(o)
    }
    fn stat_link(&mut self, unit: UnitId, item: UnitId) {
        self.link_item_stats(unit, item);
        let (o, i) = (self.o(unit), self.g(item));
        self.rest.stat_link(o, i)
    }
    fn stat_link_in_use(&mut self, unit: UnitId, item: UnitId, weapon_guid: u32) {
        self.link_item_stats_in_use(unit, item, Some(weapon_guid));
        let (o, i) = (self.o(unit), self.g(item));
        self.rest.stat_link(o, i)
    }
    fn weapon_bookkeeping(&mut self, unit: UnitId, _item: UnitId) {
        if self.state.equip_rules {
            return self.queue_equip(EquipCall::Weapons(unit));
        }
        let o = self.o(unit);
        self.rest.weapon_bookkeeping(o)
    }

    fn unit_kind(&self, unit: UnitId) -> Option<UnitKind> {
        self.kind_of(unit)
    }
    fn unit_stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.econ.stats.unit_total(unit, stat, 0)
    }

    /// Item stat 91 (requirement percent), the item/skill getter = the unit total.
    fn req_percent(&self, item: UnitId) -> i32 {
        self.econ.stats.unit_total(item, STAT_REQ_PERCENT, 0)
    }
    fn item_active_on(&self, item: UnitId, unit: UnitId) -> bool {
        if self.state.link_item_stats {
            // The item's stat list is attached to the wearer
            // (`item_link`).
            return self
                .econ
                .stats
                .unit_list(item)
                .is_some_and(|l| self.econ.stats.attached_unit(l) == Some(unit));
        }
        self.rest.item_active_on(self.g(item), self.o(unit))
    }
    fn own_contribution(&self, item: UnitId, unit: UnitId, stat: u16) -> i32 {
        self.rest.own_contribution(self.g(item), self.o(unit), stat)
    }
    /// The level requirement on the gathered values ([`InvDesk::item_level_requirement`]).
    fn level_requirement(&self, item: UnitId, unit: UnitId) -> i32 {
        self.item_level_requirement(item, Some(unit))
    }

    /// Two-handed ([`InvDesk::is_two_handed`]).
    fn two_handed(&self, item: UnitId) -> bool {
        self.is_two_handed(item)
    }
    /// One-or-two-handed on the tables with the equipment rules
    /// ([`crate::items::inventory::equip::one_or_two_handed_for`]); else
    /// the rest.
    fn one_or_two_handed(&self, unit: UnitId, item: UnitId) -> bool {
        match (self.state.equip_rules, self.class_of(item)) {
            (true, Some(r)) => crate::items::inventory::equip::one_or_two_handed_for(
                self.tables,
                r,
                self.kind_of(unit),
            ),
            _ => self.rest.one_or_two_handed(self.o(unit), self.g(item)),
        }
    }
    /// Ammo type ([`InvDesk::ammo_of`]).
    fn ammo_type(&self, item: UnitId) -> Option<i16> {
        self.ammo_of(item)
    }
    /// "Fits a free position of page 0", read as the free-spot
    /// search on page 0 (on a copy: the search may create the grid).
    fn fits_free_page0(&self, inv: &Inventory, item: UnitId) -> bool {
        let mut copy = inv.clone();
        find_free_position(&mut copy, self, self.tables, item, 0).is_some()
    }

    /// Quality.
    fn quality(&self, item: UnitId) -> u8 {
        self.econ.items.get(item).map_or(0, |i| i.quality)
    }
    /// File index (the item's set / unique / runeword row).
    fn stack_file_index(&self, item: UnitId) -> i32 {
        self.econ.items.get(item).map_or(-1, |i| i.file_index)
    }
    /// Has sockets: stat 194 ≠ 0.
    fn has_sockets(&self, item: UnitId) -> bool {
        self.econ.stats.unit_total(item, STAT_SOCKETS, 0) != 0
    }

    fn has_allowed_location(&self, item: UnitId) -> bool {
        self.rest.has_allowed_location(self.g(item))
    }
    /// Quiver kind with the equipment rules: the primary type's
    /// itemtypes `quiver` (as
    /// [`InvWorld::quiver_type`]); else the rest.
    fn quiver_kind(&self, item: UnitId) -> bool {
        match (self.state.equip_rules, self.class_of(item)) {
            (true, Some(r)) => self.tables.itype_of(r).is_some_and(|t| t.quiver != 0),
            _ => self.rest.quiver_kind(self.g(item)),
        }
    }

    /// The targeting probe: the unit's type, 6 (`Owner::NONE`) for a
    /// missing unit.
    fn targeting_probe(&self, unit: UnitId) -> u32 {
        u32::from(self.o(unit).ty)
    }
    /// S→C 0x3F (code 0xFF, the item's GUID, 0xFFFF) to the player.
    fn queue_untarget(&mut self, player: UnitId, item_guid: u32) {
        let p = self.o(player);
        self.rest
            .send(p, layouts::use_stackable(0xFF, item_guid, 0xFFFF));
    }

    /// The player's interaction (the unit record's interact info,
    /// [`crate::units::record::InteractInfo`]), its unit looked up by
    /// type and GUID; a type outside 0–5 finds none.
    fn interaction(&self, player: UnitId) -> InteractionTarget {
        let Some((ty, guid)) = self.econ.units.get(player).and_then(|r| r.interact.get()) else {
            return InteractionTarget::None;
        };
        let unit = UnitType::ALL
            .get(usize::from(ty))
            .and_then(|&t| self.econ.game.lists.find_unit(t, guid));
        match unit {
            Some(unit) => InteractionTarget::Unit { ty, unit },
            None => InteractionTarget::Missing,
        }
    }
    /// Clears the interaction on the player's unit record.
    fn clear_interaction(&mut self, player: UnitId) {
        if let Some(r) = self.econ.units.get_mut(player) {
            r.interact.reset();
        }
    }
    fn player_data_4c(&self, player: UnitId) -> u32 {
        self.rest.player_data_4c(self.o(player))
    }
    fn player_data_50(&self, player: UnitId) -> u32 {
        self.rest.player_data_50(self.o(player))
    }
    fn npc_talking(&self, npc: UnitId, player: UnitId) -> bool {
        self.rest.npc_talking(self.o(npc), self.o(player))
    }
    fn player_trade_gate(&self, player: UnitId) -> Option<bool> {
        self.rest.player_trade_gate(self.o(player))
    }
    /// Act of both unit records (unit +0x18).
    fn same_act(&self, player: UnitId, item: UnitId) -> bool {
        match (self.econ.units.get(player), self.econ.units.get(item)) {
            (Some(a), Some(b)) => a.act == b.act,
            _ => false,
        }
    }
    /// Whether the item is within `range` subtiles per axis (player position:
    /// seam; item position: item data).
    fn within_range(&self, player: UnitId, item: UnitId, range: i32) -> bool {
        let (px, py) = self.rest.pos(self.o(player));
        let (ix, iy) = self.item_pos(item);
        (i64::from(ix) - i64::from(px)).abs() <= i64::from(range)
            && (i64::from(iy) - i64::from(py)).abs() <= i64::from(range)
    }
}

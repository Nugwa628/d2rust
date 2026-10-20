//! An equipped item's stats reach its wearer: the stat link
//! attaches the item's stat list to the owner (an equip-and-swap with
//! reset 1), the leaving of a body slot detaches it, and a set
//! item runs the set-item update and the set
//! bonuses on the owner.
//!
//! The stat link is not the weapon-in-use link: that only writes the
//! weapon in use. The
//! attach is the item refresh, called from the equip path
//! after the weapon link, which calls the equip-and-swap
//! with reset 1, except reset 0 for a
//! `weap` item that is not the owner's weapon in use (the
//! hand item whose GUID is the weapon in use) when the owner can dual-wield
//! (barbarian, assassin): so a dual-wielder's only weapon is
//! static and its damage-related stats reach the owner, the second one is
//! dynamic. Not modelled, one fact: the refresh skips the equip-and-swap
//! call when item flag 0x100 (broken) is set and its fourth argument is 0.
//!
//! A body removal only
//! detaches the item's list: the deactivation that follows
//! runs its set update (r = 1) only while the list is still on the owner,
//! so the leaving piece never frees the set's owner list; the inventory
//! pass re-updates the
//! pieces still worn, and after the last piece the owner list stays with
//! stat 71 = the set id. The trace check `w1-d2-item-link`
//! covers this.
//!
//! [`InvState::link_item_stats`]: super::InvState::link_item_stats

use super::equip_rules::EquipCall;
use super::{InvDesk, InvError, InvRest};
use crate::items::inventory::equip::{ASSASSIN, BARBARIAN};
use crate::items::inventory::{
    active_inventory_item, body, equip_profile, iflag, node, ty, weapon, InvWorld, UnitKind,
    NO_GUID,
};
use crate::items::set_state::{self, QUALITY_SET};
use crate::items::{props, ListKey};
use crate::units::lifecycle::LifecycleHooks;
use crate::units::UnitId;

impl<H: LifecycleHooks, R: InvRest + ?Sized> InvDesk<'_, '_, H, R> {
    /// The weapon-in-use link (`link`) or unlink of `item` on `owner`'s
    /// inventory (writes only the weapon in use), when the desk holds it.
    /// An inventory lent out to an
    /// inventory function is that function's to update (the level
    /// requirement and the equip-without-cursor run the link themselves).
    pub(super) fn weapon_link_on(&mut self, owner: UnitId, item: UnitId, link: bool) {
        let t = self.tables;
        if let Some(mut inv) = self.state.inventories.remove(&owner) {
            if link {
                weapon::weapon_link(&mut inv, self, t, item);
            } else {
                weapon::weapon_unlink(&mut inv, self, t, item);
            }
            self.state.inventories.insert(owner, inv);
        }
    }

    /// The stat link of `item` onto `owner`; false when the switch is off.
    pub(super) fn link_item_stats(&mut self, owner: UnitId, item: UnitId) -> bool {
        let guid = self
            .state
            .inventories
            .get(&owner)
            .map(|inv| inv.weapon_guid);
        self.link_item_stats_in_use(owner, item, guid)
    }

    /// [`Self::link_item_stats`] with the owner's inventory +0x1C
    /// (`weapon_guid`), given by an inventory function that holds the
    /// inventory; `None`: no inventory.
    pub(super) fn link_item_stats_in_use(
        &mut self,
        owner: UnitId,
        item: UnitId,
        weapon_guid: Option<u32>,
    ) -> bool {
        if !self.state.link_item_stats {
            return false;
        }
        if let Some(l) = self.econ.stats.unit_list(item) {
            // Reset 0 only for a dual-wielder's weapon that
            // is not its weapon in use.
            let dual = matches!(
                self.unit_kind(owner),
                Some(UnitKind::Player {
                    class: BARBARIAN | ASSASSIN
                })
            );
            let in_use = weapon_guid.and_then(|g| self.weapon_in_use(owner, g));
            let weapon = equip_profile(self, self.tables, owner, item).weapon;
            let reset = !(dual && weapon && in_use != Some(item));
            self.econ
                .stats
                .equip(&mut *self.econ.hooks, owner, Some(l), false, reset);
        }
        if InvWorld::quality(self, item) == QUALITY_SET {
            self.set_update(EquipCall::SetLink(owner, item));
        }
        true
    }

    /// The weapon in use on `owner`'s inventory with GUID `guid`: the item
    /// at body location 4 or 5 whose GUID is `guid` and whose type is
    /// `weap`; none for −1.
    fn weapon_in_use(&self, owner: UnitId, guid: u32) -> Option<UnitId> {
        if guid == NO_GUID {
            return None;
        }
        let i = InvWorld::item_by_guid(self, guid)?;
        let d = self.state.items.get(&i)?;
        let hand = d.node_kind == node::BODY
            && matches!(d.body_loc, body::RIGHT_HAND | body::LEFT_HAND)
            && d.inv == Some(owner);
        (hand && self.tables.is_type(d.record, ty::WEAP)).then_some(i)
    }

    /// The inventory unlink's stat part: the
    /// item's list leaves `owner`, with no set update (the deactivation
    /// after it finds the list gone, see the module notes); false when
    /// the switch is off.
    pub(super) fn detach_item_stats(&mut self, owner: UnitId, item: UnitId) -> bool {
        if !self.state.link_item_stats {
            return false;
        }
        if let Some(l) = self.econ.stats.unit_list(item) {
            if self.econ.stats.attached_unit(l) == Some(owner) {
                self.econ.stats.detach(&mut *self.econ.hooks, l);
            }
        }
        true
    }

    /// The unlink of `item` from `owner` (still on the body); false when
    /// the switch is off.
    pub(super) fn unlink_item_stats(&mut self, owner: UnitId, item: UnitId) -> bool {
        if !self.state.link_item_stats {
            return false;
        }
        if let Some(l) = self.econ.stats.unit_list(item) {
            if self.econ.stats.attached_unit(l) == Some(owner) {
                self.econ.stats.detach(&mut *self.econ.hooks, l);
            }
        }
        if InvWorld::quality(self, item) == QUALITY_SET {
            self.set_update(EquipCall::SetUnlink(owner, item));
        }
        true
    }

    /// Inventory pass step 2: each active
    /// inventory item (a charm on page 0) whose list is not linked
    /// to `owner` is linked. The rest of the pass is the rest's (or the
    /// equipment rules'). A charm that leaves page 0 loses its stats in
    /// `charm_unlink`, as every unlink from the inventory does;
    /// the weapon-in-use link and unlink
    /// only set the weapon in use and play no part.
    pub(super) fn link_charms(&mut self, owner: UnitId) {
        if !self.state.link_item_stats {
            return;
        }
        // The owner's inventory is lent out during an inventory call:
        // the sweep then runs at its end.
        if self.state.inventories.contains_key(&owner) {
            self.run_link_charms(owner);
        } else {
            self.queue_equip(EquipCall::Charms(owner));
        }
    }

    /// The sweep of [`Self::link_charms`].
    pub(super) fn run_link_charms(&mut self, owner: UnitId) {
        let items = self
            .state
            .inventories
            .get(&owner)
            .map(|inv| inv.items().to_vec())
            .unwrap_or_default();
        for i in items {
            let linked = self
                .econ
                .stats
                .unit_list(i)
                .is_some_and(|l| self.econ.stats.attached_unit(l) == Some(owner));
            if !linked && active_inventory_item(self, self.tables, i, owner) {
                self.link_item_stats(owner, i);
            }
        }
    }

    /// The set update needs the owner's inventory, which is
    /// lent out during an inventory call: queued then, run at its end.
    fn set_update(&mut self, c: EquipCall) {
        let (EquipCall::SetLink(o, _) | EquipCall::SetUnlink(o, _)) = c else {
            return;
        };
        if self.state.inventories.contains_key(&o) {
            match c {
                EquipCall::SetLink(o, i) => self.run_set_link(o, i),
                EquipCall::SetUnlink(o, i) => self.run_set_unlink(o, i),
                _ => {}
            }
        } else {
            self.queue_equip(c);
        }
    }

    /// The set-item update after the link `(owner, item, 0, 0)`.
    pub(super) fn run_set_link(&mut self, owner: UnitId, item: UnitId) {
        set_state::set_item_update(self, Some(owner), Some(item), 0, 0);
    }

    /// After a set item left the body: its owner list is freed
    /// (`r` = 1), then the pieces still worn count again (the mask no
    /// longer holds the leaving one).
    pub(super) fn run_set_unlink(&mut self, owner: UnitId, item: UnitId) {
        set_state::set_item_update(self, Some(owner), Some(item), 1, 0);
        let others: Vec<UnitId> = self
            .state
            .inventories
            .get(&owner)
            .map(|inv| inv.items().to_vec())
            .unwrap_or_default()
            .into_iter()
            .filter(|&x| x != item)
            .filter(|x| {
                self.state.items.get(x).is_some_and(|d| {
                    d.node_kind == node::BODY && (body::HEAD..=body::GLOVES).contains(&d.body_loc)
                })
            })
            .filter(|&x| InvWorld::quality(self, x) == QUALITY_SET)
            .collect();
        // The leaving piece's data copy can still read as worn when the
        // queue runs: flag it no-equip (0x4000) for the set mask, then
        // restore it.
        let saved = self.state.items.get(&item).map(|d| d.flags);
        if let Some(d) = self.state.items.get_mut(&item) {
            d.flags |= iflag::F4000;
        }
        for x in others {
            set_state::set_item_update(self, Some(owner), Some(x), 0, 1);
        }
        if let (Some(f), Some(d)) = (saved, self.state.items.get_mut(&item)) {
            d.flags = f;
        }
    }

    /// The set bonuses of `item` into the owner's list of
    /// `state`, with the owner's set mask.
    pub(super) fn apply_set_bonuses(&mut self, owner: UnitId, item: UnitId, state: u32) {
        let mask = set_state::SetWorld::set_mask(self, owner, item);
        let key = ListKey {
            state: state as u16,
            flags: 0,
        };
        let r = self.econ.with_item(item, |s| {
            let mut who = s.unit_stats(owner);
            props::set_bonuses(s.tables, s.item, mask, &mut who, key);
        });
        if let Err(e) = r {
            self.state.errors.push(InvError::Economy(e));
        }
        self.sync_in();
    }
}

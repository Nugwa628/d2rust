//! The hireling seams of the 0x61 give on
//! the inventory model: the player's hireling (pet type 7) from
//! the hireling lists the host lends to [`super::InvState::hirelings`],
//! and the swap ([`crate::world::hirelings::items::swap`])
//! with [`MercItems`] as its [`HirelingItems`] provider.
//!
//! A host without hireling lists (`InvState::hirelings` = `None`) keeps
//! the rest's answers ([`super::InvRest`]'s `MovePending` part).

use super::{InvDesk, InvRest};
use crate::items::moves::{Guid, InventoryOps, MovePending, MoveUnits, Owner};
use crate::units::lifecycle::LifecycleHooks;
use crate::units::{UnitId, UnitType};
use crate::world::hirelings::items::{swap, HirelingItems, FLAG_10, FLAG_20};

impl<H: LifecycleHooks, R: InvRest + ?Sized> InvDesk<'_, '_, H, R> {
    /// The player's hireling (pet type 7) on the
    /// lent lists: the first living node's monster. `None` (outer): no
    /// lists lent.
    pub(crate) fn lent_hireling(&self, player: Owner) -> Option<Option<Owner>> {
        let st = self.state.hirelings.as_ref()?;
        let p = self.unit_of(player);
        let found = p
            .and_then(|p| st.first_node(p, false))
            .and_then(|n| self.econ.game.lists.find_unit(UnitType::Monster, n.guid))
            .and_then(|u| self.owner_of(u));
        Some(found)
    }

    /// The player whose lent hireling list holds `merc`'s node.
    fn merc_owner(&self, merc: Owner) -> Option<Owner> {
        let st = self.state.hirelings.as_ref()?;
        let (&p, _) = st
            .lists
            .iter()
            .find(|(_, l)| l.nodes.iter().any(|n| n.guid == merc.guid))?;
        self.owner_of(p)
    }

    /// The hireling's unit update step (unit update bit 0, a hireling whose
    /// owner is the receiver's player → its item messages)
    /// for the client whose player has `receiver`'s GUID,
    /// then the room clean-up's update-list reset of the hireling
    /// (the update list is cleared after the pass). `None`: not a hireling of that player
    /// or nothing pending.
    pub fn merc_items_update(
        &mut self,
        receiver: Guid,
        merc: Guid,
    ) -> Option<Result<Vec<Vec<u8>>, crate::items::moves::MoveFatal>> {
        let m = Owner {
            ty: Owner::MONSTER,
            guid: merc,
        };
        let owner = self.merc_owner(m)?;
        if owner.guid != receiver || MoveUnits::update_bits(self, m) & 1 == 0 {
            return None;
        }
        let r = crate::items::moves::merc_update_pass(self, receiver, merc);
        self.update_done(m);
        Some(r)
    }

    /// Whether the player owns the hireling `(hireling, player)`:
    /// scans the PLAYER's room list (its adjacency array, the room itself
    /// included) for the hireling's room. `None`: no lists lent.
    pub(crate) fn lent_owns_hireling(&self, player: Owner, merc: Owner) -> Option<bool> {
        self.state.hirelings.as_ref()?;
        let l = &self.econ.game.lists;
        let room = |o: Owner| {
            self.unit_of(o)
                .and_then(|u| l.unit(u))
                .and_then(|e| e.room())
        };
        let (Some(pr), Some(mr)) = (room(player), room(merc)) else {
            return Some(false);
        };
        Some(l.room(pr).is_some_and(|r| r.adjacent.contains(&mr)))
    }

    /// Equips an item on the hireling `(game, player, merc, item)` on the
    /// lent lists: the player is the owner of the list holding the merc
    /// (the 0x61 caller's player: its hireling is that list's first
    /// living node). Returns false when no lists are lent; with lists and
    /// no owner nothing happens. The swap's result is not read
    /// (the give ignores it).
    pub(crate) fn lent_equip_on_merc(&mut self, merc: Owner, item: Guid) -> bool {
        if self.state.hirelings.is_none() {
            return false;
        }
        let Some(player) = self.merc_owner(merc) else {
            return true;
        };
        let expansion = MoveUnits::expansion(self);
        let r = swap(&mut MercItems { d: self }, expansion, player, merc, item);
        // The swap's result 0 → sound event
        // 0x56 on the hireling, else 0x57, target the player
        // (the hireling's "thank you" voice when it takes the item; the client
        // sees S→C 0x2C event 0x57).
        if let (Some(m), Some(p)) = (self.unit_of(merc), self.unit_of(player)) {
            let event = if r == 0 { 0x56 } else { 0x57 };
            if let Err(e) = crate::units::sound::queue_sound(self.econ.game, m, event, Some(p)) {
                self.note_list(Err(e));
            }
        }
        true
    }
}

/// [`HirelingItems`] on the inventory desk: each call is the inventory
/// model's operation of the same rule (`items::moves` seams on
/// [`InvDesk`]), the item copy ([`InvDesk::copy_of`]) and
/// the game's timer queue.
pub struct MercItems<'x, 'd, 'a, H, R: ?Sized> {
    pub d: &'x mut InvDesk<'d, 'a, H, R>,
}

impl<H: LifecycleHooks, R: InvRest + ?Sized> MercItems<'_, '_, '_, H, R> {
    fn unit(&self, o: Owner) -> Option<UnitId> {
        self.d.unit_of(o)
    }
}

impl<H: LifecycleHooks, R: InvRest + ?Sized> HirelingItems for MercItems<'_, '_, '_, H, R> {
    fn has_inventory(&self, unit: Owner) -> bool {
        InventoryOps::has_inventory(self.d, unit)
    }
    /// Creates an inventory (the owner kind decides
    /// the grids).
    fn create_inventory(&mut self, unit: Owner) {
        let Some(u) = self.unit(unit) else {
            return;
        };
        if let Some(kind) = self.d.kind_of(u) {
            self.d.state.add_inventory(u, kind, unit.guid);
        }
    }
    /// Itemtypes `BodyLoc1` / `BodyLoc2` of the item's
    /// type; no record → (0, 0).
    fn body_locs(&self, item: Guid) -> (u8, u8) {
        self.d
            .item_unit(item)
            .and_then(|u| self.d.state.items.get(&u))
            .and_then(|it| self.d.tables.itype_of(it.record))
            .map_or((0, 0), |t| (t.bodyloc1, t.bodyloc2))
    }
    fn class(&self, unit: Owner) -> u32 {
        MoveUnits::unit_class(self.d, unit)
    }
    fn is_type(&self, item: Guid, ty: u32) -> bool {
        u16::try_from(ty).is_ok_and(|t| MoveUnits::is_type(self.d, item, t))
    }
    fn body_item(&self, unit: Owner, loc: u8) -> Option<Guid> {
        InventoryOps::body_item(self.d, unit, loc)
    }
    /// Duplicates an item: [`InvDesk::copy_of`] with
    /// fillers (the owner argument is not read).
    fn duplicate(&mut self, _owner: Owner, item: Guid) -> Option<Guid> {
        let src = self.d.item_unit(item)?;
        let copy = self.d.copy_of(src, true)?;
        Some(self.d.guid_of(copy))
    }
    fn set_mode(&mut self, item: Guid, mode: u8) {
        MoveUnits::set_mode(self.d, item, mode);
    }
    /// Cancels timers on the game's timer queue; a = 0: any
    /// argument.
    fn cancel_timers(&mut self, unit: Owner, ty: u8, a: Guid) {
        let Some(u) = self.unit(unit) else {
            return;
        };
        let arg = (a != 0).then_some(a);
        self.d.econ.game.timers.cancel_unit_events(u, ty, arg);
    }
    fn equip_from_cursor(&mut self, unit: Owner, item: Guid, loc: u8, skip: bool) {
        InventoryOps::equip_from_cursor(self.d, unit, item, loc, skip);
    }
    /// The 0x61 give's consume `(game, player, item)`: a stack
    /// loses one; otherwise, when the item is the player's cursor item,
    /// S→C 0x42 (to the player) then the cursor clears and the
    /// unit is freed (0x42 goes out before the tick's messages).
    /// Not the cursor item: nothing (result 0).
    fn consume(&mut self, player: Owner, item: Guid) {
        if MovePending::consume_one(self.d, item) {
            return;
        }
        if let (Some(p), Some(i)) = (self.unit(player), self.d.item_unit(item)) {
            self.d.take_cursor(p, i);
        }
    }
    fn clear_cursor(&mut self, player: Owner) {
        InventoryOps::set_cursor(self.d, player, None);
    }
    /// TODO: the hireling
    /// inventory pass of the take and the general inventory pass are not
    /// told apart; the seam's inventory
    /// pass is called.
    fn inventory_pass(&mut self, unit: Owner) {
        MovePending::inventory_pass(self.d, unit);
    }
    /// TODO: this refresh has no settled
    /// body or provider; nothing runs.
    fn refresh_0055f4f0(&mut self, _unit: Owner) {}
    /// `(game, unit, id, frame + 1, 0, 0)` on the game's timer
    /// queue.
    fn event_next_frame(&mut self, unit: Owner, id: u32) {
        let Some(u) = self.unit(unit) else {
            return;
        };
        let at = self.d.econ.game.frame.wrapping_add(1);
        if let Err(e) = self.d.econ.game.schedule_event(u, id, at, None, 0, 0) {
            self.d.state.errors.push(super::InvError::Economy(e.into()));
        }
    }
    fn unlink(&mut self, unit: Owner, item: Guid) {
        InventoryOps::unlink(self.d, unit, item);
    }
    fn clear_slot(&mut self, unit: Owner, loc: u8) {
        InventoryOps::clear_body_slot(self.d, unit, loc);
    }
    /// The stat refresh unlink `(game, old, merc, 0)`: old's stats leave the hireling.
    fn stat_refresh_unlink(&mut self, unit: Owner) {
        MovePending::stat_refresh_unlink(self.d, unit, 0);
    }
    fn requirements(&self, item: Guid, unit: Owner, equipping: bool) -> bool {
        InventoryOps::requirements(self.d, item, unit, equipping)
    }
    /// 0x10 through the command-flag setter (the take's
    /// "command flag 0x10");
    /// 0x20 through the item flags setter.
    fn item_flag_on(&mut self, item: Guid, flag: u32) {
        if flag == FLAG_10 {
            let f = MoveUnits::cmd_flags(self.d, item);
            MoveUnits::set_cmd_flags(self.d, item, f | flag);
        } else {
            debug_assert_eq!(flag, FLAG_20);
            let f = MoveUnits::item_flags(self.d, item);
            MoveUnits::set_item_flags(self.d, item, f | flag);
        }
    }
    /// Leaves the inventory:
    /// old's GUID goes onto the merc inventory's update list; old was
    /// already unlinked at the start of rule 4, no second unlink.
    fn leave_inventory(&mut self, unit: Owner, item: Guid) {
        InventoryOps::update_list_add(self.d, unit, item);
    }
    /// Queue the unit for
    /// update and set its update flags |= 0x1 (|= 0x2
    /// too for a player); with 0 only bit 0 is cleared.
    fn call_00621000(&mut self, unit: Owner, arg: u32) {
        let v = MoveUnits::update_bits(self.d, unit);
        if arg == 0 {
            MoveUnits::set_update_bits(self.d, unit, v & !1);
            return;
        }
        MovePending::queue_update(self.d, unit);
        let bits = if unit.is_player() { 0x3 } else { 0x1 };
        MoveUnits::set_update_bits(self.d, unit, v | bits);
    }
    /// The cursor take: the item becomes the cursor item, then the owner refresh;
    /// with none the refresh is not called (no item for the seam).
    fn become_cursor(&mut self, player: Owner, item: Option<Guid>) {
        InventoryOps::set_cursor(self.d, player, item);
        if let Some(i) = item {
            MovePending::give_cursor_item(self.d, player, i);
        }
    }
    /// Puts the item back on grid 0 at `loc`, then the page.
    fn put_back(&mut self, unit: Owner, item: Guid, page: u8, loc: u8) {
        InventoryOps::place_body(self.d, unit, item, loc);
        MoveUnits::set_page(self.d, item, page);
    }
    /// TODO: this item call `(item, 0xFF)` has no
    /// settled body; nothing runs.
    fn call_00628280(&mut self, _item: Guid, _arg: u8) {}
    /// TODO: this refresh has no settled body
    /// or provider; nothing runs.
    fn refresh_0055c460(&mut self, _merc: Owner) {}
}

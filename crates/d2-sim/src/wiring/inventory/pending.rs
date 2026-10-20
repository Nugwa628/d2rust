//! [`MovePending`] on [`InvDesk`]: the inventory move engine's host calls,
//! answered by the real world. Wired: the room list (insert and remove),
//! the update queue, "alive", "has
//! durability", item
//! freeing (through the economy) and the creation of gold
//! piles (through the economy; the request comes from the rest),
//! the item bit stream ([`super::bits`]), the gold
//! rest pile, the quest-chain notice 0x5D and the
//! book count change (on the equipment rules).
//! Every other call goes to [`InvRest`] unchanged.

use super::{InvDesk, InvError, InvRest};
use crate::items::inventory::{
    active_inventory_item, belt_removal_allowed, corpse_slot_fit, InvWorld, UnitKind,
};
use crate::items::moves::ground::gold_piles;
use crate::items::moves::{deferred, Guid, MovePending, MoveUnits, Owner, Spot};
use crate::units::lifecycle::LifecycleHooks;
use crate::units::UnitId;

/// Stat 152 `item_indesctructible`.
const STAT_INDESTRUCTIBLE: u16 = 152;

impl<H: LifecycleHooks, R: InvRest + ?Sized> MovePending for InvDesk<'_, '_, H, R> {
    /// Room list removal (a unit in no room is left as is).
    fn remove_from_room(&mut self, item: Guid) {
        if let Some(u) = self.item_unit(item) {
            let r = self.econ.game.lists.room_remove(u);
            self.note_list(r);
        }
    }
    /// Ground placement's "room added": the room list
    /// insert. An item already in the spot's room (the
    /// allocator added it) is left as is; one in another room is logged
    /// ([`InvError::OtherRoom`]).
    fn add_to_room(&mut self, item: Guid, spot: Spot) {
        let Some(u) = self.item_unit(item) else {
            return;
        };
        match self.econ.game.lists.unit(u).and_then(|e| e.room()) {
            None => {
                let r = self.econ.game.lists.room_insert(u, spot.room);
                self.note_list(r);
            }
            Some(r) if r == spot.room => {}
            Some(_) => self.state.errors.push(InvError::OtherRoom(u)),
        }
        // Unit flag 0x1000, for the host's update pass.
        self.state.dropped.insert(u);
        // The item's path follows its ground position.
        self.econ
            .hooks
            .ground_item_placed(u, spot.room, spot.x, spot.y);
    }
    fn in_room(&self, item: Guid) -> bool {
        self.item_unit(item)
            .and_then(|u| self.econ.game.lists.unit(u))
            .is_some_and(|e| e.room().is_some())
    }
    /// Queues the owner for the update pass.
    fn queue_update(&mut self, u: Owner) {
        if let Some(id) = self.unit_of(u) {
            let r = self.econ.game.lists.queue_update(id);
            self.note_list(r);
        }
    }
    /// Frees an item (for example a cube recipe's consumed input):
    /// the item is unlinked from any player inventory list or
    /// cursor still holding it (every player is checked),
    /// then the unit removal
    /// frees the unit and its item data. Nothing else.
    fn free_item(&mut self, item: Guid) {
        let Some(u) = self.item_unit(item) else {
            return;
        };
        let holders: Vec<UnitId> = self
            .state
            .inventories
            .iter()
            .filter(|(&o, inv)| {
                matches!(self.kind_of(o), Some(UnitKind::Player { .. }))
                    && (inv.contains(u) || inv.cursor() == Some(u))
            })
            .map(|(&o, _)| o)
            .collect();
        for o in holders {
            self.unlink_from(o, u);
        }
        if let Err(e) = self.econ.free_item(u) {
            self.state.errors.push(InvError::Economy(e));
        }
        self.state.inventories.remove(&u);
        self.state.expiry.remove(&u);
        self.sync_in();
    }
    /// Creates a gold pile (code `gld`): the request comes from
    /// [`InvRest::gold_request`] (layout unwritten), the item from
    /// [`crate::wiring::economy::Economy::create_item`].
    fn create_gold(&mut self, unit: Owner, _spot: Spot) -> Option<Guid> {
        let gld = self.tables.items.iter().position(|r| &r.code == b"gld ")?;
        let (mut rq, spawn) = self.rest.gold_request(unit, gld)?;
        match self.econ.create_item(&mut rq, false, spawn) {
            Ok(u) => {
                // The ground-item animation init (mode 3):
                // frame 0x1000, count 0x1100, speed 0.
                if let Some(r) = self.econ.units.get_mut(u) {
                    if r.mode == 3 {
                        r.anim.frame = 0x1000;
                        r.anim.frame_count = 0x1100;
                        r.anim.speed = 0;
                    }
                }
                self.sync_in();
                Some(self.guid_of(u))
            }
            Err(e) => {
                self.state.errors.push(InvError::Economy(e));
                None
            }
        }
    }
    /// "Has durability": items
    /// `nodurability` = 0, `durability` ≠ 0, the item has a stat list and
    /// stat 152 (`item_indesctructible`) < 1.
    fn merge_allowed(&self, src: Guid) -> bool {
        let Some(u) = self.item_unit(src) else {
            return false;
        };
        let Some(r) = self
            .state
            .items
            .get(&u)
            .and_then(|d| self.econ.tables.item(d.record))
        else {
            return false;
        };
        r.nodurability == 0
            && r.durability != 0
            && self.econ.units.get(u).is_some_and(|x| x.stats.is_some())
            && self.econ.stats.unit_total(u, STAT_INDESTRUCTIBLE, 0) < 1
    }
    /// Not dead.
    fn alive(&self, u: Owner) -> bool {
        self.unit_of(u)
            .and_then(|id| self.econ.units.get(id))
            .is_some_and(|r| !r.is_dead())
    }

    // ---- no provider: the rest ------------------------------------------

    fn distance(&self, a: Owner, b: Owner) -> i32 {
        self.rest.distance(a, b)
    }
    fn collides(&self, a: Owner, b: Owner, mask: u32) -> bool {
        self.rest.collides(a, b, mask)
    }
    /// Recorded for the host ([`InvDesk::take_item_walks`]), then the
    /// rest's seam.
    fn walk_to_item(&mut self, player: Owner, item: Guid, cursor: bool) {
        if let (Some(p), Some(i)) = (self.unit_of(player), self.item_unit(item)) {
            self.state.item_walks.push((p, i, cursor));
        }
        self.rest.walk_to_item(player, item, cursor)
    }
    fn walk_to_unit(&mut self, player: Owner, target: Owner, cursor: bool) {
        self.rest.walk_to_unit(player, target, cursor)
    }
    fn tile_warp(&mut self, player: Owner, tile: Owner) {
        self.rest.tile_warp(player, tile)
    }
    /// Uses an item at a point (`(I, I, x, y)`, [`InvDesk::dispatch_use`]); an item
    /// with no use entry goes to the rest.
    fn use_item_at(&mut self, player: Owner, item: Guid, x: i32, y: i32) -> bool {
        if self.use_entry(item).is_some() {
            return self.dispatch_use(player, item, Owner::item(item), x, y);
        }
        self.rest.use_item_at(player, item, x, y)
    }
    fn open_cube(&mut self, player: Owner, cube: Guid) -> bool {
        self.open_cube_desk(player, cube)
    }
    /// Consumes a used item: the removal message
    /// (flag 0x20), then the item leaves its inventory and is freed
    /// ([`InvDesk::remove_used_item`]); an item without a unit: the
    /// rest's.
    fn consume_item(&mut self, player: Owner, item: Guid) {
        // A used identify scroll leaves the grid; a used Town Portal
        // scroll too (the caller consumes it after the use). Both send
        // 0x9D action 5 with removal flag 0x20.
        if (self.item_unit(item).is_some()
            && super::identify::IDENTIFY_CODES.contains(&self.code(item)))
            || self.is_portal_scroll(item)
            // A used quest item.
            || (self.item_unit(item).is_some()
                && matches!(&self.code(item), b"ass " | b"xyz " | b"tr2 "))
            // The last key of a stack a locked chest or door used.
            || (self.item_unit(item).is_some()
                && self.code(item) == crate::items::moves::handlers::KEY_CODE)
            // A potion drunk from the grid.
            || (self.item_unit(item).is_some()
                && super::potion::classify(self.code(item)).is_some())
            // An item a mod game's use function used (MS7c).
            || self.mod_use_entry(item)
        {
            return self.remove_used_item(player, item);
        }
        self.rest.consume_item(player, item)
    }
    /// The item's skill from the books rows ([`InvDesk::book_item_skill`]);
    /// no row → the rest.
    fn item_skill(&self, item: Guid) -> i32 {
        match self.book_item_skill(item) {
            -1 => self.rest.item_skill(item),
            s => s,
        }
    }
    /// Whether the player has the skill, on the equipment rules when
    /// [`InvState::equip_rules`](super::InvState::equip_rules) is on.
    fn has_skill(&self, player: Owner, skill: i32) -> bool {
        match (self.state.equip_rules, self.unit_of(player)) {
            (true, Some(p)) => self.owns_skill(p, skill),
            _ => self.rest.has_skill(player, skill),
        }
    }
    fn skill_decrement(&mut self, player: Owner, skill: i32) {
        match (self.state.equip_rules, self.unit_of(player)) {
            (true, Some(p)) => self.run_skill_decrement(p, skill),
            _ => self.rest.skill_decrement(player, skill),
        }
    }
    fn set_quest_flag(&mut self, player: Owner, quest: u8, flag: u8, on: bool) {
        self.rest.set_quest_flag(player, quest, flag, on)
    }
    /// A quest item was used (a quest chain's notice):
    /// S→C 0x5D `5D chain 02 00 0000` to the player's client, with
    /// [`InvState::move_effects`](super::InvState::move_effects).
    fn quest_item_used(&mut self, player: Owner, chain: u8) {
        if self.state.move_effects {
            self.send(player, vec![0x5D, chain, 0x02, 0, 0, 0]);
        } else {
            self.rest.quest_item_used(player, chain)
        }
    }
    fn quest_tr2_used(&mut self, player: Owner) {
        self.rest.quest_tr2_used(player)
    }
    fn reset_skills_stats(&mut self, player: Owner) {
        self.rest.reset_skills_stats(player)
    }
    fn has_used_skill(&self, player: Owner) -> bool {
        self.rest.has_used_skill(player)
    }
    /// Corpse pickup, on the unit hooks
    /// ([`crate::units::hooks::UnitHooks::player_corpse_pickup`]: the
    /// action wiring's `ActionHooks::corpse_pickup`).
    fn corpse_pickup(&mut self, player: Owner, corpse: Owner) -> bool {
        let (Some(p), Some(c)) = (self.unit_of(player), self.unit_of(corpse)) else {
            return false;
        };
        let e = &mut *self.econ;
        let mut sim = crate::units::hooks::Sim {
            game: &mut *e.game,
            units: &mut *e.units,
            stats: &mut *e.stats,
            data: e.data,
        };
        e.hooks.player_corpse_pickup(&mut sim, p, c)
    }
    /// Whether the corpse's item fits its slot, on the inventory model ([`corpse_slot_fit`]).
    fn corpse_slot_fit(
        &self,
        unit: Owner,
        x: Guid,
        d: Option<Guid>,
        a: Option<Guid>,
        l: u8,
    ) -> (bool, u8) {
        let (Some(u), Some(xu)) = (self.unit_of(unit), self.item_unit(x)) else {
            return (false, l);
        };
        let d = d.and_then(|g| self.item_unit(g));
        let a = a.and_then(|g| self.item_unit(g));
        corpse_slot_fit(self, self.tables, u, xu, d, a, l)
    }
    /// The corpse list is not modelled by the desk (the rest's
    /// `corpse_taken` runs after).
    ///
    /// Corpse pickup's last step on the wired
    /// side: the corpse unit leaves its room and is freed with its
    /// inventory. S→C 0x8E `CorpseAssign`
    /// [1] 0, [2..5] the player's GUID, [6..9] the
    /// corpse's GUID, goes out at the call; the room
    /// delete record only queues the 0x0A (type 0, the corpse's GUID),
    /// which the host sends with the next tick like an item's.
    /// Only the player is told (single player: the 0x8E and the 0x0A would go
    /// to every player and the room's clients in multiplayer); the rest's
    /// `corpse_taken` still runs.
    fn corpse_taken(&mut self, player: Owner, corpse: Owner) {
        if let Some(c) = self.unit_of(corpse) {
            let mut assign = vec![0x8E, 0];
            assign.extend_from_slice(&player.guid.to_le_bytes());
            assign.extend_from_slice(&corpse.guid.to_le_bytes());
            self.rest.send(player, assign);
            self.rest.send(
                player,
                crate::units::messages::remove_unit(0, corpse.guid).to_vec(),
            );
            let r = self.econ.game.lists.room_remove(c);
            self.note_list(r);
            self.state.inventories.remove(&c);
            let (mut sim, hooks) = self.econ.split();
            if let Err(e) = crate::units::lifecycle::remove(&mut sim, hooks, c) {
                self.state.errors.push(InvError::Economy(e.into()));
            }
            self.sync_in();
        }
        self.rest.corpse_taken(player, corpse)
    }
    fn replenish_timers(&mut self, item: Guid) {
        if let Some(u) = self.item_unit(item) {
            self.schedule_replenish(u);
        }
    }
    fn player_interact(&mut self, player: Owner, other: Owner) {
        self.rest.player_interact(player, other)
    }
    fn room_at(&self, x: i32, y: i32) -> bool {
        self.rest.room_at(x, y)
    }
    fn free_spot(
        &self,
        start: (i32, i32),
        origin: (i32, i32),
        size: u32,
        mask: u32,
        mask2: u32,
        last: u32,
    ) -> Option<Spot> {
        self.rest.free_spot(start, origin, size, mask, mask2, last)
    }
    /// Whether the player's room is a town (through the unit side's levels;
    /// the stash page needs a town),
    /// else the rest's answer.
    fn in_town(&self, player: Owner) -> bool {
        let room = self
            .unit_of(player)
            .and_then(|u| self.econ.game.lists.unit(u))
            .and_then(|e| e.room());
        room.is_some_and(|r| self.econ.hooks.town_room(self.econ.game, r))
            || self.rest.in_town(player)
    }
    fn room_delete_notice(&mut self, item: Guid) {
        self.rest.room_delete_notice(item)
    }
    fn free_collision(&mut self, item: Guid) {
        if let Some(u) = self.item_unit(item) {
            self.econ.hooks.ground_item_lifted(u);
        }
        self.rest.free_collision(item)
    }
    fn room_change_notice(&mut self, item: Guid, x: i32, y: i32) {
        self.rest.room_change_notice(item, x, y)
    }
    fn stat_refresh(&mut self, u: Owner) {
        self.rest.stat_refresh(u)
    }
    fn stat_refresh_unlink(&mut self, u: Owner, b: u32) {
        self.rest.stat_refresh_unlink(u, b)
    }
    /// The weapon-in-use link, then the stat
    /// link.
    fn stat_link(&mut self, owner: Owner, item: Guid) {
        if let (Some(o), Some(i)) = (self.unit_of(owner), self.item_unit(item)) {
            self.weapon_link_on(o, i, true);
            self.link_item_stats(o, i);
        }
        self.rest.stat_link(owner, item)
    }
    /// Charm relink on the rules when [`InvState::equip_rules`] is on (a charm
    /// counts only in the inventory grid).
    /// [`InvState::equip_rules`]: super::InvState::equip_rules
    fn charm_relink(&mut self, owner: Owner, item: Guid) {
        match (
            self.state.equip_rules,
            self.unit_of(owner),
            self.item_unit(item),
        ) {
            (true, Some(o), Some(i)) => self.run_item_skill_link(o, i, true),
            _ => self.rest.charm_relink(owner, item),
        }
    }
    /// Charm unlink on the rules when the equipment rules are on.
    fn charm_unlink(&mut self, owner: Owner, item: Guid) {
        match (
            self.state.equip_rules,
            self.unit_of(owner),
            self.item_unit(item),
        ) {
            (true, Some(o), Some(i)) => self.run_item_skill_link(o, i, false),
            _ => self.rest.charm_unlink(owner, item),
        }
        if let (Some(o), Some(i)) = (self.unit_of(owner), self.item_unit(item)) {
            self.unlink_item_stats(o, i);
        }
    }
    /// Whether the item is active (counts for the owner's stats).
    fn is_active(&self, owner: Owner, item: Guid) -> bool {
        match (self.unit_of(owner), self.item_unit(item)) {
            (Some(o), Some(i)) => active_inventory_item(self, self.tables, i, o),
            _ => false,
        }
    }
    /// The inventory pass on the rules when the equipment rules are on.
    fn inventory_pass(&mut self, owner: Owner) {
        match (self.state.equip_rules, self.unit_of(owner)) {
            (true, Some(o)) => self.run_inventory_pass(o, false),
            _ => self.rest.inventory_pass(owner),
        }
        if let Some(o) = self.unit_of(owner) {
            self.link_charms(o);
        }
    }
    fn weapon_in_use_update(&mut self, owner: Owner) {
        self.rest.weapon_in_use_update(owner)
    }
    /// Weapon bookkeeping on the rules when the equipment rules are on.
    fn weapon_bookkeeping(&mut self, owner: Owner) {
        match (self.state.equip_rules, self.unit_of(owner)) {
            (true, Some(o)) => self.run_weapon_bookkeeping(o),
            _ => self.rest.weapon_bookkeeping(owner),
        }
    }
    /// The body unlink (clears the body slot), then the
    /// inventory unlink's stat detach (no set update, `item_link`).
    fn body_leave_effects(&mut self, owner: Owner, item: Guid) {
        if let (Some(o), Some(i)) = (self.unit_of(owner), self.item_unit(item)) {
            self.weapon_link_on(o, i, false);
            self.detach_item_stats(o, i);
        }
        self.rest.body_leave_effects(owner, item)
    }
    fn hireling_owner_pass(&mut self, owner: Owner) {
        self.rest.hireling_owner_pass(owner)
    }
    /// Whether the player's belt may be removed, on the player's inventory.
    fn belt_remove_allowed(&self, player: Owner) -> bool {
        belt_removal_allowed(&self.inv_or_empty(player), self)
    }
    /// The sound event `id` on `u` (`(u, id, u)`): every move
    /// site passes the unit itself as the target (refused pickup,
    /// corpse loot and can't-carry),
    /// so only its own client hears it.
    fn sound(&mut self, u: Owner, id: u32) {
        self.queue_own_sound(u, id as u16);
        self.rest.sound(u, id)
    }
    /// The sound event on the player: queued in the player's sound slot,
    /// sent as S→C 0x2C in
    /// the tick's client pass. Event 1, the player its own target: the
    /// auto pickup and the cursor pickup
    /// both pass event 1 and the player twice.
    fn pickup_sound(&mut self, player: Owner, item: Guid) {
        self.queue_own_sound(player, 1);
        self.rest.pickup_sound(player, item)
    }
    fn requirement_sound(&mut self, player: Owner) {
        self.rest.requirement_sound(player)
    }
    fn merc_sound(&mut self, player: Owner) {
        self.rest.merc_sound(player)
    }
    fn quest_flag(&self, player: Owner, quest: u8, flag: u8) -> bool {
        self.rest.quest_flag(player, quest, flag)
    }
    fn quest_item_picked(&mut self, player: Owner, item: Guid) {
        self.rest.quest_item_picked(player, item)
    }
    fn quest_item_dropped(&mut self, item: Guid) {
        self.rest.quest_item_dropped(item)
    }
    fn carry_one(&self, item: Guid) -> bool {
        self.rest.carry_one(item)
    }
    fn held_test_units(&self, player: Owner) -> Vec<Owner> {
        self.rest.held_test_units(player)
    }
    /// Copies an item with its fillers, as the hireling give's copy.
    fn copy_item(&mut self, item: Guid) -> Option<Guid> {
        let src = self.item_unit(item)?;
        let copy = self.copy_of(src, true)?;
        Some(self.guid_of(copy))
    }
    /// Puts the item on the player's cursor: mode 4,
    /// command flag 0x100000, the update list, the owner refresh.
    fn give_cursor_item(&mut self, player: Owner, item: Guid) {
        crate::items::moves::handlers::load_to_cursor(self, player, item);
    }
    /// A key stack a locked chest or door used loses one key (its owner's
    /// client gets the new quantity); its last key is not decremented here
    /// (`false`: the caller uses the item up). Other items: the rest.
    fn consume_one(&mut self, item: Guid) -> bool {
        use crate::items::moves::handlers::KEY_CODE;
        use crate::items::moves::stat::QUANTITY;
        let Some(u) = self.item_unit(item).filter(|_| self.code(item) == KEY_CODE) else {
            return self.rest.consume_one(item);
        };
        let it = Owner::item(item);
        let q = MoveUnits::stat(self, it, QUANTITY);
        if q <= 1 {
            return false;
        }
        MoveUnits::set_stat(self, it, QUANTITY, q - 1);
        let owner = self
            .state
            .items
            .get(&u)
            .map(|d| d.owner_guid)
            .filter(|&g| g != crate::items::inventory::NO_GUID);
        if let Some(g) = owner {
            self.send_item_stat(Owner::player(g), item, QUANTITY);
        }
        true
    }
    fn set_owner(&mut self, item: Guid, owner: Owner) {
        self.rest.set_owner(item, owner)
    }
    fn pile_owner(&self, item: Guid) -> Option<Owner> {
        self.rest.pile_owner(item)
    }
    fn gold_drop_has_no_owner(&self) -> bool {
        self.rest.gold_drop_has_no_owner()
    }
    fn party_share_id(&self, player: Owner) -> i32 {
        self.rest.party_share_id(player)
    }
    fn party_share(&mut self, player: Owner, take: i32) {
        self.rest.party_share(player, take)
    }
    fn owned_gold_pickup(&mut self, player: Owner, pile: Guid, take: i32) {
        self.rest.owned_gold_pickup(player, pile, take)
    }
    /// "A gold pile of `rest` at the player"
    /// (the leftover gold of a move) as one pile
    /// ([`gold_piles`], max 1; the same creation and placement as the
    /// 0x50 drop), with [`InvState::move_effects`](super::InvState::move_effects).
    fn rest_pile(&mut self, player: Owner, rest: i32) {
        if self.state.move_effects {
            gold_piles(self, player, rest, 1);
        } else {
            self.rest.rest_pile(player, rest)
        }
    }
    /// Book count change on the rules when [`InvState::equip_rules`] is on.
    ///
    /// [`InvState::equip_rules`]: super::InvState::equip_rules
    fn book_count_changed(&mut self, player: Owner, book: Guid, n: i32) {
        match (
            self.state.equip_rules,
            self.unit_of(player),
            self.item_unit(book),
        ) {
            (true, Some(o), Some(i)) => self.run_item_skill_add(o, i, n),
            _ => self.rest.book_count_changed(player, book, n),
        }
    }
    /// Uses an item on a target (`(I, T, 0, 0)`, [`InvDesk::dispatch_use`]); an item with
    /// no use entry keeps the earlier answers.
    fn use_item(&mut self, player: Owner, target: Owner, item: Guid) -> bool {
        if self.use_entry(item).is_some() {
            return self.dispatch_use(player, item, target, 0, 0);
        }
        // Potions on the player (healing, mana, rejuvenation, and the `vps`, `rvs`,
        // `rvl` potions, D2RUST's own).
        if target == player && self.use_potion(player, item) {
            return true;
        }
        // Identify scrolls and tomes on an item.
        if self.use_identify(player, target, item) {
            return true;
        }
        self.rest.use_item(player, target, item)
    }
    fn charge_update(&mut self, player: Owner, item: Guid) {
        self.rest.charge_update(player, item)
    }
    fn remove_used(&mut self, player: Owner, item: Guid) {
        if self.item_unit(item).is_some() {
            return self.remove_belt_item(player, item);
        }
        self.rest.remove_used(player, item)
    }
    /// Equips the picked item on the inventory model (`equip_without_cursor`, skip 0), then
    /// unit flag 0x2000000 cleared.
    fn equip_picked(&mut self, player: Owner, item: Guid) -> bool {
        let Some(u) = self.item_unit(item) else {
            return false;
        };
        let t = self.tables;
        let ok = self
            .with_inv(player, |inv, d| {
                crate::items::inventory::equip::equip_without_cursor(inv, d, t, u, false)
            })
            .unwrap_or(false);
        if ok {
            let o = Owner::item(item);
            let f = crate::items::moves::MoveUnits::unit_flags(self, o);
            crate::items::moves::MoveUnits::set_unit_flags(
                self,
                o,
                f & !crate::items::moves::uflag::ON_GROUND,
            );
        }
        ok
    }
    /// The filler's properties (a socketed gem, rune or jewel,
    /// [`InvDesk::apply_filler_properties`]), then the owner link
    /// on the rest.
    fn filler_linked(&mut self, filler: Guid, target: Guid) {
        if let (Some(f), Some(t)) = (self.item_unit(filler), self.item_unit(target)) {
            self.apply_filler_properties(f, t);
        }
        self.rest.filler_linked(filler, target)
    }
    /// A socket fill's runeword and recharge ([`InvDesk::socket_runeword`]);
    /// each recharged charged skill is announced to the player by S→C 0x3E
    /// stat 204. The rest's default is not
    /// asked.
    fn runeword(&mut self, player: Owner, target: Guid) -> bool {
        let Some(t) = self.item_unit(target) else {
            return false;
        };
        let (ran, recharged) = self.socket_runeword(t);
        if player.is_player() {
            for _ in recharged {
                self.rest
                    .send_item_stat(player, target, crate::items::recharge::CHARGED_SKILL);
            }
        }
        ran
    }
    /// The player's hireling, from the lent hireling lists
    /// ([`InvDesk::lent_hireling`]); none lent → the rest.
    fn hireling(&self, player: Owner) -> Option<Owner> {
        match self.lent_hireling(player) {
            Some(m) => m,
            None => self.rest.hireling(player),
        }
    }
    /// Whether the player owns the hireling, from the lent lists ([`InvDesk::lent_owns_hireling`]);
    /// none lent → the rest.
    fn owns_hireling(&self, player: Owner, merc: Owner) -> bool {
        match self.lent_owns_hireling(player, merc) {
            Some(b) => b,
            None => self.rest.owns_hireling(player, merc),
        }
    }
    /// Equips an item on a hireling, with the lent lists
    /// ([`InvDesk::lent_equip_on_merc`]); none lent → the rest.
    fn equip_on_merc(&mut self, merc: Owner, item: Guid) {
        if !self.lent_equip_on_merc(merc, item) {
            self.rest.equip_on_merc(merc, item)
        }
    }
    fn merc_after_take(&mut self, merc: Owner) {
        self.rest.merc_after_take(merc)
    }
    fn pick_npc(&mut self, player: Owner, guid: Guid, cursor: u32) -> u32 {
        self.rest.pick_npc(player, guid, cursor)
    }
    fn pick_object(&mut self, player: Owner, guid: Guid, cursor: u32) -> u32 {
        self.rest.pick_object(player, guid, cursor)
    }
    fn send(&mut self, player: Owner, bytes: Vec<u8>) {
        self.rest.send(player, bytes)
    }
    /// S→C 0x3E (`(client, item, 1, stat, value, 0)`) with the
    /// item's base stat value (layer 0, the value the client's base set
    /// writes), sent through the
    /// rest's transport. An item without a unit sends nothing.
    /// Field widths: see `units::messages::update_item_stat`.
    fn send_item_stat(&mut self, player: Owner, item: Guid, stat: u16) {
        let Some(u) = self.item_unit(item) else {
            return;
        };
        let value = self.econ.stats.unit_base(u, stat, 0);
        let msg = crate::units::messages::update_item_stat(item, stat, value, 0);
        self.rest.send(player, msg)
    }
    /// The item bit stream of the real item
    /// ([`InvDesk::item_stream`]); the rest's default is not asked.
    fn item_bits(&self, item: Guid, flags: u32, page: u8) -> Vec<u8> {
        self.item_stream(item, flags, page)
    }
    fn store_item_bits(&self, item: Guid, page: u8) -> Vec<u8> {
        self.store_stream(item, page)
    }
    /// The store messages: 0x38 for a vendor item (unit flag bit 2);
    /// 0x39 (bit 4) stays with the rest.
    fn store_messages(&mut self, client: Owner, item: Guid) -> Vec<Vec<u8>> {
        if self.update_bits(Owner::item(item)) & deferred::VENDOR_ITEM != 0 {
            // A failed encode sends nothing (as the shown-store flush).
            return deferred::store_item_message(self, item).unwrap_or_default();
        }
        self.rest.store_messages(client, item)
    }
}

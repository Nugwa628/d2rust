//! Seams of the item-move code.
//!
//! - [`InventoryOps`]: exactly the inventory operations the
//!   handlers call (provider: `items::inventory`). No
//!   defaults: every method is an inventory rule.
//! - [`MoveUnits`]: unit record fields. No defaults (provider: the units
//!   and item records through the wiring).
//! - [`MovePending`]: calls into systems that are
//!   not wired yet. Each default is the narrowest reading (nothing happens,
//!   or the value that makes the caller do nothing); when a provider lands,
//!   its methods move into the adapter that wires it.

use crate::units::RoomId;

use super::{Guid, Owner};

/// A ground position found by the free-spot search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spot {
    pub room: RoomId,
    pub x: i32,
    pub y: i32,
}

/// The inventory operations this module calls, by their inventory
/// names. Item checks return the check's result code (0 = pass).
pub trait InventoryOps {
    /// Placement state saved before a multi-item equipment move.
    type EquipmentCheckpoint;
    fn equipment_checkpoint(&self, owner: Owner) -> Self::EquipmentCheckpoint;
    /// Restore directly, without repeating the placement that failed.
    fn restore_equipment(&mut self, owner: Owner, saved: Self::EquipmentCheckpoint);

    // ---- item list, update list, cursor ---------------------------

    /// Whether the unit has an inventory record.
    fn has_inventory(&self, owner: Owner) -> bool;
    /// Cursor item.
    fn cursor(&self, owner: Owner) -> Option<Guid>;
    /// Set the cursor.
    fn set_cursor(&mut self, owner: Owner, item: Option<Guid>);
    /// The item list in link order.
    fn items(&self, owner: Owner) -> Vec<Guid>;
    /// Unlink; false when the item is missing or not in
    /// `owner`'s inventory (callers treat that as fatal where the move says so).
    fn unlink(&mut self, owner: Owner, item: Guid) -> bool;
    /// The update list, in order.
    fn update_list(&self, owner: Owner) -> Vec<Guid>;
    /// Append to the update list unless listed.
    fn update_list_add(&mut self, owner: Owner, item: Guid);
    /// Free the update list.
    fn update_list_free(&mut self, owner: Owner);
    /// GUID of the weapon in use.
    fn weapon_in_use(&self, owner: Owner) -> Option<Guid>;

    // ---- grid placement ----------------------------------------------

    /// Place at a position on `page`.
    fn place_at(&mut self, owner: Owner, item: Guid, page: u8, x: i32, y: i32) -> bool;
    /// Free-position search on `page`.
    fn find_free(&self, owner: Owner, item: Guid, page: u8) -> Option<(i32, i32)>;
    /// Item placement into a page, steps 2–9 (the
    /// caller does step 1: owner check and targeting reset). The page is
    /// the item's page field.
    fn place_in_page(
        &mut self,
        owner: Owner,
        item: Guid,
        x: i32,
        y: i32,
        find: bool,
        send: bool,
    ) -> bool;
    /// Link check.
    fn link_check(&mut self, owner: Owner, item: Guid, kind: u8) -> bool;
    /// Link a socket filler into an item's inventory (on
    /// the item's inventory, created first).
    fn link_into_item(&mut self, target: Guid, filler: Guid) -> bool;

    // ---- belt --------------------------------------------------------

    /// Beltable.
    fn beltable(&self, item: Guid) -> bool;
    /// Auto-belt gate.
    fn auto_belt_gate(&self, owner: Owner, item: Guid) -> bool;
    /// Free slot for an item.
    fn belt_free_slot(&self, owner: Owner, item: Guid) -> Option<u8>;
    /// Place in a slot.
    fn belt_place(&mut self, owner: Owner, item: Guid, slot: u32) -> bool;
    /// Compaction after a slot is emptied.
    fn belt_compact(&mut self, owner: Owner, slot: u8);
    /// The item in a belt slot (grid 1).
    fn belt_item(&self, owner: Owner, slot: u8) -> Option<Guid>;
    /// `numboxes` of the belt type of `belt`, or of record
    /// 2 without one.
    fn belt_boxes(&self, belt: Option<Guid>) -> u8;
    /// The grid item list of page `page` (grid page + 2), in grid list
    /// order.
    fn page_items(&self, owner: Owner, page: u8) -> Vec<Guid>;
    /// The body-location grid's item list (grid 0), in grid
    /// list order.
    fn body_items(&self, owner: Owner) -> Vec<Guid>;

    // ---- equipping ---------------------------------------------------

    /// The item at a body location.
    fn body_item(&self, owner: Owner, loc: u8) -> Option<Guid>;
    /// Put at a body location (= on grid 0).
    fn place_body(&mut self, owner: Owner, item: Guid, loc: u8) -> bool;
    /// Clear a body slot.
    fn clear_body_slot(&mut self, owner: Owner, loc: u8);
    /// The item to remove from a location (for equip check
    /// result 4: the two-handed item in the other hand).
    fn item_to_remove(&self, owner: Owner, loc: u8) -> Option<Guid>;
    /// Two-handed.
    fn two_handed(&self, item: Guid) -> bool;
    /// Requirements.
    fn requirements(&self, item: Guid, unit: Owner, equipping: bool) -> bool;
    /// Equip check.
    fn equip_check(&self, unit: Owner, loc: u8, item: Option<Guid>, skip: bool) -> u8;
    /// Stack test.
    fn stack_test(&self, a: Guid, b: Guid) -> bool;
    /// Equip from the cursor: (result 1, out).
    fn equip_from_cursor(&mut self, player: Owner, item: Guid, loc: u8, skip: bool)
        -> (bool, bool);
    /// Auto-equip on pickup: the location, or none.
    fn auto_equip(&self, unit: Owner, item: Guid, skip: bool) -> Option<u8>;

    // ---- shared checks -----------------------------------------------

    /// Cursor item check.
    fn check_cursor_item(&self, player: Owner, item: Guid) -> u32;
    /// Stored item check.
    fn check_stored(&self, player: Owner, item: Guid) -> u32;
    /// Stored-or-equipped check.
    fn check_stored_or_equipped(&self, player: Owner, item: Guid) -> u32;
    /// Owned item check.
    fn check_owned(&self, player: Owner, item: Guid) -> u32;
    /// Belt item check.
    fn check_belt(&self, player: Owner, item: Guid) -> u32;
    /// Ground-or-owned check.
    fn check_ground_or_owned(&self, player: Owner, item: Guid) -> u32;
    /// Busy.
    fn busy(&self, player: Owner) -> bool;
    /// Trading.
    fn trading(&self, player: Owner) -> bool;
    /// Targeting reset.
    fn targeting_reset(&mut self, player: Owner);
    /// Item-move gate; true = allowed.
    fn item_move_gate(&mut self, player: Owner, item: Option<Guid>) -> bool;
}

/// Unit record fields read or written here.
pub trait MoveUnits {
    /// Unit lookup by type and GUID.
    fn unit_exists(&self, u: Owner) -> bool;
    /// Unit class (player class, monster class id).
    fn unit_class(&self, u: Owner) -> u32;
    /// Unit mode (17 is dead for a player).
    fn unit_mode(&self, u: Owner) -> u32;
    /// Position (subtiles).
    fn pos(&self, u: Owner) -> (i32, i32);
    fn set_pos(&mut self, u: Owner, x: i32, y: i32);
    /// Unit flags.
    fn unit_flags(&self, u: Owner) -> u32;
    fn set_unit_flags(&mut self, u: Owner, v: u32);
    /// Unit update bits.
    fn update_bits(&self, u: Owner) -> u32;
    fn set_update_bits(&mut self, u: Owner, v: u32);
    /// Unit total of a stat (layer 0).
    fn stat(&self, u: Owner, id: u16) -> i32;
    /// Set a base stat (layer 0).
    fn set_stat(&mut self, u: Owner, id: u16, v: i32);
    /// Expansion game flag.
    fn expansion(&self) -> bool;
    /// Game frame.
    fn frame(&self) -> i32;
    /// The game's tick rate: durations stated in base ticks are
    /// converted through it.
    /// No default, so no world runs at the base rate by accident.
    fn tick_rate(&self) -> crate::tick::TickRate;

    // ---- item data ----

    fn mode(&self, item: Guid) -> u8;
    fn set_mode(&mut self, item: Guid, m: u8);
    fn page(&self, item: Guid) -> u8;
    fn set_page(&mut self, item: Guid, p: u8);
    fn stored_page(&self, item: Guid) -> u8;
    fn set_stored_page(&mut self, item: Guid, p: u8);
    fn body_loc(&self, item: Guid) -> u8;
    fn set_body_loc(&mut self, item: Guid, loc: u8);
    fn cmd_flags(&self, item: Guid) -> u32;
    fn set_cmd_flags(&mut self, item: Guid, v: u32);
    fn item_flags(&self, item: Guid) -> u32;
    fn set_item_flags(&mut self, item: Guid, v: u32);
    /// Ground expiry frame.
    fn set_expiry(&mut self, item: Guid, frame: i32);
    /// The unit owning the item's inventory.
    fn item_owner(&self, item: Guid) -> Option<Owner>;
    /// Itemtypes test with equivalence.
    fn is_type(&self, item: Guid, ty: u16) -> bool;
    /// Primary type (items `type`, no equivalence).
    fn primary_type(&self, item: Guid) -> u16;
    /// Stackable.
    fn stackable(&self, item: Guid) -> bool;
    /// Itemtypes `autostack` of the primary type.
    fn autostack(&self, item: Guid) -> bool;
    /// Itemtypes `quiver` of the primary type ≠ 0.
    fn quiver(&self, item: Guid) -> bool;
    /// Items code.
    fn code(&self, item: Guid) -> [u8; 4];
    /// Quality.
    fn quality(&self, item: Guid) -> u8;
    /// Unique / set file index (−1 none).
    fn file_index(&self, item: Guid) -> i32;
    /// Items `quest`.
    fn quest(&self, item: Guid) -> u8;
    /// Items `useable`.
    fn useable(&self, item: Guid) -> bool;
    /// Items `component`.
    fn component(&self, item: Guid) -> u8;
    /// Max stack.
    fn max_stack(&self, item: Guid) -> i32;
    /// Socketed with fillers.
    fn socket_filled(&self, item: Guid) -> bool;
    /// Socket-filler test.
    fn socket_filler(&self, item: Guid) -> bool;
    /// Sockets.
    fn sockets(&self, item: Guid) -> i32;
    /// Fillers in the item's inventory, in link order (counts them).
    fn fillers(&self, item: Guid) -> Vec<Guid>;
    /// Book / scroll spell.
    fn spell(&self, item: Guid) -> i32;
}

/// Calls without a written or wired owner (see the module doc).
#[allow(unused_variables)]
pub trait MovePending {
    // ---- path and placement (being written) ----

    /// Unit-to-unit distance. Default: out of range.
    fn distance(&self, a: Owner, b: Owner) -> i32 {
        i32::MAX
    }
    /// Collision between two units on `mask`.
    fn collides(&self, a: Owner, b: Owner, mask: u32) -> bool {
        false
    }
    /// Walk to an item (arrival belongs to movement).
    fn walk_to_item(&mut self, player: Owner, item: Guid, cursor: bool) {}
    /// Walk to a player or tile unit (types 0 and 5).
    fn walk_to_unit(&mut self, player: Owner, target: Owner, cursor: bool) {}
    /// Warp through a tile (unit type 5).
    fn tile_warp(&mut self, player: Owner, tile: Owner) {}
    /// Whether a room exists at (x, y).
    fn room_at(&self, x: i32, y: i32) -> bool {
        false
    }
    /// Free-spot search.
    fn free_spot(
        &self,
        start: (i32, i32),
        origin: (i32, i32),
        size: u32,
        mask: u32,
        mask2: u32,
        last: u32,
    ) -> Option<Spot> {
        None
    }
    /// The player's room is in a town level.
    fn in_town(&self, player: Owner) -> bool {
        false
    }

    // ---- rooms ------------------------

    /// Room delete notice.
    fn room_delete_notice(&mut self, item: Guid) {}
    /// Collision freed.
    fn free_collision(&mut self, item: Guid) {}
    /// Room list removal; a no-op for an item in no room.
    fn remove_from_room(&mut self, item: Guid) {}
    /// Room added at a spot (ground placement).
    fn add_to_room(&mut self, item: Guid, spot: Spot) {}
    /// Whether the item is in a room.
    fn in_room(&self, item: Guid) -> bool {
        false
    }
    /// Room-change notice with the item's old cell.
    fn room_change_notice(&mut self, item: Guid, x: i32, y: i32) {}
    /// Queue the unit for update.
    fn queue_update(&mut self, u: Owner) {}

    // ---- stat lists, item-skill link, inventory pass ---------

    /// Stat refresh.
    fn stat_refresh(&mut self, u: Owner) {}
    /// Stat refresh.
    fn stat_refresh_unlink(&mut self, u: Owner, b: u32) {}
    /// Stat link.
    fn stat_link(&mut self, owner: Owner, item: Guid) {}
    /// Item-skill link.
    fn charm_relink(&mut self, owner: Owner, item: Guid) {}
    /// Item-skill unlink.
    fn charm_unlink(&mut self, owner: Owner, item: Guid) {}
    /// Active inventory item for its owner.
    fn is_active(&self, owner: Owner, item: Guid) -> bool {
        false
    }
    /// Inventory pass.
    fn inventory_pass(&mut self, owner: Owner) {}
    /// Weapon-in-use update.
    fn weapon_in_use_update(&mut self, owner: Owner) {}
    /// Weapon bookkeeping.
    fn weapon_bookkeeping(&mut self, owner: Owner) {}
    /// Body leave effects.
    fn body_leave_effects(&mut self, owner: Owner, item: Guid) {}
    /// Hireling owner pass (→ inventory pass with 1).
    fn hireling_owner_pass(&mut self, owner: Owner) {}

    // ---- belt removal gate -----------------------------------

    /// May the belt be removed. Default: no.
    fn belt_remove_allowed(&self, player: Owner) -> bool {
        false
    }

    // ---- sounds ---------------------------------------------------------

    /// Sound event on a unit (family).
    fn sound(&mut self, u: Owner, id: u32) {}
    /// Pickup sound on the player.
    fn pickup_sound(&mut self, player: Owner, item: Guid) {}
    /// Sound of a refused requirement (id not written).
    fn requirement_sound(&mut self, player: Owner) {}
    /// Sound of a hireling give (id not written).
    fn merc_sound(&mut self, player: Owner) {}

    // ---- quests -------------------------------------

    /// Quest flag of the player's record for the current difficulty.
    /// Default: clear.
    fn quest_flag(&self, player: Owner, quest: u8, flag: u8) -> bool {
        false
    }
    /// Hook ITEMPICKEDUP.
    fn quest_item_picked(&mut self, player: Owner, item: Guid) {}
    /// Hook ITEMDROPPED.
    fn quest_item_dropped(&mut self, item: Guid) {}
    /// Carry-one unique: the record flag has the carry-one bit.
    fn carry_one(&self, item: Guid) -> bool {
        false
    }
    /// The player's corpses (the inventory's list,
    /// each node's GUID looked up as a player unit), whose
    /// item lists the held test also walks. Default: none.
    fn held_test_units(&self, player: Owner) -> Vec<Owner> {
        Vec::new()
    }

    // ---- item creation, freeing, ownership -------

    /// Create a `gld` item at a spot (draws). Default: none.
    fn create_gold(&mut self, unit: Owner, spot: Spot) -> Option<Guid> {
        None
    }
    /// Free an item.
    fn free_item(&mut self, item: Guid) {}
    /// Duplicate (draws). Default: none.
    fn copy_item(&mut self, item: Guid) -> Option<Guid> {
        None
    }
    /// Make a new item the player's cursor item.
    fn give_cursor_item(&mut self, player: Owner, item: Guid) {}
    /// Decrement a stack instead of consuming: true = it was
    /// a stack and was decremented.
    fn consume_one(&mut self, item: Guid) -> bool {
        false
    }
    /// Item owner := unit.
    fn set_owner(&mut self, item: Guid, owner: Owner) {}
    /// Pile owner.
    fn pile_owner(&self, item: Guid) -> Option<Owner> {
        None
    }
    /// A game-side test whose meaning is not known: when true, a dropped
    /// gold pile gets no owner. Default: 0 (every pile is owned).
    fn gold_drop_has_no_owner(&self) -> bool {
        false
    }
    /// Party share id; −1 none.
    fn party_share_id(&self, player: Owner) -> i32 {
        -1
    }
    /// Party share (multiplayer).
    fn party_share(&mut self, player: Owner, take: i32) {}
    /// Gold pickup of a pile owned by a player.
    fn owned_gold_pickup(&mut self, player: Owner, pile: Guid, take: i32) {}
    /// New pile of the rest at the player.
    fn rest_pile(&mut self, player: Owner, rest: i32) {}
    /// The stack merge's second branch. Default:
    /// no merge.
    fn merge_allowed(&self, src: Guid) -> bool {
        false
    }
    /// Book count change with (n) of the tome `book` (skill quantity += n,
    /// S→C 0x22).
    fn book_count_changed(&mut self, player: Owner, book: Guid, n: i32) {}

    // ---- item use (not wired yet) ----------------

    /// Use with (I, I, x, y) of an item at a position (step
    /// 3); true = used.
    fn use_item_at(&mut self, player: Owner, item: Guid, x: i32, y: i32) -> bool {
        false
    }
    /// Opening the Horadric Cube (item-use table entry 7):
    /// the interaction (type 4, the cube) and the
    /// S→C 0x77 messages; true = opened (the cube is not consumed).
    /// Default: not opened.
    fn open_cube(&mut self, player: Owner, cube: Guid) -> bool {
        false
    }
    /// Use on a target; true = used.
    fn use_item(&mut self, player: Owner, target: Owner, item: Guid) -> bool {
        false
    }
    /// Tome / skill charge update (S→C 0x22).
    fn charge_update(&mut self, player: Owner, item: Guid) {}
    /// Removal of a used item from the belt.
    fn remove_used(&mut self, player: Owner, item: Guid) {}
    /// Consume an item (S→C with flag
    /// 0x20, then the item's removal with 0).
    fn consume_item(&mut self, player: Owner, item: Guid) {}
    /// Item skill of a scroll or tome (book →
    /// books `bookskill`, scroll → `scrollskill`, else −1). Default: −1.
    fn item_skill(&self, item: Guid) -> i32 {
        -1
    }
    /// The player has skill `skill`. Default: no.
    fn has_skill(&self, player: Owner, skill: i32) -> bool {
        false
    }
    /// Skill decrement: quantity − 1, < 1 →
    /// 0 and skill 0 on the right when S is the right skill, S→C 0x22.
    fn skill_decrement(&mut self, player: Owner, skill: i32) {}
    /// Set or clear a quest flag of the player's record for the current
    /// difficulty.
    fn set_quest_flag(&mut self, player: Owner, quest: u8, flag: u8, on: bool) {}
    /// After a quest item use (S→C 0x5D `5D chain 02 00 0000` to the
    /// player's client).
    fn quest_item_used(&mut self, player: Owner, chain: u8) {}
    /// (`tr2`).
    fn quest_tr2_used(&mut self, player: Owner) {}
    /// Skills and stats reset (`toa`).
    fn reset_skills_stats(&mut self, player: Owner) {}
    /// Equip a picked-up item; true = success.
    fn equip_picked(&mut self, player: Owner, item: Guid) -> bool {
        false
    }

    // ---- sockets ---------------------------

    /// Filler properties and owner link.
    fn filler_linked(&mut self, filler: Guid, target: Guid) {}
    /// Runeword check and apply; true = applied.
    fn runeword(&mut self, player: Owner, target: Guid) -> bool {
        false
    }

    // ---- hirelings (not wired yet) -----------------------------------------

    /// The player's hireling.
    fn hireling(&self, player: Owner) -> Option<Owner> {
        None
    }
    /// The player has a used skill. Default: yes (0x61 does nothing).
    fn has_used_skill(&self, player: Owner) -> bool {
        true
    }
    /// Alive (= 0 for players; hireling alive).
    fn alive(&self, u: Owner) -> bool {
        false
    }
    /// The hireling belongs to the player.
    fn owns_hireling(&self, player: Owner, merc: Owner) -> bool {
        false
    }
    /// Equip on the hireling.
    fn equip_on_merc(&mut self, merc: Owner, item: Guid) {}
    /// After a take: hireling inventory pass with 0.
    fn merc_after_take(&mut self, merc: Owner) {}

    // ---- NPC / object / other unit picks (waypoints) -----

    /// 0x16 type 1.
    fn pick_npc(&mut self, player: Owner, guid: Guid, cursor: u32) -> u32 {
        0
    }
    /// 0x16 type 2 (objects).
    fn pick_object(&mut self, player: Owner, guid: Guid, cursor: u32) -> u32 {
        0
    }
    /// Corpse pickup with (game, player, P) steps 1–2 (type 0,
    /// State 7, the take permission, the experience return); true
    /// when the take-back follows. Default: no.
    fn corpse_pickup(&mut self, player: Owner, corpse: Owner) -> bool {
        false
    }
    /// Corpse slot fit of X for `unit` with D (the
    /// unit's item at L) and A (its item at the paired location, or D):
    /// (fit, L after the rule). Default: no fit.
    fn corpse_slot_fit(
        &self,
        unit: Owner,
        x: Guid,
        d: Option<Guid>,
        a: Option<Guid>,
        l: u8,
    ) -> (bool, u8) {
        (false, l)
    }
    /// After a full take-back: C off the player's corpse
    /// list, out of its room, S→C 0x8E
    /// `CorpseAssign` to every player, C
    /// freed. Default: nothing.
    fn corpse_taken(&mut self, player: Owner, corpse: Owner) {}
    /// Replenish timers of an item.
    /// Default: nothing.
    fn replenish_timers(&mut self, item: Guid) {}
    /// Player-to-player interaction (type 0;
    /// multiplayer).
    fn player_interact(&mut self, player: Owner, other: Owner) {}

    // ---- transport -------------------------------------------------------

    /// Queue a message to the player's client now.
    fn send(&mut self, player: Owner, bytes: Vec<u8>) {}
    /// S→C 0x3E for an item stat: the wired desk builds it
    /// with `units::messages::update_item_stat` and sends it through
    /// [`MovePending::send`]. Default: nothing.
    fn send_item_stat(&mut self, player: Owner, item: Guid, stat: u16) {}
    /// Item bit stream with item flags OR-ed with
    /// `flags` and the page shown as `page`. Default: empty.
    fn item_bits(&self, item: Guid, flags: u32, page: u8) -> Vec<u8> {
        Vec::new()
    }
    /// The bit stream of a store item shown to its trading client
    /// (with 0x38): the alt-code record exactly when the item's quality is
    /// 4–9 and it lacks item
    /// flag 0x10. Default: the plain stream.
    fn store_item_bits(&self, item: Guid, page: u8) -> Vec<u8> {
        self.item_bits(item, 0, page)
    }
    /// Store messages 0x38 / 0x39 of the dispatcher's first step.
    /// Default: none.
    fn store_messages(&mut self, client: Owner, item: Guid) -> Vec<Vec<u8>> {
        Vec::new()
    }
}

/// Everything the handlers need.
pub trait MoveWorld: InventoryOps + MoveUnits + MovePending {}
impl<T: InventoryOps + MoveUnits + MovePending> MoveWorld for T {}

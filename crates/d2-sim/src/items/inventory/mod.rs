//! The inventory model and its placement, belt, equip and shared checks
//! records and fields ([`Inventory`], [`Grid`], [`InvItem`]),
//! grids and the grid record by page ([`grid_record`]), the item list and
//! update list, the fit test, place at a position, the free-position
//! search and page placement ([`grid`]), the belt ([`belt`]),
//! body-location compatibility, requirements, the equip check, hands
//! compatible, the stack test, equip from the cursor and auto-equip
//! ([`equip`]), the item checks, busy / trading, the targeting reset and
//! the item-move gate ([`checks`]). Table data is a projection of `d2-data`
//! typed records ([`tables::InvTables`]).
//!
//! Status: implemented, not yet checked against a live game (T1–T9, B1–B5
//! and E1–E4 are synthetic; the D1–D3 data checks need the game files).
//!
//! Randomness: none. No draw happens in the grid, belt, equip or
//! requirement code; the systems these paths call
//! (stat refresh, socketing) are reached through [`InvWorld`].
//!
//! Item units, their stats and every routine owned by another system are
//! reached through the seam [`InvWorld`]. Points not settled yet stop
//! at a seam method (each says what it stands for).

pub mod belt;
pub mod bookkeeping;
pub mod checks;
pub mod equip;
pub mod grid;
pub mod levelreq;
pub mod tables;
pub mod weapon;

#[cfg(test)]
mod tests;

use crate::units::UnitId;

pub use belt::{
    auto_belt_gate, belt_boxes_of, belt_numboxes, belt_removal_allowed, belt_type, beltable,
    compact_belt, free_belt_slot, place_in_belt_slot, similar,
};
pub use checks::{
    active_inventory_item, belt_item_check, busy, cursor_item_check, ground_or_owned_check,
    item_move_gate, owned_item_check, stored_item_check, stored_or_equipped_check, targeting_reset,
    trading, usable, Interaction,
};
pub use equip::{
    auto_equip_compatible, auto_equip_location, body_location_allowed, corpse_slot_fit,
    equip_check, equip_from_cursor, equip_profile, hands_compatible, pair_location,
    requirements_met, stack_quality_ok, stack_test, EquipOutcome, EquipProfile,
};
pub use grid::{
    find_free_position, grid_record, page_grid_size, place_at_body, place_at_page, place_in_grid,
    place_in_page, place_in_page_from_cursor, search, weight,
};
pub use levelreq::{level_requirement, AffixReq, LevelReqItem, LevelReqUnit};
pub use tables::{InvTables, ItemUseTables};

/// Inventory signature (inventory +0x00). Every accessor of the base game
/// checks it; in D2RUST the type system guarantees it, so it is kept as a
/// documented constant only.
pub const SIGNATURE: u32 = 0x0102_0304;

/// "None" GUID (−1): weapon GUID, item owner.
pub const NO_GUID: u32 = u32::MAX;

/// Item unit modes the inventory sets.
pub mod mode {
    pub const STORED: u8 = 0;
    pub const EQUIPPED: u8 = 1;
    pub const BELT: u8 = 2;
    pub const GROUND: u8 = 3;
    pub const CURSOR: u8 = 4;
    pub const SOCKETED: u8 = 6;
}

/// Inventory pages.
pub mod page {
    pub const INVENTORY: u8 = 0;
    pub const TRADE1: u8 = 1;
    pub const TRADE2: u8 = 2;
    pub const CUBE: u8 = 3;
    pub const STASH: u8 = 4;
    pub const NONE: u8 = 0xFF;
}

/// Grid numbers: 0 body locations, 1 belt, 2 + page.
pub mod grid_id {
    pub const BODY: usize = 0;
    pub const BELT: usize = 1;
    /// Grid of page `p` is `PAGE + p`.
    pub const PAGE: usize = 2;
}

/// Constant grid sizes.
pub const BODY_GRID: (u8, u8) = (13, 1);
pub const BELT_GRID: (u8, u8) = (16, 1);

/// Body locations (`bodylocs` codes; the intents accept 1–10).
pub mod body {
    pub const NONE: u8 = 0;
    pub const HEAD: u8 = 1;
    pub const NECK: u8 = 2;
    pub const TORSO: u8 = 3;
    pub const RIGHT_HAND: u8 = 4;
    pub const LEFT_HAND: u8 = 5;
    pub const RIGHT_RING: u8 = 6;
    pub const LEFT_RING: u8 = 7;
    pub const BELT: u8 = 8;
    pub const FEET: u8 = 9;
    pub const GLOVES: u8 = 10;
    pub const SWAP_RIGHT: u8 = 11;
    pub const SWAP_LEFT: u8 = 12;
}

/// Node kinds (item data +0x69).
pub mod node {
    pub const NONE: u8 = 0;
    pub const PAGE: u8 = 1;
    pub const BELT: u8 = 2;
    pub const BODY: u8 = 3;
    pub const SWAP: u8 = 4;
}

/// Command flags (item data +0x14; `items/item-actions.tsv`).
pub mod cmd {
    pub const PUT_IN_CONTAINER: u32 = 0x2;
    pub const REMOVE_FROM_CONTAINER: u32 = 0x4;
    pub const EQUIP: u32 = 0x8;
    pub const UNEQUIP: u32 = 0x10;
    pub const SWAP_BODY: u32 = 0x20;
    pub const GROUND_TO_CURSOR: u32 = 0x40;
    pub const PUT_IN_CONTAINER2: u32 = 0x80;
    pub const ADD_QUANTITY: u32 = 0x100;
    pub const EQUIP2: u32 = 0x200;
    pub const PUT_IN_BELT: u32 = 0x400;
    pub const REMOVE_FROM_BELT: u32 = 0x800;
    pub const SWAP_IN_BELT: u32 = 0x1000;
    pub const PUT_IN_BELT2: u32 = 0x2000;
    pub const AUTO_UNEQUIP: u32 = 0x4000;
    pub const UNKNOWN_14: u32 = 0x8000;
    pub const INDIRECT_SWAP_BODY: u32 = 0x10000;
    pub const SWAP_IN_CONTAINER: u32 = 0x40000;
    pub const UNKNOWN_16: u32 = 0x80000;
    pub const TO_CURSOR: u32 = 0x10_0000;
    pub const WEAPON_SWITCH: u32 = 0x20_0000;
}

/// Item flags (item data +0x18).
pub mod iflag {
    pub const CHANGED: u32 = 0x1;
    pub const TARGETING: u32 = 0x4;
    pub const IDENTIFIED: u32 = 0x10;
    pub const F20: u32 = 0x20;
    pub const F40: u32 = 0x40;
    pub const F80: u32 = 0x80;
    pub const BROKEN: u32 = 0x100;
    pub const REPAIRED: u32 = 0x200;
    pub const F400: u32 = 0x400;
    pub const F4000: u32 = 0x4000;
    pub const F40000: u32 = 0x4_0000;
    pub const ETHEREAL: u32 = 0x40_0000;
}

/// Unit flag cleared on placement: targetable (unit +0xC4 bit 0x2).
pub const UNIT_FLAG_TARGETABLE: u32 = 0x2;

/// Itemtypes rows used here (D3).
pub mod ty {
    pub const BOOK: i16 = 18;
    pub const BELT: i16 = 19;
    pub const TPOT: i16 = 38;
    pub const WEAP: i16 = 45;
    pub const H2H: i16 = 67;
}

/// Stat ids read here.
pub mod stat {
    pub const STRENGTH: u16 = 0;
    pub const DEXTERITY: u16 = 2;
    pub const LEVEL: u16 = 12;
    pub const MINDAMAGE: u16 = 21;
    pub const MAXDAMAGE: u16 = 22;
    pub const SECONDARY_MINDAMAGE: u16 = 23;
    pub const SECONDARY_MAXDAMAGE: u16 = 24;
    pub const QUANTITY: u16 = 70;
    pub const ITEM_REQ_PERCENT: u16 = 91;
    pub const THROW_MINDAMAGE: u16 = 159;
    pub const THROW_MAXDAMAGE: u16 = 160;
}

/// The kind of a unit, as the inventory rules distinguish it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitKind {
    /// Player, with its class (0–6).
    Player { class: u8 },
    /// Monster (NPCs, hirelings), with its class (monstats row).
    Monster { class: u32 },
    /// Object, with its class.
    Object { class: u32 },
    /// An item (an item with sockets owns an inventory).
    Item,
    /// Missiles, tiles.
    Other,
}

impl UnitKind {
    pub fn is_player(self) -> bool {
        matches!(self, UnitKind::Player { .. })
    }
}

/// The item data fields the inventory reads and writes, held by the
/// item unit's owner (the wiring keeps one per item unit).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvItem {
    /// The item unit's GUID.
    pub guid: u32,
    /// Item class: combined items index.
    pub record: usize,
    /// Unit mode ([`mode`]).
    pub mode: u8,
    /// Command flags (+0x14).
    pub cmd_flags: u32,
    /// Item flags (+0x18).
    pub flags: u32,
    /// Body location (+0x44).
    pub body_loc: u8,
    /// Page (+0x45; 0xFF none).
    pub page: u8,
    /// Stored page (+0x47).
    pub stored_page: u8,
    /// Position: grid cell (x, y).
    pub x: i32,
    pub y: i32,
    /// Item owner: the owning player's GUID, [`NO_GUID`] otherwise.
    pub owner_guid: u32,
    /// Owning inventory (+0x5C), by the inventory's owner unit.
    pub inv: Option<UnitId>,
    /// Grid + 1 (+0x68; 0 = not in a grid).
    pub node_grid: u8,
    /// Node kind (+0x69, [`node`]).
    pub node_kind: u8,
}

impl InvItem {
    /// A fresh item record: mode 3 (ground), page none, not linked.
    pub fn new(guid: u32, record: usize) -> Self {
        Self {
            guid,
            record,
            mode: mode::GROUND,
            cmd_flags: 0,
            flags: 0,
            body_loc: body::NONE,
            page: page::NONE,
            stored_page: 0,
            x: 0,
            y: 0,
            owner_guid: NO_GUID,
            inv: None,
            node_grid: 0,
            node_kind: node::NONE,
        }
    }
}

/// One grid: its own item list and its cells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grid {
    pub width: u8,
    pub height: u8,
    /// The grid's own item list, in link order.
    pub items: Vec<UnitId>,
    /// Cells, row-major: cell (x, y) = y × width + x.
    pub cells: Vec<Option<UnitId>>,
}

impl Grid {
    /// A grid with zeroed cells.
    pub fn new(width: u8, height: u8) -> Self {
        Self {
            width,
            height,
            items: Vec::new(),
            cells: vec![None; usize::from(width) * usize::from(height)],
        }
    }

    /// The item in cell (x, y); out of bounds → none.
    pub fn cell(&self, x: i32, y: i32) -> Option<UnitId> {
        if x < 0 || y < 0 || x >= i32::from(self.width) || y >= i32::from(self.height) {
            return None;
        }
        self.cells[y as usize * usize::from(self.width) + x as usize]
    }

    fn set_rect(&mut self, x: i32, y: i32, w: u8, h: u8, v: Option<UnitId>) {
        // Signed loops with a wrapping end, as the fit test.
        for yy in y..y.wrapping_add(i32::from(h)) {
            for xx in x..x.wrapping_add(i32::from(w)) {
                if xx >= 0 && yy >= 0 && xx < i32::from(self.width) && yy < i32::from(self.height) {
                    self.cells[yy as usize * usize::from(self.width) + xx as usize] = v;
                }
            }
        }
    }

    /// Clears every cell holding `item`.
    fn clear_item(&mut self, item: UnitId) {
        for c in &mut self.cells {
            if *c == Some(item) {
                *c = None;
            }
        }
    }
}

/// The inventory record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inventory {
    /// Owner unit (+0x08).
    pub owner: UnitId,
    pub owner_kind: UnitKind,
    /// The owner's GUID (written as item owner when the owner is a player).
    pub owner_guid: u32,
    /// Item list (+0x0C / +0x10), in link order.
    items: Vec<UnitId>,
    /// Grid array (+0x14 / +0x18), indexed by grid number.
    grids: Vec<Option<Grid>>,
    /// GUID of the weapon in use (+0x1C; [`NO_GUID`] none).
    pub weapon_guid: u32,
    /// Cursor item (+0x20).
    cursor: Option<UnitId>,
    /// Count of linked items (+0x28).
    pub count: u32,
    /// Update list (+0x2C / +0x30): GUIDs in append order.
    updates: Vec<u32>,
}

impl Inventory {
    pub fn new(owner: UnitId, owner_kind: UnitKind, owner_guid: u32) -> Self {
        Self {
            owner,
            owner_kind,
            owner_guid,
            items: Vec::new(),
            grids: Vec::new(),
            weapon_guid: NO_GUID,
            cursor: None,
            count: 0,
            updates: Vec::new(),
        }
    }

    /// The item list, in link order.
    pub fn items(&self) -> &[UnitId] {
        &self.items
    }

    /// Whether `item` is in the item list.
    pub fn contains(&self, item: UnitId) -> bool {
        self.items.contains(&item)
    }

    /// Grid `g`, when created.
    pub fn grid(&self, g: usize) -> Option<&Grid> {
        self.grids.get(g).and_then(Option::as_ref)
    }

    /// Grid count (+0x18).
    pub fn grid_count(&self) -> usize {
        self.grids.len()
    }

    /// Grid `g`, created on first use with the size given;
    /// a later use with a different size fails.
    pub fn grid_or_create(&mut self, g: usize, width: u8, height: u8) -> Option<&mut Grid> {
        if self.grids.len() <= g {
            self.grids.resize(g + 1, None);
        }
        let slot = &mut self.grids[g];
        match slot {
            Some(gr) if gr.width != width || gr.height != height => None,
            Some(gr) => Some(gr),
            None => Some(slot.insert(Grid::new(width, height))),
        }
    }

    /// The item in cell (x, y) of grid `g`.
    pub fn item_at(&self, g: usize, x: i32, y: i32) -> Option<UnitId> {
        self.grid(g).and_then(|gr| gr.cell(x, y))
    }

    /// The item at body location `loc`: grid 0, x = loc.
    pub fn body_item(&self, loc: u8) -> Option<UnitId> {
        self.item_at(grid_id::BODY, i32::from(loc), 0)
    }

    /// The item in belt slot `slot`: grid 1, x = slot.
    pub fn belt_item(&self, slot: u8) -> Option<UnitId> {
        self.item_at(grid_id::BELT, i32::from(slot), 0)
    }

    /// Cursor item.
    pub fn cursor(&self) -> Option<UnitId> {
        self.cursor
    }

    /// Writes the cursor field (inventory +0x20) only. Fixtures use it to
    /// stage a state; the cursor rule itself is [`Inventory::put_cursor`].
    pub fn set_cursor(&mut self, item: Option<UnitId>) {
        self.cursor = item;
    }

    /// Set the cursor. With an item: inventory
    /// +0x20 := item, item data +0x5C := this inventory; no list link, no
    /// count change. With none: the current cursor item, if any, is
    /// unlinked (on the cursor item: only the cursor field is
    /// cleared, then its node fields, owning inventory and a matching
    /// weapon GUID).
    pub fn put_cursor<W: InvWorld + ?Sized>(&mut self, w: &mut W, item: Option<UnitId>) {
        match item {
            Some(i) => {
                let Some(d) = w.item_mut(i) else {
                    return;
                };
                d.inv = Some(self.owner);
                self.cursor = Some(i);
            }
            None => {
                if let Some(c) = self.cursor {
                    self.unlink(w, c);
                }
                self.cursor = None;
            }
        }
    }

    /// The update list (GUIDs, append order).
    pub fn update_list(&self) -> &[u32] {
        &self.updates
    }

    /// Whether a GUID is on the update list.
    pub fn is_listed(&self, guid: u32) -> bool {
        self.updates.contains(&guid)
    }

    /// Appends a GUID to the update list unless already listed.
    pub fn push_update(&mut self, guid: u32) {
        if !self.is_listed(guid) {
            self.updates.push(guid);
        }
    }

    /// Takes the update list, leaving it empty (the list free
    /// of the update-list reset).
    pub fn take_updates(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.updates)
    }

    /// Links an item: appends it at the tail of the item
    /// list and, for an item placed in grid `grid`, to that grid's list.
    /// Sets the item's owning inventory.
    pub fn link<W: InvWorld + ?Sized>(&mut self, w: &mut W, item: UnitId, grid: Option<usize>) {
        self.items.push(item);
        if let Some(Some(g)) = grid.and_then(|g| self.grids.get_mut(g)) {
            g.items.push(item);
        }
        if let Some(d) = w.item_mut(item) {
            d.inv = Some(self.owner);
        }
    }

    /// Unlinks an item: clears its cells (if it has a grid),
    /// clears the cursor if it was the cursor item (else the count drops
    /// by 1), clears the weapon GUID if it matches, zeroes the node fields.
    /// Returns false when the item is not in this inventory (the callers
    /// treat that as fatal).
    ///
    /// The cursor item is not in the item list: its unlink
    /// clears the cursor field and its node fields only.
    pub fn unlink<W: InvWorld + ?Sized>(&mut self, w: &mut W, item: UnitId) -> bool {
        match self.items.iter().position(|&i| i == item) {
            Some(pos) => {
                self.items.remove(pos);
            }
            None if self.cursor == Some(item) => {}
            None => return false,
        }
        let Some(d) = w.item_mut(item) else {
            return false;
        };
        if d.node_grid > 0 {
            if let Some(Some(g)) = self.grids.get_mut(usize::from(d.node_grid - 1)) {
                g.clear_item(item);
                g.items.retain(|&i| i != item);
            }
        }
        if self.cursor == Some(item) {
            self.cursor = None;
        } else {
            self.count = self.count.wrapping_sub(1);
        }
        if self.weapon_guid == d.guid {
            self.weapon_guid = NO_GUID;
        }
        d.node_grid = 0;
        d.node_kind = node::NONE;
        d.inv = None;
        true
    }
}

/// Interaction of a player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractionTarget {
    None,
    /// The interaction's unit no longer exists.
    Missing,
    /// A unit of this type (0 player, 1 monster, …).
    Unit {
        ty: u8,
        unit: UnitId,
    },
}

/// Seam: item units, unit stats and the routines of other specs that the
/// inventory paths call. Expected provider: the wiring of item units
/// with stats.
pub trait InvWorld {
    // --- Game.
    /// Expansion game (game +0x70).
    fn expansion(&self) -> bool;

    // --- Item units.
    /// The item data of an item unit (none: not an item / missing).
    fn item(&self, item: UnitId) -> Option<&InvItem>;
    fn item_mut(&mut self, item: UnitId) -> Option<&mut InvItem>;
    /// Unit lookup type 4 by GUID.
    fn item_by_guid(&self, guid: u32) -> Option<UnitId>;
    /// An item stat total.
    fn item_stat(&self, item: UnitId, stat: u16) -> i32;
    /// Unlinks `item` from the inventory of `owner` (an inventory other
    /// than the one placing it).
    fn unlink_from(&mut self, owner: UnitId, item: UnitId);
    /// Removes an item in mode 3 from its room (room delete notice
    /// Collision freed, room list).
    fn remove_from_room(&mut self, item: UnitId);
    /// Clears unit flag 0x2 (targetable, unit +0xC4) of the item.
    fn clear_targetable(&mut self, item: UnitId);

    // --- Owner-side routines.
    /// Link check: sockets an item when the
    /// inventory belongs to an item, else succeeds.
    fn link_check(&mut self, owner: UnitId, item: UnitId, kind: u8) -> bool;
    /// Item-skill link (scroll / tome charges into the
    /// player's skill quantity; not charms).
    fn charm_relink(&mut self, owner: UnitId, item: UnitId);
    /// "Active inventory item for its owner".
    fn active_item(&self, owner: UnitId, item: UnitId) -> bool;
    /// Stat refresh.
    fn stat_refresh(&mut self, owner: UnitId);
    /// Socketed with fillers.
    fn socket_filled(&self, item: UnitId) -> bool;
    /// Owner refresh.
    fn owner_refresh(&mut self, owner: UnitId);
    /// Inventory pass.
    fn inventory_pass(&mut self, owner: UnitId);
    /// Trade hook (page 2; multiplayer trade, out of scope).
    fn trade_hook(&mut self, owner: UnitId, item: UnitId);
    /// Weapon-in-use update.
    fn weapon_in_use_update(&mut self, unit: UnitId);
    /// Stat link.
    fn stat_link(&mut self, unit: UnitId, item: UnitId);
    /// [`Self::stat_link`] from an inventory function that holds the
    /// owner's inventory, with that inventory's weapon in use (+0x1C,
    /// after the weapon link): the stat refresh reads it.
    /// Default: the plain link.
    fn stat_link_in_use(&mut self, unit: UnitId, item: UnitId, weapon_guid: u32) {
        let _ = weapon_guid;
        self.stat_link(unit, item)
    }
    /// Weapon bookkeeping.
    fn weapon_bookkeeping(&mut self, unit: UnitId, item: UnitId);

    // --- Units (requirements, hands, checks).
    /// Kind of a unit; none when it does not exist.
    fn unit_kind(&self, unit: UnitId) -> Option<UnitKind>;
    /// A unit stat total (strength 0, dexterity 2, level 12).
    fn unit_stat(&self, unit: UnitId, stat: u16) -> i32;

    // --- Requirements.
    /// Item stat 91 `item_req_percent`, item or skill stat.
    fn req_percent(&self, item: UnitId) -> i32;
    /// The item is active on the unit.
    fn item_active_on(&self, item: UnitId, unit: UnitId) -> bool;
    /// The item's own contribution to a unit stat.
    fn own_contribution(&self, item: UnitId, unit: UnitId, stat: u16) -> i32;
    /// Level requirement ([`levelreq::level_requirement`]
    /// on the values the provider reads; never negative).
    fn level_requirement(&self, item: UnitId, unit: UnitId) -> i32;

    // --- Hands.
    /// Two-handed.
    fn two_handed(&self, item: UnitId) -> bool;
    /// One-or-two-handed for the unit.
    fn one_or_two_handed(&self, unit: UnitId, item: UnitId) -> bool;
    /// Ammo type of an item: an itemtypes row, none.
    fn ammo_type(&self, item: UnitId) -> Option<i16>;
    /// Quiver type (itemtype
    /// `quiver` ≠ 0); a default from the tables is given.
    fn quiver_type(&self, t: &InvTables, item: UnitId) -> bool {
        self.item(item)
            .and_then(|d| t.itype_of(d.record))
            .is_some_and(|r| r.quiver != 0)
    }
    /// "X fits a free position of page 0".
    fn fits_free_page0(&self, inv: &Inventory, item: UnitId) -> bool;

    // --- Stack test.
    /// Quality (item data +0).
    fn quality(&self, item: UnitId) -> u8;
    /// Unique/set file index (item data +0x28).
    fn stack_file_index(&self, item: UnitId) -> i32;
    /// Has sockets (≠ 0).
    fn has_sockets(&self, item: UnitId) -> bool;

    // --- Auto-equip.
    /// Allowed location.
    fn has_allowed_location(&self, item: UnitId) -> bool;
    /// Quiver-type item (≠ 0).
    fn quiver_kind(&self, item: UnitId) -> bool;

    // --- Targeting reset.
    /// of the targeted unit: the inventory's owner, never the
    /// item (unit type, 6 for a missing unit; 0 → queue 0x3F).
    fn targeting_probe(&self, unit: UnitId) -> u32;
    /// Queue S→C 0x3F (code 0xFF, the item's GUID, 0xFFFF).
    fn queue_untarget(&mut self, player: UnitId, item_guid: u32);

    // --- Players.
    /// The player's interaction.
    fn interaction(&self, player: UnitId) -> InteractionTarget;
    /// Clears the interaction.
    fn clear_interaction(&mut self, player: UnitId);
    /// Player data +0x4C.
    fn player_data_4c(&self, player: UnitId) -> u32;
    /// Player data +0x50.
    fn player_data_50(&self, player: UnitId) -> u32;
    /// The player is in the NPC's interaction list with state 0
    /// (talking).
    fn npc_talking(&self, npc: UnitId, player: UnitId) -> bool;
    /// The player-trade part of the busy test (multiplayer, out of scope):
    /// `Some(allowed)` when a player trade decides, `None` otherwise.
    fn player_trade_gate(&self, player: UnitId) -> Option<bool>;
    /// The item and the player are in the same act.
    fn same_act(&self, player: UnitId, item: UnitId) -> bool;
    /// Within `range` subtiles per axis.
    fn within_range(&self, player: UnitId, item: UnitId, range: i32) -> bool;
}

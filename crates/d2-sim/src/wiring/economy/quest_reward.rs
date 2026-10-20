//! The quest reward on the host's inventory model: the item created from its
//! code, its durability filled, placed in the player's inventory or, when it
//! does not fit, dropped next to the player (with the ground placement) or
//! freed.
//!
//! The inventory model is the host's ([`QuestInventory`]): a quest call
//! reaches it only when the host lends it ([`super::HostQuests::inventory`]);
//! [`QuestInv`] is the inventory wiring's provider of it
//! (`crate::wiring::inventory`). The free-spot search is the quest
//! helper's ([`crate::world::quests::helpers::free_spot`]), run by
//! [`super::HostQuests`] between the two halves here.
//!
//! Status: wired, not yet checked against a quest reward in play.

use super::{Economy, EconomyError, ItemSpawn, UnitStats};
use crate::items::bitstream::read::record_of;
use crate::items::inventory::{InvTables, InvWorld};
use crate::items::moves::{ground, Spot};
use crate::items::{flag, stat, CreateError, ItemGame, ItemRequest, ItemStats};
use crate::units::lifecycle::LifecycleHooks;
use crate::units::{UnitId, UnitType};
use crate::wiring::inventory::{InvDesk, InvRest, InvState};

/// Unit mode of an item created into an inventory (spawn mode 4 =
/// inventory / cursor).
const MODE_INVENTORY: u32 = 4;
/// The reward drop search: size 1, mask 0x3E01, limit 100.
pub const DROP_SIZE: u32 = 1;
/// Collision mask of the reward drop search.
pub const DROP_MASK: u32 = 0x3E01;
/// Search limit of the reward drop search.
pub const DROP_LIMIT: u32 = 100;

/// The host's inventory model as a quest call sees it, on the call's
/// economy (one inventory per unit: the item moves', the vendors' and the
/// cube's).
pub trait QuestInventory<H> {
    /// Places the item in the player's inventory (finding a free spot,
    /// telling the client) in the page the item data names; true when
    /// placed.
    fn place(&mut self, econ: &mut Economy<'_, H>, player: UnitId, item: UnitId) -> bool;
    /// The ground placement of `item` at `spot`.
    fn drop_at(&mut self, econ: &mut Economy<'_, H>, item: UnitId, spot: Spot);
    /// The inventory pass on the player.
    fn inventory_pass(&mut self, econ: &mut Economy<'_, H>, player: UnitId);
    /// An economy error of a quest call's item creation (the host keeps
    /// it with its other provider errors).
    fn fault(&mut self, error: EconomyError);
    /// The player's item list in link order; the quest item search reads
    /// it.
    fn items_of(&self, player: UnitId) -> Vec<UnitId>;
    /// The player's cursor item.
    fn cursor_of(&self, player: UnitId) -> Option<UnitId>;
    /// The weapon in use.
    fn weapon_in_use(&self, econ: &Economy<'_, H>, player: UnitId) -> Option<UnitId>;
    /// The item leaves the player (the client is told) and is freed.
    fn delete(&mut self, econ: &mut Economy<'_, H>, player: UnitId, item: UnitId);
    /// The GUIDs on the player's inventory update list.
    fn update_guids(&self, player: UnitId) -> Vec<u32>;
    /// A gold pile of `amount` at `unit` (one pile). False: the model has no gold piles (the caller
    /// keeps the rest's answer).
    fn gold_pile(&mut self, econ: &mut Economy<'_, H>, unit: UnitId, amount: i32) -> bool {
        let _ = (econ, unit, amount);
        false
    }
}

/// [`QuestInventory`] on the inventory wiring: an [`InvDesk`] over the
/// call's economy for each call, with the host's inventory tables, state
/// and rest. Creation errors are kept in [`QuestInv::errors`].
pub struct QuestInv<'d, R: ?Sized> {
    /// The inventory tables.
    pub tables: &'d InvTables,
    /// The inventory state.
    pub state: &'d mut InvState,
    /// The inventory wiring's rest.
    pub rest: &'d mut R,
    /// Creation errors of the quest calls.
    pub errors: Vec<EconomyError>,
}

impl<'d, R: ?Sized> QuestInv<'d, R> {
    /// An inventory model over the given tables, state and rest.
    pub fn new(tables: &'d InvTables, state: &'d mut InvState, rest: &'d mut R) -> Self {
        Self {
            tables,
            state,
            rest,
            errors: Vec::new(),
        }
    }
}

impl<H: LifecycleHooks, R: InvRest + ?Sized> QuestInventory<H> for QuestInv<'_, R> {
    fn place(&mut self, econ: &mut Economy<'_, H>, player: UnitId, item: UnitId) -> bool {
        InvDesk::new(econ, self.tables, self.state, self.rest).place(
            player,
            item,
            (0, 0),
            true,
            true,
        )
    }
    fn drop_at(&mut self, econ: &mut Economy<'_, H>, item: UnitId, spot: Spot) {
        let mut d = InvDesk::new(econ, self.tables, self.state, self.rest);
        let g = d.guid_of(item);
        ground::ground_place(&mut d, g, spot);
        d.sync_out();
    }
    fn inventory_pass(&mut self, econ: &mut Economy<'_, H>, player: UnitId) {
        let mut d = InvDesk::new(econ, self.tables, self.state, self.rest);
        InvWorld::inventory_pass(&mut d, player);
    }
    fn fault(&mut self, error: EconomyError) {
        self.errors.push(error);
    }
    fn items_of(&self, player: UnitId) -> Vec<UnitId> {
        self.state.items_of(player)
    }
    fn cursor_of(&self, player: UnitId) -> Option<UnitId> {
        self.state.cursor_of(player)
    }
    fn update_guids(&self, player: UnitId) -> Vec<u32> {
        self.state
            .of(player)
            .map(|i| i.update_list().to_vec())
            .unwrap_or_default()
    }
    /// The item at body location 5, then 4, of item type `weap` whose GUID
    /// is the inventory's weapon GUID; none while that GUID is unset. The
    /// body link and unlink write it ([`crate::items::inventory::weapon`]).
    fn weapon_in_use(&self, econ: &Economy<'_, H>, player: UnitId) -> Option<UnitId> {
        let inv = self.state.of(player)?;
        let guid = inv.weapon_guid;
        if guid == crate::items::inventory::NO_GUID {
            return None;
        }
        [5, 4]
            .into_iter()
            .filter_map(|loc| inv.item_at(crate::items::inventory::grid_id::BODY, loc, 0))
            .filter(|&i| {
                econ.items
                    .get(i)
                    .is_some_and(|d| self.tables.is_type(d.record, crate::items::ty::WEAP as i16))
            })
            .find(|&i| econ.units.get(i).is_some_and(|r| r.guid == guid))
    }
    fn delete(&mut self, econ: &mut Economy<'_, H>, player: UnitId, item: UnitId) {
        InvDesk::new(econ, self.tables, self.state, self.rest).delete_held_item(player, item);
    }
    /// One pile ([`ground::gold_piles`], max 1), as the gold pickup's rest
    /// pile.
    fn gold_pile(&mut self, econ: &mut Economy<'_, H>, unit: UnitId, amount: i32) -> bool {
        let mut d = InvDesk::new(econ, self.tables, self.state, self.rest);
        let Some(o) = d.owner_of(unit) else {
            return false;
        };
        ground::gold_piles(&mut d, o, amount, 1);
        d.sync_out();
        true
    }
}

/// The item level for a unit: a player's base stat 12, a monster's total
/// stat 12, at least 1. `None`: another unit type (its room level's
/// `MonLvl`, not read here).
pub fn item_level<H>(econ: &Economy<'_, H>, unit: UnitId) -> Option<i32> {
    let v = match econ.units.get(unit)?.ty {
        UnitType::Player => econ.stats.unit_base(unit, stat::LEVEL, 0),
        UnitType::Monster => econ.stats.unit_total(unit, stat::LEVEL, 0),
        _ => return None,
    };
    Some(v.max(1))
}

/// The item's max durability: 0 when the item has no base stat 73, else its
/// total stat 73.
///
/// "No base-array entry" is base stat 73 = 0: a base array never holds a
/// value 0.
pub(super) fn max_durability<H>(econ: &Economy<'_, H>, item: UnitId) -> i32 {
    if econ.stats.unit_base(item, stat::MAXDURABILITY, 0) == 0 {
        return 0;
    }
    econ.stats.unit_total(item, stat::MAXDURABILITY, 0)
}

/// The first half of a reward: the code lookup (absent gives none), the level
/// (`level` ≠ 0, else the player's [`item_level`]), the item creation and the
/// item made ready for placement: durability := max durability when that
/// is positive, inventory page 0. `Ok(None)`: no item (unknown code, or the
/// creation refused it).
///
/// The creation arguments: source unit = the player (class and level; no
/// player data, which only ears read), spawn mode 4 (inventory), no-sockets
/// 0, never-ethereal 1, no seeds ("use seed" 0), so a reward draws no
/// ethereal roll.
pub fn create_reward<H: LifecycleHooks>(
    econ: &mut Economy<'_, H>,
    player: UnitId,
    code: [u8; 4],
    level: i32,
    quality: u8,
) -> Result<Option<UnitId>, EconomyError> {
    create_reward_flags(
        econ,
        player,
        code,
        level,
        quality,
        crate::items::req::NEVER_ETHEREAL,
    )
}

/// [`create_reward`] with request `flags2` bits: a start item is made with
/// `NO_SOCKETS | NEVER_ETHEREAL` (spawn mode 4, quality 2, no-sockets 1,
/// never-ethereal 1), so it draws no ethereal roll from the item seed.
pub fn create_reward_flags<H: LifecycleHooks>(
    econ: &mut Economy<'_, H>,
    player: UnitId,
    code: [u8; 4],
    level: i32,
    quality: u8,
    flags2: u32,
) -> Result<Option<UnitId>, EconomyError> {
    let Some(rec) = record_of(econ.tables, code) else {
        return Ok(None);
    };
    let level = if level != 0 {
        level
    } else {
        item_level(econ, player).unwrap_or(1)
    };
    let mut rq = ItemRequest {
        unit: econ.request_unit(player, None),
        // An item level of 0 or less becomes 1.
        ilvl: level.max(1),
        item: rec as i32,
        format: ItemGame::item_format(&*econ.fields),
        quality,
        flags2,
        ..ItemRequest::default()
    };
    let spawn = ItemSpawn {
        room: None,
        mode: MODE_INVENTORY,
        init_flags: 1,
    };
    let item = match econ.create_item(&mut rq, false, spawn) {
        Ok(u) => u,
        Err(EconomyError::Create(CreateError::Fatal(f))) => return Err(f.into()),
        Err(EconomyError::Create(_)) => return Ok(None),
        Err(e) => return Err(e),
    };
    // On success the item is identified (flag 0x10).
    if let Some(i) = econ.items.get_mut(item) {
        i.flags |= flag::IDENTIFIED;
    }
    let max = max_durability(econ, item);
    if max > 0 {
        econ.with_stats(|ctx| UnitStats::new(ctx, item).set_base(stat::DURABILITY, 0, max));
    }
    if let Some(i) = econ.items.get_mut(item) {
        i.inv_page = 0;
    }
    Ok(Some(item))
}

/// What placing a created reward did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placed {
    /// In the player's inventory (identified).
    Stored(UnitId),
    /// The placement failed: the caller drops or frees it.
    Refused(UnitId),
}

/// The second half of a reward, up to the drop: place the created item;
/// when placed, identify it unless it already is.
pub fn place_reward<H: LifecycleHooks>(
    econ: &mut Economy<'_, H>,
    inv: &mut dyn QuestInventory<H>,
    player: UnitId,
    item: UnitId,
) -> Placed {
    if !inv.place(econ, player, item) {
        return Placed::Refused(item);
    }
    if let Some(i) = econ.items.get_mut(item) {
        i.flags |= flag::IDENTIFIED;
    }
    Placed::Stored(item)
}

/// The end of a reward for an item the placement refused: droppable with a
/// spot gives dropped there and returned; otherwise freed and none.
pub fn drop_or_free<H: LifecycleHooks>(
    econ: &mut Economy<'_, H>,
    inv: &mut dyn QuestInventory<H>,
    item: UnitId,
    spot: Option<Spot>,
) -> Option<UnitId> {
    if let Some(s) = spot {
        inv.drop_at(econ, item, s);
        return Some(item);
    }
    if let Err(e) = econ.free_item(item) {
        inv.fault(e);
    }
    None
}

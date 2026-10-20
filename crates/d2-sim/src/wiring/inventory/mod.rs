//! The inventory seams on their real providers: the item-move code
//! (`items::moves`) runs on the inventory model
//! (`items::inventory`), the unit records and lists, the stat lists
//! and the economy's item store.
//!
//! - [`InvDesk`] implements [`crate::items::inventory::InvWorld`]
//!   ([`inv_world`]), [`crate::items::moves::InventoryOps`] ([`ops`]),
//!   [`crate::items::moves::MoveUnits`] ([`units`]) and
//!   [`crate::items::moves::MovePending`] ([`pending`]): every move
//!   handler (`items::moves::handle`) and the per-client update pass
//!   (`items::moves::player_update`) run on it.
//! - [`host`]: the model for the other item systems (vendors, the cube):
//!   reads on [`InvState`], placement / removal / checks on [`InvDesk`],
//!   keyed by unit, so one inventory per owner serves every system.
//! - [`InvRest`]: the calls without a d2-sim provider (positions and the
//!   free-spot search of the path placement, player data, NPC
//!   interaction, item use, sockets, hirelings, sounds, transport, and
//!   the steps the inventory rules leave open).
//!
//! Ownership, one owner per field: the unit record holds the mode,
//! the unit flags and the update bits; the
//! economy's [`ItemStore`] holds the item flags, the page,
//! quality and file index; the stat lists hold the stats;
//! [`InvState`] holds the inventories and the item data fields only this
//! module uses (command flags, body location, stored page, grid position,
//! item owner, owning inventory, node fields, ground expiry). The
//! [`InvItem`] the inventory functions read is a copy of the first three
//! owners' fields plus [`InvState`]'s: [`InvDesk::new`] fills it, the
//! [`crate::items::moves::MoveUnits`] setters write through, and every
//! mutating inventory call writes it back.
//!
//! Status: wired; the move rules are not yet checked against play
//! sessions. Nothing here decides behaviour: rules stay in the
//! modules; an adapter maps a seam call to a provider call.

pub mod bits;
pub mod copy;
pub mod cube_open;
pub mod equip_rules;
pub mod host;
pub mod identify;
pub mod inv_world;
pub mod item_link;
pub mod load;
pub mod merc;
pub mod ops;
pub mod pending;
pub mod potion;
pub mod queries;
pub mod save_index;
pub mod swap;
pub mod town_portal;
pub mod units;

#[cfg(test)]
pub(crate) mod tests;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use super::economy::{Economy, EconomyError, ItemSpawn};
use crate::items::bitstream::WriteBack;
use crate::items::inventory::{InvItem, InvTables, Inventory, UnitKind};
use crate::items::moves::{deferred, Guid, MovePending, Owner};
use crate::items::ItemRequest;
use crate::units::lifecycle::LifecycleHooks;
use crate::units::lists::ListError;
use crate::units::{UnitId, UnitType};

/// What went wrong in an adapter (a provider's error), in order. The
/// handlers' own fatal asserts are `items::moves::MoveFatal`.
#[derive(Debug, PartialEq, Eq)]
pub enum InvError {
    Economy(EconomyError),
    List(ListError),
    /// A fatal assert of an item-move rule run outside a move handler
    /// (e.g. the sell's direct 0x9D).
    Move(crate::items::moves::MoveFatal),
    /// Ground placement found the item in another room than the spot's
    /// ("room added" is written for an item in no room).
    OtherRoom(UnitId),
    /// The body unlink found the body slot still holding an item after the
    /// unlink (the model clears cells only through its own unlink).
    BodySlotHeld(UnitId, u8),
    /// A step whose rule is not settled (named); the call stops
    /// there.
    Unwritten(&'static str),
    /// The item copy was given a source on the ground
    /// (a vendor copy must come from an inventory): a caller error; no copy.
    GroundCopySource(UnitId),
    /// Placement into a page was given an item that is
    /// still in a room: a caller error;
    /// not placed.
    PlacedWithRoom(UnitId),
    /// A fatal assert of the equipment bookkeeping (charms, weapon slots;
    /// [`equip_rules`]).
    Equip(crate::items::inventory::bookkeeping::EquipFatal),
}

/// The inventory state of a game: one [`Inventory`] per unit that owns
/// one (players, NPCs, hirelings, items with fillers) and the item data
/// fields of [`InvItem`] that this module owns.
#[derive(Debug, Default)]
pub struct InvState {
    pub inventories: BTreeMap<UnitId, Inventory>,
    /// The gamble lists' own inventories, one per (NPC, player GUID) node
    /// (each player sees their own gamble list: the node's inventory, not the
    /// NPC's store grid).
    pub gamble: BTreeMap<(UnitId, u32), Inventory>,
    /// Item data per item unit. Mode, item flags, page, GUID and
    /// record are copies of the unit record's and the item store's.
    pub items: BTreeMap<UnitId, InvItem>,
    /// Ground expiry (when an item on the ground disappears).
    pub expiry: BTreeMap<UnitId, i32>,
    pub errors: Vec<InvError>,
    /// The equipment rules run on the desk ([`equip_rules`]: item-skill
    /// link, inventory pass, weapon bookkeeping, the set-item update) in
    /// place of the [`InvRest`] calls of the same addresses. Off (the
    /// default): those calls go to the rest, as before; a host turns it
    /// on when its rest answers the equipment seams (the skill list,
    /// player data mouse slots, stat links).
    pub equip_rules: bool,
    /// The item-move effects run on the desk in place of the [`InvRest`]
    /// calls: the gold rest pile
    /// (leftover gold of a move) and the quest-chain notice 0x5D
    /// (a used quest item). Off (the default): those
    /// calls go to the rest, as before.
    pub move_effects: bool,
    /// An item moved onto a body slot attaches its stat list to the
    /// wearer, and its leaving detaches it, with the set-item update and
    /// the set bonuses ([`item_link`]). Off (the default):
    /// those calls go to the rest, as before.
    pub link_item_stats: bool,
    /// Town Portal scroll / tome uses of the call, taken by the host
    /// ([`InvDesk::take_portal_requests`]: the Town Portal cast,
    /// past its town refusal).
    pub portal_requests: Vec<UnitId>,
    /// The walks to a ground item the pick-ups of the call asked for
    /// (a pick-up out of reach walks first: player, item, cursor flag), taken by
    /// the host that runs them ([`InvDesk::take_item_walks`]).
    pub item_walks: Vec<(UnitId, UnitId, bool)>,
    /// Items placed on the ground
    /// since the host last took them ([`InvDesk::take_dropped`]):
    /// their unit flag 0x1000 ("dropped") is still set when the client
    /// pass of the tick sends the 0x9C action 2, but the D2RUST update
    /// pass runs after the room clean-up has cleared it.
    pub dropped: BTreeSet<UnitId>,
    /// Equipment-rule calls the inventory functions asked for while the
    /// owner's inventory was lent to them ([`InvDesk::with_inv`]); run
    /// when the call returns, before the owner refreshes.
    equip_queue: Vec<equip_rules::EquipCall>,
    /// Owner refreshes asked by the inventory functions during a call;
    /// run (`items::moves::owner_refresh`) when the call returns.
    refresh: Vec<UnitId>,
    /// The bit-stream writer's changes to items (fields the stream
    /// normalizes as it writes), queued by the read-only stream seam and
    /// written by [`InvDesk::apply_write_backs`].
    write_backs: RefCell<Vec<(UnitId, WriteBack)>>,
    /// The game's hireling lists, lent by the
    /// host that holds them for a call (the 0x61 give's hireling,
    /// owner test and swap, [`merc`]); read only here. `None` (the
    /// default): those seams stay the rest's.
    pub hirelings: Option<crate::world::hirelings::HirelingState>,
}

impl InvState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Gives `unit` an inventory (the owner kind decides
    /// the grid records).
    pub fn add_inventory(&mut self, unit: UnitId, kind: UnitKind, guid: u32) {
        self.inventories
            .entry(unit)
            .or_insert_with(|| Inventory::new(unit, kind, guid));
    }
}

/// The inventory calls without a d2-sim provider. [`MovePending`] carries
/// the item-move ones (with their narrowest defaults); the methods here
/// are the inventory model's (`items::inventory::InvWorld`) and the unit
/// fields that no d2-sim module holds yet. Methods without a default are
/// open points of the inventory rules the host must answer.
#[allow(unused_variables)]
pub trait InvRest: MovePending {
    // ---- positions (the path placement) ------------------------------------

    /// Position of a non-item unit (subtiles). Items keep theirs in the
    /// item data (ground placement writes them).
    fn pos(&self, u: Owner) -> (i32, i32) {
        (0, 0)
    }
    fn set_pos(&mut self, u: Owner, x: i32, y: i32) {}

    // ---- item creation (the economy) ----------------------------------------

    /// The creation request of a gold pile (code `gld` at
    /// combined index `gld`): its layout is not settled. Default: none
    /// (no pile).
    fn gold_request(&self, unit: Owner, gld: usize) -> Option<(ItemRequest, ItemSpawn)> {
        None
    }

    // ---- sockets ---------------------------

    /// Links an item into another item's inventory (it sockets the item). Default:
    /// fails.
    fn socket_link(&mut self, target: Guid, item: Guid, kind: u8) -> bool {
        false
    }
    /// A socket fill: the filler linked into the target's inventory
    /// (created first). Default: fails.
    fn link_into_item(&mut self, target: Guid, filler: Guid) -> bool {
        false
    }
    /// Socketed with fillers. Default: no.
    fn socket_filled(&self, item: Guid) -> bool {
        false
    }
    /// Socket-filler test. Default: no.
    fn socket_filler(&self, item: Guid) -> bool {
        false
    }
    /// Book / scroll spell. Not asked by [`InvDesk`]
    /// (`queries`: item data +0x3E). Default: 0.
    fn spell(&self, item: Guid) -> i32 {
        0
    }

    // ---- inventory model seams ---------------------

    /// Trade hook (page 2; multiplayer trade).
    fn trade_hook(&mut self, owner: Owner, item: Guid) {}
    /// The item is active on the unit.
    fn item_active_on(&self, item: Guid, unit: Owner) -> bool;
    /// The item's own contribution to a unit stat.
    fn own_contribution(&self, item: Guid, unit: Owner, stat: u16) -> i32;
    /// Level requirement. Not asked by [`InvDesk`]
    /// (`queries` gathers the values). Default: 0.
    fn level_requirement(&self, item: Guid, unit: Owner) -> i32 {
        0
    }
    /// Two-handed. Not asked by [`InvDesk`] (`queries`:
    /// items `2handed`). Default: no.
    fn two_handed(&self, item: Guid) -> bool {
        false
    }
    /// One-or-two-handed for the unit.
    fn one_or_two_handed(&self, unit: Owner, item: Guid) -> bool;
    /// Ammo type: an itemtypes row. Not asked by
    /// [`InvDesk`] (`queries`: the primary type's `shoots`). Default: none.
    fn ammo_type(&self, item: Guid) -> Option<i16> {
        None
    }
    /// Allowed location.
    fn has_allowed_location(&self, item: Guid) -> bool;
    /// Quiver-type item.
    fn quiver_kind(&self, item: Guid) -> bool;

    // ---- player data (the interact info is the unit
    // record's, `units::record::InteractInfo`) ---------------------------

    /// Player data +0x4C.
    fn player_data_4c(&self, player: Owner) -> u32;
    /// Player data +0x50.
    fn player_data_50(&self, player: Owner) -> u32;
    /// The player talks to the NPC (interaction list state 0).
    fn npc_talking(&self, npc: Owner, player: Owner) -> bool;
    /// The player-trade part of the NPC interaction gate (multiplayer).
    fn player_trade_gate(&self, player: Owner) -> Option<bool>;

    // ---- equipment seams ([`equip_rules`]: charms, weapon slots, item
    // skills). The skill list is the skills module's;
    // defaults: no skill, no row, nothing sent.

    /// books `scrollskill` (`scroll`) or `bookskill` of the spell index
    /// `spell`; `None`: no row.
    fn book_skill(&self, spell: i32, scroll: bool) -> Option<i32> {
        None
    }
    /// Stat unlink.
    fn stat_unlink(&mut self, owner: Owner, item: Guid) {}
    /// The inventory pass's refresh: S→C 0x48 for the unit.
    fn send_unit_refresh(&mut self, unit: Owner) {}
    /// The quantity of the unit's skill `skill`, owner −1;
    /// `None`: absent.
    fn skill_quantity(&self, unit: Owner, skill: i32) -> Option<i32> {
        None
    }
    /// Sets a skill's quantity.
    fn set_skill_quantity(&mut self, unit: Owner, skill: i32, q: i32) {}
    /// Learns a skill.
    fn learn_skill(&mut self, unit: Owner, skill: i32) {}
    /// The left / right mouse skill.
    fn mouse_skill(&self, unit: Owner, left: bool) -> Option<(i32, i32)> {
        None
    }
    /// Selects a mouse skill.
    fn select_skill(&mut self, unit: Owner, left: bool, skill: (i32, i32)) {}
    /// Whether the unit has the skill with (id, owner).
    fn has_skill_owned(&self, unit: Owner, skill: (i32, i32)) -> bool {
        false
    }
    /// Skill usability tests 1–4 (record / `InGame`
    /// 3, level 7, `aura` 6, `passive` 5); 0 when they pass. The desk runs
    /// the item type test (test 5) itself ([`Self::skill_item_types`]).
    /// Default: 0 (usable).
    fn skill_use_state(&mut self, unit: Owner, skill: (i32, i32)) -> u8 {
        0
    }
    /// The skills row's item-type sets for the item type test
    /// test. Default: none (the test is not run).
    fn skill_item_types(
        &self,
        skill: i32,
    ) -> Option<crate::skills::use_::item_type::ItemTypeSkill> {
        None
    }
    /// The skills.txt part of the throw test (`itypea1` > 0 is-a `thro`,
    /// `range` 2).
    fn throw_skill_row(&self, skill: i32) -> bool {
        false
    }
    /// Player data +0x70..+0x7C. Default: (0, −1).
    fn saved_mouse_skill(&self, unit: Owner, left: bool) -> (i32, i32) {
        (0, -1)
    }
    fn set_saved_mouse_skill(&mut self, unit: Owner, left: bool, skill: (i32, i32)) {}
    /// Step 2 of the weapon switch: the
    /// saved swap pairs (left, right) out, the current mouse pairs saved
    /// in their place (`SkillList::trade_swap_pairs`). Default: both
    /// (0, −1), nothing saved.
    fn trade_swap_skills(&mut self, unit: Owner) -> [(i32, i32); 2] {
        [(0, -1); 2]
    }
    /// The set bonuses.
    fn set_bonuses(&mut self, owner: Owner, item: Guid, state: u32) {}
    /// The player's first pet of type `t` (`(game, P, t, 0)`,
    /// `player::pets::first_pet`), for the Blood Golem share of
    /// `potion::golem_share`. Default: none.
    fn first_pet(&self, player: Owner, t: i32) -> Option<Owner> {
        None
    }
    /// `Param6` of the skill in `Skill1`
    /// of monstats row `class`; none without the row or
    /// with `Skill1` < 0. Default: none.
    fn monster_skill1_param6(&self, class: u16) -> Option<i32> {
        None
    }
}

/// The inventory world of one call: the economy's parts (game, unit
/// records and lists, stat lists, hooks, game fields, item store), the
/// inventory tables, the inventory state and the seams without a provider
/// (`R`).
pub struct InvDesk<'d, 'a, H, R: ?Sized> {
    pub econ: &'d mut Economy<'a, H>,
    pub tables: &'d InvTables,
    pub state: &'d mut InvState,
    pub rest: &'d mut R,
}

impl<'d, 'a, H: LifecycleHooks, R: InvRest + ?Sized> InvDesk<'d, 'a, H, R> {
    /// A desk for one call; the item data copies are filled from their
    /// owners.
    pub fn new(
        econ: &'d mut Economy<'a, H>,
        tables: &'d InvTables,
        state: &'d mut InvState,
        rest: &'d mut R,
    ) -> Self {
        let mut d = Self {
            econ,
            tables,
            state,
            rest,
        };
        d.apply_write_backs();
        d.sync_in();
        d
    }

    /// The owner of a unit, or the "none" owner (type 6).
    pub fn owner_or_none(&self, u: UnitId) -> Owner {
        self.owner_of(u).unwrap_or(Owner {
            ty: Owner::NONE,
            guid: u32::MAX,
        })
    }

    /// Unit lookup by type and GUID.
    pub fn unit_of(&self, o: Owner) -> Option<UnitId> {
        let ty = *UnitType::ALL.get(usize::from(o.ty))?;
        self.econ.game.lists.find_unit(ty, o.guid)
    }

    /// The item unit of a GUID (lookup type 4), when it has item data.
    pub fn item_unit(&self, guid: Guid) -> Option<UnitId> {
        self.econ
            .game
            .lists
            .find_unit(UnitType::Item, guid)
            .filter(|u| self.state.items.contains_key(u))
    }

    /// Type and GUID of a unit.
    pub fn owner_of(&self, u: UnitId) -> Option<Owner> {
        self.econ.units.get(u).map(|r| Owner {
            ty: r.ty.index() as u8,
            guid: r.guid,
        })
    }

    /// The GUID of a unit (−1 when it has no record).
    pub fn guid_of(&self, u: UnitId) -> Guid {
        self.econ.units.get(u).map_or(u32::MAX, |r| r.guid)
    }

    /// Fills the item data copies from the unit records and the item
    /// store: every item unit of the lists with item data; entries of
    /// units that are gone are dropped.
    pub fn sync_in(&mut self) {
        let live = self.econ.game.lists.units_of_type(UnitType::Item);
        for &u in &live {
            let (Some(r), Some(it)) = (self.econ.units.get(u), self.econ.items.get(u)) else {
                continue;
            };
            let d = self.state.items.entry(u).or_insert_with(|| {
                let mut d = InvItem::new(r.guid, it.record);
                // An item new to the model that lies on the ground (a
                // treasure drop, placed by the path code): its position
                // is the path's (static path x, y).
                if r.mode == u32::from(crate::items::moves::mode::GROUND) {
                    if let Some((x, y)) = self.econ.hooks.path_xy(u) {
                        (d.x, d.y) = (x, y);
                    }
                }
                d
            });
            d.guid = r.guid;
            d.record = it.record;
            d.mode = r.mode as u8;
            d.flags = it.flags;
            d.page = it.inv_page;
        }
        self.state.items.retain(|u, _| live.contains(u));
    }

    /// Writes the copied fields back to their owners (and the queued
    /// bit-stream write-backs).
    pub fn sync_out(&mut self) {
        self.apply_write_backs();
        for (&u, d) in &self.state.items {
            if let Some(r) = self.econ.units.get_mut(u) {
                r.mode = u32::from(d.mode);
            }
            if let Some(it) = self.econ.items.get_mut(u) {
                it.flags = d.flags;
                it.inv_page = d.page;
            }
        }
    }

    /// Runs `f` on the owner's inventory, lent out of the state, then
    /// writes the item copies back and runs the owner refreshes the call
    /// asked for. `None` when the owner has no inventory.
    pub(crate) fn with_inv<T>(
        &mut self,
        owner: Owner,
        f: impl FnOnce(&mut Inventory, &mut Self) -> T,
    ) -> Option<T> {
        let u = self.unit_of(owner)?;
        let mut inv = self.state.inventories.remove(&u)?;
        let out = f(&mut inv, self);
        self.state.inventories.insert(u, inv);
        self.sync_out();
        self.flush_equip();
        Some(out)
    }

    /// Runs the queued equipment-rule calls, then the owner refreshes the
    /// inventory functions asked for. A host calls it when a call that
    /// ran the rules directly (not through [`InvDesk::with_inv`], e.g. a
    /// body remove's inventory pass) returns, so no refresh outlives its
    /// call.
    pub fn flush_equip(&mut self) {
        self.run_equip_queue();
        for r in std::mem::take(&mut self.state.refresh) {
            if let Some(o) = self.owner_of(r) {
                deferred::owner_refresh(self, o);
            }
        }
    }

    /// The update-list reset after the update pass
    /// ([`deferred::update_list_reset`]). The per-unit flags of the
    /// room clean-up ([`deferred::room_cleanup`]) are the
    /// tick wiring's.
    pub fn update_done(&mut self, owner: Owner) {
        self.apply_write_backs();
        deferred::update_list_reset(self, owner);
    }

    /// The owner's inventory.
    pub fn inventory(&self, owner: Owner) -> Option<&Inventory> {
        self.state.inventories.get(&self.unit_of(owner)?)
    }

    /// The owner's inventory, or an empty one when it has none (the
    /// checks then see no item of the owner).
    pub(crate) fn inv_or_empty(&self, owner: Owner) -> std::borrow::Cow<'_, Inventory> {
        match self.inventory(owner) {
            Some(i) => std::borrow::Cow::Borrowed(i),
            None => {
                let u = self.unit_of(owner).unwrap_or(UnitId(u32::MAX));
                let kind = self.kind_of(u).unwrap_or(UnitKind::Other);
                std::borrow::Cow::Owned(Inventory::new(u, kind, owner.guid))
            }
        }
    }

    /// The inventory model's kind of a unit.
    pub fn kind_of(&self, u: UnitId) -> Option<UnitKind> {
        let r = self.econ.units.get(u)?;
        Some(match r.ty {
            UnitType::Player => UnitKind::Player {
                class: r.class as u8,
            },
            UnitType::Monster => UnitKind::Monster { class: r.class },
            UnitType::Object => UnitKind::Object { class: r.class },
            UnitType::Item => UnitKind::Item,
            UnitType::Missile | UnitType::Tile => UnitKind::Other,
        })
    }

    pub(crate) fn note_list(&mut self, r: Result<(), ListError>) {
        if let Err(e) = r {
            self.state.errors.push(InvError::List(e));
        }
    }

    /// `(u, event, u)`: the sound event in `u`'s slot with `u`
    /// itself the target (`units/sound.rs`), so only `u`'s client gets
    /// the S→C 0x2C.
    pub(crate) fn queue_own_sound(&mut self, u: Owner, event: u16) {
        if let Some(unit) = self.unit_of(u) {
            let r = crate::units::sound::queue_sound(self.econ.game, unit, event, Some(unit));
            self.note_list(r);
        }
    }
}

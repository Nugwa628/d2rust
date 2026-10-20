//! The interaction seams on their real providers:
//!
//! - [`npc_vendors`]: `world::npc` ↔ `world::vendors` in both directions
//!   ([`crate::world::npc::NpcVendors`] on the vendor functions,
//!   [`crate::world::vendors::NpcLink`] on [`NpcControl`] and the
//!   interaction lists), and the C→S 0x32 / 0x33 / 0x35 entry points that
//!   need both.
//! - [`npc_world`]: [`crate::world::npc::NpcWorld`] on the unit records,
//!   stat lists, timers, item data (through the economy wiring) and the
//!   quests ([`QuestControl`]); the rest stays a seam ([`NpcRest`]).
//! - [`quest_npc`]: C→S 0x31 with the quests' mercenary reward run on
//!   the NPC control block ([`Desk::quest_message`]).
//! - [`vendor_world`]: [`crate::world::vendors::VendorWorld`] on the same
//!   providers, item creation through
//!   [`crate::wiring::economy::Economy::create_item`]; the rest is
//!   [`VendorRest`].
//! - [`skill_use`]: the skill use pipeline ([`crate::skills::use_`]) on
//!   the action wiring's units, stats, timers and missiles
//!   ([`UseView`]); missile creation through [`crate::missiles`]; the rest
//!   is [`UseRest`].
//! - [`skill_events`]: timer events 5, 8 and 9 of the unit dispatch on
//!   the same pipeline (the action hooks hand them over through
//!   [`crate::wiring::action::Pending::skill_event`]).
//! - [`hirelings`]: [`crate::world::hirelings::HirelingWorld`] on the
//!   desk's providers ([`HireView`]); the mercenary calls of `NpcWorld`
//!   run the hireling rules; the rest is [`HirelingRest`].
//! - [`vitals`]: [`crate::combat::vitals::VitalsUnits`] on unit records
//!   and stat lists ([`VitalsView`]); experience on a kill from the parts
//!   the code implements ([`vitals::kill_experience`]).
//!
//! Ownership: [`Desk`] borrows the economy's parts ([`Economy`]: game,
//! unit records, stat lists, hooks, game fields, item tables and item
//! store), the game's [`QuestControl`], the vendor tables and the
//! [`InteractionState`] (one [`VendorRecord`] per NPC record, the
//! interaction lists of the NPCs). The NPC control block
//! ([`NpcControl`]) stays with the caller: its handlers take the desk as
//! their world.
//!
//! Status: wired, not checked against the game. Nothing here decides game
//! behaviour: rules stay in the modules; an adapter maps a seam call to a
//! provider call.

pub mod body_path;
pub mod hirelings;
pub mod npc_items;
pub mod npc_vendors;
pub mod npc_world;
pub mod quest_npc;
pub mod skill_events;
mod skill_rooms;
pub mod skill_use;
pub mod summon;
pub mod vendor_world;
pub mod vitals;

#[cfg(test)]
pub(crate) mod tests;

use std::collections::BTreeMap;

pub use hirelings::{HireView, HirelingRest};
pub use npc_items::{NpcInv, NpcInventory};
pub use npc_vendors::VendorDesk;
pub use npc_world::NpcRest;
pub use skill_use::{UseRest, UseView};
pub use vendor_world::VendorRest;
pub use vitals::{VitalsRest, VitalsView};

use super::economy::{Economy, EconomyError, QuestRest};
use crate::units::UnitId;
use crate::world::hirelings::{HirelingError, HirelingState, HirelingTables};
use crate::world::npc::{InteractionList, NpcControl, NpcError};
use crate::world::quests::{PlayerQuests, QuestControl, QuestError};
use crate::world::vendors::price::PriceFatal;
use crate::world::vendors::{GlobalLists, VendorRecord, VendorTables};

/// What went wrong in an adapter (a provider's error, a fatal assertion
/// of the game), in order.
#[derive(Debug, PartialEq, Eq)]
pub enum InteractionError {
    /// An error from the economy.
    Economy(EconomyError),
    /// An error from the NPC rules.
    Npc(NpcError),
    /// An error from the quest rules.
    Quest(QuestError),
    /// A fatal price error from the vendor rules.
    Price(PriceFatal),
    /// The NPC class has no NPC record (and so no vendor record).
    NoRecord(u16),
    /// A mercenary call without hireling tables
    /// ([`InteractionState::hireling_tables`]).
    NoHirelingTables,
    /// An error from the hireling rules.
    Hireling(HirelingError),
}

/// The interaction state of a game beside the NPC control block: the
/// vendor part of each NPC record (in the record order of
/// [`NpcControl::records`]) and the interaction list of each `interact`
/// NPC.
#[derive(Debug, Default)]
pub struct InteractionState {
    /// The vendor part of each NPC record.
    pub vendors: Vec<VendorRecord>,
    /// The interaction list of each `interact` NPC.
    pub lists: BTreeMap<UnitId, InteractionList>,
    /// Errors reported by the adapters, in order.
    pub errors: Vec<InteractionError>,
    /// The players' hireling lists.
    pub hirelings: HirelingState,
    /// The hireling tables (`pettype` row 7); `None`: the mercenary calls
    /// only report [`InteractionError::NoHirelingTables`].
    pub hireling_tables: Option<HirelingTables>,
    /// The hireling stat records queued on each unit's message list, in
    /// queue order, for the client pass's flush
    /// ([`InteractionState::unit_stat_messages`]).
    pub unit_stats: BTreeMap<UnitId, Vec<(u16, u32)>>,
    /// The hireling add messages (0x81 / 0x7A) a joining player's restore
    /// produced, held back for the join sync after the 0x5B: the broadcast
    /// of the creation reaches no client that is not in the game yet.
    /// `Some((player, held))` while a restore runs.
    pub join_capture: Option<(UnitId, Vec<Vec<u8>>)>,
    /// The store or gamble items added to an NPC's trade inventory since
    /// the host last took them (the client gets one 0x9C action 11 per
    /// item), in add order, and the player whose trade open added them.
    pub shown: Vec<UnitId>,
    /// The player whose trade open added the shown items.
    pub shown_player: Option<UnitId>,
    /// The NPC whose trade inventory the shown items joined.
    pub shown_npc: Option<UnitId>,
    /// The class of the NPC the shown items belong to.
    pub shown_class: u16,
    /// The trade open that added them was a gamble.
    pub shown_gamble: bool,
    /// Store items a purchase took out of the NPC's grid
    /// (`VendorWorld::take_from_store`), for S→C 0x9C action 12 to the
    /// trading player, in take order.
    pub taken: Vec<UnitId>,
    /// Approach requests (C→S 0x13 at distance 7–8) the host has not
    /// started yet: (player, NPC).
    pub approaches: Vec<(UnitId, UnitId)>,
    /// Queued interactions: (player, NPC GUID), run again when the
    /// player's run ends.
    pub queued: Vec<(UnitId, u32)>,
    /// D2RUST's own plumbing: when `defer_chat_end` is set
    /// the chat-close quest call (the quests'
    /// chat-end callbacks) is queued here as (player, NPC)
    /// instead of running on the plain economy, so the host can run it on
    /// its full quest world (object, room and free-spot calls).
    pub defer_chat_end: bool,
    /// Deferred chat-close calls: (player, NPC).
    pub chat_ends: Vec<(UnitId, UnitId)>,
}

impl InteractionState {
    /// The vendor records at game creation: one per NPC record, from its
    /// class, act and trader byte.
    pub fn new(ctl: &NpcControl, globals: &GlobalLists) -> Self {
        Self::new_for(&crate::rules::ORIGINAL, ctl, globals)
    }

    /// As [`Self::new`], with the store roles of `rules` (MS8e); `globals`
    /// is [`GlobalLists::build_for`] under the same rules.
    pub fn new_for(rules: &crate::rules::Ruleset, ctl: &NpcControl, globals: &GlobalLists) -> Self {
        Self {
            vendors: ctl
                .records
                .iter()
                .map(|r| VendorRecord::new_for(rules, r.class, r.act, r.trader != 0, globals))
                .collect(),
            lists: BTreeMap::new(),
            errors: Vec::new(),
            hirelings: HirelingState::default(),
            hireling_tables: None,
            unit_stats: BTreeMap::new(),
            join_capture: None,
            shown: Vec::new(),
            shown_player: None,
            shown_npc: None,
            shown_class: 0,
            shown_gamble: false,
            taken: Vec::new(),
            approaches: Vec::new(),
            queued: Vec::new(),
            defer_chat_end: false,
            chat_ends: Vec::new(),
        }
    }

    /// The flush of the queued hireling stats: the messages for one client
    /// update of `unit` (GUID `guid`). The queue stays until
    /// [`InteractionState::free_unit_stats`] (the room update step), so
    /// every client updating the unit in the pass gets them.
    pub fn unit_stat_messages(
        &self,
        unit: UnitId,
        guid: u32,
    ) -> Result<Vec<Vec<u8>>, crate::world::hirelings::HirelingError> {
        match self.unit_stats.get(&unit) {
            Some(q) => crate::world::hirelings::level::flush_stats(guid, q),
            None => Ok(Vec::new()),
        }
    }

    /// Frees the unit's queued records.
    pub fn free_unit_stats(&mut self, unit: UnitId) {
        self.unit_stats.remove(&unit);
    }

    /// The interaction list of an `interact` NPC (monster init embeds
    /// it).
    pub fn add_npc(&mut self, npc: UnitId) {
        self.lists.entry(npc).or_default();
    }

    /// Index of the vendor record of `class`.
    pub fn vendor_index(&self, class: u16) -> Option<usize> {
        self.vendors.iter().position(|r| r.class == class)
    }
}

/// The player data the quests need read-only (`QuestRest::quests` is the
/// mutable access; player data is not in d2-sim yet).
pub trait PlayerQuestsRef {
    /// The player's quest data, read-only.
    fn quests_ref(&self, player: UnitId) -> Option<&PlayerQuests>;
}

/// The NPC and vendor world of one call: the economy's parts, the game's
/// quests, the vendor tables, the interaction state and the seams without
/// a provider (`R`).
pub struct Desk<'d, 'a, H, R> {
    /// The economy's parts.
    pub econ: &'d mut Economy<'a, H>,
    /// The game's quest control block.
    pub quests: &'d mut QuestControl,
    /// The vendor tables.
    pub vendor_tables: &'d VendorTables,
    /// The interaction state.
    pub state: &'d mut InteractionState,
    /// The seams without a provider.
    pub rest: &'d mut R,
    /// Host milliseconds (`GetTickCount`), an input of store generation
    /// and refresh.
    pub now: u32,
    /// The host's inventory model for the NPC item services
    /// ([`NpcInventory`]); `None`: those answer from the rest.
    pub inv: Option<&'d mut dyn NpcInventory<H>>,
}

impl<'a, H, R: QuestRest> Desk<'_, 'a, H, R> {
    /// The quests' world on the same economy and rest.
    pub(crate) fn quest_world(
        &mut self,
    ) -> (
        &mut QuestControl,
        super::economy::EconomyQuests<'_, 'a, H, R>,
    ) {
        (
            self.quests,
            super::economy::EconomyQuests::new(self.econ, self.rest),
        )
    }
}

/// An item probe without stats, for the item functions that read only
/// record fields.
pub(crate) struct NoStats;

impl crate::items::ItemStats for NoStats {
    fn has_stats(&self) -> bool {
        false
    }
    fn stat(&self, _: u16, _: u16) -> i32 {
        0
    }
    fn base(&self, _: u16, _: u16) -> i32 {
        0
    }
    fn set_base(&mut self, _: u16, _: u16, _: i32) {}
    fn has_list(&self, _: crate::items::ListKey) -> bool {
        false
    }
    fn list_set(&mut self, _: crate::items::ListKey, _: u16, _: u16, _: i32) {}
    fn list_add(&mut self, _: crate::items::ListKey, _: u16, _: u16, _: i32) {}
    fn list_get(&self, _: crate::items::ListKey, _: u16, _: u16) -> i32 {
        0
    }
}

//! Item economy wiring: connects the item, treasure, cube and quest modules to
//! the real game state (units, stat lists, DRLG rooms, action hooks).
//!
//! - [`game_fields`]: [`crate::items::ItemGame`] on [`GameFields`], the
//!   game-creation fields that [`crate::game::Game`] does not hold yet (game
//!   seed, difficulty, expansion, ladder, unique bits); also the treasure
//!   [`crate::treasure::GameFacts`].
//! - [`item_stats`]: [`crate::items::ItemStats`] on
//!   [`crate::stats::StatLists`] ([`UnitStats`]).
//! - [`item_units`]: item units: allocation, creation and the per-item
//!   fields ([`ItemStore`], [`Economy`]); the request unit from unit and stat
//!   fields.
//! - [`treasure_items`]: [`crate::treasure::DropSink`] on item creation
//!   ([`ItemDrops`]); dropper and recipient from unit and stat fields.
//! - [`cube_items`]: [`crate::world::cube::CubeWorld`] on items, stats and
//!   units ([`EconomyCube`]); the rest stays a seam ([`CubeRest`]).
//! - [`quest_items`]: [`crate::world::quests::QuestWorld`] likewise
//!   ([`EconomyQuests`], [`QuestRest`]).
//! - [`death`]: a dead monster's drop on the action wiring's units and DRLG,
//!   through [`ItemDrops`] ([`DeathDrops`], [`monster_death_drop`]); the
//!   free-spot search is the path provider's floor drop when it is on with its
//!   field, else a seam ([`FreeSpot`]); [`DropPlacer`] receives the economy
//!   (its hooks hold the rooms).
//! - [`chest_drop`]: the chest drop of the object code the same way, the
//!   object being the dropper ([`object_chest_drop`]).
//! - [`drop_helpers`]: the object and quest drop helpers (armor, weapon,
//!   gold, by source unit; the code drop) the same way.
//! - [`quest_host`]: the quests' world on the wired host ([`HostQuests`]:
//!   [`EconomyQuests`] with the object, level, interaction and identify
//!   calls the action wiring provides).
//! - [`quest_reward`]: the quest reward on the host's inventory model
//!   ([`QuestInventory`], [`QuestInv`]).
//! - [`quest_objects`]: the object module's quest routes on the quest
//!   control (init / operate functions by index, object event 7).
//! - [`quest_tick`]: the quest updater step of the tick, on the same quest
//!   world, wrapped around a game's tick hooks ([`QuestTick`]).
//!
//! Status: wired, not yet checked against the game in play.

pub mod chest_drop;
pub mod cube_items;
pub mod death;
pub mod drop_helpers;
pub mod game_fields;
pub mod item_records;
pub mod item_stats;
pub mod item_units;
pub mod monster_equip;
pub mod quest_drop;
pub mod quest_host;
pub mod quest_items;
pub mod quest_objects;
pub mod quest_reward;
pub mod quest_tick;
pub mod treasure_items;

#[cfg(test)]
mod tests;

pub use chest_drop::{object_chest_drop, NoSpot, StartSpot};
pub use cube_items::{CubeRest, EconomyCube};
pub use death::{find_item_drop, monster_death_drop, DeathDrops, DropTables, FreeSpot};
pub use game_fields::GameFields;
pub use item_stats::{find_list, StatCtx, UnitStats};
pub use item_units::{Economy, ItemScope, ItemSpawn, ItemStore};
pub use monster_equip::{
    create_monster_equip, free_monster_inventory, item_at as monster_item_at, monster_property,
    put_on as monster_put_on, summon_equipment, tier_code,
};
pub use quest_drop::unit_quest_drop;
pub use quest_host::HostQuests;
pub use quest_items::{EconomyQuests, QuestDeferred, QuestRest};
pub use quest_objects::{LoanedInventory, NoInventory, QuestLoan, QuestObjectRun};
pub use quest_reward::{QuestInv, QuestInventory};
pub use quest_tick::{QuestTick, UnitSide};
pub use treasure_items::{dropper, recipient, DropPlacer, DropSpot, ItemDrops};

use crate::game::GameError;
use crate::items::{CreateError, Fatal};
use crate::units::modes::UnitError;
use crate::units::UnitId;

/// Errors of the economy wiring.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum EconomyError {
    /// Item creation failed.
    #[error(transparent)]
    Create(#[from] CreateError),
    /// Item creation hit a fatal condition.
    #[error(transparent)]
    Fatal(#[from] Fatal),
    /// A unit operation failed.
    #[error(transparent)]
    Unit(#[from] UnitError),
    /// A game operation failed.
    #[error(transparent)]
    Game(#[from] GameError),
    /// The allocator rejected an item unit (it never does: its check applies
    /// to players and monsters only).
    #[error("item unit not allocated")]
    NotAllocated,
    /// The allocator and item creation stepped the game seed differently
    /// (both derive the unit and item seeds).
    #[error("allocation and creation stepped the game seed differently")]
    SeedMismatch,
    /// The unit is not an item of the store.
    #[error("unit {0:?} is not an item of the store")]
    NotAnItem(UnitId),
    /// The unit has no unit record.
    #[error("unit {0:?} has no unit record")]
    NoRecord(UnitId),
}

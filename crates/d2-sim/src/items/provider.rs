//! A mod game's item overrides (`docs/plans/mods/seams.md` MS7).
//!
//! **Property functions (MS7a).** Every property record an item gets
//! (affixes, quality rows, gems and runes, uniques, sets, runewords, craft
//! lists) runs the seven slots of its `properties` row; each slot names a
//! property function by id. For a slot whose function the provider
//! [`has`](ItemProvider::has_property_fn), [`ItemProvider::property_fn`]
//! runs first, also for ids D2RUST has no function for; `None` runs
//! D2RUST's own function. A slot whose function neither side knows ends
//! the row, as an empty slot does. Slot 0's result is the `prev` value the
//! later slots of the record see.
//!
//! The provider works on the item through [`PropHost`]: the same stat
//! writer D2RUST's functions use (the stat's value shift, the target list
//! of the current mode, a foreign owner such as the player for set
//! bonuses) and the item seed for draws. It must stay deterministic: no
//! I/O, no clock, no floats in game state, draws only from the item seed.
//!
//! **Item events (MS7b).** Skills and item properties register event
//! handlers on a unit (on hit, on getting hit, on a kill, ...); each names
//! an event function by id. When a handler runs, a function the provider
//! [`has`](ItemProvider::has_event_fn) runs first through
//! [`ItemProvider::event_fn`], also for ids D2RUST has no function for;
//! `None` runs D2RUST's own (`combat::events::call`). The function gets the
//! game as a [`SkillHost`] (the same view a mod game's skill bodies get),
//! the handler's call and the event's damage record when there is one.
//!
//! **Stat callbacks (MS7e).** For a callback-enabled stat, the provider
//! runs before the server callback when `has_stat_callback` answers its
//! id. `Some(())` replaces the callback; `None` runs D2RUST's item event
//! registration, skill updates and vital rescaling. The event carries
//! the owner, propagation unit, stat/layer key and old/new total. The
//! total is already stored; hooks must avoid recursive writes to their
//! triggering stat and keep all writes deterministic.
//!
//! **Item use (MS7c).** An item's use function (`pSpell` of its items row,
//! or of its books row for tomes and scrolls) runs when a player uses the
//! item. For an id the provider [`has`](ItemProvider::has_use_fn),
//! [`ItemProvider::use_fn`] runs first, also for ids past D2RUST's 30
//! (such an item then counts as usable); `None` runs D2RUST's own body.
//! Identify keeps its two steps (the first use arms the item); the
//! provider runs on the second. The function works through [`UseHost`]
//! and answers whether the item was used: a refused use gets D2RUST's
//! failure reset; a used item is consumed by the item-move handler as a
//! used scroll is (a tome loses a charge instead).
//!
//! **Item generation (MS7d).** Creating an item picks its quality, then
//! runs that quality's routine (affixes, a unique or set row, a quality
//! row). For a quality the provider [`has`](ItemProvider::has_quality_fn),
//! [`ItemProvider::quality_fn`] runs first, also for qualities past
//! D2RUST's (a request names one, `ItemRequest::quality`); `None` runs
//! D2RUST's routine, and a quality neither side knows creates no item.
//! The routine works on the new item through [`GenHost`]: draws on the
//! item seed, D2RUST's property path for property records. After it the
//! item is finished as D2RUST's are (the ethereal roll, automagic affixes;
//! sockets only for low, normal and superior items).
//!
//! The provider comes from the game's rules, which the item tables carry
//! ([`ItemTables::rules`]): every path that creates an item or applies
//! properties reads the item tables, in the server and in the client's
//! item views alike.

use std::fmt::Debug;

use super::tables::{ItemTables, PropRec};
use super::{ItemRequest, ItemStats};
use crate::combat::events::EventCall;
use crate::combat::DamageRecord;
use crate::items::moves::{Guid, Owner};
use crate::rules::SkillHost;
use crate::stats::{CallbackEvent, StatHost, StatLists};
use crate::units::UnitId;

/// A mod game's item overrides (MS7). Every method's default leaves
/// D2RUST's behaviour.
pub trait ItemProvider: Send + Sync + Debug {
    /// The provider answers property function `func`.
    fn has_property_fn(&self, func: u8) -> bool {
        let _ = func;
        false
    }
    /// Runs property function `call.func` for one slot (only asked for a
    /// function [`ItemProvider::has_property_fn`] answers); returns the
    /// function's result (slot 0's becomes the record's `prev`), or
    /// `None` to run D2RUST's function for that id.
    fn property_fn(&self, host: &mut dyn PropHost, call: &PropCall) -> Option<i32> {
        let _ = (host, call);
        None
    }
    /// The provider answers event function `func`.
    fn has_event_fn(&self, func: i32) -> bool {
        let _ = func;
        false
    }
    /// Runs event function `func` for one handler (only asked for a
    /// function [`ItemProvider::has_event_fn`] answers): `call` holds the
    /// event, the handler's unit (H), the other unit (O), the registered
    /// key and level; `record` the event's damage record, if any. Returns
    /// 1 when it acted, 0 when not, or `None` to run D2RUST's function for
    /// that id.
    fn event_fn(
        &self,
        w: &mut dyn SkillHost,
        func: i32,
        call: &EventCall<UnitId>,
        record: Option<&mut DamageRecord>,
    ) -> Option<i32> {
        let _ = (w, func, call, record);
        None
    }
    /// Answers callbacks for stat `stat`. Its loaded row must enable
    /// `fCallback` and its unit list must have a value callback.
    fn has_stat_callback(&self, stat: u16) -> bool {
        let _ = stat;
        false
    }
    /// Runs after the changed total is stored. `Some(())` replaces the
    /// server callback; `None` falls through. Use `host` for writes that
    /// may notify other callbacks. Do not recursively write this stat.
    fn stat_callback(
        &self,
        lists: &mut StatLists,
        host: &mut dyn StatHost,
        event: &CallbackEvent,
    ) -> Option<()> {
        let _ = (lists, host, event);
        None
    }
    /// The provider answers item use function `entry`.
    fn has_use_fn(&self, entry: u32) -> bool {
        let _ = entry;
        false
    }
    /// Runs item use function `call.entry` (only asked for an entry
    /// [`ItemProvider::has_use_fn`] answers): `Some(true)` when the item
    /// was used, `Some(false)` when the use is refused, `None` to run
    /// D2RUST's body for that entry.
    fn use_fn(&self, host: &mut dyn UseHost, call: &UseCall) -> Option<bool> {
        let _ = (host, call);
        None
    }
    /// The provider answers quality `quality` (`items::q`).
    fn has_quality_fn(&self, quality: u8) -> bool {
        let _ = quality;
        false
    }
    /// Runs the routine of `quality` on a new item (only asked for a
    /// quality [`ItemProvider::has_quality_fn`] answers; the item already
    /// has that quality): `Some(true)` when the item is made,
    /// `Some(false)` when the creation fails (no item), `None` to run
    /// D2RUST's routine for that quality.
    fn quality_fn(&self, host: &mut dyn GenHost, quality: u8, rq: &ItemRequest) -> Option<bool> {
        let _ = (host, quality, rq);
        None
    }
}

/// One item use, as an item use function sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UseCall {
    /// The use function id (`pSpell`).
    pub entry: u32,
    /// The books row's `BookSkill`, −1 without one.
    pub extra: i32,
    /// The player using the item.
    pub player: Owner,
    /// The item.
    pub item: Guid,
    /// The target: the item itself when used from the grid, the player,
    /// or another item (identify).
    pub target: Owner,
    /// The point of a use at a point; 0 otherwise.
    pub x: i32,
    pub y: i32,
}

/// The game as an item use function sees it.
pub trait UseHost {
    /// The unit of a type and GUID.
    fn unit_of(&self, owner: Owner) -> Option<UnitId>;
    /// The item's row in the combined items array.
    fn item_record(&self, item: Guid) -> Option<usize>;
    /// The item's code (`code` of its items row).
    fn item_code(&self, item: Guid) -> Option<[u8; 4]>;
    /// A unit's stat total on `layer`.
    fn stat(&self, unit: UnitId, stat: u16, layer: u16) -> i32;
    /// Sets a unit's base stat on `layer`.
    fn set_stat(&mut self, unit: UnitId, stat: u16, layer: u16, value: i32);
    /// A draw in `0..n` on the unit's seed (0 for n ≤ 0 or no unit).
    fn roll(&mut self, unit: UnitId, n: i32) -> u32;
}

/// One slot of a property record, as a property function sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropCall {
    /// The slot's function id.
    pub func: u8,
    /// The property mode (`props::mode`: affix, quality row, gem, ...).
    pub mode: u8,
    /// The record: `properties` row, `param`, `min`, `max`.
    pub rec: PropRec,
    /// The slot's `set` column: the stat is set instead of added.
    pub set: bool,
    /// The slot's stat id.
    pub stat: u16,
    /// The slot's `val` column.
    pub val: u16,
    /// Slot 0's result for later slots of the record; 0 in slot 0.
    pub prev: i32,
}

/// The item a property function writes, and the writer D2RUST's own
/// functions use.
pub trait PropHost {
    /// The game's item tables.
    fn tables(&self) -> &ItemTables;
    /// The item's row in the combined items array.
    fn record(&self) -> usize;
    /// The item level properties read (`ilvl`, or the level a quality sets).
    fn item_level(&mut self) -> i32;
    /// The item's quality (`items::q`).
    fn quality(&self) -> u8;
    /// `min` if equal, else a roll in `min..=max` on the item seed
    /// (`props::roll_value`).
    fn roll(&mut self, min: i32, max: i32) -> i32;
    /// Adds (or with `set`, sets) `value` shifted by the stat's
    /// `valshift` in the mode's target list (the item's own, or the
    /// foreign owner's); returns `value`, or 0 when nothing was written
    /// (value 0, or a stat past `itemstatcost`).
    fn add_stat(&mut self, set: bool, stat: u16, layer: u16, value: i32) -> i32;
    /// The item's own stats (base values and lists).
    fn item_stats(&mut self) -> &mut dyn ItemStats;
}

/// A new item as a mod game's quality routine sees it.
pub trait GenHost {
    /// The game's item tables.
    fn tables(&self) -> &ItemTables;
    /// The item's row in the combined items array.
    fn record(&self) -> usize;
    /// The item level (`ilvl`, at least 1).
    fn item_level(&mut self) -> i32;
    /// A draw in `0..n` on the item seed (0 for n ≤ 0).
    fn roll(&mut self, n: i32) -> u32;
    /// Sets the item's file index (the row of its quality's table: a
    /// unique, a set item, a quality row; −1 for none).
    fn set_file_index(&mut self, index: i32);
    /// Applies a property record to the item's own list in `mode`
    /// (`props::mode`), as D2RUST's routines do (`props::apply_property`).
    fn apply_property(&mut self, mode: u8, rec: &PropRec);
    /// The item's own stats (base values and lists).
    fn item_stats(&mut self) -> &mut dyn ItemStats;
}

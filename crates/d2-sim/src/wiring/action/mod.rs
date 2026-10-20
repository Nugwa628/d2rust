//! The "action" seams wired to their providers: combat on the unit
//! records and stat lists ([`combat`]); missiles and monster AI on
//! combat, units (modes, timer events) and the DRLG (rooms, collision)
//! ([`missiles`], [`ai`]); DRLG room activation on the act room lists of
//! [`crate::units::UnitLists`] ([`rooms`]); waypoints on the DRLG levels
//! ([`waypoints`]); objects on units, timers and the DRLG ([`objects`]);
//! and one dispatcher the tick runs ([`dispatch`]).
//!
//! Ownership: [`ActionSim`] holds a [`crate::units::dispatch::UnitSystem`]
//! (unit records, stat lists, unit tables) whose hooks are [`ActionHooks`]
//! (DRLG, missile and AI state, tables, combat lists). Timer events go
//! through the unit dispatch of [`crate::units::dispatch`] (with the
//! monster freeze drop); its hooks run the missile class handler (missile
//! events), the AI think (monster type 2) and the type-10 reset.
//! Each seam call builds a short-lived [`View`] (or [`combat::CombatView`])
//! over the parts it needs.
//!
//! Calls with no provider yet go to [`Pending`] (defaults: nothing).
//! Nothing here decides game behaviour: every rule stays in its module;
//! an adapter only maps one seam call to the provider's call.

pub mod ai;
pub mod ai_forced;
pub mod ai_scan;
pub mod arena;
pub mod combat;
pub mod death;
pub mod dispatch;
pub mod dying;
pub mod event_records;
pub mod hirelings;
pub mod inactive;
mod inactive_items;
pub mod missiles;
pub mod mod_events;
pub mod monster_add;
pub mod monster_death;
pub mod monsters;
pub mod objects;
pub mod pending;
pub mod reaction;
pub mod rooms;
pub mod state_update;
pub mod switch;
pub mod town_portal;
pub mod unit_update;
pub mod units;
pub mod vitals_sync;
pub mod warp_tile;
pub mod waypoints;

#[cfg(any(test, feature = "bench-fixtures"))]
#[cfg_attr(not(test), allow(unused, dead_code))]
pub(crate) mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;

use d2_data::tables::{Levels, Missiles as MissileRow};
use d2_formats::animdata::AnimData;

use crate::combat::vitals::VitalsTables;
use crate::combat::{CombatEntry, CombatTables};
use crate::drlg::{DrlgData, DrlgError, Dungeon, LevelTypes, TileSource};
use crate::game::Game;
use crate::missiles::MissileStore;
use crate::monsters::ai::{AiStore, GameInfo};
use crate::rng::Seed;
use crate::skills::SkillTables;
use crate::stats::StatLists;
use crate::units::hooks::{Sim, UnitData};
use crate::units::modes::UnitError;
use crate::units::record::Units;
use crate::units::UnitId;
use crate::world::waypoints::{ArrivalList, WaypointData, WaypointRecords};

/// Whether `level` is empty of players: no player without state 7
/// (`playerbody`) has its room in `level`, counted when the call is made.
pub fn town_left_empty<X>(game: &Game, stats: &StatLists, h: &ActionHooks<X>, level: u32) -> bool {
    !game
        .lists
        .units_of_type(crate::units::UnitType::Player)
        .into_iter()
        .filter(|&u| !stats.has_state(u, 7))
        .any(|u| {
            game.lists
                .unit(u)
                .and_then(|e| e.room())
                .and_then(|r| h.drlg.level_id(game, r))
                == Some(level)
        })
}

pub use dispatch::{ActionSim, INVENTORY_REFRESH_EX};
pub use hirelings::HirelingCall;
pub use monsters::MonsterWorld;
pub use objects::{
    CainPortalAnswer, CainPortalCall, ObjectCase, ObjectReach, ObjectRoute, ObjectState,
    ObjectView, QuestObjectCall, QuestObjectHost, SoulSpawner,
};
pub use pending::{KillStep, NoPending, Pending, QuestEvent, SkillEvent};

/// The tables the action modules read (typed `d2_data` records).
#[derive(Debug, Clone)]
pub struct ActionTables {
    /// `missiles.txt` rows.
    pub missiles: Vec<MissileRow>,
    /// `skills.txt` tables.
    pub skills: SkillTables,
    /// Also the monstats / monstats2 the AI reads.
    pub combat: CombatTables,
    /// `levels.txt` rows (AI).
    pub levels: Vec<Levels>,
    /// `Sk1mode..Sk8mode` per monstats row ([`crate::monsters::ai::skill_modes`]).
    pub skill_modes: Vec<[u8; 8]>,
    /// `overlay` record count: the bound of the 0x11 overlay id
    /// (inclusive).
    pub overlay_count: i32,
    /// `monequip.bin` rows (summon equipment).
    pub monequip: Vec<d2_data::tables::Monequip>,
    /// `arena.bin` rows; row 0 is the game's arena type ([`arena`]).
    pub arena: Vec<d2_data::tables::Arena>,
}

/// The DRLG side of a game: the acts' DRLGs and their services.
pub struct DrlgWorld {
    /// The acts' dungeons.
    pub dungeon: Dungeon,
    /// The DRLG data tables.
    pub data: Arc<DrlgData>,
    /// The tile source.
    pub tiles: Box<dyn TileSource>,
    /// The level types.
    pub types: Box<dyn LevelTypes>,
}

/// What went wrong in an adapter (fatal assertions of the game, API misuse,
/// a provider's error), in order.
#[derive(Debug, PartialEq, Eq)]
pub enum WiringError {
    /// A store was needed while it was lent out (a missile or AI call
    /// re-entered its own module).
    Reentrant(&'static str),
    /// A pet list fatal assert.
    Pet(crate::player::pets::PetError),
    Drlg(DrlgError),
    Unit(UnitError),
    /// The AnimData name lookup failed (a name longer than 8 characters,
    /// fatal 0xD9 / 0xDA in the game).
    AnimData(d2_formats::FormatError),
    /// A path-core fatal assert.
    Path(crate::path::PathError),
    /// A walk fatal assert.
    Walk(crate::path::walk::WalkError),
    /// A placement fatal assert.
    Place(crate::path::place_seams::PlaceError),
    /// An object fatal assert.
    Object(crate::world::objects::ObjectError),
    /// Room ready on a client without a room: fatal assert 0x3EF in the
    /// game.
    NoClientRoom(crate::units::ClientId),
    /// The monster mode message: a fatal assert or a message with no known
    /// layout.
    ModeMessage(unit_update::ModeMessageError),
    /// The lightning fan / ring with a progressive step ≤ 0
    /// (a progressive step ≤ 0 would never end);
    /// D2RUST stops it (as `StatListError::EndlessExpiry`).
    EndlessProgressive {
        unit: UnitId,
        skill: i32,
        step: i32,
    },
    /// An act change fatal assert of the game, by its line code (0x19F the
    /// destination is in the client's act, 0x1AF no player, 0x1D1 / 0x1D6 a
    /// failed teleport).
    ActChange(u32),
    /// A portal pair creation fatal assert of the game, by its line code
    /// (0xE42 null room, 0xE5C a destination in another act, 0xE2B a source
    /// level above 255).
    Portal(u32),
}

/// What the Hireable AI reads of the hireling lists and table: each living
/// hireling unit's owner and its node's `hireling` `Id`, and the rows. The host that holds the
/// lists writes it.
#[derive(Debug, Clone, Default)]
pub struct HirelingAiFacts {
    /// Each living hireling unit to its owner and `hireling` `Id`.
    pub ids: BTreeMap<UnitId, (UnitId, u32)>,
    /// The `hireling` rows.
    pub rows: Option<Arc<crate::world::hirelings::HirelingRows>>,
}

/// First byte of the host-only mark the client pass sends for a queued
/// item when [`ActionHooks::item_marks`] is on: `[MARK, guid LE u32]`. Not
/// a game message (no S→C id uses 0xFF); the host replaces it with the
/// item's announcement.
pub const GROUND_ITEM_MARK: u8 = 0xFF;

/// Host-only marks of a player's unit update in the client pass's queue
/// walk when [`ActionHooks::item_marks`] is on: `[MARK, guid LE u32]` of
/// the updated player. [`PLAYER_ITEMS_MARK`]: the item dispatcher (0x47 /
/// 0x48) after the 0x15; [`PLAYER_SOUND_MARK`]: the unit flag 0x400 sound
/// after the mode messages. The host sends them there.
pub const PLAYER_ITEMS_MARK: u8 = 0xFE;
/// See [`PLAYER_ITEMS_MARK`].
pub const PLAYER_SOUND_MARK: u8 = 0xFD;
/// Host-only mark of the client pass's queue walk: the trade open's store
/// items (one 0x9C each, sent by their item updates) sit in the room's
/// update queue at the place they were queued, so the host sends them at
/// this mark ([`ActionHooks::store_anchor`]).
pub const STORE_ITEMS_MARK: u8 = 0xFC;
/// Host-only mark `[MARK, hireling GUID LE u32]` of a monster's unit update
/// (the hireling's item update list) when [`ActionHooks::item_marks`] is
/// on; the host sends the item messages there if the hireling is the
/// receiver's.
pub const MERC_ITEMS_MARK: u8 = 0xFA;
/// Host-only mark `[MARK, item GUID LE u32]` of a room join's add messages
/// for an item of the joined room, when [`ActionHooks::item_marks`] is on:
/// the host sends the ground update (0x9C action 2 / 3) if the item is
/// already announced.
pub const JOIN_ITEM_MARK: u8 = 0xF9;
/// Host-only mark `[MARK, item GUID LE u32]` of an armor piece whose durability
/// ran out in a hit: the host breaks the item on the inventory model at this place of the tick.
pub const ITEM_BREAK_MARK: u8 = 0xF8;

/// Host-only mark a queued quest event leaves in the sent stream, addressed
/// to the unit whose AI raised it (never a client): the event runs after
/// the tick, and its messages are spliced in at the mark, where the game
/// sent them from inside the AI (before the Summoner's 0x4C).
/// `[MARK, 0, 0, 0, 0]`.
pub const QUEST_EVENT_MARK: u8 = 0xFB;

pub use crate::units::lifecycle::ActTail;

/// The [`crate::units::hooks::UnitHooks`] of [`ActionSim`]'s unit system
/// and the state every action adapter shares.
pub struct ActionHooks<X> {
    /// The tables the action modules read.
    pub tables: Arc<ActionTables>,
    /// The DRLG side of the game.
    pub drlg: DrlgWorld,
    /// Lent to the missile code during a missile call (`None` then).
    pub missiles: Option<MissileStore>,
    /// Lent to the AI code during a think (`None` then).
    pub ai: Option<AiStore>,
    /// D2RUST's own: dev tools. The dev switches;
    /// all off unless the interactive client's dev handlers set them.
    pub dev: crate::dev::DevSwitches,
    /// The rules this game runs under,
    /// set when the game is built, never changed.
    pub rules: crate::rules::Ruleset,
    /// The second word of the coordinate records monsters hold as their
    /// vision record, by record identity; absent = 0.
    pub vision_seen: BTreeMap<crate::monsters::init::VisionRecord, u32>,
    /// Game fields the AI reads.
    pub ai_info: GameInfo,
    /// Combat lists, first = newest.
    pub combat_lists: BTreeMap<UnitId, Vec<CombatEntry>>,
    /// The pending event records of the units.
    pub event_records: event_records::EventRecords,
    /// The process-wide element hit-class byte.
    pub hit_class: u8,
    /// The game seed (unit allocation).
    pub game_seed: Seed,
    /// The unique bits: the game's one
    /// store, read and set by every item creation (the chest drop of
    /// [`Self::object_drops`], a monster drop, the economy of the host's
    /// handlers and the lent quest parts). Zero in a new game.
    pub uniques: crate::items::UniqueBits,
    /// The game's one item store (the item data of every item unit:
    /// drops, stores, inventories, the cube; `crate::wiring::economy`).
    /// Lent to an economy for a call (empty then).
    pub items: crate::wiring::economy::ItemStore,
    /// Waypoint records per player.
    pub waypoints: BTreeMap<UnitId, WaypointRecords>,
    /// ByTime's base time t of each stat-list owner's act
    /// (`StatHost::act_time`): seeded at the game entry and the act change
    /// for players (the act's time, 0 for a fresh act), refreshed for
    /// every player and monster (a hireling's by-time items) standing in
    /// the act at each environment advance
    /// (`ActionSim::advance_environment`). The game reads t live from the
    /// owner's act for any owner type; a monster created between two
    /// advances has no t until the next one (the advance runs every tick,
    /// first).
    pub act_times: BTreeMap<UnitId, i32>,
    /// The object control's arrival list: written
    /// by travel (the host's C→S 0x49), read by init function 17.
    pub arrivals: ArrivalList,
    /// The waypoint tables init function 17 runs on inside the object's
    /// creation;
    /// `None`: the init route goes to [`Pending::object_route`].
    pub waypoint_init: Option<Arc<WaypointData>>,
    /// The object control, the object
    /// tables and the host tick ([`objects`]). `None` (the default): not
    /// created ([`ActionSim::create_objects`]); the object routes keep
    /// their [`Pending`] answers. Lent to an object call (`None` then).
    pub objects: Option<ObjectState>,
    /// The object state is lent out for a call.
    objects_out: bool,
    /// The portal pairs' links and the players' portal GUIDs
    /// ([`town_portal`]).
    pub portals: town_portal::PortalLinks,
    /// The rooms' delete lists: {type, GUID} records, newest first; turned
    /// into S→C 0x0A by the per-client update ([`View::send_room_deletes`])
    /// and freed by the tick's room clean-up. Only the portal removals
    /// write here so far.
    pub room_deletes: BTreeMap<crate::units::RoomId, Vec<(u8, u32)>>,
    /// The players whose step event returned 2 (stopped) since the host
    /// last took the list (the queued object operate fires only on that
    /// stop). Filled by `wiring::path::walk::player_step`; the host takes
    /// it at the end of the tick's player-update step.
    pub player_stops: Vec<crate::units::UnitId>,
    /// The 0x13 tile cases that started a run to the tile (player, tile
    /// GUID; [`View::warp_tile_message`]): the host queues the case again
    /// for the player's stop (the pending 0x13 `{5, GUID}`) and takes the
    /// list after each call.
    pub tile_walks: Vec<(crate::units::UnitId, u32)>,
    /// The drop state of the object code's chest drop
    /// ([`crate::wiring::economy::object_chest_drop`];
    /// the `levels` rows are the object tables'). `None` (the default):
    /// no drop (`ChestWorld::chest_drop` answers none, as before the
    /// provider).
    pub object_drops: Option<Box<crate::wiring::economy::DeathDrops>>,
    /// The arena state (kill event and 0x65, [`arena`]); `None`: the host
    /// does not model it.
    pub arena: Option<arena::ArenaState>,
    /// Whether the client pass marks each queued ground item at its place
    /// in the walk ([`GROUND_ITEM_MARK`]) for the host to announce there
    /// (inside the unit update). Off by default: no mark is sent.
    pub item_marks: bool,
    /// The NPC's room and the unit that headed its update queue when the trade open queued the
    /// store items (the queue prepends): the walk sends [`STORE_ITEMS_MARK`] just before that
    /// unit, or at the end of that room's queue when it was empty, then clears it. `None`: no
    /// store items pending.
    pub store_anchor: Option<(crate::units::RoomId, Option<UnitId>)>,
    /// Players whose pets follow them after a placement, for the host that
    /// holds the pet lists. `None` (the default): the call does nothing (no pet list in the action
    /// wiring).
    pub pet_follows: Option<Vec<UnitId>>,
    /// The host's pending-message count when the first follow of
    /// [`Self::pet_follows`] was queued (`Pending::sent_len`): the game runs
    /// the follow inside the placement, so its messages sit at that point
    /// of the outbox. `None`: not known.
    pub pet_follow_mark: Option<usize>,
    /// The hireling facts the Hireable AI reads, published by the host that holds the hireling
    /// lists ([`HirelingAiFacts`]); empty: the AI finds no hireling.
    pub hireling_ai: HirelingAiFacts,
    /// Monsters killed by the kill with flag 1 (every caller but the
    /// expired-pet kill), for the host that holds the hireling lists (the
    /// owner is a player). `None` (the default): nothing is recorded.
    pub pet_deaths: Option<Vec<UnitId>>,
    /// The host runs the item update pass after the tick and wants a
    /// player's own state / stat sends after it (the item messages precede
    /// the player's own state and stat messages). On,
    /// `View::player_tail` holds them until [`ActionSim::flush_player_tail`].
    pub defer_player_tail: bool,
    /// Set around a player's state / stat sends while [`Self::defer_player_tail`].
    pub capture_tail: bool,
    /// The held sends: (receiving player, bytes).
    pub player_tail: Vec<(UnitId, Vec<u8>)>,
    /// Players whose state-changed bits wait for [`ActionSim::flush_player_tail`]:
    /// with [`Self::defer_player_tail`] their update runs after the tick's
    /// room clean-up, which therefore leaves their bits (the update step runs
    /// before the clean-up).
    pub player_state_done: Vec<UnitId>,
    /// Players whose mode-17 start ran (after the corpse creation), for
    /// the host that holds the hireling lists: the hireling dies with its
    /// owner, in every game type. `None` (the default): nothing is
    /// recorded.
    pub owner_deaths: Option<Vec<UnitId>>,
    /// The hireling calls met in order (save restore, join follow, act
    /// change; [`HirelingCall`]), for the host that holds the hireling
    /// lists. `None` (the default): nothing is recorded.
    pub hireling_calls: Option<Vec<HirelingCall>>,
    /// The quest/NPC act changes asked (`LifecycleHooks::request_act_change`:
    /// player, destination level, argument), for the host that runs them
    /// (`wiring::path::act_change`) after the call.
    pub act_changes: Vec<(UnitId, u32, u32)>,
    /// The calls of an NPC act travel that follow its act change (act
    /// change, then the act completion, then the waypoint), held until the host has run the
    /// change so their messages follow the change's.
    pub act_tails: Vec<ActTail>,
    /// The vendor part of the town-leave refresh (player, level left, the
    /// town's "empty" counted at the call by [`town_left_empty`]), met by a
    /// level change or a level warp, for the host that holds the vendor
    /// records, which runs the vendor refresh after the call.
    pub town_leaves: Vec<(UnitId, u32, bool)>,
    /// The removal records of ground items that leave their room (GUID,
    /// room; a room delete record): lifted (picked up to the cursor,
    /// inventory or belt) or freed (a gold pile). Sent as S→C 0x0A by the
    /// per-client update's removals (the client room's adjacent rooms only)
    /// and freed by the tick's room clean-up. The
    /// other unit types' records are [`Self::room_deletes`] (portals)
    /// and [`Self::removal_notices`] (hireling frees).
    pub removed_items: Vec<(u32, crate::units::RoomId)>,
    /// The loaded `AnimData.d2` (parsed by `d2-formats`): the records `UnitHooks::anim_record`
    /// looks up by COF name. `None`: no record for any unit (as before the table is given).
    pub anim_data: Option<Arc<AnimData>>,
    /// The monsters' skill sequences (monstats slot sequences and
    /// `monseq`), loaded for a monster
    /// in mode 14. `None`: a monster's mode 14 plays its AnimData record.
    pub monster_sequences: Option<Arc<crate::skills::sequences::MonsterSequences>>,
    /// `experience.txt` / `charstats.txt` of the experience on a kill.
    /// `None`: no experience is given.
    pub vitals: Option<Arc<VitalsTables>>,
    /// The target of the monster mode change running now (the record
    /// argument of the mode change); set by the kill's death mode change.
    /// Also the killer of a player's DT start (its unit target, [`death`]):
    /// the host that starts it sets it.
    pub mode_target: Option<UnitId>,
    /// The unit whose death clean-up ran in the DT start running now
    /// ([`monster_death::death_cleanup`]).
    pub death_cleaned: Option<UnitId>,
    /// The mode of the monster mode change running now (the record's
    /// mode), for the start functions that read it (the attack / skill
    /// start).
    pub monster_request: u32,
    /// The monster state (monster data, umods, monster init) lent by the
    /// host that owns it ([`monsters`]: `WorldSim` lends its world state
    /// around its timer events and tick hooks). `None`: the monster
    /// routes keep their [`Pending`] answers.
    pub monster_world: Option<Box<dyn MonsterWorld<X>>>,
    /// The monster world is taken out for a call.
    monster_world_out: bool,
    /// Mode-3 umod dispatches asked while the monster world
    /// was out (a umod callback's own damage): run when the outer dispatch
    /// ends ([`ActionHooks::run_umods`]).
    pub(super) deferred_umod_hits: Vec<UnitId>,
    /// (owner, missile) of mode-5 dispatches asked while the monster world
    /// is out (a missile made by a umod callback); the world host that
    /// made the missile runs them right after the creation.
    pub(in crate::wiring) deferred_missile_umods: Vec<(UnitId, UnitId)>,
    /// The quest control lent by the host that holds it
    /// ([`objects::QuestObjectHost`]): a quest init, operate or object
    /// event 7 the object module hands back runs on it at once, inside the
    /// allocation, dispatch or event.
    /// `None`: queued for the host ([`ObjectState::route_quests`]) or
    /// handed to [`Pending::object_route`], as before.
    pub quest_host: Option<Box<dyn objects::QuestObjectHost<X>>>,
    /// The quest host is running a route: routes it raises are queued and
    /// run right after it.
    quest_host_out: bool,
    /// The preset paths the quest map-AI stores keep; a handle is the
    /// index + 1.
    pub map_ai_paths: Vec<Vec<crate::monsters::ai::MapNode>>,
    /// Objects allocated by [`View::allocate`] whose per-kind init waits
    /// for the allocation's game-seed step to be written back (`None`
    /// outside such an allocation).
    deferred_inits: Option<Vec<UnitId>>,
    /// Units part-way through allocation (after the add to the unit lists,
    /// before the finish), with the allocation's room argument, innermost
    /// last.
    alloc_rooms: Vec<(UnitId, Option<crate::units::RoomId>)>,
    /// The unit path records and tables ([`crate::wiring::path`]).
    /// `None` (the default): the path seams keep their [`Pending`]
    /// answers; [`ActionHooks::enable_paths`] turns the provider on.
    pub paths: Option<Box<crate::wiring::path::PathState>>,
    /// The table data of the skill bodies
    /// ([`crate::skills::use_::bodies::BodyTables`]). `None`: no
    /// itemstatcost flags, state groups or overlays (every lookup answers
    /// "no record").
    pub bodies: Option<Arc<crate::skills::use_::bodies::BodyTables>>,
    /// Unit event handler lists, first = head.
    pub handlers: BTreeMap<UnitId, Vec<crate::skills::use_::bodies::Handler>>,
    /// The unit event iteration on [`Self::handlers`]
    /// ([`combat::UnitEventFn`]). `None` (the default): every unit event
    /// goes to [`Pending::unit_event`];
    /// `ActionHooks::enable_unit_events` (a host with
    /// [`crate::wiring::interaction::UseRest`]) turns the registry on.
    pub unit_events: Option<combat::UnitEventFn<X>>,
    /// The curse helper behind [`crate::combat::CombatWorld::curse`]
    /// ([`combat::ApplyStateFn`]). `None` (the default): the curse goes to
    /// [`Pending::curse`]; `ActionHooks::enable_unit_events` installs
    /// `apply_state`.
    pub apply_state: Option<combat::ApplyStateFn<X>>,
    /// The client vitals sync's caches ([`vitals_sync`]).
    /// `None` (the default): the sync is off;
    /// [`ActionHooks::enable_vitals_sync`] turns it on.
    pub sync: Option<vitals_sync::SyncState>,
    /// The death state of the players' clients ([`death`]).
    pub death: death::DeathState,
    /// The session state of the clients and players ([`switch`]): player names, hot keys,
    /// skill hands, portal flags.
    pub session: switch::SessionState,
    /// The players' server skill lists ([`crate::skills::list`]): created
    /// by the player init and filled by the character load; the join's S→C
    /// 0x94 reads them.
    /// TODO: the combat and skill seams
    /// ([`Pending::skill_list`]) do not read them yet.
    pub skill_lists: BTreeMap<UnitId, crate::skills::list::SkillList>,
    /// The players' pet lists, created on a player's first summon
    /// ([`crate::wiring::interaction::summon`]).
    pub pet_lists: BTreeMap<UnitId, crate::player::pets::PetLists>,
    /// Mercenaries linked to their player (`LifecycleHooks::set_ai_owner`
    /// with a player owner, called only by the hireling init): the units whose
    /// pet type is 7, for the 0xAC owner GUID.
    pub hireling_units: std::collections::BTreeSet<UnitId>,
    /// The removal notices (type, GUID) of units freed outside the tick's
    /// steps (`LifecycleHooks::queue_removal_notice`), sent by the next
    /// per-client update.
    pub removal_notices: Vec<(u8, u32)>,
    /// The skill entries a summon's `set_skill` (the entry of the skill
    /// with owner −1, added when missing, base level:= v) gives a monster:
    /// skill id → base level. The AI reads them (`Skill1` entry and level);
    /// a monster without one asks
    /// [`Pending::ai_skill_entry`].
    pub monster_skills: BTreeMap<UnitId, SkillList>,
    /// The AI control's minion spawn class of a unit, noted while
    /// the AI store is lent to the think, for the skill start that reads it.
    pub spawn_class_notes: BTreeMap<UnitId, i32>,
    /// The entries monster init gives a monster (`Skill<i>` at `Sk<i>lvl` +
    /// the monster skill bonus): skill id → base level. Read for the level
    /// ([`Pending::ai_skill_level`] is asked for a unit without one) and
    /// by the entry lookup [`ActionHooks::monster_entry_of`].
    pub natural_skills: BTreeMap<UnitId, SkillList>,
    /// A unit's source unit (set by `link_source`); read when the unit's
    /// second flag word has bit 0x400.
    pub unit_source: BTreeMap<UnitId, UnitId>,
    /// A monster's equipped items by body location (its inventory's
    /// body slots): D2RUST's own record of the
    /// holdings `has_item_at` reads (no monster inventory model here).
    pub monster_equip: BTreeMap<UnitId, BTreeMap<u8, UnitId>>,
    /// The inactive-unit store ([`inactive`]). `None` (the default): the
    /// tick's compress does nothing and the restore is the host's, as before.
    pub inactive: Option<crate::units::inactive::InactiveStore>,
    /// The warp tiles' records while [`Self::inactive`] is off: tick step
    /// 9 stores and frees each tile of a deactivated room here, and the
    /// room's restore re-creates them after the host's restore (in the
    /// store's order). A D2RUST-only path:
    /// the tile part of the store without the rest of it; the game's
    /// rule is the store (play turns it on).
    pub fallback_tiles: crate::units::inactive::InactiveStore,
    /// Seams with no provider yet.
    pub x: X,
    /// Scratch seed handed out for a unit without a record (an error is
    /// logged with it).
    pub orphan_seed: Seed,
    /// Lists freed with a remove callback by the stat lists (unit, state,
    /// callback id, list), taken by `UnitHooks::lists_expired` after the
    /// expiry walk (`units.rs`).
    pub removed_lists: Vec<(UnitId, u32, u32, crate::stats::lists::ListId)>,
    /// Skill stat callbacks (stats 83, 126, 127, 188: unit, stat, layer)
    /// the stat lists raised; their "refresh all" runs at the unit's next
    /// update ([`Pending::skill_stat_refresh`]).
    pub skill_refresh: Vec<(UnitId, u16, u16)>,
    /// The players' stat 97 / 107 changes (unit, stat, layer, new total)
    /// in write order, from `StatHost::skill_stat_changed`: the
    /// item-move rest syncs the item skills for each (d2-server
    /// `preview_skills::sync_oskills`).
    pub oskill_changes: Vec<(UnitId, u16, u16, i32)>,
    /// Adapter errors, in order.
    pub errors: Vec<WiringError>,
}

/// A monster's skill entries, `(skill id, level)`, in the order the game's
/// unit list holds them: a new entry is appended at the tail, and an entry
/// that is found only has its level updated, so inserting a present skill
/// keeps its place.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillList(Vec<(i32, i32)>);

impl SkillList {
    /// The level of `skill`.
    pub fn get(&self, skill: &i32) -> Option<&i32> {
        self.0.iter().find(|e| e.0 == *skill).map(|e| &e.1)
    }
    /// Whether `skill` has an entry.
    pub fn contains_key(&self, skill: &i32) -> bool {
        self.get(skill).is_some()
    }
    /// Sets the level of `skill`: in place when present, else at the tail.
    /// Returns the previous level.
    pub fn insert(&mut self, skill: i32, level: i32) -> Option<i32> {
        match self.0.iter_mut().find(|e| e.0 == skill) {
            Some(e) => Some(std::mem::replace(&mut e.1, level)),
            None => {
                self.0.push((skill, level));
                None
            }
        }
    }
    /// The entries in list order.
    pub fn iter(&self) -> impl Iterator<Item = (&i32, &i32)> {
        self.0.iter().map(|e| (&e.0, &e.1))
    }
    /// The skill ids in list order.
    pub fn keys(&self) -> impl Iterator<Item = &i32> {
        self.0.iter().map(|e| &e.0)
    }
    /// The entries as an id → level map (the order is lost).
    pub fn to_map(&self) -> BTreeMap<i32, i32> {
        self.0.iter().copied().collect()
    }
}

impl<X: Pending> ActionHooks<X> {
    /// A monster's minion owner. With the AI store, the AI control's owner
    /// (when set, its GUID) looked up as a player by that GUID (the owner
    /// type is not read, so a pack leader's GUID names a player or
    /// nothing); without it,
    /// [`Pending::minion_owner`].
    pub(crate) fn minion_owner_of(&self, game: &Game, unit: UnitId) -> Option<UnitId> {
        match self.ai.as_ref() {
            Some(ai) => {
                let o = ai.control(unit)?.minion_owner?;
                game.lists.find_unit(crate::units::UnitType::Player, o.guid)
            }
            None => self.x.minion_owner(unit),
        }
    }

    /// Adds a skill entry to a monster: the entry goes next to the init entries
    /// ([`ActionHooks::natural_skills`]) in the unit's list ([`ActionHooks::monster_skills`],
    /// which then shadows them), then [`Pending::monster_passive_refresh`]. The list keeps
    /// insertion order ([`SkillList`]: a new entry is appended at the tail, an entry that is
    /// found only has its level updated): the init entries, then Attack, then the class skills
    /// in record order.
    pub(crate) fn assign_monster_skill(
        &mut self,
        sim: &mut Sim<'_>,
        unit: UnitId,
        skill: i32,
        level: i32,
    ) {
        let natural = self.natural_skills.get(&unit).cloned().unwrap_or_default();
        self.monster_skills
            .entry(unit)
            .or_insert(natural)
            .insert(skill, level);
        X::monster_passive_refresh(self, sim, unit, skill, level);
    }

    /// Queues the pet follow of `player` (no-op without the
    /// queue), noting where in the host's pending messages it happened.
    pub fn queue_pet_follow(&mut self, player: UnitId) {
        if let Some(q) = self.pet_follows.as_mut() {
            if q.is_empty() {
                self.pet_follow_mark = self.x.sent_len();
            }
            q.push(player);
        }
    }

    /// The unit's skill list: its list in [`ActionHooks::skill_lists`]
    /// when it has one, else
    /// [`Pending::skill_list`]; a monster the host lists nothing for
    /// gets its own entries (owner −1): its init entries
    /// ([`ActionHooks::natural_skills`]) with each summon entry
    /// ([`ActionHooks::monster_skills`]) setting the base level of its
    /// skill, or adding it (the entries `skill()` and `blvl` read).
    pub fn skill_list_of(&self, unit: UnitId) -> Vec<crate::skills::SkillEntry> {
        if let Some(l) = self.skill_lists.get(&unit) {
            return l.view();
        }
        let list = self.x.skill_list(unit);
        if !list.is_empty() {
            return list;
        }
        // The list order (init entries, then the assigned ones), each
        // skill once.
        let mut skills: Vec<i32> = Vec::new();
        for s in [&self.natural_skills, &self.monster_skills]
            .into_iter()
            .filter_map(|m| m.get(&unit))
            .flat_map(|m| m.keys().copied())
        {
            if !skills.contains(&s) {
                skills.push(s);
            }
        }
        skills
            .into_iter()
            .filter_map(|skill| self.monster_entry_of(unit, skill))
            .collect()
    }

    /// The unit's current skill entry: its list's current entry when it has a list, else
    /// [`Pending::used_skill`] with the base level of the monster's entry
    /// for that skill: its summon entry ([`ActionHooks::monster_skills`]),
    /// else its init entry ([`ActionHooks::natural_skills`]: `Sk<i>lvl`
    /// plus the bonus), as the AI's `skill_level`
    /// reads them.
    pub fn used_skill_of(&self, unit: UnitId) -> Option<crate::skills::SkillEntry> {
        match self.skill_lists.get(&unit).filter(|l| l.current.is_some()) {
            Some(l) => l.current.and_then(|i| l.view().get(i).copied()),
            // A monster's partial list (an aura start's, `summon.rs`) holds
            // no AI-requested skill: that one lives in the seam.
            None => {
                let mut e = self.x.used_skill(unit)?;
                let levels = self
                    .monster_skills
                    .get(&unit)
                    .or_else(|| self.natural_skills.get(&unit));
                if let Some(&base) = levels.and_then(|m| m.get(&e.skill)) {
                    e.base = base;
                }
                Some(e)
            }
        }
    }

    /// For a unit without a skill list: the
    /// monster's entry of `skill` (owner −1) from its summon entries
    /// ([`ActionHooks::monster_skills`]), else its init entries
    /// ([`ActionHooks::natural_skills`]); none
    /// when it has no such entry.
    pub fn monster_entry_of(&self, unit: UnitId, skill: i32) -> Option<crate::skills::SkillEntry> {
        let base = self
            .monster_skills
            .get(&unit)
            .and_then(|m| m.get(&skill))
            .or_else(|| self.natural_skills.get(&unit).and_then(|m| m.get(&skill)))?;
        Some(crate::skills::SkillEntry {
            skill,
            base: *base,
            level_bonus: 0,
            owner_guid: -1,
            charges: 0,
            has_charges: false,
        })
    }

    /// The E-flags word of entry `e`: the unit's skill list owns it when it
    /// has one (a skill start writes there), else the host seam.
    pub fn entry_flags_of(&self, unit: UnitId, e: &crate::skills::SkillEntry) -> u32 {
        match self
            .skill_lists
            .get(&unit)
            .and_then(|l| l.find(e.skill, e.owner_guid).and_then(|i| l.entries.get(i)))
        {
            Some(le) => le.flags,
            None => self.x.entry_flags(unit, e),
        }
    }
}

impl<X> ActionHooks<X> {
    /// Creates the hooks over the tables, the DRLG and the host `x`.
    pub fn new(tables: Arc<ActionTables>, drlg: DrlgWorld, game_seed: Seed, x: X) -> Self {
        Self {
            tables,
            drlg,
            missiles: Some(MissileStore::new()),
            ai: Some(AiStore::new()),
            dev: crate::dev::DevSwitches::default(),
            rules: crate::rules::Ruleset::Original,
            ai_info: GameInfo::default(),
            combat_lists: BTreeMap::new(),
            event_records: Default::default(),
            hit_class: 0,
            game_seed,
            uniques: crate::items::UniqueBits::default(),
            items: crate::wiring::economy::ItemStore::new(),
            waypoints: BTreeMap::new(),
            act_times: BTreeMap::new(),
            arrivals: ArrivalList::default(),
            waypoint_init: None,
            objects: None,
            objects_out: false,
            portals: Default::default(),
            room_deletes: BTreeMap::new(),
            player_stops: Vec::new(),
            tile_walks: Vec::new(),
            object_drops: None,
            item_marks: false,
            store_anchor: None,
            arena: None,
            pet_follows: None,
            pet_follow_mark: None,
            hireling_ai: HirelingAiFacts::default(),
            pet_deaths: None,
            defer_player_tail: false,
            capture_tail: false,
            player_tail: Vec::new(),
            player_state_done: Vec::new(),
            owner_deaths: None,
            hireling_calls: None,
            act_changes: Vec::new(),
            act_tails: Vec::new(),
            town_leaves: Vec::new(),
            removed_items: Vec::new(),
            anim_data: None,
            monster_sequences: None,
            vitals: None,
            mode_target: None,
            death_cleaned: None,
            monster_request: 0,
            monster_world: None,
            monster_world_out: false,
            deferred_umod_hits: Vec::new(),
            deferred_missile_umods: Vec::new(),
            vision_seen: BTreeMap::new(),
            quest_host: None,
            quest_host_out: false,
            map_ai_paths: Vec::new(),
            deferred_inits: None,
            alloc_rooms: Vec::new(),
            paths: None,
            bodies: None,
            handlers: BTreeMap::new(),
            unit_events: None,
            apply_state: None,
            sync: None,
            death: death::DeathState::default(),
            session: switch::SessionState::default(),
            skill_lists: BTreeMap::new(),
            pet_lists: BTreeMap::new(),
            hireling_units: std::collections::BTreeSet::new(),
            removal_notices: Vec::new(),
            monster_skills: BTreeMap::new(),
            spawn_class_notes: BTreeMap::new(),
            natural_skills: BTreeMap::new(),
            unit_source: BTreeMap::new(),
            monster_equip: BTreeMap::new(),
            inactive: None,
            fallback_tiles: crate::units::inactive::InactiveStore::default(),
            x,
            orphan_seed: Seed::init(),
            removed_lists: Vec::new(),
            skill_refresh: Vec::new(),
            oskill_changes: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// Turns the path provider on ([`crate::wiring::path`]): from now on
    /// the path seams are answered by `d2_sim::path` instead of
    /// [`Pending`]. Call before any unit is allocated (units allocated
    /// earlier have no path record).
    pub fn enable_paths(&mut self) -> Result<(), crate::path::PathError> {
        self.paths = Some(Box::new(crate::wiring::path::PathState::new()?));
        Ok(())
    }

    /// Sets the host's `GetTickCount` the object calls read (an input of
    /// `d2-sim`, never read by it). No object state: nothing.
    pub fn set_host_tick(&mut self, ms: u32) {
        if let Some(st) = self.objects.as_mut() {
            st.host_tick = ms;
        }
    }

    /// The missile store (outside a missile call).
    pub fn missile_store(&self) -> &MissileStore {
        self.missiles.as_ref().expect("not lent out")
    }

    /// The AI store (outside a think).
    pub fn ai_store(&mut self) -> &mut AiStore {
        self.ai.as_mut().expect("not lent out")
    }
}

/// The unit side of a seam call: unit records, stat lists, unit tables
/// and the shared state. Missile and AI seams are implemented on it
/// ([`missiles`], [`ai`]); the game comes with each call.
pub struct View<'a, X> {
    /// The unit records.
    pub units: &'a mut Units,
    /// The stat lists.
    pub stats: &'a mut StatLists,
    /// The unit tables.
    pub data: &'a UnitData,
    /// The shared hooks state.
    pub h: &'a mut ActionHooks<X>,
}

impl<'a, X> View<'a, X> {
    /// A view over a [`Sim`]'s parts (the game stays with the caller).
    pub fn of(
        units: &'a mut Units,
        stats: &'a mut StatLists,
        data: &'a UnitData,
        h: &'a mut ActionHooks<X>,
    ) -> Self {
        Self {
            units,
            stats,
            data,
            h,
        }
    }

    /// The [`Sim`] of a unit operation on `game`.
    pub fn sim<'b>(&'b mut self, game: &'b mut Game) -> Sim<'b> {
        Sim {
            game,
            units: self.units,
            stats: self.stats,
            data: self.data,
        }
    }
}

impl<X: Pending> View<'_, X> {
    /// Whether the item has durability. The items-row test (an
    /// item unit, `nodurability` = 0 and `durability` ≠ 0) is
    /// [`Pending::item_has_durability`]; then the maximum durability
    /// (0 without a base stat 73, else the total of stat 73) must be ≠ 0
    /// and stat 152 (`item_indesctructible`) below 1.
    pub fn item_has_durability(&self, item: UnitId) -> bool {
        const MAX_DURABILITY: u16 = 73;
        const INDESTRUCTIBLE: u16 = 152;
        self.h.x.item_has_durability(item)
            && self.stats.unit_base(item, MAX_DURABILITY, 0) != 0
            && self.stats.unit_total(item, MAX_DURABILITY, 0) != 0
            && self.stats.unit_total(item, INDESTRUCTIBLE, 0) < 1
    }
}

#[cfg(test)]
mod skill_list_order {
    use super::SkillList;

    /// Init entries, then Attack, then the class skills in record order; a
    /// level update keeps the entry's place (a new entry is appended, an
    /// existing one is updated in place).
    #[test]
    fn entries_keep_insertion_order_and_updates_stay_in_place() {
        let mut l = SkillList::default();
        for (s, lv) in [(352, 4), (0, 1), (278, 3), (36, 2)] {
            assert_eq!(l.insert(s, lv), None);
        }
        assert_eq!(l.keys().copied().collect::<Vec<_>>(), [352, 0, 278, 36]);
        assert_eq!(l.insert(0, 7), Some(1));
        assert_eq!(l.insert(352, 9), Some(4));
        assert_eq!(
            l.iter().map(|(&s, &v)| (s, v)).collect::<Vec<_>>(),
            [(352, 9), (0, 7), (278, 3), (36, 2)]
        );
        assert_eq!(l.get(&278), Some(&3));
        assert!(!l.contains_key(&1));
        // A summon's list starts as a copy of the init entries.
        let mut m = l.clone();
        m.insert(5, 1);
        assert_eq!(m.keys().copied().collect::<Vec<_>>(), [352, 0, 278, 36, 5]);
        assert_eq!(l.keys().count(), 4);
    }
}

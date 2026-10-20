//! Objects: the object control, creation and the init dispatch, the
//! object animation at a mode change, the generic init functions, preset
//! classes, the operate dispatch, the object timer events and the client
//! update messages.
//!
//! Per-function behavior lives in submodules: chests, breakables and traps
//! ([`chests`]), shrines ([`shrines`]), doors, wells, portals and
//! torches ([`misc`]). Units, rooms, items, monsters, stats and
//! messages belong to other systems; they are reached through
//! [`ObjectWorld`] and the per-module extension traits, bundled as
//! [`ObjectHost`]. Quest objects (the quest code), waypoints
//! (the waypoint code) and the `todo` rows of `object-functions.tsv`
//! are reported to the caller ([`Route`]) instead of run here. A mod
//! game runs its own operate functions first ([`ObjectProvider`]).

use std::collections::BTreeMap;

use d2_data::tables::{Leveldefs, Levels, Objects, Objgroup, Shrines};

use crate::rng::Seed;
use crate::rules::Ruleset;
use crate::units::{RoomId, UnitId};
use crate::world::quests::act3::InitPoint;
use crate::world::quests::placeholders;

pub mod chests;
#[cfg(test)]
pub(crate) mod fake;
pub mod mech;
pub mod misc;
#[cfg(test)]
mod mod_object_tests;
pub mod populate;
pub mod provider;
pub mod shrines;
#[cfg(test)]
mod tests;

pub use chests::ChestWorld;
pub use mech::MechWorld;
pub use misc::MiscWorld;
pub use provider::ObjectProvider;
pub use shrines::{ShrineWorld, StateList, StateRequest};

/// `objects.txt` rows the dispatchers accept.
pub const CLASS_BOUND: u16 = 573;
/// Init function numbers D2RUST has (0-79).
pub const INIT_FN_BOUND: u8 = 80;
/// Operate function numbers D2RUST has (0-100). A mod game's operate
/// functions may use numbers past it (MS8b).
pub const OPERATE_FN_BOUND: u8 = 101;
/// Object mode bound.
pub const MODE_BOUND: u8 = 8;
/// Classes the operate dispatch refuses.
pub const REFUSED_CLASSES: [u16; 3] = [22, 121, 122];
/// The stash class: operable with an item on the cursor.
pub const STASH_CLASS: u16 = 267;
/// Player class id of the assassin (opens locked chests without a key).
pub const ASSASSIN: u8 = 6;
/// Town levels by act.
pub const TOWNS: [u32; 5] = [1, 40, 75, 103, 109];

/// Unit flags used for objects.
pub mod oflags {
    /// Changed: sends S→C 0x0E.
    pub const CHANGED: u32 = 0x1;
    pub const SELECTABLE: u32 = 0x2;
    pub const ATTACKABLE: u32 = 0x4;
    /// Cleared at init.
    pub const INIT_CLEARED: u32 = 0x8;
    /// Keep mode: blocks the PreOperate roll (set by the preset spawner).
    pub const KEEP_MODE: u32 = 0x80;
    pub const HOVER_FREED: u32 = 0x100;
    pub const SOUND_QUEUED: u32 = 0x400;
}

/// Object timer event types.
pub mod oevent {
    pub const END_ANIM: u8 = 1;
    pub const WELL_REFILL: u8 = 2;
    pub const TRAP: u8 = 4;
    pub const SHRINE_RESET: u8 = 5;
    pub const HOVER: u8 = 6;
    pub const QUEST: u8 = 7;
    pub const DELAYED_PORTAL: u8 = 11;
}

/// Sound ids.
pub mod sound {
    pub const PORTAL: u8 = 8;
    pub const UNLOCK: u8 = 11;
    pub const TRAP_ARMED: u8 = 13;
    pub const PORTAL_REFUSED: u8 = 19;
    pub const LOCKED: u8 = 22;
}

/// Fatal object errors, returned instead of aborting.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ObjectError {
    #[error("object mode {0} ≥ 8")]
    Mode(u8),
    #[error("object class {0} ≥ 573")]
    Class(u16),
    #[error("init function {0} ≥ 80")]
    InitFn(u8),
    #[error("operate function {0} ≥ 101")]
    OperateFn(u8),
    #[error("shrine class {0} has an empty list")]
    EmptyShrineList(u8),
    #[error("town level {0} > 255")]
    TownLevel(u32),
    #[error("well charges {0} out of range at refill")]
    WellCharges(u8),
    #[error("unit {0:?} has no object data")]
    NoData(UnitId),
    #[error("{table} has no row {row}")]
    NoRow { table: &'static str, row: u32 },
    #[error("object {0} allocation failed (fatal)")]
    AllocFailed(u16),
    #[error("shrine effect with no operator (null read)")]
    ShrineNoOperator,
    #[error("portal operated with no player")]
    PortalOperator,
    #[error("portal {0:?}: no destination room")]
    NoPortalDestination(UnitId),
    #[error("portal travel: placing {0:?} failed (fatal)")]
    PortalPlacement(UnitId),
    #[error("object {0:?}: no warp tile in its room (fatal)")]
    NoWarpTile(UnitId),
    #[error("key test with no unit")]
    KeyTestNoUnit,
    #[error("room {0:?} has no active room seed")]
    NoActiveRoom(RoomId),
    #[error("populate function {0} ≥ 10")]
    PopulateFn(u8),
    #[error("populate density {0} out of range")]
    Density(u8),
    #[error("room theme {0}: no body")]
    Theme(u32),
}

// ------------------------------------------------------------------ tables

/// The tables the object code reads.
#[derive(Debug, Clone, Default)]
pub struct ObjectTables {
    pub objects: Vec<Objects>,
    pub shrines: Vec<Shrines>,
    pub levels: Vec<Levels>,
    /// `objgroup.txt` (`d2exp`; object population).
    pub objgroup: Vec<Objgroup>,
    /// `leveldefs`: the portal quest gate.
    pub leveldefs: Vec<Leveldefs>,
}

impl ObjectTables {
    pub fn object(&self, class: u16) -> Result<&Objects, ObjectError> {
        self.objects.get(class as usize).ok_or(ObjectError::NoRow {
            table: "objects",
            row: class.into(),
        })
    }
    pub fn shrine(&self, id: u16) -> Result<&Shrines, ObjectError> {
        self.shrines.get(id as usize).ok_or(ObjectError::NoRow {
            table: "shrines",
            row: id.into(),
        })
    }
    pub fn level(&self, id: u32) -> Option<&Levels> {
        self.levels.get(id as usize)
    }
    /// `MonLvl1` read as i16.
    pub fn mon_lvl1(&self, level: u32) -> i16 {
        self.level(level).map_or(0, |l| l.monlvl1 as i16)
    }
}

/// Per-mode `objects.txt` columns (indexed 0–7; `mode < 8` checked by the
/// callers).
pub fn frame_cnt(o: &Objects, mode: u8) -> u32 {
    [
        o.framecnt0,
        o.framecnt1,
        o.framecnt2,
        o.framecnt3,
        o.framecnt4,
        o.framecnt5,
        o.framecnt6,
        o.framecnt7,
    ][mode as usize & 7]
}
pub fn frame_delta(o: &Objects, mode: u8) -> u16 {
    [
        o.framedelta0,
        o.framedelta1,
        o.framedelta2,
        o.framedelta3,
        o.framedelta4,
        o.framedelta5,
        o.framedelta6,
        o.framedelta7,
    ][mode as usize & 7]
}
pub fn start(o: &Objects, mode: u8) -> u8 {
    [
        o.start0, o.start1, o.start2, o.start3, o.start4, o.start5, o.start6, o.start7,
    ][mode as usize & 7]
}
pub fn selectable(o: &Objects, mode: u8) -> u8 {
    [
        o.selectable0,
        o.selectable1,
        o.selectable2,
        o.selectable3,
        o.selectable4,
        o.selectable5,
        o.selectable6,
        o.selectable7,
    ][mode as usize & 7]
}
/// The `FrameCnt1` column value (the table stores it × 256): "fc1" of
/// the ENDANIM delays.
pub fn fc1(o: &Objects) -> i32 {
    (o.framecnt1 >> 8) as i32
}

// ----------------------------------------------------------- object data

/// An object's data plus the object unit fields
/// only object code uses (spark byte, timer argument, drop code, last
/// host tick). Kept per object unit in [`ObjectControl::data`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ObjectData {
    pub guid: u32,
    pub class: u16,
    /// `InteractType`.
    pub interact: u8,
    /// Portal flags (portal creation ORs 0x3, travel 0x5).
    pub portal_flags: u8,
    /// The shrines.txt row.
    pub shrine: Option<u16>,
    /// Operator GUID + 1 (0 = none).
    pub operator: u32,
    /// The spark byte.
    pub spark: u8,
    /// The timer argument's owner GUID (−1 at allocation).
    pub owner: Option<i32>,
    /// Drop item code (0 = none).
    pub drop_code: u32,
    /// Last host tick of a door or gate operation.
    pub last_tick: u32,
}

/// A level's object region (with the object population counters).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    /// `levels.Act`.
    pub act: u8,
    /// Rooms counted so far.
    pub counted: i32,
    /// Populated-room total, 0x7FFFFFFF until set.
    pub w08: i32,
    /// Health shrines.
    pub health: i32,
    /// Shrines (≤ 10).
    pub shrines: i32,
    /// Wells (≤ 4).
    pub wells: i32,
    /// The act's trap monster pick (−1 until a chest trap picks it).
    pub w1c: i32,
    /// Well points.
    pub well_points: [(i32, i32); 4],
    /// Shrine points.
    pub shrine_points: [(i32, i32); 10],
}

impl Region {
    /// A region as the control build leaves it.
    pub fn new(act: u8) -> Self {
        Self {
            act,
            counted: 0,
            w08: 0x7FFF_FFFF,
            health: 0,
            shrines: 0,
            wells: 0,
            w1c: -1,
            well_points: [(0, 0); 4],
            shrine_points: [(0, 0); 10],
        }
    }
}

/// The game's object control and the per-object data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectControl {
    /// The control seed.
    pub seed: Seed,
    /// One region per level id 1 … count − 1 (0 = none).
    pub regions: Vec<Option<Region>>,
    /// Shrine row ids per `effectclass` 0–7.
    pub shrine_lists: [Vec<u16>; 8],
    pub data: BTreeMap<UnitId, ObjectData>,
}

impl ObjectControl {
    /// Built once per game. Steps `game_seed` once;
    /// returns the control and `lo'`.
    pub fn new(game_seed: &mut Seed, t: &ObjectTables) -> (Self, u32) {
        let lo = game_seed.step();
        let mut regions = vec![None];
        for l in t.levels.iter().skip(1) {
            regions.push(Some(Region::new(l.act)));
        }
        let mut shrine_lists: [Vec<u16>; 8] = Default::default();
        for (i, s) in t.shrines.iter().enumerate() {
            if let Some(list) = shrine_lists.get_mut(s.effectclass as usize) {
                list.push(i as u16);
            }
        }
        (
            Self {
                seed: Seed::init_low(lo),
                regions,
                shrine_lists,
                data: BTreeMap::new(),
            },
            lo,
        )
    }

    pub fn get(&self, obj: UnitId) -> Result<&ObjectData, ObjectError> {
        self.data.get(&obj).ok_or(ObjectError::NoData(obj))
    }
    pub fn get_mut(&mut self, obj: UnitId) -> Result<&mut ObjectData, ObjectError> {
        self.data.get_mut(&obj).ok_or(ObjectError::NoData(obj))
    }
}

// ------------------------------------------------------------------ seams

/// What an operator is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    /// A player and its class id.
    Player(u8),
    Monster,
    Other,
}

/// The base seam to the rest of the game. Expected providers in brackets.
pub trait ObjectWorld {
    /// The game tick.
    fn frame(&self) -> i32;
    /// The game's tick rate: durations stated in base ticks are
    /// converted through it.
    /// No default, so no world runs at the base rate by accident.
    fn tick_rate(&self) -> crate::tick::TickRate;
    /// The host's millisecond clock (an input; `d2-sim` never reads the
    /// OS clock).
    fn host_tick(&self) -> u32;
    /// The unit's GUID.
    fn guid(&self, unit: UnitId) -> u32;
    /// The object unit with `guid`.
    fn find_object(&self, guid: u32) -> Option<UnitId>;
    /// The kind of a unit that operates (units).
    fn operator(&self, unit: UnitId) -> Operator;
    /// The unit's mode.
    fn mode(&self, unit: UnitId) -> u8;
    /// Store the mode, set flag 0x1 and, when `queue`, queue the unit for
    /// update. The animation is set by [`set_mode`] through
    /// [`ObjectWorld::set_anim`].
    fn write_mode(&mut self, unit: UnitId, mode: u8, queue: bool);
    /// The animation frame, frame count and speed.
    fn set_anim(&mut self, unit: UnitId, frame_count: i32, frame: i32, speed: i16);
    /// The unit's seed.
    fn unit_seed(&mut self, unit: UnitId) -> Option<&mut Seed>;
    /// The unit flags ([`oflags`]).
    fn flags(&self, unit: UnitId) -> u32;
    fn set_flags(&mut self, unit: UnitId, flags: u32);
    /// Queue for the update pass.
    fn queue_update(&mut self, unit: UnitId);
    /// The unit's room (path / DRLG).
    fn room(&self, unit: UnitId) -> Option<RoomId>;
    /// The level id of the unit's room (DRLG).
    fn level(&self, unit: UnitId) -> Option<u32>;
    /// The level id of a room (DRLG). Default: none.
    fn room_level(&self, room: RoomId) -> Option<u32> {
        let _ = room;
        None
    }
    /// Sub-tile position (path).
    fn position(&self, unit: UnitId) -> (i32, i32);
    /// Schedule object event `ev` at `frame`.
    fn schedule(&mut self, unit: UnitId, ev: u8, frame: i32);
    /// Cancel every timer of the unit.
    fn cancel_timers(&mut self, unit: UnitId);
    /// Stamp the object's footprint at the room and point given (an init
    /// passes its record's; the door and the gate their own).
    fn stamp_footprint(&mut self, unit: UnitId, room: Option<RoomId>, x: i32, y: i32);
    /// Free the object's footprint.
    fn free_footprint(&mut self, unit: UnitId);
    /// Attach sound `id` to `unit` (no target when `to` is `None`); `now`:
    /// sent at once to the unit's client.
    fn sound(&mut self, unit: UnitId, id: u8, to: Option<UnitId>, now: bool);
    /// The key test and use (locked chests).
    fn key_test(&mut self, player: UnitId) -> bool;
    /// The operator is in interact range ([`interact_range`]).
    fn in_interact_range(&self, operator: UnitId, object: UnitId) -> bool;
    /// The player's interact info is active.
    fn interact_active(&self, player: UnitId) -> bool;
    /// The player is busy (it can't operate).
    fn player_busy(&self, player: UnitId) -> bool;
    /// An item is on the player's cursor (inventory).
    fn cursor_item(&self, player: UnitId) -> bool;
    /// Allocate an object unit of `class` in `room` at
    /// (x, y) and `mode`. A provider that cannot run [`create`] here (the
    /// control is lent to the caller) leaves it to [`allocate`]. `None`:
    /// not allocated.
    fn allocate_object(
        &mut self,
        room: RoomId,
        class: u16,
        x: i32,
        y: i32,
        mode: u8,
    ) -> Option<UnitId>;
    /// Add an [`Self::allocate_object`] unit to the world after its
    /// [`create`] ([`allocate`]) ran: the init comes first. A provider that
    /// links in [`Self::allocate_object`] keeps the default (nothing).
    fn add_object(&mut self, _obj: UnitId, _room: RoomId, _x: i32, _y: i32) {}
    /// The room's sub-tile box (the placeholder init; default: no room data).
    fn soul_box(&mut self, _room: RoomId) -> Option<crate::drlg::TileRect> {
        None
    }
    /// The size × size box at (x, y) collides with `mask` (default: collides).
    fn soul_collides(&mut self, _room: RoomId, _x: i32, _y: i32, _size: i32, _mask: u32) -> bool {
        true
    }
    /// Spawn monster `class` at (x, y) in mode 1 (default: nothing spawned).
    fn soul_spawn(&mut self, _room: RoomId, _x: i32, _y: i32, _class: u16) -> Option<UnitId> {
        None
    }
    /// The act II staff-tomb level (the quest code).
    fn staff_tomb_level(&self) -> u32;
    /// The mode := `mode` written directly: no mode set, no animation
    /// setup, no queue, no changed flag.
    fn store_mode(&mut self, unit: UnitId, mode: u8);
    /// Init 13: quest chain `chain`'s record exists → link the object to
    /// it and return `true`. Default: no record.
    fn quest_link(&mut self, object: UnitId, chain: u8) -> bool {
        let _ = (object, chain);
        false
    }
    /// The room holding the point among `room`
    /// and its adjacency array.
    fn room_at(&self, room: RoomId, x: i32, y: i32) -> Option<RoomId> {
        let _ = (room, x, y);
        None
    }
    /// A gold drop at (x, y) in `room`. Default: nothing.
    fn gold_drop(&mut self, room: RoomId, x: i32, y: i32) {
        let _ = (room, x, y);
    }
    /// The point is free of `mask`. Default: not free.
    fn point_free(&self, room: RoomId, x: i32, y: i32, mask: u32) -> bool {
        let _ = (room, x, y, mask);
        false
    }
    /// The game's rules; a mod game's operate functions come from them
    /// ([`ObjectProvider`]). Default: D2RUST's.
    fn rules(&self) -> &Ruleset {
        &crate::rules::ORIGINAL
    }
}

/// Every seam the dispatchers need.
pub trait ObjectHost: ObjectWorld + ChestWorld + ShrineWorld + MiscWorld + MechWorld {}
impl<T: ObjectWorld + ChestWorld + ShrineWorld + MiscWorld + MechWorld> ObjectHost for T {}

// ------------------------------------------------------------------ modes

/// Store an object's `mode`, then set up its animation. `queue`: the
/// mode set queues the unit (every caller except the ENDANIM event).
pub fn set_mode<W: ObjectWorld>(
    t: &ObjectTables,
    w: &mut W,
    obj: UnitId,
    class: u16,
    mode: u8,
    queue: bool,
) -> Result<(), ObjectError> {
    if mode >= MODE_BOUND {
        return Err(ObjectError::Mode(mode));
    }
    let o = t.object(class)?;
    // The same mode only queues the unit and sets flag 0x1 (no animation
    // setup, no draw).
    let same = w.mode(obj) == mode;
    w.write_mode(obj, mode, queue);
    if same {
        return Ok(());
    }
    let frame_count = frame_cnt(o, mode) as i32;
    let frame = i32::from(start(o, mode)) * 256;
    let d = frame_delta(o, mode) as i16;
    let speed = if o.sync != 0 {
        d
    } else {
        let r = w.unit_seed(obj).map_or(0, |s| s.roll(i32::from(d >> 3))) as i32;
        // A 32-bit sum of the sign-extended values, clamped, low
        // 16 bits stored.
        let s = r + i32::from(d) - i32::from(d >> 4);
        s.clamp(0, 0x7FFF) as i16
    };
    w.set_anim(obj, frame_count, frame, speed);
    Ok(())
}

/// The mode change of an object whose class is in its data.
pub fn set_object_mode<W: ObjectWorld>(
    ctl: &ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    obj: UnitId,
    mode: u8,
) -> Result<(), ObjectError> {
    let class = ctl.get(obj)?.class;
    set_mode(t, w, obj, class, mode, true)
}

/// ENDANIM: event 1 at frame + fc1 + 1. The animation shows one
/// frame per base tick, so its `fc1` frames last `fc1` base ticks.
pub fn schedule_endanim<W: ObjectWorld>(w: &mut W, o: &Objects, obj: UnitId) {
    let at = w.frame() + w.tick_rate().ticks(fc1(o)) + 1;
    w.schedule(obj, oevent::END_ANIM, at);
}

fn set_flag<W: ObjectWorld>(w: &mut W, u: UnitId, f: u32, on: bool) {
    let v = w.flags(u);
    w.set_flags(u, if on { v | f } else { v & !f });
}

/// Clear flag 0x2 (selectable).
pub fn clear_selectable<W: ObjectWorld>(w: &mut W, u: UnitId) {
    set_flag(w, u, oflags::SELECTABLE, false);
}

// ------------------------------------------------------------------ routes

/// Who runs an init or operate function number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Run by this module.
    Here,
    /// The quest code.
    Quest,
    /// The waypoint code.
    Waypoint,
    /// Not written yet: nothing runs.
    NotCovered,
    /// Null table entry.
    Null,
}

/// The route of init function `n`.
pub fn init_route(n: u8) -> Route {
    match n {
        0 | 35 | 36 | 40 => Route::Null,
        1 | 2 | 3 | 5 | 11 | 12 | 16 | 57 => Route::Here,
        17 => Route::Waypoint,
        8 | 10 | 14 | 22 | 24 | 26 | 27 | 28 | 34 | 51 | 58 => Route::Here,
        n if n < INIT_FN_BOUND => Route::Quest,
        _ => Route::Null,
    }
}

/// The route of operate function `n`.
pub fn operate_route(n: u8) -> Route {
    match n {
        0 | 35..=38 | 60 | 74..=100 => Route::Null,
        1 | 2 | 3 | 4 | 5 | 7 | 8 | 11 | 14 | 15 | 22 | 68 => Route::Here,
        // Mechanisms ([`mech`]).
        13 | 16 | 17 | 18 | 19 | 20 | 26 | 27 | 29 | 30 | 32 | 47 | 48 | 50 | 51 | 61 => {
            Route::Here
        }
        23 => Route::Waypoint,
        n if n < OPERATE_FN_BOUND => Route::Quest,
        _ => Route::Null,
    }
}

// ------------------------------------------------------------------ creation

/// What creation ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Created {
    /// The init function's route (a quest, waypoint or not-covered init
    /// ran nothing here; the caller runs or records it).
    pub init: Route,
    pub init_fn: u8,
}

/// The init dispatch of a freshly allocated object whose unit already
/// holds `mode` (the allocation mode). Runs before the unit is added to
/// the world. [`create_init`] then [`create_rest`]; a caller that runs a
/// quest init itself calls them apart, the init between them.
#[allow(clippy::too_many_arguments)]
pub fn create<W: ObjectWorld>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    obj: UnitId,
    class: u16,
    guid: u32,
    room: Option<RoomId>,
    mode: u8,
    x: i32,
    y: i32,
) -> Result<Created, ObjectError> {
    let created = create_init(ctl, t, w, obj, class, guid, room, mode, x, y)?;
    create_rest(ctl, t, w, obj, mode)?;
    Ok(created)
}

/// The object data, the flags, the timers and the init function this
/// module runs; any other init's route is returned for the caller, who
/// runs it before [`create_rest`].
#[allow(clippy::too_many_arguments)]
pub fn create_init<W: ObjectWorld>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    obj: UnitId,
    class: u16,
    guid: u32,
    room: Option<RoomId>,
    mode: u8,
    x: i32,
    y: i32,
) -> Result<Created, ObjectError> {
    let data = ctl.data.entry(obj).or_default();
    *data = ObjectData {
        guid,
        class,
        ..ObjectData::default()
    };
    set_flag(w, obj, oflags::INIT_CLEARED, false);
    if mode >= MODE_BOUND {
        return Err(ObjectError::Mode(mode));
    }
    // The class bound before the record is read.
    if class >= CLASS_BOUND {
        return Err(ObjectError::Class(class));
    }
    let o = t.object(class)?;
    set_flag(w, obj, oflags::ATTACKABLE, o.isattackable0 != 0);
    w.cancel_timers(obj);
    let init_fn = o.initfn;
    if init_fn >= INIT_FN_BOUND {
        return Err(ObjectError::InitFn(init_fn));
    }
    let route = init_route(init_fn);
    if route == Route::Here {
        run_init(ctl, t, w, obj, init_fn, room, x, y)?;
    }
    Ok(Created {
        init: route,
        init_fn,
    })
}

/// After the init: selectable (by the allocation mode `m0`), the
/// `PreOperate` draw, the owner.
pub fn create_rest<W: ObjectWorld>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    obj: UnitId,
    m0: u8,
) -> Result<(), ObjectError> {
    let class = ctl.get(obj)?.class;
    let o = t.object(class)?;
    set_flag(w, obj, oflags::SELECTABLE, selectable(o, m0) != 0);
    if o.preoperate != 0 && w.flags(obj) & oflags::KEEP_MODE == 0 && ctl.seed.roll(14) == 0 {
        set_mode(t, w, obj, class, 2, true)?;
    }
    ctl.get_mut(obj)?.owner = Some(-1);
    Ok(())
}

/// The init functions this module runs.
#[allow(clippy::too_many_arguments)]
fn run_init<W: ObjectWorld>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    obj: UnitId,
    n: u8,
    room: Option<RoomId>,
    x: i32,
    y: i32,
) -> Result<(), ObjectError> {
    // The level of the init record's room: creation runs before the unit
    // is added to the world, so the unit has no room of its own yet.
    let level = room
        .and_then(|r| w.room_level(r))
        .or_else(|| w.level(obj))
        .unwrap_or(0);
    match n {
        1 => init_shrine(ctl, t, obj, level),
        2 => {
            init_urn(ctl, t, obj, level);
            Ok(())
        }
        3 => init_chest(ctl, t, w, obj, level),
        57 => {
            init_chest(ctl, t, w, obj, level)?;
            ctl.get_mut(obj)?.spark = 1;
            Ok(())
        }
        16 => {
            let class = ctl.get(obj)?.class;
            let p2 = t.object(class)?.parm2 as u8;
            ctl.get_mut(obj)?.interact = p2.wrapping_mul(2);
            Ok(())
        }
        5 => Ok(()),
        11 => init_town_portal(ctl, t, w, obj, level, room, x, y),
        12 => init_permanent_portal(ctl, t, w, obj, level, room, x, y),
        8 | 10 | 13 | 14 | 22 | 24 | 26 | 27 | 28 | 34 | 51 | 58 => {
            mech::init(ctl, t, w, obj, n, room, x, y)
        }
        _ => Ok(()),
    }
}

/// The shrine pick of `class` for `level`.
pub fn shrine_pick(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    class: u8,
    level: u32,
) -> Result<u16, ObjectError> {
    let class = if class == 0 || class > 4 { 2 } else { class };
    let list = &ctl.shrine_lists[class as usize];
    if list.is_empty() {
        return Err(ObjectError::EmptyShrineList(class));
    }
    let mut id = 1;
    for _ in 0..8 {
        let i = ctl.seed.roll(list.len() as i32) as usize;
        id = list[i];
        if id == 0 {
            id = 1;
        }
        if level >= t.shrine(id)?.levelmin {
            break;
        }
    }
    Ok(id)
}

/// Init 1: shrine.
fn init_shrine(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    obj: UnitId,
    level: u32,
) -> Result<(), ObjectError> {
    let class = ctl.get(obj)?.class;
    let parm0 = t.object(class)?.parm0;
    let mut id = if parm0 != 0 {
        let c = match parm0 {
            1 => 2,
            2 => 3,
            _ => {
                if ctl.seed.roll(10) == 0 {
                    1
                } else {
                    4
                }
            }
        };
        shrine_pick(ctl, t, c, level)?
    } else {
        let mut id = 1;
        for _ in 0..8 {
            let n = t.shrines.len() as i32 - 1;
            id = if n < 1 {
                1
            } else {
                ctl.seed.roll(n) as u16 + 1
            };
            if level >= t.shrine(id)?.levelmin {
                break;
            }
        }
        id
    };
    // Shrine ids 4, 5 and 16 become 2, 3 and 18.
    id = match id {
        4 => 2,
        5 => 3,
        16 => 18,
        x => x,
    };
    let d = ctl.get_mut(obj)?;
    d.interact = id as u8;
    d.shrine = Some(id);
    Ok(())
}

/// Init 2: urn.
fn init_urn(ctl: &mut ObjectControl, t: &ObjectTables, obj: UnitId, level: u32) {
    let lvl = i32::from(t.mon_lvl1(level));
    let r = ctl.seed.roll(100);
    let threshold = ((lvl / 8) + 5) as u16;
    let interact = if r < u32::from(threshold) {
        1 + ctl.seed.roll(8) as u8
    } else {
        0
    };
    if let Some(d) = ctl.data.get_mut(&obj) {
        d.interact = interact;
    }
}

/// Init 3: chest.
fn init_chest<W: ObjectWorld>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    obj: UnitId,
    level: u32,
) -> Result<(), ObjectError> {
    init_urn(ctl, t, obj, level);
    let class = ctl.get(obj)?.class;
    let lockable = t.object(class)?.lockable != 0;
    let locked = if lockable {
        let lvl = i32::from(t.mon_lvl1(level));
        (ctl.seed.roll(100) as i32) < lvl / 2 + 8
    } else {
        false
    };
    let d = ctl.get_mut(obj)?;
    d.interact = if locked {
        d.interact | 0x80
    } else {
        d.interact & 0x7F
    };
    let lo = ctl.seed.step();
    if let Some(s) = w.unit_seed(obj) {
        *s = Seed::init_low(lo % 65534 + 1);
    }
    Ok(())
}

/// Init 11: town portal.
#[allow(clippy::too_many_arguments)]
fn init_town_portal<W: ObjectWorld>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    obj: UnitId,
    level: u32,
    room: Option<RoomId>,
    x: i32,
    y: i32,
) -> Result<(), ObjectError> {
    let act = t.level(level).map_or(0, |l| l.act);
    let town = TOWNS.get(act as usize).copied().unwrap_or(0);
    if town > 255 {
        return Err(ObjectError::TownLevel(town));
    }
    let class = ctl.get(obj)?.class;
    ctl.get_mut(obj)?.interact = town as u8;
    if w.mode(obj) == 1 {
        w.stamp_footprint(obj, room, x, y);
        schedule_endanim(w, t.object(class)?, obj);
    }
    Ok(())
}

/// Init 12: permanent portal.
#[allow(clippy::too_many_arguments)]
fn init_permanent_portal<W: ObjectWorld>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    obj: UnitId,
    level: u32,
    room: Option<RoomId>,
    x: i32,
    y: i32,
) -> Result<(), ObjectError> {
    let class = ctl.get(obj)?.class;
    let o = t.object(class)?;
    if w.mode(obj) == 0 {
        set_mode(t, w, obj, class, 1, true)?;
        w.stamp_footprint(obj, room, x, y);
        schedule_endanim(w, o, obj);
    }
    let tomb = w.staff_tomb_level();
    let d = ctl.get_mut(obj)?;
    let dest = match level {
        1 => Some(39),
        39 => Some(1),
        38 => Some(4),
        4 => Some(38),
        74 => Some(46),
        46 => Some(74),
        73 => Some(tomb),
        l if l == tomb => Some(73),
        121 => Some(109),
        109 => Some(121),
        125 => Some(111),
        126 => Some(112),
        127 => Some(117),
        _ => None,
    };
    if let Some(dest) = dest {
        d.interact = dest as u8;
    } else if let Some(want) = match level {
        111 => Some(125u8),
        112 => Some(126),
        117 => Some(127),
        _ => None,
    } {
        if d.interact != want {
            d.interact = want;
            let at = w.frame() + 1;
            w.schedule(obj, oevent::DELAYED_PORTAL, at);
        }
    }
    Ok(())
}

/// An object allocated from inside an object call: the unit through
/// [`ObjectWorld::allocate_object`], then, when the provider could not
/// run the init dispatch (the control is held by the caller), creation
/// on it here.
#[allow(clippy::too_many_arguments)]
pub fn allocate<W: ObjectWorld>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    room: RoomId,
    class: u16,
    x: i32,
    y: i32,
    mode: u8,
) -> Result<Option<UnitId>, ObjectError> {
    let Some(obj) = w.allocate_object(room, class, x, y, mode) else {
        return Ok(None);
    };
    if !ctl.data.contains_key(&obj) {
        let guid = w.guid(obj);
        let created = create_init(ctl, t, w, obj, class, guid, Some(room), mode, x, y)?;
        // The trapped-soul placeholder's init (init 46) runs here, between
        // the init dispatch and the `PreOperate` draw, on the control seed
        // that is lent to this call.
        if created.init == Route::Quest && created.init_fn == TRAPPED_SOUL_INIT {
            let mut souls = Souls {
                w: &mut *w,
                seed: &mut ctl.seed,
                t,
                class,
            };
            placeholders::trapped_soul_init(&mut souls, obj, Some(InitPoint { room, x, y }));
        }
        create_rest(ctl, t, w, obj, mode)?;
        w.add_object(obj, room, x, y);
    }
    Ok(Some(obj))
}

/// Init function 46: the trapped-soul placeholder.
const TRAPPED_SOUL_INIT: u8 = 46;

/// The placeholder init's world on an object call: the control seed
/// lent to the call and the object world's seams.
struct Souls<'a, W> {
    w: &'a mut W,
    seed: &'a mut crate::rng::Seed,
    t: &'a ObjectTables,
    class: u16,
}

impl<W: ObjectWorld> placeholders::SoulWorld for Souls<'_, W> {
    fn soul_seed(&mut self) -> Option<&mut crate::rng::Seed> {
        Some(&mut *self.seed)
    }
    fn soul_mode(&self, object: UnitId) -> i32 {
        i32::from(self.w.mode(object))
    }
    fn soul_set_mode(&mut self, object: UnitId, mode: i32) {
        if let Ok(m) = u8::try_from(mode) {
            let _ = set_mode(self.t, self.w, object, self.class, m, true);
        }
    }
    fn soul_box(&mut self, room: RoomId) -> Option<crate::drlg::TileRect> {
        self.w.soul_box(room)
    }
    fn soul_collides(&mut self, room: RoomId, x: i32, y: i32, size: i32, mask: u32) -> bool {
        self.w.soul_collides(room, x, y, size, mask)
    }
    fn soul_spawn(&mut self, room: RoomId, x: i32, y: i32, class: u16) -> Option<UnitId> {
        self.w.soul_spawn(room, x, y, class)
    }
}

// ------------------------------------------------------------------ presets

/// {class, min, max} of preset shrines 574–579.
pub const PRESET_SHRINES: [(u16, u16, u16); 6] = [
    (136, 2, 6),
    (136, 7, 7),
    (136, 8, 11),
    (136, 12, 12),
    (136, 1, 5),
    (136, 14, 14),
];

/// What a preset class 574–582 did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// Class ≤ 573 or index ≥ 9: no handler.
    None,
    /// The object allocated (or `None` when the allocation failed).
    Object(Option<UnitId>),
    /// A handler run by the host: 582 (its class comes from the quest
    /// code).
    NotCovered(u32),
}

/// The spawner of DS1 preset classes beyond `objects.txt`.
#[allow(clippy::too_many_arguments)]
pub fn create_preset<W: ObjectHost>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    room: RoomId,
    level: u32,
    class: u32,
    x: i32,
    y: i32,
    mode: u8,
) -> Result<Preset, ObjectError> {
    if class <= 573 {
        return Ok(Preset::None);
    }
    let index = class - 574;
    if index >= 9 {
        return Ok(Preset::None);
    }
    match class {
        574..=579 => {
            let (c, min, max) = PRESET_SHRINES[index as usize];
            let Some(obj) = allocate(ctl, t, w, room, c, x, y, mode)? else {
                return Ok(Preset::Object(None));
            };
            let r = w
                .unit_seed(obj)
                .map_or(0, |s| s.roll(i32::from(max) - i32::from(min)));
            let mut id = min + r as u16;
            if id == 4 || id == 5 {
                id = 2;
            }
            if let Some(d) = ctl.data.get_mut(&obj) {
                d.interact = id as u8;
                d.shrine = Some(id);
            }
            Ok(Preset::Object(Some(obj)))
        }
        580 => {
            // 580 runs 581's handler with 371 at level 25, else with its
            // own class (the class draw happens at every level).
            let c = if level == 25 { PRESET_CHEST_371 } else { 580 };
            let Some(obj) = preset_chest(ctl, t, w, room, level, c, x, y)? else {
                return Ok(Preset::Object(None));
            };
            let spark = if (62..=64).contains(&level) {
                ctl.seed.roll(100) <= 50
            } else {
                true
            };
            if spark {
                if let Some(d) = ctl.data.get_mut(&obj) {
                    d.spark = 1;
                    if level == 25 {
                        d.interact = 3;
                    }
                }
            }
            set_flag(w, obj, oflags::KEEP_MODE, true);
            // The ordinary mode set (already in mode 0: no setup, no draw;
            // queued, flag 0x1).
            let oc = ctl.get(obj)?.class;
            set_mode(t, w, obj, oc, 0, true)?;
            Ok(Preset::Object(Some(obj)))
        }
        581 => Ok(Preset::Object(preset_chest(
            ctl, t, w, room, level, class, x, y,
        )?)),
        _ => Ok(Preset::NotCovered(class)),
    }
}

/// The class argument that replaces 581's pick (580 at level 25).
const PRESET_CHEST_371: u32 = 371;

/// 581's picks for Acts I, IV and V (`lo' mod 13`).
const PRESET_CHESTS_OTHER: [u16; 13] =
    [5, 6, 139, 140, 141, 144, 176, 177, 198, 240, 241, 242, 243];

/// Preset chest 581: one control-seed step picks the chest class by the
/// act of the room's level; a class argument of 371 replaces the pick.
/// The object is allocated in mode 0 whatever the preset mode; its own
/// init runs inside the allocation.
#[allow(clippy::too_many_arguments)]
fn preset_chest<W: ObjectHost>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    room: RoomId,
    level: u32,
    class: u32,
    x: i32,
    y: i32,
) -> Result<Option<UnitId>, ObjectError> {
    let act = t.level(level).map_or(0, |l| l.act);
    let lo = ctl.seed.step();
    let pick = match act {
        1 if level == 74 => [387, 389, 390, 391][(lo & 3) as usize],
        1 => [87, 88][(lo & 1) as usize],
        2 if level == 83 => [329, 330, 331, 332][(lo & 3) as usize],
        2 => [181, 183][(lo & 1) as usize],
        _ => PRESET_CHESTS_OTHER[(lo % 13) as usize],
    };
    let c = if class == PRESET_CHEST_371 { 371 } else { pick };
    allocate(ctl, t, w, room, c, x, y, 0)
}

// ------------------------------------------------------------------ operate

/// One operate call's record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Operate {
    pub object: UnitId,
    pub operator: Option<UnitId>,
    pub class: u16,
    pub operate_fn: u8,
}

/// What the dispatch did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dispatch {
    /// A function ran here (or the dispatch returned without one): its
    /// return value.
    Done(i32),
    /// Operate 23: the caller runs the waypoint code.
    Waypoint(Operate),
    /// A quest function: the caller runs the quest code's.
    Quest(Operate),
    /// A `todo` function: nothing ran. Also a portal (operate 15) whose
    /// destination step the host left unrun.
    NotCovered(Operate),
}

/// The operate entry with the range check. Returns the entry's result
/// (0: no such object, else 1) and what the dispatch did.
pub fn operate_in_range<W: ObjectHost>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    operator: Option<UnitId>,
    guid: u32,
) -> Result<(i32, Option<Dispatch>), ObjectError> {
    let Some(obj) = w.find_object(guid) else {
        return Ok((0, None));
    };
    if let Some(op) = operator {
        if w.operator(op) == Operator::Monster {
            let class = ctl.get(obj)?.class;
            if t.object(class)?.monsterok == 0 {
                return Ok((1, None));
            }
        }
        if !w.in_interact_range(op, obj) {
            return Ok((1, None));
        }
    }
    let d = dispatch(ctl, t, w, obj, operator)?;
    Ok((1, Some(d)))
}

/// The operate dispatch (skills call it directly).
pub fn dispatch<W: ObjectHost>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    obj: UnitId,
    operator: Option<UnitId>,
) -> Result<Dispatch, ObjectError> {
    let class = ctl.get(obj)?.class;
    // A player can't operate while interacting, busy or holding an item
    // (except at the stash).
    if let Some(p) = operator {
        if let Operator::Player(_) = w.operator(p) {
            if w.interact_active(p)
                || w.player_busy(p)
                || (w.cursor_item(p) && class != STASH_CLASS)
            {
                return Ok(Dispatch::Done(0));
            }
        }
    }
    let n = t.object(class)?.operatefn;
    // A mod game's functions may use numbers past D2RUST's bound.
    let rules = w.rules().clone();
    let provider = rules.objects().filter(|p| p.has_operate(n));
    if n >= OPERATE_FN_BOUND && provider.is_none() {
        return Err(ObjectError::OperateFn(n));
    }
    if REFUSED_CLASSES.contains(&class) {
        return Ok(Dispatch::Done(0));
    }
    crate::cov!(Object, class, n);
    let op = Operate {
        object: obj,
        operator,
        class,
        operate_fn: n,
    };
    // The mod's function first; `None` falls through to D2RUST's.
    if let Some(p) = provider {
        if let Some(r) = p.operate(ctl, t, w, &op) {
            return r.map(Dispatch::Done);
        }
    }
    // The function's route.
    Ok(match operate_route(n) {
        Route::Null => Dispatch::Done(0),
        Route::Waypoint => Dispatch::Waypoint(op),
        Route::Quest => Dispatch::Quest(op),
        Route::NotCovered => Dispatch::NotCovered(op),
        Route::Here => match run_operate(ctl, t, w, &op)? {
            Some(r) => Dispatch::Done(r),
            None => Dispatch::NotCovered(op),
        },
    })
}

/// The operate functions this module runs; `None`: the function stopped
/// at a step not written yet (the portal's destination).
fn run_operate<W: ObjectHost>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    op: &Operate,
) -> Result<Option<i32>, ObjectError> {
    match op.operate_fn {
        1 | 3 | 4 | 5 | 7 | 14 | 68 => chests::operate(ctl, t, w, op).map(Some),
        2 => shrines::operate(ctl, t, w, op).map(Some),
        8 => misc::door(ctl, t, w, op).map(Some),
        11 => misc::torch(ctl, t, w, op).map(Some),
        15 => misc::portal(ctl, t, w, op),
        22 => misc::well(ctl, t, w, op).map(Some),
        13 | 16..=20 | 26 | 27 | 29 | 30 | 32 | 47 | 48 | 50 | 51 | 61 => {
            mech::operate(ctl, t, w, op).map(Some)
        }
        _ => Ok(Some(0)),
    }
}

// ------------------------------------------------------------------ events

/// What an object timer event did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventRun {
    Done,
    /// Event 7: the quest object event (the quest code).
    Quest,
    /// An event type with no handler.
    NotCovered(u8),
}

/// The object timer event handlers.
pub fn object_event<W: ObjectHost>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    obj: UnitId,
    ev: u8,
) -> Result<EventRun, ObjectError> {
    match ev {
        oevent::END_ANIM => end_anim(ctl, t, w, obj)?,
        oevent::WELL_REFILL => misc::well_refill(ctl, t, w, obj)?,
        oevent::TRAP => chests::trap_event(ctl, t, w, obj)?,
        oevent::SHRINE_RESET => shrines::reset_event(ctl, t, w, obj)?,
        oevent::HOVER => shrines::hover_event(ctl, t, w, obj)?,
        oevent::QUEST => return Ok(EventRun::Quest),
        oevent::DELAYED_PORTAL => delayed_portal(ctl, t, w, obj)?,
        // Mechanisms.
        0 | 3 | 8 | 9 | 10 => mech::event(ctl, t, w, obj, ev)?,
        e => return Ok(EventRun::NotCovered(e)),
    }
    Ok(EventRun::Done)
}

/// Event 1: mode 1 and `Mode2` ≠ 0 → the mode field := 2 written
/// directly (no setup, draw, queue or changed flag); inside that branch,
/// `HasCollision2` = 0 → free the footprint.
fn end_anim<W: ObjectWorld>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    obj: UnitId,
) -> Result<(), ObjectError> {
    let class = ctl.get(obj)?.class;
    let o = t.object(class)?;
    if w.mode(obj) == 1 && o.mode2 != 0 {
        w.store_mode(obj, 2);
        if o.hascollision2 == 0 {
            w.free_footprint(obj);
        }
    }
    Ok(())
}

/// Event 11: a portal object by the object's level: 111 → 125, 112 →
/// 126, else 127.
///
/// TODO: the portal creator's placement and mode are not known yet; the
/// matching portal is requested from the seam.
fn delayed_portal<W: ObjectHost>(
    _ctl: &mut ObjectControl,
    _t: &ObjectTables,
    w: &mut W,
    obj: UnitId,
) -> Result<(), ObjectError> {
    let level = w.level(obj).unwrap_or(0);
    let row = match level {
        111 => 125,
        112 => 126,
        _ => 127,
    };
    w.create_level_portal(obj, row);
    Ok(())
}

// ------------------------------------------------------------------ client messages

/// S→C 0x0E (12 bytes): the object state.
pub fn state_message(guid: u32, selectable: bool, mode: u32) -> [u8; 12] {
    let mut m = [0u8; 12];
    m[0] = 0x0E;
    m[1] = 2;
    m[2..6].copy_from_slice(&guid.to_le_bytes());
    m[6] = 3;
    m[7] = u8::from(selectable);
    m[8..12].copy_from_slice(&mode.to_le_bytes());
    m
}

/// S→C 0x4D (17 bytes): a shrine operated.
pub fn shrine_message(object: u32, operator: u32, code: u8) -> [u8; 17] {
    let mut m = [0u8; 17];
    m[0] = 0x4D;
    m[1] = 2;
    m[2..6].copy_from_slice(&object.to_le_bytes());
    m[6..10].copy_from_slice(&operator.to_le_bytes());
    m[10] = code;
    m
}

/// S→C 0x60 (7 bytes): portal flags, `InteractType` (destination level),
/// object GUID.
pub fn portal_message(d: &ObjectData) -> [u8; 7] {
    let g = d.guid.to_le_bytes();
    [0x60, d.portal_flags, d.interact, g[0], g[1], g[2], g[3]]
}

/// What the update pass sends for one queued object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateMessage {
    State([u8; 12]),
    Shrine([u8; 17]),
    /// S→C 0x60 ([`portal_message`]).
    Portal([u8; 7]),
}

/// The update pass for one queued object: the messages it sends to each
/// client, in order. The sound, hover and flag work goes through
/// [`MiscWorld::update_extras`].
pub fn update_messages<W: ObjectWorld>(
    ctl: &ObjectControl,
    t: &ObjectTables,
    w: &W,
    obj: UnitId,
) -> Result<Vec<UpdateMessage>, ObjectError> {
    let mut out = Vec::new();
    let flags = w.flags(obj);
    if flags & oflags::CHANGED == 0 {
        return Ok(out);
    }
    let d = ctl.get(obj)?;
    let mode = w.mode(obj);
    out.push(UpdateMessage::State(state_message(
        d.guid,
        flags & oflags::SELECTABLE != 0,
        u32::from(mode),
    )));
    let o = t.object(d.class)?;
    if o.subclass & 4 != 0 {
        out.push(UpdateMessage::Portal(portal_message(d)));
    } else if mode == 1 && o.subclass & 1 != 0 && d.operator != 0 {
        let code = match d.shrine {
            Some(id) => t.shrine(id)?.code,
            None => 0,
        };
        // The operator field − 1 (the operator's GUID).
        out.push(UpdateMessage::Shrine(shrine_message(
            d.guid,
            d.operator.wrapping_sub(1),
            code,
        )));
    }
    Ok(out)
}

#[cfg(test)]
mod mutant_tests;

/// Interact range `R(U, O)`: `u` the unit's whole-sub-tile position and
/// its X size, `o` the object's position and `objects.txt` `SizeX` /
/// `SizeY`, `dist` the unit distance of U to O. The widened footprint
/// less its four outer corner cells; a unit wider than 2 skips the
/// corner cut.
pub fn interact_range(
    u: (i32, i32),
    u_size_x: i32,
    o: (i32, i32),
    size: (i32, i32),
    dist: i32,
) -> bool {
    // At distance 0.
    if dist == 0 {
        return true;
    }
    // The halves truncate.
    let (sx, sy) = size;
    let (bx, by) = (o.0 - sx / 2, o.1 - sy / 2);
    // No size: the 3 × 3 box around the base.
    if sx < 1 || sy < 1 {
        return (bx - 1..=bx + 1).contains(&u.0) && (by - 1..=by + 1).contains(&u.1);
    }
    // Outside the widened footprint.
    if !(bx - 2..=bx + sx + 2).contains(&u.0) || !(by - 2..=by + sy + 2).contains(&u.1) {
        return false;
    }
    // A unit wider than 2 skips the corner cut (unsigned compare).
    if u_size_x as u32 > 2 {
        return true;
    }
    // The footprint less its four outer corners.
    (by - 1..=by + sy + 1).contains(&u.1) || (bx - 1..=bx + sx + 1).contains(&u.0)
}

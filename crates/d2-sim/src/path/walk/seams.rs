//! What walk code needs from systems it does not own.
//!
//! Walk code works on the core's [`DynamicPath`]
//! ([`super::super::record`]); a provider copies the unit's record out and back
//! ([`PathWorld::load_path`], [`PathWorld::store_path`]). [`PathWorld`]
//! is the record store, footprint and room-list side on top of the
//! core's [`CollisionRooms`]; the walk code
//! calls the core's collision and footprint functions on it.
//! [`WalkUnits`] is everything unit-, mode-,
//! stat-, skill- and message-shaped; its defaults are the narrowest
//! reading (nothing happens, or the value that makes the caller do
//! nothing), like `wiring::action::Pending`, and each default names the
//! system that will own it.

use super::resync::ResyncRing;
use crate::path::collision::CollisionRooms;
use crate::path::history::PositionHistory;
use crate::path::record::DynamicPath;
use crate::path::PathError;
use crate::rng::Seed;
use crate::tick::TickRate;
use crate::units::{ClientId, RoomId, UnitId, UnitType};

/// The sub-tile point type (moved to the path core).
pub use crate::path::coords::Point;
/// The target unit (+0x58..+0x60), now a field of the core record.
pub use crate::path::record::TargetUnit;

/// Current point index (+0x24), read signed.
pub(crate) fn index(path: &DynamicPath) -> i32 {
    path.cur_point as i32
}

/// Point count (+0x28), read signed.
pub(crate) fn count(path: &DynamicPath) -> i32 {
    path.point_count as i32
}

/// `p` inside the room's sub-tile rect (half-open, room +0x4C..+0x58).
/// A room without a rect (not active) contains no cell.
pub(crate) fn room_contains<R: CollisionRooms + ?Sized>(rooms: &R, room: RoomId, p: Point) -> bool {
    rooms
        .subtile_rect(room)
        .is_some_and(|r| r.contains(p.x, p.y))
}

/// A fatal error (an assert or a null dereference the
/// game does not survive).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WalkError {
    #[error("fatal: {0}")]
    Fatal(&'static str),
    /// A fatal assert of the path core (e.g. the type set).
    #[error("path core: {0}")]
    Path(#[from] PathError),
}

/// The skills.txt fields of the unit's used skill that walk code reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsedSkill {
    /// Skill id.
    pub id: u16,
    /// `SeqInput` (+0x16).
    pub seq_input: i32,
    /// `srvdofunc` (+0x2E).
    pub srvdofunc: i32,
    /// `interrupt` flag (byte +7 & mask).
    pub interrupt: bool,
    /// The skill flags read by the walk code (skill +0x0C).
    pub skill_flags: u32,
}

/// What the path core does not compute itself: path record storage,
/// footprints by unit, town rooms and room lists. Collision queries,
/// cell lookup and footprint moves are the core's functions
/// ([`crate::path::collision`], [`crate::path::footprint`]) over the
/// [`CollisionRooms`] supertrait. Provider: the wiring, holding the
/// game, units, DRLG grids and path store in one object (with
/// [`WalkUnits`]).
pub trait PathWorld: CollisionRooms {
    /// Copies the unit's dynamic path out; `None` = no path (unit +0x2C
    /// null).
    fn load_path(&self, unit: UnitId) -> Option<DynamicPath>;
    /// Writes the record back into the unit's path.
    fn store_path(&mut self, unit: UnitId, path: &DynamicPath);
    /// The room is in a town.
    fn room_in_town(&self, room: RoomId) -> bool;
    /// Footprint remove `(unit, force)` at the unit's stored
    /// position; returns whether it cleared.
    fn remove_footprint(&mut self, unit: UnitId, force: bool) -> bool;
    /// Footprint add `(unit)` at the unit's stored position.
    fn add_footprint(&mut self, unit: UnitId);
    /// The unit leaves its room's unit list.
    fn room_list_remove(&mut self, unit: UnitId, room: RoomId);
    /// Room list insert.
    fn room_list_insert(&mut self, unit: UnitId, room: RoomId);
    /// Queue for update.
    fn queue_for_update(&mut self, unit: UnitId);
    /// The room's client array, sorted by client address (the DRLG
    /// room rule).
    fn room_clients(&self, room: RoomId) -> Vec<ClientId>;
    /// The room test: `room` (path +0x20) is still a room of
    /// the unit's act. Default: yes.
    fn room_in_unit_act(&self, unit: UnitId, room: RoomId) -> bool {
        let _ = (unit, room);
        true
    }
}

/// Units, modes, timers, stats, skills and messages. Every default is
/// the narrowest reading; the owning system is named on each.
#[allow(unused_variables)]
pub trait WalkUnits {
    /// Unit type (unit +0x00).
    fn unit_type(&self, unit: UnitId) -> UnitType;
    /// Class (unit +0x04).
    fn class(&self, unit: UnitId) -> u32 {
        0
    }
    /// GUID (unit +0x0C).
    fn guid(&self, unit: UnitId) -> u32 {
        unit.0
    }
    /// The game frame (`Game::frame`), read by the mode request.
    fn frame(&self) -> i32;
    /// The game's tick rate: walk speeds, drains and movement timings
    /// are stated at the base rate and converted through it.
    /// No default, so no world runs at the base rate by accident.
    fn tick_rate(&self) -> TickRate;
    /// Current mode (unit +0x10).
    fn mode(&self, unit: UnitId) -> u32;
    /// Unit lookup by type and GUID.
    fn find_unit(&self, ty: UnitType, guid: u32) -> Option<UnitId> {
        None
    }
    /// Position of any unit in sub-tiles.
    fn position(&self, unit: UnitId) -> Point {
        Point::default()
    }
    /// Unit size.
    fn unit_size(&self, unit: UnitId) -> i32 {
        0
    }
    /// Door test; `Some(orientation)` for a
    /// door. Owner: the objects spec.
    fn door_orientation(&self, unit: UnitId) -> Option<bool> {
        None
    }
    /// The smallest positive expire among the unit's type-1 timers
    /// (0 if none). Owner: the tick.
    fn first_type1_expire(&self, unit: UnitId) -> i32 {
        0
    }
    /// Inventory cursor item (inventory +0x20 ≠ 0). Owner: inventory spec.
    fn has_cursor_item(&self, unit: UnitId) -> bool {
        false
    }
    fn has_state(&self, unit: UnitId, state: u16) -> bool {
        false
    }
    /// `stat` of the stat list of `state`.
    fn state_stat(&self, unit: UnitId, state: u16, stat: u16) -> i32 {
        0
    }
    /// Stat value, the unit total.
    fn stat(&self, unit: UnitId, stat: u16) -> i32 {
        0
    }
    /// Item/skill stat getter.
    fn item_stat(&self, unit: UnitId, stat: u16) -> i32 {
        0
    }
    /// Base stat add.
    fn add_base_stat(&mut self, unit: UnitId, stat: u16, delta: i32) {}
    /// Base stat set.
    fn set_base_stat(&mut self, unit: UnitId, stat: u16, value: i32) {}
    /// The unit seed (unit +0x20).
    fn seed(&mut self, unit: UnitId) -> &mut Seed;
    /// The used skill's skills.txt fields; `None` = no used
    /// skill or no row.
    fn used_skill(&self, unit: UnitId) -> Option<UsedSkill> {
        None
    }
    /// Sets the used skill (`None` = no skill).
    fn set_used_skill(&mut self, unit: UnitId, skill: Option<u16>) {}
    /// Player data +0x154 := 0 and +0x150 := 0.
    fn clear_queued_action(&mut self, unit: UnitId) {}
    /// Mode set. The velocity half of
    /// the animation-rate routine is [`super::velocity`]; the
    /// walk entry points run it right after this call.
    fn set_mode(&mut self, unit: UnitId, mode: u32) {}
    /// Cancel the unit's events of a type.
    fn cancel_events(&mut self, unit: UnitId, ty: u8) {}
    /// Every-tick event 0.
    fn schedule_event0(&mut self, unit: UnitId) {}
    /// Start function of a mode other than 2, 3, 6, 19 (table
    /// point or unit form. Owner: the mode's code.
    fn start_other_mode(&mut self, unit: UnitId, mode: u32, target: StartTarget) {}
    /// Starting mode 3 or 19 attaches the run stat list
    /// (flag 4, stat 67 = `value`). Owner: the stat lists.
    fn attach_run_stats(&mut self, unit: UnitId, value: i32) {}
    /// charstats `WalkVelocity`, `RunVelocity`, `RunDrain` of a player.
    fn charstats_velocity(&self, unit: UnitId) -> (i32, i32, i32) {
        (0, 0, 0)
    }
    /// monstats `Velocity` and the `npc` bit of a monster.
    fn monstats_velocity(&self, unit: UnitId) -> (i32, bool) {
        (0, false)
    }
    /// The velocity base `(type, class)` of velocity rule 2 (× 256
    /// applied by the caller): charstats `WalkVelocity` for a player,
    /// monstats `Velocity` for a monster, of the unit's **draw identity**
    /// (the draw identity). Default: the unit's own type and class.
    fn velocity_base(&self, unit: UnitId) -> i32 {
        match self.unit_type(unit) {
            UnitType::Player => self.charstats_velocity(unit).0,
            _ => self.monstats_velocity(unit).0,
        }
    }
    /// armor `speed` of the torso item (body location 3); `None` = none.
    fn torso_speed(&self, unit: UnitId) -> Option<i32> {
        None
    }
    /// The monster may be in town.
    fn monster_can_be_in_town(&self, unit: UnitId) -> bool {
        true
    }
    /// Unit flag set (unit +0xC4).
    fn set_unit_flag(&mut self, unit: UnitId, bit: u32) {}
    /// State 13 event-0 call; its result is ignored and
    /// the step goes on. Owner: the skills spec.
    fn state13_step(&mut self, unit: UnitId) {}
    /// State 22 call of the monster walk event 0.
    /// Owner: the skills spec.
    fn state22_step(&mut self, unit: UnitId) {}
    /// A monster's AI room memo (monster data +0x50) := 0.
    fn clear_ai_room_memo(&mut self, unit: UnitId) {}
    /// The client's player (client record); `None` = none.
    fn client_player(&self, client: ClientId) -> Option<UnitId> {
        None
    }
    /// Unit removal message (S→C 0x0A, type and GUID).
    /// Owner: the unit-update spec.
    fn send_unit_removal(&mut self, client: ClientId, unit: UnitId) {}
    /// Unit add messages. Owner: the unit-update spec.
    fn send_unit_add(&mut self, client: ClientId, unit: UnitId) {}
    /// The player's position history (player data +0xA0..+0x14C,
    /// the placement position-history rule); `None` = not kept (nothing is
    /// written).
    fn position_history(&mut self, unit: UnitId) -> Option<&mut PositionHistory> {
        None
    }

    // ---- C→S 0x5F resync ---------------------------------------

    /// The player's client (player data +0x9C) exists; the
    /// handler treats a null client as fatal.
    fn has_client(&self, unit: UnitId) -> bool {
        true
    }
    /// The player is dead. Owner: the units code.
    fn is_dead(&self, unit: UnitId) -> bool {
        false
    }
    /// The placement
    /// (unit placement) of the resync; true = placed. The caller
    /// stores the path record before and loads it after.
    fn place_resync(&mut self, unit: UnitId, x: i32, y: i32) -> bool {
        false
    }
    /// The client's 5-slot resync ring (client +0x3C0); `None` = not kept
    /// (nothing recorded, never full).
    fn resync_ring(&mut self, unit: UnitId) -> Option<&mut ResyncRing> {
        None
    }
    /// The game type byte (game +0x6A).
    fn game_type(&self) -> u8 {
        0
    }
    /// The resync lock: set state 108; attach a stat list
    /// (flags 2, expire `expire`, state 108, remove callback
    /// Clearing state 108); schedule event 12 at `expire`.
    fn resync_lock(&mut self, unit: UnitId, expire: i32) {}
    /// Sends message bytes to the player's own client.
    fn send_to_client(&mut self, unit: UnitId, bytes: &[u8]) {}
}

/// Target of a mode start (point or unit form).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartTarget {
    Point(Point),
    Unit(UnitId),
}

/// Search inputs: endpoints and rooms, distance limits, path type,
/// footprint size and pattern, and movement collision mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathInfo {
    pub start: Point,
    pub target: Point,
    pub start_room: Option<RoomId>,
    pub target_room: Option<RoomId>,
    /// Target slack r.
    pub slack: i32,
    pub max_distance: i32,
    pub idastar_score: i32,
    pub path_type: u32,
    pub size: i32,
    pub pattern: u32,
    pub move_mask: u16,
}

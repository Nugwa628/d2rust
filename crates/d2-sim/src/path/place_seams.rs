//! Seams of the free-point searches, unit placement and warps
//! ([`super::search`], [`super::place`], [`super::warp`]).
//!
//! [`CollisionView`] lists exactly the path-core operations
//! those modules call; its
//! provider is the path core (path records, collision grids,
//! footprints). [`PlaceHost`] holds what the units, messages and walk
//! code answer, [`LevelView`] what the DRLG answers, [`WarpTileView`] the
//! DRLG side of the warp tile preset. None of them has a provider
//! yet: every method states the rule it stands for, and the hosts
//! that have no answer take the narrowest default (nothing happens).

use std::fmt::Debug;

use super::coords::Point;
use crate::drlg::TileRect;

/// Collision masks used by the searches, placement and warps.
pub mod mask {
    /// Player move / placement: WALL, NOPLAYER, OBJECT, DOOR, NO_PATH.
    pub const PLAYER_PLACE: u32 = 0x1C09;
    /// Item floor: WALL, ITEM, OBJECT, DOOR, NO_PATH, PET.
    pub const ITEM_FLOOR: u32 = 0x3E01;
    /// Walk-back field: WALL, DOOR.
    pub const FIELD: u32 = 0x801;
    /// The act change's free point.
    pub const ACT_CHANGE: u32 = 0x1C89;
    /// The portal pair's field-search field mask (the portal objects'
    /// Rule 5; the spot mask is [`ITEM_FLOOR`]).
    pub const PORTAL_FIELD: u32 = 0xC01;
    /// The portal pair's destination free point (the portal objects'
    /// And 13).
    pub const PORTAL_DEST: u32 = 0xBE11;
}

/// The value a cell without a room (or a room without a grid) reads,
/// unmasked.
pub const MISSING_ROOM_VALUE: u32 = 0x27;

/// The path-core operations that the searches, placement and warps call.
/// Provider: the path core (`d2_sim::path`).
pub trait CollisionView {
    /// An active room (the collision grid's owner).
    type Room: Copy + Eq + Debug;
    /// A unit with a path record.
    type Unit: Copy + Eq + Debug;

    // ---- collision queries --------------------------------------

    /// Rule 1, cell lookup: the hint room if
    /// its rect contains the cell, else the first room of its adjacency
    /// array containing it, else none; a null hint gives none.
    fn cell_room(&self, hint: Option<Self::Room>, x: i32, y: i32) -> Option<Self::Room>;
    /// The sub-tile rect of a room (room +0x4C..+0x58).
    fn room_rect(&self, room: Self::Room) -> TileRect;
    /// Rule 2, cell value: the grid value of the cell looked up from
    /// `room`, unmasked; no room or no grid → [`MISSING_ROOM_VALUE`].
    fn cell_value(&self, room: Self::Room, x: i32, y: i32) -> u32;
    /// Point query: the masked cell value
    /// (rule 2: a missing room reads 0x27 unmasked).
    fn point_query(&self, room: Self::Room, x: i32, y: i32, mask: u32) -> u32;
    /// Size query: size 0, 1 point; 2 plus;
    /// 3 3×3 box; other sizes 0xFFFF. OR of the masked values.
    fn size_query(&self, room: Self::Room, x: i32, y: i32, size: i32, mask: u32) -> u32;
    /// Box query: the `sx` × `sy` box centred
    /// on (x, y) (left = x − sx/2, bottom = y − sy/2, unsigned halving),
    /// clipped into sub-boxes at room edges; OR of the masked values.
    fn box_query(&self, room: Self::Room, x: i32, y: i32, sx: u32, sy: u32, mask: u32) -> u32;

    // ---- unit path facts ------------------------------------

    /// The unit has a path record (unit +0x2C ≠ null).
    fn has_path(&self, unit: Self::Unit) -> bool;
    /// Room of a unit.
    fn unit_room(&self, unit: Self::Unit) -> Option<Self::Room>;
    /// Unit size.
    fn unit_size(&self, unit: Self::Unit) -> i32;

    // ---- moves --------------------------------------------

    /// Teleport: always
    /// succeeds (footprint move, position, room, movement reset).
    fn teleport(&mut self, unit: Self::Unit, room: Self::Room, x: i32, y: i32);
    /// Adding a unit to the world (flag 0) as game
    /// entry calls it for the player: the room-changed
    /// flag is set.
    fn add_player_to_world(&mut self, unit: Self::Unit, room: Self::Room, x: i32, y: i32);
    /// Game entry's room switch of the player's client to `room`
    /// (game entry; the room switch
    /// rule): S→C 0x07 and the add messages for
    /// every room of the room's adjacency array, client +0x1B4 := `room`.
    /// Default: nothing.
    fn client_room_switch(&mut self, player: Self::Unit, room: Self::Room) {
        let _ = (player, room);
    }
}

/// The unit flags 2 (+0xC8) bits sets.
pub mod flags2 {
    /// `alt` = 0.
    pub const PLACED: u32 = 0x10000;
    /// `alt` ≠ 0.
    pub const PLACED_ALT: u32 = 0x800;
}

/// Timer event a player placement schedules (rule 6, the unit
/// timers), with callback, this many frames ahead.
pub const PLACE_TIMER_EVENT: u8 = 14;
/// Frames ahead of the type-14 timer (Constants table).
pub const PLACE_TIMER_DELAY: u32 = 50;

/// S→C messages of placement and warps. Unit type and
/// GUID are the host's to fill in from the unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceMessage<U> {
    /// 0x07 MapReveal: tile x, tile y of a room, its level.
    MapReveal { x: u16, y: u16, level: u8 },
    /// 0x15 ReassignPlayer: unit, x, y, flag.
    ReassignPlayer { unit: U, x: u16, y: u16, flag: u8 },
    /// 0x0D PlayerStop: unit, a, x, y, b, life percent.
    PlayerStop {
        unit: U,
        a: u8,
        x: u16,
        y: u16,
        b: u8,
        life_pct: u8,
    },
    /// 0x7E (5 bytes), the last message of game entry:
    /// only the id is written
    /// (the other bytes stay zero).
    GameEntryDone,
}

/// Units, messages, timers and walk calls of placement and warps outside the path core.
#[allow(unused_variables)]
pub trait PlaceHost<U> {
    /// Unit type 0.
    fn is_player(&self, unit: U) -> bool;
    /// Queue for update.
    fn queue_update(&mut self, unit: U) {}
    /// Unit flags 2 (+0xC8) |= `bits` ([`flags2`]).
    fn or_flags2(&mut self, unit: U, bits: u32) {}
    /// Room-change messages `(game, unit, 0)` (the walk's
    /// room-change rule).
    fn room_change_messages(&mut self, unit: U) {}
    /// Sends a message to the client of `player`: every
    /// message of placement and warps goes to the moving player's own client
    /// only.
    fn send(&mut self, player: U, msg: PlaceMessage<U>) {}
    /// Player data +0x148, +0x14C := x, y.
    fn set_player_point(&mut self, player: U, x: i32, y: i32) {}
    /// Position history write: entry := (x, y)
    /// unconditionally ([`super::history::PositionHistory::place_write`]).
    fn history_write(&mut self, player: U, x: i32, y: i32) {}
    /// Schedules timer `event` for `unit` at frame + `delay` (callback
    /// For event 14).
    fn schedule_event(&mut self, unit: U, event: u8, delay: u32) {}
    /// Pets follow the player (pet / mercenary rules, open
    /// question 7).
    fn pets_follow(&mut self, player: U) {}
    /// Player mode request, point form: walk (mode 2) to (x, y).
    fn request_walk(&mut self, player: U, x: i32, y: i32) {}
    /// Life percent.
    fn life_percent(&self, unit: U) -> u8 {
        0
    }
}

/// A room's 0x07 MapReveal fields: tile x, tile y, level id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoomReveal {
    pub tile_x: i32,
    pub tile_y: i32,
    pub level: u32,
}

/// The destination of a warp tile and
/// the facts reads from it and its lvlwarp record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WarpDestination<R> {
    /// The destination tile's room.
    pub room: R,
    /// The destination tile's position.
    pub point: Point,
    /// lvlwarp `ExitWalkX`, `ExitWalkY` (+0x14, +0x18).
    pub exit_walk_x: i32,
    pub exit_walk_y: i32,
    /// Level of the source tile and of the destination.
    pub source_level: u32,
    pub level: u32,
}

/// DRLG answers of placement and warps.
#[allow(unused_variables)]
pub trait LevelView<R> {
    /// An act.
    type Act: Copy + Debug;
    /// Spawn room and position of a level (the DRLG spawn rule;
    /// may draw `roll` on the level seed): the room and the position
    /// in tiles. `None`: no spawn room.
    fn spawn_room(&mut self, act: Self::Act, level: u32, tile_index: u32) -> Option<(R, i32, i32)>;
    /// The act's start level (act +0x08).
    fn act_start_level(&self, act: Self::Act) -> u32;
    /// A room's 0x07 fields (tile x, tile y, level id).
    fn room_reveal(&self, room: R) -> RoomReveal;
    /// In the
    /// source DRLG room's warp-link list the first link whose lvlwarp
    /// `Id` = class gives the destination DRLG room D; D's link back to
    /// the source gives the record (`ExitWalkX/Y` are the destination
    /// side's); the destination tile is the first tile unit of class =
    /// that record's `Id` in D's active room. `None`: nothing.
    fn warp_destination(&self, tile_room: R, tile_class: u32) -> Option<WarpDestination<R>> {
        None
    }
    /// Quest warp gate
    /// (warp rule 3; its checks belong to the quests). Non-zero
    /// blocks.
    fn quest_gate(&self, source_level: u32, level: u32) -> u32 {
        0
    }
}

/// lvlwarp fields reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LvlWarp {
    /// `Id`: the warp tile's class.
    pub id: u32,
    pub offset_x: i32,
    pub offset_y: i32,
}

/// DRLG side of the warp tile preset.
pub trait WarpTileView {
    /// A DRLG room.
    type DrlgRoom: Copy + Debug;
    /// The room's tile rect (tile x, y, width, height).
    fn tile_rect(&self, room: Self::DrlgRoom) -> TileRect;
    /// lvlwarp record of warp slot `slot` of the room's level for
    /// direction letter `letter` (the level warp rule).
    /// `None` is fatal there.
    fn lvlwarp(&self, room: Self::DrlgRoom, slot: u32, letter: u8) -> Option<LvlWarp>;
    /// Prepends a preset unit to the room's list: unit
    /// type, class, mode, position in sub-tiles relative to the room.
    fn add_preset_unit(
        &mut self,
        room: Self::DrlgRoom,
        ty: u8,
        class: u32,
        mode: u32,
        x: i32,
        y: i32,
    );
}

/// Fatal asserts of placement and warps, and malformed walk-back
/// fields; D2RUST returns them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlaceError {
    /// Unit without a path.
    #[error("placement of a unit without a path")]
    NoPath,
    /// Act null.
    #[error("level spawn point without an act")]
    NoAct,
    /// Game entry: no spawn room (fatal assert).
    #[error("game entry without a spawn room")]
    NoSpawnRoom,
    /// No free point around the spawn (edge case 6).
    #[error("no free point around the level spawn")]
    SpawnNotFree,
    /// No lvlwarp record for the slot and letter.
    #[error("no lvlwarp record for warp slot {slot}")]
    NoLvlWarp { slot: u32 },
    /// A walk-back read outside the 256 × 256 field (unreachable
    /// with the shipped field file and the 50-ring searches; it would read
    /// past the buffer).
    #[error("walk-back field read outside the field at ({x}, {y})")]
    FieldOutOfRange { x: i32, y: i32 },
    /// A field byte above 8 (not in the shipped field file).
    #[error("walk-back field direction {0} > 8")]
    FieldDirection(u8),
    /// A walk that does not reach the centre (not in the shipped
    /// field file; it would loop forever).
    #[error("walk-back field walk does not reach the centre")]
    FieldNoEnd,
}

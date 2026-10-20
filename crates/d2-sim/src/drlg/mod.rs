//! The dungeon random level generator (DRLG) of one act: levels, level
//! seeds, vis/warp records, DRLG rooms, rooms-near order, statuses and
//! activation, tile grids and collision.
//!
//! What a level *contains* (which rooms, their rects, flags and source
//! grids) is decided by the level-type generators (preset, maze, outdoor). They plug in
//! through [`LevelTypes`]. Active rooms reach the unit lists of
//! [`crate::units`] through [`ActRooms`]. DT1 tiles arrive parsed through
//! [`TileSource`]. Table data comes from `d2_data::tables` records via
//! [`DrlgData`].
//!
//! Module map:
//! - [`data`]: the table view ([`DrlgData`]) built from typed records;
//! - [`seams`]: the traits other systems provide;
//! - [`level`]: act DRLG creation, levels, seeds, warps, coordinates,
//!   level lifecycle, spawn room;
//! - [`room`]: DRLG rooms, rooms-near arrays, statuses, propagation, build;
//! - [`active`]: active rooms, adjacency arrays, clients, deactivation;
//! - [`tiles`]: tile library, tile choice, grid fill, linking, animation;
//! - [`collision`]: collision grids;
//! - [`logic`]: logical rooms (coordinate lists) and their lookups.

/// Active rooms: creation, adjacency, clients and removal.
pub mod active;
/// Collision grids of active rooms.
pub mod collision;
/// The table view the DRLG reads.
pub mod data;
/// The act DRLG: levels, seeds, warps, lifecycle and spawn room.
pub mod level;
/// Logical rooms: coordinate lists and their lookups.
pub mod logic;
/// The maze level-type generator.
pub mod maze;
/// The outdoor level-type generator.
pub mod outdoor;
/// The preset level-type generator.
pub mod preset;
/// DRLG rooms: rooms-near arrays, statuses, propagation and build.
pub mod room;
/// The traits other systems provide to the DRLG.
pub mod seams;
/// Tile library, tile choice, grid fill, linking and animation.
pub mod tiles;

#[cfg(test)]
mod tests;

use thiserror::Error;

pub use active::ActiveRoom;
pub use collision::CollisionGrid;
pub use data::{DoorTables, DrlgData, LevelDef, WallClass, WallRemap, WarpDef};
pub use level::{BuildCursor, Drlg, Dungeon, Level, SpawnTile, WarpRecord};
pub use logic::{CoordRec, LogicGrids, LogicInfo};
pub use room::{view_gap_for_frame, view_gap_for_width, DrlgRoom, RoomKind, WarpLink, NEAR_GAP};
pub use seams::{
    ActRooms, ClientPreset, LevelTypes, NoLevelTypes, PresetUnit, Services, TileInfo, TileSource,
};
pub use tiles::{CellGrid, GridPass, RoomGrids, RoomTiles, TileRecord, TileRef};

/// A DRLG room (RoomEx) of one [`Drlg`], by slot. Slots are never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DrlgRoomId(pub u32);

/// A level of one [`Drlg`], by slot (allocation order). Not a level id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LevelIdx(pub u32);

/// A rectangle in tile coordinates (signed).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TileRect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub w: i32,
    /// Height.
    pub h: i32,
}

impl TileRect {
    /// A rect from its position and size.
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    /// Half-open containment: the right and bottom edges are outside.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.x <= x && x < self.x + self.w && self.y <= y && y < self.y + self.h
    }

    /// Closed containment (border included).
    pub fn contains_closed(&self, x: i32, y: i32) -> bool {
        self.x <= x && x <= self.x + self.w && self.y <= y && y <= self.y + self.h
    }
}

/// Sub-tiles per tile.
pub const SUBTILES: i32 = 5;

/// Town level ids, act 1..5.
pub const TOWN_LEVELS: [u32; 5] = [1, 40, 75, 103, 109];

/// First level id of each act, plus the end (1024).
pub const ACT_BOUNDARIES: [u32; 6] = [1, 40, 75, 103, 109, 1024];

/// Room flags (D2MOO names).
pub mod room_flags {
    /// Warp toward vis slot `i`: `WARP_0 << i`.
    pub const WARP_0: u32 = 0x10;
    /// All eight warp bits.
    pub const WARP_MASK: u32 = 0xFF0;
    /// The room holds a waypoint.
    pub const WAYPOINT: u32 = 0x1_0000;
    /// The room holds a small waypoint.
    pub const WAYPOINT_SMALL: u32 = 0x2_0000;
    /// Either waypoint flag.
    pub const ANY_WAYPOINT: u32 = 0x3_0000;
    /// The room is revealed on the automap.
    pub const AUTOMAP_REVEAL: u32 = 0x4_0000;
    /// No line-of-sight drawing (D2MOO `NO_LOS_DRAW`).
    pub const NO_LOS_DRAW: u32 = 0x8_0000;
    /// Tiles / active room built (`HAS_ROOM`).
    pub const HAS_ROOM: u32 = 0x10_0000;
    /// Tiles-kept marker; nothing sets it.
    pub const TILES_KEPT: u32 = 0x20_0000;
    /// A portal: blocks removal.
    pub const PORTAL: u32 = 0x40_0000;
    /// No population (next to a town).
    pub const NO_POPULATION: u32 = 0x80_0000;
    /// The room's tile library is loaded.
    pub const TILE_LIB_LOADED: u32 = 0x100_0000;
    /// The room's preset units have been added.
    pub const PRESET_UNITS_ADDED: u32 = 0x200_0000;
    /// The preset-unit list was handed to population; nothing clears it.
    pub const PRESETS_HANDED_OUT: u32 = 0x400_0000;
    /// Room has animated tiles.
    pub const ANIMATED: u32 = 0x800_0000;
}

/// Act number (0..4) of a level id: the first `a` with `id < ACT_BOUNDARIES[a+1]`; ids
/// ≥ 1024 give 0.
pub fn act_of_level(id: u32) -> u8 {
    for a in 0..5u8 {
        if id < ACT_BOUNDARIES[a as usize + 1] {
            return a;
        }
    }
    0
}

/// Whether a level id is an act's town.
pub fn is_town(id: u32) -> bool {
    TOWN_LEVELS.contains(&id)
}

/// Invalid generation states (returned instead of aborting) and D2RUST API misuse.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DrlgError {
    #[error("level id {0} has no leveldefs row")]
    UnknownLevel(u32),
    #[error("unknown DRLG room {0:?}")]
    UnknownRoom(DrlgRoomId),
    #[error("unknown level slot {0:?}")]
    UnknownLevelIdx(LevelIdx),
    #[error("vis/warp record with level id 0")]
    WarpRecordLevelZero,
    #[error("no lvlwarp row with Id {0}")]
    NoLvlWarp(i32),
    #[error("room tile library is full (fatal 0x2A)")]
    LibraryFull,
    #[error("DT1 file not supplied: {0:?}")]
    MissingDt1(String),
    #[error("no tile for a key and no (10, 0, 0) fallback (fatal 0x73)")]
    NoTile,
    #[error("missing animation frame {0} (fatal 0xB6 / 0xCB)")]
    MissingFrame(u32),
    #[error("linked cell type {0} beyond the wall-remap index table (wall-remap.md §2)")]
    WallRemapType(u32),
    #[error("level has no waypoint room for spawn-tile index 13")]
    NoWaypointRoom,
    #[error("level has no rooms to spawn in")]
    NoSpawnRoom,
    #[error("removal test on a client-copy DRLG")]
    ClientCopyRemoval,
    #[error("not an active room of this DRLG")]
    NotActive,
    /// A level-type generator (preset, maze, outdoor) failed on this level id; the
    /// provider keeps its own error.
    #[error("level type generator failed on level {0} (error kept by the provider)")]
    LevelType(u32),
    /// A corner record R first in its link chain (nothing after it) was met with a merge
    /// code other than 3, which needs a following record.
    #[error("linked corner record has no record after it in its chain")]
    LinkedCornerNoNext,
}

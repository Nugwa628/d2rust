//! Preset (DS1) maps, rooms and levels: lvlprest file choice, preset map
//! allocation, DS1 loading and unit conversion, the area build (scan,
//! pops, waypoints, rooms), the unit filter, the first activation of a
//! preset room, room grids and unit transfer, door preset units.
//!
//! Inputs are plain data: [`PresetData`] (lvlprest, monpreset, counts,
//! the embedded `preset-tables.tsv`), DS1 files parsed outside the sim
//! through [`Ds1Source`], and the act's [`Drlg`]. Randomness only through
//! the level and room seeds, in a fixed draw order.
//!
//! Public API (for the DrlgType 2 level type and for the maze and outdoor
//! generators, which place preset maps through their own seams):
//! - DrlgType 2: [`Presets::init_level`], [`Presets::generate`]
//!   [`Presets::reset_level`];
//! - any level type: [`Presets::alloc_map`], [`Presets::build_area`]
//!   [`Presets::map_mut`] (picked file, link grid);
//! - room build: [`Presets::add_preset_units`], [`Presets::room_grids`]
//!   [`Presets::door_unit`], [`Presets::room_units`],
//!   [`Presets::preset_units`], [`Presets::tombstones`].
//!
//! Module map: [`data`] (tables), [`ds1`] (DS1 record, conversion,
//! cache), `map`, `room`.

pub mod data;
pub mod ds1;
mod map;
mod room;

#[cfg(test)]
mod gaps_numbered_tests;
#[cfg(test)]
mod gaps_tests;
#[cfg(test)]
mod mutant_tests;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use thiserror::Error;

use super::data::DrlgData;
use super::{DrlgError, DrlgRoomId, LevelIdx, TileRect};

pub use data::{DoorRow, MonPresetRow, PresetData, PresetDef, PresetTables};
pub use ds1::{Ds1Cache, Ds1File, Ds1Id, Ds1Input, Ds1ObjectInput, Ds1PathInput, Ds1Source};
pub use room::DoorOutcome;

/// Preset errors: fatal layout errors and bad inputs.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PresetError {
    #[error("level {0} labeled as preset but no preset claims it (fatal, Preset.cpp 0xAFB)")]
    NoPresetForLevel(u32),
    #[error("no lvlprest row {0}")]
    UnknownDef(u32),
    #[error("level {0:?} has no preset info (init_level not run)")]
    NoPresetInfo(LevelIdx),
    #[error("unknown preset map {0:?}")]
    UnknownMap(MapId),
    #[error("room {0:?} is not a preset room of this DRLG")]
    NotPresetRoom(DrlgRoomId),
    #[error("picked file {0} has no lvlprest File column")]
    BadPickedFile(i32),
    #[error("DS1 file not supplied: {0:?}")]
    MissingDs1(String),
    #[error("bad DS1 input: {0}")]
    BadDs1(String),
    #[error("DS1 size {ds1:?} differs from the map size {map:?} (fatal, lines 0x8B9 / 0x8BA)")]
    SizeMismatch { ds1: (u32, u32), map: (i32, i32) },
    #[error("DS1 cell read outside the file's layer")]
    LayerOutOfRange,
    #[error("map has no DS1 loaded")]
    Ds1NotLoaded,
    #[error("DS1 act {0} is negative (out-of-table read)")]
    NegativeAct(i32),
    #[error("DS1 item id {0} reads beyond the one-entry item code table")]
    ItemCodeBeyondTable(u32),
    #[error("door table unit type {0} (only 1 and 2 are specified)")]
    UnsupportedDoorType(u32),
    #[error("bad table data: {0}")]
    BadData(String),
    #[error("preset-tables.tsv: {0}")]
    Tsv(String),
    #[error(transparent)]
    Drlg(#[from] DrlgError),
}

/// One path point (action, x, y).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathPoint {
    pub action: u32,
    pub x: i32,
    pub y: i32,
}

/// A preset unit (0x20 bytes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetUnit {
    /// 1 monster, 2 object, 4 item (DS1 type; others as stored).
    pub unit_type: u32,
    pub class: i32,
    pub mode: u32,
    /// Sub-tiles: DS1-relative in a file, level-absolute on a map,
    /// room-relative in a room.
    pub x: i32,
    pub y: i32,
    /// Bit 0: spawned / fixed.
    pub flags: u32,
    /// Points stay level-absolute after the room transfer.
    pub path: Option<Vec<PathPoint>>,
}

impl PresetUnit {
    /// The DRLG seam's view.
    pub fn to_seam(&self) -> super::PresetUnit {
        super::PresetUnit {
            unit_type: self.unit_type,
            class: self.class as u32,
            x: self.x,
            y: self.y,
        }
    }
}

/// A preset map of one [`Presets`], by slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MapId(pub u32);

/// A pop entry: group, sub index, timer, rectangle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PopEntry {
    /// Style while scanning; `style / 4 − 1` after the finish.
    pub group: u32,
    pub sub: u32,
    /// Fade timer (presentation only; never set by the sim).
    pub timer: u32,
    /// Level tiles after the finish.
    pub rect: TileRect,
    corner1: (i32, i32),
    corner2: (i32, i32),
}

/// A preset map (0x58 bytes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetMap {
    pub level: LevelIdx,
    /// lvlprest index (`Def`).
    pub def: u32,
    pub picked_file: i32,
    /// Tiles.
    pub rect: TileRect,
    /// Per-8×8-cell link values (+0x24, read only when +0x20 is set),
    /// indexed like the area cell grid (row-major, `w/8 + 1` per
    /// row); `None` = no link grid (every room's link 0). Nothing
    /// sets it, so production maps keep `None`.
    pub link_grid: Option<Vec<u32>>,
    /// Preset units, head first.
    pub units: Vec<PresetUnit>,
    pub hardcoded_pending: bool,
    pub ds1: Option<Ds1Id>,
    pub pops: Vec<PopEntry>,
}

/// Level preset info (level +0x14).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresetInfo {
    pub map: Option<MapId>,
    /// Picked file index, −1 = none yet.
    pub direction: i32,
}

/// Preset room data (0xF8 bytes) plus the room's preset-unit list
/// (room +0x5C).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetRoom {
    pub def: u32,
    pub map: MapId,
    /// Preset room flag 1 (single room).
    pub single: bool,
    /// Link value from the map's link grid.
    pub link: u32,
    /// Room preset units, head first (room-relative sub-tiles).
    pub units: Vec<PresetUnit>,
    /// Level 17 tombstones, set once at the first grid build.
    pub tombstones: Option<Vec<(i32, i32)>>,
}

/// Preset state of one act DRLG (`Drlg`'s levels and rooms by slot).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Presets {
    info: BTreeMap<LevelIdx, PresetInfo>,
    /// Map list per level (+0x1B0), head first.
    level_maps: BTreeMap<LevelIdx, Vec<MapId>>,
    maps: Vec<Option<PresetMap>>,
    rooms: BTreeMap<DrlgRoomId, PresetRoom>,
}

/// What preset code reads besides the DRLG: the DRLG table view, the
/// preset tables, the DS1 provider and the process-wide DS1 cache.
pub struct PresetCtx<'a> {
    pub drlg: &'a DrlgData,
    pub data: &'a PresetData,
    pub source: &'a dyn Ds1Source,
    pub cache: &'a mut Ds1Cache,
}

impl Presets {
    pub fn info(&self, level: LevelIdx) -> Option<&PresetInfo> {
        self.info.get(&level)
    }

    /// The act layout's overwrite of the direction (set by the
    /// outdoor act layouts).
    pub fn set_direction(&mut self, level: LevelIdx, direction: i32) -> Result<(), PresetError> {
        self.info
            .get_mut(&level)
            .ok_or(PresetError::NoPresetInfo(level))?
            .direction = direction;
        Ok(())
    }

    /// The level's maps, head first.
    pub fn level_maps(&self, level: LevelIdx) -> &[MapId] {
        self.level_maps.get(&level).map_or(&[], Vec::as_slice)
    }

    pub fn map(&self, id: MapId) -> Result<&PresetMap, PresetError> {
        self.maps
            .get(id.0 as usize)
            .and_then(Option::as_ref)
            .ok_or(PresetError::UnknownMap(id))
    }

    /// Mutable map: outdoor code sets `picked_file` here.
    pub fn map_mut(&mut self, id: MapId) -> Result<&mut PresetMap, PresetError> {
        self.maps
            .get_mut(id.0 as usize)
            .and_then(Option::as_mut)
            .ok_or(PresetError::UnknownMap(id))
    }

    pub fn room(&self, room: DrlgRoomId) -> Result<&PresetRoom, PresetError> {
        self.rooms
            .get(&room)
            .ok_or(PresetError::NotPresetRoom(room))
    }

    /// The room's preset units, head first (room +0x5C).
    /// The source side of the room's unit paths: the path of the room's preset unit
    /// `index`, moved out (the preset keeps none).
    pub fn take_unit_path(&mut self, room: DrlgRoomId, index: usize) -> Option<Vec<PathPoint>> {
        self.rooms.get_mut(&room)?.units.get_mut(index)?.path.take()
    }

    /// The path of the room's preset unit `index`, kept (a copy, as
    /// Makes for the quest map-AI stores).
    pub fn unit_path(&self, room: DrlgRoomId, index: usize) -> Option<Vec<PathPoint>> {
        self.rooms.get(&room)?.units.get(index)?.path.clone()
    }

    pub fn room_units(&self, room: DrlgRoomId) -> &[PresetUnit] {
        self.rooms.get(&room).map_or(&[], |r| r.units.as_slice())
    }

    /// Prepends a unit to a preset room's list (the warp
    /// tile preset). `false`: not a
    /// preset room of this act.
    pub fn prepend_room_unit(&mut self, room: DrlgRoomId, unit: PresetUnit) -> bool {
        match self.rooms.get_mut(&room) {
            Some(r) => {
                r.units.insert(0, unit);
                true
            }
            None => false,
        }
    }

    /// [`super::LevelTypes::preset_units`]: the seam view of the room's
    /// preset units, in list order.
    pub fn preset_units(&self, room: DrlgRoomId) -> Vec<super::PresetUnit> {
        self.room_units(room)
            .iter()
            .map(PresetUnit::to_seam)
            .collect()
    }

    /// The room's units with flag bit 0 set, in list order (the client
    /// presets).
    pub fn client_presets(&self, room: DrlgRoomId) -> Vec<super::ClientPreset> {
        self.room_units(room)
            .iter()
            .filter(|u| u.flags & 1 != 0)
            .map(|u| super::ClientPreset {
                unit_type: u.unit_type,
                class: u.class,
                mode: u.mode,
                x: u.x,
                y: u.y,
            })
            .collect()
    }

    /// A type-2 room's tombstones, else none.
    pub fn tombstones(&self, room: DrlgRoomId) -> Option<&[(i32, i32)]> {
        self.rooms.get(&room)?.tombstones.as_deref()
    }
}

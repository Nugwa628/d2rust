//! The traits the DRLG needs from other parts of the simulation, and the bundle of
//! services a room build uses.
//!
//! | Seam | Expected provider |
//! |---|---|
//! | [`LevelTypes`] | the preset, maze and outdoor level-type generators |
//! | [`ActRooms`] | `crate::units::UnitLists` (implemented in [`super::active`]) |
//! | [`TileSource`] | the caller holding parsed DT1s (`d2_formats::dt1::Dt1`; d2-server / world) |

use crate::units::RoomId;

use super::data::DrlgData;
use super::level::Drlg;
use super::tiles::RoomGrids;
use super::{DrlgError, DrlgRoomId, LevelIdx};

/// The level-type generators: act placement, level init, room generation, resets, preset
/// units and room grids. Every method receives the DRLG; a method that allocates levels
/// calls [`Drlg::get_or_alloc_level`] with `self` as the `types` argument.
///
/// Draws: each method draws only from the seeds it reaches through `drlg` (level seeds,
/// room seeds, the DRLG seed for the act placer).
#[allow(unused_variables)]
pub trait LevelTypes {
    /// Creates and places the act's levels at DRLG creation. Draws from the DRLG seed.
    fn create_act_levels(&mut self, drlg: &mut Drlg, data: &DrlgData) -> Result<(), DrlgError> {
        Ok(())
    }

    /// Level-data init at allocation (the preset generator also picks its preset file here).
    /// The level is not yet in the list. For maze and preset levels the DRLG sets size and
    /// position after this call.
    fn init_level(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        level: LevelIdx,
    ) -> Result<(), DrlgError> {
        Ok(())
    }

    /// Room generation. Creates rooms with [`Drlg::alloc_room`] and links them with
    /// [`Drlg::link_room`]; the level seed was just re-initialized.
    fn generate(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        level: LevelIdx,
    ) -> Result<(), DrlgError> {
        Ok(())
    }

    /// Type data reset when a level's rooms are freed; also clears the preset map and
    /// build list.
    fn reset_level(&mut self, drlg: &mut Drlg, level: LevelIdx) {}

    /// Status-3 handler: adds a preset room's units. The DRLG sets the flag after.
    fn add_preset_units(&mut self, drlg: &mut Drlg, room: DrlgRoomId) -> Result<(), DrlgError> {
        Ok(())
    }

    /// The preset units of a room (the spawn-room choice reads the type-2 ones).
    fn preset_units(&self, drlg: &Drlg, room: DrlgRoomId) -> Vec<PresetUnit> {
        Vec::new()
    }

    /// The path of the room's preset unit `index`, as (action, x, y) points in level
    /// sub-tiles, if it has one.
    fn unit_path(
        &self,
        _drlg: &Drlg,
        _room: DrlgRoomId,
        _index: usize,
    ) -> Option<Vec<(u32, i32, i32)>> {
        None
    }

    /// The client presets of a room: its preset units whose flag word has bit 0 set, in
    /// list order, room-relative.
    fn client_presets(&self, drlg: &Drlg, room: DrlgRoomId) -> Vec<ClientPreset> {
        Vec::new()
    }

    /// Whether generating this level on a client copy streams its rooms for the automap:
    /// a preset level whose lvlprest `AutoMap` ≠ 0, other than the town-automap levels 40,
    /// 103, 109 (levels 1 and 75 are not excluded).
    fn automap_streams(&self, drlg: &Drlg, level: LevelIdx) -> bool {
        false
    }

    /// Grid init: the room's packed source grids. Runs right after the room seed reset;
    /// outdoor grids draw from the room seed.
    fn room_grids(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        room: DrlgRoomId,
    ) -> Result<RoomGrids, DrlgError> {
        Ok(RoomGrids::default())
    }

    /// Frees the type data of a room's tiles.
    fn free_room_tiles(&mut self, drlg: &mut Drlg, room: DrlgRoomId) {}

    /// A hidden door cell's preset unit. The door tables have no built-in values; the
    /// `roll(3)` for objects 91–92 is drawn from the room seed
    /// (`drlg.room_mut(room).seed`) by the provider. `orientation` is the cell's tile type
    /// (9 = right door). Returns whether the door record (when there is one) gets flag
    /// 0x20: a unit was added, or `roll(3)` gave 0.
    #[allow(clippy::too_many_arguments)]
    fn door_unit(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        room: DrlgRoomId,
        wx: i32,
        wy: i32,
        cell: u32,
        orientation: u32,
    ) -> bool {
        false
    }

    /// An exit cell's warp tile preset (for a hidden cell, and for the wall warp tiles of
    /// sub 0 or 4): world tile (`wx`, `wy`), packed `cell`, cell type `orientation` (10 or
    /// 11). `Ok(true)`: a type-5 preset unit was prepended to the room's list; `Ok(false)`:
    /// none (far edge, or a provider with no warp tiles); `Err`: no lvlwarp record.
    #[allow(clippy::too_many_arguments)]
    fn warp_unit(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        room: DrlgRoomId,
        wx: i32,
        wy: i32,
        cell: u32,
        orientation: u32,
    ) -> Result<bool, DrlgError> {
        Ok(false)
    }
}

/// A [`LevelTypes`] that generates nothing (levels stay empty).
#[derive(Clone, Copy, Debug, Default)]
pub struct NoLevelTypes;

impl LevelTypes for NoLevelTypes {}

/// A preset unit of a room: unit type (2 = object), class, position in sub-tiles relative
/// to the room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresetUnit {
    /// Unit type (2 = object).
    pub unit_type: u32,
    /// Class (row of the unit's table).
    pub class: u32,
    /// Sub-tile x relative to the room.
    pub x: i32,
    pub y: i32,
}

/// A client preset: the creator's type, class, mode and room-relative sub-tile position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientPreset {
    /// Unit type to create.
    pub unit_type: u32,
    /// Class to create.
    pub class: i32,
    /// Mode to create it in.
    pub mode: u32,
    /// Sub-tile x relative to the room.
    pub x: i32,
    pub y: i32,
}

/// The act room list the active rooms live in. Provider: `UnitLists`
/// ([`super::active`]).
pub trait ActRooms {
    /// Allocates an active room record and prepends it to `act`'s room list, setting the
    /// act's pending-room flag. `flags` are active-room flags (bit 0 populated, bit 2 no
    /// update).
    fn create_active_room(&mut self, act: u8, flags: u32) -> RoomId;

    /// Writes a room's adjacency array, in order.
    fn set_adjacent(&mut self, room: RoomId, adjacent: &[RoomId]);

    /// Unlinks the room from the act list (if still linked) and frees the record. Returns
    /// the active-room flags as they were, for `other flags := flags & 1`.
    fn remove_active_room(&mut self, room: RoomId) -> u32;
}

/// One DT1 tile header as the DRLG reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileInfo {
    /// Orientation.
    pub orientation: u32,
    /// Main index.
    pub main: u32,
    /// Sub index.
    pub sub: u32,
    /// Rarity / frame number.
    pub rarity: u32,
    /// Material flags.
    pub material: u16,
    /// Sub-tile flags, bottom row first.
    pub subtile_flags: [u8; 25],
    /// Roof height. Read only by the client draw.
    pub roof_height: u16,
    /// Height. Read only by the client draw.
    pub height: i32,
    /// Light direction. Read only by the client draw: it picks the wall light points.
    pub light_direction: u32,
}

/// Parsed DT1 files by path (no I/O in the sim). Paths are the `lvltypes` `File` strings
/// verbatim and the three fixed library paths ([`super::tiles::FIXED_LIBRARY`]); the
/// provider resolves them (case-insensitive, relative to `DATA\GLOBAL\Tiles\`).
pub trait TileSource {
    /// The tiles of a DT1 file in file order, or `None` if unknown.
    fn dt1(&self, path: &[u8]) -> Option<&[TileInfo]>;
}

/// The services a room build needs.
pub struct Services<'a> {
    /// The table view.
    pub data: &'a DrlgData,
    /// The parsed DT1 tiles.
    pub tiles: &'a dyn TileSource,
    /// The level-type generators.
    pub types: &'a mut dyn LevelTypes,
    /// The act room list.
    pub rooms: &'a mut dyn ActRooms,
}

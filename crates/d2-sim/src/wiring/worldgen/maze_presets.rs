//! Maze → preset: [`MazeToPreset`] is the [`MazePresets`] of the maze
//! generator, on the act's real [`Presets`]. A cell's DS1 map is
//! allocated on the level seed (the default `roll(Files)`), its file
//! overwritten when the maze chose one, then built into preset rooms
//! (single-room mode for cells of at most 12 × 12).

use crate::drlg::maze::{MapId, MazeLink, MazePresets};
use crate::drlg::outdoor::Outdoor;
use crate::drlg::preset::{self, Ds1Cache, Ds1Source, PresetCtx, PresetData, Presets};
use crate::drlg::{Drlg, DrlgData, DrlgError, DrlgRoomId, LevelIdx, TileRect};

use super::levels::{preset_err, AllocView};
use super::WorldgenError;

/// The preset side of one maze generation (see the module doc).
pub struct MazeToPreset<'a> {
    pub presets: &'a mut Presets,
    /// Outdoor inits of levels the maze allocates.
    pub outdoor: &'a mut Outdoor,
    pub pd: &'a PresetData,
    pub src: &'a dyn Ds1Source,
    pub cache: &'a mut Ds1Cache,
    pub errors: &'a mut Vec<WorldgenError>,
}

impl MazePresets for MazeToPreset<'_> {
    /// Get-or-allocate a level with the type inits of the other generators
    /// (the maze one is busy).
    fn level(&mut self, drlg: &mut Drlg, data: &DrlgData, id: u32) -> Result<LevelIdx, DrlgError> {
        let mut alloc = AllocView {
            maze: None,
            presets: &mut *self.presets,
            outdoor: Some(&mut *self.outdoor),
            pd: self.pd,
            src: self.src,
            cache: &mut *self.cache,
            errors: &mut *self.errors,
        };
        drlg.get_or_alloc_level(data, &mut alloc, id)
    }

    /// The preset info's direction (−1 reads as `u32::MAX`, which the
    /// maze's finder table rejects).
    fn preset_direction(&self, drlg: &Drlg, level: LevelIdx) -> Result<u32, DrlgError> {
        self.presets
            .info(level)
            .map(|i| i.direction as u32)
            .ok_or(DrlgError::LevelType(drlg.level(level).id))
    }

    /// Allocates the cell's DS1 map: draws `roll(Files)` on the level seed.
    fn alloc_map(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        level: LevelIdx,
        def: u32,
        rect: TileRect,
    ) -> Result<MapId, DrlgError> {
        let id = drlg.level(level).id;
        let ctx = PresetCtx {
            drlg: data,
            data: self.pd,
            source: self.src,
            cache: &mut *self.cache,
        };
        self.presets
            .alloc_map(drlg, &ctx, level, def, rect)
            .map(|m| MapId(m.0))
            .map_err(|e| preset_err(self.errors, id, e))
    }

    /// Sets the map's picked file.
    fn set_map_file(&mut self, map: MapId, file: i32) {
        match self.presets.map_mut(preset::MapId(map.0)) {
            Ok(m) => m.picked_file = file,
            Err(e) => self.errors.push(WorldgenError::Preset(e)),
        }
    }

    /// Builds the map into preset rooms with room flags 0, in single-room
    /// mode when `small`; returns the last room built, which the maze links
    /// to the cell's links.
    // TODO: DRLG rooms have no orth link list (`drlg::room`); the built
    // rooms' links live in the maze generation state only and are not kept
    // past generation.
    fn build_map(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        level: LevelIdx,
        map: MapId,
        small: bool,
        _links: &[MazeLink],
    ) -> Result<Option<DrlgRoomId>, DrlgError> {
        let id = drlg.level(level).id;
        let mut ctx = PresetCtx {
            drlg: data,
            data: self.pd,
            source: self.src,
            cache: &mut *self.cache,
        };
        self.presets
            .build_area(drlg, &mut ctx, level, preset::MapId(map.0), 0, small)
            .map_err(|e| preset_err(self.errors, id, e))
    }
}

//! Outdoor levels (leveldefs `DrlgType` 3): the act-wide placer that positions an act's
//! outdoor chain at DRLG creation, and level generation: coarse 8×8-tile cell grids, the
//! vertex polygon, borders, cliffs, rivers, paths, fixed presets, lvlsub substitution
//! ([`tilesub`]) and one DRLG room per cell.
//!
//! State lives in [`Outdoor`] (one per act DRLG, beside the [`Drlg`]): an outdoor record
//! per level and an outdoor room record per room. Table data comes from
//! `d2_data::tables` through [`OutdoorData`]. Two narrow seams reach code owned elsewhere:
//!
//! | Seam | What | Provider |
//! |---|---|---|
//! | [`OutdoorPresets`] | preset map and build area of a preset cell | `drlg::preset` |
//! | [`SubFiles`] | parsed lvlsub DS1 files | the caller holding parsed DS1s (d2-server / world) |
//!
//! [`OutdoorTypes`] adapts this module to [`LevelTypes`] for outdoor
//! levels and forwards every other level to another provider.
//!
//! Module map:
//! - [`grid`]: cell grids, stamping, fit tests, shuffles, placers;
//! - [`place`]: act-wide placement;
//! - [`vertex`]: vertex polygon, link flags, borders;
//! - [`wild`]: Act I;
//! - [`acts`]: Acts II and IV (`act3` and `act5` hold Acts III and V);
//! - [`rooms`]: cells to rooms, outdoor room grids;
//! - [`tilesub`]: lvlsub substitution.

/// Act III level build.
pub mod act3;
/// Act V level build.
pub mod act5;
/// Acts II and IV level builds.
pub mod acts;
/// Cell grids, stamping, fit tests and placers.
pub mod grid;
/// Act III jungle layout.
pub mod jungle;
/// Kurast and Travincal stamps.
pub mod kurast;
/// Path floor tiles.
pub mod path_floor;
/// Act-wide placement of outdoor levels.
pub mod place;
/// Cells to rooms and outdoor room grids.
pub mod rooms;
/// lvlsub tile substitution.
pub mod tilesub;
/// Vertex polygon, link flags and borders.
pub mod vertex;
/// Act I level build.
pub mod wild;

#[cfg(test)]
mod cold_plains_tests;
#[cfg(test)]
mod gaps_tests;
#[cfg(test)]
mod mutant_tests;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use d2_data::tables::{text, Leveldefs, Lvlprest, Lvlsub};
use thiserror::Error;

use super::data::DrlgData;
use super::level::Drlg;
use super::seams::{LevelTypes, PresetUnit};
use super::tiles::{CellGrid, RoomGrids, TileRef};
use super::{act_of_level, DrlgError, DrlgRoomId, LevelIdx};

pub use grid::Grid;
pub use tilesub::{SubHit, SubRow};

/// DRLG type of outdoor levels (leveldefs `DrlgType`).
pub const DRLG_OUTDOOR: u32 = 3;

/// Fatal errors of the outdoor code (states the game treats as fatal) and seam failures.
/// [`OutdoorTypes`] reports them through [`DrlgError`] (see there).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum OutdoorError {
    /// An error from the DRLG core.
    #[error(transparent)]
    Drlg(#[from] DrlgError),
    /// A preset id has no lvlprest row.
    #[error("lvlprest row {0} does not exist")]
    UnknownPreset(u32),
    /// A preset with no files cannot give out a file.
    #[error("build list of lvlprest {0} has Files = 0 (division by zero, outdoor.md edge case 3)")]
    NoFiles(u32),
    /// A neighbour entry has an unknown direction.
    #[error("neighbour entry with direction {0} (fatal, outdoor.md §4)")]
    UnknownDirection(i32),
    /// An Act II exit preset found no place.
    #[error("Act II exit preset {0} not placed (fatal, outdoor.md §8)")]
    ExitNotPlaced(u32),
    /// The caller did not supply a needed lvlsub DS1 file.
    #[error("lvlsub DS1 file not supplied: {0:?}")]
    MissingSubFile(Vec<u8>),
    /// An lvlsub DS1 file has no groups.
    #[error("lvlsub DS1 file without groups: {0:?} (fatal, outdoor-tilesub.md §1)")]
    SubFileNoGroups(Vec<u8>),
    /// No lvlsub rows exist for a type.
    #[error("no lvlsub rows of type {0} (fatal, outdoor-tilesub.md §1)")]
    NoSubRows(i32),
    /// The Act V style map lacks a row.
    #[error("Act V style map has no row for style {0}, value {1} (fatal)")]
    StyleMap(u32, u32),
    /// A sub-theme index is outside 0..4.
    #[error("sub theme {0} out of range 0..4")]
    SubTheme(i32),
    /// A siege strip piece would start left of the grid.
    #[error("siege strip piece {0} at x < 0 (fatal, outdoor.md §11)")]
    SiegeStrip(u32),
    /// The link driver backtracked past its first row.
    #[error("link driver backtracked past its first row")]
    DriverUnderflow,
    /// A level has no allocation.
    #[error("level {0} is not allocated")]
    LevelMissing(u32),
    /// A fatal game error with its error code.
    #[error("fatal error {0:#x} (outdoor-act3-act5.md)")]
    Fatal(u32),
    /// A state on which the game would crash (no error id): reported as fatal.
    #[error("invalid outdoor layout: {0}")]
    Crash(&'static str),
    /// The jungle id grid has fewer entries than the level has blocks (found, needed).
    #[error("jungle id array of {0} entries, SXb·SYb = {1} (outdoor-act3-act5.md §3)")]
    JungleIdsShort(usize, usize),
}

// ---- data ----------------------------------------------------------------

/// The leveldefs columns the outdoor code reads beyond [`DrlgData`] (`SubType`,
/// `SubTheme`, `SubWaypoint`, `SubShrine`; −1 = none).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubDefs {
    /// `SubType`: the lvlsub type of the level.
    pub sub_type: i32,
    pub sub_theme: i32,
    /// `SubWaypoint`: the sub-theme row for the waypoint.
    pub sub_waypoint: i32,
    /// `SubShrine`: the sub-theme row for shrines.
    pub sub_shrine: i32,
}

/// One lvlprest row (row index = `Def`): the columns the outdoor code reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PresetDef {
    /// `SizeX` in tiles.
    pub size_x: i32,
    /// `SizeY` in tiles.
    pub size_y: i32,
    /// `Files`: drives every build-list roll.
    pub files: i32,
}

/// The table view of the outdoor code: leveldefs sub columns, lvlprest sizes and file
/// counts, lvlsub rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutdoorData {
    /// Row = level id.
    pub levels: Vec<SubDefs>,
    /// Row = lvlprest index (`Def`).
    pub presets: Vec<PresetDef>,
    /// lvlsub rows in file order.
    pub subs: Vec<SubRow>,
}

impl OutdoorData {
    /// Builds the view from the raw leveldefs, lvlprest and lvlsub tables.
    pub fn from_tables(leveldefs: &[Leveldefs], lvlprest: &[Lvlprest], lvlsub: &[Lvlsub]) -> Self {
        let i = |v: u32| v as i32;
        Self {
            levels: leveldefs
                .iter()
                .map(|r| SubDefs {
                    sub_type: i(r.subtype),
                    sub_theme: i(r.subtheme),
                    sub_waypoint: i(r.subwaypoint),
                    sub_shrine: i(r.subshrine),
                })
                .collect(),
            presets: lvlprest
                .iter()
                .map(|r| PresetDef {
                    size_x: i(r.sizex),
                    size_y: i(r.sizey),
                    files: i(r.files),
                })
                .collect(),
            subs: lvlsub
                .iter()
                .map(|r| SubRow {
                    type_: i(r.type_),
                    file: text(&r.file[..]).to_vec(),
                    check_all: i(r.checkall),
                    bord_type: i(r.bordtype),
                    dt1_mask: r.dt1mask,
                    grid_size: i(r.gridsize),
                    prob: [r.prob0, r.prob1, r.prob2, r.prob3, r.prob4].map(i),
                    trials: [r.trials0, r.trials1, r.trials2, r.trials3, r.trials4].map(i),
                    max: [r.max0, r.max1, r.max2, r.max3, r.max4].map(i),
                })
                .collect(),
        }
    }

    /// The lvlprest row `p`, or `OutdoorError::UnknownPreset` when it does not exist.
    pub fn preset(&self, p: u32) -> Result<&PresetDef, OutdoorError> {
        self.presets
            .get(p as usize)
            .ok_or(OutdoorError::UnknownPreset(p))
    }

    /// Leveldefs sub columns of a level (all −1 for a missing row).
    pub fn sub_defs(&self, id: u32) -> SubDefs {
        self.levels.get(id as usize).copied().unwrap_or(SubDefs {
            sub_type: -1,
            sub_theme: -1,
            sub_waypoint: -1,
            sub_shrine: -1,
        })
    }
}

// ---- seams ----------------------------------------------------------------

/// The preset-room side of a preset cell. Provider: `drlg::preset`.
pub trait OutdoorPresets {
    /// Builds one preset cell of an outdoor level, in this order: allocate a preset map
    /// for lvlprest `def` at tile `(x, y)` with **the level seed** (it draws
    /// `roll(Files(def))` on `drlg.level_mut(level).seed`); set the map's file to
    /// `file`; build the area with room flags `room_flags`, which creates and links the
    /// rooms.
    #[allow(clippy::too_many_arguments)]
    fn build_preset_cell(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        level: LevelIdx,
        def: u32,
        x: i32,
        y: i32,
        file: u32,
        room_flags: u32,
    ) -> Result<(), DrlgError>;
}

/// One group of a substitution DS1: a box and its variant count (the DS1 group's
/// `unknown` field).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubGroup {
    /// Left edge of the box.
    pub x: i32,
    pub y: i32,
    /// Box width.
    pub w: i32,
    /// Box height.
    pub h: i32,
    /// Number of variants the group offers.
    pub variants: i32,
}

/// A loaded substitution DS1. Grids are (DS1 width + 1) × (height + 1) packed cells as
/// the DS1 decoder yields them. Wall layer `k ≥ 1` is read ORed with `k << 18`; the
/// provider supplies the raw values.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubFile {
    /// DS1 substitution method: 1 fixed, 2 random.
    pub method: u32,
    /// The substitution groups.
    pub groups: Vec<SubGroup>,
    /// Floor layer 0, if the file has floor layers.
    pub floor: Option<CellGrid>,
    /// Wall layers in order.
    pub walls: Vec<CellGrid>,
    /// Tile-type (orientation) grid per wall layer.
    pub tile_types: Vec<CellGrid>,
    /// Shadow layer, if the file has one.
    pub shadow: Option<CellGrid>,
    /// Preset units, positions in sub-tiles.
    pub units: Vec<PresetUnit>,
}

impl SubFile {
    /// Pattern floor value (0 without floor layers).
    pub fn floor_at(&self, x: i32, y: i32) -> u32 {
        self.floor.as_ref().map_or(0, |g| g.get(x, y))
    }

    /// Pattern wall value of layer `k` (0 without that layer), with
    /// `k << 18` for `k ≥ 1`.
    pub fn wall_at(&self, k: usize, x: i32, y: i32) -> u32 {
        self.walls
            .get(k)
            .map_or(0, |g| g.get(x, y) | ((k as u32) << 18))
    }

    /// Tile-type value of layer `k` (0 without that layer).
    pub fn tile_type_at(&self, k: usize, x: i32, y: i32) -> u32 {
        self.tile_types.get(k).map_or(0, |g| g.get(x, y))
    }

    /// Shadow value (0 without a shadow layer).
    pub fn shadow_at(&self, x: i32, y: i32) -> u32 {
        self.shadow.as_ref().map_or(0, |g| g.get(x, y))
    }
}

/// Parsed lvlsub DS1 files by lvlsub `File` string (no I/O in the sim).
/// Files stay loaded for the session.
pub trait SubFiles {
    /// The parsed file for an lvlsub `File` string, if supplied.
    fn sub_file(&self, file: &[u8]) -> Option<&SubFile>;
}

/// A [`SubFiles`] by name.
#[derive(Clone, Debug, Default)]
pub struct SubFileMap(pub BTreeMap<Vec<u8>, SubFile>);

impl SubFiles for SubFileMap {
    fn sub_file(&self, file: &[u8]) -> Option<&SubFile> {
        self.0.get(file)
    }
}

// ---- state ----------------------------------------------------------------

/// Vertex flag bit 0: the edge from this vertex is a level link.
pub const VERTEX_LINK: u32 = 0x1;
/// Vertex flag bit 1: link to a preset level.
pub const VERTEX_PRESET_LINK: u32 = 0x2;

/// A polygon vertex, in cell coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Vertex {
    /// Cell x.
    pub x: i32,
    pub y: i32,
    /// Direction of the edge leaving this vertex.
    pub direction: u8,
    /// `VERTEX_LINK` and `VERTEX_PRESET_LINK` bits.
    pub flags: u32,
}

impl Vertex {
    /// True when the edge from this vertex is a level link.
    pub fn is_link(&self) -> bool {
        self.flags & VERTEX_LINK != 0
    }

    /// True when the edge links to a preset level.
    pub fn is_preset_link(&self) -> bool {
        self.flags & VERTEX_PRESET_LINK != 0
    }
}

/// A neighbour ("orth") entry: level, direction (0 W, 1 N, 2 E, 3 S, −1 none), init flag,
/// box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Orth {
    /// Id of the neighbouring level.
    pub level_id: u32,
    /// Side the neighbour lies on.
    pub direction: i32,
    /// Whether the entry has been initialised.
    pub init: bool,
    /// The neighbour's tile box.
    pub rect: super::TileRect,
    /// The neighbour is a preset level (DrlgType 2).
    pub preset: bool,
}

/// A build-list node: preset id, file count, current file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildNode {
    /// The lvlprest id.
    pub preset: u32,
    /// Number of files the preset has.
    pub files: i32,
    /// The file handed out last.
    pub current: i32,
}

/// A path end-point: x, y, direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PathPoint {
    /// Cell x.
    pub x: i32,
    pub y: i32,
    /// Direction of the path at this point.
    pub direction: i32,
}

/// The four end-points of one dirt path: start, start-adjusted, join-adjusted, join.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PathEnds {
    /// Where the path starts.
    pub start: PathPoint,
    /// The start point after adjustment.
    pub start_adjusted: PathPoint,
    /// The join point after adjustment.
    pub join_adjusted: PathPoint,
    /// Where the path joins another.
    pub join: PathPoint,
}

/// The outdoor record of a level.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutdoorLevel {
    /// Level flags.
    pub flags: u32,
    /// Grids 0..3 (gw × gh).
    pub grids: [Grid; 4],
    /// Vertex polygon, head first (circular).
    pub vertices: Vec<Vertex>,
    /// Path end-points per path.
    pub path_ends: Vec<PathEnds>,
    /// Path vertex lists (tile coordinates after jitter).
    pub paths: Vec<Vec<(i32, i32)>>,
    /// Neighbour entries, head first.
    pub orth: Vec<Orth>,
    /// Build list, head first.
    pub build_list: Vec<BuildNode>,
    /// Act III jungle block ids, row-major from the top-left block; written at act
    /// creation.
    pub jungle_ids: Option<Vec<u32>>,
    /// Act III jungle clearing count.
    pub jungle_clearings: i32,
    /// Border substitution replacements of the last generation, in order (a record for
    /// inspection; see [`SubHit`]).
    pub sub_hits: Vec<SubHit>,
}

impl OutdoorLevel {
    /// Grid width in cells.
    pub fn gw(&self) -> i32 {
        self.grids[0].w
    }

    /// Grid height in cells.
    pub fn gh(&self) -> i32 {
        self.grids[0].h
    }
}

/// The outdoor record of a room and what the room's substitutions produced at its last
/// grid build.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutdoorRoom {
    /// Outdoor flags: the cell's grid-2 value.
    pub flags: u32,
    /// Extra flags: the cell's grid-3 value.
    pub flags_ex: u32,
    /// lvlsub type of the room.
    pub sub_type: i32,
    pub sub_theme: i32,
    /// Picked sub-theme rows.
    pub picked: u32,
    /// Grids of the last build (9 × 9): tile type, wall, floor.
    pub tile_type: CellGrid,
    /// Wall grid of the last build.
    pub wall: CellGrid,
    /// Floor grid of the last build.
    pub floor: CellGrid,
    /// Roof-list growth of the last build.
    pub roof_count: u32,
    /// Shadow tiles of the last build: world tile (x, y) and cell value.
    pub shadows: Vec<(i32, i32, u32)>,
    /// The tiles chosen for `shadows`, in order (empty when the build
    /// had no picker).
    pub shadow_tiles: Vec<TileRef>,
    /// Preset units added by the last build, in sub-tiles relative to the room.
    pub units: Vec<PresetUnit>,
}

/// The outdoor state of one act DRLG.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outdoor {
    /// Outdoor record per level.
    pub levels: BTreeMap<LevelIdx, OutdoorLevel>,
    /// Outdoor record per room.
    pub rooms: BTreeMap<DrlgRoomId, OutdoorRoom>,
    /// Preset direction written by the placer for levels 1, 27 and 40, by level id.
    /// Read by `drlg::preset`, which owns the preset info.
    pub preset_direction: BTreeMap<u32, i32>,
}

impl Outdoor {
    /// The outdoor record of a level, if it has one.
    pub fn level(&self, l: LevelIdx) -> Option<&OutdoorLevel> {
        self.levels.get(&l)
    }

    /// The outdoor record of a room, if it has one.
    pub fn room(&self, r: DrlgRoomId) -> Option<&OutdoorRoom> {
        self.rooms.get(&r)
    }

    /// The record of a level, created zeroed on first use. Initialisation at allocation
    /// only zeroes it, so creating it lazily gives the same state.
    pub fn info_mut(&mut self, l: LevelIdx) -> &mut OutdoorLevel {
        self.levels.entry(l).or_default()
    }

    /// Resets a level's outdoor record to zero at allocation.
    pub fn init_level(&mut self, l: LevelIdx) {
        self.levels.insert(l, OutdoorLevel::default());
    }

    /// Resets a level's type data when its rooms are freed: grids, polygon, paths and the
    /// build list.
    pub fn reset_level(&mut self, drlg: &Drlg, l: LevelIdx) {
        // Clears flags 0x20 (cliffs) and 0x40 (cave placed), drops the four grids, the
        // polygon and the path lists, and zeroes the path end-points. The remaining flags
        // and the neighbour entries stay: only act creation writes them. A regeneration
        // therefore places cliffs and the cave entrance again.
        // The reset runs after every room of the level was freed, while the level's room
        // list still names them, so the list cannot be walked here (it would read freed
        // rooms). A room's outdoor record goes with the room: drop the records of freed
        // rooms.
        self.rooms.retain(|&r, _| drlg.try_room(r).is_some());
        if let Some(info) = self.levels.get_mut(&l) {
            info.flags &= !(0x20 | 0x40);
            info.grids = Default::default();
            info.vertices.clear();
            info.path_ends.clear();
            info.paths.clear();
            info.build_list.clear();
        }
    }

    /// Frees the tile grids (tile type, wall, floor) of a room.
    pub fn free_room_tiles(&mut self, room: DrlgRoomId) {
        if let Some(r) = self.rooms.get_mut(&room) {
            r.tile_type = CellGrid::default();
            r.wall = CellGrid::default();
            r.floor = CellGrid::default();
        }
    }
}

/// The act of a level for dispatch to a builder: level 134 counts as Act II.
pub fn dispatch_act(id: u32) -> u8 {
    if id == 134 {
        1
    } else {
        act_of_level(id)
    }
}

// ---- LevelTypes adapter ---------------------------------------------------

/// [`LevelTypes`] for outdoor levels (DrlgType 3); every other level and
/// every other hook goes to `others`.
///
/// Errors: [`LevelTypes`] returns [`DrlgError`], which has no outdoor
/// variants (this module may not edit it). An [`OutdoorError::Drlg`] is
/// returned as is; any other outdoor error is kept in `last_error` and
/// reported as `DrlgError::UnknownLevel(level id)`.
pub struct OutdoorTypes<'a> {
    /// The outdoor state.
    pub outdoor: &'a mut Outdoor,
    /// Table view.
    pub od: &'a OutdoorData,
    /// Parsed substitution files.
    pub subs: &'a dyn SubFiles,
    /// Builder of preset cells.
    pub presets: &'a mut dyn OutdoorPresets,
    /// Provider for every non-outdoor level and hook.
    pub others: &'a mut dyn LevelTypes,
    /// The last outdoor error that was not an `OutdoorError::Drlg`.
    pub last_error: Option<OutdoorError>,
}

/// Allocation view used by the act placer: outdoor inits are lazy
/// ([`Outdoor::info_mut`]), everything else goes to the other provider.
struct NonOutdoor<'a> {
    others: &'a mut dyn LevelTypes,
}

impl LevelTypes for NonOutdoor<'_> {
    fn init_level(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        l: LevelIdx,
    ) -> Result<(), DrlgError> {
        if drlg.level(l).drlg_type == DRLG_OUTDOOR {
            return Ok(());
        }
        self.others.init_level(drlg, data, l)
    }

    fn generate(&mut self, drlg: &mut Drlg, data: &DrlgData, l: LevelIdx) -> Result<(), DrlgError> {
        self.others.generate(drlg, data, l)
    }
}

impl OutdoorTypes<'_> {
    fn report(&mut self, id: u32, e: OutdoorError) -> DrlgError {
        match e {
            OutdoorError::Drlg(d) => d,
            other => {
                self.last_error = Some(other);
                DrlgError::UnknownLevel(id)
            }
        }
    }

    fn is_outdoor(drlg: &Drlg, l: LevelIdx) -> bool {
        drlg.level(l).drlg_type == DRLG_OUTDOOR
    }
}

impl LevelTypes for OutdoorTypes<'_> {
    fn automap_streams(&self, drlg: &Drlg, l: LevelIdx) -> bool {
        !Self::is_outdoor(drlg, l) && self.others.automap_streams(drlg, l)
    }

    fn create_act_levels(&mut self, drlg: &mut Drlg, data: &DrlgData) -> Result<(), DrlgError> {
        let mut alloc = NonOutdoor {
            others: &mut *self.others,
        };
        let r = self
            .outdoor
            .create_act_levels(drlg, data, self.od, &mut alloc);
        r.map_err(|e| self.report(0, e))
    }

    fn init_level(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        l: LevelIdx,
    ) -> Result<(), DrlgError> {
        if Self::is_outdoor(drlg, l) {
            self.outdoor.init_level(l);
            Ok(())
        } else {
            self.others.init_level(drlg, data, l)
        }
    }

    fn generate(&mut self, drlg: &mut Drlg, data: &DrlgData, l: LevelIdx) -> Result<(), DrlgError> {
        if !Self::is_outdoor(drlg, l) {
            return self.others.generate(drlg, data, l);
        }
        let id = drlg.level(l).id;
        let r = self
            .outdoor
            .generate(drlg, data, self.od, self.subs, &mut *self.presets, l);
        r.map_err(|e| self.report(id, e))
    }

    fn reset_level(&mut self, drlg: &mut Drlg, l: LevelIdx) {
        if Self::is_outdoor(drlg, l) {
            self.outdoor.reset_level(drlg, l);
        } else {
            self.others.reset_level(drlg, l);
        }
    }

    fn add_preset_units(&mut self, drlg: &mut Drlg, room: DrlgRoomId) -> Result<(), DrlgError> {
        self.others.add_preset_units(drlg, room)
    }

    fn preset_units(&self, drlg: &Drlg, room: DrlgRoomId) -> Vec<PresetUnit> {
        self.others.preset_units(drlg, room)
    }

    fn room_grids(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        room: DrlgRoomId,
    ) -> Result<RoomGrids, DrlgError> {
        if !self.outdoor.rooms.contains_key(&room) {
            return self.others.room_grids(drlg, data, room);
        }
        let id = drlg.level(drlg.room(room).level).id;
        let r = self.outdoor.room_grids(drlg, self.od, self.subs, room);
        r.map_err(|e| self.report(id, e))
    }

    fn free_room_tiles(&mut self, drlg: &mut Drlg, room: DrlgRoomId) {
        if self.outdoor.rooms.contains_key(&room) {
            self.outdoor.free_room_tiles(room);
        } else {
            self.others.free_room_tiles(drlg, room);
        }
    }

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
        self.others
            .door_unit(drlg, data, room, wx, wy, cell, orientation)
    }

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
        self.others
            .warp_unit(drlg, data, room, wx, wy, cell, orientation)
    }
}

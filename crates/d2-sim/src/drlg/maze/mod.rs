//! Maze levels (DrlgType 1): a grid of equal-sized preset cells grown
//! from the middle of the level rectangle, linked by shape, stamped with
//! special cells, moved into the level rect, themed, then handed to the
//! preset builder with a DS1 file index.
//!
//! Module map:
//! - [`specials`]: the special-room tables, parsed from
//!   `tables/drlg/maze-specials.tsv`;
//! - [`cells`]: cells, links and the cell primitives;
//! - [`layout`]: the generation sequence, layout builders, special cells,
//!   neighbour placement, theme pass and build.
//!
//! Cells are real DRLG rooms of the level ([`Drlg::alloc_room`], which
//! draws a level-seed step and a room-seed step), kept in the level's
//! room list newest first. Their maze data (def, file, lock, links) lives
//! here beside them: `drlg::room` has no orth links. The rooms the cells
//! are built into get their links here too; they are dropped
//! with the generation state. Nothing reads a built room's links after
//! generation except the room free, so dropping them is not observable.
//!
//! Seams: [`MazePresets`] reaches `drlg::preset` (DS1 map alloc and its
//! file draw, building a map into rooms, a preset level's direction) and
//! the act's get-or-allocate. [`Maze`] is not a [`super::LevelTypes`]:
//! the composite provider dispatching by DrlgType calls
//! [`Maze::init_level`], [`Maze::generate`] and [`Maze::reset_level`] and
//! maps [`MazeError`] (DrlgError has no variant for it yet).

pub mod cells;
pub mod layout;
pub mod specials;

#[cfg(test)]
mod links_tests;
#[cfg(test)]
mod mutant_tests;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use d2_data::tables::{Lvlmaze, Lvlprest};
use thiserror::Error;

use super::data::DrlgData;
use super::level::Drlg;
use super::{DrlgError, DrlgRoomId, LevelIdx, TileRect};

pub use cells::{Cell, LinkTarget, MazeLink};
pub use specials::{SpecialRow, Specials, SPECIALS_TSV};

/// One `lvlmaze.bin` record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MazeRow {
    /// `Level` (+0).
    pub level: u32,
    /// `Rooms[d]` (+4 + 4·d).
    pub rooms: [u32; 3],
    /// `SizeX` (+16): cell width in tiles.
    pub size_x: i32,
    /// `SizeY` (+20): cell height in tiles.
    pub size_y: i32,
    /// `Merge` (+24): per-mille link chance (compared signed).
    pub merge: i32,
}

impl MazeRow {
    /// Converts a decoded `lvlmaze` record.
    pub fn from_record(r: &Lvlmaze) -> Self {
        Self {
            level: r.level,
            rooms: [r.rooms, r.rooms_n, r.rooms_h],
            size_x: r.sizex as i32,
            size_y: r.sizey as i32,
            merge: r.merge as i32,
        }
    }
}

/// The tables maze code reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MazeData {
    /// lvlmaze records in file order.
    pub rows: Vec<MazeRow>,
    /// lvlprest `Def` → `Files` (+64), first row of each def.
    pub prest_files: BTreeMap<u32, u32>,
    /// The special-room tables.
    pub specials: Specials,
}

impl MazeData {
    /// The view from decoded records (`d2_data::tables::decode_all`).
    pub fn from_tables(lvlmaze: &[Lvlmaze], lvlprest: &[Lvlprest]) -> Self {
        let mut prest_files = BTreeMap::new();
        for p in lvlprest {
            prest_files.entry(p.def).or_insert(p.files);
        }
        Self {
            rows: lvlmaze.iter().map(MazeRow::from_record).collect(),
            prest_files,
            specials: Specials::shipped(),
        }
    }

    /// The index of the first record whose `Level` is `id`, in file order. No match is
    /// an error.
    pub fn row_index(&self, id: u32) -> Result<usize, MazeError> {
        self.rows
            .iter()
            .position(|r| r.level == id)
            .ok_or(MazeError::NoMazeRow(id))
    }

    /// lvlprest `Files` of a def.
    pub fn files(&self, def: u32) -> Result<u32, MazeError> {
        self.prest_files
            .get(&def)
            .copied()
            .ok_or(MazeError::NoPrest(def))
    }
}

/// A DS1 map allocated by [`MazePresets::alloc_map`] (provider handle).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MapId(pub u32);

/// What maze code needs from `drlg::preset` and the act (a narrow seam; the provider is
/// the preset level type).
pub trait MazePresets {
    /// Get-or-allocate a level through the act's level types (maze code asks for the
    /// Outer Cloister, 27, and the Chaos Sanctum, 108).
    fn level(&mut self, drlg: &mut Drlg, data: &DrlgData, id: u32) -> Result<LevelIdx, DrlgError>;

    /// A preset level's direction (from its preset data).
    fn preset_direction(&self, drlg: &Drlg, level: LevelIdx) -> Result<u32, DrlgError>;

    /// Allocates a DS1 map for `def` at `rect` in `level`. Draws `roll(Files)` on the
    /// level seed as the map's default file.
    fn alloc_map(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        level: LevelIdx,
        def: u32,
        rect: TileRect,
    ) -> Result<MapId, DrlgError>;

    /// Overwrites a map's file.
    fn set_map_file(&mut self, map: MapId, file: i32);

    /// Builds the map into room(s) with room flags 0, in single-room mode when `small`
    /// (the cell is at most 12 × 12). Returns the last room built (`None` only when it
    /// built none). `links` are the cell's init-flag links in list order (newest first),
    /// for the provider's information: maze code itself links the returned room to
    /// them.
    fn build_map(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        level: LevelIdx,
        map: MapId,
        small: bool,
        links: &[MazeLink],
    ) -> Result<Option<DrlgRoomId>, DrlgError>;
}

/// Maze generation errors: placements the layout cannot continue from, and missing
/// data.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MazeError {
    /// A DRLG error from below.
    #[error(transparent)]
    Drlg(#[from] DrlgError),
    /// The level has no lvlmaze record.
    #[error("no lvlmaze record for level {0}")]
    NoMazeRow(u32),
    /// The level was generated before `init_level`.
    #[error("level {0} has no lvlmaze record stored (init_level not run)")]
    NotInitialized(u32),
    /// The difficulty has no `Rooms` entry.
    #[error("lvlmaze record has no Rooms entry for difficulty {0}")]
    BadDifficulty(u8),
    /// The level type has no maze builder or shape table.
    #[error("level type {0} is not a maze type (\"Some really bad voodoo\")")]
    BadLevelType(u32),
    /// No lvlprest row has this def.
    #[error("no lvlprest row with Def {0}")]
    NoPrest(u32),
    /// The def has no DS1 files, so files cannot rotate.
    #[error("lvlprest Def {0} has Files 0 (division by zero)")]
    ZeroFiles(u32),
    /// A placement was rejected where the layout needs a cell to continue from.
    #[error("placement rejected for a null cell: {0}")]
    NullCell(&'static str),
    /// The generated cells are larger than the level rect.
    #[error("cells do not fit the level rect")]
    DoesNotFit,
    /// A Sewers 1 corner swap found no unlocked cell with the def.
    #[error("Act 3 Sewers 1 corner swap: no unlocked cell with def {0}")]
    MissingSwap(u32),
    /// The tomb hub cell has no def in 444..447.
    #[error("tomb hub def {0:?} is not 444..447")]
    TombHub(Option<u32>),
    /// The Outer Cloister direction has no extreme-cell finder.
    #[error("Outer Cloister preset direction {0} outside the finder table")]
    BarracksDirection(u32),
    /// A special table has no such row.
    #[error("special table {0} has no row {1}")]
    NoSpecialRow(&'static str, usize),
}

/// A file rotation record: the cells of one def take successive DS1 files in turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rotation {
    /// The preset def.
    pub def: u32,
    /// The def's file count.
    pub n: u32,
    /// The file most recently handed out.
    pub v: i32,
}

/// Maze type data of one level: the lvlmaze record and the file rotation list (head
/// first).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MazeLevel {
    /// Index into `MazeData::rows`, once the level is initialised.
    pub row: Option<usize>,
    /// The file rotation records, newest first.
    pub rotation: Vec<Rotation>,
}

/// The maze level type: its tables and the per-level type data of every
/// act (keyed by act and level slot).
#[derive(Clone, Debug, Default)]
pub struct Maze {
    /// The tables.
    pub data: MazeData,
    levels: BTreeMap<(u8, LevelIdx), MazeLevel>,
}

impl Maze {
    /// A maze level type over `data`, with no levels initialised.
    pub fn new(data: MazeData) -> Self {
        Self {
            data,
            levels: BTreeMap::new(),
        }
    }

    /// Stores the level's lvlmaze record. The DRLG then sets the level's position and
    /// size.
    pub fn init_level(&mut self, drlg: &Drlg, level: LevelIdx) -> Result<(), MazeError> {
        let row = self.data.row_index(drlg.level(level).id)?;
        self.levels.entry((drlg.act, level)).or_default().row = Some(row);
        Ok(())
    }

    /// The type data of a level, if any.
    pub fn level_data(&self, drlg: &Drlg, level: LevelIdx) -> Option<&MazeLevel> {
        self.levels.get(&(drlg.act, level))
    }

    /// Type data reset when the level's rooms are freed and the level is kept: the
    /// rotation records are dropped, the lvlmaze record stays (it is cleared only when
    /// the level itself is freed, [`Maze::free_level`]).
    pub fn reset_level(&mut self, drlg: &Drlg, level: LevelIdx) {
        if let Some(m) = self.levels.get_mut(&(drlg.act, level)) {
            m.rotation.clear();
        }
    }

    /// Drops all maze data of a level that is freed without being kept.
    pub fn free_level(&mut self, drlg: &Drlg, level: LevelIdx) {
        self.levels.remove(&(drlg.act, level));
    }

    /// Generates the level's cells and builds them.
    pub fn generate(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        presets: &mut dyn MazePresets,
        level: LevelIdx,
    ) -> Result<(), MazeError> {
        let id = drlg.level(level).id;
        let key = (drlg.act, level);
        let row = self
            .levels
            .get(&key)
            .and_then(|m| m.row)
            .ok_or(MazeError::NotInitialized(id))?;
        let row = self.data.rows[row];
        let rotation = &mut self.levels.get_mut(&key).expect("stored above").rotation;
        let mut gen = cells::Gen::new(drlg, level, row, &self.data)?;
        layout::generate(&mut gen, data, presets, rotation)
    }
}

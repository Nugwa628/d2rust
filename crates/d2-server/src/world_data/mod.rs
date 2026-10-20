//! Real-file providers of the DRLG data seams: parsed DS1 files for
//! [`Ds1Source`] (lvlprest) and [`SubFiles`] (lvlsub), parsed DT1 tile
//! headers for [`TileSource`], and the table views the level types read
//! ([`tables`]).
//!
//! The sim does no I/O: the server reads the bytes (from the archive set,
//! [`archive`]), parses them with `d2_formats::{ds1, dt1}` and hands the
//! sim the seam types. This module adds no DRLG rule: the DS1 parsing
//! rules stay in `d2_sim::drlg::preset` ([`Ds1File::from_input`]); here
//! the parsed file is only re-expressed as the seam's plain data (stored
//! width / height, orientations as stored).
//!
//! Everything is loaded up front ([`WorldFiles::load`]): every DS1 a
//! lvlprest or lvlsub row names and every DT1 a lvltypes row names, plus
//! the fixed tile library. A named file that is missing or does not parse
//! is a load error; nothing falls back.

pub mod archive;
pub mod game;
pub mod tables;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use d2_formats::ds1::Ds1;
use d2_formats::dt1::{Dt1, Dt1Tile};
use d2_sim::drlg::outdoor::{OutdoorData, SubFile, SubFileMap, SubFiles, SubGroup};
use d2_sim::drlg::preset::{
    Ds1File, Ds1Input, Ds1ObjectInput, Ds1PathInput, Ds1Source, PresetData, PresetError,
};
use d2_sim::drlg::tiles::{CellGrid, ACT_EDGE_TILE, FIXED_LIBRARY};
use d2_sim::drlg::{DrlgData, TileInfo, TileSource};

/// The archive prefix of tile files (`DATA\GLOBAL\TILES\`).
pub const TILE_PREFIX: &[u8] = b"DATA\\GLOBAL\\TILES\\";

/// A file the level types need could not be provided.
#[derive(Debug, thiserror::Error)]
pub enum WorldDataError {
    /// The reader could not supply the file (missing from the archives,
    /// or a read error).
    #[error("{path}: {detail}")]
    Read { path: String, detail: String },
    /// The file was read but `d2_formats` could not parse it.
    #[error("{path}: {source}")]
    Parse {
        path: String,
        source: d2_formats::FormatError,
    },
    /// `d2_formats::ds1` already remaps the orientations of v < 7 files
    /// with its 25-entry table; the stored values the seam takes
    /// (`Ds1Input::orientations`, mapped by the sim's 42-entry table) are
    /// lost. No such file is supported.
    #[error("{path}: DS1 version {version} < 7: stored orientations are not available")]
    OldOrientations { path: String, version: u32 },
    /// A path is not ASCII (archive names are).
    #[error("path {0:?} is not ASCII")]
    BadPath(Vec<u8>),
    /// A lvlsub file's preset units failed to build.
    #[error("{path}: {source}")]
    Preset { path: String, source: PresetError },
    /// A table the views need is absent or of the wrong size.
    #[error("table {table}: {detail}")]
    Table { table: String, detail: String },
}

/// The archive name of a table tile path: a string that already carries
/// [`TILE_PREFIX`] (ASCII case-insensitive; a fixed-up table, or the fixed
/// library) is the name; otherwise every `/` becomes `\` and, if the
/// string has more than one character, the prefix is prepended.
pub fn archive_name(path: &[u8]) -> Vec<u8> {
    if has_prefix(path) {
        return path.to_vec();
    }
    let s: Vec<u8> = path
        .iter()
        .map(|&b| if b == b'/' { b'\\' } else { b })
        .collect();
    if s.len() > 1 {
        [TILE_PREFIX, &s].concat()
    } else {
        s
    }
}

fn has_prefix(path: &[u8]) -> bool {
    path.len() >= TILE_PREFIX.len() && path[..TILE_PREFIX.len()].eq_ignore_ascii_case(TILE_PREFIX)
}

/// A table string names a file: more than one character (the 0/1-character
/// strings are the `0` placeholders, which the path fix leaves alone).
pub fn names_file(s: &[u8]) -> bool {
    s.len() > 1
}

/// A parsed DS1 as the preset seam takes it (`Ds1Input`): the stored
/// width and height (`d2_formats` reports them + 1), the stored act, the
/// layers and records as stored.
pub fn ds1_input(path: &[u8], d: &Ds1) -> Result<Ds1Input, WorldDataError> {
    if d.version < 7 {
        return Err(WorldDataError::OldOrientations {
            path: String::from_utf8_lossy(path).into_owned(),
            version: d.version,
        });
    }
    Ok(Ds1Input {
        version: d.version,
        width: d.width.wrapping_sub(1),
        height: d.height.wrapping_sub(1),
        act: d.act as i32,
        tag_type: d.tag_type,
        walls: d.walls.clone(),
        orientations: d.orientations.clone(),
        floors: d.floors.clone(),
        shadow: d.shadow.clone(),
        objects: d
            .objects
            .iter()
            .map(|o| Ds1ObjectInput {
                kind: o.kind,
                id: o.id,
                x: o.x,
                y: o.y,
                flags: o.flags,
            })
            .collect(),
        paths: d
            .paths
            .iter()
            .map(|p| Ds1PathInput {
                x: p.x,
                y: p.y,
                points: p.points.iter().map(|q| (q.x, q.y, q.action)).collect(),
            })
            .collect(),
    })
}

/// A lvlsub DS1 as the outdoor seam takes it: the DS1 file record after the
/// preset parser ([`Ds1File::from_input`]), its groups (variant count = the
/// group's last value), and grids of (W + 1) × (H + 1) cells.
/// Wall layers are raw (the `k << 18` OR is the sim's, [`SubFile::wall_at`]).
pub fn sub_file(path: &[u8], d: &Ds1, pd: &PresetData) -> Result<SubFile, WorldDataError> {
    let input = ds1_input(path, d)?;
    let file = Ds1File::from_input(&input, pd).map_err(|source| WorldDataError::Preset {
        path: String::from_utf8_lossy(path).into_owned(),
        source,
    })?;
    let (w, h) = (file.stride(), file.height as usize + 1);
    let grid = |cells: &Vec<u32>| CellGrid {
        width: w,
        height: h,
        cells: cells.clone(),
    };
    Ok(SubFile {
        method: file.tag_type,
        groups: d
            .groups
            .iter()
            .map(|g| SubGroup {
                x: g.x as i32,
                y: g.y as i32,
                w: g.width as i32,
                h: g.height as i32,
                variants: g.unknown as i32,
            })
            .collect(),
        floor: file.floors.first().map(grid),
        walls: file.walls.iter().map(grid).collect(),
        tile_types: file.orientations.iter().map(grid).collect(),
        shadow: Some(grid(&file.shadow)),
        units: file.units.iter().map(|u| u.to_seam()).collect(),
    })
}

/// One DT1 tile header as the DRLG reads it.
pub fn tile_info(t: &Dt1Tile) -> TileInfo {
    TileInfo {
        orientation: t.orientation,
        main: t.main_index,
        sub: t.sub_index,
        rarity: t.rarity,
        material: t.material_flags,
        subtile_flags: t.subtile_flags,
        roof_height: t.roof_height,
        height: t.height,
        light_direction: t.light_direction,
    }
}

/// Parsed lvlprest DS1 files, keyed by the table string the preset code
/// asks with.
#[derive(Clone, Debug, Default)]
pub struct Ds1Files(pub BTreeMap<Vec<u8>, Ds1Input>);

impl Ds1Source for Ds1Files {
    fn ds1(&self, path: &[u8]) -> Option<&Ds1Input> {
        self.0.get(path)
    }
}

/// DT1 tile headers in file order, keyed by the table string (or fixed
/// library path) the room build asks with.
#[derive(Clone, Debug, Default)]
pub struct Dt1Files(pub BTreeMap<Vec<u8>, Vec<TileInfo>>);

impl TileSource for Dt1Files {
    fn dt1(&self, path: &[u8]) -> Option<&[TileInfo]> {
        self.0.get(path).map(Vec::as_slice)
    }
}

/// Every file the level types read, parsed.
#[derive(Clone, Debug, Default)]
pub struct WorldFiles {
    /// The lvlprest DS1 files.
    pub ds1: Ds1Files,
    /// The lvlsub DS1 files, in the shape the outdoor generator reads.
    pub subs: SubFileMap,
    /// The DT1 tile headers of every level type and the fixed library.
    pub dt1: Dt1Files,
}

impl WorldFiles {
    /// Loads every DS1 named by a lvlprest `File1`–`File6` or lvlsub
    /// `File` string and every DT1 named by a lvltypes `File` string, the
    /// fixed library or an act's base library. `read` gets the archive name
    /// ([`archive_name`]) and returns the file's bytes.
    pub fn load(
        drlg: &DrlgData,
        pd: &PresetData,
        od: &OutdoorData,
        read: impl FnMut(&str) -> Result<Vec<u8>, String>,
    ) -> Result<Self, WorldDataError> {
        let read = std::cell::RefCell::new(read);
        Self::load_typed(
            drlg,
            pd,
            od,
            |path| parse_ds1(path, &mut *read.borrow_mut()),
            |path| {
                let bytes = read_file(path, &mut *read.borrow_mut())?;
                Dt1::parse(&bytes).map_err(|source| WorldDataError::Parse {
                    path: lossy(path),
                    source,
                })
            },
        )
    }

    /// [`WorldFiles::load`] over typed readers:
    /// `ds1` / `dt1` get the file's path as the table names it and return
    /// the decoded file, from the archives or from a native folder.
    pub fn load_typed(
        drlg: &DrlgData,
        pd: &PresetData,
        od: &OutdoorData,
        mut ds1: impl FnMut(&[u8]) -> Result<Ds1, WorldDataError>,
        mut dt1: impl FnMut(&[u8]) -> Result<Dt1, WorldDataError>,
    ) -> Result<Self, WorldDataError> {
        let mut out = Self::default();
        for path in pd.defs.iter().flat_map(|d| d.file.iter()) {
            if !names_file(path) || out.ds1.0.contains_key(path) {
                continue;
            }
            let d = ds1(path)?;
            out.ds1.0.insert(path.clone(), ds1_input(path, &d)?);
        }
        for path in od.subs.iter().map(|r| &r.file) {
            if !names_file(path) || out.subs.0.contains_key(path) {
                continue;
            }
            let d = ds1(path)?;
            out.subs.0.insert(path.clone(), sub_file(path, &d, pd)?);
        }
        let library = drlg
            .lvltypes
            .iter()
            .flatten()
            .map(Vec::as_slice)
            .chain(FIXED_LIBRARY);
        for path in library {
            if !names_file(path) || out.dt1.0.contains_key(path) {
                continue;
            }
            let dt1 = dt1(path)?;
            out.dt1
                .0
                .insert(path.to_vec(), dt1.tiles.iter().map(tile_info).collect());
        }
        // The acts' base libraries, read by the client's edge floor record.
        // A library the install lacks is left out: only a level drawing edge
        // floors needs it, and the client reports it there.
        for &(path, _) in ACT_EDGE_TILE.iter().flatten() {
            match dt1(path) {
                Ok(d) => {
                    out.dt1
                        .0
                        .insert(path.to_vec(), d.tiles.iter().map(tile_info).collect());
                }
                Err(WorldDataError::Read { .. }) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }
}

impl SubFiles for WorldFiles {
    fn sub_file(&self, file: &[u8]) -> Option<&SubFile> {
        self.subs.sub_file(file)
    }
}

fn lossy(path: &[u8]) -> String {
    String::from_utf8_lossy(path).into_owned()
}

/// The archive name (`\` separators, ASCII) of a table's file path.
pub fn file_name(path: &[u8]) -> Result<String, WorldDataError> {
    let name = archive_name(path);
    if !name.is_ascii() {
        return Err(WorldDataError::BadPath(path.to_vec()));
    }
    String::from_utf8(name).map_err(|e| WorldDataError::BadPath(e.into_bytes()))
}

fn read_file(
    path: &[u8],
    read: &mut impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<Vec<u8>, WorldDataError> {
    let name = file_name(path)?;
    read(&name).map_err(|detail| WorldDataError::Read { path: name, detail })
}

fn parse_ds1(
    path: &[u8],
    read: &mut impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<Ds1, WorldDataError> {
    let bytes = read_file(path, read)?;
    Ds1::parse(&bytes).map_err(|source| WorldDataError::Parse {
        path: lossy(path),
        source,
    })
}

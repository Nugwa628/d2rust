//! Map preview: one DS1 map and its DT1 tiles, turned into something drawable.
//!
//! [`load`] reads a map, every tile file it lists and its act palette from the archives.
//! [`tiles`] assembles each tile's blocks into one image and keeps them in a
//! [`TileLibrary`]; [`layout`] places the map's cells in isometric screen space as an
//! ordered draw list; [`cpu`] paints that list into pixels. [`add_sprites`] puts two demo
//! sprites on top.
//!
//! No Bevy types appear here: everything is unit-testable, and the GPU preview draws the
//! same layout from the same tile library.

pub mod cpu;
pub mod layout;
pub mod tiles;

#[cfg(test)]
mod simplification_tests;

use d2_formats::dc6::Dc6;
use d2_formats::dcc::Dcc;
use d2_formats::ds1::Ds1;
use d2_formats::dt1::Dt1;
use d2_formats::mpq::ArchiveSet;
use d2_formats::palette::Palette;

pub use layout::{build, Bounds, DrawItem, Layout};
pub use tiles::{TileImage, TileKey, TileLibrary};

/// Everything needed to draw one preset map.
pub struct LoadedMap {
    pub ds1: Ds1,
    /// The tiles of every DT1 file the map lists that was found.
    pub library: TileLibrary,
    /// The palette of the map's act.
    pub palette: Palette,
    /// DT1 files the map lists but the archives do not have.
    pub missing_files: Vec<String>,
}

/// Why a map or sprite could not be loaded. Every variant names the archive path.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// The file could not be read from the archives.
    #[error("{path}: {source}")]
    Mpq {
        path: String,
        source: d2_formats::mpq::MpqError,
    },
    /// The file was read but does not parse.
    #[error("{path}: {source}")]
    Format {
        path: String,
        source: d2_formats::FormatError,
    },
}

/// Reads `path` from the archives.
fn read(archives: &ArchiveSet, path: &str) -> Result<Vec<u8>, LoadError> {
    archives.read(path).map_err(|source| LoadError::Mpq {
        path: path.to_owned(),
        source,
    })
}

/// Reads `path` from the archives and parses it with `parse`.
fn read_parsed<T>(
    archives: &ArchiveSet,
    path: &str,
    parse: impl FnOnce(&[u8]) -> Result<T, d2_formats::FormatError>,
) -> Result<T, LoadError> {
    parse(&read(archives, path)?).map_err(|source| LoadError::Format {
        path: path.to_owned(),
        source,
    })
}

/// Loads a DS1, its DT1 files and its act palette straight from the archives. Tile files
/// the archives lack are listed in [`LoadedMap::missing_files`], not an error.
pub fn load(archives: &ArchiveSet, ds1_path: &str) -> Result<LoadedMap, LoadError> {
    let ds1 = read_parsed(archives, ds1_path, Ds1::parse)?;
    let mut library = TileLibrary::new();
    let mut missing_files = Vec::new();
    for path in tiles::dt1_paths(&ds1) {
        if !archives.contains(&path) {
            missing_files.push(path);
            continue;
        }
        library.add_dt1(&read_parsed(archives, &path, Dt1::parse)?);
    }
    let palette = read_parsed(archives, &palette_path(ds1.act), Palette::parse)?;
    Ok(LoadedMap {
        ds1,
        library,
        palette,
        missing_files,
    })
}

/// The palette file of a DS1's 0-based act (acts past the fifth use the fifth's).
pub fn palette_path(act: u32) -> String {
    format!(r"data\global\palette\act{}\pal.dat", act.min(4) + 1)
}

/// Demo DCC sprite: the lit bonfire of the Rogue Encampment, body layer.
pub const DEMO_DCC: &str = r"data\global\objects\RB\TR\rbtrlitonhth.dcc";
/// Demo DC6 sprite: a piece of the control panel.
pub const DEMO_DC6: &str = r"data\global\ui\PANEL\ctrlpnl7.DC6";

/// Puts the demo sprites on top of the map, after every map item: frame 0 of the DCC's
/// first direction, anchored at the centre of the map's middle cell, and the DC6's
/// first-direction frames side by side, 32 pixels in from the map's top-left corner.
/// Sprites use a tile key no map tile has (orientation `u32::MAX`).
pub fn add_sprites(layout: &mut Layout, lib: &mut TileLibrary, ds1: &Ds1, dcc: &Dcc, dc6: &Dc6) {
    let key = TileKey {
        orientation: u32::MAX,
        main: 0,
        sub: 0,
    };
    let mut push = |layout: &mut Layout, w: u32, h: u32, pixels: &[u8], x: i32, y: i32| {
        if w == 0 || h == 0 {
            return;
        }
        lib.images.push(TileImage {
            x0: 0,
            y0: 0,
            width: w,
            height: h,
            pixels: pixels.to_vec(),
            roof_height: 0,
        });
        layout.items.push(DrawItem {
            image: lib.images.len() - 1,
            x,
            y,
            key,
            cell: (-1, -1),
            source: layout::Source::Sprite,
        });
    };
    let corner = layout.bounds.map(|b| (b.x0, b.y0)).unwrap_or((0, 0));

    let (cx, cy) = (ds1.width as i32 / 2, ds1.height as i32 / 2);
    let (sx, sy) = layout::cell_origin(cx, cy);
    let (ax, ay) = (sx + layout::CELL_WIDTH / 2, sy + layout::CELL_HEIGHT / 2);
    if let Some(f) = dcc.directions.first().and_then(|d| d.frames.first()) {
        push(
            layout,
            f.width,
            f.height,
            &f.pixels,
            ax + f.x_min,
            ay + f.y_min,
        );
    }

    let frames = dc6.header.frames_per_direction as usize;
    let mut x = corner.0 + 32;
    for f in dc6.frames.iter().take(frames) {
        push(layout, f.width, f.height, &f.pixels, x, corner.1 + 32);
        x += f.width as i32;
    }
    layout.update_bounds(lib);
}

/// Loads the demo sprites ([`DEMO_DCC`], [`DEMO_DC6`]) straight from the archives.
pub fn load_sprites(archives: &ArchiveSet) -> Result<(Dcc, Dc6), LoadError> {
    let dcc = read_parsed(archives, DEMO_DCC, Dcc::parse)?;
    let dc6 = read_parsed(archives, DEMO_DC6, Dc6::parse)?;
    Ok((dcc, dc6))
}

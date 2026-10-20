//! Sprite frames: palette-index images cut out of the game's sprite files,
//! the store that numbers them, and the atlas they are packed into.
//!
//! This is the sprite side of the client's asset pipeline:
//! - `set`: a parsed DCC, DC6 or DT1 file is cut into [`FrameSet`]s (one
//!   per direction or per tile) of [`IndexFrame`]s, keyed by
//!   [`FrameSetKey`] (canonical path + part).
//! - [`store`]: the resident sets, each frame numbered with a scene
//!   `FrameId`; it feeds the CPU compositor directly and builds the GPU
//!   compositor's atlas.
//! - [`atlas`]: a deterministic shelf packer that copies frame pixels into
//!   shared `PAGE_SIZE²` pages, CPU side.
//! - [`upload`]: the only Bevy edge, turning changed pages into textures.
//!
//! Plain Rust: no Bevy types outside [`upload`], the page upload edge.
//! Frames carry no color and no screen position: offsets are the file
//! format's own values, unchanged. How they become a screen position is
//! decided by `crate::rules::placement`, not here. Everything here is
//! strict: a malformed frame, a wrong part or a non-canonical path is an
//! error, never a silent default.

pub mod atlas;
pub mod store;
pub mod upload;

#[cfg(test)]
mod tests;

use std::fmt;

use d2_formats::dc6::Dc6;
use d2_formats::dcc::Dcc;
use d2_formats::dt1::Dt1;

pub use atlas::{Atlas, AtlasError, AtlasSlot, CheckReport, GUTTER, PAGE_SIZE};
pub use store::{FrameStore, StoreError};

/// One decoded frame: 8-bit palette indices, row-major, top row first,
/// index 0 = transparent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexFrame {
    pub width: u32,
    pub height: u32,
    /// The format's own offsets, unchanged: DC6 `offset_x/offset_y`, DCC
    /// frame box top-left `(x_min, y_min)`, DT1 tile image `(x0, y0)`.
    /// Offsets → screen pixel: `crate::rules::placement`
    /// (`render/sprite-placement.md` §8), which reads [`IndexFrame::anchor`].
    pub x_off: i32,
    pub y_off: i32,
    /// What `y_off` names (`render/sprite-placement.md` §2, §4, §8).
    pub anchor: FrameAnchor,
    pub pixels: Vec<u8>,
}

/// What `y_off` names and how the original's rasterizer clips the frame
/// (`render/sprite-placement.md` §2–§5, §7, §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FrameAnchor {
    /// `y_off` is the top row and the drawn rows are the image rows inside
    /// the frame: DCC frame boxes (bottom-up cels, §3), DT1 tile images
    /// (block drawers, §7), and frames built with [`IndexFrame::new`]
    /// (synthetic frames whose offsets are already the top-left).
    #[default]
    Top,
    /// `y_off` is the bottom row, inclusive: a DC6 frame with
    /// `flip & 1 = 0` (bottom-up cel, §2).
    Bottom,
    /// `y_off` is the top row of a top-down cel: a DC6 frame with
    /// `flip & 1 = 1` (§4), clipped by the rasterizer as if bottom-up.
    TopDown,
}

impl IndexFrame {
    /// A frame whose pixel count must equal `width × height`.
    pub fn new(
        width: u32,
        height: u32,
        x_off: i32,
        y_off: i32,
        pixels: Vec<u8>,
    ) -> Result<Self, FrameError> {
        let expected = u64::from(width) * u64::from(height);
        if pixels.len() as u64 != expected {
            return Err(FrameError::PixelCount {
                width,
                height,
                len: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            x_off,
            y_off,
            anchor: FrameAnchor::Top,
            pixels,
        })
    }

    /// The frame with `anchor` set.
    pub fn with_anchor(mut self, anchor: FrameAnchor) -> Self {
        self.anchor = anchor;
        self
    }

    /// No pixels at all (a 0-wide or 0-high frame).
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// The frame of block 0 in a DT1 tile's frame set ([`FrameSet::from_dt1`]).
pub const DT1_BLOCK_FRAME0: usize = 1;

/// Which part of a file a frame set holds (`assets.md` §A3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FramePart {
    /// One direction of a DCC or DC6.
    Dir(u8),
    /// One tile of a DT1, by its index in the file's tile list.
    Tile(u32),
}

/// Key of a derived frame set: a canonical archive path plus the part.
///
/// The path is the canonical form of `assets.md` §A1 (lowercase ASCII,
/// `/` separators, no leading `/`). Canonicalizing is the asset task's
/// job (C1); this type only refuses a path not already canonical.
/// Ordered (`BTreeMap` keys for deterministic eviction, §A5).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FrameSetKey {
    path: String,
    part: FramePart,
}

impl FrameSetKey {
    pub fn new(path: impl Into<String>, part: FramePart) -> Result<Self, FrameError> {
        let path = path.into();
        let canonical = !path.is_empty()
            && !path.starts_with('/')
            && path
                .bytes()
                .all(|b| b.is_ascii() && !b.is_ascii_uppercase() && b != b'\\');
        if !canonical {
            return Err(FrameError::NonCanonicalPath(path));
        }
        Ok(Self { path, part })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn part(&self) -> FramePart {
        self.part
    }
}

/// The frames of one direction (DCC/DC6) or one tile (DT1), in file
/// order. A DT1 tile without blocks has no image (`map-preview.md` §Tile
/// images) and gives an empty set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameSet {
    pub frames: Vec<IndexFrame>,
}

impl FrameSet {
    /// Budget size (`assets.md` §A5): `Σ width × height` plus a fixed
    /// header per frame.
    pub fn byte_size(&self) -> u64 {
        let header = std::mem::size_of::<IndexFrame>() as u64;
        self.frames
            .iter()
            .map(|f| f.pixels.len() as u64 + header)
            .sum()
    }

    /// Direction `dir` of a DCC; frames keep their DCC frame boxes. A frame
    /// whose `variable0` is odd is an error: the original would draw it
    /// top-down from the box's bottom row (`sprite-placement.md` §3, §4,
    /// open question 2), which a frame box cannot express.
    pub fn from_dcc(dcc: &Dcc, dir: u8) -> Result<Self, FrameError> {
        let d = dcc
            .directions
            .get(usize::from(dir))
            .ok_or(FrameError::NoPart {
                part: FramePart::Dir(dir),
                count: dcc.directions.len(),
            })?;
        let frames = d
            .frames
            .iter()
            .enumerate()
            .map(|(i, f)| {
                if f.variable0 & 1 != 0 {
                    return Err(FrameError::DccVariable0 {
                        dir,
                        frame: i,
                        variable0: f.variable0,
                    });
                }
                IndexFrame::new(f.width, f.height, f.x_min, f.y_min, f.pixels.clone())
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { frames })
    }

    /// Direction `dir` of a DC6; frames keep `offset_x/offset_y` and carry
    /// the orientation bit as [`FrameAnchor`] (`sprite-placement.md` §8).
    /// A `flip` other than 0 or 1 is an error: the original tests bit 0
    /// only (§1) while the decoder reads any non-zero `flip` as top-down,
    /// so such a frame's rows would not be the original's. A file whose
    /// version is not 6 or whose flags word has bit 2 is an error too:
    /// the original's rasterizer refuses it with a fatal error
    /// (`sprite-placement.md` Edge cases 4).
    pub fn from_dc6(dc6: &Dc6, dir: u8) -> Result<Self, FrameError> {
        let (version, flags) = (dc6.header.version, dc6.header.flags);
        if version != 6 || flags & 4 != 0 {
            return Err(FrameError::Dc6Header { version, flags });
        }
        let dirs = dc6.header.directions as usize;
        let per = dc6.header.frames_per_direction as usize;
        if usize::from(dir) >= dirs {
            return Err(FrameError::NoPart {
                part: FramePart::Dir(dir),
                count: dirs,
            });
        }
        let frames = (0..per)
            .map(|i| {
                let f = dc6
                    .frame(usize::from(dir), i)
                    .ok_or(FrameError::MissingFrame { dir, frame: i })?;
                let anchor = match f.flip {
                    0 => FrameAnchor::Bottom,
                    1 => FrameAnchor::TopDown,
                    flip => {
                        return Err(FrameError::Dc6Flip {
                            dir,
                            frame: i,
                            flip,
                        })
                    }
                };
                Ok(
                    IndexFrame::new(f.width, f.height, f.offset_x, f.offset_y, f.pixels.clone())?
                        .with_anchor(anchor),
                )
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { frames })
    }

    /// Tile `tile` of a DT1: frame 0 is its blocks assembled into one
    /// image by the `map-preview.md` rule (`crate::map::tiles::assemble`);
    /// frame [`DT1_BLOCK_FRAME0`] + `i` is block `i` alone (offsets the
    /// block's x, y), for the per-block draws of `render/shading.md` §4 r4
    /// (each block lights only its own pixels). No blocks: no frames.
    pub fn from_dt1(dt1: &Dt1, tile: u32) -> Result<Self, FrameError> {
        let t = dt1.tiles.get(tile as usize).ok_or(FrameError::NoPart {
            part: FramePart::Tile(tile),
            count: dt1.tiles.len(),
        })?;
        let Some(img) = crate::map::tiles::assemble(t) else {
            return Ok(Self { frames: Vec::new() });
        };
        let mut frames = vec![IndexFrame::new(
            img.width, img.height, img.x0, img.y0, img.pixels,
        )?];
        for b in &t.blocks {
            let (w, h) = b.size();
            frames.push(IndexFrame::new(
                w as u32,
                h as u32,
                i32::from(b.x),
                i32::from(b.y),
                b.pixels.clone(),
            )?);
        }
        Ok(Self { frames })
    }
}

/// A file parsed by `d2-formats`, ready to give frame sets.
#[derive(Debug, Clone, Copy)]
pub enum FrameSource<'a> {
    Dcc(&'a Dcc),
    Dc6(&'a Dc6),
    Dt1(&'a Dt1),
}

impl FrameSource<'_> {
    /// The frame set `part` of this file. A direction of a DT1 or a tile
    /// of a DCC/DC6 is an error (M07), never a guess.
    pub fn frame_set(&self, part: FramePart) -> Result<FrameSet, FrameError> {
        match (*self, part) {
            (FrameSource::Dcc(f), FramePart::Dir(d)) => FrameSet::from_dcc(f, d),
            (FrameSource::Dc6(f), FramePart::Dir(d)) => FrameSet::from_dc6(f, d),
            (FrameSource::Dt1(f), FramePart::Tile(t)) => FrameSet::from_dt1(f, t),
            (_, part) => Err(FrameError::WrongPartKind(part)),
        }
    }

    /// Number of parts (directions or tiles) the file has.
    pub fn part_count(&self) -> usize {
        match *self {
            FrameSource::Dcc(f) => f.directions.len(),
            FrameSource::Dc6(f) => f.header.directions as usize,
            FrameSource::Dt1(f) => f.tiles.len(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// The pixel buffer length is not `width × height`.
    #[error("frame {width}x{height} has {len} pixels")]
    PixelCount { width: u32, height: u32, len: usize },
    /// The requested direction or tile index is past the end of the file.
    #[error("{part} out of range: the file has {count}")]
    NoPart { part: FramePart, count: usize },
    /// A DC6 direction is missing one of its frames.
    #[error("DC6 direction {dir} has no frame {frame}")]
    MissingFrame { dir: u8, frame: usize },
    /// A DCC frame has an odd `variable0` (a top-down cel), which a frame
    /// box cannot express.
    #[error("DCC direction {dir} frame {frame}: variable0 {variable0} is odd (top-down cel)")]
    DccVariable0 {
        dir: u8,
        frame: usize,
        variable0: u32,
    },
    #[error("DC6 version {version}, flags {flags:#x}: requires version 6 and flags bit 2 clear")]
    Dc6Header { version: i32, flags: u32 },
    /// A DC6 frame's `flip` is neither 0 nor 1.
    #[error("DC6 direction {dir} frame {frame}: flip {flip} is neither 0 nor 1")]
    Dc6Flip { dir: u8, frame: usize, flip: u32 },
    /// A direction was asked of a DT1, or a tile of a DCC/DC6.
    #[error("{0} does not apply to this file type")]
    WrongPartKind(FramePart),
    /// The path is not in canonical form.
    #[error("path {0:?} is not canonical (lowercase ASCII, '/' separators, no leading '/')")]
    NonCanonicalPath(String),
}

impl fmt::Display for FramePart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FramePart::Dir(d) => write!(f, "direction {d}"),
            FramePart::Tile(t) => write!(f, "tile {t}"),
        }
    }
}

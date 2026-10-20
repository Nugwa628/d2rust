//! The front end's draw items as plain cels: every item of
//! [`crate::ui::front_end::DrawItem`] becomes one or more [`Cel`]s, an
//! index image (or a solid rectangle) at a top-left frame pixel with a draw
//! mode and an optional colour map. Both pictures of the front end are
//! drawn from the same cels:
//! - the 800 × 600 index frame on the CPU (`super::front_host`), pixel for
//!   pixel as before;
//! - the native UI pass at the window's resolution (`super::front_ui`),
//!   when the picture is shown enlarged.
//!
//! A text glyph's cel also names its font sheet and glyph ([`CelGlyph`]),
//! so the native pass can draw it from its enhanced bitmap when HD
//! Text is on. The CPU picture never reads that.

use std::hash::{Hash, Hasher};

use d2_formats::dc6::Dc6;
use d2_formats::font::FontTable;

/// The pixels of a cel.
#[derive(Debug, Clone, Copy)]
pub enum CelPixels<'a> {
    /// Palette indices, row-major, top row first; index 0 is transparent.
    Image { px: &'a [u8], w: u32, h: u32 },
    /// A `w × h` rectangle of palette index `index` (0 included: the
    /// dimming draws index 0 through an alpha table).
    Solid { w: u32, h: u32, index: u8 },
}

/// One cel of a front-end draw item.
#[derive(Debug, Clone, Copy)]
pub struct Cel<'a> {
    /// Names the image's pixels for the native pass's atlas: equal keys
    /// are equal pixels ([`cel_key`]). Unused for a solid.
    pub key: u64,
    pub pixels: CelPixels<'a>,
    /// The frame pixel of the top-left.
    pub x: i32,
    pub y: i32,
    /// The draw mode: 5 writes the index, 0–2 alpha, 3 additive,
    /// 4 multiplicative, 6 max-component (through the PL2 tables).
    pub mode: u8,
    /// Whether the draw mode goes through the PL2 tables; `false` writes
    /// the index whatever the mode (D2RUST's own boxes).
    pub tables: bool,
    /// The colour map each image index goes through first (text colours,
    /// paper-doll colours). Never applied to a solid.
    pub map: Option<[u8; 256]>,
    /// For a text glyph: its font sheet and glyph (HD Text).
    pub glyph: Option<CelGlyph<'a>>,
}

/// The font sheet and glyph a text cel draws.
#[derive(Debug, Clone, Copy)]
pub struct CelGlyph<'a> {
    /// The sheet's `.dc6` path as the archives name it (`font_info`).
    pub sheet: &'static str,
    /// The sheet's glyph table and cels.
    pub table: &'a FontTable,
    pub dc6: &'a Dc6,
    /// The cel of the sheet this glyph is.
    pub frame: u16,
}

impl Cel<'_> {
    /// The cel's width and height.
    pub fn size(&self) -> (u32, u32) {
        match self.pixels {
            CelPixels::Image { w, h, .. } | CelPixels::Solid { w, h, .. } => (w, h),
        }
    }
}

/// The atlas key of an image: its kind (`"dc6"`, `"ui"`, `"font"`,
/// `"dcc"`), its file or font, and its frame (and direction).
pub fn cel_key(kind: &str, name: &str, a: u32, b: u32) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (kind, name, a, b).hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_tell_kinds_files_and_frames_apart() {
        let k = cel_key("dc6", "x", 1, 0);
        assert_eq!(k, cel_key("dc6", "x", 1, 0));
        for other in [
            cel_key("ui", "x", 1, 0),
            cel_key("dc6", "y", 1, 0),
            cel_key("dc6", "x", 2, 0),
            cel_key("dc6", "x", 1, 1),
        ] {
            assert_ne!(k, other);
        }
    }
}

//! Which image draws a UI cel at the output resolution: the one lookup
//! point of the native UI pass (`gpu_compositor::ui_pass`) for its art.
//!
//! Every UI item stands for a cel of a DC6 file (a panel piece, a button,
//! a font glyph sheet's glyph), known by its file and frame number
//! ([`crate::frames::FrameStore::owner`]). The pass asks [`UiArt::cel`]
//! for each item, with the output scale, and draws the answer:
//! - [`CelImage::Original`]: the frame's own cel, sharp-scaled;
//! - [`CelImage::IndexedGlyph`]: an enhanced glyph with its palette indices;
//! - [`CelImage::Replacement`]: a higher-resolution RGBA image standing
//!   for that cel, with its own pixels per frame pixel.
//!
//! An HD asset pack plugs in here: a [`UiArt`] that maps (file, frame) to
//! an image of the pack, loaded at run time from a local folder of the
//! player's (never part of the game's files or this repository), and
//! falls back to the loaded cel for any cel the pack does not cover. The
//! replacement keeps the cel's place and size on screen, so layout and
//! the mouse are unchanged whatever the pack holds.
//!
//! Text is not a setting: the pass always draws a text glyph from a cached,
//! edge-aware enhancement of the loaded bitmap glyph ([`hd_text`]) at the
//! output scale, and everything else from its own cel. Game and front end
//! use the same cache through [`hd_text_art`].

use std::collections::BTreeMap;
use std::sync::Arc;

use d2_formats::font::FontTable;

use crate::assets::path::CanonicalPath;
use crate::frames::{FrameSetKey, FrameStore};
pub use crate::gpu_compositor::ui_pass::{CelImage, RepAtlas};

pub mod hd_text;

/// The art the native UI pass draws with: enhanced glyphs for text, the
/// cel itself for everything else.
pub fn current() -> &'static dyn UiArt {
    hd_text::art()
}

/// The glyph enhancer the pass draws text with. The front end's menus ask
/// it for their glyphs (`app::front_ui`).
pub fn hd_text_art() -> &'static hd_text::HdTextArt {
    hd_text::art()
}

/// One UI cel the pass asks about, with what an art may read to answer:
/// the cel's file and frame number (`None`: a cel the frame store does not
/// know by file, always drawn as itself), the frame store holding its
/// pixels, and the loaded font glyph tables (by `.tbl` path).
#[derive(Clone, Copy)]
pub struct CelRef<'a> {
    pub file: Option<(&'a FrameSetKey, usize)>,
    pub frames: &'a FrameStore,
    pub fonts: &'a BTreeMap<CanonicalPath, FontTable>,
}

/// The source of the UI's art at the output resolution.
pub trait UiArt: Send + Sync {
    /// The image that draws `cel` at `scale` output pixels per frame pixel.
    fn cel(&self, cel: &CelRef<'_>, scale: f32) -> CelImage;

    /// The replacement atlas the [`CelImage::Replacement`]s given so far
    /// are in; `None` when there are none.
    fn atlas(&self) -> Option<Arc<RepAtlas>> {
        None
    }
}

/// The game's own art: every cel is drawn as itself.
#[derive(Debug, Clone, Copy, Default)]
pub struct OriginalArt;

impl UiArt for OriginalArt {
    fn cel(&self, _cel: &CelRef<'_>, _scale: f32) -> CelImage {
        CelImage::Original
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::FramePart;

    #[test]
    fn the_original_art_draws_every_cel_as_itself() {
        let key = FrameSetKey::new("data/global/ui/panel/invchar6.dc6", FramePart::Dir(0))
            .expect("canonical path");
        let (frames, fonts) = (FrameStore::default(), BTreeMap::new());
        for file in [Some((&key, 2)), None] {
            let cel = CelRef {
                file,
                frames: &frames,
                fonts: &fonts,
            };
            assert_eq!(OriginalArt.cel(&cel, 3.6), CelImage::Original);
        }
        assert!(OriginalArt.atlas().is_none());
    }

    /// With no setting involved, the pass's art draws a font glyph from an
    /// enhanced image at the output scale: text goes through the display
    /// upscaler, not as the frame's own pixels.
    #[test]
    fn a_font_glyph_is_drawn_enhanced_at_the_output_scale_by_default() {
        use crate::frames::{FrameAnchor, FrameSet, IndexFrame};
        let key = FrameSetKey::new("data/local/font/latin/font16.dc6", FramePart::Dir(0))
            .expect("canonical path");
        let mut frames = FrameStore::default();
        frames
            .insert(
                key.clone(),
                FrameSet {
                    frames: vec![IndexFrame {
                        width: 3,
                        height: 3,
                        x_off: 0,
                        y_off: 2,
                        anchor: FrameAnchor::Bottom,
                        pixels: vec![0, 5, 0, 5, 9, 5, 0, 5, 0],
                    }],
                },
            )
            .expect("inserted");
        let fonts = BTreeMap::new();
        let cel = CelRef {
            file: Some((&key, 0)),
            frames: &frames,
            fonts: &fonts,
        };
        for scale in [1.8, 2.4, 3.6] {
            assert!(
                matches!(current().cel(&cel, scale), CelImage::IndexedGlyph { .. }),
                "a glyph at {scale}x must not be the frame's own cel"
            );
        }
        assert!(current().atlas().is_some());
    }

    #[test]
    fn a_cel_that_is_not_a_font_glyph_is_drawn_as_itself() {
        let key = FrameSetKey::new("data/global/ui/panel/invchar6.dc6", FramePart::Dir(0))
            .expect("canonical path");
        let (frames, fonts) = (FrameStore::default(), BTreeMap::new());
        for file in [Some((&key, 2)), None] {
            let cel = CelRef {
                file,
                frames: &frames,
                fonts: &fonts,
            };
            assert_eq!(current().cel(&cel, 3.6), CelImage::Original);
        }
    }
}

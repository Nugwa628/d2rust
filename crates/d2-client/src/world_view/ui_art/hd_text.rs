//! HD text from the player's loaded bitmap glyphs. Scale2x rounds diagonal
//! edges without inventing colours. Repeated passes produce an integer
//! prescale above the output scale; the UI pass smooths the
//! final edges. Indexed atlas texels go through the item's existing palette
//! maps before filtering, preserving colour codes and multicolour glyphs.
//! Only the glyph image changes: advances, baselines, clipping and layout
//! remain in frame pixels. Images are cached in memory for the active scale.

use super::{CelRef, UiArt};
use crate::frames::{FrameAnchor, FrameSetKey, IndexFrame};
use crate::gpu_compositor::ui_pass::{CelImage, RepAtlas, REP_PAGE};
use d2_formats::font::FontTable;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

pub const MAX_PAGES: usize = 8;

/// Edge-aware 2x expansion. Every output index comes from the input;
/// transparent index zero participates in edge detection like any colour.
fn scale2x(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut dst = vec![0; w * h * 4];
    let pixel = |x: usize, y: usize| src[y * w + x];
    for y in 0..h {
        for x in 0..w {
            let e = pixel(x, y);
            let b = pixel(x, y.saturating_sub(1));
            let d = pixel(x.saturating_sub(1), y);
            let f = pixel((x + 1).min(w - 1), y);
            let g = pixel(x, (y + 1).min(h - 1));
            let out = if b != g && d != f {
                [
                    if d == b { d } else { e },
                    if b == f { f } else { e },
                    if d == g { d } else { e },
                    if g == f { f } else { e },
                ]
            } else {
                [e; 4]
            };
            let at = y * 2 * w * 2 + x * 2;
            dst[at..at + 2].copy_from_slice(&out[..2]);
            dst[at + w * 2..at + w * 2 + 2].copy_from_slice(&out[2..]);
        }
    }
    dst
}

#[derive(Debug, Clone, Copy)]
pub struct GlyphCel<'a> {
    pub width: u32,
    pub height: u32,
    pub top: i32,
    pub pixels: &'a [u8],
}

impl<'a> GlyphCel<'a> {
    pub fn of_frame(frame: &'a IndexFrame) -> Self {
        let top = match frame.anchor {
            FrameAnchor::Bottom => frame.y_off - frame.height as i32 + 1,
            FrameAnchor::Top | FrameAnchor::TopDown => frame.y_off,
        };
        Self {
            width: frame.width,
            height: frame.height,
            top,
            pixels: &frame.pixels,
        }
    }
}

pub trait FontSheet {
    fn path(&self) -> &str;
    fn table(&self) -> Option<&FontTable>;
    fn cel(&self, index: usize) -> Option<GlyphCel<'_>>;
}

struct StoreSheet<'a, 'b> {
    key: &'b FrameSetKey,
    cel: &'b CelRef<'a>,
}
impl FontSheet for StoreSheet<'_, '_> {
    fn path(&self) -> &str {
        self.key.path()
    }
    fn table(&self) -> Option<&FontTable> {
        None
    }
    fn cel(&self, index: usize) -> Option<GlyphCel<'_>> {
        let frames = self.cel.frames;
        Some(GlyphCel::of_frame(
            frames.frame(frames.id(self.key, index).ok()?)?,
        ))
    }
}

fn is_font_sheet(path: &str) -> bool {
    path.ends_with(".dc6") && path.contains("/font/")
}

#[derive(Default)]
struct State {
    scale_bits: u32,
    cels: BTreeMap<String, BTreeMap<usize, CelImage>>,
    atlas: Arc<RepAtlas>,
    x: u32,
    y: u32,
    row_h: u32,
}

#[derive(Default)]
pub struct HdTextArt {
    state: Mutex<State>,
}

fn next_atlas_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl HdTextArt {
    pub fn new() -> Self {
        Self::default()
    }

    fn render(st: &mut State, cel: GlyphCel<'_>, scale: f32) -> Option<CelImage> {
        let (mut w, mut h) = (cel.width as usize, cel.height as usize);
        if w == 0
            || h == 0
            || cel.pixels.len() != w.checked_mul(h)?
            || !cel.pixels.iter().any(|&p| p != 0)
        {
            return None;
        }
        let mut pixels = cel.pixels.to_vec();
        let mut prescale = 1u32;
        while (prescale as f32) <= scale.max(1.0) {
            if w * 2 > REP_PAGE as usize || h * 2 > REP_PAGE as usize {
                return None;
            }
            pixels = scale2x(&pixels, w, h);
            w *= 2;
            h *= 2;
            prescale *= 2;
        }
        let (w, h) = (w as u32, h as u32);
        if st.x + w > REP_PAGE {
            st.x = 0;
            st.y += st.row_h;
            st.row_h = 0;
        }
        let atlas = Arc::make_mut(&mut st.atlas);
        if atlas.pages.is_empty() || st.y + h > REP_PAGE {
            if atlas.pages.len() >= MAX_PAGES {
                return None;
            }
            atlas.pages.push(RepAtlas::blank_page());
            st.x = 0;
            st.y = 0;
            st.row_h = 0;
        }
        let page = atlas.pages.len() - 1;
        let (x, y) = (st.x, st.y);
        let dst = &mut atlas.pages[page];
        for row in 0..h {
            for col in 0..w {
                let index = pixels[(row * w + col) as usize];
                let at = (((y + row) * REP_PAGE + x + col) * 4) as usize;
                dst[at..at + 4].copy_from_slice(&[index, 0, 0, if index == 0 { 0 } else { 255 }]);
            }
        }
        st.x += w;
        st.row_h = st.row_h.max(h);
        atlas.id = next_atlas_id();
        Some(CelImage::IndexedGlyph {
            page: page as u32,
            rect: [x, y, w, h],
            scale: prescale as f32,
        })
    }

    pub fn glyph(&self, sheet: &dyn FontSheet, index: usize, scale: f32) -> CelImage {
        if !is_font_sheet(sheet.path()) || !scale.is_finite() || scale <= 0.0 {
            return CelImage::Original;
        }
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.scale_bits != scale.to_bits() {
            *st = State {
                scale_bits: scale.to_bits(),
                ..State::default()
            };
        }
        if let Some(image) = st.cels.get(sheet.path()).and_then(|c| c.get(&index)) {
            return *image;
        }
        let Some(cel) = sheet.cel(index) else {
            return CelImage::Original;
        };
        let image = Self::render(&mut st, cel, scale).unwrap_or(CelImage::Original);
        st.cels
            .entry(sheet.path().to_owned())
            .or_default()
            .insert(index, image);
        image
    }

    pub fn glyph_atlas(&self) -> Option<Arc<RepAtlas>> {
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        (!st.atlas.pages.is_empty()).then(|| st.atlas.clone())
    }
}

/// The one glyph cache the game and the front end share.
pub fn art() -> &'static HdTextArt {
    static ART: OnceLock<HdTextArt> = OnceLock::new();
    ART.get_or_init(HdTextArt::new)
}

impl UiArt for HdTextArt {
    fn cel(&self, cel: &CelRef<'_>, scale: f32) -> CelImage {
        let Some((key, index)) = cel.file else {
            return CelImage::Original;
        };
        self.glyph(&StoreSheet { key, cel }, index, scale)
    }
    fn atlas(&self) -> Option<Arc<RepAtlas>> {
        self.glyph_atlas()
    }
}

#[cfg(test)]
#[path = "hd_text_tests.rs"]
pub(crate) mod tests;

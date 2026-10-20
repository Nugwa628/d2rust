//! The native UI pass: the game frame's UI items (panels, the HUD, text,
//! tooltips, item labels, chat, the automap, the cursor) drawn at the
//! output resolution on top of the scaled world, so their edges and their
//! text stay crisp at 1080p, 1440p and 2160p instead of being the frame's
//! own pixels enlarged.
//!
//! The game frame itself is composed exactly as without the pass, UI
//! included (`super::pack`): it is the next frame's base and what the frame
//! hashes read. Only what is presented changes: the scene (the frame
//! without its UI items) is scaled as the frame was, and this pass lays the
//! UI over it.
//!
//! Rules:
//! - **Placement.** Every item covers exactly the output pixels whose
//!   centres the presentation maps into its frame pixels
//!   (`ui::frame::first_pixel` on each edge, the inverse of the mouse's
//!   `Presentation::to_frame`). An item's edges therefore sit on output
//!   pixels, side-by-side items meet without a gap or an overlap, and a
//!   click lands on the item it lands on in the frame.
//! - **Scaling.** Inside an item every frame pixel is a solid block; only
//!   the output pixel where two of them meet is blended (the sharp rule of
//!   `world_view::sharp`, applied in the item's own pixels).
//! - **Colour.** A cel's index goes through the item's shade chain and the
//!   frame palette; index 0 is transparent.
//! - **Blending.** The pass computes what the UI does to the picture under
//!   it, per output pixel and channel: `out = under · keep + add`, in the
//!   palette's own units ([`UiBlend`]). Opaque art replaces, the alpha
//!   draw modes keep part of the picture, the additive mode adds and the
//!   multiplicative mode scales; composing items in order keeps that form,
//!   so the whole UI is two images that the presentation applies to the
//!   scaled scene. Max-component draws are shown as additive.
//! - **Art source.** Which image draws an item is asked at one point,
//!   [`CelImage`] (`world_view::ui_art`): the frame's own cel from the
//!   atlas, or a replacement image from the replacement atlas
//!   ([`RepAtlas`]: premultiplied RGBA at its own pixels per frame pixel,
//!   such as HD Text's glyphs). A replacement covers the same output
//!   pixels as the cel it stands for, clipped the same way, and is sampled
//!   bilinearly; a tinted one (a glyph's coverage) is coloured by one
//!   palette index through the item's shade chain, so text colours and
//!   their blends stay the game's.
//!
//! Buffers (WGSL bindings of group 0, `ui_pass.wgsl`):
//!
//! | Binding | Buffer | Layout |
//! |---|---|---|
//! | 0 | params (uniform) | 8 × u32: output width, height, bin cols, rows, item count, map rows, atlas pages, replacement pages |
//! | 1 | items | [`UI_ITEM_SIZE`] bytes per item, draw order ([`UiItem`]) |
//! | 2 | bin ranges | `cols × rows + 1` u32 prefix sums over 32 × 32 output-pixel bins |
//! | 3 | bin items | u32 item indices, bins concatenated, each in draw order |
//! | 4 | maps | the frame's map table (`super::pack`) |
//! | 5 | atlas | the frame atlas (`super::pack`) |
//! | 6 | palette | 256 u32: bytes r, g, b, 255 |
//! | 7 | cover (out) | Rgba8Unorm storage texture, output sized: `1 − keep` per channel |
//! | 8 | add (out) | Rgba8Unorm storage texture, output sized: `add` per channel |
//! | 9 | replacement atlas | Rgba8Unorm 2D array, [`REP_PAGE`] square layers, premultiplied ([`RepAtlas`]) |

use std::sync::{Arc, Mutex};

use d2_formats::palette::Palette;

use crate::frames::atlas::AtlasPage;
use crate::frames::PAGE_SIZE;
use crate::scene::{BlendOp, DrawItem, FrameId, FrameSource, MapTable, Rect};
use crate::ui::frame::first_pixel;

use super::pack::SlotSource;
use super::{GpuError, WORKGROUP};

/// The pass's compute shader (WGSL).
pub const UI_SHADER: &str = include_str!("ui_pass.wgsl");

/// Size of the params uniform in bytes.
pub const UI_PARAMS_SIZE: usize = 32;
/// Size of one packed item in bytes (seven 16-byte vectors).
pub const UI_ITEM_SIZE: usize = 112;
/// Side of a bin in output pixels.
pub const UI_BIN: u32 = 32;
/// [`UiItem::source`] of an item drawn from the frame atlas.
pub const SOURCE_ATLAS: u32 = 0;
/// [`UiItem::source`] of an item drawn from the replacement atlas.
pub const SOURCE_REPLACEMENT: u32 = 1;
/// [`UiItem::rep`] tint word: the image is coloured by a palette index.
pub const REP_TINTED: u32 = 0x100;
/// Replacement texels hold a palette index in red, and coverage in alpha.
pub const REP_INDEXED: u32 = 0x200;
/// Side of a replacement atlas page in pixels.
pub const REP_PAGE: u32 = 1024;

/// What one UI draw does to the picture under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiBlend {
    /// The art replaces the picture where it is drawn.
    Opaque,
    /// `under · (1 − a) + art · a`, `a` in 256ths (0..=256).
    Alpha(u16),
    /// `under + art`.
    Add,
    /// `under · art`.
    Mul,
}

impl UiBlend {
    /// The shader's kind and alpha words.
    pub fn words(self) -> (u32, u32) {
        match self {
            UiBlend::Opaque => (0, 0),
            UiBlend::Alpha(a) => (1, u32::from(a.min(256))),
            UiBlend::Add => (2, 0),
            UiBlend::Mul => (3, 0),
        }
    }
}

/// The kind of a blend table of the frame's map table, for turning an
/// item's index-table blend into a [`UiBlend`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableKind {
    /// An alpha level: the art's share, in 256ths, as measured from the
    /// table ([`fit_alpha`]).
    Alpha(u16),
    Add,
    Mul,
    /// Max-component: shown as additive.
    Max,
}

/// The [`UiBlend`] of an item's blend op. `kind` names the table a table
/// op reads (`None`: an unknown table, shown as half alpha). A table read
/// transposed ([`BlendOp::IndexTableSrcRow`]) swaps the roles of art and
/// picture, so an alpha level `a` becomes `1 − a`.
pub fn ui_blend(op: BlendOp, kind: impl Fn(crate::scene::MapId) -> Option<TableKind>) -> UiBlend {
    let (base, swapped) = match op {
        BlendOp::Opaque => return UiBlend::Opaque,
        BlendOp::IndexTable(b) => (b, false),
        BlendOp::IndexTableSrcRow(b) => (b, true),
    };
    match kind(base) {
        Some(TableKind::Alpha(a)) if swapped => UiBlend::Alpha(256 - a.min(256)),
        Some(TableKind::Alpha(a)) => UiBlend::Alpha(a.min(256)),
        Some(TableKind::Add | TableKind::Max) => UiBlend::Add,
        Some(TableKind::Mul) => UiBlend::Mul,
        None => UiBlend::Alpha(128),
    }
}

/// The art's share (256ths) of an alpha blend table (256 rows, row =
/// picture index, column = art index) under `palette`: the least-squares
/// fit of `out = under + a · (art − under)` over every pair and channel.
pub fn fit_alpha(rows: &[[u8; 256]], palette: &Palette) -> u16 {
    let c = |i: u8| palette.colors[usize::from(i)];
    let (mut num, mut den) = (0f64, 0f64);
    for (d, row) in rows.iter().enumerate().take(256) {
        let under = c(d as u8);
        for (s, &out) in row.iter().enumerate() {
            let art = c(s as u8);
            let out = c(out);
            for (u, a, o) in [
                (under.r, art.r, out.r),
                (under.g, art.g, out.g),
                (under.b, art.b, out.b),
            ] {
                let diff = f64::from(a) - f64::from(u);
                num += (f64::from(o) - f64::from(u)) * diff;
                den += diff * diff;
            }
        }
    }
    if den <= 0.0 {
        return 128;
    }
    (num / den * 256.0).round().clamp(0.0, 256.0) as u16
}

/// Where an item's art comes from at the output scale: the one lookup
/// point an HD asset pack plugs into (`world_view::ui_art`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CelImage {
    /// The frame's own cel from the atlas, sharp-scaled.
    Original,
    /// An enhanced bitmap glyph: red holds the palette index. The item's
    /// colour maps apply before filtering, exactly as for bitmap text.
    IndexedGlyph {
        page: u32,
        rect: [u32; 4],
        scale: f32,
    },
    /// A higher-resolution image standing for the cel, in the replacement
    /// atlas ([`RepAtlas`]): its page, its rectangle there (x, y, width,
    /// height), its pixels per frame pixel (its top-left is the cel's
    /// top-left) and, for a coverage image, the palette index that colours
    /// it (`tint`: the image's colour channels are multiplied by that
    /// index's colour after the item's shade chain).
    Replacement {
        page: u32,
        rect: [u32; 4],
        scale: f32,
        tint: Option<u8>,
    },
}

/// The replacement atlas: [`REP_PAGE`]-square pages of premultiplied RGBA
/// (4 bytes per pixel, rows top to bottom). Indexed glyphs instead store
/// their palette index in red and coverage in alpha. `id` names the contents: two
/// atlases with the same `id` hold the same pixels, so the GPU copy is
/// only made again when it changes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RepAtlas {
    pub id: u64,
    pub pages: Vec<Vec<u8>>,
}

impl RepAtlas {
    /// A new, empty page's bytes.
    pub fn blank_page() -> Vec<u8> {
        vec![0; (REP_PAGE * REP_PAGE * 4) as usize]
    }

    /// Pixel (`x`, `y`) of `page`, premultiplied 0..1; transparent outside.
    pub fn texel(&self, page: u32, x: i64, y: i64) -> [f32; 4] {
        let side = i64::from(REP_PAGE);
        let Some(p) = self.pages.get(page as usize) else {
            return [0.0; 4];
        };
        if !(0..side).contains(&x) || !(0..side).contains(&y) {
            return [0.0; 4];
        }
        let at = ((y * side + x) * 4) as usize;
        std::array::from_fn(|c| f32::from(p[at + c]) / 255.0)
    }
}

/// How the frame is presented: its size and the output image's
/// (`ui::frame::Presentation::image_w`, `image_h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiPlacement {
    pub frame_w: u32,
    pub frame_h: u32,
    pub out_w: u32,
    pub out_h: u32,
}

impl UiPlacement {
    /// The first output pixel of frame column `x` (may lie outside the
    /// output for a column outside the frame).
    pub fn x(&self, x: i32) -> i64 {
        first_pixel(x, self.out_w, self.frame_w)
    }

    /// The first output pixel of frame row `y`.
    pub fn y(&self, y: i32) -> i64 {
        first_pixel(y, self.out_h, self.frame_h)
    }

    /// Whether the output is larger than the frame on either axis: the
    /// pass only has something to add then.
    pub fn enlarges(&self) -> bool {
        self.out_w > self.frame_w || self.out_h > self.frame_h
    }
}

/// One UI item as the shader reads it (WGSL `Item`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UiItem {
    /// Output pixels drawn: x0, y0, x1, y1, ends exclusive.
    pub area: [u32; 4],
    /// Atlas texel of the image's top-left pixel.
    pub texel: [u32; 2],
    pub page: u32,
    pub shade_len: u32,
    /// The drawable part of the image, image pixels: x0, y0, x1, y1.
    pub src: [u32; 4],
    pub shade: [u32; 4],
    pub blend: u32,
    pub alpha: u32,
    pub source: u32,
    /// Output position of the image's top-left and output pixels per
    /// image pixel on each axis.
    pub place: [f32; 4],
    /// For a replacement ([`SOURCE_REPLACEMENT`]; `texel` and `page` are
    /// then its place in the replacement atlas): its width and height,
    /// its pixels per frame pixel (`f32` bits) and its tint
    /// ([`REP_TINTED`] | palette index, or 0).
    pub rep: [u32; 4],
}

impl UiItem {
    /// Draws the item from replacement `image` (its page, rectangle,
    /// scale and tint in the replacement atlas); an
    /// [`CelImage::Original`] leaves the item as it is.
    pub fn use_image(&mut self, image: CelImage) {
        let (page, rect, scale, flags) = match image {
            CelImage::Original => return,
            CelImage::Replacement {
                page,
                rect,
                scale,
                tint,
            } => (
                page,
                rect,
                scale,
                tint.map_or(0, |t| REP_TINTED | u32::from(t)),
            ),
            CelImage::IndexedGlyph { page, rect, scale } => (page, rect, scale, REP_INDEXED),
        };
        self.texel = [rect[0], rect[1]];
        self.page = page;
        self.source = SOURCE_REPLACEMENT;
        self.rep = [rect[2], rect[3], scale.to_bits(), flags];
    }

    pub fn to_le_bytes(&self) -> [u8; UI_ITEM_SIZE] {
        let a = self.area;
        let s = self.src;
        let h = self.shade;
        let words = [
            a[0],
            a[1],
            a[2],
            a[3],
            self.texel[0],
            self.texel[1],
            self.page,
            self.shade_len,
            s[0],
            s[1],
            s[2],
            s[3],
            h[0],
            h[1],
            h[2],
            h[3],
            self.blend,
            self.alpha,
            self.source,
            0,
            self.place[0].to_bits(),
            self.place[1].to_bits(),
            self.place[2].to_bits(),
            self.place[3].to_bits(),
            self.rep[0],
            self.rep[1],
            self.rep[2],
            self.rep[3],
        ];
        let mut out = [0u8; UI_ITEM_SIZE];
        for (chunk, w) in out.as_chunks_mut::<4>().0.iter_mut().zip(words) {
            chunk.copy_from_slice(&w.to_le_bytes());
        }
        out
    }

    pub fn from_le_bytes(bytes: &[u8; UI_ITEM_SIZE]) -> Self {
        let w = words(bytes);
        UiItem {
            area: [w[0], w[1], w[2], w[3]],
            texel: [w[4], w[5]],
            page: w[6],
            shade_len: w[7],
            src: [w[8], w[9], w[10], w[11]],
            shade: [w[12], w[13], w[14], w[15]],
            blend: w[16],
            alpha: w[17],
            source: w[18],
            place: [
                f32::from_bits(w[20]),
                f32::from_bits(w[21]),
                f32::from_bits(w[22]),
                f32::from_bits(w[23]),
            ],
            rep: [w[24], w[25], w[26], w[27]],
        }
    }
}

/// The params uniform (WGSL `Params`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UiParams {
    pub width: u32,
    pub height: u32,
    pub cols: u32,
    pub rows: u32,
    pub item_count: u32,
    pub map_rows: u32,
    pub pages: u32,
    /// Pages of the replacement atlas bound with the items.
    pub rep_pages: u32,
}

impl UiParams {
    pub fn to_le_bytes(&self) -> [u8; UI_PARAMS_SIZE] {
        let w = [
            self.width,
            self.height,
            self.cols,
            self.rows,
            self.item_count,
            self.map_rows,
            self.pages,
            self.rep_pages,
        ];
        let mut out = [0u8; UI_PARAMS_SIZE];
        for (chunk, w) in out.as_chunks_mut::<4>().0.iter_mut().zip(w) {
            chunk.copy_from_slice(&w.to_le_bytes());
        }
        out
    }
}

/// One UI draw for [`pack_ui`]: the frame's item, its blend and its art.
#[derive(Debug, Clone, Copy)]
pub struct UiDraw<'a> {
    pub item: &'a DrawItem,
    pub blend: UiBlend,
    pub image: CelImage,
}

/// The pass's inputs except the atlas, the maps and the palette, which are
/// the frame's.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UiPacked {
    pub params: UiParams,
    pub items: Vec<UiItem>,
    pub bin_ranges: Vec<u32>,
    pub bin_items: Vec<u32>,
    /// The replacement atlas the items' replacements are in (none: an item
    /// asking for one draws nothing).
    pub rep: Option<Arc<RepAtlas>>,
}

impl UiPacked {
    /// Binds the replacement atlas the items' replacements were placed in.
    pub fn set_rep(&mut self, rep: Option<Arc<RepAtlas>>) {
        self.params.rep_pages = rep.as_ref().map_or(0, |r| r.pages.len() as u32);
        self.rep = rep;
    }

    pub fn pixel_count(&self) -> usize {
        self.params.width as usize * self.params.height as usize
    }

    pub fn params_bytes(&self) -> [u8; UI_PARAMS_SIZE] {
        self.params.to_le_bytes()
    }

    pub fn items_bytes(&self) -> Vec<u8> {
        if self.items.is_empty() {
            return vec![0; UI_ITEM_SIZE];
        }
        self.items.iter().flat_map(|i| i.to_le_bytes()).collect()
    }

    pub fn bin_ranges_bytes(&self) -> Vec<u8> {
        self.bin_ranges
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect()
    }

    pub fn bin_items_bytes(&self) -> Vec<u8> {
        if self.bin_items.is_empty() {
            return vec![0; 4];
        }
        self.bin_items
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect()
    }
}

/// Packs the UI draws (in draw order) for the pass at `placement`. `view`
/// is the frame's view rectangle, `map_rows` and `pages` the frame's map
/// table length and atlas page count. An item that draws nothing on the
/// output is packed but put in no bin.
pub fn pack_ui<F, S>(
    draws: &[UiDraw<'_>],
    frames: &F,
    slots: &S,
    pages: u32,
    map_rows: u32,
    view: Rect,
    placement: UiPlacement,
) -> Result<UiPacked, GpuError>
where
    F: FrameSource + ?Sized,
    S: SlotSource + ?Sized,
{
    let mut items = Vec::with_capacity(draws.len());
    for (index, d) in draws.iter().enumerate() {
        items.push(pack_item(index, d, frames, slots, pages, view, placement)?);
    }
    Ok(bin_items(items, placement, map_rows, pages))
}

/// Puts packed items (in draw order) into the output's bins: the pass's
/// inputs at `placement` over `map_rows` map rows and `pages` atlas pages.
/// An item that draws nothing on the output is kept but put in no bin.
pub fn bin_items(
    items: Vec<UiItem>,
    placement: UiPlacement,
    map_rows: u32,
    pages: u32,
) -> UiPacked {
    let (w, h) = (placement.out_w, placement.out_h);
    let cols = w.div_ceil(UI_BIN).max(1);
    let rows = h.div_ceil(UI_BIN).max(1);
    let mut bins: Vec<Vec<u32>> = vec![Vec::new(); (cols * rows) as usize];
    for (index, item) in items.iter().enumerate() {
        let [x0, y0, x1, y1] = item.area;
        if x0 < x1 && y0 < y1 {
            for r in y0 / UI_BIN..y1.div_ceil(UI_BIN) {
                for c in x0 / UI_BIN..x1.div_ceil(UI_BIN) {
                    bins[(r * cols + c) as usize].push(index as u32);
                }
            }
        }
    }
    let mut bin_ranges = vec![0u32];
    let mut bin_items = Vec::new();
    for b in bins {
        bin_items.extend_from_slice(&b);
        bin_ranges.push(bin_items.len() as u32);
    }
    UiPacked {
        params: UiParams {
            width: w,
            height: h,
            cols,
            rows,
            item_count: items.len() as u32,
            map_rows,
            pages,
            rep_pages: 0,
        },
        items,
        bin_ranges,
        bin_items,
        rep: None,
    }
}

/// Where an image lands on the output: its top-left at frame pixel
/// (`ix`, `iy`), `width × height` image pixels, of which the frame pixels
/// `visible` (x0, y0, x1, y1, ends exclusive, inside the image) are drawn.
/// Returns the item's output `area` and `place` ([`UiItem`]).
pub fn place_image(
    p: UiPlacement,
    (ix, iy): (i64, i64),
    (width, height): (u32, u32),
    [ax0, ay0, ax1, ay1]: [i64; 4],
) -> ([u32; 4], [f32; 4]) {
    let to_i32 = |v: i64| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    let clamp_x = |v: i64| v.clamp(0, i64::from(p.out_w)) as u32;
    let clamp_y = |v: i64| v.clamp(0, i64::from(p.out_h)) as u32;
    let area = [
        clamp_x(p.x(to_i32(ax0))),
        clamp_y(p.y(to_i32(ay0))),
        clamp_x(p.x(to_i32(ax1))),
        clamp_y(p.y(to_i32(ay1))),
    ];
    let (ox0, ox1) = (p.x(to_i32(ix)), p.x(to_i32(ix + i64::from(width))));
    let (oy0, oy1) = (p.y(to_i32(iy)), p.y(to_i32(iy + i64::from(height))));
    let place = [
        ox0 as f32,
        oy0 as f32,
        (ox1 - ox0) as f32 / width.max(1) as f32,
        (oy1 - oy0) as f32 / height.max(1) as f32,
    ];
    (area, place)
}

fn pack_item<F, S>(
    index: usize,
    d: &UiDraw<'_>,
    frames: &F,
    slots: &S,
    pages: u32,
    view: Rect,
    p: UiPlacement,
) -> Result<UiItem, GpuError>
where
    F: FrameSource + ?Sized,
    S: SlotSource + ?Sized,
{
    let item = d.item;
    let frame = frames.frame(item.frame)?;
    let (width, height) = (frame.width(), frame.height());
    let (kind, alpha) = d.blend.words();
    let mut out = UiItem {
        shade_len: item.shade.maps().len() as u32,
        blend: kind,
        alpha,
        ..UiItem::default()
    };
    match d.image {
        CelImage::Original => {
            let slot = slot_of(index, item.frame, slots, pages, width, height)?;
            out.texel = [slot.x, slot.y];
            out.page = slot.page;
            out.source = SOURCE_ATLAS;
        }
        CelImage::Replacement { .. } | CelImage::IndexedGlyph { .. } => {
            out.use_image(d.image);
        }
    }
    for (dst, m) in out.shade.iter_mut().zip(item.shade.maps()) {
        *dst = m.0;
    }
    if width == 0 || height == 0 {
        return Ok(out);
    }
    // The same drawable area as the frame's compose, relative to the view.
    let image = Rect::new(item.x, item.y, width, height);
    let Some(area) = image.intersect(&item.clip).and_then(|r| r.intersect(&view)) else {
        return Ok(out);
    };
    // Frame pixels relative to the view.
    let (vx, vy) = (i64::from(view.x), i64::from(view.y));
    let (ix, iy) = (i64::from(item.x), i64::from(item.y));
    let (ax0, ay0) = (i64::from(area.x), i64::from(area.y));
    let (ax1, ay1) = (ax0 + i64::from(area.width), ay0 + i64::from(area.height));
    (out.area, out.place) = place_image(
        p,
        (ix - vx, iy - vy),
        (width, height),
        [ax0 - vx, ay0 - vy, ax1 - vx, ay1 - vy],
    );
    out.src = [
        (ax0 - ix) as u32,
        (ay0 - iy) as u32,
        (ax1 - ix) as u32,
        (ay1 - iy) as u32,
    ];
    Ok(out)
}

fn slot_of<S: SlotSource + ?Sized>(
    index: usize,
    frame: FrameId,
    slots: &S,
    pages: u32,
    width: u32,
    height: u32,
) -> Result<crate::frames::AtlasSlot, GpuError> {
    let slot = slots
        .slot(frame)
        .ok_or(GpuError::SlotMissing { index, frame })?;
    if (slot.w, slot.h) != (width, height) {
        return Err(GpuError::SlotSize {
            index,
            slot_w: slot.w,
            slot_h: slot.h,
            width,
            height,
        });
    }
    let fits = |at: u32, len: u32| u64::from(at) + u64::from(len) <= u64::from(PAGE_SIZE);
    if !slot.is_empty() && (slot.page >= pages || !fits(slot.x, slot.w) || !fits(slot.y, slot.h)) {
        return Err(GpuError::SlotPage {
            index,
            page: slot.page,
            pages,
        });
    }
    Ok(slot)
}

/// One output pixel of the pass: `1 − keep` and `add` per channel (0..1).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UiPixel {
    pub cover: [f32; 3],
    pub add: [f32; 3],
}

impl UiPixel {
    /// The two texels as the storage textures hold them (Rgba8Unorm,
    /// alpha 255).
    pub fn bytes(&self) -> ([u8; 4], [u8; 4]) {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        (
            [q(self.cover[0]), q(self.cover[1]), q(self.cover[2]), 255],
            [q(self.add[0]), q(self.add[1]), q(self.add[2]), 255],
        )
    }

    /// The pass applied to one picture colour (palette units 0..1).
    pub fn apply(&self, under: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|c| under[c] * (1.0 - self.cover[c]) + self.add[c])
    }
}

/// The sharp rule on one axis (the shader's `sharp_axis`): the left image
/// pixel to blend and the right one's weight.
pub fn sharp_axis(u: f32, s: f32) -> (f32, f32) {
    let scale = s.ceil().max(1.0);
    let base = u.floor();
    let d = u - base - 0.5;
    let solid = 0.5 - 0.5 / scale + crate::world_view::sharp::SOLID_SLACK;
    let p = base + (d - d.clamp(-solid, solid)) * scale + 0.5;
    let left = (p - 0.5).floor();
    (left, p - 0.5 - left)
}

/// Runs the shader's algorithm on the CPU over the packed items, for the
/// tests: the frame's map table, atlas pages and palette (the replacement
/// atlas is the packed one).
pub fn emulate_ui(
    packed: &UiPacked,
    pages: &[AtlasPage],
    maps: &MapTable,
    palette: &Palette,
) -> Vec<UiPixel> {
    let items: Vec<UiItem> = packed
        .items_bytes()
        .as_chunks::<UI_ITEM_SIZE>()
        .0
        .iter()
        .map(UiItem::from_le_bytes)
        .collect();
    let p = packed.params;
    let colour = |it: &UiItem, index: u8| -> [f32; 3] {
        let mut s = index;
        for &m in &it.shade[..it.shade_len as usize] {
            s = maps.rows()[m as usize][usize::from(s)];
        }
        let c = palette.colors[usize::from(s)];
        [
            f32::from(c.r) / 255.0,
            f32::from(c.g) / 255.0,
            f32::from(c.b) / 255.0,
        ]
    };
    let texel = |it: &UiItem, x: i32, y: i32| -> [f32; 4] {
        let [sx0, sy0, sx1, sy1] = it.src.map(|v| v as i32);
        if x < sx0 || y < sy0 || x >= sx1 || y >= sy1 {
            return [0.0; 4];
        }
        let (tx, ty) = (it.texel[0] + x as u32, it.texel[1] + y as u32);
        let page = &pages[it.page as usize].pixels;
        let index = page[(ty * PAGE_SIZE + tx) as usize];
        if index == 0 {
            return [0.0; 4];
        }
        let [r, g, b] = colour(it, index);
        [r, g, b, 1.0]
    };
    let mix = |a: [f32; 4], b: [f32; 4], t: f32| -> [f32; 4] {
        std::array::from_fn(|i| a[i] * (1.0 - t) + b[i] * t)
    };
    let rep_texel = |it: &UiItem, x: i32, y: i32| -> [f32; 4] {
        if x < 0 || y < 0 || x >= it.rep[0] as i32 || y >= it.rep[1] as i32 {
            return [0.0; 4];
        }
        let c = match (&packed.rep, it.page < p.rep_pages) {
            (Some(rep), true) => rep.texel(
                it.page,
                i64::from(it.texel[0]) + i64::from(x),
                i64::from(it.texel[1]) + i64::from(y),
            ),
            _ => [0.0; 4],
        };
        if it.rep[3] & REP_INDEXED != 0 {
            let rgb = colour(it, (c[0] * 255.0).round() as u8);
            [rgb[0] * c[3], rgb[1] * c[3], rgb[2] * c[3], c[3]]
        } else {
            c
        }
    };
    // A replacement at frame-image position (`u`, `v`): clipped to the
    // drawable part in frame pixels, sampled bilinearly in its own pixels.
    let rep_pixel = |it: &UiItem, u: f32, v: f32| -> [f32; 4] {
        let [sx0, sy0, sx1, sy1] = it.src.map(|v| v as f32);
        let (fu, fv) = (u.floor(), v.floor());
        if fu < sx0 || fv < sy0 || fu >= sx1 || fv >= sy1 {
            return [0.0; 4];
        }
        let s = f32::from_bits(it.rep[2]);
        let (rx, ry) = (u * s - 0.5, v * s - 0.5);
        let (bx, by) = (rx.floor(), ry.floor());
        let (wx, wy) = (rx - bx, ry - by);
        let (x, y) = (bx as i32, by as i32);
        let top = mix(rep_texel(it, x, y), rep_texel(it, x + 1, y), wx);
        let bottom = mix(rep_texel(it, x, y + 1), rep_texel(it, x + 1, y + 1), wx);
        let mut c = mix(top, bottom, wy);
        if it.rep[3] & REP_TINTED != 0 {
            let t = colour(it, (it.rep[3] & 0xff) as u8);
            for ch in 0..3 {
                c[ch] *= t[ch];
            }
        }
        c
    };
    let mut out = Vec::with_capacity(packed.pixel_count());
    for py in 0..p.height {
        for px in 0..p.width {
            let bin = ((py / UI_BIN) * p.cols + px / UI_BIN) as usize;
            let mut keep = [1f32; 3];
            let mut plus = [0f32; 3];
            for k in packed.bin_ranges[bin]..packed.bin_ranges[bin + 1] {
                let it = &items[packed.bin_items[k as usize] as usize];
                let [x0, y0, x1, y1] = it.area;
                if px < x0 || px >= x1 || py < y0 || py >= y1 {
                    continue;
                }
                let u = (px as f32 + 0.5 - it.place[0]) / it.place[2];
                let v = (py as f32 + 0.5 - it.place[1]) / it.place[3];
                let c = if it.source == SOURCE_REPLACEMENT {
                    rep_pixel(it, u, v)
                } else {
                    let (lx, wx) = sharp_axis(u, it.place[2]);
                    let (ly, wy) = sharp_axis(v, it.place[3]);
                    let (x, y) = (lx as i32, ly as i32);
                    let top = mix(texel(it, x, y), texel(it, x + 1, y), wx);
                    let bottom = mix(texel(it, x, y + 1), texel(it, x + 1, y + 1), wx);
                    mix(top, bottom, wy)
                };
                if c[3] <= 0.0 {
                    continue;
                }
                let rgb = [c[0], c[1], c[2]];
                let (m, a): ([f32; 3], [f32; 3]) = match it.blend {
                    1 => {
                        let alpha = it.alpha as f32 / 256.0;
                        ([1.0 - alpha * c[3]; 3], rgb.map(|v| v * alpha))
                    }
                    2 => ([1.0; 3], rgb),
                    3 => (rgb.map(|v| 1.0 - c[3] + v), [0.0; 3]),
                    _ => ([1.0 - c[3]; 3], rgb),
                };
                for ch in 0..3 {
                    keep[ch] *= m[ch];
                    plus[ch] = plus[ch] * m[ch] + a[ch];
                }
            }
            out.push(UiPixel {
                cover: keep.map(|k| (1.0 - k).clamp(0.0, 1.0)),
                add: plus.map(|v| v.clamp(0.0, 1.0)),
            });
        }
    }
    out
}

/// The pass's pipeline on a device, with the replacement atlas's GPU copy
/// (made again only when the atlas's `id` changes) and an empty stand-in
/// bound when there is none.
pub struct UiPass {
    pipeline: wgpu::ComputePipeline,
    no_rep: wgpu::Texture,
    rep: Mutex<Option<(u64, u32, wgpu::Texture)>>,
}

fn rep_texture(device: &wgpu::Device, side: u32, layers: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ui replacement atlas"),
        size: wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: layers.max(1),
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

impl UiPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("d2 native ui"),
            source: wgpu::ShaderSource::Wgsl(UI_SHADER.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("draw_ui"),
            layout: None,
            module: &module,
            entry_point: Some("draw_ui"),
            compilation_options: Default::default(),
            cache: None,
        });
        UiPass {
            pipeline,
            no_rep: rep_texture(device, 1, 1),
            rep: Mutex::new(None),
        }
    }

    /// The replacement atlas texture for `packed`: its GPU copy, made (and
    /// its pages copied in through `encoder`) when the atlas changed.
    fn rep_atlas(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        packed: &UiPacked,
    ) -> wgpu::Texture {
        use wgpu::util::DeviceExt;
        let Some(rep) = packed.rep.as_ref().filter(|r| !r.pages.is_empty()) else {
            return self.no_rep.clone();
        };
        let mut held = self.rep.lock().unwrap_or_else(|e| e.into_inner());
        let layers = rep.pages.len() as u32;
        if let Some((id, n, t)) = held.as_ref() {
            if *id == rep.id && *n == layers {
                return t.clone();
            }
        }
        let texture = rep_texture(device, REP_PAGE, layers);
        for (layer, page) in rep.pages.iter().enumerate() {
            let staging = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ui replacement page"),
                contents: page,
                usage: wgpu::BufferUsages::COPY_SRC,
            });
            encoder.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(REP_PAGE * 4),
                        rows_per_image: Some(REP_PAGE),
                    },
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: REP_PAGE,
                    height: REP_PAGE,
                    depth_or_array_layers: 1,
                },
            );
        }
        *held = Some((rep.id, layers, texture.clone()));
        texture
    }

    /// Records the pass into `encoder`: `packed` over the frame's atlas
    /// (`pages` layers), map table and palette, into the `cover` and `add`
    /// storage textures (Rgba8Unorm, `packed`'s output size). Nothing is
    /// submitted.
    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        packed: &UiPacked,
        atlas: &wgpu::Texture,
        maps: &wgpu::Buffer,
        palette: &wgpu::Buffer,
        cover: &wgpu::TextureView,
        add: &wgpu::TextureView,
    ) -> Result<(), GpuError> {
        use wgpu::util::DeviceExt;
        if packed.pixel_count() == 0 {
            return Ok(());
        }
        let limits = device.limits();
        let needed = u64::from(
            packed
                .params
                .width
                .max(packed.params.height)
                .div_ceil(WORKGROUP),
        );
        if needed > u64::from(limits.max_compute_workgroups_per_dimension) {
            return Err(GpuError::Limit {
                what: "workgroups per dimension",
                needed,
                limit: u64::from(limits.max_compute_workgroups_per_dimension),
            });
        }
        let storage = wgpu::BufferUsages::STORAGE;
        let init = |label, contents: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage,
            })
        };
        let params = init(
            "ui params",
            &packed.params_bytes(),
            wgpu::BufferUsages::UNIFORM,
        );
        let items = init("ui items", &packed.items_bytes(), storage);
        let ranges = init("ui bin ranges", &packed.bin_ranges_bytes(), storage);
        let bin_items = init("ui bin items", &packed.bin_items_bytes(), storage);
        let atlas_view = atlas.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let rep_view =
            self.rep_atlas(device, encoder, packed)
                .create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2Array),
                    ..Default::default()
                });
        fn entry(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry { binding, resource }
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("draw_ui"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                entry(0, params.as_entire_binding()),
                entry(1, items.as_entire_binding()),
                entry(2, ranges.as_entire_binding()),
                entry(3, bin_items.as_entire_binding()),
                entry(4, maps.as_entire_binding()),
                entry(5, wgpu::BindingResource::TextureView(&atlas_view)),
                entry(6, palette.as_entire_binding()),
                entry(7, wgpu::BindingResource::TextureView(cover)),
                entry(8, wgpu::BindingResource::TextureView(add)),
                entry(9, wgpu::BindingResource::TextureView(&rep_view)),
            ],
        });
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(
            packed.params.width.div_ceil(WORKGROUP),
            packed.params.height.div_ceil(WORKGROUP),
            1,
        );
        Ok(())
    }
}

fn words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes(*c))
        .collect()
}

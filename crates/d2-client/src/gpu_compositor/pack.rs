//! Packing the draw list for the GPU compositor: plain Rust, explicit
//! little-endian. The compositor draws the game frame (world and screen
//! UI) into the persistent index framebuffer; everything it reads is
//! packed here, and [`emulate`] runs its per-pixel algorithm on the CPU so
//! the layout is checked in CI without a GPU.
//!
//! Buffers (WGSL bindings of group 0, `compositor.wgsl`):
//!
//! | Binding | Buffer | Layout |
//! |---|---|---|
//! | 0 | params (uniform) | 12 × u32: view width, height, bin cols, rows, item count, map rows, atlas pages, clear rows, clear after (0/1), native UI (0/1), base layout ([`BASE_WIDE_WORD`]), 0 |
//! | 1 | items | [`ITEM_SIZE`] bytes per item, list order ([`GpuItem`]) |
//! | 2 | bin ranges | `cols × rows + 1` u32 prefix sums: bin `b` (row-major) owns `bin_items[ranges[b]..ranges[b+1]]` |
//! | 3 | bin items | u32 item indices, bins concatenated, each in list order |
//! | 4 | maps | map table rows of 256 bytes, unchanged: byte `b` of a row is bits `8·(b%4)` of word `b/4` (little-endian) |
//! | 5 | atlas | R8Uint 2048×2048 texture array, layer = atlas page |
//! | 6 | indices (out) | one u32 per view pixel, row-major, value 0..=255 |
//! | 7 | palette | 256 u32: bytes r, g, b, 255 |
//! | 8 | rgba (out) | one u32 per view pixel: bytes r, g, b, a (with the native UI pass: the scene, the frame without its UI items) |
//! | 9 | base | the frame's start framebuffer, one byte per view pixel, row-major, four per word (little-endian), zero-padded to a whole word; or, with the base layout word 1, the previous frame's index output itself (one u32 per pixel, as binding 6) |
//! | 10 | layers (out) | one u32 per view pixel: the world layer's index in bits 0–7, bit 8 set while a screen- or UI-layer item contributes to the pixel, the scene's index (the frame without UI items) in bits 16–23, bit 24 set while a screen-layer item contributes to the scene ([`LayerPixel`]) |
//! | 11 | world rgba (out) | one u32 per view pixel: the world layer through the palette |
//! | 12 | screen rgba (out) | one u32 per view pixel: the frame's colour where the screen layer still contributes, else all zero (transparent); with the native UI pass the scene's colour where its screen part still contributes |
//!
//! The layer outputs serve smooth scrolling between ticks
//! (`world_view::smooth`): the world layer can be shown shifted by a
//! sub-tick camera offset while the screen layer (UI, the local player)
//! stays put. They never change the index framebuffer, which stays the
//! exact tick frame.
//!
//! The native UI pass (`super::ui_pass`) draws the UI items at the
//! output resolution instead of in the frame's own pixels. With it on
//! ([`Params::native_ui`]) the presented RGBA and the screen layer leave
//! the UI items out (the scene); the index framebuffer still holds them,
//! so the frame chain and the frame hashes never depend on the pass.
//!
//! Storage buffers cannot be empty, so an empty item list, bin-item list or
//! map table is padded with one zeroed record that nothing references.

use crate::frames::atlas::AtlasPage;
use crate::frames::{Atlas, AtlasSlot, IndexFrame, PAGE_SIZE};
use crate::scene::{
    self, Bins, BlendOp, DrawItem, FrameId, FrameImage, FramePlan, FrameSource, MapTable, Rect,
    SceneError, BIN_SIZE,
};

/// The params word that says how the base is laid out: 0 (what [`Packed`]
/// holds) four pixels per word; 1 one pixel per word, when the device binds
/// the previous frame's index output on the GPU as the base instead
/// (`Gpu::frame_inputs_over`). Packed params always hold 0 here.
pub const BASE_WIDE_WORD: usize = 10;

use super::GpuError;

/// Size of the params uniform in bytes.
pub const PARAMS_SIZE: usize = 48;
/// Size of one packed item in bytes (five `vec4<u32>`).
pub const ITEM_SIZE: usize = 80;
/// Bytes per map-table row.
pub const MAP_ROW_SIZE: usize = 256;
/// [`GpuItem::blend`] of [`BlendOp::Opaque`].
pub const BLEND_OPAQUE: u32 = 0;
/// [`GpuItem::blend`] of [`BlendOp::IndexTable`].
pub const BLEND_INDEX_TABLE: u32 = 1;
/// [`GpuItem::blend`] of [`BlendOp::IndexTableSrcRow`].
pub const BLEND_INDEX_TABLE_SRC_ROW: u32 = 2;
/// [`GpuItem::layer`] of an item that moves with the world (map tiles,
/// other units, missiles): the default.
pub const LAYER_WORLD: u32 = 0;
/// [`GpuItem::layer`] of an item fixed on screen while the camera blends
/// between ticks: the local player the camera follows, the screen fade.
pub const LAYER_SCREEN: u32 = 1;
/// [`GpuItem::layer`] of a UI item: on the screen layer like
/// [`LAYER_SCREEN`], and left out of the presented scene when the native
/// UI pass draws it.
pub const LAYER_UI: u32 = 2;

/// Where the GPU finds a frame: the atlas slot of each [`FrameId`] (C3's
/// atlas owns the numbering of slots, C4's frame source the ids).
pub trait SlotSource {
    fn slot(&self, id: FrameId) -> Option<AtlasSlot>;
}

/// A slice of slots: `FrameId(n)` is element `n` (as `[FrameImage]` is a
/// frame source).
impl SlotSource for [AtlasSlot] {
    fn slot(&self, id: FrameId) -> Option<AtlasSlot> {
        self.get(id.0 as usize).copied()
    }
}

impl SlotSource for Vec<AtlasSlot> {
    fn slot(&self, id: FrameId) -> Option<AtlasSlot> {
        self.as_slice().slot(id)
    }
}

/// A slice of slots that may be missing: `FrameId(n)` is element `n`,
/// `None` while the frame is not in the atlas (the in-app atlas places
/// frames as draw lists need them).
impl SlotSource for [Option<AtlasSlot>] {
    fn slot(&self, id: FrameId) -> Option<AtlasSlot> {
        self.get(id.0 as usize).copied().flatten()
    }
}

/// One draw item as the shader reads it (WGSL `Item`, 5 × `vec4<u32>`).
/// Coordinates are relative to the view's top-left, so every field is a
/// non-negative integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GpuItem {
    /// Drawable area (image ∩ clip ∩ view): x0, y0, x1, y1, ends exclusive.
    /// All zero when the item draws nothing.
    pub area: [u32; 4],
    /// Atlas texel of the area's top-left pixel.
    pub texel: [u32; 2],
    /// Atlas page (texture array layer).
    pub page: u32,
    /// Number of used shade slots, 0..=4.
    pub shade_len: u32,
    /// Shade chain map rows; unused slots 0.
    pub shade: [u32; 4],
    /// [`BLEND_OPAQUE`], [`BLEND_INDEX_TABLE`] or
    /// [`BLEND_INDEX_TABLE_SRC_ROW`].
    pub blend: u32,
    /// First row of the blend table (both table ops), else 0.
    pub blend_base: u32,
    /// Light gradient ([`scene::LightGradient`]): kind code
    /// ([`scene::GradientKind::code`], 0 = none), map row of light map 0,
    /// corners `c0 | c1 << 8 | c2 << 16 | c3 << 24`, and the block column
    /// and row of the area's top-left pixel `dx | dy << 16` (both 0…31).
    pub light: [u32; 4],
    /// [`LAYER_WORLD`], [`LAYER_SCREEN`] or [`LAYER_UI`]: which
    /// presentation layer the item belongs to. It never changes the
    /// composed frame, only the layer outputs.
    pub layer: u32,
}

impl GpuItem {
    fn words(&self) -> [u32; ITEM_SIZE / 4] {
        let a = self.area;
        let s = self.shade;
        [
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
            self.blend,
            self.blend_base,
            self.layer,
            0,
            self.light[0],
            self.light[1],
            self.light[2],
            self.light[3],
        ]
    }

    pub fn to_le_bytes(&self) -> [u8; ITEM_SIZE] {
        let mut out = [0u8; ITEM_SIZE];
        for (chunk, w) in out.as_chunks_mut::<4>().0.iter_mut().zip(self.words()) {
            chunk.copy_from_slice(&w.to_le_bytes());
        }
        out
    }

    pub fn from_le_bytes(bytes: &[u8; ITEM_SIZE]) -> Self {
        let w = words(bytes);
        GpuItem {
            area: [w[0], w[1], w[2], w[3]],
            texel: [w[4], w[5]],
            page: w[6],
            shade_len: w[7],
            shade: [w[8], w[9], w[10], w[11]],
            blend: w[12],
            blend_base: w[13],
            layer: w[14],
            light: [w[16], w[17], w[18], w[19]],
        }
    }
}

/// The params uniform (WGSL `Params`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Params {
    pub width: u32,
    pub height: u32,
    pub cols: u32,
    pub rows: u32,
    pub item_count: u32,
    pub map_rows: u32,
    pub pages: u32,
    /// [`FramePlan::clear_rows`].
    pub clear_rows: u32,
    /// [`FramePlan::clear_after`] as 0 / 1.
    pub clear_after: u32,
    /// 1: the UI items are presented by the native UI pass, so the
    /// presented RGBA and the screen layer leave them out.
    pub native_ui: u32,
}

impl Params {
    pub fn to_le_bytes(&self) -> [u8; PARAMS_SIZE] {
        let w = [
            self.width,
            self.height,
            self.cols,
            self.rows,
            self.item_count,
            self.map_rows,
            self.pages,
            self.clear_rows,
            self.clear_after,
            self.native_ui,
            0,
            0,
        ];
        let mut out = [0u8; PARAMS_SIZE];
        for (chunk, w) in out.as_chunks_mut::<4>().0.iter_mut().zip(w) {
            chunk.copy_from_slice(&w.to_le_bytes());
        }
        out
    }

    pub fn from_le_bytes(bytes: &[u8; PARAMS_SIZE]) -> Self {
        let w = words(bytes);
        Params {
            width: w[0],
            height: w[1],
            cols: w[2],
            rows: w[3],
            item_count: w[4],
            map_rows: w[5],
            pages: w[6],
            clear_rows: w[7],
            clear_after: w[8],
            native_ui: w[9],
        }
    }
}

/// Everything the shader reads except the atlas pages, which stay with the
/// atlas (C3) and are bound as they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packed {
    pub view: Rect,
    pub params: Params,
    pub items: Vec<GpuItem>,
    pub bin_ranges: Vec<u32>,
    pub bin_items: Vec<u32>,
    /// Map table rows, concatenated.
    pub maps: Vec<u8>,
    /// The frame's start framebuffer, one byte per view pixel (all 0 from
    /// [`pack`]; set by [`Packed::with_frame`]).
    pub base: Vec<u8>,
}

impl Packed {
    /// Pixels of the view (= length of the index framebuffer).
    pub fn pixel_count(&self) -> usize {
        self.params.width as usize * self.params.height as usize
    }

    pub fn params_bytes(&self) -> [u8; PARAMS_SIZE] {
        self.params.to_le_bytes()
    }

    pub fn items_bytes(&self) -> Vec<u8> {
        if self.items.is_empty() {
            return vec![0; ITEM_SIZE];
        }
        self.items.iter().flat_map(|i| i.to_le_bytes()).collect()
    }

    pub fn bin_ranges_bytes(&self) -> Vec<u8> {
        u32s_le(&self.bin_ranges)
    }

    pub fn bin_items_bytes(&self) -> Vec<u8> {
        if self.bin_items.is_empty() {
            return vec![0; 4];
        }
        u32s_le(&self.bin_items)
    }

    /// The base, zero-padded to a whole word (at least one).
    pub fn base_bytes(&self) -> Vec<u8> {
        let mut out = self.base.clone();
        out.resize(self.base.len().div_ceil(4).max(1) * 4, 0);
        out
    }

    /// The clears of the frame, as packed.
    pub fn plan(&self) -> FramePlan {
        FramePlan {
            clear_rows: self.params.clear_rows,
            clear_after: self.params.clear_after != 0,
        }
    }

    /// One frame of the frame cycle (`composition.md` §3) instead of a
    /// single frame from 0: `base` is the previous frame (view sized) and
    /// `plan` its clears, the same inputs as `scene::compose_frame`.
    pub fn with_frame(mut self, base: &[u8], plan: FramePlan) -> Result<Self, GpuError> {
        if base.len() != self.pixel_count() {
            return Err(SceneError::BaseSize {
                len: base.len(),
                pixels: self.pixel_count() as u64,
            }
            .into());
        }
        if plan.clear_rows > self.params.height {
            return Err(SceneError::FramePlan(plan).into());
        }
        self.base = base.to_vec();
        self.params.clear_rows = plan.clear_rows;
        self.params.clear_after = u32::from(plan.clear_after);
        Ok(self)
    }

    /// Another draw list over the same frame: `items` binned by `bins`
    /// (over this frame's view), with this frame's size, map rows, page
    /// count and clears. The map rows and the base are left empty: the
    /// compositor reads them from the frame's own upload
    /// ([`super::device::FrameInputs`]), so only the items and bins move.
    /// Used for the display frames between ticks (`world_view::unit_motion`).
    pub fn with_items<F, S>(
        &self,
        items: &[DrawItem],
        bins: &Bins,
        frames: &F,
        slots: &S,
    ) -> Result<Packed, GpuError>
    where
        F: FrameSource + ?Sized,
        S: SlotSource + ?Sized,
    {
        if bins.view() != self.view {
            return Err(GpuError::BinsMismatch);
        }
        let (gpu_items, bin_ranges, bin_items) =
            pack_lists(items, bins, frames, slots, self.params.pages)?;
        Ok(Packed {
            view: self.view,
            params: Params {
                item_count: items.len() as u32,
                ..self.params
            },
            items: gpu_items,
            bin_ranges,
            bin_items,
            maps: Vec::new(),
            base: Vec::new(),
        })
    }

    /// Puts every item for which `screen(index)` holds (index in list
    /// order) on the screen layer, the others on the world layer.
    pub fn set_screen_layer(&mut self, screen: impl Fn(usize) -> bool) {
        self.set_layers(|i| if screen(i) { LAYER_SCREEN } else { LAYER_WORLD });
    }

    /// Gives every item the layer `layer(index)` (index in list order):
    /// [`LAYER_WORLD`], [`LAYER_SCREEN`] or [`LAYER_UI`].
    pub fn set_layers(&mut self, layer: impl Fn(usize) -> u32) {
        for (i, item) in self.items.iter_mut().enumerate() {
            item.layer = layer(i);
        }
    }

    pub fn maps_bytes(&self) -> Vec<u8> {
        if self.maps.is_empty() {
            return vec![0; MAP_ROW_SIZE];
        }
        self.maps.clone()
    }
}

/// Packs `items` (in draw order) and their `bins` for the shader.
///
/// Validation is the CPU compositor's: the bins are rebuilt with
/// [`scene::bin`] (which checks every item as [`scene::compose`] does) and
/// must equal `bins`. On top, every item's frame needs a slot of the
/// frame's size inside one of `pages` atlas pages. The slot's bytes are not
/// compared here: a packing error must show as a verify mismatch (§A2).
pub fn pack<F, S>(
    items: &[DrawItem],
    bins: &Bins,
    frames: &F,
    slots: &S,
    maps: &MapTable,
    pages: u32,
) -> Result<Packed, GpuError>
where
    F: FrameSource + ?Sized,
    S: SlotSource + ?Sized,
{
    let view = bins.view();
    if scene::bin(items, frames, maps, view)? != *bins {
        return Err(GpuError::BinsMismatch);
    }
    let (gpu_items, bin_ranges, bin_items) = pack_lists(items, bins, frames, slots, pages)?;
    let params = Params {
        width: view.width,
        height: view.height,
        cols: bins.cols(),
        rows: bins.rows(),
        item_count: items.len() as u32,
        map_rows: maps.len() as u32,
        pages,
        clear_rows: 0,
        clear_after: 0,
        native_ui: 0,
    };
    Ok(Packed {
        view,
        params,
        items: gpu_items,
        bin_ranges,
        bin_items,
        maps: maps.rows().iter().flatten().copied().collect(),
        base: vec![0; view.width as usize * view.height as usize],
    })
}

/// The packed items and the bin lists: `(items, bin ranges, bin items)`.
type Lists = (Vec<GpuItem>, Vec<u32>, Vec<u32>);

/// Packs `items` and flattens `bins` (row by row) into the compositor's
/// lists.
fn pack_lists<F, S>(
    items: &[DrawItem],
    bins: &Bins,
    frames: &F,
    slots: &S,
    pages: u32,
) -> Result<Lists, GpuError>
where
    F: FrameSource + ?Sized,
    S: SlotSource + ?Sized,
{
    let view = bins.view();
    let mut gpu_items = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        gpu_items.push(pack_item(index, item, frames, slots, pages, &view)?);
    }
    let mut bin_ranges = vec![0u32];
    let mut bin_items = Vec::new();
    for r in 0..bins.rows() {
        for c in 0..bins.cols() {
            bin_items.extend_from_slice(bins.list(c, r));
            bin_ranges.push(bin_items.len() as u32);
        }
    }
    Ok((gpu_items, bin_ranges, bin_items))
}

fn pack_item<F, S>(
    index: usize,
    item: &DrawItem,
    frames: &F,
    slots: &S,
    pages: u32,
    view: &Rect,
) -> Result<GpuItem, GpuError>
where
    F: FrameSource + ?Sized,
    S: SlotSource + ?Sized,
{
    let frame = frames.frame(item.frame)?;
    let (width, height) = (frame.width(), frame.height());
    let slot = slots.slot(item.frame).ok_or(GpuError::SlotMissing {
        index,
        frame: item.frame,
    })?;
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
    let mut out = GpuItem {
        page: slot.page,
        ..GpuItem::default()
    };
    for (dst, m) in out.shade.iter_mut().zip(item.shade.maps()) {
        *dst = m.0;
    }
    out.shade_len = item.shade.maps().len() as u32;
    match item.blend {
        BlendOp::Opaque => out.blend = BLEND_OPAQUE,
        BlendOp::IndexTable(base) => {
            out.blend = BLEND_INDEX_TABLE;
            out.blend_base = base.0;
        }
        BlendOp::IndexTableSrcRow(base) => {
            out.blend = BLEND_INDEX_TABLE_SRC_ROW;
            out.blend_base = base.0;
        }
    }
    // The same area as the CPU's `DrawItem::resolve`.
    let image = Rect::new(item.x, item.y, width, height);
    if let Some(area) = image.intersect(&item.clip).and_then(|r| r.intersect(view)) {
        // `area` lies inside both the view and the image, so every
        // difference below is in 0..view or 0..frame size.
        let vx = (i64::from(area.x) - i64::from(view.x)) as u32;
        let vy = (i64::from(area.y) - i64::from(view.y)) as u32;
        out.area = [vx, vy, vx + area.width, vy + area.height];
        out.texel = [
            slot.x + (i64::from(area.x) - i64::from(item.x)) as u32,
            slot.y + (i64::from(area.y) - i64::from(item.y)) as u32,
        ];
        if let Some(g) = item.shade.gradient() {
            // `scene::bin` checked the area lies inside the block.
            let dx = (i64::from(area.x) - i64::from(g.x)) as u32;
            let dy = (i64::from(area.y) - i64::from(g.y)) as u32;
            let [c0, c1, c2, c3] = g.corners.map(u32::from);
            out.light = [
                g.kind.code(),
                g.light0.0,
                c0 | c1 << 8 | c2 << 16 | c3 << 24,
                dx | dy << 16,
            ];
        }
    }
    Ok(out)
}

/// Runs the shader's `compose` algorithm on the packed *bytes* (decoded
/// with [`GpuItem::from_le_bytes`] and friends), on the CPU. Equal to
/// [`scene::compose`] when packing is right; CI checks that, so the GPU's
/// inputs are proven without a GPU. `pages` are the atlas pages.
pub fn emulate(packed: &Packed, pages: &[AtlasPage]) -> Result<Vec<u8>, GpuError> {
    Ok(emulate_layers(packed, pages)?
        .iter()
        .map(|p| p.index)
        .collect())
}

/// One pixel of the compositor's outputs: the frame's index, the world
/// layer's index, whether the pixel still depends on the screen layer
/// (screen or UI items), the scene's index (the frame without UI items)
/// and whether the scene still depends on a screen item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayerPixel {
    pub index: u8,
    pub world: u8,
    pub screen: bool,
    pub scene: u8,
    pub scene_screen: bool,
}

impl LayerPixel {
    /// The shader's `layers` word.
    pub fn word(self) -> u32 {
        u32::from(self.world)
            | u32::from(self.screen) << 8
            | u32::from(self.scene) << 16
            | u32::from(self.scene_screen) << 24
    }

    /// The index the presented image shows: the frame, or with the native
    /// UI pass the scene.
    pub fn shown(self, native_ui: bool) -> u8 {
        if native_ui {
            self.scene
        } else {
            self.index
        }
    }

    /// Whether the screen layer shows this pixel (else it is transparent).
    pub fn on_screen_layer(self, native_ui: bool) -> bool {
        if native_ui {
            self.scene_screen
        } else {
            self.screen
        }
    }
}

/// [`emulate`] with the layer outputs, as the shader computes them: the
/// frame's index is the normal compose; the world layer composes only the
/// world items over the same base (a blend reads the world layer's own
/// value); translucent blends preserve the screen contribution until an
/// opaque world item covers it.
pub fn emulate_layers(packed: &Packed, pages: &[AtlasPage]) -> Result<Vec<LayerPixel>, GpuError> {
    if pages.len() != packed.params.pages as usize {
        return Err(GpuError::PageCount {
            pages: pages.len(),
            packed: packed.params.pages,
        });
    }
    let params = Params::from_le_bytes(&packed.params_bytes());
    let items: Vec<GpuItem> = packed
        .items_bytes()
        .as_chunks::<ITEM_SIZE>()
        .0
        .iter()
        .map(GpuItem::from_le_bytes)
        .collect();
    let ranges = words(&packed.bin_ranges_bytes());
    let bin_items = words(&packed.bin_items_bytes());
    let maps = words(&packed.maps_bytes());
    let base = words(&packed.base_bytes());
    let map_byte = |row: u32, b: u32| (maps[(row * 64 + b / 4) as usize] >> ((b % 4) * 8)) & 0xFF;
    let base_byte = |i: u32| (base[(i / 4) as usize] >> ((i % 4) * 8)) & 0xFF;
    let mut out = Vec::with_capacity(packed.pixel_count());
    for py in 0..params.height {
        for px in 0..params.width {
            let bin = (py / BIN_SIZE) * params.cols + px / BIN_SIZE;
            let mut value = if py >= params.clear_rows {
                base_byte(py * params.width + px)
            } else {
                0
            };
            let mut world = value;
            let mut screen = false;
            let mut scene = value;
            let mut scene_screen = false;
            for k in ranges[bin as usize]..ranges[bin as usize + 1] {
                let it = &items[bin_items[k as usize] as usize];
                let [x0, y0, x1, y1] = it.area;
                if px < x0 || px >= x1 || py < y0 || py >= y1 {
                    continue;
                }
                let tx = it.texel[0] + (px - x0);
                let ty = it.texel[1] + (py - y0);
                let page = &pages[it.page as usize].pixels;
                let src = u32::from(page[(ty * PAGE_SIZE + tx) as usize]);
                if src == 0 {
                    continue;
                }
                let mut s = src;
                for &m in &it.shade[..it.shade_len as usize] {
                    s = map_byte(m, s);
                }
                if it.light[0] != 0 {
                    // A wall or RLE floor block clipped on the left: the
                    // pixels of the run the clip cut do not advance the
                    // light column, so scan left to the run's start.
                    let cut_x = (it.light[3] & 0xFFFF) as i32;
                    let mut run_cut = 0;
                    if matches!(it.light[0], 1 | 2) && cut_x > 0 {
                        let mut start = (px - x0) as i32;
                        while start > -cut_x {
                            let tx = (it.texel[0] as i32 + start - 1) as u32;
                            if page[(ty * PAGE_SIZE + tx) as usize] == 0 {
                                break;
                            }
                            start -= 1;
                        }
                        run_cut = (-start).max(0);
                    }
                    s = map_byte(gradient_row(it, px - x0, py - y0, run_cut), s);
                }
                let blend = |dst: u32| match it.blend {
                    BLEND_OPAQUE => s,
                    BLEND_INDEX_TABLE_SRC_ROW => map_byte(it.blend_base + s, dst),
                    _ => map_byte(it.blend_base + dst, s),
                };
                value = blend(value);
                let on_screen = it.layer != LAYER_WORLD;
                let translucent = it.blend != BLEND_OPAQUE;
                screen = on_screen || (screen && translucent);
                if !on_screen {
                    world = blend(world);
                }
                if it.layer != LAYER_UI {
                    scene = blend(scene);
                    scene_screen = on_screen || (scene_screen && translucent);
                }
            }
            if params.clear_after != 0 {
                value = 0;
                world = 0;
                screen = false;
                scene = 0;
                scene_screen = false;
            }
            out.push(LayerPixel {
                index: value as u8,
                world: world as u8,
                screen,
                scene: scene as u8,
                scene_screen,
            });
        }
    }
    Ok(out)
}

/// The shader's `gradient_row`: the light map row of area pixel
/// `(ax, ay)` of a gradient-lit item, from the packed words only.
fn gradient_row(it: &GpuItem, ax: u32, ay: u32, run_cut: i32) -> u32 {
    let [kind, light0, corners, origin] = it.light;
    let c = |i: u32| ((corners >> (8 * i)) & 0xFF) as i32;
    let (scale, shift) = if kind == 1 { (32, 8) } else { (16, 7) };
    let x = (ax + (origin & 0xFFFF)) as i32 - run_cut;
    let r = (ay + (origin >> 16)) as i32;
    let a = (scale * c(0) + r * (c(3) - c(0))) >> shift;
    let b = (scale * c(1) + r * (c(2) - c(1))) >> shift;
    light0 + ((32 * a + x * (b - a)) >> 5) as u32
}

/// Frames put in an atlas, for synthetic cases: slot `n` holds
/// `FrameImage` `n`, so the image list is both the CPU's frame source and,
/// through `slots`, the GPU's.
#[derive(Debug, Clone)]
pub struct AtlasFrames {
    pub atlas: Atlas,
    pub slots: Vec<AtlasSlot>,
}

impl AtlasFrames {
    /// Packs `images` as one frame set (C3's shelf packer, offsets 0).
    pub fn from_images(images: &[FrameImage], max_pages: u32) -> Result<Self, GpuError> {
        let frames = images
            .iter()
            .map(|i| IndexFrame::new(i.width, i.height, 0, 0, i.pixels.clone()))
            .collect::<Result<Vec<_>, _>>()?;
        let mut atlas = Atlas::new(max_pages)?;
        let slots = atlas.insert_set(&frames)?;
        Ok(AtlasFrames { atlas, slots })
    }

    pub fn pages(&self) -> &[AtlasPage] {
        self.atlas.pages()
    }

    pub fn page_count(&self) -> u32 {
        self.atlas.pages().len() as u32
    }
}

fn u32s_le(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

fn words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes(*c))
        .collect()
}

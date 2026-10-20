//! The front end's menus at the screen's real resolution: the native UI
//! pass (`gpu_compositor::ui_pass`) for the 800 × 600 front-end picture.
//!
//! Every front-end screen is all UI (art, buttons, text, the paper dolls,
//! the pointer), so when the window shows the picture enlarged (1.8× at
//! 1080p, 3.6× at 2160p) every cel is drawn by the pass at the output
//! resolution instead of being the picture's own pixels enlarged. The
//! CPU picture is still composed exactly as before: tests, the side fill
//! and windows without the pass read it.
//!
//! How:
//! - **Cels.** The front end's draw items become cels
//!   (`super::front_cels`), the same list the CPU picture is drawn from.
//! - **Atlas.** Each image cel is put once in a front-end atlas, by its key
//!   (file and frame), so only new art is uploaded. A solid rectangle is
//!   one atlas texel stretched over its rectangle and coloured through a
//!   map row that sends it to the rectangle's index (index 0 included).
//! - **Blending.** A cel's draw mode becomes a [`UiBlend`]: the three
//!   alpha levels are measured from the screen's PL2 tables under the
//!   palette shown, then additive, multiplicative, and max-component shown
//!   as additive. A cel without tables is opaque.
//! - **Placement.** Every cel covers exactly the output pixels the mouse
//!   maps into its frame pixels (`ui::frame::first_pixel` on its edges),
//!   so clicks land where they always did.
//! - **Presenting.** A [`NativeUiMaterial`] quad on the picture's place
//!   shows the pass's UI over the picture's background (palette index 0).
//! - **Text.** Loaded glyph bitmaps are always
//!   enhanced through `world_view::ui_art::hd_text`, sharing the game's
//!   glyph cache. Indexed images keep every source colour and use the
//!   existing text-colour maps. Glyph boxes and placement never change.
//!   The CPU index picture, its hashes and side fill never depend on this
//!   output-only enhancement.
//!
//! Used when the native UI path is active (`world_view::native_ui::placement`):
//! whenever the window shows the picture enlarged.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::schedule::camera_driver;
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::renderer::{
    RenderContext, RenderDevice, RenderGraph, RenderGraphSystems, RenderQueue,
};
use bevy::render::texture::GpuImage;
use bevy::render::RenderApp;
use d2_formats::palette::{Palette, Pl2};

use super::front_cels::{Cel, CelGlyph, CelPixels};
use crate::frames::atlas::{Atlas, AtlasError, AtlasPage, AtlasSlot};
use crate::frames::IndexFrame;
use crate::gpu_compositor::ui_pass::{
    bin_items, fit_alpha, place_image, CelImage, UiBlend, UiItem, UiPacked, UiPass, UiPlacement,
    SOURCE_ATLAS,
};
use crate::world_view::native_ui::{NativeUiMaterial, NativeUiTargets};
use crate::world_view::ui_art::hd_text::{FontSheet, GlyphCel, HdTextArt};

/// The front-end atlas's page budget (4 MiB each): far more than any
/// screen's art.
const MAX_PAGES: u32 = 8;

/// Walks a frame's cels, handing each one to the callback.
pub type CelWalk<'a> = &'a mut dyn FnMut(&mut dyn FnMut(Cel<'_>));

/// The atlas texel solid rectangles are drawn from.
const SOLID_TEXEL: u8 = 1;

/// The [`UiBlend`] of each draw mode (0..=7) under the PL2 tables `pl2`
/// and the palette shown: modes 0–2 the alpha levels 2, 1, 0 as measured
/// from their tables, 3 additive, 4 multiplicative, 6 max-component (shown
/// as additive), the others opaque. No tables: every mode is opaque.
pub fn mode_blends(pl2: Option<&Pl2>, shown: &Palette) -> [UiBlend; 8] {
    let mut out = [UiBlend::Opaque; 8];
    let Some(t) = pl2 else {
        return out;
    };
    for (mode, slot) in out.iter_mut().enumerate().take(3) {
        if let Some(rows) = t.alpha_blend.get(2 - mode).filter(|r| r.len() >= 256) {
            *slot = UiBlend::Alpha(fit_alpha(rows, shown));
        }
    }
    out[3] = UiBlend::Add;
    out[4] = UiBlend::Mul;
    out[6] = UiBlend::Add;
    out
}

/// One front-end frame for the pass: the packed items, the map rows (bytes,
/// 256 per row) and the palette (256 words of r, g, b, 255).
#[derive(Debug, Clone, PartialEq)]
pub struct FrontUiFrame {
    pub packed: UiPacked,
    pub maps: Vec<u8>,
    pub palette: Vec<u8>,
}

/// The pass's palette buffer of `p`: 256 words of r, g, b, 255.
pub fn palette_bytes(p: &Palette) -> Vec<u8> {
    p.colors.iter().flat_map(|c| [c.r, c.g, c.b, 255]).collect()
}

/// The front-end atlas and the cels it holds, kept across frames.
pub struct FrontUiCels {
    atlas: Atlas,
    slots: HashMap<u64, AtlasSlot>,
    solid: AtlasSlot,
    /// Bumped whenever the atlas's pages change.
    version: u64,
    pages: Option<(u64, Arc<Vec<AtlasPage>>)>,
    /// The palette and tables the blends were measured for.
    blends: Option<(Vec<u8>, bool, [UiBlend; 8])>,
}

impl Default for FrontUiCels {
    fn default() -> Self {
        Self::new()
    }
}

impl FrontUiCels {
    pub fn new() -> Self {
        let mut atlas = Atlas::new(MAX_PAGES).expect("pages");
        let solid = atlas
            .insert_set(&[IndexFrame::new(1, 1, 0, 0, vec![SOLID_TEXEL]).expect("1x1")])
            .expect("an empty atlas fits one texel")[0];
        Self {
            atlas,
            slots: HashMap::new(),
            solid,
            version: 1,
            pages: None,
            blends: None,
        }
    }

    /// The atlas pages and their version (shared until they change).
    pub fn pages(&mut self) -> (u64, Arc<Vec<AtlasPage>>) {
        match &self.pages {
            Some((v, p)) if *v == self.version => (*v, p.clone()),
            _ => {
                let p = Arc::new(self.atlas.pages().to_vec());
                self.pages = Some((self.version, p.clone()));
                (self.version, p)
            }
        }
    }

    /// The blends of the draw modes, measured again when the palette or
    /// the tables changed.
    fn blends(&mut self, pl2: Option<&Pl2>, shown: &Palette) -> [UiBlend; 8] {
        let key = palette_bytes(shown);
        if let Some((k, t, b)) = &self.blends {
            if *k == key && *t == pl2.is_some() {
                return *b;
            }
        }
        let b = mode_blends(pl2, shown);
        self.blends = Some((key, pl2.is_some(), b));
        b
    }

    /// The atlas slot of an image cel, put in on first sight.
    fn slot(&mut self, key: u64, px: &[u8], w: u32, h: u32) -> Result<AtlasSlot, AtlasError> {
        if let Some(s) = self.slots.get(&key) {
            if (s.w, s.h) == (w, h) {
                return Ok(*s);
            }
        }
        let mut pixels = px.to_vec();
        pixels.resize((w * h) as usize, 0);
        let frame = IndexFrame::new(w, h, 0, 0, pixels).map_err(|_| AtlasError::PixelCount {
            width: w,
            height: h,
            len: px.len(),
        })?;
        let slot = self.atlas.insert_set(&[frame])?[0];
        self.version += 1;
        self.slots.insert(key, slot);
        Ok(slot)
    }

    /// Starts again with an empty atlas (it filled up).
    fn reset(&mut self) {
        *self = Self {
            version: self.version + 1,
            blends: self.blends.take(),
            ..Self::new()
        };
    }

    /// Packs one frame: `walk` hands every cel in draw order to its
    /// callback (`super::front_host::item_cels`); `pl2` and `shown` are
    /// the tables and palette in use; `place` is the frame's presentation;
    /// `hd` is the text glyph art (`world_view::ui_art::hd_text_art`), `None`
    /// to draw every cel as itself. `None` when a cel can't be put in the
    /// atlas even after starting it again.
    pub fn pack(
        &mut self,
        pl2: Option<&Pl2>,
        shown: &Palette,
        place: UiPlacement,
        hd: Option<&HdTextArt>,
        walk: CelWalk<'_>,
    ) -> Option<FrontUiFrame> {
        let blends = self.blends(pl2, shown);
        for attempt in 0..2 {
            if attempt == 1 {
                self.reset();
            }
            let mut b = Builder {
                cels: self,
                blends,
                place,
                hd,
                items: Vec::new(),
                rows: Vec::new(),
                row_ids: HashMap::new(),
                failed: false,
            };
            walk(&mut |c| b.push(&c));
            if b.failed {
                continue;
            }
            let (items, rows) = (b.items, b.rows);
            let pages = self.atlas.pages().len() as u32;
            let mut maps = Vec::with_capacity(rows.len().max(1) * 256);
            for r in &rows {
                maps.extend_from_slice(r);
            }
            if maps.is_empty() {
                maps.resize(256, 0);
            }
            let mut packed = bin_items(items, place, rows.len() as u32, pages);
            if let Some(hd) = hd {
                packed.set_rep(hd.glyph_atlas());
            }
            return Some(FrontUiFrame {
                packed,
                maps,
                palette: palette_bytes(shown),
            });
        }
        None
    }
}

/// The picture the pass shows for `frame`, run on the CPU
/// (`ui_pass::emulate_ui`): RGBA rows at the output size, the UI over the
/// background (palette index 0). For captures without a window.
pub fn emulate_rgba(cels: &mut FrontUiCels, frame: &FrontUiFrame) -> Vec<u8> {
    let mut maps = crate::scene::MapTable::new();
    for row in frame.maps.as_chunks::<256>().0 {
        maps.push(*row);
    }
    let mut palette = Palette {
        colors: [d2_formats::palette::Rgb { r: 0, g: 0, b: 0 }; 256],
    };
    for (c, b) in palette
        .colors
        .iter_mut()
        .zip(frame.palette.as_chunks::<4>().0)
    {
        *c = d2_formats::palette::Rgb {
            r: b[0],
            g: b[1],
            b: b[2],
        };
    }
    let (_, pages) = cels.pages();
    let under =
        [frame.palette[0], frame.palette[1], frame.palette[2]].map(|v| f32::from(v) / 255.0);
    crate::gpu_compositor::ui_pass::emulate_ui(&frame.packed, &pages, &maps, &palette)
        .iter()
        .flat_map(|p| {
            let [r, g, b] = p
                .apply(under)
                .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
            [r, g, b, 255]
        })
        .collect()
}

/// A front-end font sheet as HD Text reads it: the DC6 and glyph table
/// the menus draw from, under the sheet's path.
struct DcSheet<'a> {
    path: String,
    glyph: CelGlyph<'a>,
}

impl<'a> DcSheet<'a> {
    fn new(glyph: CelGlyph<'a>) -> Self {
        Self {
            path: glyph.sheet.replace('\\', "/").to_ascii_lowercase(),
            glyph,
        }
    }
}

impl FontSheet for DcSheet<'_> {
    fn path(&self) -> &str {
        &self.path
    }

    fn table(&self) -> Option<&d2_formats::font::FontTable> {
        Some(self.glyph.table)
    }

    fn cel(&self, index: usize) -> Option<GlyphCel<'_>> {
        // A sheet's cels stand on their bottom-left, as the menus place
        // them: the top row is the y offset less the height, plus one.
        let f = self.glyph.dc6.frames.get(index)?;
        Some(GlyphCel {
            width: f.width,
            height: f.height,
            top: f.offset_y - f.height as i32 + 1,
            pixels: &f.pixels,
        })
    }
}

/// The text image of cel `c` at `place`'s scale: the glyph drawn from the
/// enhanced bitmap, or [`CelImage::Original`] when `c` is not a glyph, there
/// is no glyph art (`hd` is `None`) or the art keeps the cel.
fn hd_glyph(hd: Option<&HdTextArt>, c: &Cel<'_>, place: UiPlacement) -> CelImage {
    let (Some(hd), Some(g)) = (hd, c.glyph) else {
        return CelImage::Original;
    };
    let scale = place.out_h as f32 / place.frame_h.max(1) as f32;
    hd.glyph(&DcSheet::new(g), usize::from(g.frame), scale)
}

/// One frame's packing in progress.
struct Builder<'a> {
    cels: &'a mut FrontUiCels,
    blends: [UiBlend; 8],
    place: UiPlacement,
    hd: Option<&'a HdTextArt>,
    items: Vec<UiItem>,
    rows: Vec<[u8; 256]>,
    row_ids: HashMap<[u8; 256], u32>,
    failed: bool,
}

impl Builder<'_> {
    fn row(&mut self, r: [u8; 256]) -> u32 {
        let next = self.rows.len() as u32;
        *self.row_ids.entry(r).or_insert_with(|| {
            self.rows.push(r);
            next
        })
    }

    fn push(&mut self, c: &Cel<'_>) {
        if self.failed {
            return;
        }
        let (w, h) = c.size();
        if w == 0 || h == 0 {
            return;
        }
        let (fw, fh) = (i64::from(self.place.frame_w), i64::from(self.place.frame_h));
        let (x0, y0) = (i64::from(c.x), i64::from(c.y));
        let (x1, y1) = (x0 + i64::from(w), y0 + i64::from(h));
        let vis = [x0.max(0), y0.max(0), x1.min(fw), y1.min(fh)];
        if vis[0] >= vis[2] || vis[1] >= vis[3] {
            return;
        }
        let blend = if c.tables {
            self.blends[usize::from(c.mode.min(7))]
        } else {
            UiBlend::Opaque
        };
        let (kind, alpha) = blend.words();
        let (area, mut place) = place_image(self.place, (x0, y0), (w, h), vis);
        let mut item = UiItem {
            area,
            blend: kind,
            alpha,
            source: SOURCE_ATLAS,
            ..UiItem::default()
        };
        match c.pixels {
            CelPixels::Image { px, .. } => {
                let image = hd_glyph(self.hd, c, self.place);
                if image == CelImage::Original {
                    let slot = match self.cels.slot(c.key, px, w, h) {
                        Ok(s) => s,
                        Err(_) => {
                            self.failed = true;
                            return;
                        }
                    };
                    item.texel = [slot.x, slot.y];
                    item.page = slot.page;
                } else {
                    // From the replacement atlas, over the same output
                    // pixels and clipped the same way as the cel.
                    item.use_image(image);
                }
                item.src = [
                    (vis[0] - x0) as u32,
                    (vis[1] - y0) as u32,
                    (vis[2] - x0) as u32,
                    (vis[3] - y0) as u32,
                ];
                if let Some(m) = c.map {
                    item.shade[0] = self.row(m);
                    item.shade_len = 1;
                }
            }
            CelPixels::Solid { index, .. } => {
                // One texel stretched over the whole rectangle.
                let slot = self.cels.solid;
                item.texel = [slot.x, slot.y];
                item.page = slot.page;
                item.src = [0, 0, 1, 1];
                place[2] *= w as f32;
                place[3] *= h as f32;
                item.shade[0] = self.row([index; 256]);
                item.shade_len = 1;
            }
        }
        item.place = place;
        self.items.push(item);
    }
}

/// The front end's pass for the render world: its packed items, the atlas
/// (shared, replaced when its version changes), map rows, palette and the
/// two output-sized images it writes.
#[derive(Resource, Clone)]
pub struct FrontUiJob {
    /// One per drawn frame: a job is run once.
    pub seq: u64,
    pub packed: Arc<UiPacked>,
    pub pages: Arc<Vec<AtlasPage>>,
    pub pages_version: u64,
    pub maps: Arc<Vec<u8>>,
    pub palette: Arc<Vec<u8>>,
    pub targets: NativeUiTargets,
}

impl ExtractResource for FrontUiJob {
    type Source = FrontUiJob;

    fn extract_resource(source: &Self::Source) -> Self {
        source.clone()
    }
}

/// The pass is available: the app renders and [`add_front_ui`] ran.
#[derive(Resource)]
pub struct FrontUiOn;

/// The pass's output images for the front end and the 1 × 1 background
/// image the material shows where no cel is (palette index 0).
#[derive(Resource)]
pub struct FrontUiImages {
    pub targets: Option<NativeUiTargets>,
    pub background: Handle<Image>,
    pub colour: [u8; 3],
}

/// The render world's state of the pass.
#[derive(Resource, Default)]
struct FrontUiState {
    pass: Option<UiPass>,
    atlas: Option<(u64, wgpu::Texture)>,
    last: Option<u64>,
}

/// Adds the front end's pass when the app renders (once): the job's
/// extraction, the render-graph system and the presenting material.
pub fn add_front_ui(app: &mut App) {
    if app.world().contains_resource::<FrontUiOn>() {
        return;
    }
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render_app.init_resource::<FrontUiState>().add_systems(
        RenderGraph,
        front_ui_node
            .in_set(RenderGraphSystems::Render)
            .before(camera_driver),
    );
    crate::world_view::native_ui::add_native_ui(app);
    app.add_plugins(ExtractResourcePlugin::<FrontUiJob>::default())
        .insert_resource(FrontUiOn);
}

/// Runs a new [`FrontUiJob`] on Bevy's device, before the cameras draw.
fn front_ui_node(
    job: Option<Res<FrontUiJob>>,
    mut state: ResMut<FrontUiState>,
    images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut ctx: RenderContext,
) -> Result {
    use wgpu::util::DeviceExt;
    let Some(job) = job else { return Ok(()) };
    if state.last == Some(job.seq) {
        return Ok(());
    }
    // The targets are uploaded in the render world's prepare step; until
    // then the job waits.
    let (Some(cover), Some(add)) = (images.get(&job.targets.cover), images.get(&job.targets.add))
    else {
        return Ok(());
    };
    let want = (job.packed.params.width, job.packed.params.height);
    for image in [cover, add] {
        let size = image.texture_descriptor.size;
        if (size.width, size.height) != want {
            // A resize is on its way: the next job has the new targets.
            return Ok(());
        }
    }
    let state = &mut *state;
    let wgpu_device = device.wgpu_device();
    if state.atlas.as_ref().map(|a| a.0) != Some(job.pages_version) {
        let texture =
            crate::gpu_compositor::device::upload_atlas_pages(wgpu_device, &queue.0, &job.pages);
        state.atlas = Some((job.pages_version, texture));
    }
    let (_, atlas) = state.atlas.as_ref().ok_or("atlas missing")?;
    let storage = |label, contents: &[u8]| {
        wgpu_device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents,
            usage: wgpu::BufferUsages::STORAGE,
        })
    };
    let maps = storage("front ui maps", &job.maps);
    let palette = storage("front ui palette", &job.palette);
    let pass = state.pass.get_or_insert_with(|| UiPass::new(wgpu_device));
    let view = |image: &GpuImage| image.texture.create_view(&Default::default());
    pass.encode(
        wgpu_device,
        ctx.command_encoder(),
        &job.packed,
        atlas,
        &maps,
        &palette,
        &view(cover),
        &view(add),
    )?;
    state.last = Some(job.seq);
    Ok(())
}

/// A 1 × 1 sRGB image of one colour: the picture under the front end's UI.
pub fn background_image(colour: [u8; 3]) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![colour[0], colour[1], colour[2], 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = bevy::image::ImageSampler::nearest();
    image
}

/// The quad presenting the front end with the pass.
#[derive(Component)]
pub struct FrontUiQuad;

/// The material of the quad: the background colour as the picture, the
/// pass's images over it, the presentation's scale.
pub fn quad_material(
    images: &FrontUiImages,
    targets: &NativeUiTargets,
    scale: Vec4,
) -> NativeUiMaterial {
    NativeUiMaterial {
        frame: images.background.clone(),
        world: images.background.clone(),
        screen: images.background.clone(),
        cover: targets.cover.clone(),
        add: targets.add.clone(),
        scale,
        world_uv: Vec4::new(0.0, 0.0, 1.0, 1.0),
        flags: Vec4::ZERO,
        // No world layer here, so no edge fill from a tick before.
        prev: images.background.clone(),
        prev_uv: crate::world_view::smooth::NO_PREV,
    }
}

#[cfg(test)]
#[path = "front_ui_tests.rs"]
mod tests;

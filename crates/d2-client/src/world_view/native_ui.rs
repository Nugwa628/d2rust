//! The UI at the screen's real resolution: what the game view needs around
//! the native UI pass (`gpu_compositor::ui_pass`).
//!
//! When the frame is shown larger than its own size (1.8× at 1080p, 2.4×
//! at 1440p, 3.6× at 2160p for the 600-line frames), the HUD, the panels,
//! the menus and all text are drawn by that pass at the output resolution
//! on top of the scaled world, instead of being the frame's own pixels
//! enlarged. Text is part of this pass, not a setting of its own: it is
//! drawn at the output resolution from enhanced glyphs
//! (`ui_art::hd_text`) at every View, Window and Scale choice that enlarges
//! the frame. At 1× (the frame shown at its own size) there is nothing to
//! upscale and the composed frame is presented as it is.
//!
//! This module holds:
//! - the switch (`[video] native_ui`, on by default) and when the pass is
//!   used ([`placement`]);
//! - which frame items are UI ([`is_ui`]) and how each is blended
//!   ([`TableKinds`]);
//! - the two output-sized images the pass writes ([`NativeUiTargets`]);
//! - the presenting material ([`NativeUiMaterial`], `native_ui.wgsl`): it
//!   shows the scene exactly as the sharp material shows the frame (or the
//!   smooth camera's world and screen layers between ticks) and lays the
//!   UI over it, per output pixel.
//!
//! The game frame, its hashes and the mouse mapping never depend on any of
//! this: the frame is composed with its UI as always, and every UI item is
//! placed on the output pixels the mouse maps into it.

use std::sync::atomic::{AtomicBool, Ordering};

use bevy::asset::embedded_asset;
use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, TextureFormat, TextureUsages};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dPlugin};
use d2_formats::palette::Palette;

use std::sync::Arc;

use crate::frames::AtlasSlot;
use crate::gpu_compositor::pack::{Packed, LAYER_SCREEN, LAYER_UI, LAYER_WORLD};
use crate::gpu_compositor::ui_pass::{
    fit_alpha, pack_ui, ui_blend, TableKind, UiDraw, UiPlacement,
};
use crate::gpu_compositor::GpuError;

use super::node::UiJob;
use crate::rules::shading::ShadeTables;
use crate::scene::order::pass;
use crate::scene::{DrawItem, ItemTag, MapId, MapTable};
use crate::ui::frame::Presentation;

/// `[video] native_ui`: whether the UI is drawn at the output resolution.
static ENABLED: AtomicBool = AtomicBool::new(true);

/// Turns the native UI pass on or off (from the settings).
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// Whether the native UI pass is switched on.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// The pass's placement for presentation `p`, when the pass is used: it is
/// enabled and the output is larger than the frame. `None`: the frame is
/// presented with its own UI, unchanged.
pub fn placement(p: &Presentation) -> Option<UiPlacement> {
    placement_if(p, enabled())
}

/// [`placement`] with the switch given.
pub fn placement_if(p: &Presentation, on: bool) -> Option<UiPlacement> {
    let place = UiPlacement {
        frame_w: p.frame_w,
        frame_h: p.frame_h,
        out_w: p.image_w,
        out_h: p.image_h,
    };
    (on && place.enlarges()).then_some(place)
}

/// Whether a frame item is UI, drawn by the native pass: everything of the
/// UI pass (panels, the automap, text, the cursor) and anything tagged as
/// a UI element.
pub fn is_ui(item: &DrawItem) -> bool {
    item.key.pass() >= pass::UI || matches!(item.tag, ItemTag::Ui(_))
}

/// The kinds of the act's blend tables, the alpha levels measured from
/// the tables under the frame palette. Measured once per act palette.
#[derive(Debug, Clone, Default)]
pub struct TableKinds {
    kinds: Vec<(MapId, TableKind)>,
}

impl TableKinds {
    /// The kinds of `tables`' blend tables in `maps` under `palette`.
    pub fn measure(tables: &ShadeTables, maps: &MapTable, palette: &Palette) -> Self {
        let rows = |base: MapId| -> Option<&[[u8; 256]]> {
            let start = base.0 as usize;
            maps.rows().get(start..start + 256)
        };
        let mut kinds = Vec::new();
        for base in tables.alpha {
            if let Some(r) = rows(base) {
                kinds.push((base, TableKind::Alpha(fit_alpha(r, palette))));
            }
        }
        kinds.push((tables.additive, TableKind::Add));
        kinds.push((tables.multiplicative, TableKind::Mul));
        kinds.push((tables.max_component, TableKind::Max));
        TableKinds { kinds }
    }

    /// The kind of the table starting at `base`.
    pub fn kind(&self, base: MapId) -> Option<TableKind> {
        self.kinds.iter().find(|(b, _)| *b == base).map(|(_, k)| *k)
    }
}

/// The measured table kinds and the palette and tables they were measured
/// for (a new act's palette measures again).
#[derive(Resource, Default)]
pub struct TableKindCache {
    key: Option<(ShadeTables, [u8; 768])>,
    kinds: TableKinds,
}

impl TableKindCache {
    /// The kinds for `tables` under `palette`, measured when either
    /// changed.
    pub fn get(&mut self, tables: &ShadeTables, maps: &MapTable, palette: &Palette) -> &TableKinds {
        let mut bytes = [0u8; 768];
        for (i, c) in palette.colors.iter().enumerate() {
            bytes[3 * i..3 * i + 3].copy_from_slice(&[c.r, c.g, c.b]);
        }
        let key = (*tables, bytes);
        if self.key.as_ref() != Some(&key) {
            self.kinds = TableKinds::measure(tables, maps, palette);
            self.key = Some(key);
        }
        &self.kinds
    }
}

/// The two images the native UI pass writes, the output image's size:
/// `cover` (how much of the picture the UI hides, per channel) and `add`
/// (what it puts there). All zero is "no UI".
#[derive(Resource, Clone, Debug)]
pub struct NativeUiTargets {
    pub cover: Handle<Image>,
    pub add: Handle<Image>,
    pub size: UVec2,
}

/// An output-sized image the pass writes and the material reads.
fn target_image(size: UVec2) -> Image {
    let mut image = Image::new_target_texture(size.x, size.y, TextureFormat::Rgba8Unorm, None);
    image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING
        | TextureUsages::STORAGE_BINDING
        | TextureUsages::COPY_DST
        | TextureUsages::COPY_SRC;
    // Never read on the CPU: the GPU's zero-initialised texture is "no UI".
    image.data = None;
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    image
}

/// Makes the pass's targets the output size of presentation `p` (new
/// images when the size changed), when the pass is used.
pub fn ensure_targets(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    current: Option<&NativeUiTargets>,
    p: &Presentation,
) -> Option<NativeUiTargets> {
    let place = placement(p)?;
    let size = UVec2::new(place.out_w, place.out_h);
    if let Some(t) = current.filter(|t| t.size == size) {
        return Some(t.clone());
    }
    let targets = new_targets(images, size);
    commands.insert_resource(targets.clone());
    Some(targets)
}

/// A new pair of the pass's images, `size` output pixels (all zero: no
/// UI).
pub fn new_targets(images: &mut Assets<Image>, size: UVec2) -> NativeUiTargets {
    NativeUiTargets {
        cover: images.add(target_image(size)),
        add: images.add(target_image(size)),
        size,
    }
}

/// The presented picture with the native UI over it. The picture is the
/// frame image (the scene: the frame without its UI) or, while the camera
/// or a unit is between two ticks, the smooth camera's world layer
/// (shifted) under its screen layer; either is sharp-scaled exactly as
/// [`super::sharp::SharpMaterial`] does. The UI images are read per output
/// pixel and applied in the palette's own units.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct NativeUiMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub frame: Handle<Image>,
    #[texture(2)]
    pub world: Handle<Image>,
    #[texture(3)]
    pub screen: Handle<Image>,
    #[texture(4)]
    pub cover: Handle<Image>,
    #[texture(5)]
    pub add: Handle<Image>,
    /// xy: the presentation's scale on each axis
    /// ([`super::sharp::scale`]).
    #[uniform(6)]
    pub scale: Vec4,
    /// The world layer's `uv_rect` (`super::smooth::shifted_uv`).
    #[uniform(7)]
    pub world_uv: Vec4,
    /// x: 1 to show the world and screen layers, 0 the frame.
    #[uniform(8)]
    pub flags: Vec4,
    /// The tick before's world layer, read where the shifted world
    /// layer has no picture (`super::smooth::prev_uv`).
    #[texture(9)]
    pub prev: Handle<Image>,
    /// Its `uv_rect`; zero-sized: no fill.
    #[uniform(10)]
    pub prev_uv: Vec4,
}

impl Material2d for NativeUiMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://d2_client/world_view/native_ui.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Opaque
    }
}

/// Registers the material and its shader.
pub struct NativeUiPlugin;

impl Plugin for NativeUiPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "native_ui.wgsl");
        app.add_plugins(Material2dPlugin::<NativeUiMaterial>::default());
    }
}

/// Adds [`NativeUiPlugin`] when the app renders and nothing added it yet.
pub fn add_native_ui(app: &mut App) {
    if app.get_sub_app(bevy::render::RenderApp).is_some()
        && !app.is_plugin_added::<NativeUiPlugin>()
    {
        app.add_plugins(NativeUiPlugin);
    }
}

/// The quad that presents the picture with the native UI.
#[derive(Component, Debug, Clone, Copy)]
pub struct NativeUiQuad;

/// The layer of a frame item for the compositor (`gpu_compositor::pack`):
/// UI, fixed on screen while the camera blends between ticks (the local
/// player, the screen fade: `super::smooth::on_screen_layer`), or world.
pub fn item_layer(item: &DrawItem, local_player: Option<u32>) -> u32 {
    if is_ui(item) {
        LAYER_UI
    } else if super::smooth::on_screen_layer(item, local_player) {
        LAYER_SCREEN
    } else {
        LAYER_WORLD
    }
}

/// The native UI pass of a frame: its UI items (in draw order) packed at
/// `place`, each blended as its draw mode does and drawn with the art
/// `art` gives for it. `frame` is the frame's packed list (its view, map
/// rows, atlas pages and clears); a frame cleared after drawing shows no
/// UI either.
#[allow(clippy::too_many_arguments)]
pub fn ui_job(
    items: &[DrawItem],
    frame: &Packed,
    assets: &super::ViewAssets,
    slots: &[Option<AtlasSlot>],
    place: UiPlacement,
    tables: &mut TableKindCache,
    art: &dyn super::ui_art::UiArt,
    targets: &NativeUiTargets,
) -> Result<UiJob, GpuError> {
    let kinds = assets
        .shades
        .as_ref()
        .map(|s| tables.get(s, &assets.maps, &assets.palette).clone());
    let scale = place.out_h as f32 / place.frame_h.max(1) as f32;
    let draws: Vec<UiDraw<'_>> = if frame.params.clear_after != 0 {
        Vec::new()
    } else {
        items
            .iter()
            .filter(|i| is_ui(i))
            .map(|item| UiDraw {
                item,
                blend: ui_blend(item.blend, |b| kinds.as_ref().and_then(|k| k.kind(b))),
                image: art.cel(
                    &super::ui_art::CelRef {
                        file: assets.frames.owner(item.frame),
                        frames: &assets.frames,
                        fonts: &assets.fonts,
                    },
                    scale,
                ),
            })
            .collect()
    };
    let mut packed = pack_ui(
        &draws,
        &assets.frames,
        slots,
        frame.params.pages,
        frame.params.map_rows,
        frame.view,
        place,
    )?;
    packed.set_rep(art.atlas());
    Ok(UiJob {
        packed: Arc::new(packed),
        cover: targets.cover.clone(),
        add: targets.add.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{DrawKey, FrameId};
    use crate::ui::frame::ScaleMode;

    fn item(pass_id: u32, tag: ItemTag) -> DrawItem {
        let mut d = DrawItem::new(FrameId(0), 0, 0);
        d.key = DrawKey::new(pass_id, 0, 0, 0).unwrap();
        d.tag = tag;
        d
    }

    #[test]
    fn the_ui_pass_and_ui_tagged_items_are_ui() {
        assert!(is_ui(&item(pass::UI, ItemTag::None)));
        assert!(is_ui(&item(pass::WALLS_UNITS, ItemTag::Ui(3))));
        assert!(!is_ui(&item(pass::SCREEN_FADE, ItemTag::None)));
        assert!(!is_ui(&item(pass::WALLS_UNITS, ItemTag::Unit(1))));
        assert!(!is_ui(&item(pass::FLOORS, ItemTag::Tile { x: 0, y: 0 })));
    }

    #[test]
    fn the_pass_is_used_only_when_the_output_is_larger_than_the_frame() {
        let p = |w, h, fw, fh| Presentation::with_mode(w, h, fw, fh, ScaleMode::Fit).unwrap();
        // 1x: an 800 x 600 window shows the frame as it is.
        assert_eq!(placement_if(&p(800, 600, 800, 600), true), None);
        // 640 x 480 shows an 800 x 600 frame smaller: nothing to gain.
        assert_eq!(placement_if(&p(640, 480, 800, 600), true), None);
        for (w, h) in [(1920, 1080), (2560, 1440), (3840, 2160)] {
            let q = p(w, h, 1068, 600);
            let place = placement_if(&q, true).expect("enlarged");
            assert_eq!((place.out_w, place.out_h), (q.image_w, q.image_h));
            assert_eq!((place.frame_w, place.frame_h), (1068, 600));
        }
        // Switched off: never.
        assert_eq!(placement_if(&p(3840, 2160, 1068, 600), false), None);
    }
}

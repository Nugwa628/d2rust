//! Panel art of the base-game UI in the world view: the `ui_image` hook
//! for the [`ImageRef`]s of `ui::panels` ([`PanelArtRules`]) and the
//! loader that makes their DC6 files resident ([`PanelArtLoader`]).
//!
//! An [`ImageRef`] names a file of [`UiFiles`] (`data\global\ui\<name>.dc6`)
//! and a frame of its direction 0. A panel cel draw is light 0xFF
//! (`v = 0xFF` has no `L`) with the request's
//! [`crate::ui::CelLook`]: the plain draw is mode 5, no remap (the
//! opaque copy), a colored draw remaps by `k` or an item colour,
//! and the draw mode picks the blend table
//! ([`super::ui_bind::ui_cel_ops`]). The request's
//! point is the cel draw position, placed by
//! ([`draw_position`]). Rectangles
//! are [`super::ui_bind::rect_sprite`]; the
//! loader makes their frames and colour rows resident.

use std::sync::Arc;

use crate::assets::path::{CanonicalPath, FileSource};
use crate::bridge::world::ClientWorld;
use crate::bridge::ClientUnit;
use crate::composite::{ComponentFrame, ComponentRequest, CompositeError, UnitParams};
use crate::frames::{FramePart, FrameSet, FrameSetKey, IndexFrame};
use crate::rules::camera::FrameSize;
use crate::rules::placement::draw_position;
use crate::rules::shading::{ITEM_PALETTE_FILES, ITEM_PALETTE_MAPS, MAP_BYTES};
use crate::scene::{BlendOp, MapId, ShadeChain};
use crate::ui::panels::UiFiles;
use crate::ui::{ImageRef, ImageRequest, Remap, TextRequest, UiDraw};

use super::ui_bind::{ensure_rects, ui_cel_ops, ui_remap, TextColors};
use super::{TileDraw, UiRules, UiSprite, UnitPose, ViewAssets, ViewError, ViewRules};

const SPEC: &str = "ui/panels.md";

/// The archive name of a [`UiFiles`] name (relative to
/// `DATA\GLOBAL\UI\`, `.dc6`).
pub fn archive_name(name: &str) -> String {
    if let Some(item) = name.strip_prefix(crate::ui::panels::inv_items::ITEMS_PREFIX) {
        return crate::ui::inv_grid::inventory_path(item); // ui/inventory.md §8 r2
    }
    if let Some(label) = name.strip_prefix(crate::ui::original::esc_art::LOCAL_PREFIX) {
        return format!("data\\local\\ui\\eng\\{label}.dc6"); // frontend-options.md §O2 r3
    }
    format!("data\\global\\ui\\{name}.dc6")
}

/// The frame set of an image's file: direction 0 of its DC6.
pub fn image_set(files: &UiFiles, image: ImageRef) -> Result<FrameSetKey, ViewError> {
    let name = files
        .name(image.file)
        .ok_or_else(|| ViewError::Unresolved {
            what: "UI image file",
            spec: SPEC,
            message: format!("file id {} names no panel file", image.file),
        })?;
    let path = CanonicalPath::new(&archive_name(name)).map_err(|e| ViewError::Unresolved {
        what: "UI image file",
        spec: SPEC,
        message: format!("{name}: {e}"),
    })?;
    FrameSetKey::new(path.as_str(), FramePart::Dir(0)).map_err(|e| ViewError::Unresolved {
        what: "UI image file",
        spec: SPEC,
        message: format!("{name}: {e}"),
    })
}

/// The panel cel draw of `req` (module doc); `colors` are the frame's
/// text-colour maps (remap `k` 1–12).
pub fn panel_sprite(
    files: &UiFiles,
    req: &ImageRequest,
    colors: Option<&TextColors>,
    assets: &ViewAssets,
) -> Result<UiSprite, ViewError> {
    let set = image_set(files, req.image)?;
    let index = req.image.frame as usize;
    let frame: &IndexFrame = assets.frame(&set, index)?;
    let (x, y) = draw_position(frame, req.at.x, req.at.y);
    let remap = ui_remap(req.look.remap, colors, assets)?;
    let (shade, blend) = ui_cel_ops(assets.shades.as_ref(), req.look.mode, remap)?;
    Ok(UiSprite {
        frame: ComponentFrame { set, index },
        x,
        y,
        shade,
        blend,
        clip: None,
    })
}

/// `rules` with the base-game panel art as its `ui_image` answer; every
/// other hook is `rules`'.
pub struct PanelArtRules<R> {
    pub rules: R,
    pub files: UiFiles,
    /// With text colours, `ui_text` is the base-game text hooks with the
    /// act's PL2 text-colour maps
    /// ([`super::ui_bind::OriginalTextHooks`]); `None`: `rules`' answer.
    pub text: Option<SharedTextColors>,
}

/// The frame's PL2 text-colour maps, set when the
/// palette act's maps are pushed; `None` inside: not yet pushed (only
/// colour 0 draws).
pub type SharedTextColors = std::sync::Arc<std::sync::RwLock<Option<super::ui_bind::TextColors>>>;

impl<R: ViewRules> ViewRules for PanelArtRules<R> {
    fn tiles(&self, world: &ClientWorld, assets: &ViewAssets) -> Result<Vec<TileDraw>, ViewError> {
        self.rules.tiles(world, assets)
    }

    fn unit_pose(
        &self,
        world: &ClientWorld,
        unit: &ClientUnit,
    ) -> Result<Option<UnitPose>, ViewError> {
        self.rules.unit_pose(world, unit)
    }

    fn unit_params(
        &self,
        world: &ClientWorld,
        unit: &ClientUnit,
        pose: &UnitPose,
    ) -> Result<UnitParams, ViewError> {
        self.rules.unit_params(world, unit, pose)
    }

    fn unit_shadows(
        &self,
        world: &ClientWorld,
        unit: &ClientUnit,
        pose: &UnitPose,
        at: Option<crate::rules::draw_order::OrderKey>,
        draws: &[crate::composite::ComponentDraw],
        assets: &ViewAssets,
    ) -> Result<Vec<crate::scene::DrawItem>, ViewError> {
        self.rules
            .unit_shadows(world, unit, pose, at, draws, assets)
    }

    fn unit_slot_calls(
        &self,
        unit: &ClientUnit,
        pose: &UnitPose,
        cof: &d2_formats::cof::Cof,
    ) -> Result<Vec<crate::world_view::SlotCall>, ViewError> {
        self.rules.unit_slot_calls(unit, pose, cof)
    }

    fn unit_shadow_key(&self, unit: &ClientUnit) -> Option<crate::rules::draw_order::OrderKey> {
        self.rules.unit_shadow_key(unit)
    }

    fn unit_draws_body(&self, unit: &ClientUnit) -> bool {
        self.rules.unit_draws_body(unit)
    }

    fn unit_light_word(&self, unit: &ClientUnit) -> Option<u32> {
        self.rules.unit_light_word(unit)
    }

    fn unit_client_point(&self, unit: &ClientUnit) -> Option<(i32, i32)> {
        self.rules.unit_client_point(unit)
    }

    fn component_call(
        &self,
        unit: &ClientUnit,
        req: &ComponentRequest<'_>,
    ) -> Option<crate::scene::CelArgs> {
        self.rules.component_call(unit, req)
    }

    fn unit_listed_key(&self, unit: &ClientUnit) -> Option<crate::rules::draw_order::OrderKey> {
        self.rules.unit_listed_key(unit)
    }

    fn component_frame(
        &self,
        unit: &ClientUnit,
        pose: &UnitPose,
        req: &ComponentRequest<'_>,
    ) -> Result<ComponentFrame, CompositeError> {
        self.rules.component_frame(unit, pose, req)
    }

    fn component_item_trans(
        &self,
        unit: &ClientUnit,
        req: &ComponentRequest<'_>,
    ) -> Option<(u8, u8)> {
        self.rules.component_item_trans(unit, req)
    }

    fn component_slot_frame(
        &self,
        unit: &ClientUnit,
        pose: &UnitPose,
        req: &ComponentRequest<'_>,
    ) -> Result<Option<ComponentFrame>, CompositeError> {
        self.rules.component_slot_frame(unit, pose, req)
    }

    fn place(
        &self,
        unit: &ClientUnit,
        pose: &UnitPose,
        req: &ComponentRequest<'_>,
        image: &IndexFrame,
    ) -> Result<(i32, i32), CompositeError> {
        self.rules.place(unit, pose, req, image)
    }

    fn shade(
        &self,
        unit: &ClientUnit,
        req: &ComponentRequest<'_>,
    ) -> Result<ShadeChain, CompositeError> {
        self.rules.shade(unit, req)
    }

    fn blend(
        &self,
        unit: &ClientUnit,
        req: &ComponentRequest<'_>,
    ) -> Result<BlendOp, CompositeError> {
        self.rules.blend(unit, req)
    }
}

impl<R: UiRules> UiRules for PanelArtRules<R> {
    fn ui_image(&self, req: &ImageRequest, assets: &ViewAssets) -> Result<UiSprite, ViewError> {
        let colors = self
            .text
            .as_ref()
            .and_then(|t| *t.read().unwrap_or_else(|e| e.into_inner()));
        panel_sprite(&self.files, req, colors.as_ref(), assets)
    }

    fn ui_text(&self, req: &TextRequest, assets: &ViewAssets) -> Result<Vec<UiSprite>, ViewError> {
        let Some(text) = &self.text else {
            return self.rules.ui_text(req, assets);
        };
        let colors = *text.read().unwrap_or_else(|e| e.into_inner());
        let hooks = super::ui_bind::OriginalTextHooks {
            colors,
            shades: assets.shades,
        };
        super::ui_bind::text_sprites(&hooks, req, assets)
    }

    /// Pass 11: everything after the world draw,
    /// as [`crate::rules::OriginalView`] answers; with no camera the
    /// frame is the UI alone and keeps the same pass.
    fn ui_pass(&self) -> Result<u32, ViewError> {
        Ok(crate::scene::order::pass::UI)
    }
}

/// The archive name of item palette file `name` (the loader).
pub fn item_palette_name(name: &str) -> String {
    format!("data\\global\\items\\palette\\{name}.dat")
}

/// Reads the panel DC6 files a frame's UI draws name, once each, from the
/// user's archives (archive order is the source's).
pub struct PanelArtLoader {
    pub source: Arc<dyn FileSource>,
    pub files: UiFiles,
    /// File id → (name, frame set key) of the draws already resolved: the
    /// key is built once, not per draw per frame (q-perf). An entry is
    /// valid while `files` names the id the same.
    keys: std::sync::Mutex<std::collections::HashMap<u32, (String, FrameSetKey)>>,
}

impl PanelArtLoader {
    pub fn new(source: Arc<dyn FileSource>, files: UiFiles) -> Self {
        Self {
            source,
            files,
            keys: Default::default(),
        }
    }

    /// [`image_set`], memoized by file id.
    fn set_of(&self, image: ImageRef) -> Result<FrameSetKey, ViewError> {
        let mut keys = self.keys.lock().unwrap_or_else(|e| e.into_inner());
        if let (Some((name, key)), Some(now)) = (keys.get(&image.file), self.files.name(image.file))
        {
            if name == now {
                return Ok(key.clone());
            }
        }
        let key = image_set(&self.files, image)?;
        if let Some(name) = self.files.name(image.file) {
            keys.insert(image.file, (name.to_owned(), key.clone()));
        }
        Ok(key)
    }

    /// Pushes the 21 maps of each item palette file (the loader reads
    /// 5,376 bytes per file) in file order; returns
    /// the first row. A file no archive holds, or a short one, is an error.
    fn push_item_palettes(&self, assets: &mut ViewAssets) -> Result<MapId, ViewError> {
        let maps = usize::from(ITEM_PALETTE_MAPS);
        let mut rows = Vec::with_capacity(ITEM_PALETTE_FILES.len() * maps);
        for name in ITEM_PALETTE_FILES {
            let archive = item_palette_name(name);
            let fail = |message: String| ViewError::Unresolved {
                what: "item palette file",
                spec: "render/shading.md",
                message: format!("{archive}: {message}"),
            };
            let bytes = self
                .source
                .read_file(&archive)
                .ok_or_else(|| fail("in no archive".into()))?
                .map_err(fail)?;
            let need = maps * MAP_BYTES;
            if bytes.len() < need {
                return Err(fail(format!("{} bytes, {need} needed", bytes.len())));
            }
            for m in 0..maps {
                let row: [u8; MAP_BYTES] = bytes[m * MAP_BYTES..(m + 1) * MAP_BYTES]
                    .try_into()
                    .expect("one map");
                rows.push(row);
            }
        }
        let base = MapId(assets.maps.len() as u32);
        for row in rows {
            assets.maps.push(row);
        }
        Ok(base)
    }

    /// Makes every image's frame set resident. A file no archive holds, or
    /// one that does not parse, is an error: never skipped.
    pub fn ensure(&self, draws: &[UiDraw], assets: &mut ViewAssets) -> Result<(), ViewError> {
        ensure_rects(draws, FrameSize::play(), assets)?;
        let item_colors = draws.iter().any(
            |d| matches!(d, UiDraw::Image(r) if matches!(r.look.remap, Remap::ItemColor { .. })),
        );
        if item_colors && assets.item_palettes.is_none() {
            assets.item_palettes = Some(self.push_item_palettes(assets)?);
        }
        for d in draws {
            let UiDraw::Image(req) = d else { continue };
            let set = self.set_of(req.image)?;
            if assets.frames.contains(&set) {
                continue;
            }
            let name = self.files.name(req.image.file).unwrap_or_default();
            let archive = archive_name(name);
            let fail = |message: String| ViewError::Unresolved {
                what: "UI image file",
                spec: SPEC,
                message: format!("{archive}: {message}"),
            };
            // The 16:9 side bars are built from the control panel's own
            // frames; without them (or in a narrower frame) they are blank.
            if name == crate::ui::panels::border::SIDE_FILE {
                let art = archive_name(crate::ui::panels::border::SIDE_ART);
                let made = crate::assets::path::read_dc6(self.source.as_ref(), &art)
                    .and_then(Result::ok)
                    .and_then(|dc6| FrameSet::from_dc6(&dc6, 0).ok())
                    .and_then(|panel| {
                        crate::ui::panels::border::side_bar_frames(
                            &panel.frames,
                            &assets.palette,
                            crate::ui::panels::border::SIDE_TILE_W,
                        )
                    });
                let frames = made.map(Vec::from).unwrap_or_else(|| {
                    let blank = crate::frames::IndexFrame::new(1, 1, 0, 0, vec![0]).expect("1 × 1");
                    vec![blank; 2]
                });
                assets.frames.insert(set, FrameSet { frames })?;
                continue;
            }
            // D2RUST's own, unverified (the preview): the HUD's fill cels are made
            // here; a HUD file no archive holds draws nothing (logged).
            if let Some(frames) = crate::ui::original::hud::preview_set(
                name,
                || self.source.read_file(&archive).is_none(),
                &assets.palette,
            ) {
                assets.frames.insert(set, frames)?;
                continue;
            }
            let dc6 = crate::assets::path::read_dc6(self.source.as_ref(), &archive)
                .ok_or_else(|| fail("in no archive".into()))?
                .map_err(fail)?;
            let frames = FrameSet::from_dc6(&dc6, 0).map_err(|e| fail(e.to_string()))?;
            assets.frames.insert(set, frames)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::path::MemorySource;
    use crate::ui::panels::PanelTables;
    use crate::ui::{Point, FRAME};
    use d2_formats::palette::{Palette, Rgb};

    /// A DC6 of one direction with `frames` frames of `w` × `h` literal
    /// pixels.
    pub(crate) fn dc6(frames: u32, w: u32, h: u32) -> Vec<u8> {
        let mut rows = Vec::new();
        for _ in 0..h {
            rows.push(w as u8);
            rows.extend((0..w).map(|i| 1 + i as u8));
            rows.push(0x80);
        }
        let mut d = Vec::new();
        for v in [6i32, 1, 0] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        d.extend_from_slice(&[0xEE; 4]);
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&frames.to_le_bytes());
        let mut at = d.len() + 4 * frames as usize;
        let mut body = Vec::new();
        for _ in 0..frames {
            d.extend_from_slice(&(at as u32).to_le_bytes());
            for v in [0u32, w, h, 0, 0, 0, 0, rows.len() as u32] {
                body.extend_from_slice(&v.to_le_bytes());
            }
            body.extend_from_slice(&rows);
            body.extend_from_slice(&[0xEE; 3]);
            at += 32 + rows.len() + 3;
        }
        d.extend(body);
        d
    }

    fn assets() -> ViewAssets {
        ViewAssets::new(Palette {
            colors: [Rgb::default(); 256],
        })
    }

    fn image(file: u32, frame: u32, x: i32, y: i32) -> ImageRequest {
        ImageRequest {
            image: ImageRef { file, frame },
            at: Point::new(x, y),
            clip: FRAME,
            look: crate::ui::CelLook::PLAIN,
            call: crate::ui::draw::CelCall::Draw,
        }
    }

    #[test]
    fn panel_cels_load_once_and_draw_bottom_anchored() {
        let files = PanelTables::load().unwrap().files;
        let id = files.id("panel\\buysellbtn").unwrap();
        let mut src = MemorySource::default();
        src.insert("data\\global\\ui\\panel\\buysellbtn.dc6", dc6(12, 4, 3));
        let loader = PanelArtLoader::new(Arc::new(src), files.clone());
        let mut a = assets();
        let draws = [
            UiDraw::Image(image(id, 10, 418, 476)),
            UiDraw::Image(image(id, 11, 418, 476)),
        ];
        loader.ensure(&draws, &mut a).unwrap();
        assert_eq!(a.frames.len(), 12, "one file, read once");
        loader.ensure(&draws, &mut a).unwrap();
        assert_eq!(a.frames.len(), 12);
        // The cel draw at (418, 476) covers rows 476 − h + 1 … 476.
        let s = panel_sprite(&files, &image(id, 10, 418, 476), None, &a).unwrap();
        assert_eq!((s.x, s.y, s.frame.index), (418, 474, 10));
        assert_eq!(s.frame.set.path(), "data/global/ui/panel/buysellbtn.dc6");
        assert_eq!((s.shade, s.blend), (ShadeChain::EMPTY, BlendOp::Opaque));
        // Past the file's frames: an error, not a skip.
        assert!(panel_sprite(&files, &image(id, 12, 0, 0), None, &a).is_err());
    }

    #[test]
    fn missing_or_unknown_files_are_errors() {
        let files = PanelTables::load().unwrap().files;
        let id = files.id("panel\\invchar6").unwrap();
        let loader = PanelArtLoader::new(Arc::new(MemorySource::default()), files.clone());
        let mut a = assets();
        let missing = [UiDraw::Image(image(id, 0, 0, 0))];
        assert!(loader.ensure(&missing, &mut a).is_err());
        let unknown = [UiDraw::Image(image(u32::MAX, 0, 0, 0))];
        assert!(loader.ensure(&unknown, &mut a).is_err());
        assert!(a.frames.is_empty());
    }

    // q-perf: the memoized key is the key `image_set` builds, every time.
    #[test]
    fn the_memoized_set_key_equals_image_set() {
        let files = PanelTables::load().unwrap().files;
        let id = files.id("panel\\invchar6").unwrap();
        let loader = PanelArtLoader::new(Arc::new(MemorySource::default()), files.clone());
        let r = ImageRef { file: id, frame: 0 };
        let want = image_set(&files, r).unwrap();
        assert_eq!(loader.set_of(r).unwrap(), want);
        assert_eq!(loader.set_of(r).unwrap(), want);
        assert!(loader
            .set_of(ImageRef {
                file: u32::MAX,
                frame: 0
            })
            .is_err());
    }
}

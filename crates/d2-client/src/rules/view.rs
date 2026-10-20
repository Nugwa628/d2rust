//! [`OriginalView`]: the world view's placement hooks, answered by the camera and placement
//! rules.
//!
//! `tiles` places map tiles (handed positions from the camera, culled to the view, DT1
//! images placed block by block), `unit_params` sets the frame clip and `place` puts each
//! component cel on screen. Every other question (which tiles and units, poses, component
//! frames, draw keys, shading, blend, UI) goes to the wrapped rules `R` or the
//! [`ViewSource`].

use d2_formats::cof::Cof;
use d2_formats::dt1::Dt1Tile;

use crate::bridge::world::ClientWorld;
use crate::bridge::ClientUnit;
use crate::composite::{
    ComponentDraw, ComponentFrame, ComponentRequest, CompositeError, UnitParams,
};
use crate::frames::{FrameAnchor, IndexFrame, DT1_BLOCK_FRAME0};
use crate::scene::{BlendOp, DrawItem, DrawKey, GradientKind, LightGradient, Rect, ShadeChain};
use crate::ui::{ImageRequest, TextRequest};
use crate::world_view::{TileDraw, UiRules, UiSprite, UnitPose, ViewAssets, ViewError, ViewRules};

use super::camera::{Camera, TileList, UnitPosition};
use super::draw_order::{OrderKey, UnitSlot};
use super::placement;
use super::unit_composite::{cof_box_visible, shadow_box_visible};

/// One DT1 block's rectangle in tile coordinates (`b.x`, `b.y`, size), for the per-block
/// wall culling ([`Camera::wall_block_visible`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl BlockRect {
    /// Every block of a DT1 tile, in file order.
    pub fn of_tile(tile: &Dt1Tile) -> Vec<BlockRect> {
        tile.blocks
            .iter()
            .map(|b| {
                let (w, h) = b.size();
                BlockRect {
                    x: i32::from(b.x),
                    y: i32::from(b.y),
                    width: w as u32,
                    height: h as u32,
                }
            })
            .collect()
    }
}

/// The shade and blend of one DT1 block of a tile: each 32-pixel block has its own light,
/// and translucent walls blend block by block. A gradient in `shade` is positioned at the
/// block when the tile is placed (its `x`, `y` are overwritten).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockShade {
    pub block: BlockRect,
    pub shade: ShadeChain,
    pub blend: BlendOp,
}

/// One map tile to draw, as the map and draw order state it: the cell, its tile list, its
/// assembled DT1 image, its blocks, and its shade, blend and draw key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapTile {
    /// Absolute tile cell `(tx, ty)`.
    pub cell: (i32, i32),
    pub list: TileList,
    /// The DT1 tile frame set (`FramePart::Tile`) and frame 0.
    pub frame: ComponentFrame,
    /// The tile's blocks (`BlockRect::of_tile`), for wall culling.
    pub blocks: Vec<BlockRect>,
    /// The tile's shade (a light map), for the whole tile when
    /// [`ViewSource::tile_blocks`] gives no per-block shades.
    pub shade: ShadeChain,
    /// The tile's blend (roof fade, translucent walls), for the whole tile when no blocks
    /// are given.
    pub blend: BlendOp,
    /// The tile's place in the draw order ([`super::draw_order::source`]).
    pub key: DrawKey,
}

/// The inputs placement reads from the client world model. An implementation whose inputs
/// the model does not hold returns an error, never a default.
pub trait ViewSource {
    /// The unit's position as the client keeps it: 16.16 subtiles of a moving unit,
    /// integer subtiles of a static one, as the server's messages last placed it.
    fn unit_position(&self, unit: &ClientUnit) -> Result<UnitPosition, String>;

    /// The per-unit extra offsets `(ox, oy)` added to the unit's draw position:
    /// `(ox, oy + oz)` of the client motion record, plus the object and missile offsets
    /// (`rules::unit_composite::unit_offset`).
    fn unit_offset(&self, unit: &ClientUnit, pose: &UnitPose) -> Result<(i32, i32), String>;

    /// The shadow's extra offset: `(ox + oz / 2, oy + oz / 2)` of the motion record plus
    /// the object offsets, so a raised unit's shadow lands below and beside it. The
    /// default is [`Self::unit_offset`] (equal without a height `oz`).
    fn unit_shadow_offset(&self, unit: &ClientUnit, pose: &UnitPose) -> Result<(i32, i32), String> {
        self.unit_offset(unit, pose)
    }

    /// The tiles of the frame, each with its list and draw key. A source with a map-tile
    /// feed is wrapped in `draw_order::source::OrderedSource`, which answers this; a
    /// source without one states its tiles directly.
    fn map_tiles(
        &self,
        world: &ClientWorld,
        assets: &ViewAssets,
    ) -> Result<Vec<MapTile>, ViewError>;

    /// The unit's slot in the frame's draw order. [`UnitSlot::Unordered`] (the default)
    /// leaves the unit and its keys to the wrapped rules.
    fn unit_slot(&self, _unit: &ClientUnit) -> UnitSlot {
        UnitSlot::Unordered
    }

    /// The unit's shadow entry in the frame's shadow pass: its draw key position. `None`
    /// (the default): no shadow is drawn.
    fn unit_shadow_slot(&self, _unit: &ClientUnit) -> Option<OrderKey> {
        None
    }

    /// The per-block shade and blend of a tile (`rules::lighting::draws` and
    /// `rules::shading` answer them from the frame's light map). Empty (the default) = the tile's own `shade`
    /// and `blend` for the whole tile.
    fn tile_blocks(&self, _tile: &MapTile) -> Result<Vec<BlockShade>, ViewError> {
        Ok(Vec::new())
    }

    /// The tile's light digest for the render facts export; `None` (the default): not
    /// measured.
    fn tile_light(&self, _tile: &MapTile) -> Option<String> {
        None
    }

    /// The alpha byte of a translucent wall tile's record, as the render facts export
    /// reports it; `None` (the default): not measured.
    fn tile_alpha(&self, _tile: &MapTile) -> Option<u8> {
        None
    }
}

/// The camera and placement rules over wrapped rules `R` (pose, component frames, draw
/// keys, shading, blend, UI) and a [`ViewSource`]. Build one per drawn frame: `camera` is
/// that frame's one camera.
#[derive(Debug, Clone, Copy)]
pub struct OriginalView<'a, R: ?Sized, S: ?Sized> {
    pub camera: Camera,
    pub rules: &'a R,
    pub source: &'a S,
    /// Play preview (decision D1, `// D2RUST's own, unverified`): a unit cel
    /// cut by the frame edge is placed and left to the frame clip instead
    /// of failing the frame. Off by
    /// default: the strict path.
    pub edge_clip: bool,
}

const CAMERA: &str = "render/camera.md";
const PLACEMENT: &str = "render/sprite-placement.md";

fn unresolved(what: &'static str, spec: &'static str, message: impl Into<String>) -> ViewError {
    ViewError::Unresolved {
        what,
        spec,
        message: message.into(),
    }
}

fn component_error(
    req: &ComponentRequest<'_>,
    what: &'static str,
    message: String,
) -> CompositeError {
    CompositeError::Unresolved {
        slot: req.slot.slot,
        component: req.slot.component,
        what,
        message,
    }
}

impl<'a, R: ?Sized, S: ?Sized> OriginalView<'a, R, S> {
    pub fn new(camera: Camera, rules: &'a R, source: &'a S) -> Self {
        OriginalView {
            camera,
            rules,
            source,
            edge_clip: false,
        }
    }

    /// This view with the preview edge clip on or off ([`Self::edge_clip`]).
    pub fn with_edge_clip(mut self, on: bool) -> Self {
        self.edge_clip = on;
        self
    }

    /// The draw of one map tile, or `None` when culled. A tile handed to its drawer that
    /// puts no pixel in the frame (a floor or roof inside the view rectangle but wholly
    /// off-frame, a wall whose every block is culled) is a call without pixels: a draw with
    /// an empty clip ([`TileDraw::is_call_only`]), kept for the draw-call log, never
    /// composed.
    pub fn tile(&self, tile: &MapTile, image: &IndexFrame) -> Result<Option<TileDraw>, ViewError> {
        if image.anchor != FrameAnchor::Top {
            return Err(unresolved(
                "tile placement",
                PLACEMENT,
                "a DT1 tile image must be top-anchored (§8)",
            ));
        }
        let cam = &self.camera;
        // The tile drawers' clip: top 0, bottom `H − 47` (the rows above the control
        // panel), not the whole frame.
        let full = cam.size.rect();
        let frame = Rect::new(full.x, full.y, full.width, full.height.saturating_sub(47));
        let handed = cam.tile_handed(tile.list, tile.cell.0, tile.cell.1);
        let origin = cam.block_origin(tile.list, handed);
        let clip = match tile.list {
            TileList::Floor | TileList::Roof { .. } => {
                if !cam.floor_roof_visible(handed) {
                    return Ok(None);
                }
                Some(frame)
            }
            TileList::Wall => self.wall_clip(tile, origin, frame)?,
        };
        let placed = placement::place(image, origin.0, origin.1, clip.unwrap_or(frame));
        let clip = clip
            .and(placed.clip)
            .unwrap_or_else(|| Rect::new(placed.x, placed.y, 0, 0));
        Ok(Some(TileDraw {
            frame: tile.frame.clone(),
            x: placed.x,
            y: placed.y,
            clip,
            shade: tile.shade,
            blend: tile.blend,
            key: tile.key,
            cell: tile.cell,
            light: None,
            alpha: None,
        }))
    }

    /// The draws of one tile: [`Self::tile`] when `blocks` is empty, else
    /// one draw per block shade (in the tile's block order): the block's
    /// own image (frame [`DT1_BLOCK_FRAME0`] + `i` of the tile's set, so a block lights
    /// only its own pixels even where isometric neighbours' rectangles overlap) at its
    /// screen position (`placement::block_pixel`), clipped to the tile's clip, with the
    /// block's shade (its gradient moved to the block) and blend. Blocks outside the tile's
    /// clip (culled) draw nothing.
    pub fn tile_draws(
        &self,
        tile: &MapTile,
        image: &IndexFrame,
        blocks: &[BlockShade],
    ) -> Result<Vec<TileDraw>, ViewError> {
        let Some(whole) = self.tile(tile, image)? else {
            return Ok(Vec::new());
        };
        if blocks.is_empty() || whole.is_call_only() {
            return Ok(vec![whole]);
        }
        let cam = &self.camera;
        let origin = cam.block_origin(
            tile.list,
            cam.tile_handed(tile.list, tile.cell.0, tile.cell.1),
        );
        let mut out = Vec::with_capacity(blocks.len());
        // Shades name the tile's blocks in block order (some may be left
        // out): block `i` is the next block at the shade's position and
        // width. A shade may be shorter than its block: a floor gradient
        // lights rows 0…14 only, and the draw covers the shade's rows.
        let mut next = 0;
        for b in blocks {
            let same = |r: &BlockRect| {
                r.x == b.block.x
                    && r.y == b.block.y
                    && r.width == b.block.width
                    && b.block.height <= r.height
            };
            let Some(i) = (next..tile.blocks.len()).find(|&j| same(&tile.blocks[j])) else {
                return Err(unresolved(
                    "tile blocks",
                    PLACEMENT,
                    format!(
                        "tile {:?}: block shade {:?} is not a block of the tile in order",
                        tile.cell, b.block
                    ),
                ));
            };
            next = i + 1;
            let (x, y) = placement::block_pixel(origin, (b.block.x, b.block.y), (0, 0));
            let Some(clip) = whole
                .clip
                .intersect(&Rect::new(x, y, b.block.width, b.block.height))
            else {
                continue;
            };
            let (shade, clip) = match b.shade.gradient() {
                Some(g) => {
                    // An isometric floor block (15 rows high) is lit like an iso
                    // floor: its light origin is its first drawn pixel.
                    let kind = match (g.kind, tile.blocks[i].height) {
                        (GradientKind::RleFloor, 15) => GradientKind::IsoFloor,
                        (kind, _) => kind,
                    };
                    let g = LightGradient { x, y, kind, ..*g };
                    // A gradient covers `kind.rows()` rows (15 for an RLE
                    // floor block): the draw stays inside it.
                    let Some(clip) = clip.intersect(&g.block()) else {
                        continue;
                    };
                    (b.shade.with_gradient(g), clip)
                }
                None => (b.shade, clip),
            };
            out.push(TileDraw {
                frame: ComponentFrame {
                    set: tile.frame.set.clone(),
                    index: DT1_BLOCK_FRAME0 + i,
                },
                x,
                y,
                clip,
                shade,
                blend: b.blend,
                ..whole.clone()
            });
        }
        // The drawer is called whether or not a block lands in the frame:
        // a tile whose every block draws nothing is one call without
        // pixels.
        if out.is_empty() {
            let clip = Rect::new(whole.x, whole.y, 0, 0);
            out.push(TileDraw { clip, ..whole });
        }
        Ok(out)
    }

    /// Wall block culling as a clip of the assembled image: the
    /// frame when every block is kept, `None` when none is, else the
    /// bounding box of the kept blocks, which must not touch a culled
    /// block (otherwise one clip cannot express the cull: an error).
    fn wall_clip(
        &self,
        tile: &MapTile,
        origin: (i32, i32),
        frame: Rect,
    ) -> Result<Option<Rect>, ViewError> {
        let screen = |b: &BlockRect| {
            let (x, y) = placement::block_pixel(origin, (b.x, b.y), (0, 0));
            Rect::new(x, y, b.width, b.height)
        };
        let (kept, culled): (Vec<Rect>, Vec<Rect>) = {
            let mut k = Vec::new();
            let mut c = Vec::new();
            for b in &tile.blocks {
                let r = screen(b);
                if self.camera.wall_block_visible(r.x, r.y) {
                    k.push(r);
                } else {
                    c.push(r);
                }
            }
            (k, c)
        };
        if culled.is_empty() {
            return Ok(Some(frame));
        }
        let Some(first) = kept.first() else {
            return Ok(None);
        };
        let (mut x0, mut y0) = (i64::from(first.x), i64::from(first.y));
        let (mut x1, mut y1) = (x0, y0);
        for r in &kept {
            x0 = x0.min(i64::from(r.x));
            y0 = y0.min(i64::from(r.y));
            x1 = x1.max(i64::from(r.x) + i64::from(r.width));
            y1 = y1.max(i64::from(r.y) + i64::from(r.height));
        }
        let bounds = Rect::new(x0 as i32, y0 as i32, (x1 - x0) as u32, (y1 - y0) as u32);
        if let Some(c) = culled.iter().find(|c| c.intersect(&bounds).is_some()) {
            return Err(unresolved(
                "wall block culling",
                CAMERA,
                format!(
                    "tile {:?}: culled block at ({}, {}) overlaps the kept blocks",
                    tile.cell, c.x, c.y
                ),
            ));
        }
        Ok(bounds.intersect(&frame))
    }
}

impl<R: ViewRules + ?Sized, S: ViewSource + ?Sized> ViewRules for OriginalView<'_, R, S> {
    fn tiles(&self, world: &ClientWorld, assets: &ViewAssets) -> Result<Vec<TileDraw>, ViewError> {
        let mut out = Vec::new();
        for (index, tile) in self.source.map_tiles(world, assets)?.iter().enumerate() {
            let image = assets
                .frame(&tile.frame.set, tile.frame.index)
                .map_err(|e| ViewError::Tile {
                    index,
                    error: Box::new(e),
                })?;
            let tile_error = |e| ViewError::Tile {
                index,
                error: Box::new(e),
            };
            let blocks = self.source.tile_blocks(tile).map_err(tile_error)?;
            let light = self.source.tile_light(tile);
            let alpha = self.source.tile_alpha(tile);
            out.extend(
                self.tile_draws(tile, image, &blocks)
                    .map_err(tile_error)?
                    .into_iter()
                    .map(|d| TileDraw {
                        light: light.clone(),
                        alpha,
                        ..d
                    }),
            );
        }
        Ok(out)
    }

    fn unit_pose(
        &self,
        world: &ClientWorld,
        unit: &ClientUnit,
    ) -> Result<Option<UnitPose>, ViewError> {
        if self.source.unit_slot(unit) == UnitSlot::NotDrawn {
            return Ok(None);
        }
        self.rules.unit_pose(world, unit)
    }

    /// The COF box test at the unit's final draw position (unit position, extra offsets,
    /// camera origins and panel shift), for the composite types 0–2.
    fn unit_box_visible(
        &self,
        unit: &ClientUnit,
        pose: &UnitPose,
        cof: &Cof,
    ) -> Result<bool, ViewError> {
        if unit.key.unit_type > 2 {
            return Ok(true);
        }
        let at = self
            .source
            .unit_position(unit)
            .map_err(|m| unresolved("unit position", CAMERA, m))?
            .client();
        let extra = self
            .source
            .unit_offset(unit, pose)
            .map_err(|m| unresolved("unit offset", CAMERA, m))?;
        let (x, y) = self.camera.unit_draw(at, extra);
        let size = self.camera.size;
        Ok(cof_box_visible(
            cof,
            x,
            y,
            size.width as u32,
            size.height as u32,
        ))
    }

    /// The shadow's own box test at the shadow position (2 pixels left, `oz / 2` on both axes), types
    /// 0–2.
    fn unit_shadow_box_visible(
        &self,
        unit: &ClientUnit,
        pose: &UnitPose,
        cof: &Cof,
    ) -> Result<bool, ViewError> {
        if unit.key.unit_type > 2 {
            return Ok(true);
        }
        let at = self
            .source
            .unit_position(unit)
            .map_err(|m| unresolved("unit position", CAMERA, m))?
            .client();
        let extra = self
            .source
            .unit_shadow_offset(unit, pose)
            .map_err(|m| unresolved("unit shadow offset", CAMERA, m))?;
        let (x, y) = self.camera.unit_draw(at, extra);
        let size = self.camera.size;
        Ok(shadow_box_visible(
            cof,
            x - 2,
            y,
            size.width as u32,
            size.height as u32,
        ))
    }

    /// Draw keys from the draw order when the source has one, else from the wrapped rules;
    /// the clip is the whole frame (the play area is not a clip).
    fn unit_params(
        &self,
        world: &ClientWorld,
        unit: &ClientUnit,
        pose: &UnitPose,
    ) -> Result<UnitParams, ViewError> {
        let mut params = self.rules.unit_params(world, unit, pose)?;
        if let UnitSlot::Drawn(at) = self.source.unit_slot(unit) {
            params.pass = at.pass;
            params.major = at.major;
            params.minor = at.minor;
        }
        params.clip = self.camera.size.rect();
        Ok(params)
    }

    fn component_frame(
        &self,
        unit: &ClientUnit,
        pose: &UnitPose,
        req: &ComponentRequest<'_>,
    ) -> Result<ComponentFrame, CompositeError> {
        self.rules.component_frame(unit, pose, req)
    }

    /// The wrapped rules' shadow draws, keyed at the shadow pass slot the
    /// source states; none without one.
    fn unit_shadows(
        &self,
        world: &ClientWorld,
        unit: &ClientUnit,
        pose: &UnitPose,
        _at: Option<OrderKey>,
        draws: &[ComponentDraw],
        assets: &ViewAssets,
    ) -> Result<Vec<DrawItem>, ViewError> {
        let Some(at) = self.source.unit_shadow_slot(unit) else {
            return Ok(Vec::new());
        };
        self.rules
            .unit_shadows(world, unit, pose, Some(at), draws, assets)
    }

    fn unit_slot_calls(
        &self,
        unit: &ClientUnit,
        pose: &UnitPose,
        cof: &Cof,
    ) -> Result<Vec<crate::world_view::SlotCall>, ViewError> {
        self.rules.unit_slot_calls(unit, pose, cof)
    }

    /// The shadow pass slot the source states.
    fn unit_shadow_key(&self, unit: &ClientUnit) -> Option<OrderKey> {
        self.source.unit_shadow_slot(unit)
    }

    fn unit_draws_body(&self, unit: &ClientUnit) -> bool {
        self.rules.unit_draws_body(unit)
    }

    fn unit_light_word(&self, unit: &ClientUnit) -> Option<u32> {
        self.rules.unit_light_word(unit)
    }

    /// The unit's client point the camera draws it at.
    fn unit_client_point(&self, unit: &ClientUnit) -> Option<(i32, i32)> {
        let c = self.source.unit_position(unit).ok()?.client();
        Some((c.x, c.y))
    }

    fn component_call(
        &self,
        unit: &ClientUnit,
        req: &ComponentRequest<'_>,
    ) -> Option<crate::scene::CelArgs> {
        self.rules.component_call(unit, req)
    }

    fn unit_listed_key(&self, unit: &ClientUnit) -> Option<OrderKey> {
        match self.source.unit_slot(unit) {
            UnitSlot::Drawn(at) => Some(at),
            _ => None,
        }
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

    /// The camera's unit draw position, then the cel's placement. A cel whose rasterizer
    /// rows differ from the unit's frame clip (a top-down DC6 cel crossing an edge) is an
    /// error, unless [`Self::edge_clip`] is on: the unit's components share one clip.
    fn place(
        &self,
        unit: &ClientUnit,
        pose: &UnitPose,
        req: &ComponentRequest<'_>,
        image: &IndexFrame,
    ) -> Result<(i32, i32), CompositeError> {
        let at = self
            .source
            .unit_position(unit)
            .map_err(|m| component_error(req, "unit position", m))?
            .client();
        let extra = self
            .source
            .unit_offset(unit, pose)
            .map_err(|m| component_error(req, "unit offset", m))?;
        let (x, y) = self.camera.unit_draw(at, extra);
        let frame = self.camera.size.rect();
        let placed = placement::place(image, x, y, frame);
        // D2RUST's own, unverified (D1): the preview clips at the frame edge.
        if !self.edge_clip && !placed.same_as_frame(image.width, image.height, frame) {
            return Err(component_error(
                req,
                "placement",
                format!(
                    "top-down cel at ({x}, {y}) is cut by the rasterizer (TODO(spec: {PLACEMENT}) §4): \
                     a per-component clip is needed"
                ),
            ));
        }
        Ok((placed.x, placed.y))
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

impl<R: UiRules + ?Sized, S: ?Sized> UiRules for OriginalView<'_, R, S> {
    fn ui_image(&self, req: &ImageRequest, assets: &ViewAssets) -> Result<UiSprite, ViewError> {
        self.rules.ui_image(req, assets)
    }

    fn ui_text(&self, req: &TextRequest, assets: &ViewAssets) -> Result<Vec<UiSprite>, ViewError> {
        self.rules.ui_text(req, assets)
    }

    /// Pass 11: everything drawn after the world.
    fn ui_pass(&self) -> Result<u32, ViewError> {
        Ok(crate::scene::order::pass::UI)
    }
}

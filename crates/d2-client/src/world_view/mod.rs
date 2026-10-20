//! World view: the client's model ([`ClientWorld`]) turned into the scene
//! draw list each frame, then ordered and composed (on the CPU as a
//! reference image, on the GPU for play). Plain Rust, no Bevy types
//! outside [`present`].
//!
//! Frames form a cycle: a [`FrameCycle`] keeps the index framebuffer
//! between frames, each frame's clears are `cycle.plan(blank_screen)`
//! (the BlankScreen setting of the player's level,
//! [`ViewFeed::blank_screen`]), and the composed indices become the next
//! frame's base ([`compose_cycle_cpu`], [`GpuAtlas::compose_cycle`]). Areas
//! a level does not clear keep last frame's picture, which is how the game
//! looks there.
//!
//! Units go through the COF composite ([`composite::build_with`]) in
//! unit-key order, frames come from the frame store of resident frame sets
//! ([`ViewAssets::frames`], [`FrameStore`]: `(FrameSetKey, index)` → scene
//! [`FrameId`]), UI requests become items in emission order ([`ui_bind`]),
//! and the list is sorted by draw key ([`scene::order`], stable). Every
//! game rule of the picture (which COF and frame a unit shows, where a
//! sprite goes, draw keys, shading, blend, map tiles, UI art and text
//! layout) is a method of [`ViewRules`] or [`ui_bind::UiRules`].
//! [`Unspecified`] answers them with the narrowest neutral behavior:
//! nothing the model does not state is drawn, and anything that would need
//! a rule to draw is an error, never a default.
//!
//! The client decides no outcome here (the server is authoritative): this
//! module only reads the model.

pub mod anim_clock;
pub mod automap_view;
pub mod background_view;
pub mod corpse_click;
pub mod disguise;
pub mod display_clock;
pub mod feed;
pub mod frame_resize;
pub mod ground_items;
pub mod hw_cursor;
pub mod input_script;
pub mod interact;
pub mod light_sources;
pub mod live_view;
pub mod missiles;
pub mod model_feed;
pub mod monster_label;
pub mod monster_shift;
pub mod monster_walk;
pub mod native_ui;
pub mod native_ui_shots;
pub mod near_rooms;
pub mod node;
pub mod object_click;
pub mod object_label;
pub mod overlay;
pub mod palshift;
pub mod panel_art;
pub mod pointer_input;
pub mod present;
pub mod preview;
pub mod preview_blocks;
pub mod preview_light;
pub mod script_skills;
pub mod sharp;
pub mod shots;
pub mod skill_motion;
pub mod smooth;
pub mod state_tint;
pub mod swap_key;
pub mod tile_assets;
pub mod ui_art;
pub mod ui_bind;
pub mod unit_assets;
pub mod unit_facts;
pub mod unit_motion;
pub mod unit_rules;
pub mod unit_shadow;
pub mod visibility;
pub mod walk;
pub mod walk_room;
pub mod weather_view;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;

use d2_formats::cof::Cof;
use d2_formats::font::FontTable;
use d2_formats::palette::Palette;

use crate::assets::path::CanonicalPath;
use crate::bridge::world::ClientWorld;
use crate::bridge::ClientUnit;
use crate::composite::{self, ComponentFrame, ComponentRequest, CompositeError, UnitParams};
use crate::frames::atlas::AtlasPage;
use crate::frames::{
    Atlas, AtlasError, AtlasSlot, FrameSetKey, FrameStore, IndexFrame, StoreError,
};
use crate::gpu_compositor::{self, Gpu, GpuError};
use crate::scene::{
    self, BlendOp, DrawItem, DrawKey, FrameCycle, FrameId, ItemTag, MapId, MapTable, Rect,
    SceneError, ShadeChain,
};

pub use feed::{
    blank_screen, build_frame, build_frame_placed, camera_and_mode, camera_at, frame_anchor,
    frame_camera, FeedLight, NoCamera, NoFeed, RunningShake, ViewFeed,
};
pub use model_feed::ModelFeed;
pub use present::{
    place_frame, UiSounds, WorldViewGpu, WorldViewPlugin, WorldViewState, WorldViewUi,
};
pub use ui_bind::{
    original_text_font, run_ui, run_ui_with, text_sprites, OriginalTextHooks, TextColors, TextFont,
    TextHooks, UiQueue, UiRules, UiRunError, UiSprite,
};

/// The region composed each frame: the full 800×600 frame. Which part of
/// the world it shows is the camera's, decided by
/// the [`ViewRules`] placement hooks, not by the view rectangle.
pub const VIEW: Rect = Rect::FRAME;

/// The region the play path composes: the whole play frame
/// ([`crate::rules::camera::FrameSize::play`]); [`VIEW`] unless
/// `play --res 640x480` chose the 640 × 480 frame.
pub fn play_view() -> Rect {
    crate::rules::camera::FrameSize::play().rect()
}

/// Errors of a world-view frame. Strict: a frame the rules cannot fully
/// answer fails as a whole; nothing is skipped or defaulted, so a missing
/// rule shows up as an error instead of a wrong picture.
#[derive(Debug, thiserror::Error)]
pub enum ViewError {
    /// A rule hook could not answer; `what` names the hook and `spec` the
    /// rule it is missing.
    #[error("{what} (TODO(spec: {spec})): {message}")]
    Unresolved {
        what: &'static str,
        spec: &'static str,
        message: String,
    },
    #[error("COF {0:?} is not loaded")]
    CofMissing(CanonicalPath),
    #[error("font table {0:?} is not loaded")]
    FontMissing(CanonicalPath),
    #[error(transparent)]
    Text(#[from] crate::ui::TextError),
    /// A frame the frame store does not hold (set not resident, index
    /// past the set's end).
    #[error(transparent)]
    Frame(#[from] StoreError),
    /// The GPU atlas was built for another store (evicting frame sets
    /// would rebuild both; not wired).
    #[error("GPU atlas holds {atlas} frames, the frame store {store}: rebuild the atlas")]
    AtlasAhead { atlas: usize, store: usize },
    #[error("unit ({unit_type}, {guid}): {error}")]
    Unit {
        unit_type: u8,
        guid: u32,
        error: CompositeError,
    },
    #[error("map tile {index}: {error}")]
    Tile { index: usize, error: Box<ViewError> },
    #[error("UI request {index}: {error}")]
    Ui { index: usize, error: Box<ViewError> },
    #[error(transparent)]
    Scene(#[from] SceneError),
    #[error(transparent)]
    Atlas(#[from] AtlasError),
    #[error(transparent)]
    Gpu(#[from] GpuError),
}

impl ViewError {
    /// The error of a hook the neutral rules cannot answer.
    pub fn unresolved(what: &'static str, spec: &'static str) -> Self {
        ViewError::Unresolved {
            what,
            spec,
            message: "the neutral rules draw no unit component".into(),
        }
    }
}

/// Everything a frame reads besides the model: parsed COFs, the frame
/// store of resident frame sets (a set missing here is an error, never a
/// skipped draw: loading decides what is resident), the map table
/// (PL2 rows, blend tables) and the frame palette.
#[derive(Debug, Clone)]
pub struct ViewAssets {
    pub cofs: BTreeMap<CanonicalPath, Cof>,
    /// Parsed font tables (glyph sizes and frames) for UI text.
    pub fonts: BTreeMap<CanonicalPath, FontTable>,
    /// Resident frame sets and their scene ids (verify-map's store: the
    /// same numbering for the CPU reference and the GPU atlas).
    pub frames: FrameStore,
    pub maps: MapTable,
    /// The presented palette: one per frame, no per-region palettes; the
    /// act's `pal.pl2` through [`scene::present_palette`]
    /// ([`ViewAssets::from_pl2`]).
    pub palette: Palette,
    /// The shade tables of the act's PL2 (pushed into `maps`), once a
    /// feed has them: unit shadows read the zero and alpha maps. `None`:
    /// no shadow is drawn.
    pub shades: Option<crate::rules::shading::ShadeTables>,
    /// Row `c` = every entry `c` (`rules::blend::color_row`), rows
    /// `base … base + 255`, once a UI rectangle needs them: a filled UI
    /// rectangle draws through one of these rows
    /// ([`ui_bind::ensure_rects`]).
    pub color_rows: Option<MapId>,
    /// The 8 item palette files' 21 maps each (item colour remaps):
    /// map `c` of file `t` is row `base + 21·(t − 1) + c`, once a UI cel
    /// with an item colour needs them ([`panel_art::PanelArtLoader`]).
    pub item_palettes: Option<MapId>,
    /// The act's 12 text-colour maps once pushed ([`TextColors`]): the
    /// rendering facts read a glyph's colour index from its shade map.
    pub text_colors: Option<TextColors>,
    /// Each resident DT1 tile's header `orientation.main.sub.rarity`, for
    /// the rendering facts' `tile` cell.
    pub tile_ids: BTreeMap<FrameSetKey, [u32; 4]>,
}

impl ViewAssets {
    /// No COFs or frames, an empty map table.
    pub fn new(palette: Palette) -> Self {
        ViewAssets {
            cofs: BTreeMap::new(),
            fonts: BTreeMap::new(),
            frames: FrameStore::new(),
            maps: MapTable::new(),
            palette,
            shades: None,
            color_rows: None,
            item_palettes: None,
            text_colors: None,
            tile_ids: BTreeMap::new(),
        }
    }

    /// [`ViewAssets::new`] with the presented palette of an act's
    /// `pal.pl2` (its first 1,024 bytes are the base palette).
    pub fn from_pl2(pl2: &[u8]) -> Result<Self, ViewError> {
        Ok(Self::new(scene::present_palette(pl2)?))
    }

    /// The scene id of frame `index` of the resident set `key`.
    pub fn id(&self, key: &FrameSetKey, index: usize) -> Result<FrameId, ViewError> {
        Ok(self.frames.id(key, index)?)
    }

    /// Frame `index` of the resident set `key`.
    pub fn frame(&self, key: &FrameSetKey, index: usize) -> Result<&IndexFrame, ViewError> {
        let id = self.id(key, index)?;
        Ok(self
            .frames
            .frame(id)
            .expect("the store gave the id, so it holds the frame"))
    }
}

/// Which animation a unit shows this frame: a COF and the COF direction
/// and frame (the sim owns animation state; the client never advances it
/// on its own).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitPose {
    /// Fraction of the starting source interval, in 1/256 units.
    pub sub_frame: u8,
    pub cof: CanonicalPath,
    pub dir: usize,
    pub frame: usize,
    /// The unit's 64-step direction after snapping to the COF's direction
    /// count: the direction the cel draws with, reported in the rendering
    /// facts.
    pub dir64: u8,
}

/// One map tile draw, fully answered by [`ViewRules::tiles`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileDraw {
    /// The DT1 tile frame set and frame.
    pub frame: ComponentFrame,
    /// Screen top-left, after the camera.
    pub x: i32,
    pub y: i32,
    pub clip: Rect,
    pub shade: ShadeChain,
    pub blend: BlendOp,
    pub key: DrawKey,
    /// Debug label (`ItemTag::Tile`).
    pub cell: (i32, i32),
    /// The tile's light digest for the rendering facts
    /// (`ViewSource::tile_light`); `None`: not measured.
    pub light: Option<String>,
    /// The wall's alpha byte (`ViewSource::tile_alpha`); `None`: not
    /// measured.
    pub alpha: Option<u8>,
}

/// A composite slot whose file is in no archive ([`ViewRules::unit_slot_calls`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotCall {
    /// The slot index (the draw key's `sub`).
    pub slot: u8,
    /// The COF layer of the slot's component.
    pub layer: usize,
    /// The component file the call names.
    pub path: CanonicalPath,
}

/// A cel draw call that puts no pixel in the frame: a [`SlotCall`] keyed
/// in the unit's pass (or its shadow pass). Logged in
/// [`WorldFrame::unit_calls`] for the rendering facts, never composed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitCall {
    /// The cel call's mode and palette (`ViewRules::component_call`).
    pub call: Option<crate::scene::CelArgs>,
    pub key: DrawKey,
    pub tag: ItemTag,
    /// The cel file; `None`: the unit draw call alone, its body culled by
    /// the COF box pre-test.
    pub path: Option<CanonicalPath>,
    /// The cel context's direction ([`UnitPose::dir64`]).
    pub dir64: u8,
    pub frame: usize,
    /// The shadow pass call (`CelDrawShadow`).
    pub shadow: bool,
}

impl TileDraw {
    /// A drawer call that puts no pixel in the frame (empty clip): logged
    /// in [`WorldFrame::calls`] for the rendering facts, never composed.
    pub fn is_call_only(&self) -> bool {
        self.clip.width == 0 || self.clip.height == 0
    }
}

/// The game-rule questions of the world view, one hook per question: what
/// the picture shows, as opposed to how this module builds it.
pub trait ViewRules {
    /// The map tiles to draw and how: which records (the near rooms and
    /// their tile records, from the client's level layout), their screen
    /// position and culling. `rules::OriginalView` answers it from its
    /// `ViewSource`.
    fn tiles(&self, world: &ClientWorld, assets: &ViewAssets) -> Result<Vec<TileDraw>, ViewError>;

    /// Which COF, COF direction and frame the unit shows, from the unit's
    /// mode, direction and frame as the server sent them. `None` = the
    /// unit is not drawn.
    fn unit_pose(
        &self,
        world: &ClientWorld,
        unit: &ClientUnit,
    ) -> Result<Option<UnitPose>, ViewError>;

    /// The draw key (pass, major, minor) and clip (the frame) shared by
    /// the unit's components.
    fn unit_params(
        &self,
        world: &ClientWorld,
        unit: &ClientUnit,
        pose: &UnitPose,
    ) -> Result<UnitParams, ViewError>;

    /// The component's frame set (file path from token, variant, mode, weapon class; file
    /// direction) and frame.
    fn component_frame(
        &self,
        unit: &ClientUnit,
        pose: &UnitPose,
        req: &ComponentRequest<'_>,
    ) -> Result<ComponentFrame, CompositeError>;

    /// Whether the composite of `unit` with
    /// `pose` passes the COF box pre-test at its final screen position.
    /// The default (no camera) passes every unit.
    fn unit_box_visible(
        &self,
        _unit: &ClientUnit,
        _pose: &UnitPose,
        _cof: &Cof,
    ) -> Result<bool, ViewError> {
        Ok(true)
    }

    /// Whether the shadow of `unit` passes its own pre-test (the sheared
    /// box: a shadow can be on screen while its body is not). The default (no
    /// camera) passes every unit.
    fn unit_shadow_box_visible(
        &self,
        _unit: &ClientUnit,
        _pose: &UnitPose,
        _cof: &Cof,
    ) -> Result<bool, ViewError> {
        Ok(true)
    }

    /// The unit's shadow draws, keyed at the shadow pass slot `at` (`None`
    /// until `OriginalView` fills it from the frame's draw order), from the
    /// component draws `draws` the unit just built. The default draws
    /// none.
    fn unit_shadows(
        &self,
        _world: &ClientWorld,
        _unit: &ClientUnit,
        _pose: &UnitPose,
        _at: Option<crate::rules::draw_order::OrderKey>,
        _draws: &[composite::ComponentDraw],
        _assets: &ViewAssets,
    ) -> Result<Vec<DrawItem>, ViewError> {
        Ok(Vec::new())
    }

    /// The slots of `unit`'s composite whose component request succeeds
    /// but whose file is in no archive: they draw nothing, but still count
    /// as a cel draw call (and a shadow call), which the rendering facts
    /// log. The default has none.
    fn unit_slot_calls(
        &self,
        _unit: &ClientUnit,
        _pose: &UnitPose,
        _cof: &Cof,
    ) -> Result<Vec<SlotCall>, ViewError> {
        Ok(Vec::new())
    }

    /// The unit's shadow pass slot, the key of
    /// its shadow calls; `None` (the default) draws no shadow call.
    fn unit_shadow_key(&self, _unit: &ClientUnit) -> Option<crate::rules::draw_order::OrderKey> {
        None
    }

    /// Whether the unit draw reaches the body's cels: `false` for an
    /// object whose `objects` row has `Draw` = 0. The shadow pass does not
    /// read `Draw`. The default: always.
    fn unit_draws_body(&self, _unit: &ClientUnit) -> bool {
        true
    }

    /// The unit's slot when the frame's draw order lists it (the walk
    /// over the room lists runs the unit draw for every listed unit);
    /// `None` (the default): not listed, or no draw order.
    fn unit_listed_key(&self, _unit: &ClientUnit) -> Option<crate::rules::draw_order::OrderKey> {
        None
    }

    /// The light word the unit draw and its cel calls use, for the
    /// rendering facts; `None` (the default): not measured.
    fn unit_light_word(&self, _unit: &ClientUnit) -> Option<u32> {
        None
    }

    /// The unit's client pixel point, where the unit draw places it, for
    /// the rendering facts; `None` (the default): not measured.
    fn unit_client_point(&self, _unit: &ClientUnit) -> Option<(i32, i32)> {
        None
    }

    /// The draw mode and palette argument of a component's cel call, for
    /// the rendering facts; `None` (the default): not measured.
    fn component_call(
        &self,
        _unit: &ClientUnit,
        _req: &ComponentRequest<'_>,
    ) -> Option<crate::scene::CelArgs> {
        None
    }

    /// The `transparent` / `transtbl` of the item record whose code is the
    /// component's armor-class token (an item's own transparency settings
    /// carry over to the component that shows it); `None` (the default):
    /// no such item, or not known.
    fn component_item_trans(
        &self,
        _unit: &ClientUnit,
        _req: &ComponentRequest<'_>,
    ) -> Option<(u8, u8)> {
        None
    }

    /// The component's frame, or `None` when the slot draws nothing
    /// (a failed component request, or a missing file). The default draws
    /// every slot with
    /// [`ViewRules::component_frame`].
    fn component_slot_frame(
        &self,
        unit: &ClientUnit,
        pose: &UnitPose,
        req: &ComponentRequest<'_>,
    ) -> Result<Option<ComponentFrame>, CompositeError> {
        self.component_frame(unit, pose, req).map(Some)
    }

    /// Screen top-left of the component image: the unit's position through
    /// the camera, then the frame's own offsets (`image.x_off/y_off`).
    fn place(
        &self,
        unit: &ClientUnit,
        pose: &UnitPose,
        req: &ComponentRequest<'_>,
        image: &IndexFrame,
    ) -> Result<(i32, i32), CompositeError>;

    /// The component's colour remap `P` and light map `L`
    /// (`rules::lighting::view::LitRules`).
    fn shade(
        &self,
        unit: &ClientUnit,
        req: &ComponentRequest<'_>,
    ) -> Result<ShadeChain, CompositeError>;

    /// The component's draw mode (with the
    /// COF layer override) as a blend op.
    fn blend(
        &self,
        unit: &ClientUnit,
        req: &ComponentRequest<'_>,
    ) -> Result<BlendOp, CompositeError>;
}

/// The neutral rules while the model lacks the inputs the game rules
/// read: no map tiles, no unit drawn (the model states nothing a unit
/// looks like: equipped items, component choices), and every hook that
/// would be needed to draw something is an error. A frame built with these
/// is the empty list (plus nothing from the UI unless a panel draws, which
/// is then an error).
#[derive(Debug, Clone, Copy, Default)]
pub struct Unspecified;

fn component_unresolved(req: &ComponentRequest<'_>, what: &'static str) -> CompositeError {
    CompositeError::Unresolved {
        slot: req.slot.slot,
        component: req.slot.component,
        what,
        message: "the neutral rules draw no unit component".into(),
    }
}

impl ViewRules for Unspecified {
    fn tiles(&self, _: &ClientWorld, _: &ViewAssets) -> Result<Vec<TileDraw>, ViewError> {
        Ok(Vec::new())
    }

    fn unit_pose(&self, _: &ClientWorld, _: &ClientUnit) -> Result<Option<UnitPose>, ViewError> {
        Ok(None)
    }

    fn unit_params(
        &self,
        _: &ClientWorld,
        _: &ClientUnit,
        _: &UnitPose,
    ) -> Result<UnitParams, ViewError> {
        Err(ViewError::unresolved(
            "unit draw key",
            "render/draw-order.md",
        ))
    }

    fn component_frame(
        &self,
        _: &ClientUnit,
        _: &UnitPose,
        req: &ComponentRequest<'_>,
    ) -> Result<ComponentFrame, CompositeError> {
        Err(component_unresolved(req, "component frame"))
    }

    fn place(
        &self,
        _: &ClientUnit,
        _: &UnitPose,
        req: &ComponentRequest<'_>,
        _: &IndexFrame,
    ) -> Result<(i32, i32), CompositeError> {
        Err(component_unresolved(req, "placement"))
    }

    fn shade(
        &self,
        _: &ClientUnit,
        req: &ComponentRequest<'_>,
    ) -> Result<ShadeChain, CompositeError> {
        Err(component_unresolved(req, "shade"))
    }

    fn blend(&self, _: &ClientUnit, req: &ComponentRequest<'_>) -> Result<BlendOp, CompositeError> {
        Err(component_unresolved(req, "blend"))
    }
}

/// One built frame: the ordered draw list. Its frame ids are
/// [`ViewAssets::frames`] ids.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorldFrame {
    /// Sorted by [`DrawKey`] (stable: equal keys keep build order).
    pub items: Vec<DrawItem>,
    /// Units drawn, and units the rules left undrawn (`unit_pose` = None).
    pub units_drawn: usize,
    pub units_hidden: usize,
    /// The camera the frame was placed with; `None` without one. Read
    /// back by [`visibility`] (the view origin the client reports is the
    /// last drawn frame's).
    pub camera: Option<crate::rules::camera::Camera>,
    /// Pass 9's calls in call order (the weather pass: the lightning flash
    /// or the rain and snow lines), drawn as the items of
    /// [`weather_view::is_sky_call_path`]; read by the facts export, which
    /// writes one row per call.
    pub sky: Vec<crate::rules::draw_order::weather::SkyDraw>,
    /// Tile drawer calls that put no pixel in the frame
    /// ([`TileDraw::is_call_only`]), as items with an empty clip, sorted
    /// by key; not composed. Read by the facts export.
    pub calls: Vec<DrawItem>,
    /// The cel draw kind of each UI draw, by its index in the frame's UI
    /// list (the items' `ItemTag::Ui`): glyphs of a text are
    /// `CelDrawColor`. Read by the facts export.
    pub ui_calls: Vec<crate::ui::draw::UiCelInfo>,
    /// Each drawn unit's cel context direction ([`UnitPose::dir64`]) by
    /// its draw slot ([`DrawKey::slot`] of its pass key and of its shadow
    /// key; a GUID is unique per unit type only), for the rendering facts.
    pub unit_dirs: BTreeMap<u64, u8>,
    /// Each listed unit's unit-draw arguments by the draw slot of its pass
    /// key, for the rendering facts.
    pub unit_rows: BTreeMap<u64, UnitRowArgs>,
    /// Each tile's light digest by its draw key (`TileDraw::light`), for
    /// the rendering facts.
    pub tile_lights: BTreeMap<DrawKey, String>,
    /// Each translucent wall's alpha byte by its draw key
    /// (`TileDraw::alpha`): the `TileDrawTrans` `mode`.
    pub tile_alphas: BTreeMap<DrawKey, u8>,
    /// Cel draw calls without pixels (a component file in no archive),
    /// sorted by key; not composed. Read by the facts export.
    pub unit_calls: Vec<UnitCall>,
    /// Each drawn unit's body cels by COF layer, for the hover layer
    /// test ([`crate::bridge::hover::pick_layers`]).
    pub unit_layers: crate::bridge::hover::UnitLayers,
    /// The listed units whose body failed the COF box pre-test: the
    /// composite stops before its overlays, so their overlays are not
    /// drawn either.
    pub body_culled: std::collections::BTreeSet<crate::bridge::world::UnitKey>,
    /// The units whose unit draw draws nothing (no pose, `Draw` 0, or the
    /// body failed the box pre-test): only a unit that drew gets its
    /// "drawn" flag (0x10000000), so the automap does not see these drawn
    /// ([`near_rooms::MapState::units_undrawn`]).
    pub draw_zero: std::collections::BTreeSet<crate::bridge::world::UnitKey>,
    /// The units' draw slots of the frame's draw order; `None` when no
    /// order was computed (no map feed).
    /// The layers drawn after the build (ground items) take their keys
    /// from it.
    pub slots: Option<BTreeMap<crate::bridge::world::UnitKey, crate::rules::draw_order::UnitSlot>>,
    /// The units the frame runs the unit draw for (listed, with or
    /// without a pose), in build order: each takes one from its hit flash
    /// counter after the frame ([`crate::bridge::Bridge::take_hit_flashes`]).
    pub unit_draws: Vec<crate::bridge::world::UnitKey>,
    /// The highlight pre-pass of each unit drawn with a non-zero hit
    /// flash counter, by the draw slot of its pass
    /// key: its body cels in slot order, the cels without pixels (a file
    /// in no archive) included. Calls without pixels: not composed; the
    /// facts export writes one `CelDrawHilight` row per cel after the
    /// unit row.
    pub hit_flash: BTreeMap<u64, HitFlash>,
    /// The moving unit (player, monster, missile) each draw slot belongs
    /// to: the slots of its body, its shadow and the overlays drawn with
    /// it. Between ticks the items of a slot move with their unit
    /// ([`unit_motion`]).
    pub movers: BTreeMap<u64, crate::bridge::world::UnitKey>,
    /// Each posed unit's drawn COF frame ([`anim_clock`]): what the
    /// animation-rate check follows over ticks.
    pub unit_frames: BTreeMap<crate::bridge::world::UnitKey, anim_clock::DrawnFrame>,
}

/// One unit's hit flash pre-pass: the unit's body drawn once more in a
/// flat colour under the body, for the frames after it was hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HitFlash {
    /// The flat colour of the pre-pass: the act palette's index
    /// nearest to the life colour of [`hit_flash_rgb`]; `None` without a
    /// palette match.
    pub color: Option<u8>,
    /// The body cels in slot order.
    pub cels: Vec<HitFlashCel>,
}

/// The hit flash colour: blue for a class with `monstats` `interact`; else by life percent
/// `stat 6 × 100 / stat 7` (max life 0 → green): under 33 red, under 66
/// yellow, else green.
pub fn hit_flash_rgb(unit: &ClientUnit) -> (u8, u8, u8) {
    if unit.hit_flash_interact {
        return (0, 0, 0xC0);
    }
    let max = i64::from(unit.stat(7));
    if max != 0 {
        let pct = i64::from(unit.stat(6)) * 100 / max;
        if pct < 33 {
            return (0xE0, 0, 0);
        }
        if pct < 66 {
            return (0xE0, 0xE0, 0);
        }
    }
    (0, 0xC0, 0)
}

/// One cel of a hit flash pre-pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HitFlashCel {
    /// A composite cel with pixels (its frame).
    Frame(crate::scene::FrameId),
    /// A slot whose file is in no archive: the file and the cel frame.
    Call { path: CanonicalPath, frame: usize },
}

/// The unit draw's arguments of one listed unit, for the rendering facts:
/// its type, client point and light word; `None` where the rules do not
/// measure it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UnitRowArgs {
    pub unit_type: u8,
    pub point: Option<(i32, i32)>,
    pub light: Option<u32>,
}

impl UnitRowArgs {
    fn of<R: ViewRules + ?Sized>(rules: &R, unit: &ClientUnit) -> UnitRowArgs {
        UnitRowArgs {
            unit_type: unit.key.unit_type,
            point: rules.unit_client_point(unit),
            light: rules.unit_light_word(unit),
        }
    }
}

/// The COF composite's resolver for one unit: the hooks. Frame ids are not a hook: they
/// come from the frame store ([`composite::build_with`]).
struct UnitResolver<'a, R: ?Sized> {
    rules: &'a R,
    unit: &'a ClientUnit,
    pose: &'a UnitPose,
    assets: &'a ViewAssets,
}

fn not_resident(req: &ComponentRequest<'_>, what: &'static str, e: ViewError) -> CompositeError {
    CompositeError::Unresolved {
        slot: req.slot.slot,
        component: req.slot.component,
        what,
        message: e.to_string(),
    }
}

impl<R: ViewRules + ?Sized> composite::ComponentResolver for UnitResolver<'_, R> {
    fn frame(&self, req: &ComponentRequest<'_>) -> Result<ComponentFrame, CompositeError> {
        self.rules.component_frame(self.unit, self.pose, req)
    }

    fn slot_frame(
        &self,
        req: &ComponentRequest<'_>,
    ) -> Result<Option<ComponentFrame>, CompositeError> {
        self.rules.component_slot_frame(self.unit, self.pose, req)
    }

    fn place(
        &self,
        req: &ComponentRequest<'_>,
        frame: &ComponentFrame,
    ) -> Result<(i32, i32), CompositeError> {
        let image = self
            .assets
            .frame(&frame.set, frame.index)
            .map_err(|e| not_resident(req, "place", e))?;
        self.rules.place(self.unit, self.pose, req, image)
    }

    fn shade(&self, req: &ComponentRequest<'_>) -> Result<ShadeChain, CompositeError> {
        self.rules.shade(self.unit, req)
    }

    fn blend(&self, req: &ComponentRequest<'_>) -> Result<BlendOp, CompositeError> {
        self.rules.blend(self.unit, req)
    }
}

/// The screen rectangle `(left, top, right, bottom)` of each unit's body
/// cels as [`build`] composites them (shadows left out), by unit, without
/// the map or a draw order: the frame rectangle the hover test widens,
/// for a run that draws no frame (`state-dump --input`), and the drawn
/// layers of its layer test. A unit whose pose, COF or composite does not resolve
/// has no rectangle; one log line each.
pub fn unit_body_rects<R: ViewRules + ?Sized>(
    world: &ClientWorld,
    rules: &R,
    assets: &ViewAssets,
) -> (
    crate::bridge::hover::UnitRects,
    crate::bridge::hover::UnitLayers,
    Vec<String>,
) {
    let mut out = BTreeMap::new();
    let mut layers = BTreeMap::new();
    let mut log = Vec::new();
    for unit in world.view_units() {
        let unit = &*unit;
        let pose = match rules.unit_pose(world, unit) {
            Ok(Some(p)) => p,
            Ok(None) => continue,
            Err(e) => {
                log.push(format!("unit {:?}: {e}", unit.key));
                continue;
            }
        };
        let Some(cof) = assets.cofs.get(&pose.cof) else {
            continue;
        };
        let visible = rules.unit_box_visible(unit, &pose, cof);
        if !rules.unit_draws_body(unit) || !matches!(visible, Ok(true)) {
            continue;
        }
        let params = match rules.unit_params(world, unit, &pose) {
            Ok(p) => p,
            Err(e) => {
                log.push(format!("unit {:?}: {e}", unit.key));
                continue;
            }
        };
        let resolver = UnitResolver {
            rules,
            unit,
            pose: &pose,
            assets,
        };
        let draws = match composite::build_with(
            cof,
            pose.dir,
            pose.frame,
            &params,
            &resolver,
            &assets.frames,
        ) {
            Ok(d) => d,
            Err(e) => {
                log.push(format!("unit {:?}: {e}", unit.key));
                continue;
            }
        };
        for d in &draws {
            let Some(f) = assets.frames.frame(d.item.frame) else {
                continue;
            };
            let (l, t) = (d.item.x, d.item.y);
            let (r, b) = (l + f.width as i32, t + f.height as i32);
            out.entry(unit.key)
                .and_modify(|e: &mut (i32, i32, i32, i32)| {
                    *e = (e.0.min(l), e.1.min(t), e.2.max(r), e.3.max(b))
                })
                .or_insert((l, t, r, b));
        }
        layers.insert(unit.key, layer_rects(cof, &draws, &assets.frames));
    }
    (out, layers, log)
}

/// The hover layer of each drawn body cel of a unit: its COF layer and component, the layer's `Selectable` byte, the
/// drawn cel.
fn layer_rects(
    cof: &Cof,
    draws: &[composite::ComponentDraw],
    frames: &crate::frames::FrameStore,
) -> Vec<crate::bridge::hover::LayerRect> {
    draws
        .iter()
        .filter_map(|d| {
            let f = frames.frame(d.item.frame)?;
            let (l, t) = (d.item.x, d.item.y);
            Some(crate::bridge::hover::LayerRect {
                layer: d.slot.layer,
                component: d.slot.component,
                selectable: cof.layers.get(d.slot.layer)?.selectable != 0,
                rect: (l, t, l + f.width as i32, t + f.height as i32),
            })
        })
        .collect()
}

/// Builds and orders the frame's draw list: map tiles, then units in key
/// order (each through the COF composite), then the UI requests in
/// emission order; then the stable sort by key. Frame ids are the frame
/// store's. Any error fails the frame.
pub fn build<R: ViewRules + UiRules + ?Sized>(
    world: &ClientWorld,
    ui: &[crate::ui::UiDraw],
    rules: &R,
    assets: &ViewAssets,
) -> Result<WorldFrame, ViewError> {
    let mut items = Vec::new();
    let mut calls = Vec::new();
    let mut tile_lights = BTreeMap::new();
    let mut tile_alphas = BTreeMap::new();

    for (index, t) in rules.tiles(world, assets)?.into_iter().enumerate() {
        let at = |error| ViewError::Tile {
            index,
            error: Box::new(error),
        };
        let id = assets.id(&t.frame.set, t.frame.index).map_err(at)?;
        if let Some(l) = &t.light {
            tile_lights.insert(t.key, l.clone());
        }
        if let Some(a) = t.alpha {
            tile_alphas.insert(t.key, a);
        }
        let mut item = DrawItem::new(id, t.x, t.y);
        item.clip = t.clip;
        item.shade = t.shade;
        item.blend = t.blend;
        item.key = t.key;
        item.tag = ItemTag::Tile {
            x: t.cell.0,
            y: t.cell.1,
        };
        if t.is_call_only() {
            calls.push(item);
        } else {
            items.push(item);
        }
    }

    let (mut units_drawn, mut units_hidden) = (0, 0);
    let mut unit_dirs = BTreeMap::new();
    let mut movers = BTreeMap::new();
    let mut unit_rows = BTreeMap::new();
    let mut unit_calls = Vec::new();
    let mut body_culled = std::collections::BTreeSet::new();
    let mut draw_zero = std::collections::BTreeSet::new();
    let mut unit_layers = BTreeMap::new();
    let mut unit_draws = Vec::new();
    let mut hit_flash = BTreeMap::new();
    let mut unit_frames = BTreeMap::new();
    for unit in world.view_units() {
        let unit = &*unit;
        let Some(pose) = rules.unit_pose(world, unit)? else {
            // A listed unit without a pose (no COF, e.g. object 78
            // `invisible town sound`, token TA, `Draw` 0) still gets the
            // unit draw call; it draws no cel.
            if let Some(at) = rules.unit_listed_key(unit) {
                let key = DrawKey::new(at.pass, at.major, at.minor, 0).map_err(ViewError::Scene)?;
                unit_rows.insert(key.slot(), UnitRowArgs::of(rules, unit));
                unit_draws.push(unit.key);
                unit_calls.push(UnitCall {
                    key,
                    tag: ItemTag::Unit(unit.key.guid),
                    path: None,
                    dir64: 0,
                    frame: 0,
                    shadow: false,
                    call: None,
                });
            }
            draw_zero.insert(unit.key);
            units_hidden += 1;
            continue;
        };
        let cof = assets
            .cofs
            .get(&pose.cof)
            .ok_or_else(|| ViewError::CofMissing(pose.cof.clone()))?;
        unit_frames.insert(
            unit.key,
            anim_clock::DrawnFrame {
                class: unit.class,
                mode: unit.mode,
                frame: u16::try_from(pose.frame).unwrap_or(u16::MAX),
                frames: u16::from(cof.frames),
            },
        );
        // The body's COF box pre-test does not cull the unit's shadow:
        // the shadow has its own sheared box.
        let draws_body = rules.unit_draws_body(unit);
        let body = draws_body && rules.unit_box_visible(unit, &pose, cof)?;
        if draws_body && !body {
            body_culled.insert(unit.key);
        }
        if !body {
            draw_zero.insert(unit.key);
        }
        let shadow_ok = rules.unit_shadow_box_visible(unit, &pose, cof)?;
        let shadow_at = rules.unit_shadow_key(unit).filter(|_| shadow_ok);
        let params = rules.unit_params(world, unit, &pose)?;
        if let Ok(k) = DrawKey::new(params.pass, params.major, params.minor, 0) {
            unit_rows.insert(k.slot(), UnitRowArgs::of(rules, unit));
        }
        unit_draws.push(unit.key);
        // A listed unit whose body then fails the COF box pre-test still
        // gets the unit draw call: the call alone, no cel (e.g. a torch
        // just off screen).
        if !body {
            unit_calls.push(UnitCall {
                key: DrawKey::new(params.pass, params.major, params.minor, 0)
                    .map_err(ViewError::Scene)?,
                tag: params.tag,
                path: None,
                dir64: pose.dir64,
                frame: pose.frame,
                shadow: false,
                call: None,
            });
        }
        if !body && shadow_at.is_none() {
            units_hidden += 1;
            continue;
        }
        let resolver = UnitResolver {
            rules,
            unit,
            pose: &pose,
            assets,
        };
        let draws = composite::build_with(
            cof,
            pose.dir,
            pose.frame,
            &params,
            &resolver,
            &assets.frames,
        )
        .map_err(|error| ViewError::Unit {
            unit_type: unit.key.unit_type,
            guid: unit.key.guid,
            error,
        })?;
        let mut draws = draws;
        for d in &mut draws {
            let req = ComponentRequest {
                cof,
                dir: pose.dir,
                frame: pose.frame,
                slot: d.slot,
                layer: &cof.layers[d.slot.layer],
            };
            d.item.call = rules.component_call(unit, &req);
        }
        // A non-zero hit flash counter draws the pre-pass of the body cels, except in state 143
        // `attached`.
        let flashes = body && unit.hit_flash > 0 && !unit.states.contains(&143);
        let mut flash_cels = Vec::new();
        let shadows = if shadow_ok {
            rules.unit_shadows(world, unit, &pose, None, &draws, assets)?
        } else {
            Vec::new()
        };
        if body {
            unit_layers.insert(unit.key, layer_rects(cof, &draws, &assets.frames));
            if flashes {
                flash_cels.extend(
                    draws
                        .iter()
                        .map(|d| (d.slot.slot, HitFlashCel::Frame(d.item.frame))),
                );
            }
            items.extend(draws.into_iter().map(|d| d.item));
        }
        items.extend(shadows);
        if let Ok(k) = DrawKey::new(params.pass, params.major, params.minor, 0) {
            unit_dirs.insert(k.slot(), pose.dir64);
            if unit_motion::moves(unit.key) {
                movers.insert(k.slot(), unit.key);
            }
        }
        if let Some(Ok(k)) = shadow_at.map(|a| DrawKey::new(a.pass, a.major, a.minor, 0)) {
            unit_dirs.insert(k.slot(), pose.dir64);
            if unit_motion::moves(unit.key) {
                movers.insert(k.slot(), unit.key);
            }
        }
        for c in rules.unit_slot_calls(unit, &pose, cof)? {
            let args = rules.component_call(
                unit,
                &ComponentRequest {
                    cof,
                    dir: pose.dir,
                    frame: pose.frame,
                    slot: composite::Slot {
                        slot: c.slot,
                        component: cof.layers[c.layer].component,
                        layer: c.layer,
                    },
                    layer: &cof.layers[c.layer],
                },
            );
            let call = |pass, major, minor, shadow| -> Result<UnitCall, ViewError> {
                Ok(UnitCall {
                    key: DrawKey::new(pass, major, minor, c.slot).map_err(ViewError::Scene)?,
                    tag: params.tag,
                    path: Some(c.path.clone()),
                    dir64: pose.dir64,
                    frame: pose.frame,
                    shadow,
                    call: args,
                })
            };
            if body {
                unit_calls.push(call(params.pass, params.major, params.minor, false)?);
            }
            if flashes {
                flash_cels.push((
                    c.slot,
                    HitFlashCel::Call {
                        path: c.path.clone(),
                        frame: pose.frame,
                    },
                ));
            }
            if let (Some(at), true) = (shadow_at, cof.layers[c.layer].shadow != 0) {
                unit_calls.push(call(at.pass, at.major, at.minor, true)?);
            }
        }
        if flashes {
            if let Ok(k) = DrawKey::new(params.pass, params.major, params.minor, 0) {
                flash_cels.sort_by_key(|(slot, _)| *slot);
                let palette: Vec<[u8; 3]> = assets
                    .palette
                    .colors
                    .iter()
                    .map(|c| [c.r, c.g, c.b])
                    .collect();
                hit_flash.insert(
                    k.slot(),
                    HitFlash {
                        color: crate::ui::inv_grid::nearest_index(&palette, hit_flash_rgb(unit)),
                        cels: flash_cels.into_iter().map(|(_, c)| c).collect(),
                    },
                );
            }
        }
        if body {
            units_drawn += 1;
        } else {
            units_hidden += 1;
        }
    }

    ui_bind::ui_items(ui, rules, assets, &mut items)?;
    let ui_calls = ui
        .iter()
        .map(|d| match d {
            crate::ui::UiDraw::Image(r) => crate::ui::draw::UiCelInfo {
                call: r.call,
                mode: r.look.mode,
                pal: match r.look.remap {
                    crate::ui::Remap::None => Some(0),
                    crate::ui::Remap::Palette(k) => Some(k),
                    crate::ui::Remap::ItemColor { .. } => None,
                },
                text: false,
            },
            crate::ui::UiDraw::Text(t) => crate::ui::draw::UiCelInfo {
                // Vertical text draws each glyph with CelDrawEx, other
                // text with CelDrawColor.
                call: match t.opts {
                    crate::ui::TextOpts::Vertical { .. } => crate::ui::draw::CelCall::Ex,
                    _ => crate::ui::draw::CelCall::Color,
                },
                mode: t.opts.mode(),
                pal: None,
                text: true,
            },
            crate::ui::UiDraw::Rect(_) | crate::ui::UiDraw::Line(_) => crate::ui::draw::UiCelInfo {
                call: crate::ui::draw::CelCall::Draw,
                mode: 5,
                pal: Some(0),
                text: false,
            },
        })
        .collect();

    scene::order(&mut items);
    Ok(WorldFrame {
        items,
        units_drawn,
        units_hidden,
        camera: None,
        sky: Vec::new(),
        calls: {
            scene::order(&mut calls);
            calls
        },
        unit_dirs,
        unit_rows,
        tile_lights,
        tile_alphas,
        ui_calls,
        unit_calls: {
            unit_calls.sort_by_key(|c| c.key);
            unit_calls
        },
        slots: None,
        unit_layers,
        body_culled,
        draw_zero,
        unit_draws,
        hit_flash,
        movers,
        unit_frames,
    })
}

/// CPU reference image of a built frame: RGBA8, alpha 255,
/// [`play_view`] sized.
pub fn compose_cpu(frame: &WorldFrame, assets: &ViewAssets) -> Result<Vec<u8>, ViewError> {
    Ok(scene::compose_rgba(
        &frame.items,
        &assets.frames,
        &assets.maps,
        &assets.palette,
        play_view(),
    )?)
}

/// One frame of the frame cycle on the CPU reference: `cycle.compose` with the plan of `blank_screen`, the draws onto
/// the persistent framebuffer; returns the presented RGBA8 image through
/// the frame palette. The cycle must be [`play_view`] sized. On error the
/// cycle is unchanged.
pub fn compose_cycle_cpu(
    cycle: &mut FrameCycle,
    blank_screen: bool,
    frame: &WorldFrame,
    assets: &ViewAssets,
) -> Result<Vec<u8>, ViewError> {
    check_cycle(cycle)?;
    let indices = cycle.compose(blank_screen, &frame.items, &assets.frames, &assets.maps)?;
    Ok(scene::to_rgba(indices, &assets.palette))
}

/// The world view composes [`play_view`]; a cycle of another size has no
/// frame mapping.
fn check_cycle(cycle: &FrameCycle) -> Result<(), ViewError> {
    let view = play_view();
    if cycle.view() != view {
        return Err(SceneError::BaseSize {
            len: cycle.pixels().len(),
            pixels: u64::from(view.width) * u64::from(view.height),
        }
        .into());
    }
    Ok(())
}

/// The atlas the GPU compositor reads: a cache of the frame store's frames
/// on the GPU's pages. `slots[n]` is where `FrameId(n)` is, `None` while
/// it is not placed. A frame is placed the first time a draw list needs it
/// ([`GpuAtlas::ensure_for`]) and never moves while it stays. The pages
/// hold a fixed budget (`max_pages`), while the store keeps every frame
/// set the session loaded (every level, act and monster seen): when the
/// frames a draw list needs no longer fit beside the ones placed before,
/// the atlas starts over empty and places only that draw list's frames,
/// so changing levels never runs out of room. A start-over changes every
/// page ([`GpuAtlas::version`]), so the next job uploads them again.
#[derive(Debug, Clone)]
pub struct GpuAtlas {
    atlas: Atlas,
    slots: Vec<Option<AtlasSlot>>,
    /// Frames placed since the atlas last started over.
    placed: usize,
    /// Changes whenever the pages do: frames placed, or a start-over.
    version: u64,
    /// Times the atlas started over (logs and tests).
    restarts: u64,
}

impl GpuAtlas {
    pub fn new(max_pages: u32) -> Result<Self, ViewError> {
        Ok(GpuAtlas {
            atlas: Atlas::new(max_pages)?,
            slots: Vec::new(),
            placed: 0,
            version: 0,
            restarts: 0,
        })
    }

    /// Places every frame of `store` that is not placed yet, in id order
    /// (a whole store that must fit: tests and the check tools).
    pub fn ensure(&mut self, store: &FrameStore) -> Result<(), ViewError> {
        self.track(store)?;
        let missing: Vec<usize> = (0..store.len())
            .filter(|&n| self.slots[n].is_none())
            .collect();
        self.place(store, &missing)
    }

    /// Places the frames `items` draw that are not placed yet, in id
    /// order. When they do not fit beside the frames placed before, the
    /// atlas starts over with only the frames `items` draw; only a draw
    /// list whose own frames exceed the whole budget is an error.
    pub fn ensure_for(&mut self, store: &FrameStore, items: &[DrawItem]) -> Result<(), ViewError> {
        self.track(store)?;
        let mut wanted: Vec<usize> = items.iter().map(|i| i.frame.0 as usize).collect();
        wanted.sort_unstable();
        wanted.dedup();
        // A frame the store does not hold is the packer's error to report.
        wanted.retain(|&n| n < store.len());
        let missing: Vec<usize> = wanted
            .iter()
            .copied()
            .filter(|&n| self.slots[n].is_none())
            .collect();
        match self.place(store, &missing) {
            Err(ViewError::Atlas(AtlasError::Full { pages })) => {
                self.restart()?;
                bevy::log::info!(
                    "world view: sprite atlas full ({pages} pages), started over with the {} frames in view",
                    wanted.len()
                );
                self.place(store, &wanted)
            }
            other => other,
        }
    }

    /// Grows the slot table to the store's frames; a store smaller than
    /// the table is another store (a new game's).
    fn track(&mut self, store: &FrameStore) -> Result<(), ViewError> {
        if self.slots.len() > store.len() {
            return Err(ViewError::AtlasAhead {
                atlas: self.slots.len(),
                store: store.len(),
            });
        }
        self.slots.resize(store.len(), None);
        Ok(())
    }

    /// Places frames `ids` (ascending, none placed) as one set.
    fn place(&mut self, store: &FrameStore, ids: &[usize]) -> Result<(), ViewError> {
        if ids.is_empty() {
            return Ok(());
        }
        let frames = store.frames();
        let set: Vec<&IndexFrame> = ids.iter().map(|&n| &frames[n]).collect();
        let slots = self.atlas.insert_frames(&set)?;
        for (&n, slot) in ids.iter().zip(slots) {
            self.slots[n] = Some(slot);
        }
        self.placed += ids.len();
        self.version += 1;
        Ok(())
    }

    /// Empties the pages: no frame is placed.
    fn restart(&mut self) -> Result<(), ViewError> {
        self.atlas = Atlas::new(self.atlas.max_pages())?;
        self.slots.fill(None);
        self.placed = 0;
        self.version += 1;
        self.restarts += 1;
        Ok(())
    }

    pub fn atlas(&self) -> &Atlas {
        &self.atlas
    }

    /// Frames placed since the atlas last started over.
    pub fn frames(&self) -> usize {
        self.placed
    }

    /// The pages' version: it changes whenever a frame is placed or the
    /// atlas starts over.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Times the atlas started over.
    pub fn restarts(&self) -> u64 {
        self.restarts
    }

    /// The pages to hand the render node after frames were packed: the
    /// pages packed into since the last call are copied, every other page
    /// is `last`'s own (shared, so the node skips it). Marks the pages
    /// clean. After a start-over every page is new, so none is shared.
    pub fn share_pages(&mut self, last: &[Arc<AtlasPage>]) -> Vec<Arc<AtlasPage>> {
        let dirty = self.atlas.take_dirty();
        self.atlas
            .pages()
            .iter()
            .enumerate()
            .map(|(i, page)| match last.get(i) {
                Some(held) if !dirty.contains(&(i as u32)) => Arc::clone(held),
                _ => Arc::new(page.clone()),
            })
            .collect()
    }

    /// The slot of each [`FrameId`] (`slots[n]` for `FrameId(n)`; `None`:
    /// not placed).
    pub fn slots(&self) -> &[Option<AtlasSlot>] {
        &self.slots
    }

    /// Packs `frame` for the compute compositor: bins, items, slots,
    /// map rows. Call [`GpuAtlas::ensure_for`] (or [`GpuAtlas::ensure`])
    /// first.
    pub fn pack(
        &self,
        frame: &WorldFrame,
        assets: &ViewAssets,
    ) -> Result<gpu_compositor::Packed, ViewError> {
        let bins = scene::bin(&frame.items, &assets.frames, &assets.maps, play_view())?;
        Ok(gpu_compositor::pack(
            &frame.items,
            &bins,
            &assets.frames,
            self.slots(),
            &assets.maps,
            self.atlas.pages().len() as u32,
        )?)
    }

    /// GPU image of a built frame: equal to [`compose_cpu`] by
    /// design (`d2-client verify` proves it on the synthetic cases).
    pub fn compose(
        &mut self,
        gpu: &Gpu,
        frame: &WorldFrame,
        assets: &ViewAssets,
    ) -> Result<Vec<u8>, ViewError> {
        self.ensure_for(&assets.frames, &frame.items)?;
        let packed = self.pack(frame, assets)?;
        Ok(gpu
            .compose_rgba(&packed, self.atlas.pages(), &assets.palette)?
            .1)
    }

    /// [`GpuAtlas::pack`] as one frame of `cycle`:
    /// the packed frame starts from `cycle.pixels()` with the clears of
    /// `cycle.plan(blank_screen)`, returned beside the packed buffers.
    /// Hand the composed indices to `cycle.commit(plan, indices)`.
    pub fn pack_cycle(
        &self,
        cycle: &FrameCycle,
        blank_screen: bool,
        frame: &WorldFrame,
        assets: &ViewAssets,
    ) -> Result<(gpu_compositor::Packed, scene::FramePlan), ViewError> {
        check_cycle(cycle)?;
        let plan = cycle.plan(blank_screen);
        let packed = self.pack(frame, assets)?.with_frame(cycle.pixels(), plan)?;
        Ok((packed, plan))
    }

    /// GPU image of one frame of `cycle`: [`GpuAtlas::pack_cycle`],
    /// composed, the indices read back and committed to `cycle`. Equal to
    /// [`compose_cycle_cpu`] by design. On error the cycle is unchanged.
    pub fn compose_cycle(
        &mut self,
        gpu: &Gpu,
        cycle: &mut FrameCycle,
        blank_screen: bool,
        frame: &WorldFrame,
        assets: &ViewAssets,
    ) -> Result<Vec<u8>, ViewError> {
        self.ensure_for(&assets.frames, &frame.items)?;
        let (packed, plan) = self.pack_cycle(cycle, blank_screen, frame, assets)?;
        let (indices, rgba) = gpu.compose_rgba(&packed, self.atlas.pages(), &assets.palette)?;
        cycle.commit(plan, indices)?;
        Ok(rgba)
    }
}

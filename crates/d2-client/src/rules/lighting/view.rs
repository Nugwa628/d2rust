//! The world view's `shade` and `blend` hooks answered from the frame's
//! light.
//!
//! [`LitRules`] wraps the view rules of a frame and gives every unit
//! component its cel ops: the draw mode (normal, translucent, ghostly,
//! highlighted…, see [`crate::rules::blend`]) and the light byte read from
//! the frame's [`LightMap`] at the unit's sub-tile. Every other hook goes
//! to the wrapped rules.
//!
//! The per-unit inputs the client model does not hold yet (the unit's
//! sub-tile, the ghostly flag, the unit override inputs, the hover target,
//! the component's remap) come from a [`LookFeed`]; a feed that cannot
//! answer returns an error, never a default.

use crate::bridge::world::ClientWorld;
use crate::bridge::ClientUnit;
use crate::composite::{ComponentFrame, ComponentRequest, CompositeError, UnitParams};
use crate::frames::IndexFrame;
use crate::scene::{BlendOp, MapId, ShadeChain};
use crate::ui::{ImageRequest, TextRequest};
use crate::world_view::{TileDraw, UiRules, UiSprite, UnitPose, ViewAssets, ViewError, ViewRules};

use super::super::blend::{
    cel_ops, component_mode, unit_override, OverrideInput, UnitKind, MODE_HIGHLIGHT,
};
use super::super::shading::{hover_light, ShadeTables};
use super::map::LightMap;

/// The inputs of one component draw's mode and colour map that the client
/// model does not state yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentLook {
    /// A ghostly monster (monster-data flag 0x40): drawn translucent.
    pub ghostly: bool,
    /// The inputs of the unit's own draw-mode override (a `fade` stat or
    /// a translucent item); `None` = the feed states none and
    /// [`LitRules`] builds them from the unit's `fade` stat and the
    /// component's item record.
    pub override_input: Option<OverrideInput>,
    /// The unit is the hover target and can be highlighted.
    pub hovered: bool,
    /// The component's remap colour map `P` (an item's or monster's hue),
    /// `None` = no remap.
    pub remap: Option<MapId>,
    /// The name of a remap that is not one of the act's hue maps (a
    /// monster's palette shift); `None`: named from `remap`.
    pub pal: Option<crate::scene::CelPal>,
}

/// The per-unit inputs of [`LitRules`], each from the client model; an
/// implementation whose input the model does not hold returns an error.
pub trait LookFeed {
    /// The unit's sub-tile (its position); its light is the light-map cell
    /// of that sub-tile.
    fn light_subtile(&self, unit: &ClientUnit) -> Result<(i32, i32), String>;

    /// The component's look inputs: the remap colour map, the ghostly
    /// flag, the unit override and the hover state of the draw mode.
    fn look(&self, unit: &ClientUnit, req: &ComponentRequest<'_>) -> Result<ComponentLook, String>;

    /// Whether the unit is drawn highlighted, for the light byte its unit
    /// draw hands its overlays.
    fn hovered(&self, _unit: &ClientUnit) -> bool {
        false
    }
}

/// The light byte a unit's draw hands its overlays (speech balloons, auras
/// and the like are lit exactly like their host): the intensity of the
/// unit's light-map cell, doubled and clamped when the unit is drawn
/// highlighted ([`hover_light`]). A unit whose sub-tile the feed cannot
/// state draws its overlays at full light (0xFF).
pub fn unit_light_byte(light: &FrameLight, feed: &dyn LookFeed, unit: &ClientUnit) -> u8 {
    let Ok(at) = feed.light_subtile(unit) else {
        return 0xFF;
    };
    let v = light.map.read(at.0 * 8, at.1 * 8).i;
    if feed.hovered(unit) {
        hover_light(v)
    } else {
        v
    }
}

/// The lighting state of one drawn frame: the act's shading tables in the
/// view's map table and the frame's light map (rebuilt every drawn frame).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameLight {
    pub tables: ShadeTables,
    pub map: LightMap,
}

/// The draw mode of a component ([`component_mode`]): a COF layer whose
/// override byte is set brings its own translucency mode.
pub fn layer_mode(look: &ComponentLook, req: &ComponentRequest<'_>) -> u8 {
    let layer = (req.layer.override_translucency != 0).then_some(req.layer.new_translucency);
    let r = look.override_input.as_ref().and_then(unit_override);
    component_mode(look.ghostly, r, look.hovered, layer)
}

/// The cel ops of a component: its draw mode and the light byte `v`, the
/// intensity of the unit's light-map cell, doubled and clamped in the
/// highlight mode so a hovered unit stands out.
pub fn component_ops(
    light: &FrameLight,
    subtile: (i32, i32),
    look: &ComponentLook,
    req: &ComponentRequest<'_>,
) -> (ShadeChain, BlendOp) {
    let mode = layer_mode(look, req);
    let mut v = light.map.read(subtile.0 * 8, subtile.1 * 8).i;
    if mode == MODE_HIGHLIGHT {
        v = hover_light(v);
    }
    cel_ops(&light.tables, mode, look.remap, v)
}

/// View rules `R` with `shade` and `blend` answered from the frame's light
/// and a [`LookFeed`].
#[derive(Debug, Clone, Copy)]
pub struct LitRules<'a, R: ?Sized, F: ?Sized> {
    pub rules: &'a R,
    pub feed: &'a F,
    pub light: &'a FrameLight,
}

fn look_error(req: &ComponentRequest<'_>, what: &'static str, message: String) -> CompositeError {
    CompositeError::Unresolved {
        slot: req.slot.slot,
        component: req.slot.component,
        what,
        message,
    }
}

impl<R: ViewRules + ?Sized, F: LookFeed + ?Sized> LitRules<'_, R, F> {
    /// The feed's look, with the unit override inputs built from the
    /// unit's `fade` stat and the component's item record
    /// ([`ViewRules::component_item_trans`]) when the feed states none.
    fn look(&self, unit: &ClientUnit, req: &ComponentRequest<'_>) -> Result<ComponentLook, String> {
        let mut look = self.feed.look(unit, req)?;
        if look.override_input.is_none() {
            look.override_input = Some(override_of(
                unit,
                req,
                self.rules.component_item_trans(unit, req),
            ));
        }
        Ok(look)
    }

    fn ops(
        &self,
        unit: &ClientUnit,
        req: &ComponentRequest<'_>,
    ) -> Result<(ShadeChain, BlendOp), CompositeError> {
        let at = self
            .feed
            .light_subtile(unit)
            .map_err(|m| look_error(req, "unit light", m))?;
        let look = self
            .look(unit, req)
            .map_err(|m| look_error(req, "component look", m))?;
        Ok(component_ops(self.light, at, &look, req))
    }
}

/// The translucency override inputs the client model states: the unit
/// kind, its stat 181 `fade`, the component and its item record's
/// `transparent` / `transtbl`. A monster's inventory
/// ethereal test and the player's ethereal hand items are not stated
/// (`D2RUST's own, unverified`: taken as none).
pub fn override_of(
    unit: &ClientUnit,
    req: &ComponentRequest<'_>,
    item_trans: Option<(u8, u8)>,
) -> OverrideInput {
    use crate::bridge::world::{MONSTER, PLAYER};
    let kind = match unit.key.unit_type {
        PLAYER => UnitKind::Player,
        MONSTER => UnitKind::Monster,
        _ => UnitKind::Other,
    };
    OverrideInput {
        kind,
        fade: unit.stats.get(&STAT_FADE).copied().unwrap_or(0),
        monster_has_ethereal: false,
        item_trans,
        component: req.slot.component,
        item_ethereal: false,
    }
}

/// Stat 181 `fade`: a unit fading out draws translucent.
const STAT_FADE: u16 = 181;

impl<R: ViewRules + ?Sized, F: LookFeed + ?Sized> ViewRules for LitRules<'_, R, F> {
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
        let at = self.feed.light_subtile(unit).ok()?;
        Some(self.light.map.read(at.0 * 8, at.1 * 8).word())
    }

    fn unit_client_point(&self, unit: &ClientUnit) -> Option<(i32, i32)> {
        self.rules.unit_client_point(unit)
    }

    fn component_item_trans(
        &self,
        unit: &ClientUnit,
        req: &ComponentRequest<'_>,
    ) -> Option<(u8, u8)> {
        self.rules.component_item_trans(unit, req)
    }

    /// The draw mode and the remap, named by its table: one of the act's
    /// 128 hue maps starting at `remap0`, a named palette shift, or none.
    fn component_call(
        &self,
        unit: &ClientUnit,
        req: &ComponentRequest<'_>,
    ) -> Option<crate::scene::CelArgs> {
        use crate::scene::{CelArgs, CelPal};
        let look = self.look(unit, req).ok()?;
        let base = self.light.tables.remap0.0;
        let pal = match (look.pal, look.remap) {
            (Some(p), _) => p,
            (None, None) => CelPal::Null,
            (None, Some(m)) if m.0 >= base && m.0 - base < 128 => CelPal::Hue((m.0 - base) as u8),
            (None, Some(_)) => CelPal::Other,
        };
        Some(CelArgs {
            mode: layer_mode(&look, req),
            pal,
        })
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
        Ok(self.ops(unit, req)?.0)
    }

    fn blend(
        &self,
        unit: &ClientUnit,
        req: &ComponentRequest<'_>,
    ) -> Result<BlendOp, CompositeError> {
        Ok(self.ops(unit, req)?.1)
    }
}

impl<R: UiRules + ?Sized, F: ?Sized> UiRules for LitRules<'_, R, F> {
    fn ui_image(&self, req: &ImageRequest, assets: &ViewAssets) -> Result<UiSprite, ViewError> {
        self.rules.ui_image(req, assets)
    }

    fn ui_text(&self, req: &TextRequest, assets: &ViewAssets) -> Result<Vec<UiSprite>, ViewError> {
        self.rules.ui_text(req, assets)
    }

    fn ui_pass(&self) -> Result<u32, ViewError> {
        self.rules.ui_pass()
    }
}

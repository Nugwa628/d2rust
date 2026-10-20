//! Gives the bridge the visibility predicate of the position check from the
//! world view ([`crate::world_view::visibility`]): the unit art the play
//! UI's loader fills ([`super::ui::UnitArt`]) and the last drawn frame's camera
//! ([`WorldViewState::camera`]); and, from the same art, the layer cel offsets
//! of the game's layer offsets.
//!
//! Without the unit art (synthetic data: no unit is drawn) the predicate still
//! runs over an empty art store, so every unit reads as not visible and the
//! check corrects instead of refusing the message: no COF is resident for rule
//! 2 (PROVISIONAL, with the module doc of `world_view::visibility`).

use std::sync::Arc;

use crate::bridge::world::CelOffsetFn;
use crate::bridge::BridgeResource;
use crate::world_view::unit_assets::{cel_offset_fn, UnitLooks};
use crate::world_view::visibility::ViewVisibility;
use crate::world_view::WorldViewState;

/// Installs the predicate on the bridge. Call it after the bridge, the
/// world view and (when there is one) the play UI's unit art exist.
pub fn add_visibility(app: &mut impl crate::app::session::Setup) {
    let world = app.world();
    let Some(camera) = world
        .get_resource::<WorldViewState>()
        .map(|s| s.camera.clone())
    else {
        return;
    };
    let (looks, art) = world.get_resource::<super::ui::UnitArt>().map_or_else(
        || (Arc::new(UnitLooks::default()), Default::default()),
        |a| (a.0.looks.clone(), a.0.art.clone()),
    );
    // The layer cel offsets, read
    // from the same unit art.
    let cel_offset = CelOffsetFn::new(cel_offset_fn(looks.clone(), art.clone()));
    let visible = ViewVisibility { looks, art, camera }.into_fn();
    if let Some(mut bridge) = app.world_mut().get_resource_mut::<BridgeResource>() {
        bridge.0.set_visibility(Some(visible));
        bridge.0.set_cel_offset(Some(cel_offset));
    }
}

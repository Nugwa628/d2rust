//! The play preview's monster motion in the app: [`MonsterWalk`] holds
//! the [`MonsterMotion`] tracks and [`monster_walk_frame`] steps them on
//! the model after each bridge frame, before the unit mirror and the
//! draw (D2RUST's own, unverified; see [`crate::bridge::motion`]). Only
//! `play` adds it.

use bevy::prelude::*;

use crate::bridge::motion::MonsterMotion;
use crate::bridge::BridgeResource;

/// The monster tracks of the play preview.
#[derive(Resource, Debug, Default)]
pub struct MonsterWalk(pub MonsterMotion);

/// Steps the monster tracks after the bridge frame.
pub fn monster_walk_frame(mut bridge: ResMut<BridgeResource>, mut walk: ResMut<MonsterWalk>) {
    bridge.0.preview_motion(&mut walk.0);
}

/// The systems of the wiring below, once per app (`app::session`).
pub fn register_monster_walk(app: &mut App) {
    super::present::configure_preview_order(app);
    app.add_systems(
        PreUpdate,
        monster_walk_frame
            .in_set(super::present::PreviewOrder::MonsterWalk)
            .run_if(resource_exists::<BridgeResource>)
            .run_if(resource_exists::<MonsterWalk>),
    );
}

/// Adds [`MonsterWalk`] and [`monster_walk_frame`].
pub fn add_monster_walk(app: &mut impl crate::app::session::Setup) {
    app.register("monster-walk", register_monster_walk);
    app.world_mut().init_resource::<MonsterWalk>();
}

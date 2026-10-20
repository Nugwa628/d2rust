//! Grouped properties integration tests. Register new tests here by path.

#[path = "../e2e_support/mod.rs"]
mod e2e_support;
#[path = "../prop_support/mod.rs"]
mod prop_support;

#[path = "../prop_atlas.rs"]
mod prop_atlas;
#[path = "../prop_bridge.rs"]
mod prop_bridge;
#[path = "../prop_cache.rs"]
mod prop_cache;
#[path = "../prop_client_bridge.rs"]
mod prop_client_bridge;
#[path = "../prop_controls.rs"]
mod prop_controls;
#[path = "../prop_scene.rs"]
mod prop_scene;
#[path = "../prop_ui.rs"]
mod prop_ui;
#[path = "../prop_world_view.rs"]
mod prop_world_view;
#[path = "../prop_worldsim.rs"]
mod prop_worldsim;

//! Grouped world integration tests. Register new tests here by path.

#[path = "../app_support/mod.rs"]
mod app_support;
#[path = "../e2e_support/mod.rs"]
mod e2e_support;
#[path = "../real_rig/mod.rs"]
mod real_rig;

#[path = "../audio_unit_sounds.rs"]
mod audio_unit_sounds;
#[path = "../bug_scenarios.rs"]
mod bug_scenarios;
#[path = "../e2e_full_loop.rs"]
mod e2e_full_loop;
#[path = "../e2e_imbue.rs"]
mod e2e_imbue;
#[path = "../e2e_pet_action.rs"]
mod e2e_pet_action;
#[path = "../e2e_single_player.rs"]
mod e2e_single_player;
#[path = "../e2e_vendor.rs"]
mod e2e_vendor;
#[path = "../e2e_walk.rs"]
mod e2e_walk;
#[path = "../play_act5.rs"]
mod play_act5;
#[path = "../play_identify.rs"]
mod play_identify;
#[path = "../play_native.rs"]
mod play_native;
#[path = "../play_smoke.rs"]
mod play_smoke;
#[path = "../prov_data_tables.rs"]
mod prov_data_tables;
#[path = "../rate_baselines.rs"]
mod rate_baselines;
#[path = "../save_roundtrip.rs"]
mod save_roundtrip;
#[path = "../scenario_smoke.rs"]
mod scenario_smoke;
#[path = "../seam_belt_tables.rs"]
mod seam_belt_tables;
#[path = "../seam_bridge_app.rs"]
mod seam_bridge_app;
#[path = "../seam_drlg_coords.rs"]
mod seam_drlg_coords;
#[path = "../seam_item_grids.rs"]
mod seam_item_grids;
#[path = "../seam_messages.rs"]
mod seam_messages;
#[path = "../seam_world_screen.rs"]
mod seam_world_screen;
#[path = "../smoke_save.rs"]
mod smoke_save;
#[path = "../soak_walk.rs"]
mod soak_walk;

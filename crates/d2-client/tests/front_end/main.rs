//! Grouped front_end integration tests. Register new tests here by path.

#[path = "../app_support/mod.rs"]
mod app_support;

#[path = "../front_end.rs"]
mod front_end;
#[path = "../front_end_char_select.rs"]
mod front_end_char_select;
#[path = "../front_end_controls.rs"]
mod front_end_controls;
#[path = "../front_end_create.rs"]
mod front_end_create;
#[path = "../front_end_credits.rs"]
mod front_end_credits;
#[path = "../front_end_difficulty.rs"]
mod front_end_difficulty;
#[path = "../front_end_draw.rs"]
mod front_end_draw;
#[path = "../front_end_main_menu.rs"]
mod front_end_main_menu;
#[path = "../front_end_online.rs"]
mod front_end_online;
#[path = "../front_end_unreachable.rs"]
mod front_end_unreachable;
#[path = "../front_host.rs"]
mod front_host;
#[path = "../front_host_real.rs"]
mod front_host_real;
#[path = "../front_host_screens.rs"]
mod front_host_screens;
#[path = "../front_start.rs"]
mod front_start;
#[path = "../online_smoke.rs"]
mod online_smoke;
#[path = "../smoke_frontend.rs"]
mod smoke_frontend;

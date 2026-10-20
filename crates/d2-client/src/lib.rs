//! Bevy client. Renders state and sends intents; never decides outcomes.
//! All contact with the game goes through [`bridge`].
//!
//! [`map`] is plain Rust (no Bevy types): asset assembly and the CPU
//! reference renderer used to check the GPU output. [`scene`] is plain
//! Rust too: the draw list and the CPU reference compositor, as is
//! [`composite`], which turns COF frames into scene draw items.
//! [`gpu_compositor`] is the GPU twin of the CPU compositor; [`verify`]
//! runs the render cases (CPU reference vs GPU). [`world_view`] turns the
//! bridge's client world into the frame's draw list and composes it.
//! [`facts`] writes and compares rendering fact sets (`facts/render/`).
//! [`mods`] holds a mod game's client hooks ([`mods::ClientMods`]).

pub mod app;
pub mod assets;
pub mod audio;
pub mod bridge;
pub mod composite;
pub mod controls;
pub mod facts;
pub mod frames;
pub mod gpu_compositor;
pub mod launch;
pub mod map;
pub mod mods;
pub mod mute;
pub mod recording;
pub mod render;
pub mod rules;
pub mod scenario_script;
pub mod scene;
pub mod ui;
pub mod user_dir;
pub mod verify;
pub mod world_view;

//! The world's light.
//!
//! Every drawn frame the client builds a 48 × 48 light map around the local
//! player: each cell starts from its room's ambient light, cells that block
//! light are flagged, and then every light record (the player's own light,
//! torches, glowing monsters, missiles, shrines…) adds its intensity and
//! colour to the cells it reaches, shadowed by the flagged cells when the
//! light quality allows it. The draws then read the map: a unit takes the
//! cell under its feet, walls take six points along their face, floors and
//! roofs take an 8 × 8 grid.
//!
//! Around the map sit the slower parts of lighting: the act's day / night
//! environment, the scripted ambient overrides (the Den of Evil glow, the
//! red glow of the last Act IV levels, the darkness events) and the light
//! quality that the measured draw rate picks.
//!
//! Everything here is plain Rust client state with no Bevy types. The
//! environment and the overrides use doubles and `sin`, which is fine
//! because lighting is client-only and never feeds `d2-sim`.
//!
//! - [`map`]: the light map, its ambient fill, its blocks-light flags, the
//!   build order of one frame and the map's digest.
//! - [`records`]: light records, the light list and its per-frame update.
//! - [`contribute`]: how one record adds its light to the map (plain,
//!   shadowed or from a cached shadow grid).
//! - [`quality`]: the light quality and the draw-rate meter that drives it.
//! - [`environment`]: the act environment (day and night) and a room's
//!   ambient.
//! - [`overrides`]: the scripted ambient overrides and the unique-event
//!   messages that start them.
//! - [`sources`]: what creates and changes light records.
//! - [`draws`]: the light values handed to unit, wall, floor and roof
//!   draws.
//! - [`view`]: [`view::LitRules`], the world view's `shade` / `blend` hooks
//!   answered from the frame's light map.

pub mod contribute;
pub mod draws;
pub mod environment;
pub mod map;
pub mod overrides;
pub mod quality;
pub mod records;
pub mod sources;

#[cfg(test)]
mod draws_tests;
pub mod view;

#[cfg(test)]
pub(crate) mod view_tests;

pub use map::{LightCell, LightMap};
#[cfg(test)]
mod core_tests;

#[cfg(test)]
mod env_tests;

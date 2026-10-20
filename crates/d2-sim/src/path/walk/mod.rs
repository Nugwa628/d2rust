//! Walk and run: the mode request (C→S 0x01–0x04), the position resync
//! (C→S 0x5F), every path type, missile paths,
//! path compute, target preparation, toward / straight / A*, velocity,
//! direction and facing, per-tick movement, and the S→C byte builders.
//!
//! Seams: [`seams::PathWorld`] (path record store, footprints by unit,
//! room lists, on the core's [`crate::path::CollisionRooms`]) and
//! [`seams::WalkUnits`] (units, modes, timers, stats, skills, messages),
//! both on one context object (the provider holds the game, units, DRLG
//! grids and path store together). Records, tables, collision queries
//! and footprint moves are the path core's
//! ([`super::footprint`]).
//! Integer and 16.16 fixed-point arithmetic only: the
//! x87 target lead adds 0, and the blessed
//! hammer's float32 products are computed exactly from the sine table's
//! bit patterns ([`sine`]).

pub mod find;
pub mod geom;
pub mod messages;
pub mod missile;
pub mod other;
pub mod request;
pub mod resync;
pub mod seams;
pub mod sine;
pub mod step;
pub mod velocity;

#[cfg(test)]
mod mutant_tests;
#[cfg(test)]
mod tests;

pub use request::{handle_message, request, Outcome, WalkTarget};
pub use seams::{PathInfo, PathWorld, Point, TargetUnit, WalkError, WalkUnits};
pub use step::{Step, Walk};

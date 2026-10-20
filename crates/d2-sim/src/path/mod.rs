//! Unit position, collision footprints and placement
//! and walk / run movement.
//!
//! | Module | Covers |
//! |---|---|
//! | [`coords`] | coordinates |
//! | [`record`] | path records, size / pattern / footprint mask |
//! | [`collision`] | collision queries ([`collision::CollisionRooms`] seam) |
//! | [`footprint`] | footprints, moving a footprint ([`footprint::PathMotion`] seam) |
//! | [`tables`] | `sim/path-tables.tsv` |
//! | [`search`] | nearest free point, coarse free-box search |
//! | [`place`] | floor drop, placing a unit, level spawn ([`place_seams`]: its seams) |
//! | [`warp`] | warp tiles and arrival |
//! | [`history`] | player position history |
//! | [`line`] | cell line test |
//! | [`walk`] |: walk / run requests, path compute, per-tick movement, missile paths, the 0x5F resync |
//!
//! No placement function draws; in
//! [`walk`], the concentration roll, the charged-bolt
//! path and the 0x5F lock draw on the unit seed.

pub mod collision;
pub mod coords;
pub mod footprint;
pub mod history;
pub mod line;
pub mod place;
pub mod place_seams;
pub mod record;
pub mod search;
pub mod tables;
pub mod walk;
pub mod warp;

#[cfg(test)]
mod gap_tests;
#[cfg(test)]
mod mutant_tests;
#[cfg(test)]
mod tests;

pub use collision::CollisionRooms;
pub use coords::Point;
pub use footprint::{FootShape, Footprint, PathMotion, RemoveRule};
pub use record::{
    DynamicKind, DynamicPath, MonsterShape, ObjectShape, PathKind, PathPoint, StaticPath,
    TargetUnit, UnitPath, UnitShape,
};
pub use tables::PathTables;

/// Errors of the path code: bad embedded tables, and fatal
/// fatal asserts.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("path-tables.tsv: {0}")]
    Tsv(String),
    /// Fatal assert.
    #[error("path type {0}: fatal assert")]
    PathType(u32),
    /// Fatal assert: a non-zero point without a room.
    #[error("teleport to ({x}, {y}) without a room")]
    TeleportNoRoom { x: i32, y: i32 },
    /// Fatal assert: a re-path budget above 255.
    #[error("re-path budget {0} > 255: fatal assert")]
    RepathBudget(u32),
}

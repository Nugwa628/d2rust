//! The path seams wired to `d2_sim::path`: unit path records kept with
//! the units, collision rooms on the DRLG, walk / run (`path::walk`) on
//! the unit system, and placement / warps (`path::place`, `path::warp`)
//! on the DRLG levels.
//!
//! - [`rooms`]: [`crate::path::CollisionRooms`] on
//!   [`crate::wiring::action::DrlgWorld`].
//! - [`units`]: the unit path record in [`PathState`]; the path part of
//!   unit allocation, size and shape, footprints, the path setters the
//!   missile and AI adapters call, and the path free at removal.
//! - [`walk`]: [`PathCtx`], the one context `path::walk` runs on
//!   (`PathWorld` + `WalkUnits`) and `path::footprint::PathMotion`;
//!   the walk request, the per-tick player step (event 0) and the unit
//!   step.
//! - [`missiles`]: the missile bodies' path seams (target point and
//!   position, set type, step counts, teleport).
//! - [`monsters`]: monster walk / run: the path part of the monster mode
//!   set, a monster mode's velocity, the walk event 0 and the mode end.
//! - [`place`]: `CollisionView`, `PlaceHost` and `LevelView` on a shared
//!   [`PathCtx`]; placing a unit, the level warp and the floor drop.
//! - `act_change`: the act change of a level warp into another act.
//! - `warp_dest`: the destination of a warp tile.
//!
//! The provider is opt-in per game: [`crate::wiring::action::ActionHooks::paths`]
//! is `None` until a host calls `enable_paths`; then every path seam of
//! the action and worldgen adapters is answered here, and
//! [`crate::wiring::action::Pending`]'s path methods are no longer
//! called. Without it the adapters keep their `Pending` answers (hosts
//! and tests that fake positions). Nothing here decides game behaviour:
//! each adapter maps one seam call to one call of the path code.

pub mod act_change;
pub mod missiles;
pub mod monsters;
pub mod place;
pub mod rooms;
pub mod units;
pub mod walk;
mod warp_dest;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod motion_tests;

#[cfg(test)]
mod init_cb_tests;

#[cfg(test)]
mod frames_tests;

#[cfg(test)]
mod mutant_tests;

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::path::history::PositionHistory;
use crate::path::search::ExpField;
use crate::path::{PathTables, UnitPath};
use crate::units::UnitId;

pub use walk::PathCtx;

/// The path records of a game's units (freed at removal) and the constant
/// path tables.
#[derive(Clone, Debug)]
pub struct PathState {
    pub tables: Arc<PathTables>,
    pub records: BTreeMap<UnitId, UnitPath>,
    /// `data\global\ExpField.D2` (d2data.mpq), loaded by the host: the
    /// floor drop's walk-back field. `None`: the floor-drop seams keep
    /// their pending answers.
    pub field: Option<Arc<ExpField>>,
    /// The players' position history, written by the placement and the
    /// walk step; read by monster AI.
    pub history: BTreeMap<UnitId, PositionHistory>,
    /// The AI's mode request staged for the monster mode set that follows
    /// (the request record's target), taken by
    /// [`ActionHooks::monster_path_setup`](crate::wiring::action::ActionHooks).
    pub mode_request: Option<(UnitId, crate::monsters::ai::ModeTarget)>,
    /// The monster's velocity request staged by an AI mode request for the
    /// movement set-up that consumes it (the AI store is lent out during
    /// the think).
    pub mode_velocity: Option<(UnitId, crate::monsters::ai::VelocityRequest)>,
    /// The movement set-up's fields of the AI param record, per monster.
    pub setup: BTreeMap<UnitId, monsters::MoveSetup>,
    /// The staged request's path-type byte when the AI overwrote the
    /// builder's value.
    pub mode_request_byte: Option<(UnitId, u8)>,
}

impl PathState {
    /// No records, the embedded tables.
    pub fn new() -> Result<Self, crate::path::PathError> {
        Ok(Self {
            tables: Arc::new(PathTables::spec()?),
            records: BTreeMap::new(),
            field: None,
            history: BTreeMap::new(),
            mode_request: None,
            mode_velocity: None,
            setup: BTreeMap::new(),
            mode_request_byte: None,
        })
    }

    /// The unit's path record (`None`: the unit has none).
    pub fn record(&self, unit: UnitId) -> Option<&UnitPath> {
        self.records.get(&unit)
    }

    /// The unit's dynamic path, if it has one.
    pub fn dynamic(&self, unit: UnitId) -> Option<&crate::path::DynamicPath> {
        match self.records.get(&unit)? {
            UnitPath::Dynamic(p) => Some(p),
            UnitPath::Static(_) => None,
        }
    }

    /// The unit's dynamic path, mutable.
    pub fn dynamic_mut(&mut self, unit: UnitId) -> Option<&mut crate::path::DynamicPath> {
        match self.records.get_mut(&unit)? {
            UnitPath::Dynamic(p) => Some(p),
            UnitPath::Static(_) => None,
        }
    }
}

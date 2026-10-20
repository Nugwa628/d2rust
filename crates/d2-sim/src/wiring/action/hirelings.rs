//! The hireling calls the action wiring meets but cannot run: the
//! hireling lists live with the host that holds the interaction state
//! (`d2-server`'s `WiredWorld`), so the action wiring queues each call
//! ([`super::ActionHooks::hireling_calls`]) in the order it met it, and
//! the host runs the rules (`crate::world::hirelings::life`) on its
//! lists when the handler or tick that queued them returns.

use crate::units::UnitId;
use crate::world::hirelings::life::{Loader, SavedHireling};

/// One queued hireling call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HirelingCall {
    /// Restore `player`'s saved hireling from the save load (game join).
    Restore {
        player: UnitId,
        saved: SavedHireling,
        loader: Loader,
    },
    /// The join follow: the hireling follows `player` after the join
    /// placement.
    JoinFollow(UnitId),
    /// The act change of `player` (from the level warp to another act):
    /// the classic-game hireling handling, then the pet follow.
    ActChange(UnitId),
    /// The level-up tail of the kill's hireling add: the level stats at
    /// `level`, the stat messages, event 0x5B and unit event 12.
    /// Run by the host after the tick step, not inside the kill, so the
    /// level stats and the 0x2C event 0x5B go out in the same frame as the
    /// kill's other messages.
    LevelUp {
        player: UnitId,
        merc: UnitId,
        level: i32,
    },
}

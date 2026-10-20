//! A mod game's missile overrides (`docs/plans/mods/seams.md` MS6): its
//! server-do and server-hit functions.
//!
//! A `missiles.txt` row names its server-do (`SrvDoFunc`) and server-hit
//! (`SrvHitFunc`) functions by index. The mod answers first and D2RUST's
//! own body runs when it doesn't:
//! - **Slots.** [`MissileProvider::has`] adds indices past D2RUST's
//!   tables (server-do 1-52, server-hit 1-70) and fills its empty ones;
//!   the class handler and the hit handler call a slot only when one of
//!   the two has it.
//! - **Bodies.** Every server-do and server-hit call goes through
//!   [`crate::missiles::catalogue::run_srv_do`] /
//!   [`crate::missiles::catalogue::run_srv_hit`], including the calls one
//!   D2RUST body makes to another, which ask [`MissileProvider::srv_do`] /
//!   [`MissileProvider::srv_hit`] first. A mod body returning `Some`
//!   replaces D2RUST's for that index wherever it is reached.
//!
//! The bodies get the same world D2RUST's do, as a `dyn` world, so they
//! can call D2RUST's missile helpers and bodies. They must stay
//! deterministic: no I/O, no clock, no floats in game state, draws only
//! from the game's seeds.

use std::fmt::Debug;

use crate::game::Game;
use crate::missiles::{Ctx, MissileWorld};
use crate::units::UnitId;

/// Which function table a missile slot is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MissileSlot {
    /// Server-do: runs on the missile's class event (its flight).
    Do,
    /// Server-hit: runs when the missile hits a unit or the floor.
    Hit,
}

/// A mod game's missile functions (MS6). Every method's default leaves
/// D2RUST's behaviour.
pub trait MissileProvider: Send + Sync + Debug {
    /// Whether the mod has a body for `index` of `slot`. The handlers
    /// call a slot past D2RUST's table, or an empty one of it, only when
    /// this says so.
    fn has(&self, slot: MissileSlot, index: i16) -> bool {
        let _ = (slot, index);
        false
    }

    /// Server-do `index` for missile `m`: `Some(result)` replaces
    /// D2RUST's body (2 removes the missile); `None` runs D2RUST's.
    fn srv_do(
        &self,
        game: &mut Game,
        cx: &mut Ctx<'_, dyn MissileWorld + '_>,
        index: i16,
        m: UnitId,
    ) -> Option<i32> {
        let _ = (game, cx, index, m);
        None
    }

    /// Server-hit `index` for missile `m` hitting `unit` (`None` = the
    /// floor), with the hit handler's current result `c`: `Some(result)`
    /// replaces D2RUST's body; `None` runs D2RUST's.
    fn srv_hit(
        &self,
        game: &mut Game,
        cx: &mut Ctx<'_, dyn MissileWorld + '_>,
        index: i16,
        m: UnitId,
        unit: Option<UnitId>,
        c: i32,
    ) -> Option<i32> {
        let _ = (game, cx, index, m, unit, c);
        None
    }
}

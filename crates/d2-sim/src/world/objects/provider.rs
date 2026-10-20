//! A mod game's object operate functions (`docs/plans/mods/seams.md` MS8b).
//!
//! An object's `objects.txt` row names its operate function by number. The
//! operate dispatch ([`super::dispatch`]) asks the mod first:
//! - [`ObjectProvider::has_operate`] claims a function number. D2RUST's
//!   table ends at [`super::OPERATE_FN_BOUND`]; a claimed number past it is
//!   the mod's own slot instead of a fatal error, so the mod sets its own
//!   slot count. A mod may also claim one of D2RUST's numbers (a chest,
//!   a quest object, a waypoint ...) to take it over.
//! - [`ObjectProvider::operate`] runs a claimed function. `None` falls
//!   through to D2RUST's route for the number (past the bound: nothing
//!   runs, the dispatch returns 0).
//!
//! The dispatch's own checks stay D2RUST's and run first: the player
//! refusals (interacting, busy, an item on the cursor away from the stash)
//! and the refused classes. Unclaimed numbers run exactly as under
//! `Original`.
//!
//! Providers get the object world as `dyn ObjectHost` and must stay
//! deterministic: no I/O, no clock, no floats in game state, draws only
//! from the game's seeds (the control seed in [`ObjectControl`], the unit
//! seeds through [`super::ObjectWorld::unit_seed`]).

use std::fmt::Debug;

use super::{ObjectControl, ObjectError, ObjectHost, ObjectTables, Operate};

/// A mod game's object operate functions. Every method defaults to "not
/// the mod's", so a provider only writes what its game adds.
pub trait ObjectProvider: Send + Sync + Debug {
    /// The mod runs operate function `n` (any number, D2RUST's or past
    /// [`super::OPERATE_FN_BOUND`]).
    fn has_operate(&self, n: u8) -> bool {
        let _ = n;
        false
    }

    /// Run claimed operate function `op.operate_fn` on `op.object`. `Some`:
    /// the function's return value (or a fatal error); `None`: D2RUST's
    /// route for the number runs instead.
    fn operate(
        &self,
        ctl: &mut ObjectControl,
        t: &ObjectTables,
        w: &mut dyn ObjectHost,
        op: &Operate,
    ) -> Option<Result<i32, ObjectError>> {
        let _ = (ctl, t, w, op);
        None
    }
}

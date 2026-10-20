//! A mod game's Horadric Cube overrides (`docs/plans/mods/seams.md` MS8a):
//! its recipe op codes and its output kinds.
//!
//! The mod answers first and D2RUST's own code runs when it doesn't:
//! - **Ops.** A recipe's `op` picks a test. [`CubeProvider::op_scope`] says
//!   where the mod tests an op code (the recipe on the player, the slot-0
//!   candidate, or every candidate); [`CubeProvider::op_test`] is that test.
//!   Codes it gives no scope run D2RUST's tests ([`super::op_info`]; codes
//!   D2RUST doesn't know always pass). A mod may take over one of D2RUST's
//!   codes too: then D2RUST's test for that code doesn't run.
//! - **Outputs.** [`CubeProvider::output`] makes an output slot before
//!   D2RUST looks at its kind: a portal-like event, nothing, or an item.
//!   A mod item goes through D2RUST's finishing (removals, craft mods,
//!   quantity, repair, recharge, sockets) and is placed on commit, like
//!   D2RUST's own outputs. `None` keeps D2RUST's output for the slot.
//!
//! Op codes and output kinds are the record's bytes, so there is no slot
//! count to extend: a mod game uses any code or kind its tables write.
//! Providers get the cube's world as `dyn CubeWorld` and must stay
//! deterministic: no I/O, no clock, no floats in game state, draws only
//! from the game's seeds ([`CubeWorld::game_seed`], [`CubeWorld::item_seed`]).

use std::fmt::Debug;

use super::{Capture, CubeWorld, OpScope, Recipe};
use crate::units::UnitId;

/// What a mod game's output slot made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CubeOutput {
    /// No item; the transmute succeeds when this is `true` (later slots may
    /// still change it), as D2RUST's portal kinds do.
    Event(bool),
    /// Nothing for this slot; the transmute's success is unchanged.
    Nothing,
    /// An item the mod made: D2RUST finishes it and places it on commit.
    Item(UnitId),
}

/// A mod game's cube ops and outputs (MS8a). Every method's default
/// leaves D2RUST's behaviour.
pub trait CubeProvider: Send + Sync + Debug {
    /// Where the mod tests op code `op`; `None`: D2RUST's test for the code.
    fn op_scope(&self, op: u8) -> Option<OpScope> {
        let _ = op;
        None
    }
    /// The test of the recipe's op (one [`CubeProvider::op_scope`] scoped)
    /// on `unit`: the player at [`OpScope::Recipe`], the slot-0 candidate at
    /// [`OpScope::Input0`], each candidate of every slot at
    /// [`OpScope::Inputs`]. `true` passes.
    fn op_test(&self, w: &dyn CubeWorld, recipe: &Recipe, unit: UnitId) -> bool {
        let _ = (w, recipe, unit);
        true
    }
    /// Makes output slot `slot` (0-2) of `recipe` for `player`. `capture` is
    /// what the input match captured for the slot and `level` the output
    /// level D2RUST computed. `None`: D2RUST's output for the slot's kind.
    fn output(
        &self,
        w: &mut dyn CubeWorld,
        player: UnitId,
        recipe: &Recipe,
        slot: usize,
        capture: &Capture,
        level: i32,
    ) -> Option<CubeOutput> {
        let _ = (w, player, recipe, slot, capture, level);
        None
    }
}

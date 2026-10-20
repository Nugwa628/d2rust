//! The poke tools' way to the server: a link whose server game can run a
//! poke directive (`d2_sim::poke`) between frames implements
//! [`PokeTarget`]; the bridge forwards the request, so tools reach the
//! game through the bridge like everything else (step 5).

use d2_sim::poke::{GotoTarget, GotoWalk, PokeOp, PokeResult};

/// A link whose server game can run a poke between two frames.
pub trait PokeTarget {
    type Error;
    /// Runs `op` on the state after the last tick (references resolved
    /// there), before the next frame's drain.
    fn poke(&mut self, op: &PokeOp) -> Result<PokeResult, Self::Error>;
    /// Runs one step of a `goto` walk with the state the
    /// caller keeps between steps; returns the result and the new state.
    fn goto_step(
        &mut self,
        target: GotoTarget,
        walk: GotoWalk,
    ) -> Result<(PokeResult, GotoWalk), Self::Error>;
}

impl<L: PokeTarget> super::Bridge<L> {
    /// Runs `op` on the server game, between two frames.
    pub fn poke(&mut self, op: &PokeOp) -> Result<PokeResult, L::Error> {
        self.link.poke(op)
    }

    /// One step of a `goto` walk on the server game, between two frames.
    pub fn goto_step(
        &mut self,
        target: GotoTarget,
        walk: GotoWalk,
    ) -> Result<(PokeResult, GotoWalk), L::Error> {
        self.link.goto_step(target, walk)
    }
}

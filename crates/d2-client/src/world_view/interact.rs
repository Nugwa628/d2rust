//! The `play` preview's pending action: a click on a unit out of reach
//! walks toward it and keeps the action pending
//! ([`ClickOut::Pend`]). Once per client tick the pending action
//! runs ([`crate::bridge::Bridge::pending_step`]):
//! out of reach it requests the walk to the unit again (C→S 0x02 / 0x04)
//! and keeps the record; in reach it runs the action (0x13: the interact
//! sender). The base game runs it in the loop pass
//! after the input and before the server tick, so a click's
//! pend acts in the click's own pass; `play` runs it in
//! `present::pending_frame`, before a bridge frame that ticks.
//!
//! Items and other target types keep the preview's own reading (D2RUST's own,
//! unverified): "arrived" is the predicted walk ([`crate::bridge::predict`])
//! seen and ended, or no walk seen within [`NO_WALK_FRAMES`] ticks. A new
//! press that sets no pending record drops the old one.

use crate::bridge::click::PendStep;
use crate::bridge::link::ServerLink;
use crate::bridge::output::Output;
use crate::bridge::{Bridge, BridgeError};
use crate::controls::click::{ClickOut, Pending};

/// Ticks to wait for the walk to show before interacting anyway (items
/// and other targets). D2RUST's own, unverified.
pub const NO_WALK_FRAMES: u32 = 8;

/// The pending action of the preview (module doc).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreviewInteract {
    pub pending: Option<Pending>,
    /// The predicted walk toward the target was seen (items, other types).
    seen_walk: bool,
    /// Ticks since the pending record was set (items, other types).
    frames: u32,
    /// The server tick the step last ran in: one step per client tick.
    tick: Option<u64>,
}

impl PreviewInteract {
    /// Notes one loop pass's click results: the last [`ClickOut::Pend`]
    /// sets (or clears) the record; a press with none clears it.
    pub fn note(&mut self, outs: &[ClickOut], pressed: bool) {
        let mut set = None;
        for o in outs {
            if let ClickOut::Pend(p) = o {
                set = Some(*p);
            }
        }
        let tick = self.tick;
        match set {
            Some(p) => {
                *self = PreviewInteract {
                    pending: p,
                    tick,
                    ..Default::default()
                }
            }
            None if pressed => {
                *self = PreviewInteract {
                    tick,
                    ..Default::default()
                }
            }
            None => {}
        }
    }

    /// The pending action's step of the client tick after server tick
    /// `tick` (at most once per tick; later calls in the same tick do
    /// nothing). `walking`: the prediction's walk state; `run`: the run
    /// lock (& 8); `local_at`: the player's own (predicted) precise
    /// position, which the reach test of an object reads.
    pub fn frame<L: ServerLink>(
        &mut self,
        bridge: &mut Bridge<L>,
        walking: bool,
        tick: u64,
        run: bool,
        local_at: Option<(u32, u32)>,
    ) -> Result<Vec<Output>, BridgeError> {
        if self.tick == Some(tick) {
            return Ok(Vec::new());
        }
        self.tick = Some(tick);
        let Some(p) = self.pending else {
            return Ok(Vec::new());
        };
        let (step, out) = bridge.pending_step(p, run, local_at)?;
        match step {
            PendStep::Walk => Ok(out),
            PendStep::Stop | PendStep::Act => {
                self.pending = None;
                Ok(out)
            }
            PendStep::Other => {
                self.frames += 1;
                if walking {
                    self.seen_walk = true;
                    return Ok(Vec::new());
                }
                if !self.seen_walk && self.frames < NO_WALK_FRAMES {
                    return Ok(Vec::new());
                }
                *self = PreviewInteract {
                    tick: self.tick,
                    ..Default::default()
                };
                bridge.interact(p.target)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::world::UnitKey;

    fn pend(g: u32) -> ClickOut {
        ClickOut::Pend(Some(Pending {
            code: 0x13,
            target: UnitKey::new(2, g),
        }))
    }

    #[test]
    fn a_pend_is_kept_and_a_bare_press_drops_it() {
        let mut i = PreviewInteract::default();
        i.note(&[ClickOut::Retarget, pend(7)], true);
        assert_eq!(i.pending.unwrap().target, UnitKey::new(2, 7));
        i.note(&[], false);
        assert!(i.pending.is_some(), "no press: kept");
        i.note(&[ClickOut::Retarget], true);
        assert!(i.pending.is_none(), "a press without a pend drops it");
    }
}

//! The NPC approach of the wired host (C→S 0x13 at distance 7–8): the
//! run request to the NPC (no skill, mode 3), then the queued
//! interaction, which runs the 0x13 handling again when the player's run
//! stops ("talk on arrival"). The client sends no second 0x13. Part of
//! `handlers::world`; the item and object approaches have the same shape.
//!
//! D2RUST's own, unverified: the arrival is read from the player's mode
//! leaving walk / run / town walk at the end of tick step 4 (after the
//! timer queue moved the player, `WiredWorld::timer_step_work`), not from
//! the step result of the player's step event; the queue is dropped by
//! the player's next walk request (`clear_queued_action`).

use d2_sim::game::Game;
use d2_sim::units::UnitId;
use d2_sim::world::npc::NpcWorld;

use super::wired::{TradeRest, WiredWorld};
use super::{ActionEvents, NpcRun, WorldHost};

/// Unit modes of a moving player (walk, run, town walk).
const MOVING: [u8; 3] = [2, 3, 6];

impl<R: TradeRest, S> WiredWorld<R, S> {
    /// Drops the queued interaction of `player` (a new walk request).
    pub(super) fn drop_queued(&mut self, player: UnitId) {
        self.state.queued.retain(|q| q.0 != player);
    }

    /// The queued interactions whose run has ended run the 0x13 handling
    /// again (end of tick step 4).
    pub(super) fn arrivals<D: ActionEvents>(&mut self, game: &mut Game, events: &mut D)
    where
        Self: WorldHost<D>,
    {
        let waiting = std::mem::take(&mut self.state.queued);
        for (player, guid) in waiting {
            let moving = self.desk(game, events, |desk, _, _| {
                MOVING.contains(&NpcWorld::mode(desk, player))
            });
            if moving {
                self.state.queued.push((player, guid));
                continue;
            }
            let mut msg = vec![0x13, 1, 0, 0, 0];
            msg.extend_from_slice(&guid.to_le_bytes());
            let call = NpcRun { player, msg: &msg };
            // The arrival does not queue another approach.
            self.arriving = true;
            WorldHost::<D>::npc(self, game, events, call);
            self.arriving = false;
            self.state.approaches.clear();
        }
    }

    /// The approach requests of the last calls start their run and queue
    /// the interaction (end of a tick).
    pub(super) fn approaches<D: ActionEvents>(&mut self, game: &mut Game, events: &mut D) {
        let asked = std::mem::take(&mut self.state.approaches);
        for (player, npc) in asked {
            let guid = self.desk(game, events, |desk, _, _| NpcWorld::guid(desk, npc));
            let started = events.action().with(game, |g, v| {
                v.h.paths.is_some()
                    && d2_sim::wiring::path::PathCtx::of(v, g)
                        .approach_unit(player, npc)
                        .is_some()
            });
            if started {
                self.drop_queued(player);
                self.state.queued.push((player, guid));
            }
        }
    }
}

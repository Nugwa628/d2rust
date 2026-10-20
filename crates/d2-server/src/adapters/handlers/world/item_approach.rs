//! The walk to a ground item of the wired host (C→S 0x16 at distance ≥ 5
//! or with a collision): the run request to the item (player mode 3
//! toward (type 4, GUID)), then the pick-up when the run ends. Part of
//! `handlers::world`. As the NPC approach
//! ([`super::npc_approach`]), the arrival is read from the player's mode
//! leaving walk / run / town walk at the end of tick step 4, and the
//! 0x16 handling runs again with the same cursor flag.
//!
//! The arrival test and the repeated 0x16 stand in for the pick-up that
//! the end of the player's walk triggers. A new walk request drops the
//! queued pick-up (`clear_queued_action`).

use d2_sim::game::Game;
use d2_sim::units::UnitId;
use d2_sim::world::npc::NpcWorld;

use super::wired::{TradeRest, WiredWorld};
use super::{ActionEvents, WorldHost};
use crate::adapters::handlers::items::moves::MoveRun;

/// Unit modes of a moving player (walk, run, town walk).
const MOVING: [u8; 3] = [2, 3, 6];

impl<R: TradeRest, S> WiredWorld<R, S> {
    /// Starts the run of `player` to `item` and queues the pick-up.
    pub(super) fn start_item_walk<D: ActionEvents>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        (player, item, cursor): (UnitId, UnitId, bool),
    ) {
        let Some(guid) = game.lists.unit(item).map(|e| e.guid) else {
            return;
        };
        let started = events.action().with(game, |g, v| {
            v.h.paths.is_some()
                && d2_sim::wiring::path::PathCtx::of(v, g)
                    .approach_unit(player, item)
                    .is_some()
        });
        if started {
            self.drop_queued(player);
            self.item_queued.retain(|q| q.0 != player);
            self.item_queued.push((player, guid, cursor));
        }
    }

    /// The queued pick-ups whose run has ended run the 0x16 handling
    /// again (end of tick step 4, `WiredWorld::timer_step_work`); its
    /// messages join the inventory sends.
    pub(super) fn item_arrivals<D: ActionEvents>(&mut self, game: &mut Game, events: &mut D)
    where
        Self: WorldHost<D>,
    {
        let waiting = std::mem::take(&mut self.item_queued);
        for (player, guid, cursor) in waiting {
            let moving = self.desk(game, events, |desk, _, _| {
                MOVING.contains(&NpcWorld::mode(desk, player))
            });
            if moving {
                self.item_queued.push((player, guid, cursor));
                continue;
            }
            let mut msg = vec![0x16, 4, 0, 0, 0];
            msg.extend_from_slice(&guid.to_le_bytes());
            msg.extend_from_slice(&u32::from(cursor).to_le_bytes());
            let call = MoveRun { player, msg: &msg };
            let Some((_, sent, _)) = WorldHost::<D>::moves(self, game, events, call) else {
                continue;
            };
            // The arrival does not walk again (a second out-of-reach
            // answer is dropped).
            self.item_queued.retain(|q| q.0 != player);
            self.inv_sent
                .extend(sent.into_iter().filter_map(|(u, b)| Some((u?, b))));
        }
    }
}

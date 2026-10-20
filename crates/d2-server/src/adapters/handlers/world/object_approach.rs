//! The walk to an object of the wired host (C→S 0x13, object not in
//! interact range or the line blocked): the run request to the object
//! (player mode 3 toward (type 2, GUID)), then the 0x13 object case again
//! when the run ends. Part of `handlers::world`; same shape as the NPC
//! and item approaches ([`super::npc_approach`], [`super::item_approach`]).
//!
//! The queued operate fires only on the player's stop: step result 2 of
//! the player's step event, which the sim lists in
//! `ActionHooks::player_stops`; it is taken at the end of tick step 4. A
//! mode change for any other reason (hit, knockback, death, a skill)
//! keeps the operate queued; a new run request drops it. The retry runs
//! in the stop's tick; a run it starts there is ended at once by the
//! ENDANIM neutral start of the step result 2 and the queued
//! action is cleared (the player stops at the blocked step, the retry's
//! run is computed, the mode is neutral that frame and stays).

use d2_sim::game::Game;
use d2_sim::units::UnitId;
use d2_sim::wiring::action::ObjectCase;

use super::wired::{TradeRest, WiredWorld};
use super::{ActionEvents, WaypointOperate, WorldHost};

impl<R: TradeRest, S> WiredWorld<R, S> {
    /// Starts the run of `player` to `object` and queues the operate.
    /// `retry`: called from the stop-frame retry (no new queue; the run is
    /// ended by the neutral start that follows the step result 2).
    pub(super) fn start_object_walk<D: ActionEvents>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        (player, object): (UnitId, UnitId),
        retry: bool,
    ) {
        let Some(guid) = game.lists.unit(object).map(|e| e.guid) else {
            return;
        };
        let started = events.action().with(game, |g, v| {
            v.h.paths.is_some()
                && d2_sim::wiring::path::PathCtx::of(v, g)
                    .approach_unit(player, object)
                    .is_some()
        });
        if started && retry {
            events.action().with(game, |g, v| {
                d2_sim::wiring::path::PathCtx::of(v, g).neutral_start(player);
            });
        } else if started {
            self.drop_queued(player);
            self.item_queued.retain(|q| q.0 != player);
            self.object_queued.retain(|q| q.0 != player);
            self.tile_queued.retain(|q| q.0 != player);
            self.object_queued.push((player, guid));
        }
    }

    /// The queued operates of the players that stopped (step result 2)
    /// this tick run the 0x13 object case again (end of tick step 4).
    pub(super) fn object_arrivals<D: ActionEvents>(&mut self, game: &mut Game, events: &mut D)
    where
        Self: WorldHost<D>,
    {
        let stops = events
            .action()
            .with(game, |_, v| std::mem::take(&mut v.h.player_stops));
        let waiting = std::mem::take(&mut self.object_queued);
        for (player, guid) in waiting {
            if !stops.contains(&player) {
                self.object_queued.push((player, guid));
                continue;
            }
            match WorldHost::<D>::objects(self, game, events, player, guid) {
                // Still out of range: a new run (the queued action cleared after it),
                // ended by the ENDANIM neutral start (module doc).
                Some(ObjectCase::Walk(object)) => {
                    self.start_object_walk(game, events, (player, object), true);
                }
                Some(ObjectCase::Waypoint(_)) => {
                    let _ = WorldHost::<D>::waypoints(
                        self,
                        game,
                        events,
                        WaypointOperate { player, guid },
                    );
                }
                Some(ObjectCase::Code(_)) | None => {}
            }
        }
        // The tile walks: the pending 0x13 `{5, GUID}` fires on the stop;
        // within 5 it warps, else the run restarts and is ended by the
        // neutral start like the object retry.
        let waiting = std::mem::take(&mut self.tile_queued);
        for (player, guid) in waiting {
            if !stops.contains(&player) {
                self.tile_queued.push((player, guid));
                continue;
            }
            let _ = events.action().warp_tile_message(game, player, guid);
            let again = events
                .action()
                .with(game, |_, v| std::mem::take(&mut v.h.tile_walks));
            if !again.is_empty() {
                events.action().with(game, |g, v| {
                    d2_sim::wiring::path::PathCtx::of(v, g).neutral_start(player);
                });
            }
        }
    }
}

//! The hireling callers of the wired host that the action wiring queues
//! ([`HirelingCall`] on `ActionHooks::hireling_calls`): the save restore,
//! the join follow and the act change's hireling part, run on this host's
//! hireling lists ([`WiredWorld::state`]) with the rules of
//! `d2_sim::world::hirelings::life`. Part of `handlers::world`; the tick
//! drives it through [`WiredWorld::hireling_calls`].
//!
//! TODO: each call would run inside its caller (the save load, the join
//! placement, the level warp); here it runs when the handler or tick that
//! queued it returns (the hireling lists are this host's, not the action
//! wiring's), so its messages follow the call's other messages. A restore
//! queued by the load runs before the join follow the entry queued (queue
//! order). The classic act change's extra step runs after the warp's
//! placement, where it would run before it.

use d2_sim::game::Game;
use d2_sim::items::inventory::UnitKind;
use d2_sim::units::lifecycle::{ActTail, AllocRequest};
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::action::HirelingCall;
use d2_sim::wiring::interaction::InteractionError;
use d2_sim::world::hirelings::life::{self, Loader, RestoreSkip, SavedHireling, MODE_DEAD};
use d2_sim::world::hirelings::{level, HirelingError};
use d2_sim::world::npc::NpcWorld;

use super::wired::{TradeRest, WiredWorld};
use super::ActionEvents;

/// The player's act as the hireling restore reads it: the client act (the
/// act of the client's room) when the player has a client, else the
/// unit's act (the act of its room); neither → 0.
fn player_act(game: &Game, player: UnitId) -> u32 {
    let l = &game.lists;
    let client_room = l
        .clients()
        .into_iter()
        .filter_map(|c| l.client(c))
        .find(|e| e.player == Some(player))
        .and_then(|e| e.room);
    let room = client_room.or_else(|| l.unit(player).and_then(|e| e.room()));
    room.and_then(|r| l.room(r)).map_or(0, |r| u32::from(r.act))
}

impl<R: TradeRest + Default + 'static, S> WiredWorld<R, S> {
    /// The quest/NPC act changes the calls asked
    /// (`LifecycleHooks::request_act_change(game, player, level, arg)`,
    /// Warriv's "Go East"): the level warp with the argument as its spawn
    /// tile index, which becomes the act change when the level is in
    /// another act ([`d2_sim::wiring::path::act_change`]). Run when the
    /// handler or tick that asked returns.
    ///
    /// TODO: the request's level-0 form (the town of the player's act)
    /// and its same-act form (a class-59 pair at the player's (x − 5, y))
    /// are not run: no caller reaches them.
    fn act_changes<D: ActionEvents>(&mut self, game: &mut Game, events: &mut D) {
        let a = events.action();
        if a.sys.hooks.act_changes.is_empty() || a.sys.hooks.paths.is_none() {
            a.sys.hooks.act_changes.clear();
            return;
        }
        let q = std::mem::take(&mut a.sys.hooks.act_changes);
        // The town-leave refresh's quest part (the act's NPC intro list,
        // Lut Gholein's included) runs on the lent quest parts and queues its 0x5D at once, ahead of
        // the act change's messages.
        self.lend_quests(events, |_, ev| {
            ev.action().with(game, |g, v| {
                for (player, level, arg) in q {
                    let c = d2_sim::wiring::path::PathCtx::of(v, g);
                    if d2_sim::wiring::path::place::level_warp(c, player, level, arg).is_none() {
                        let c = d2_sim::wiring::path::PathCtx::of(v, g);
                        d2_sim::wiring::path::act_change::run(c, player, level, arg);
                    }
                }
            })
        });
        // Then the calls the act travel held behind the change.
        let tails = std::mem::take(&mut events.action().sys.hooks.act_tails);
        if tails.is_empty() {
            return;
        }
        self.desk(game, events, |desk, _, _| {
            for t in tails {
                match t {
                    ActTail::Completion {
                        player,
                        npc,
                        level,
                        from,
                    } => NpcWorld::act_completion(desk, player, npc, level, from),
                    ActTail::Waypoint { player, level } => {
                        NpcWorld::activate_waypoint(desk, player, level)
                    }
                }
            }
        });
    }

    /// The hireling calls the action wiring queued, in order
    /// (`ActionHooks::hireling_calls`, on from the first frame). Without
    /// hireling tables the game has no hireling: the queue is dropped.
    /// A restore's skip is not reported (the queue's loader is
    /// [`Loader::Current`], whose skips are "no hireling"); a fatal skip
    /// goes to the interaction errors.
    pub fn hireling_calls<D: ActionEvents>(&mut self, game: &mut Game, events: &mut D) {
        self.act_changes(game, events);
        self.town_leaves(game, events);
        let q = events
            .action()
            .sys
            .hooks
            .hireling_calls
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default();
        if q.is_empty() || self.state.hireling_tables.is_none() {
            return;
        }
        for call in q {
            match call {
                HirelingCall::Restore {
                    player,
                    saved,
                    loader,
                } => {
                    self.state.join_capture = Some((player, Vec::new()));
                    if let Err(RestoreSkip::Fatal(id)) =
                        self.restore_hireling(game, events, player, &saved, loader)
                    {
                        self.state
                            .errors
                            .push(InteractionError::Hireling(HirelingError::Fatal(id)));
                    }
                    if let Some((_, held)) = self.state.join_capture.take() {
                        if !held.is_empty() {
                            events
                                .action()
                                .sys
                                .hooks
                                .session
                                .join_pets
                                .entry(player)
                                .or_default()
                                .extend(held);
                        }
                    }
                }
                HirelingCall::JoinFollow(p) => {
                    self.desk(game, events, |desk, _, _| {
                        desk.with_hirelings(|w, t, st| life::join_follow(w, t, st, p))
                    });
                }
                HirelingCall::ActChange(p) => {
                    self.desk(game, events, |desk, _, _| {
                        desk.with_hirelings(|w, t, st| {
                            // Classic runs its extra act-change step first; then
                            // the follow runs in both game types.
                            if !d2_sim::world::hirelings::HirelingWorld::expansion(w) {
                                life::classic_act_change(w, st, p);
                            }
                            life::follow(w, t, st, p);
                        })
                    });
                }
                HirelingCall::LevelUp {
                    player,
                    merc,
                    level: l,
                } => {
                    let r = self.desk(game, events, |desk, _, _| {
                        desk.with_hirelings(|w, t, st| {
                            level::apply_level(w, t, st, player, Some(merc), l);
                            let r = level::send_stats(w, st, player);
                            d2_sim::world::hirelings::HirelingWorld::level_events(w, player, merc);
                            r
                        })
                    });
                    if let Some(Err(e)) = r {
                        self.state.errors.push(InteractionError::Hireling(e));
                    }
                }
            }
        }
    }

    /// The save restore for `player` with the saved hireling `saved`: the
    /// plan ([`life::restore_plan`]); the hire slot of the name in the
    /// hire list of the seller's NPC record marked hired when it is not
    /// offered; the unit of the plan's class and mode allocated **with no
    /// room** at (0, 0); the init and the node's saved values
    /// ([`life::restore`]); experience and level
    /// ([`level::restore_experience`], not on the version-0x47 path); the
    /// dead steps and the inventory in the loader's order
    /// ([`life::restore_tail`]; the inventory is the inventory model's,
    /// [`WiredWorld::inventory`]). `Ok(None)`: no unit (no hireling
    /// tables, the allocation refused, or the level loop freed it). A
    /// living hireling gets a room at the join follow.
    pub fn restore_hireling<D: ActionEvents>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        player: UnitId,
        saved: &SavedHireling,
        loader: Loader,
    ) -> Result<Option<UnitId>, RestoreSkip> {
        let Some(t) = self.state.hireling_tables.as_ref() else {
            return Ok(None);
        };
        let a = events.action();
        let expansion = a.sys.data.expansion;
        let difficulty = a.sys.hooks.ai_info.difficulty;
        let plan = life::restore_plan(
            t,
            expansion,
            difficulty,
            saved,
            player_act(game, player),
            loader,
        )?;
        // The slot of the name, only when it is not offered: the offer
        // list is indexed at name − NameFirst; with one name range per act
        // that is the slot holding the name, found here by search. A seller
        // without a hire list marks nothing (a missing NPC record is no
        // error here:
        // D2RUST marks nothing).
        let slot = u16::try_from(plan.seller)
            .ok()
            .and_then(|c| self.npc.record_mut(c))
            .and_then(|r| r.hire.as_mut())
            .and_then(|h| h.slots.iter_mut().find(|s| s.name == plan.name));
        if let Some(s) = slot {
            if !s.offered {
                s.hired = true;
            }
        }
        // The unit is allocated with the plan's class and mode, no room, and
        // added to the game.
        let req = AllocRequest {
            ty: UnitType::Monster,
            class: plan.class,
            room: None,
            add: true,
            fixed_guid: None,
            mode: u32::from(plan.mode),
            allied: false,
        };
        // TODO: what a refused allocation does is undecided; there is no
        // hireling.
        let Some(merc) = events.action().with(game, |g, v| v.allocate(g, &req, 0, 0)) else {
            return Ok(None);
        };
        let dead = plan.mode == MODE_DEAD;
        let kept = self.desk(game, events, |desk, _, inv| {
            let r = desk.with_hirelings(|w, t, st| -> Result<bool, HirelingError> {
                life::restore(w, t, st, player, merc, &plan, saved)?;
                if loader != Loader::OldV47
                    && !level::restore_experience(
                        w,
                        t,
                        st,
                        player,
                        merc,
                        saved.experience,
                        loader.is_old(),
                    )?
                {
                    return Ok(false);
                }
                life::restore_tail(w, st, player, merc, dead, loader);
                life::restore_stats(w, st, player, merc);
                Ok(true)
            });
            let kept = match r {
                Some(Ok(k)) => k,
                Some(Err(e)) => {
                    desk.state.errors.push(InteractionError::Hireling(e));
                    false
                }
                None => false,
            };
            // A hireling with no inventory gets one on the inventory model
            // (the rest's `ensure_inventory` has none).
            if kept {
                if let (Some(inv), Some(guid)) = (inv, desk.econ.units.get(merc).map(|r| r.guid)) {
                    let kind = UnitKind::Monster { class: plan.class };
                    inv.state.add_inventory(merc, kind, guid);
                }
            }
            kept
        });
        Ok(kept.then_some(merc))
    }
}

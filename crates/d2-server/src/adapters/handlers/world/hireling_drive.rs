//! The host's per-frame hireling work, after the hireling calls and the
//! pet follows: the pet sweep, then the hireling facts the Hireable think
//! reads ([`ActionHooks::hireling_ai`]). Part of the wired world host; the
//! game tick calls [`WiredWorld::drive_hirelings`] once per frame.
//!
//! The hirelings and summoned pets think by their monstats AI (Hireable
//! for a hireling): the hire and the save restore run on the lent
//! monster world, so the unit has its AI control and its owner link;
//! a summon gets the link at its spawn. The live host makes no unit
//! without the link, so it needs no stand-in think.
//!
//! [`ActionHooks::hireling_ai`]: d2_sim::wiring::action::ActionHooks

use d2_sim::game::Game;
use d2_sim::units::UnitType;

use super::wired::{TradeRest, WiredWorld};
use super::ActionEvents;

impl<R: TradeRest, S> WiredWorld<R, S> {
    /// One frame of the host's hireling work (module docs): the pet
    /// sweep, then the hireling facts. Without hireling tables there are
    /// no facts.
    pub fn drive_hirelings<D: ActionEvents>(&mut self, game: &mut Game, events: &mut D) {
        events.action().with(game, |g, v| v.pet_sweep(g));
        self.publish_hireling_facts(game, events);
    }
}

impl<R: TradeRest, S> WiredWorld<R, S> {
    /// The hireling facts of the Hireable AI (`ActionHooks::hireling_ai`):
    /// each living hireling unit's owner and node `Id` from the hireling
    /// lists, and the rows of the hireling tables (set once).
    fn publish_hireling_facts<D: ActionEvents>(&mut self, game: &Game, events: &mut D) {
        let Some(t) = self.state.hireling_tables.as_ref() else {
            return;
        };
        let ids = self
            .state
            .hirelings
            .lists
            .iter()
            .flat_map(|(&p, l)| l.nodes.iter().filter(|n| !n.dead).map(move |n| (p, n)))
            .filter_map(|(p, n)| {
                let m = game.lists.find_unit(UnitType::Monster, n.guid)?;
                Some((m, (p, n.id)))
            })
            .collect();
        let facts = &mut events.action().sys.hooks.hireling_ai;
        facts.ids = ids;
        if facts.rows.is_none() {
            facts.rows = Some(std::sync::Arc::new(t.rows.clone()));
        }
    }
}

//! The action wiring's side of a mod game's event hooks (MS11,
//! [`crate::rules::EventHooks`]): [`CombatView`] is the hooks'
//! [`EventHost`], and [`fire`] / [`ActionSim::mod_event`] ask the game's
//! hooks at each event point. Without hooks (every D2RUST game) nothing
//! is asked: one check of the ruleset.

use crate::game::Game;
use crate::rng::Seed;
use crate::rules::{EventHooks, EventHost};
use crate::units::{UnitId, UnitType};

use super::combat::CombatView;
use super::{ActionSim, Pending};

impl<X: Pending> EventHost for CombatView<'_, X> {
    fn game(&mut self) -> &mut Game {
        self.game
    }
    fn unit_type(&self, unit: UnitId) -> Option<UnitType> {
        self.game.lists.unit(unit).map(|e| e.ty)
    }
    fn class_id(&self, unit: UnitId) -> i32 {
        self.v.units.get(unit).map_or(0, |r| r.class as i32)
    }
    fn stat(&self, unit: UnitId, stat: u16, layer: u16) -> i32 {
        self.v.stats.unit_total(unit, stat, layer)
    }
    fn set_stat(&mut self, unit: UnitId, stat: u16, value: i32) {
        self.v.set_base(unit, stat, value);
    }
    fn has_state(&self, unit: UnitId, state: u16) -> bool {
        self.v.stats.has_state(unit, u32::from(state))
    }
    fn seed(&mut self, unit: UnitId) -> Option<&mut Seed> {
        self.v.units.get(unit)?;
        Some(self.v.seed(unit))
    }
    fn level(&self, unit: UnitId) -> Option<u32> {
        let room = self.game.lists.unit(unit)?.room()?;
        self.v.h.drlg.level_id(&*self.game, room)
    }
    fn send(&mut self, player: UnitId, bytes: &[u8]) {
        self.v.h.x.send(player, bytes);
    }
}

/// Asks the game's event hooks with the view as their host; nothing under
/// D2RUST's own rules or when the mod game has no event hooks.
pub fn fire<X: Pending, R>(
    cv: &mut CombatView<'_, X>,
    f: impl FnOnce(&dyn EventHooks, &mut dyn EventHost) -> R,
) -> Option<R> {
    cv.v.h.rules.events()?;
    // The ruleset is cloned (one `Arc`) so the hook can borrow the view.
    let rules = cv.v.h.rules.clone();
    let hooks = rules.events()?;
    Some(f(hooks, cv))
}

impl<X: Pending> ActionSim<X> {
    /// Asks the game's event hooks ([`fire`]) over this game.
    pub fn mod_event<R>(
        &mut self,
        game: &mut Game,
        f: impl FnOnce(&dyn EventHooks, &mut dyn EventHost) -> R,
    ) -> Option<R> {
        self.sys.hooks.rules.events()?;
        self.combat(game, |cv, _| fire(cv, f))
    }

    /// The game was created ([`EventHooks::game_created`]); its builder or
    /// session layer calls this once, before the first client loads.
    pub fn game_created(&mut self, game: &mut Game) {
        self.mod_event(game, |e, h| e.game_created(h));
    }

    /// The game's last client left ([`EventHooks::game_freed`]); its
    /// session layer calls this once.
    pub fn game_freed(&mut self, game: &mut Game) {
        self.mod_event(game, |e, h| e.game_freed(h));
    }

    /// The end of the tick: the periodic hook when its period is due
    /// (counted in base ticks, so a faster tick rate keeps its timing).
    pub(super) fn periodic_events(&mut self, game: &mut Game) {
        let Some(period) = self.sys.hooks.rules.events().map(|e| e.period()) else {
            return;
        };
        let Ok(period) = i32::try_from(period) else {
            return;
        };
        if period == 0 || !crate::tick::is_due(game.frame, game.ticks(period)) {
            return;
        }
        self.mod_event(game, |e, h| e.periodic(h));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::rules::{ModRules, Ruleset};
    use crate::tick::TickHooks;
    use crate::wiring::action::reaction::kill_by;
    use crate::wiring::action::tests::Fx;

    /// Test event hooks: every call logged; a chat line starting with `!`
    /// is taken and answered.
    #[derive(Debug, Default)]
    struct Log {
        seen: Mutex<Vec<String>>,
        period: u32,
    }

    impl Log {
        fn push(&self, s: String) {
            self.seen.lock().unwrap().push(s);
        }
        fn seen(&self) -> Vec<String> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl EventHooks for Log {
        fn game_created(&self, h: &mut dyn EventHost) {
            self.push(format!("created {}", h.game().frame));
        }
        fn game_freed(&self, h: &mut dyn EventHost) {
            self.push(format!("freed {}", h.game().frame));
        }
        fn player_joined(&self, _: &mut dyn EventHost, player: UnitId) {
            self.push(format!("joined {}", player.0));
        }
        fn unit_killed(&self, h: &mut dyn EventHost, killer: Option<UnitId>, victim: UnitId) {
            let ty = h.unit_type(victim);
            self.push(format!(
                "killed {:?} {} {ty:?}",
                killer.map(|k| k.0),
                victim.0
            ));
        }
        fn period(&self) -> u32 {
            self.period
        }
        fn periodic(&self, h: &mut dyn EventHost) {
            self.push(format!("periodic {}", h.game().frame));
        }
        fn level_entered(&self, _: &mut dyn EventHost, player: UnitId, from: Option<u32>, to: u32) {
            self.push(format!("level {} {from:?} {to}", player.0));
        }
        fn chat_command(&self, h: &mut dyn EventHost, player: UnitId, text: &[u8]) -> bool {
            self.push(format!(
                "chat {} {}",
                player.0,
                String::from_utf8_lossy(text)
            ));
            if text.starts_with(b"!") {
                h.send(player, b"ok");
                return true;
            }
            false
        }
    }

    #[derive(Debug)]
    struct TestGame(Arc<Log>);

    impl ModRules for TestGame {
        fn name(&self) -> &str {
            "test-events"
        }
        fn events(&self) -> Option<&dyn EventHooks> {
            Some(self.0.as_ref())
        }
    }

    fn modded(fx: &mut Fx, period: u32) -> Arc<Log> {
        let log = Arc::new(Log {
            period,
            ..Log::default()
        });
        fx.sim.sys.hooks.rules = Ruleset::Mod(Arc::new(TestGame(log.clone())));
        log
    }

    /// A kill tells the hooks the killer and the victim, after D2RUST's
    /// kill (the monster is in its death mode by then).
    #[test]
    fn a_kill_reaches_the_hooks_after_the_kill() {
        let mut fx = Fx::new();
        let log = modded(&mut fx, 0);
        let a = fx.a;
        let p = fx.spawn(UnitType::Player, 0, a, 10, 10);
        let m = fx.spawn(UnitType::Monster, 0, a, 12, 12);
        fx.sim.combat(&mut fx.game, |w, _| kill_by(w, m, Some(p)));
        assert_eq!(
            log.seen(),
            [format!("killed Some({}) {} Some(Monster)", p.0, m.0)]
        );
        assert_eq!(
            fx.sim.sys.units.get(m).map(|r| r.mode),
            Some(crate::units::modes::monster_mode::DT)
        );
    }

    /// The periodic hook runs at the end of every tick its period (in base
    /// ticks) is due; a period of 0 never runs.
    #[test]
    fn the_periodic_hook_runs_every_period() {
        let mut fx = Fx::new();
        let log = modded(&mut fx, 5);
        for _ in 0..12 {
            fx.tick();
        }
        assert_eq!(log.seen(), ["periodic 5", "periodic 10"]);
        let mut fx = Fx::new();
        let log = modded(&mut fx, 0);
        for _ in 0..12 {
            fx.tick();
        }
        assert!(log.seen().is_empty());
    }

    /// Created and freed come from the game's session layer; join and the
    /// first level come from the tick's client steps.
    #[test]
    fn game_join_and_level_events() {
        let mut fx = Fx::new();
        let log = modded(&mut fx, 0);
        fx.sim.game_created(&mut fx.game);
        let a = fx.a;
        let p = fx.spawn(UnitType::Player, 0, a, 10, 10);
        let c = fx.game.lists.add_client(Some(p), None, 0);
        fx.sim.join_sequence(&mut fx.game, c);
        fx.sim.client_level_change(&mut fx.game, c);
        fx.sim.game_freed(&mut fx.game);
        let level = crate::wiring::action::tests::LEVEL;
        assert_eq!(
            log.seen(),
            [
                "created 0".to_string(),
                format!("joined {}", p.0),
                format!("level {} None {level}", p.0),
                "freed 0".to_string(),
            ]
        );
    }

    /// A chat command is the hook's answer; its host sends to the player.
    #[test]
    fn a_chat_command_is_taken_by_the_hook() {
        let mut fx = Fx::new();
        let log = modded(&mut fx, 0);
        let a = fx.a;
        let p = fx.spawn(UnitType::Player, 0, a, 10, 10);
        fx.sim.sys.hooks.x.sent.clear();
        let taken = |fx: &mut Fx, text: &'static [u8]| {
            fx.sim
                .mod_event(&mut fx.game, |e, h| e.chat_command(h, p, text))
        };
        assert_eq!(taken(&mut fx, b"!where"), Some(true));
        assert_eq!(taken(&mut fx, b"hello"), Some(false));
        assert_eq!(fx.sim.sys.hooks.x.sent, [(p, b"ok".to_vec())]);
        assert_eq!(log.seen().len(), 2);
    }

    /// Under D2RUST's own rules nothing is asked.
    #[test]
    fn d2rusts_own_rules_ask_nothing() {
        let mut fx = Fx::new();
        let a = fx.a;
        let p = fx.spawn(UnitType::Player, 0, a, 10, 10);
        let asked = fx
            .sim
            .mod_event(&mut fx.game, |e, h| e.chat_command(h, p, b"!x"));
        assert_eq!(asked, None);
    }
}

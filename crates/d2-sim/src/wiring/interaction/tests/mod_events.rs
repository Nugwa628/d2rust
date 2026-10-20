//! A mod game's event functions (`docs/plans/mods/seams.md` MS7b): asked
//! first for the functions its item provider answers, also past D2RUST's
//! ids; `None` and the functions it doesn't answer run D2RUST's own.

use std::sync::Arc;

use crate::combat::events::EventCall;
use crate::combat::{CombatWorld, DamageRecord};
use crate::items::provider::ItemProvider;
use crate::rules::{ModRules, Ruleset, SkillHost};
use crate::skills::use_::bodies::Handler;
use crate::units::{UnitId, UnitType};

use super::skill_use::{skills, Fx};
use super::stat_data;

/// `restinpeace`, set on O by D2RUST's function 29.
const RESTINPEACE: u32 = 172;
const FN_REST_IN_PEACE: i32 = 29;
/// An event function id D2RUST has none for.
const MOD_FN: i32 = 40;
/// The stat the mod's functions mark on O: strength.
const MARK: u16 = 0;
const EVENT: u8 = 9;

/// Function 40 marks O with the handler's level and halves the event's
/// physical damage; function 29 does the same when `rest` is set, and
/// otherwise falls through to D2RUST's.
#[derive(Debug)]
struct Events {
    rest: bool,
}

impl ItemProvider for Events {
    fn has_event_fn(&self, func: i32) -> bool {
        func == MOD_FN || func == FN_REST_IN_PEACE
    }
    fn event_fn(
        &self,
        w: &mut dyn SkillHost,
        func: i32,
        call: &EventCall<UnitId>,
        record: Option<&mut DamageRecord>,
    ) -> Option<i32> {
        if func == FN_REST_IN_PEACE && !self.rest {
            return None;
        }
        w.set_stat(call.o?, MARK, call.l);
        if let Some(r) = record {
            r.physical /= 2;
        }
        Some(1)
    }
}

#[derive(Debug)]
struct Game(Events);

impl ModRules for Game {
    fn name(&self) -> &str {
        "mod-events"
    }
    fn items(&self) -> Option<&dyn ItemProvider> {
        Some(&self.0)
    }
}

fn handler(func: i32) -> Handler {
    Handler {
        event: EVENT,
        key_type: 1,
        key: 1,
        skill: 0,
        level: 7,
        func,
    }
}

/// The player holds one handler of `func`; event 9 runs with the monster
/// as O and a record of 100 physical damage. Returns (mark on O, O has
/// `restinpeace`, the record's physical damage).
fn run(rules: Ruleset, func: i32) -> (i32, bool, i32) {
    let mut fx = Fx::with(stat_data(), skills());
    let p = fx.spawn(UnitType::Player, 10, 10);
    let m = fx.spawn(UnitType::Monster, 12, 10);
    fx.sim.sys.hooks.rules = rules;
    fx.sim.sys.hooks.enable_unit_events();
    fx.sim.sys.hooks.handlers.insert(p, vec![handler(func)]);
    let mut rec = DamageRecord {
        physical: 100,
        ..DamageRecord::default()
    };
    fx.sim.with(&mut fx.game, |g, v| {
        CombatWorld::unit_event(&mut v.combat(g), EVENT, p, m, &mut rec)
    });
    let mark = fx.sim.with(&mut fx.game, |_, v| v.stat(m, MARK));
    (
        mark,
        fx.sim.sys.stats.has_state(m, RESTINPEACE),
        rec.physical,
    )
}

fn mod_rules(rest: bool) -> Ruleset {
    Ruleset::Mod(Arc::new(Game(Events { rest })))
}

#[test]
fn unknown_function_does_nothing_under_original() {
    assert_eq!(run(Ruleset::Original, MOD_FN), (0, false, 100));
}

#[test]
fn mod_function_past_the_table_runs() {
    assert_eq!(run(mod_rules(false), MOD_FN), (7, false, 50));
}

#[test]
fn mod_overrides_a_d2rust_function_or_falls_through() {
    assert_eq!(run(mod_rules(true), FN_REST_IN_PEACE), (7, false, 50));
    let own = run(Ruleset::Original, FN_REST_IN_PEACE);
    assert_eq!(own, (0, true, 100));
    assert_eq!(run(mod_rules(false), FN_REST_IN_PEACE), own);
}

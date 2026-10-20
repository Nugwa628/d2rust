//! A mod game's AI (`docs/plans/mods/seams.md` MS5): its table rows and
//! bodies answer before D2RUST's, past D2RUST's table and in place of its
//! own; without an answer D2RUST's run.

use std::sync::{Arc, Mutex};

use super::*;
use crate::rules::{ModRules, MonsterProvider, Ruleset};

/// A monstats `AI` index past D2RUST's table.
const NEW_AI: u16 = 200;
/// The mod's own init and think ids.
const MOD_INIT: u32 = 1;
const MOD_THINK: u32 = 2;
/// D2RUST's Zombie: AI row 3 and its think.
const ZOMBIE_AI: u16 = 3;
const ZOMBIE: u32 = 0x005E_FE20;
/// A special AI state D2RUST has no row for.
const NEW_STATE: u32 = 30;

const MOD_ROW: AiRecord = AiRecord {
    target_mode: 1,
    init: MOD_INIT,
    think: MOD_THINK,
    alt: 0,
};

/// The mod's AI: row `NEW_AI` and special state `NEW_STATE` run its own
/// init and think; with `replace` its think also stands in for the
/// Zombie's. Its think walks to (1, 2) through D2RUST's mode helper.
#[derive(Debug, Default)]
struct ModAi {
    replace: bool,
    log: Mutex<Vec<String>>,
}

impl ModAi {
    fn note(&self, s: String) {
        self.log.lock().unwrap().push(s);
    }
}

impl MonsterProvider for ModAi {
    fn ai_record(&self, ai: u16) -> Option<AiRecord> {
        (ai == NEW_AI).then_some(MOD_ROW)
    }
    fn special_record(&self, state: u32, _class: i32) -> Option<AiRecord> {
        (state == NEW_STATE).then_some(MOD_ROW)
    }
    fn ai_init(
        &self,
        _game: &mut Game,
        cx: &mut Ctx<'_, dyn AiHost + '_>,
        function: u32,
        unit: UnitId,
    ) -> bool {
        if function != MOD_INIT {
            return false;
        }
        cx.store.control_mut(unit).unwrap().params = [7, 0, 0];
        self.note(format!("init {function}"));
        true
    }
    fn ai_think(
        &self,
        game: &mut Game,
        cx: &mut Ctx<'_, dyn AiHost + '_>,
        function: u32,
        unit: UnitId,
        p: &TickParam,
    ) -> bool {
        if function != MOD_THINK && !(self.replace && function == ZOMBIE) {
            return false;
        }
        self.note(format!("think {function}"));
        let _ = p;
        request_mode(game, cx, unit, mode::WALK, ModeTarget::Point(1, 2));
        true
    }
}

impl ModRules for ModAi {
    fn name(&self) -> &str {
        "mod-ai"
    }
    fn monsters(&self) -> Option<&dyn MonsterProvider> {
        Some(self)
    }
}

fn install_mod(w: &mut World, replace: bool) -> Arc<ModAi> {
    let m = Arc::new(ModAi {
        replace,
        ..ModAi::default()
    });
    w.fake.rules = Ruleset::Mod(m.clone());
    m
}

fn reinstall(w: &mut World, state: u32) {
    let mon = w.mon;
    w.with(|g, cx| install(g, cx, mon, state));
}

#[test]
fn original_rules_give_row_0_past_the_table() {
    let mut w = World::new(monstats(NEW_AI, [0; 5], 15));
    let mon = w.mon;
    assert_eq!(w.with(|_, cx| cx.record(mon)), AI_TABLE[0]);
    assert_eq!(w.store.control(mon).unwrap().function, AI_TABLE[0].think);
}

#[test]
fn a_mod_row_past_the_table_installs_and_runs_its_bodies() {
    let mut w = World::new(monstats(NEW_AI, [0; 5], 15));
    let m = install_mod(&mut w, false);
    let mon = w.mon;
    reinstall(&mut w, 0);
    let c = w.store.control(mon).unwrap();
    assert_eq!((c.function, c.params), (MOD_THINK, [7, 0, 0]));
    w.run(true, 5);
    assert_eq!(*m.log.lock().unwrap(), ["init 1", "think 2"]);
    assert_eq!(w.fake.modes().len(), 1);
    assert!(w.store.unhandled.is_empty());
}

#[test]
fn a_mod_special_state_row_installs() {
    let mut w = World::new(monstats(ZOMBIE_AI, [0; 5], 15));
    install_mod(&mut w, false);
    let mon = w.mon;
    reinstall(&mut w, NEW_STATE);
    let c = w.store.control(mon).unwrap();
    assert_eq!((c.function, c.special_state), (MOD_THINK, NEW_STATE));
    // D2RUST's rows stay for the states it has.
    reinstall(&mut w, 0);
    assert_eq!(w.store.control(mon).unwrap().function, ZOMBIE);
}

#[test]
fn a_mod_think_replaces_d2rusts_for_its_id() {
    // D2RUST's Zombie, then the same think with a mod that leaves it,
    // then one that replaces it.
    let run = |replace: Option<bool>| {
        let mut w = World::new(monstats(ZOMBIE_AI, [0; 5], 15));
        let m = replace.map(|r| install_mod(&mut w, r));
        w.seed(12345);
        w.run(true, 5);
        let log = m.map(|m| m.log.lock().unwrap().clone()).unwrap_or_default();
        (w.fake.log.clone(), log)
    };
    let (original, _) = run(None);
    assert_eq!(run(Some(false)), (original.clone(), Vec::new()));
    let (replaced, log) = run(Some(true));
    assert_eq!(log, ["think 6225440"]);
    assert_ne!(replaced, original);
    let modes: Vec<&String> = replaced.iter().filter(|s| s.starts_with("mode")).collect();
    assert_eq!(modes, ["mode 2 Point(1, 2)"]);
}

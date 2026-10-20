//! A mod game's umods (`docs/plans/mods/seams.md` MS5): its init and
//! callbacks run before D2RUST's, for umods past D2RUST's table and in
//! place of D2RUST's own; without an answer D2RUST's run.

use std::sync::Arc;

use super::*;
use crate::rules::{ModRules, MonsterProvider, Ruleset};

/// A umod id past D2RUST's table.
const NEW_UMOD: u8 = 50;
/// D2RUST's "leveladd": level + 3, experience × 5.
const LEVELADD: u8 = 4;
/// D2RUST's umod with a type-7 callback (restarts the think).
const EVENT7_UMOD: u8 = 41;

/// The mod's umods: `NEW_UMOD`, and `LEVELADD` / `EVENT7_UMOD` replaced
/// when `replace` is set. Its init adds 7 levels; its callbacks add 1 to
/// stat `MARK` per call.
#[derive(Debug)]
struct Umods {
    replace: bool,
}

const MARK: u16 = 300;

impl Umods {
    fn owns(&self, umod: u8) -> bool {
        umod == NEW_UMOD || (self.replace && matches!(umod, LEVELADD | EVENT7_UMOD))
    }
}

impl MonsterProvider for Umods {
    fn umod_init(
        &self,
        _cx: &Ctx<'_>,
        h: &mut dyn InitHost,
        unit: UnitId,
        umod: u8,
        _unique: bool,
    ) -> bool {
        if !self.owns(umod) {
            return false;
        }
        let l = h.stat(unit, stat::LEVEL);
        h.set_stat(unit, stat::LEVEL, l + 7);
        true
    }
    fn umod_callback(
        &self,
        _cx: &Ctx<'_>,
        h: &mut dyn InitHost,
        unit: UnitId,
        umod: u8,
        _unique: bool,
        _mode: u8,
    ) -> bool {
        if !self.owns(umod) {
            return false;
        }
        let m = h.stat(unit, MARK);
        h.set_stat(unit, MARK, m + 1);
        true
    }
}

impl ModRules for Umods {
    fn name(&self) -> &str {
        "umods"
    }
    fn monsters(&self) -> Option<&dyn MonsterProvider> {
        Some(self)
    }
}

fn install(f: &mut Fake, replace: bool) {
    f.rules = Ruleset::Mod(Arc::new(Umods { replace }));
}

fn event_types(f: &Fake, u: UnitId) -> Vec<u8> {
    let t = &f.game.timers;
    let mut v: Vec<u8> = t
        .unit_timers(u)
        .into_iter()
        .filter_map(|id| t.event(id).map(|e| e.0))
        .collect();
    v.sort_unstable();
    v
}

#[test]
fn original_rules_ignore_umods_past_the_table() {
    let mut f = fake(vec![velocity_mon(0, 1)]);
    let cx = f.cx;
    let u = f.monster(0, 1);
    f.set_stat(u, stat::LEVEL, 5);
    run_umod_init(&cx, &mut f, u, NEW_UMOD, true);
    f.store.entry(u).umods = [NEW_UMOD, 0, 0, 0, 0, 0, 0, 0, 0];
    dispatch(&cx, &mut f, u, None, 2);
    assert_eq!((f.s(u, stat::LEVEL), f.s(u, MARK)), (5, 0));
}

#[test]
fn a_mod_adds_umods_past_the_table() {
    let mut f = fake(vec![velocity_mon(0, 1)]);
    install(&mut f, false);
    let cx = f.cx;
    let u = f.monster(0, 1);
    f.set_stat(u, stat::LEVEL, 5);
    run_umod_init(&cx, &mut f, u, NEW_UMOD, true);
    assert_eq!(f.s(u, stat::LEVEL), 12);
    // Its callback runs for each list slot holding it, for any mode,
    // with D2RUST's callbacks for the other umods.
    f.store.entry(u).umods = [NEW_UMOD, EVENT7_UMOD, NEW_UMOD, 0, 0, 0, 0, 0, 0];
    handle_event7(&cx, &mut f, u);
    assert_eq!(f.s(u, MARK), 2);
    assert_eq!(event_types(&f, u), [2, 7]);
    dispatch(&cx, &mut f, u, None, 1);
    assert_eq!(f.s(u, MARK), 4);
    assert!(f.store.unhandled.is_empty());
}

#[test]
fn a_mod_umod_replaces_d2rusts() {
    for (replace, level, exp) in [(false, 8, 50), (true, 12, 10)] {
        let mut f = fake(vec![velocity_mon(0, 1)]);
        install(&mut f, replace);
        let cx = f.cx;
        let u = f.monster(0, 1);
        f.set_stat(u, stat::LEVEL, 5);
        f.set_stat(u, stat::EXPERIENCE, 10);
        run_umod_init(&cx, &mut f, u, LEVELADD, false);
        assert_eq!(
            (f.s(u, stat::LEVEL), f.s(u, stat::EXPERIENCE)),
            (level, exp),
            "replace {replace}"
        );
        // The type-7 callback: D2RUST's restarts the think and schedules
        // the next event; the mod's only marks.
        f.store.entry(u).umods = [EVENT7_UMOD, 0, 0, 0, 0, 0, 0, 0, 0];
        handle_event7(&cx, &mut f, u);
        let (types, mark): (&[u8], i32) = if replace { (&[], 1) } else { (&[2, 7], 0) };
        assert_eq!(event_types(&f, u), types, "replace {replace}");
        assert_eq!(f.s(u, MARK), mark);
    }
}

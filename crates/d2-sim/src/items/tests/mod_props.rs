//! A mod game's property functions (`docs/plans/mods/seams.md` MS7a):
//! asked first for the functions they answer, also past D2RUST's ids;
//! `None` and the functions they don't answer run D2RUST's own.

use std::sync::Arc;

use super::*;
use crate::items::props::{apply_property, roll_value, PropCtx};
use crate::items::provider::{ItemProvider, PropCall, PropHost};
use crate::rules::{ModRules, Ruleset};

/// A property function id D2RUST has none for.
const MOD_FN: u8 = 40;
const STAT_A: u16 = 7;
const STAT_B: u16 = 9;

/// Function 40 adds twice a roll of `min..=max`; function 1 with `param`
/// 7 adds 1000 and otherwise falls through to D2RUST's.
#[derive(Debug)]
struct Funcs;

impl ItemProvider for Funcs {
    fn has_property_fn(&self, func: u8) -> bool {
        matches!(func, MOD_FN | 1)
    }
    fn property_fn(&self, host: &mut dyn PropHost, call: &PropCall) -> Option<i32> {
        match call.func {
            MOD_FN => {
                let v = host.roll(call.rec.min, call.rec.max) * 2;
                Some(host.add_stat(call.set, call.stat, 0, v))
            }
            1 if call.rec.param == 7 => Some(host.add_stat(false, call.stat, 0, 1000)),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct Game(Option<Funcs>);

impl ModRules for Game {
    fn name(&self) -> &str {
        "test"
    }
    fn items(&self) -> Option<&dyn ItemProvider> {
        self.0.as_ref().map(|f| f as &dyn ItemProvider)
    }
}

fn row(slots: &[(u8, u16)]) -> PropertyRec {
    let mut p = PropertyRec::default();
    for (k, &(func, stat)) in slots.iter().enumerate() {
        p.slots[k] = PropSlot {
            func,
            stat,
            set: 0,
            val: 0,
        };
    }
    p
}

fn tables_with(rules: Ruleset, slots: &[(u8, u16)]) -> (ItemTables, usize) {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(ty::STAF, b"sst "));
    t.properties = vec![row(slots)];
    t.rules = rules;
    (t, i)
}

fn mod_rules(funcs: bool) -> Ruleset {
    Ruleset::Mod(Arc::new(Game(funcs.then_some(Funcs))))
}

fn run(t: &ItemTables, i: usize, r: PropRec) -> Item<FakeStats> {
    let mut it = item(i, 4242);
    apply_property(t, &mut it, &mut PropCtx::item(0), &r);
    it
}

#[test]
fn unknown_function_ends_the_row_under_original() {
    let (t, i) = tables_with(Ruleset::Original, &[(MOD_FN, STAT_A), (1, STAT_B)]);
    let it = run(&t, i, rec(0, 0, 3, 9));
    assert_eq!(it.stats.item_list(STAT_A, 0), 0);
    assert_eq!(it.stats.item_list(STAT_B, 0), 0);
    assert_eq!(it.item_seed, Seed::init_low(4242));
}

#[test]
fn mod_function_past_the_table_runs() {
    let (t, i) = tables_with(mod_rules(true), &[(MOD_FN, STAT_A)]);
    let it = run(&t, i, rec(0, 0, 3, 9));
    let mut s = Seed::init_low(4242);
    let v = roll_value(&mut s, 3, 9) * 2;
    assert_eq!(it.stats.item_list(STAT_A, 0), v);
    assert_eq!(it.item_seed, s);
}

#[test]
fn slot_0_result_is_prev_for_later_slots() {
    // Function 3 takes `prev` when it is non-zero instead of rolling.
    let (t, i) = tables_with(mod_rules(true), &[(MOD_FN, STAT_A), (3, STAT_B)]);
    let it = run(&t, i, rec(0, 0, 3, 9));
    let a = it.stats.item_list(STAT_A, 0);
    assert_ne!(a, 0);
    assert_eq!(it.stats.item_list(STAT_B, 0), a);
}

#[test]
fn mod_overrides_a_d2rust_function_or_falls_through() {
    let (t, i) = tables_with(mod_rules(true), &[(1, STAT_A)]);
    let it = run(&t, i, rec(0, 7, 3, 9));
    assert_eq!(it.stats.item_list(STAT_A, 0), 1000);
    assert_eq!(it.item_seed, Seed::init_low(4242));

    let (o, _) = tables_with(Ruleset::Original, &[(1, STAT_A)]);
    let own = run(&o, i, rec(0, 0, 3, 9));
    let fell = run(&t, i, rec(0, 0, 3, 9));
    assert_eq!(
        fell.stats.item_list(STAT_A, 0),
        own.stats.item_list(STAT_A, 0)
    );
    assert_ne!(own.stats.item_list(STAT_A, 0), 0);
    assert_eq!(fell.item_seed, own.item_seed);
}

#[test]
fn mod_without_item_provider_is_d2rusts() {
    let slots = [(1, STAT_A), (MOD_FN, STAT_B)];
    let (o, i) = tables_with(Ruleset::Original, &slots);
    let (m, _) = tables_with(mod_rules(false), &slots);
    let a = run(&o, i, rec(0, 0, 3, 9));
    let b = run(&m, i, rec(0, 0, 3, 9));
    assert_eq!(a.stats.item_list(STAT_A, 0), b.stats.item_list(STAT_A, 0));
    assert_eq!(b.stats.item_list(STAT_B, 0), 0);
    assert_eq!(a.item_seed, b.item_seed);
}

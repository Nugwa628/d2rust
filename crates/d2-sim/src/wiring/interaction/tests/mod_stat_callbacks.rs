//! Stat callbacks through real game rules and real stat-list writes.

use std::sync::{Arc, Mutex};

use crate::items::provider::ItemProvider;
use crate::rules::{ModRules, Ruleset};
use crate::stats::lists::NoHost;
use crate::stats::{key_stat, CallbackEvent, StatHost, StatLists, ValueCallback};
use crate::units::{UnitId, UnitType};

use super::skill_use::{skills, Fx};

const MAX_LIFE: u16 = 7;
const LIFE: u16 = 6;
const CUSTOM: u16 = 350;

#[derive(Debug)]
struct Callbacks {
    stat: u16,
    answers: bool,
    handles: bool,
    seen: Mutex<Vec<CallbackEvent>>,
}

impl ItemProvider for Callbacks {
    fn has_stat_callback(&self, stat: u16) -> bool {
        self.answers && stat == self.stat
    }

    fn stat_callback(
        &self,
        lists: &mut StatLists,
        host: &mut dyn StatHost,
        ev: &CallbackEvent,
    ) -> Option<()> {
        self.seen.lock().unwrap().push(*ev);
        if !self.handles {
            return None;
        }
        assert_eq!(
            lists.unit_total(ev.owner, key_stat(ev.key), ev.key as u16),
            ev.new
        );
        lists.unit_set(host, ev.owner, LIFE, ev.new / 4, 0);
        Some(())
    }
}

impl ModRules for Callbacks {
    fn name(&self) -> &str {
        "stat-callback-tests"
    }
    fn items(&self) -> Option<&dyn ItemProvider> {
        Some(self)
    }
}

#[derive(Debug)]
struct NoItems;
impl ModRules for NoItems {
    fn name(&self) -> &str {
        "no-item-hooks"
    }
}

fn provider(stat: u16, answers: bool, handles: bool) -> Arc<Callbacks> {
    Arc::new(Callbacks {
        stat,
        answers,
        handles,
        seen: Mutex::new(Vec::new()),
    })
}

// Changing maximum life normally preserves the current-life fraction.
fn rescale(rules: Ruleset) -> (i32, i32) {
    let mut fx = Fx::with(crate::stats::tests::data(), skills());
    let p = fx.spawn(UnitType::Player, 10, 10);
    fx.sim
        .sys
        .stats
        .unit_set(&mut fx.sim.sys.hooks, p, MAX_LIFE, 100 << 8, 0);
    fx.sim
        .sys
        .stats
        .unit_set(&mut fx.sim.sys.hooks, p, LIFE, 50 << 8, 0);
    fx.sim.sys.hooks.rules = rules;
    fx.sim
        .sys
        .stats
        .unit_set(&mut fx.sim.sys.hooks, p, MAX_LIFE, 200 << 8, 0);
    (
        fx.sim.sys.stats.unit_total(p, MAX_LIFE, 0),
        fx.sim.sys.stats.unit_total(p, LIFE, 0),
    )
}

#[test]
fn hook_changes_the_stat_result_and_replaces_rescaling() {
    let hook = provider(MAX_LIFE, true, true);
    assert_eq!(rescale(Ruleset::Mod(hook.clone())), (200 << 8, 50 << 8));
    let seen = hook.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!((seen[0].old, seen[0].new), (100 << 8, 200 << 8));
    assert_eq!(key_stat(seen[0].key), MAX_LIFE);
}

#[test]
fn no_hook_unanswered_stat_and_none_preserve_d2rust_results() {
    let expected = rescale(Ruleset::Original);
    assert_eq!(expected, (200 << 8, 100 << 8));
    assert_eq!(rescale(Ruleset::Mod(Arc::new(NoItems))), expected);
    let ignored = provider(MAX_LIFE, false, true);
    assert_eq!(rescale(Ruleset::Mod(ignored.clone())), expected);
    assert!(ignored.seen.lock().unwrap().is_empty());
    let fallback = provider(MAX_LIFE, true, false);
    assert_eq!(rescale(Ruleset::Mod(fallback.clone())), expected);
    assert_eq!(fallback.seen.lock().unwrap().len(), 1);
}

#[test]
fn added_callback_receives_layer_and_attachment_then_removal_in_order() {
    let mut data = (*crate::stats::tests::data()).clone();
    let table = crate::stats::tests::itemstatcost_with(|rows| {
        use d2_data::tables::{Itemstatcost, Record};
        rows[usize::from(CUSTOM) * Itemstatcost::SIZE + 5] |= 0x08;
    });
    data.stats = crate::stats::StatTable::from_fixed(&table).unwrap();
    let mut fx = Fx::with(Arc::new(data), skills());
    let p = fx.spawn(UnitType::Player, 10, 10);
    let hook = provider(CUSTOM, true, true);
    fx.sim.sys.hooks.rules = Ruleset::Mod(hook.clone());
    let lists = &mut fx.sim.sys.stats;
    let host = &mut fx.sim.sys.hooks;
    let item_unit = UnitId(555);
    let item = lists.alloc_extended(&mut NoHost, item_unit, UnitType::Item, 55, 0, 0, None);
    lists.set(&mut NoHost, item, CUSTOM, 80, 13, None);
    lists.attach(host, p, item, true);
    assert_eq!(lists.unit_total(p, LIFE, 0), 20);
    lists.detach(host, item);
    assert_eq!(lists.unit_total(p, LIFE, 0), 0);
    let seen = hook.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(
        (seen[0].old, seen[0].new, seen[1].old, seen[1].new),
        (0, 80, 80, 0)
    );
    for ev in seen.iter() {
        assert_eq!(ev.owner, p);
        assert_eq!(ev.unit, Some(item_unit));
        assert_eq!(ev.list, lists.unit_list(p).unwrap());
        assert_eq!(ev.key as u16, 13);
        assert_eq!(key_stat(ev.key), CUSTOM);
    }
}

#[test]
fn callback_requires_the_row_flag_and_a_list_callback() {
    let mut fx = Fx::with(crate::stats::tests::data(), skills());
    let p = fx.spawn(UnitType::Player, 10, 10);
    let hook = provider(CUSTOM, true, true);
    fx.sim.sys.hooks.rules = Ruleset::Mod(hook.clone());
    fx.sim
        .sys
        .stats
        .unit_set(&mut fx.sim.sys.hooks, p, CUSTOM, 80, 13);
    assert!(hook.seen.lock().unwrap().is_empty());

    let hook = provider(MAX_LIFE, true, true);
    fx.sim.sys.hooks.rules = Ruleset::Mod(hook.clone());
    fx.sim
        .sys
        .stats
        .alloc_extended(&mut fx.sim.sys.hooks, p, UnitType::Player, 1, 0, 0, None);
    fx.sim
        .sys
        .stats
        .unit_set(&mut fx.sim.sys.hooks, p, MAX_LIFE, 100 << 8, 0);
    assert!(hook.seen.lock().unwrap().is_empty());
    // Enabling the list callback makes the same row eligible.
    fx.sim.sys.stats.alloc_extended(
        &mut fx.sim.sys.hooks,
        p,
        UnitType::Player,
        1,
        0,
        0,
        Some(ValueCallback::Server),
    );
    fx.sim
        .sys
        .stats
        .unit_set(&mut fx.sim.sys.hooks, p, MAX_LIFE, 100 << 8, 0);
    assert_eq!(hook.seen.lock().unwrap().len(), 1);
}

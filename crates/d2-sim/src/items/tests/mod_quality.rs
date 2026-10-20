//! A mod game's quality routines (`docs/plans/mods/seams.md` MS7d):
//! asked first for the qualities its item provider answers, also past
//! D2RUST's; `None` runs D2RUST's routine.

use std::sync::Arc;

use super::*;
use crate::items::provider::{GenHost, ItemProvider};
use crate::items::quality::dispatch;
use crate::items::{q, req, ItemRequest};
use crate::rules::{ModRules, Ruleset};

/// A quality D2RUST has none for.
const MOD_QUALITY: u8 = 12;
/// The stat the mod's routine writes through property function 1.
const STAT_A: u16 = 7;

/// Quality 12 takes file index 3 and applies properties row 0 (function 1
/// on stat 7, 5..=9); magic answers `magic`.
#[derive(Debug)]
struct Gen {
    magic: Option<bool>,
}

impl ItemProvider for Gen {
    fn has_quality_fn(&self, quality: u8) -> bool {
        matches!(quality, MOD_QUALITY | q::MAGIC)
    }
    fn quality_fn(&self, host: &mut dyn GenHost, quality: u8, _: &ItemRequest) -> Option<bool> {
        if quality == q::MAGIC {
            return self.magic;
        }
        host.set_file_index(3);
        host.apply_property(0, &rec(0, 0, 5, 9));
        Some(true)
    }
}

#[derive(Debug)]
struct Game(Gen);

impl ModRules for Game {
    fn name(&self) -> &str {
        "gen-test"
    }
    fn items(&self) -> Option<&dyn ItemProvider> {
        Some(&self.0)
    }
}

/// A ring and properties row 0 (function 1 on stat 7), under `rules`.
fn tables_with(rules: Ruleset) -> (ItemTables, usize) {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.properties = vec![prop1(1, STAT_A)];
    t.rules = rules;
    (t, i)
}

fn mod_rules(magic: Option<bool>) -> Ruleset {
    Ruleset::Mod(Arc::new(Game(Gen { magic })))
}

/// Runs the quality dispatch for a new ring requesting `quality`.
fn run(t: &ItemTables, i: usize, quality: u8) -> (bool, Item<FakeStats>) {
    let mut it = item(i, 0xDEAD_BEEF);
    let mut rq = ItemRequest {
        quality,
        flags2: req::NEVER_ETHEREAL,
        ..Default::default()
    };
    let made = dispatch(t, &mut FakeGame::default(), &mut it, &mut rq).unwrap();
    (made, it)
}

#[test]
fn a_quality_past_d2rusts_makes_no_item_under_original() {
    let (t, i) = tables_with(Ruleset::Original);
    assert!(!run(&t, i, MOD_QUALITY).0);
}

#[test]
fn a_mod_quality_runs_its_routine() {
    let (t, i) = tables_with(mod_rules(None));
    let (made, it) = run(&t, i, MOD_QUALITY);
    assert!(made);
    assert_eq!(it.quality, MOD_QUALITY);
    assert_eq!(it.file_index, 3);
    assert!((5..=9).contains(&it.stats.item_list(STAT_A, 0)));
}

/// A mod's refusal of magic makes no item; its `None` runs D2RUST's magic
/// routine (no affix rows: down to normal, as under D2RUST's rules).
#[test]
fn a_mod_overrides_magic_or_falls_through() {
    let (t, i) = tables_with(mod_rules(Some(false)));
    assert!(!run(&t, i, q::MAGIC).0);

    let (t, i) = tables_with(mod_rules(None));
    let (o, _) = tables_with(Ruleset::Original);
    let (made, it) = run(&t, i, q::MAGIC);
    let (own_made, own) = run(&o, i, q::MAGIC);
    assert!(made && own_made);
    assert_eq!(it.quality, q::NORMAL);
    assert_eq!(it.quality, own.quality);
    assert_eq!(it.item_seed, own.item_seed);
}

//! A mod game's stores (`docs/plans/mods/seams.md` MS8e), with a synthetic
//! provider: class 527 is the mod's NPC (it sells from column 11, gambles,
//! hires and repairs), Akara repairs, Charsi no longer repairs and keeps
//! her store, and class 601 names a column past the item tables' columns.
//! Every other class stays D2RUST's.

use std::sync::Arc;

use super::store::{open, StoreCtx};
use super::tests::{index, npc, p, tables, Fake, NPC};
use super::trade::{repair, RepairMsg};
use super::*;
use crate::rng::Seed;
use crate::rules::{ModRules, NpcProvider, Ruleset, StoreProvider};
use crate::world::npc::NpcRoles;

/// The mod's NPC class.
const MOD_NPC: u16 = 527;
/// A mod class whose column is past the item tables' columns.
const BAD_COLUMN: u16 = 601;

#[derive(Debug)]
struct TestStores;

impl StoreProvider for TestStores {
    fn roles(&self, class: u16) -> Option<StoreRoles> {
        match class {
            MOD_NPC => Some(StoreRoles {
                column: Some(11),
                global_column: Some(11),
                flagged: true,
                no_refresh: false,
                repairs: true,
            }),
            class::AKARA => Some(StoreRoles {
                repairs: true,
                ..StoreRoles::d2rust(class)
            }),
            class::CHARSI => Some(StoreRoles {
                repairs: false,
                no_refresh: true,
                ..StoreRoles::d2rust(class)
            }),
            BAD_COLUMN => Some(StoreRoles {
                column: Some(COLUMNS + 23),
                global_column: Some(COLUMNS),
                ..StoreRoles::default()
            }),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct TestNpcs;

impl NpcProvider for TestNpcs {
    fn roles(&self, class: u16) -> Option<NpcRoles> {
        (class == MOD_NPC).then_some(NpcRoles {
            trades: true,
            gambles: true,
            hires: true,
            ..NpcRoles::default()
        })
    }
}

#[derive(Debug)]
struct TestRules(TestStores, TestNpcs);

impl ModRules for TestRules {
    fn name(&self) -> &str {
        "store test mod"
    }
    fn stores(&self) -> Option<&dyn StoreProvider> {
        Some(&self.0)
    }
    fn npcs(&self) -> Option<&dyn NpcProvider> {
        Some(&self.1)
    }
}

fn modded() -> Ruleset {
    Ruleset::Mod(Arc::new(TestRules(TestStores, TestNpcs)))
}

/// The synthetic tables with one item in column 11 and the mod's NPC
/// classes in `interact`.
fn mod_tables() -> VendorTables {
    let mut t = tables();
    let skc = index(&t, "skc");
    t.items[skc].columns[11] = [1, 1, 0, 0, 0];
    t.interact.push(MOD_NPC);
    t.interact.push(BAD_COLUMN);
    t
}

#[test]
fn a_mod_npc_gets_a_store_column() {
    let t = mod_tables();
    // D2RUST: column 11 is never built and class 527 has no store.
    let g = GlobalLists::build(&t);
    assert!(g.columns[11].items.is_empty());
    let r = VendorRecord::new(MOD_NPC, 0, true, &g);
    assert!(r.items.is_empty() && !r.has_gamble && !r.flag24);
    // The mod: column 11 is built and copied, with the mod's flags.
    let rules = modded();
    let g = GlobalLists::build_for(&t, &rules);
    assert_eq!(g.columns.len(), COLUMNS);
    assert_eq!(g.columns[11].items.len(), 1);
    let r = VendorRecord::new_for(&rules, MOD_NPC, 0, true, &g);
    assert_eq!(r.items, g.columns[11].items);
    assert!(r.has_gamble && r.flag24 && r.flag25);
    // A column past the item tables' columns is dropped.
    assert_eq!(StoreRoles::of(&rules, BAD_COLUMN).column, None);
    let r = VendorRecord::new_for(&rules, BAD_COLUMN, 0, true, &g);
    assert!(r.items.is_empty());
    // D2RUST's classes keep D2RUST's columns.
    let gh = VendorRecord::new_for(&rules, class::GHEED, 0, true, &g);
    assert_eq!(gh, VendorRecord::new(class::GHEED, 0, true, &g));
}

fn trade_open(w: &mut Fake, rec: &mut VendorRecord, t: &VendorTables) {
    let mut seed = Seed::new(1, 666);
    let mut c = StoreCtx {
        tables: t,
        seed: &mut seed,
    };
    open(&mut c, rec, w, npc(), p(), true, false, 0).unwrap();
}

#[test]
fn a_mod_npc_makes_its_hire_list() {
    let t = mod_tables();
    for (rules, hired) in [(Ruleset::Original, false), (modded(), true)] {
        let g = GlobalLists::build_for(&t, &rules);
        let mut rec = VendorRecord::new_for(&rules, MOD_NPC, 0, true, &g);
        let mut w = Fake::new(MOD_NPC);
        w.rules = rules;
        trade_open(&mut w, &mut rec, &t);
        let made = w.log.iter().any(|l| *l == format!("hire list {MOD_NPC}"));
        assert_eq!(made, hired);
        assert_eq!(w.hire_made, vec![MOD_NPC]);
    }
}

#[test]
fn a_mod_may_keep_a_store_from_refreshing() {
    let t = mod_tables();
    for (rules, refreshed) in [(Ruleset::Original, true), (modded(), false)] {
        let g = GlobalLists::build_for(&t, &rules);
        let mut rec = VendorRecord::new_for(&rules, class::CHARSI, 0, true, &g);
        rec.store_generated = true;
        rec.refresh_pending = true;
        let mut w = Fake::new(class::CHARSI);
        w.rules = rules;
        trade_open(&mut w, &mut rec, &t);
        assert_eq!(!w.new_inventories.is_empty(), refreshed);
        assert_eq!(rec.refresh_pending, !refreshed);
    }
}

#[test]
fn the_mod_decides_who_repairs() {
    let t = tables();
    // Item 0x20 does not exist: a repairer answers "not in the inventory"
    // (3), anyone else "not a repairer" (1).
    let m = RepairMsg {
        npc: NPC,
        item: 0x20,
        all: false,
    };
    for (class, original, modded_result) in [
        (class::CHARSI, 3, 1),
        (class::AKARA, 1, 3),
        (MOD_NPC, 1, 3),
        (class::FARA, 3, 3),
    ] {
        let mut w = Fake::new(class);
        assert_eq!(repair(&t, &mut w, p(), &m), Ok(original), "{class}");
        let mut w = Fake::new(class);
        w.rules = modded();
        assert_eq!(repair(&t, &mut w, p(), &m), Ok(modded_result), "{class}");
    }
}

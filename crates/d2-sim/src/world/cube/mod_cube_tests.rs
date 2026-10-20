//! A mod game's cube ops and outputs (`docs/plans/mods/seams.md` MS8a): a
//! synthetic provider on the fake world. Its op codes and output kinds run
//! first; everything it leaves alone runs D2RUST's code.

use std::sync::Arc;

use super::tests::{code_in, data, gem_recipe, input, out, recipe, Fake, GCV, GFV, LEG, P, TBK};
use super::*;
use crate::rules::{ModRules, Ruleset};

/// A recipe op on the player: passes when stat `param` is at least `value`.
const OP_PLAYER_STAT: u8 = 200;
/// A slot-0 op: passes when the candidate's file index is `value`.
const OP_FILE_INDEX: u8 = 201;
/// An every-candidate op: passes for magic or better candidates.
const OP_MAGIC_INPUTS: u8 = 202;
/// An output kind D2RUST doesn't have: makes item `item` of the slot.
const KIND_MAKE: u8 = 0x10;
/// An output kind D2RUST doesn't have: an event that succeeds.
const KIND_EVENT: u8 = 0x11;
/// An output kind D2RUST doesn't have: answered with nothing.
const KIND_NOTHING: u8 = 0x12;

#[derive(Debug)]
struct TestCube;

impl CubeProvider for TestCube {
    fn op_scope(&self, op: u8) -> Option<OpScope> {
        match op {
            OP_PLAYER_STAT => Some(OpScope::Recipe),
            OP_FILE_INDEX => Some(OpScope::Input0),
            OP_MAGIC_INPUTS => Some(OpScope::Inputs),
            _ => None,
        }
    }
    fn op_test(&self, w: &dyn CubeWorld, recipe: &Recipe, unit: UnitId) -> bool {
        match recipe.op {
            OP_PLAYER_STAT => w.stat(unit, StatRead::Value, recipe.param as u16) >= recipe.value,
            OP_FILE_INDEX => w.item_file_index(unit) as i32 == recipe.value,
            OP_MAGIC_INPUTS => w.item_quality(unit) >= 4,
            _ => unreachable!("only scoped ops are tested"),
        }
    }
    fn output(
        &self,
        w: &mut dyn CubeWorld,
        player: UnitId,
        recipe: &Recipe,
        slot: usize,
        _capture: &Capture,
        level: i32,
    ) -> Option<CubeOutput> {
        let o = recipe.outputs[slot];
        match o.kind {
            KIND_MAKE => {
                let request = ItemRequest {
                    player: Some(player),
                    level,
                    class: u32::from(o.item),
                    ..ItemRequest::default()
                };
                Some(
                    w.create_item(&request)
                        .map_or(CubeOutput::Nothing, CubeOutput::Item),
                )
            }
            KIND_EVENT => Some(CubeOutput::Event(true)),
            KIND_NOTHING => Some(CubeOutput::Nothing),
            // The mod opens D2RUST's Pandemonium portal kind.
            kind::PANDEMONIUM => Some(CubeOutput::Event(true)),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct TestRules(TestCube);

impl ModRules for TestRules {
    fn name(&self) -> &str {
        "test-cube"
    }
    fn cube(&self) -> Option<&dyn CubeProvider> {
        Some(&self.0)
    }
}

fn modded() -> Fake {
    Fake {
        rules: Ruleset::Mod(Arc::new(TestRules(TestCube))),
        ..Fake::new()
    }
}

/// Three chipped gems with the recipe's op set to `op`.
fn gems_with_op(op: u8, param: i32, value: i32) -> CubeData {
    let mut r = gem_recipe();
    r.op = op;
    r.param = param;
    r.value = value;
    data(vec![r])
}

fn three_gems(f: &mut Fake, quality: u8) -> Vec<UnitId> {
    (0..3).map(|_| f.add(GCV, quality)).collect()
}

#[test]
fn recipe_scope_op_tests_the_player() {
    let d = gems_with_op(OP_PLAYER_STAT, 12, 30);
    let mut f = modded();
    three_gems(&mut f, 2);
    f.player_stats.insert(12, 29);
    assert_eq!(d.transmute(&mut f, P).record, None);
    let mut f = modded();
    three_gems(&mut f, 2);
    f.player_stats.insert(12, 30);
    assert!(d.transmute(&mut f, P).committed);
}

#[test]
fn unknown_op_passes_under_d2rust_rules() {
    // D2RUST doesn't know code 200: it passes, whatever the player's stat.
    let d = gems_with_op(OP_PLAYER_STAT, 12, 30);
    let mut f = Fake::new();
    three_gems(&mut f, 2);
    assert!(d.transmute(&mut f, P).committed);
}

#[test]
fn input0_scope_op_tests_the_slot0_candidate() {
    let d = gems_with_op(OP_FILE_INDEX, 0, 5);
    let mut f = modded();
    let gems = three_gems(&mut f, 2);
    // Two of three pass: slot 0 needs all three.
    f.it(gems[0]).file_index = 5;
    f.it(gems[1]).file_index = 5;
    assert_eq!(d.transmute(&mut f, P).record, None);
    f.it(gems[2]).file_index = 5;
    assert!(d.transmute(&mut f, P).committed);
}

#[test]
fn inputs_scope_op_tests_every_candidate() {
    let mut r = recipe(2, &[code_in(LEG), code_in(TBK)], out(kind::ITEMCODE, GFV));
    r.op = OP_MAGIC_INPUTS;
    let d = data(vec![r]);
    let mut f = modded();
    f.add(LEG, 4);
    f.add(TBK, 2);
    assert_eq!(d.transmute(&mut f, P).record, None);
    let mut f = modded();
    f.add(LEG, 4);
    f.add(TBK, 5);
    assert!(d.transmute(&mut f, P).committed);
}

#[test]
fn ops_the_mod_leaves_run_d2rusts_tests() {
    // Op 1 (day of month in param..=value) is D2RUST's under a mod too.
    let d = gems_with_op(1, 20, 25);
    let mut f = modded();
    three_gems(&mut f, 2);
    f.date = (15, 3);
    assert_eq!(d.transmute(&mut f, P).record, None);
    f.date = (21, 3);
    assert!(d.transmute(&mut f, P).committed);
}

#[test]
fn mod_output_item_is_finished_and_placed() {
    let mut a = out(KIND_MAKE, GFV);
    a.flags = output_flags::RCH;
    let d = data(vec![recipe(3, &[input(1, GCV, 3)], a)]);
    let mut f = modded();
    three_gems(&mut f, 2);
    let t = d.transmute(&mut f, P);
    assert!(t.committed);
    assert_eq!(t.outputs.len(), 1);
    let x = t.outputs[0];
    assert_eq!(f.items[&x].class, Some(GFV));
    // D2RUST's finishing ran (recharge), then the commit placed it.
    assert!(f.log.contains(&"recharge".to_string()));
    assert!(f.log.contains(&format!("place {}", x.0)));
    assert_eq!(f.cube_contents(), vec![Some(GFV)]);
}

#[test]
fn mod_output_event_and_nothing() {
    let d = data(vec![recipe(
        2,
        &[code_in(LEG), code_in(TBK)],
        out(KIND_EVENT, 0),
    )]);
    let mut f = modded();
    f.add(LEG, 2);
    f.add(TBK, 2);
    let t = d.transmute(&mut f, P);
    assert!(t.committed && t.outputs.is_empty());
    assert!(f.cube_contents().is_empty());

    let d = data(vec![recipe(
        2,
        &[code_in(LEG), code_in(TBK)],
        out(KIND_NOTHING, 0),
    )]);
    let mut f = modded();
    f.add(LEG, 2);
    f.add(TBK, 2);
    let t = d.transmute(&mut f, P);
    assert_eq!((t.record, t.committed), (Some(0), false));
    assert_eq!(f.cube_contents().len(), 2);
}

#[test]
fn mod_takes_over_a_d2rust_kind() {
    let d = data(vec![recipe(
        2,
        &[code_in(LEG), code_in(TBK)],
        out(kind::PANDEMONIUM, 0),
    )]);
    // D2RUST's Pandemonium kind fails ...
    let mut f = Fake::new();
    f.add(LEG, 2);
    f.add(TBK, 2);
    assert!(!d.transmute(&mut f, P).committed);
    // ... the mod's succeeds.
    let mut f = modded();
    f.add(LEG, 2);
    f.add(TBK, 2);
    assert!(d.transmute(&mut f, P).committed);
}

#[test]
fn kinds_the_mod_leaves_run_d2rusts_outputs() {
    let d = data(vec![gem_recipe()]);
    let mut plain = Fake::new();
    three_gems(&mut plain, 2);
    let a = d.transmute(&mut plain, P);
    let mut f = modded();
    three_gems(&mut f, 2);
    let b = d.transmute(&mut f, P);
    assert_eq!((a.record, a.committed), (b.record, b.committed));
    assert_eq!(plain.log, f.log);
    assert_eq!(plain.requests, f.requests);
}

//! A mod game's operate functions (`docs/plans/mods/seams.md` MS8b): a
//! synthetic provider on the object fake. The numbers it claims run first;
//! everything it leaves alone runs D2RUST's code.

use std::sync::Arc;

use super::fake::*;
use super::*;
use crate::rules::{ModRules, Ruleset};

const O: UnitId = UnitId(1);
const P: UnitId = UnitId(2);
/// A torch row (D2RUST's operate 11).
const TORCH: u16 = 100;
/// A torch row the mod takes over.
const MOD_TORCH: u16 = 101;
/// A refused class with a claimed function.
const REFUSED: u16 = REFUSED_CLASSES[0];

/// The mod's own slot past D2RUST's bound: stores mode 3, returns 7.
const FN_OWN: u8 = 150;
/// Claimed past the bound, answered with `None`.
const FN_DECLINED: u8 = 151;
/// Past the bound and not claimed.
const FN_UNCLAIMED: u8 = 152;
/// Claimed past the bound, a fatal error.
const FN_FATAL: u8 = 153;
/// D2RUST's torch.
const FN_TORCH: u8 = 11;
/// A quest function of D2RUST's the mod runs itself.
const FN_QUEST: u8 = 6;
/// A quest function the mod leaves alone.
const FN_OTHER_QUEST: u8 = 9;

#[derive(Debug)]
struct TestObjects;

impl ObjectProvider for TestObjects {
    fn has_operate(&self, n: u8) -> bool {
        matches!(n, FN_OWN | FN_DECLINED | FN_FATAL | FN_TORCH | FN_QUEST)
    }
    fn operate(
        &self,
        _ctl: &mut ObjectControl,
        _t: &ObjectTables,
        w: &mut dyn ObjectHost,
        op: &Operate,
    ) -> Option<Result<i32, ObjectError>> {
        match op.operate_fn {
            FN_OWN => {
                w.store_mode(op.object, 3);
                Some(Ok(7))
            }
            FN_FATAL => Some(Err(ObjectError::NoData(op.object))),
            FN_TORCH if op.class == MOD_TORCH => Some(Ok(5)),
            FN_QUEST => Some(Ok(2)),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct TestRules(TestObjects);

impl ModRules for TestRules {
    fn name(&self) -> &str {
        "test-objects"
    }
    fn objects(&self) -> Option<&dyn ObjectProvider> {
        Some(&self.0)
    }
}

fn mod_rules() -> Ruleset {
    Ruleset::Mod(Arc::new(TestRules(TestObjects)))
}

/// Tables where row `class` has operate function `n`; the two torch rows
/// keep 11.
fn tables(class: u16, n: u8) -> ObjectTables {
    let mut objects: Vec<Objects> = (0..400).map(|_| blank_object()).collect();
    objects[TORCH as usize].operatefn = FN_TORCH;
    objects[MOD_TORCH as usize].operatefn = FN_TORCH;
    objects[class as usize].operatefn = n;
    ObjectTables {
        objects,
        ..ObjectTables::default()
    }
}

/// Object `O` of `class` in mode 0 and player `P`, under `rules`.
fn world(class: u16, rules: Ruleset, t: &ObjectTables) -> (ObjectControl, Fake) {
    let (mut ctl, _) = ObjectControl::new(&mut Seed::init(), t);
    ctl.data.insert(
        O,
        ObjectData {
            guid: 0x1234,
            class,
            ..ObjectData::default()
        },
    );
    let mut f = Fake {
        rules,
        ..Fake::default()
    };
    f.modes.insert(O, 0);
    f.guids.insert(O, 0x1234);
    f.operators.insert(P, Operator::Player(0));
    (ctl, f)
}

/// Dispatch row `class` with function `n` under `rules`; the result and
/// the object's mode after.
fn run(class: u16, n: u8, rules: Ruleset) -> (Result<Dispatch, ObjectError>, u8) {
    let t = tables(class, n);
    let (mut ctl, mut f) = world(class, rules, &t);
    let r = dispatch(&mut ctl, &t, &mut f, O, Some(P));
    (r, f.modes[&O])
}

fn op(class: u16, n: u8) -> Operate {
    Operate {
        object: O,
        operator: Some(P),
        class,
        operate_fn: n,
    }
}

#[test]
fn mod_slot_past_the_bound_runs() {
    let (r, mode) = run(200, FN_OWN, mod_rules());
    assert_eq!((r, mode), (Ok(Dispatch::Done(7)), 3));
    // The same row is fatal under D2RUST's rules.
    let (r, mode) = run(200, FN_OWN, Ruleset::Original);
    assert_eq!((r, mode), (Err(ObjectError::OperateFn(FN_OWN)), 0));
}

#[test]
fn unclaimed_numbers_past_the_bound_stay_fatal() {
    let (r, _) = run(200, FN_UNCLAIMED, mod_rules());
    assert_eq!(r, Err(ObjectError::OperateFn(FN_UNCLAIMED)));
}

#[test]
fn declined_slot_falls_through_to_nothing() {
    // Claimed past the bound but answered `None`: D2RUST has no function
    // there, so nothing runs and the dispatch returns 0.
    let (r, mode) = run(200, FN_DECLINED, mod_rules());
    assert_eq!((r, mode), (Ok(Dispatch::Done(0)), 0));
}

#[test]
fn mod_errors_come_back() {
    let (r, _) = run(200, FN_FATAL, mod_rules());
    assert_eq!(r, Err(ObjectError::NoData(O)));
}

#[test]
fn mod_takes_over_one_torch_and_leaves_the_other() {
    // The mod's torch row: its function, D2RUST's torch doesn't run.
    let (r, mode) = run(MOD_TORCH, FN_TORCH, mod_rules());
    assert_eq!((r, mode), (Ok(Dispatch::Done(5)), 0));
    // The plain torch row: the mod answers `None`, D2RUST's torch lights.
    let (r, mode) = run(TORCH, FN_TORCH, mod_rules());
    assert_eq!((r.clone(), mode), (Ok(Dispatch::Done(1)), 1));
    assert_eq!(run(TORCH, FN_TORCH, Ruleset::Original), (r, mode));
}

#[test]
fn mod_runs_a_quest_function_itself() {
    let (r, _) = run(200, FN_QUEST, mod_rules());
    assert_eq!(r, Ok(Dispatch::Done(2)));
    // Unclaimed quest numbers still go to the quest code.
    let (r, _) = run(200, FN_OTHER_QUEST, mod_rules());
    assert_eq!(r, Ok(Dispatch::Quest(op(200, FN_OTHER_QUEST))));
    let (r, _) = run(200, FN_QUEST, Ruleset::Original);
    assert_eq!(r, Ok(Dispatch::Quest(op(200, FN_QUEST))));
}

#[test]
fn d2rust_checks_run_before_the_mod() {
    // A busy player is refused before the mod's function.
    let t = tables(200, FN_OWN);
    let (mut ctl, mut f) = world(200, mod_rules(), &t);
    f.busy.insert(P);
    assert_eq!(
        dispatch(&mut ctl, &t, &mut f, O, Some(P)),
        Ok(Dispatch::Done(0))
    );
    assert_eq!(f.modes[&O], 0);
    assert!(f.calls.is_empty());
    // A refused class stays refused, claimed function or not.
    let (r, mode) = run(REFUSED, FN_OWN, mod_rules());
    assert_eq!((r, mode), (Ok(Dispatch::Done(0)), 0));
}

#[test]
fn operate_entry_reaches_the_mod() {
    let t = tables(200, FN_OWN);
    let (mut ctl, mut f) = world(200, mod_rules(), &t);
    assert_eq!(
        operate_in_range(&mut ctl, &t, &mut f, Some(P), 0x1234),
        Ok((1, Some(Dispatch::Done(7))))
    );
    assert_eq!(f.modes[&O], 3);
}

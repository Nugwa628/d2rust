//! A mod game's quest functions (`docs/plans/mods/seams.md` MS8d), with a
//! synthetic provider: chain 0 (Warriv's gossip) gets the mod's own text
//! and is never active, chain 3 reports quest-log state 7, chain 1's
//! sequence fails, and Flavie's records report their event 0 unhandled.
//! Every other chain stays D2RUST's.

use std::sync::Arc;

use super::tests::{control, Fake, NAVI_U, P1, WARRIV_U};
use super::*;
use crate::rules::{ModRules, QuestProvider, Ruleset};

/// The mod's gossip line for Warriv.
const MOD_LINE: (u16, u32) = (900, 0);

#[derive(Debug)]
struct TestQuests;

impl QuestProvider for TestQuests {
    fn callback(
        &self,
        ctl: &mut QuestControl,
        w: &mut dyn QuestWorld,
        i: usize,
        args: EventArgs,
        list: Option<&mut TextList>,
        _force: bool,
    ) -> Option<bool> {
        match (ctl.records[i].chain, args.event) {
            (0, event::NPC_ACTIVATE) => {
                if let (Some(list), Some(n)) = (list, args.target) {
                    if w.monster_class(n) == Some(npc::WARRIV1) {
                        list.push(MOD_LINE);
                    }
                }
                Some(true)
            }
            (25 | 30, event::NPC_ACTIVATE) => Some(false),
            _ => None,
        }
    }

    fn active(
        &self,
        ctl: &QuestControl,
        _w: &mut dyn QuestWorld,
        i: usize,
        _player: UnitId,
        _npc_class: u16,
    ) -> Option<bool> {
        (ctl.records[i].chain == 0).then_some(false)
    }

    fn status(
        &self,
        ctl: &QuestControl,
        _w: &mut dyn QuestWorld,
        i: usize,
        _player: UnitId,
        _pf: &QuestFlags,
    ) -> Option<Option<u8>> {
        (ctl.records[i].chain == 3).then_some(Some(7))
    }

    fn sequence(
        &self,
        _ctl: &mut QuestControl,
        _w: &mut dyn QuestWorld,
        chain: u8,
    ) -> Option<bool> {
        (chain == 1).then_some(false)
    }
}

#[derive(Debug)]
struct TestRules(TestQuests);

impl ModRules for TestRules {
    fn name(&self) -> &str {
        "quest test mod"
    }
    fn quests(&self) -> Option<&dyn QuestProvider> {
        Some(&self.0)
    }
}

fn modded() -> Fake {
    let mut f = Fake::new();
    f.rules = Ruleset::Mod(Arc::new(TestRules(TestQuests)));
    f
}

#[test]
fn the_mod_callback_runs_first() {
    let (mut ctl, _) = control();
    let mut o = Fake::new();
    let mut list = TextList::new();
    ctl.npc_activate(&mut o, P1, WARRIV_U, &mut list);
    assert!(list.contains(&(0, 0)) && !list.contains(&MOD_LINE));

    let (mut ctl, _) = control();
    let mut f = modded();
    let mut list = TextList::new();
    ctl.npc_activate(&mut f, P1, WARRIV_U, &mut list);
    assert!(list.contains(&MOD_LINE) && !list.contains(&(0, 0)));
    assert!(f.log.iter().all(|l| !l.starts_with("unhandled 0 ")));
}

#[test]
fn a_mod_callback_may_report_unhandled() {
    let (mut ctl, _) = control();
    let mut f = modded();
    let mut list = TextList::new();
    ctl.npc_activate(&mut f, P1, NAVI_U, &mut list);
    // Both Flavie records report their event 0 function; no Flavie text.
    assert!(f.log.iter().any(|l| l.starts_with("unhandled 25 ")));
    assert!(f.log.iter().any(|l| l.starts_with("unhandled 30 ")));
    assert!(list.is_empty());
    // Under `Original` Flavie talks and nothing is reported.
    let (mut ctl, _) = control();
    let mut o = Fake::new();
    let mut list = TextList::new();
    ctl.npc_activate(&mut o, P1, NAVI_U, &mut list);
    assert!(!list.is_empty());
    assert!(o.log.iter().all(|l| !l.starts_with("unhandled 25 ")));
}

#[test]
fn the_mod_active_function_runs_first() {
    for (mut f, wants) in [(Fake::new(), true), (modded(), false)] {
        let (mut ctl, _) = control();
        ctl.picked = true;
        assert_eq!(
            ctl.npc_wants_interact(&mut f, P1, WARRIV_U, npc::WARRIV1, true),
            Ok(wants)
        );
        assert_eq!(f.sent.is_empty(), !wants);
    }
}

#[test]
fn the_mod_status_function_runs_first() {
    let (ctl, _) = control();
    let pf = QuestFlags::default();
    let i3 = ctl.find(3).unwrap();
    let mut o = Fake::new();
    let original = act1::status_fn(&ctl, &mut o, i3, P1, &pf, 0);
    assert_ne!(original, Some(7));
    let mut f = modded();
    assert_eq!(act1::status_fn(&ctl, &mut f, i3, P1, &pf, 0), Some(7));
    // Chain 0 is left to D2RUST: no status.
    let i0 = ctl.find(0).unwrap();
    assert_eq!(act1::status_fn(&ctl, &mut f, i0, P1, &pf, 0), None);
}

#[test]
fn the_mod_sequence_function_runs_first() {
    let (mut ctl, _) = control();
    let mut o = Fake::new();
    // Chain 1 at state 1, not its pass state: D2RUST's walk stops, true.
    assert!(act1::sequence(&mut ctl, &mut o, 1));
    let mut f = modded();
    assert!(!act1::sequence(&mut ctl, &mut f, 1));
    // Chain 2 (state 0, not-intro) is D2RUST's: state 0 → 1, true.
    assert!(act1::sequence(&mut ctl, &mut f, 2));
    assert_eq!(ctl.record(2).unwrap().state, 1);
}

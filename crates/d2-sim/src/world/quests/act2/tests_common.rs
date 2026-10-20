// Spec: specs/world/quests-act2.md §1.1, §1.4, §2 (Test vectors)
//! The Act II record init, the sequence chain and the §1.1 shorthands
//! shared by every Act II quest, on the quests' fake world.

use super::*;
use crate::world::quests::tests::*;
use crate::world::quests::{QuestControl, TimerFn};

/// Act II player record setup: P1 in Lut Gholein.
fn fake() -> Fake {
    let mut f = Fake::new();
    f.p(P1).act = Some(ACT);
    f.p(P1).level = Some(TOWN);
    f
}

// Covers: specs/world/quests-act2.md §2
#[test]
fn records_init() {
    let (ctl, _) = control();
    // (chain, state, status, init_no, seq_id, active, status fn).
    let want = [
        (8, 1, 0, 4, Some(13), None),
        (9, 0, 13, 0, None, Some(0x0059_E630)),
        (10, 0, 0, 4, Some(11), None),
        (11, 0, 0, 5, Some(13), None),
        (12, 0, 0, 2, None, None),
        (13, 0, 0, 4, None, Some(0x0059_CA50)),
    ];
    for (chain, state, status, init_no, seq, sfn) in want {
        let r = ctl.record(chain).unwrap();
        assert_eq!(
            (r.state, r.status, r.init_no, r.seq_id, r.status_fn),
            (state, status, init_no, seq, sfn),
            "chain {chain}"
        );
        assert!(r.active && r.not_intro && r.act == ACT, "chain {chain}");
        assert_eq!(r.extra.a2, Extra::default());
    }
    // The gossip records keep the table defaults.
    for chain in [7, 26, 27] {
        let r = ctl.record(chain).unwrap();
        assert!(!r.active && r.state == 0 && r.status == 0, "chain {chain}");
    }
}

fn states(ctl: &QuestControl) -> [u8; 4] {
    [8, 10, 11, 13].map(|c| ctl.record(c).unwrap().state)
}

// Covers: specs/world/quests-act2.md §1.4
#[test]
fn sequence_chain() {
    let mut f = fake();
    // Radament not done: seq(8) returns 1 and nothing moves.
    let (mut ctl, _) = control();
    assert!(sequence(&mut ctl, &mut f, 8));
    assert_eq!(states(&ctl), [1, 0, 0, 0]);
    // Radament finished (state 5): Tombs 0 → 1, then seq(10): Tainted
    // Sun at 0 (not 5) returns 1.
    ctl.record_mut(8).unwrap().state = 5;
    assert!(sequence(&mut ctl, &mut f, 8));
    assert_eq!(states(&ctl), [5, 0, 0, 1]);
    // Tainted Sun finished: seq(10) → seq(11): Arcane 0 → 1 and seq(13)
    // (Tombs already 1, stays); returns 1.
    ctl.record_mut(10).unwrap().state = 5;
    assert!(sequence(&mut ctl, &mut f, 10));
    assert_eq!(states(&ctl), [5, 5, 1, 1]);
    // Arcane at 1: seq(11) only returns 1.
    assert!(sequence(&mut ctl, &mut f, 11));
    assert_eq!(states(&ctl), [5, 5, 1, 1]);
}

// Covers: specs/world/quests-act2.md §1.4
#[test]
fn sequence_chain_intro_and_absent() {
    let mut f = fake();
    let (mut ctl, _) = control();
    // Radament off (intro): seq(8) walks on whatever its state.
    ctl.record_mut(8).unwrap().not_intro = false;
    assert!(sequence(&mut ctl, &mut f, 8));
    assert_eq!(states(&ctl), [1, 0, 0, 1]);
    // An intro Tombs record does not move, and returns seq(10) = 1.
    let (mut ctl, _) = control();
    ctl.record_mut(13).unwrap().not_intro = false;
    assert!(sequence(&mut ctl, &mut f, 13));
    assert_eq!(ctl.record(13).unwrap().state, 0);
    // Chain 13 absent: seq(8) at state 5 returns 0.
    let (mut ctl, _) = control();
    ctl.record_mut(8).unwrap().state = 5;
    ctl.records.retain(|r| r.chain != 13);
    assert!(!sequence(&mut ctl, &mut f, 8));
    // Chain 10 absent: seq(13) returns 1 (and still moves state 0 → 1).
    let (mut ctl, _) = control();
    ctl.records.retain(|r| r.chain != 10);
    assert!(sequence(&mut ctl, &mut f, 13));
    assert_eq!(ctl.record(13).unwrap().state, 1);
    // Chain 11 absent: seq(10) at state 5 returns 0.
    let (mut ctl, _) = control();
    ctl.record_mut(10).unwrap().state = 5;
    ctl.records.retain(|r| r.chain != 11);
    assert!(!sequence(&mut ctl, &mut f, 10));
    // Chain 9 and 12 have none: the dispatcher returns 0.
    assert!(!sequence(&mut ctl, &mut f, 9));
    assert!(!sequence(&mut ctl, &mut f, 12));
}

// Covers: specs/world/quests-act2.md §1.4
#[test]
fn game_entry_runs_chain_8() {
    // `quests.md` §3 step 3 calls seq(8) with the other roots: a first
    // player entering with Radament switched off moves the Tombs to 1.
    let mut f = fake();
    let (mut ctl, _) = control();
    ctl.player_enters(&mut f, P1, 1).unwrap();
    assert_eq!(states(&ctl), [1, 0, 0, 0]);
    let (mut ctl, _) = control();
    ctl.record_mut(8).unwrap().not_intro = false;
    ctl.player_enters(&mut f, P1, 1).unwrap();
    assert_eq!(states(&ctl), [1, 0, 0, 1]);
}

// Covers: specs/world/quests-act2.md §1.1
#[test]
fn status_to_all_and_completion_flag() {
    let mut f = fake();
    f.players.insert(
        P2,
        Player {
            guid: 2,
            act: Some(ACT),
            level: Some(TOWN),
            ..Player::default()
        },
    );
    let p3 = crate::units::UnitId(3);
    f.players.insert(
        p3,
        Player {
            guid: 3,
            act: Some(0),
            level: Some(1),
            ..Player::default()
        },
    );
    let (mut ctl, _) = control();
    let i = ctl.find(12).unwrap();
    ctl.records[i].flags = 0x20;
    // P2 has 13.0 (no 13, 14, 15): F skips it; P3 is in Act I (send
    // status's act test).
    f.p(P2).quests.flags[0].set(13, 0);
    status_all(&mut ctl, &mut f, i, 4);
    assert_eq!(ctl.records[i].flags, 0);
    assert_eq!(f.sent, [(P1, vec![0x5D, 12, 0, 4, 0, 0])]);
    // 13.14 makes F send again.
    f.sent.clear();
    f.p(P2).quests.flags[0].set(13, 14);
    status_all(&mut ctl, &mut f, i, 2);
    assert_eq!(f.sent.len(), 2);
    // Silent: status only.
    f.sent.clear();
    status_silent(&mut ctl, i, 7);
    assert!(f.sent.is_empty());
    assert_eq!(ctl.records[i].status, 7);
    // Completion flag: players without 13.0 and 13.1 (P1, P3; P3 in
    // Act I still gets it, act argument 0).
    f.p(P1).quests.flags[0].set(13, 1);
    completion_flag(&mut f, 12, 13);
    assert_eq!(f.sent, [(p3, vec![0x5D, 12, 0, 12, 0, 0])]);
    assert!(f.flags(p3).get(13, 14));
    assert!(!f.flags(P1).get(13, 14));
}

// Covers: specs/world/quests-act2.md §1.1, §1.3
#[test]
fn quick_remove_and_gold() {
    let f = fake();
    let (mut ctl, _) = control();
    let i = ctl.find(8).unwrap();
    // Empty list: nothing.
    quick_remove(&mut ctl, &f, i, Some(P1));
    ctl.records[i].guids.add(1);
    ctl.records[i].guids.add(9);
    quick_remove(&mut ctl, &f, i, Some(P1));
    assert_eq!(ctl.records[i].guids.0, [9]);
    // §1.3 gold: quest seed {12345, 666} → lo' 22752887 → 7 piles.
    let mut f = fake();
    ctl.seed = crate::rng::Seed::new(12345, 666);
    let o = crate::units::UnitId(0x40);
    chest_gold(&mut ctl, &mut f, o);
    assert_eq!(f.log, vec!["gold 64".to_string(); 7]);
}

// Covers: specs/world/quests-act2.md §1.1
#[test]
fn timers_route_through_act2() {
    let (mut ctl, _) = control();
    add_timer(&mut ctl, 8, Timer::RadamentStatus, 12);
    assert_eq!(ctl.timers[0].func, TimerFn::Act2(Timer::RadamentStatus));
    assert_eq!((ctl.timers[0].chain, ctl.timers[0].due), (8, 12));
    // Added while the updater runs: a fault, no timer.
    ctl.executing = true;
    add_timer(&mut ctl, 8, Timer::Darken, 1);
    assert_eq!(ctl.timers.len(), 1);
    assert_eq!(ctl.faults.len(), 1);
}

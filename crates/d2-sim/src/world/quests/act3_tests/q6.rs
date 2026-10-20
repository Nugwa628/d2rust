// Spec: specs/world/quests-act3.md §8 (A3Q6 The Guardian tests)
//! A3Q6 (chain 20, slot 22) callback by callback.

use super::*;

const CH: u8 = 20;
const S: u8 = 22;
const MEPHISTO_U: UnitId = UnitId(0x40);
const GATE_U: UnitId = UnitId(0x50);
const BRIDGE_U: UnitId = UnitId(0x51);
const NAT_OBJ_U: UnitId = UnitId(0x52);
const NAT_MON_U: UnitId = UnitId(0x53);

fn setup() -> (QuestControl, Fake3, usize) {
    let (ctl, _) = control();
    let i = ctl.find(CH).unwrap();
    (ctl, Fake3::new(), i)
}

fn ev(e: u8, target: Option<UnitId>, player: Option<UnitId>, a: u32, b: u32) -> EventArgs {
    EventArgs {
        event: e,
        target,
        player,
        a,
        b,
    }
}

fn x(ctl: &QuestControl) -> &act3::q6::Extra {
    &ctl.record(CH).unwrap().extra.act3.q6
}

fn xm(ctl: &mut QuestControl) -> &mut act3::q6::Extra {
    &mut ctl.record_mut(CH).unwrap().extra.act3.q6
}

fn setf(f: &mut Fake3, p: UnitId, slot: u8, b: u8) {
    f.p(p).quests.flags[0].set(slot, b);
}

fn level(ctl: &mut QuestControl, f: &mut Fake3, old: u32, new: u32) {
    call(
        ctl,
        f,
        CH,
        ev(event::CHANGED_LEVEL, Some(P1), Some(P1), old, new),
    );
}

fn start(ctl: &mut QuestControl, f: &mut Fake3) {
    call(
        ctl,
        f,
        CH,
        ev(event::PLAYER_STARTED_GAME, Some(P1), Some(P1), 0, 0),
    );
}

fn sounds(f: &Fake3) -> Vec<String> {
    f.log()
        .into_iter()
        .filter(|l| l.starts_with("sound"))
        .collect()
}

// ------------------------------------------------------------ §8.2

// Covers: specs/world/quests-act3.md §8.2, §1.2
#[test]
fn event0_message_selection() {
    let (mut ctl, mut f, i) = setup();
    assert!(text(&mut ctl, &mut f, CH, P1, ORMUS_U).is_empty());
    ctl.records[i].state = 1;
    assert_eq!(text(&mut ctl, &mut f, CH, P1, ORMUS_U), [(628, 0)]);
    ctl.records[i].state = 2;
    assert_eq!(text(&mut ctl, &mut f, CH, P1, ORMUS_U), [(631, 2)]);
    // No intro test in this event.
    ctl.records[i].not_intro = false;
    assert_eq!(text(&mut ctl, &mut f, CH, P1, ORMUS_U), [(631, 2)]);
    // State > 5: table state 6 only with 22.13.
    ctl.records[i].state = 6;
    assert!(text(&mut ctl, &mut f, CH, P1, ORMUS_U).is_empty());
    setf(&mut f, P1, S, 13);
    assert_eq!(text(&mut ctl, &mut f, CH, P1, ORMUS_U), [(658, 2)]);
    // 22.0: table state 6 only when listed (even in state 1).
    ctl.records[i].state = 1;
    setf(&mut f, P1, S, 0);
    assert!(text(&mut ctl, &mut f, CH, P1, ORMUS_U).is_empty());
    ctl.records[i].guids.add(1);
    assert_eq!(text(&mut ctl, &mut f, CH, P1, ORMUS_U), [(658, 2)]);
    // 22.11: table state 5 (menu 1 → 0).
    setf(&mut f, P1, S, 11);
    assert_eq!(text(&mut ctl, &mut f, CH, P1, ORMUS_U), [(658, 0)]);
}

// Covers: specs/world/quests-act3.md §8.2
#[test]
fn active_function() {
    let (mut ctl, mut f, i) = setup();
    let a = |ctl: &QuestControl, f: &mut Fake3, c| act3::active(ctl, f, i, P1, c);
    assert!(!a(&ctl, &mut f, act3::npc::ORMUS));
    ctl.records[i].state = 1;
    assert!(a(&ctl, &mut f, act3::npc::ORMUS));
    assert!(!a(&ctl, &mut f, act3::npc::CAIN3));
    setf(&mut f, P1, S, 0);
    assert!(!a(&ctl, &mut f, act3::npc::ORMUS));
    // 22.11: any NPC.
    setf(&mut f, P1, S, 11);
    assert!(a(&ctl, &mut f, act3::npc::CAIN3));
    assert!(a(&ctl, &mut f, act3::npc::HRATLI));
}

// Covers: specs/world/quests-act3.md §8.2, §8.1
#[test]
fn ormus_628_vector() {
    // Test vector: Lam Esen done, Ormus 628 → Guardian state 2.
    let (mut ctl, mut f, i) = setup();
    ctl.game.set(17, 13);
    ctl.records[i].state = 1;
    say(&mut ctl, &mut f, P1, ORMUS_U, 628);
    assert_eq!(ctl.records[i].state, 2);
    assert!(x(&ctl).ormus_started);
    assert!(f.log().iter().any(|l| l.starts_with("0x27 36 ")));
    // Lam Esen not done: state 3.
    let (mut ctl, mut f, i) = setup();
    say(&mut ctl, &mut f, P1, ORMUS_U, 628);
    assert_eq!(ctl.records[i].state, 3);
    // 22.0 without 22.11: ignored.
    let (mut ctl, mut f, i) = setup();
    setf(&mut f, P1, S, 0);
    say(&mut ctl, &mut f, P1, ORMUS_U, 628);
    assert_eq!(ctl.records[i].state, 0);
    assert!(!f.log().iter().any(|l| l.starts_with("0x27")));
}

// Covers: specs/world/quests-act3.md §8.2
#[test]
fn messages_657_663() {
    let (mut ctl, mut f, i) = setup();
    setf(&mut f, P1, S, 0);
    setf(&mut f, P1, S, 11);
    setf(&mut f, P1, S, 13);
    ctl.records[i].state = 6;
    say(&mut ctl, &mut f, P1, HRATLI_U, 660);
    let r = ctl.record(CH).unwrap();
    assert_eq!((r.state, r.status), (7, 13));
    assert!(r.guids.contains(1));
    assert!(!f.flags(P1).get(S, 11));
    assert!(sent_5d(&f).is_empty());
    assert!(f.log().iter().any(|l| l.starts_with("0x27 34 ")));
    // Without 22.11 (and 22.0 clear): only the refresh.
    let (mut ctl, mut f, i) = setup();
    setf(&mut f, P1, S, 13);
    say(&mut ctl, &mut f, P1, CAIN3_U, 657);
    assert_eq!(ctl.records[i].state, 0);
    assert!(ctl.records[i].guids.0.is_empty());
    assert!(f.log().iter().any(|l| l.starts_with("0x27 32 ")));
    // 22.11 without 22.13: no status change.
    let (mut ctl, mut f, i) = setup();
    setf(&mut f, P1, S, 11);
    ctl.records[i].state = 6;
    say(&mut ctl, &mut f, P1, CAIN3_U, 663);
    assert_eq!(ctl.records[i].state, 6);
    assert!(!f.flags(P1).get(S, 11) && ctl.records[i].guids.contains(1));
    // Outside 657–663: nothing.
    let (mut ctl, mut f, _) = setup();
    setf(&mut f, P1, S, 11);
    say(&mut ctl, &mut f, P1, CAIN3_U, 664);
    assert!(f.flags(P1).get(S, 11));
}

// Covers: specs/world/quests-act3.md §8.2, §1.3
#[test]
fn chat_end_and_sequence_install() {
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
    // The seq fn (state 0, not-intro): state 1 and callback 2.
    assert!(act3::sequence(&mut ctl, &mut f, CH));
    assert_eq!(ctl.records[i].state, 1);
    assert!(ctl.records[i].has_callback(event::NPC_DEACTIVATE));
    // Chat end without the pending flag: nothing.
    call(
        &mut ctl,
        &mut f,
        CH,
        ev(event::NPC_DEACTIVATE, Some(ORMUS_U), Some(P1), 0, 0),
    );
    assert!(f.f.sent.is_empty());
    xm(&mut ctl).ormus_started = true;
    ctl.records[i].state = 3;
    call(
        &mut ctl,
        &mut f,
        CH,
        ev(event::NPC_DEACTIVATE, Some(CAIN3_U), Some(P1), 0, 0),
    );
    assert!(f.f.sent.is_empty());
    call(
        &mut ctl,
        &mut f,
        CH,
        ev(event::NPC_DEACTIVATE, Some(ORMUS_U), Some(P1), 0, 0),
    );
    assert_eq!(sent_5d(&f), [(P1, vec![0x5D, CH, 0, 2, 0, 0])]);
    assert!(!x(&ctl).ormus_started);
    assert!(!ctl.records[i].has_callback(event::NPC_DEACTIVATE));
    // No flag iterate here.
    assert!(!f.flags(P1).get(S, 3));
}

// ------------------------------------------------------------ §8.3, §8.4

// Covers: specs/world/quests-act3.md §8.3, §8.4 r3
#[test]
fn flag_iterate_vector() {
    // Test vector: state 5, status 3 → 22.8 (status 3 sent on entering
    // Durance 1 from status 2).
    let (mut ctl, mut f, i) = setup();
    f.add_player(P2, 100);
    setf(&mut f, P2, S, 11);
    ctl.records[i].state = 5;
    ctl.records[i].status = 2;
    level(&mut ctl, &mut f, 83, 100);
    assert_eq!(ctl.records[i].status, 3);
    assert!(f.flags(P1).get(S, 8));
    // P2 (22.11) skipped; status 3 not sent to it (F: 22.0 / 22.15
    // clear → sent).
    assert!(!f.flags(P2).get(S, 8));
    assert_eq!(sent_5d(&f).len(), 2);
    // The other rows.
    for (state, status, b) in [(2, 0, 2), (3, 0, 3), (4, 2, 5), (5, 0, 8)] {
        let (mut ctl, mut f, i) = setup();
        ctl.records[i].state = state;
        ctl.records[i].status = status;
        level(&mut ctl, &mut f, 83, 100);
        let fl = f.flags(P1);
        let set: Vec<u8> = (2..=9).filter(|&k| fl.get(S, k)).collect();
        assert_eq!(set, [b], "state {state}");
    }
}

// Covers: specs/world/quests-act3.md §8.3, §8.4 r4
#[test]
fn durance_3_status_4() {
    // State 4 with status 2 / 3 / 4 → 22.4 / 22.5 / 22.6 rows reached via
    // level 102 (status 4).
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].state = 4;
    ctl.records[i].status = 2;
    level(&mut ctl, &mut f, 101, 102);
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (4, 4));
    assert!(f.flags(P1).get(S, 6));
    assert_eq!(sent_5d(&f), [(P1, vec![0x5D, CH, 0, 4, 0, 0])]);
    // Status already 4, state 1 → S(4, 5) = 5; 22.9; nothing sent.
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].state = 1;
    ctl.records[i].status = 4;
    level(&mut ctl, &mut f, 101, 102);
    assert_eq!(ctl.records[i].state, 5);
    assert!(f.flags(P1).get(S, 9));
    assert!(sent_5d(&f).is_empty());
    // State 6 → S(4, 5); Lam Esen done → 4.
    let (mut ctl, mut f, i) = setup();
    ctl.game.set(17, 13);
    ctl.records[i].state = 6;
    ctl.records[i].status = 4;
    level(&mut ctl, &mut f, 101, 102);
    assert_eq!(ctl.records[i].state, 4);
}

// Covers: specs/world/quests-act3.md §8.4 r3
#[test]
fn durance_1_rules() {
    // State 1, status 0: status 3 to all, state S(4, 5), iterate.
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].state = 1;
    level(&mut ctl, &mut f, 83, 100);
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (5, 3));
    assert!(f.flags(P1).get(S, 8));
    // State 1, status 1: no send but still state and iterate.
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].state = 1;
    ctl.records[i].status = 1;
    level(&mut ctl, &mut f, 83, 100);
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (5, 1));
    assert!(sent_5d(&f).is_empty());
    // State 2, status 3: nothing at all (no send → no iterate).
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].state = 2;
    ctl.records[i].status = 3;
    level(&mut ctl, &mut f, 83, 100);
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (2, 3));
    assert!(!f.flags(P1).get(S, 2));
}

// Covers: specs/world/quests-act3.md §8.4 r2, §edge-cases-original-bugs r9
#[test]
fn guardian_starts_from_ruined_fane() {
    // Level 100 then moves state 1 on to S(4, 5) (§8.4 step 3).
    for (new, want) in [(97, 0), (98, 1), (99, 1), (100, 5), (101, 1)] {
        let (mut ctl, mut f, i) = setup();
        ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
        level(&mut ctl, &mut f, 80, new);
        assert_eq!(ctl.records[i].state, want, "level {new}");
        assert_eq!(
            ctl.records[i].has_callback(event::NPC_DEACTIVATE),
            want != 0
        );
    }
    // Intro: no start.
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].not_intro = false;
    level(&mut ctl, &mut f, 80, 98);
    assert_eq!(ctl.records[i].state, 0);
}

// Covers: specs/world/quests-act3.md §8.4 r1
#[test]
fn leaving_docks() {
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].guids.add(1);
    ctl.records[i].state = 3;
    ctl.records[i].status = 1;
    level(&mut ctl, &mut f, 75, 76);
    assert!(ctl.records[i].guids.0.is_empty());
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (5, 2));
    assert!(sent_5d(&f).is_empty());
    // Iterate with state 5 status 2 → 22.7.
    assert!(f.flags(P1).get(S, 7));
    // The player has 22.11: no change.
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].state = 2;
    setf(&mut f, P1, S, 11);
    level(&mut ctl, &mut f, 75, 76);
    assert_eq!(ctl.records[i].state, 2);
}

// ------------------------------------------------------------ §8.5

/// The Hellgate (GUID 0x50) and Mephisto (class 242).
fn mephisto_world(ctl: &mut QuestControl, f: &mut Fake3) {
    f.f.monsters.insert(
        MEPHISTO_U,
        (
            0x40,
            242,
            UnitKind::Monster {
                class: 242,
                superunique: None,
                owner: None,
            },
        ),
    );
    f.f.objects.insert(GATE_U, (0x50, 342, 0));
    xm(ctl).gate_known = true;
    xm(ctl).gate_guid = 0x50;
}

fn kill(ctl: &mut QuestControl, f: &mut Fake3, killer: Option<UnitId>) {
    call(
        ctl,
        f,
        CH,
        ev(event::MONSTER_KILLED, Some(MEPHISTO_U), killer, 0, 0),
    );
}

// Covers: specs/world/quests-act3.md §8.5 text, §8.5 r1, §8.5 r2, §10; specs/world/quests-act3-2.md §11.2
#[test]
fn mephisto_death() {
    let (mut ctl, mut f, i) = setup();
    assert_eq!(act3::monster_link(242, 242), Some(CH));
    assert_eq!(act3::monster_link(242, 704), None);
    mephisto_world(&mut ctl, &mut f);
    f.f.frame = 50;
    ctl.records[i].state = 5;
    ctl.records[i].clear_callback(event::PLAYER_LEAVES_GAME);
    // P1 the killer (level 102); P2 in 102; P3 in 101, P2's party; P4
    // in 102 with 22.11; P5 in Act I in P2's party; P6 elsewhere.
    let (p3, p4, p5, p6) = (UnitId(3), UnitId(4), UnitId(5), UnitId(6));
    f.p(P1).level = Some(102);
    f.add_player(P2, 102);
    f.add_player(p3, 101);
    f.add_player(p4, 102);
    f.add_player(p5, 102);
    f.add_player(p6, 80);
    f.p(p5).act = Some(0);
    setf(&mut f, p4, S, 11);
    f.f.party.insert(P2, vec![P2, p3, p5]);
    for p in [P1, P2, p3, p4, p5, p6] {
        f.f.client_flags.insert(p, 0);
    }
    kill(&mut ctl, &mut f, Some(P1));
    let r = ctl.record(CH).unwrap();
    assert_eq!(r.state, 6);
    assert!(r.has_callback(event::PLAYER_LEAVES_GAME));
    // Credits: P1, P2 (level 102), P3 (party); P5 is in Act I... but in
    // level 102 too, so it is credited by `0x005BC190`.
    for p in [P1, P2, p3, p5] {
        let fl = f.flags(p);
        assert!(fl.get(S, 13) && fl.get(S, 0) && fl.get(S, 11), "{p:?}");
    }
    assert!(!f.flags(p4).get(S, 0));
    assert!(!f.flags(p6).get(S, 0));
    // The character progression once per credit, in credit order (the
    // level-102 walk before the party pass; `quests-act3-2.md` §11.2:
    // classic, normal → 4·0 + 3 into bits 8–12).
    let prog: Vec<String> = f
        .log()
        .into_iter()
        .filter(|l| l.starts_with("progression"))
        .collect();
    assert_eq!(
        prog,
        [
            "progression 1 0x0300",
            "progression 2 0x0300",
            "progression 5 0x0300",
            "progression 3 0x0300"
        ]
    );
    assert!(!f.log().iter().any(|l| l.contains("0x538680")));
    // Completion flag: P6 only (P4 has 22.11).
    assert!(f.flags(p6).get(S, 14) && !f.flags(p4).get(S, 14));
    let done: Vec<UnitId> = sent_5d(&f)
        .into_iter()
        .filter(|m| m.1 == [0x5D, CH, 0, 12, 0, 0])
        .map(|m| m.0)
        .collect();
    assert_eq!(done, [p6]);
    assert_eq!(
        sounds(&f),
        ["sound 1 66", "sound 2 66", "sound 3 66", "sound 5 66"]
    );
    // Timer period 12, the extra +0x00 set.
    assert!(x(&ctl).timer);
    assert_eq!(ctl.timers.len(), 1);
    assert_eq!(
        (
            ctl.timers[0].func,
            ctl.timers[0].chain,
            ctl.timers[0].period
        ),
        (TimerFn::Act3(act3::Timer::MephistoStatus), CH, 12)
    );
    // Always: game 22.13, the Hellgate opens, soulstones, FX 11.
    assert!(ctl.game.get(S, 13));
    let log = f.log();
    let gate = log.iter().position(|l| l == "mode 80 1").unwrap();
    assert_eq!(log[gate + 1], "event1 80 66");
    assert_eq!(x(&ctl).gate_mode, 2);
    let stones = log.iter().filter(|l| *l == "qdrop 64 mss  2 false").count();
    assert_eq!(stones, 4);
    assert_eq!(x(&ctl).stones_to_drop, 4);
    assert_eq!(x(&ctl).stones_dropped, 4);
    assert!(x(&ctl).stone_dropped);
    assert_eq!(ctl.fx, 11);
    assert!(f.f.sent.iter().any(|m| m.1 == [0x89, 11]));
}

// Covers: specs/world/quests-act3.md §8.5 r1
#[test]
fn mephisto_killer_with_pending_bit() {
    let (mut ctl, mut f, _) = setup();
    mephisto_world(&mut ctl, &mut f);
    // The killer has 22.11 but not 22.0: credited, no soulstone count.
    setf(&mut f, P1, S, 11);
    f.drops = vec![false];
    xm(&mut ctl).timer = true;
    kill(&mut ctl, &mut f, Some(P1));
    assert!(f.flags(P1).get(S, 0));
    assert_eq!(x(&ctl).stones_to_drop, 0);
    assert!(!f.log().iter().any(|l| l.starts_with("qdrop")));
    // The timer exists already: none added.
    assert!(ctl.timers.is_empty());
    // A killer with 22.0: nothing.
    let (mut ctl, mut f, _) = setup();
    mephisto_world(&mut ctl, &mut f);
    setf(&mut f, P1, S, 0);
    kill(&mut ctl, &mut f, Some(P1));
    assert!(!f.flags(P1).get(S, 13));
    assert!(!f.log().iter().any(|l| l.contains("0x538680")));
    assert!(!f.log().iter().any(|l| l.starts_with("progression")));
}

// Covers: specs/world/quests-act3.md §8.5 r2, §edge-cases-original-bugs r11
#[test]
fn mephisto_death_in_intro() {
    let (mut ctl, mut f, i) = setup();
    mephisto_world(&mut ctl, &mut f);
    ctl.records[i].not_intro = false;
    ctl.records[i].state = 7;
    f.f.frame = 10;
    kill(&mut ctl, &mut f, Some(P1));
    assert_eq!(ctl.records[i].state, 6);
    assert!(ctl.game.get(S, 13));
    assert!(!f.flags(P1).get(S, 0));
    assert!(ctl.timers.is_empty());
    assert!(f.log().contains(&"mode 80 1".to_string()));
    assert_eq!(x(&ctl).gate_mode, 2);
    assert_eq!(ctl.fx, 11);
    // Hellgate unknown: no mode change.
    let (mut ctl, mut f, _) = setup();
    kill(&mut ctl, &mut f, None);
    assert!(!f.log().iter().any(|l| l.starts_with("mode")));
    assert_eq!(x(&ctl).gate_mode, 2);
}

// Covers: specs/world/quests-act3.md §8.5 r1
#[test]
fn status_timer() {
    let (mut ctl, mut f, i) = setup();
    xm(&mut ctl).timer = true;
    ctl.records[i].status = 3;
    assert!(act3::run_timer(
        &mut ctl,
        &mut f,
        act3::Timer::MephistoStatus,
        CH
    ));
    assert_eq!(ctl.records[i].status, 4);
    assert!(!x(&ctl).timer);
    assert_eq!(sent_5d(&f), [(P1, vec![0x5D, CH, 0, 4, 0, 0])]);
    // Already 4: nothing sent.
    f.f.sent.clear();
    assert!(act3::run_timer(
        &mut ctl,
        &mut f,
        act3::Timer::MephistoStatus,
        CH
    ));
    assert!(f.f.sent.is_empty());
}

// Covers: specs/world/quests-act3.md §8.5 text
#[test]
fn leave_game_list_remove() {
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].guids.add(1);
    ctl.records[i].guids.add(9);
    call(
        &mut ctl,
        &mut f,
        CH,
        ev(event::PLAYER_LEAVES_GAME, Some(P1), Some(P1), 0, 0),
    );
    assert_eq!(ctl.records[i].guids.0, [9]);
}

// ------------------------------------------------------------ §8.8

// Covers: specs/world/quests-act3.md §8.8, §edge-cases-original-bugs r10
#[test]
fn game_start_vector() {
    // Test vector: only 22.3 → state 2, status 2 (edge case 10: 22.2 →
    // state 3).
    for (bits, want) in [
        (&[3u8][..], (2, 2)),
        (&[2][..], (3, 2)),
        (&[2, 3][..], (2, 2)),
        (&[4][..], (4, 2)),
        (&[5, 4][..], (4, 3)),
        (&[6][..], (4, 4)),
        (&[7, 2][..], (5, 2)),
        (&[8][..], (5, 3)),
        (&[9, 4][..], (5, 4)),
        (&[][..], (0, 0)),
    ] {
        let (mut ctl, mut f, i) = setup();
        for &b in bits {
            setf(&mut f, P1, S, b);
        }
        start(&mut ctl, &mut f);
        let r = &ctl.records[i];
        assert_eq!((r.state, r.status), want, "{bits:?}");
        assert!(!ctl.game.get(S, 13));
    }
    // 22.0, 22.11 or 23.0: game 22.13, Hellgate mode 2.
    for (slot, b) in [(S, 0), (S, 11), (23, 0)] {
        let (mut ctl, mut f, i) = setup();
        setf(&mut f, P1, slot, b);
        setf(&mut f, P1, S, 3);
        start(&mut ctl, &mut f);
        assert!(ctl.game.get(S, 13));
        assert_eq!(x(&ctl).gate_mode, 2);
        assert_eq!(ctl.records[i].state, 0);
    }
}

// ------------------------------------------------------------ §8.6, §8.7

// Covers: specs/world/quests-act3.md §8.6, §1.4
#[test]
fn hellgate_init() {
    // Outer Steppes (level 104): mode 2, nothing stored.
    let (mut ctl, mut f, _) = setup();
    f.f.objects.insert(GATE_U, (0x50, 342, 0));
    f.levels.insert(GATE_U, 104);
    act3::hellgate_init(&mut ctl, &mut f, GATE_U);
    assert_eq!(f.log(), ["mode 80 2"]);
    assert!(!x(&ctl).gate_known);
    // Durance 3: GUID stored, mode := +0x0C.
    f.levels.insert(GATE_U, 102);
    f.f.log.clear();
    xm(&mut ctl).gate_mode = 2;
    act3::hellgate_init(&mut ctl, &mut f, GATE_U);
    assert_eq!(f.log(), ["mode 80 2"]);
    assert!(x(&ctl).gate_known);
    assert_eq!(x(&ctl).gate_guid, 0x50);
    xm(&mut ctl).gate_mode = 0;
    f.f.log.clear();
    act3::hellgate_init(&mut ctl, &mut f, GATE_U);
    assert_eq!(f.log(), ["mode 80 0"]);
}

// Covers: specs/world/quests-act3.md §8.6
#[test]
fn bridge_init_and_event() {
    let (mut ctl, mut f, _) = setup();
    f.f.objects.insert(BRIDGE_U, (0x51, 341, 0));
    f.f.frame = 200;
    act3::bridge_init(&mut ctl, &mut f, BRIDGE_U);
    assert_eq!(f.log(), ["mode 81 0", "event7 81 220"]);
    assert!(x(&ctl).bridge_known);
    assert_eq!(x(&ctl).bridge_guid, 0x51);
    // Before Mephisto dies: only the re-schedule.
    f.f.log.clear();
    f.player_near = Some(P1);
    act3::bridge_event(&mut ctl, &mut f, BRIDGE_U);
    assert_eq!(f.log(), ["event7 81 224"]);
    // Mephisto dead, no player near.
    xm(&mut ctl).gate_mode = 2;
    f.player_near = None;
    f.f.log.clear();
    act3::bridge_event(&mut ctl, &mut f, BRIDGE_U);
    assert_eq!(f.log(), ["near 81 18", "event7 81 224"]);
    // A player near: mode 0 → 1, then 1 → 2 with the collision freed.
    f.player_near = Some(P1);
    f.f.log.clear();
    act3::bridge_event(&mut ctl, &mut f, BRIDGE_U);
    assert_eq!(f.log(), ["near 81 18", "mode 81 1", "event7 81 224"]);
    assert_eq!(x(&ctl).bridge_mode, 2);
    f.f.log.clear();
    act3::bridge_event(&mut ctl, &mut f, BRIDGE_U);
    assert_eq!(
        f.log(),
        [
            "near 81 18",
            "mode 81 2",
            "free collision 81",
            "event7 81 224"
        ]
    );
    // In mode 2: stops.
    f.f.log.clear();
    act3::bridge_event(&mut ctl, &mut f, BRIDGE_U);
    assert!(f.log().is_empty());
    // Init with +0x10 = 2: mode 2, no event.
    act3::bridge_init(&mut ctl, &mut f, BRIDGE_U);
    assert_eq!(f.log(), ["mode 81 2"]);
}

// Covers: specs/world/quests-act3.md §8.6, §10
#[test]
fn durance_warp_opens_both() {
    let (mut ctl, mut f, _) = setup();
    // Nothing known: only the modes stored.
    act3::durance_warp(&mut ctl, &mut f);
    assert!(f.log().is_empty());
    assert_eq!((x(&ctl).gate_mode, x(&ctl).bridge_mode), (2, 2));
    f.f.objects.insert(GATE_U, (0x50, 342, 0));
    f.f.objects.insert(BRIDGE_U, (0x51, 341, 0));
    xm(&mut ctl).gate_known = true;
    xm(&mut ctl).gate_guid = 0x50;
    xm(&mut ctl).bridge_known = true;
    xm(&mut ctl).bridge_guid = 0x51;
    // Through the object warp to a level other than 102.
    ctl.object_warp(&mut f, P1, 101);
    assert_eq!(f.log(), ["mode 80 1", "mode 81 2"]);
}

// Covers: specs/world/quests-act3.md §8.7
#[test]
fn natalya_spawn() {
    let (mut ctl, mut f, _) = setup();
    f.f.pos.insert(NAT_OBJ_U, (10, 20, RoomId(3)));
    // Both spawn modes fail.
    act3::natalya_init(&mut ctl, &mut f, NAT_OBJ_U);
    assert_eq!(
        f.log(),
        [
            "spawn 297 10 20 mode 1 r 4294967295",
            "spawn 297 10 20 mode 1 r 3"
        ]
    );
    assert!(!x(&ctl).natalya_spawned);
    // The second succeeds.
    f.f.log.clear();
    f.f.spawns = vec![None, Some(NAT_MON_U)];
    f.f.monsters.insert(
        NAT_MON_U,
        (
            0x53,
            297,
            UnitKind::Monster {
                class: 297,
                superunique: None,
                owner: None,
            },
        ),
    );
    act3::natalya_init(&mut ctl, &mut f, NAT_OBJ_U);
    assert_eq!(f.log().len(), 2);
    assert!(x(&ctl).natalya_spawned);
    assert_eq!(x(&ctl).natalya_guid, 0x53);
    // No stored map AI in 1.14d: never applied.
    assert!(!x(&ctl).ai_applied);
    // She exists: no new spawn.
    f.f.log.clear();
    act3::natalya_init(&mut ctl, &mut f, NAT_OBJ_U);
    assert!(f.log().is_empty());
    // Gone: spawned again (first mode works).
    f.f.monsters.remove(&NAT_MON_U);
    f.f.spawns = vec![Some(UnitId(0x54))];
    act3::natalya_init(&mut ctl, &mut f, NAT_OBJ_U);
    assert_eq!(f.log(), ["spawn 297 10 20 mode 1 r 4294967295"]);
    assert_eq!(x(&ctl).natalya_guid, 0x54);
    // Game 22.13: nothing.
    let (mut ctl, mut f, _) = setup();
    f.f.pos.insert(NAT_OBJ_U, (10, 20, RoomId(3)));
    ctl.game.set(S, 13);
    act3::natalya_init(&mut ctl, &mut f, NAT_OBJ_U);
    assert!(f.log().is_empty());
}

// Spec: specs/world/quests-act5-2.md §6 (Test vectors, Edge cases)
//! A5Q4 Betrayal of Harrogath callback by callback on the quests' fake
//! world.

use super::super::super::tests::*;
use super::super::super::*;
use crate::units::RoomId;

const ANYA_U: UnitId = UnitId(0x40);
const NIHL_U: UnitId = UnitId(0x41);
const LARZUK_U: UnitId = UnitId(0x42);
const DUMMY_U: UnitId = UnitId(0x43);

fn monster(f: &mut Fake, u: UnitId, class: u16, superunique: Option<u32>) {
    let kind = UnitKind::Monster {
        class: u32::from(class),
        superunique,
        owner: None,
    };
    f.monsters.insert(u, (u.0, class, kind));
}

/// P1 in Harrogath (Act V), Anya and Larzuk in town.
fn fake() -> Fake {
    let mut f = Fake::new();
    f.p(P1).act = Some(4);
    f.p(P1).level = Some(109);
    monster(&mut f, ANYA_U, 512, None);
    monster(&mut f, LARZUK_U, 511, None);
    f
}

fn add_p2(f: &mut Fake, act: u8, level: u32) {
    f.players.insert(
        P2,
        Player {
            guid: 2,
            act: Some(act),
            level: Some(level),
            ..Player::default()
        },
    );
}

fn rec(ctl: &QuestControl) -> usize {
    ctl.find(34).unwrap()
}

fn ev(event: u8, target: Option<UnitId>, a: u32, b: u32) -> EventArgs {
    EventArgs {
        event,
        target,
        player: Some(P1),
        a,
        b,
    }
}

fn text(ctl: &mut QuestControl, f: &mut Fake, n: UnitId) -> TextList {
    let mut list = TextList::new();
    let i = rec(ctl);
    assert!(super::callback(
        ctl,
        f,
        i,
        ev(event::NPC_ACTIVATE, Some(n), 0, 0),
        Some(&mut list)
    ));
    list
}

fn strings(l: &TextList) -> Vec<u16> {
    l.iter().map(|e| e.0).collect()
}

fn status_of(ctl: &QuestControl, f: &mut Fake, p: UnitId) -> Option<u8> {
    let pf = f.flags(p);
    super::status(ctl, f, rec(ctl), p, &pf, 0x0058_AF00)
}

/// The 0x5D messages for chain 34 sent so far: (player, status).
fn status_msgs(f: &Fake) -> Vec<(UnitId, u8)> {
    f.sent
        .iter()
        .filter(|m| m.1[0] == 0x5D && m.1[1] == 34)
        .map(|m| (m.0, m.1[3]))
        .collect()
}

// Covers: specs/world/quests-act5-2.md §6.9
#[test]
fn status_function() {
    // Vector: 38.1 + 38.4 → 5.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.p(P1).quests.flags[0].set(38, 1);
    f.p(P1).quests.flags[0].set(38, 4);
    assert_eq!(status_of(&ctl, &mut f, P1), Some(5));
    // 38.13 alone → 4; 38.0 wins over everything → 0.
    let mut f = fake();
    f.p(P1).quests.flags[0].set(38, 13);
    assert_eq!(status_of(&ctl, &mut f, P1), Some(4));
    f.p(P1).quests.flags[0].set(38, 0);
    assert_eq!(status_of(&ctl, &mut f, P1), Some(0));
    // Not credited: 38.14 → 12; state > 3 → 0; else the status byte.
    let mut f = fake();
    let i = rec(&ctl);
    ctl.records[i].status = 2;
    ctl.records[i].state = 3;
    assert_eq!(status_of(&ctl, &mut f, P1), Some(2));
    ctl.records[i].state = 4;
    assert_eq!(status_of(&ctl, &mut f, P1), Some(0));
    f.p(P1).quests.flags[0].set(38, 14);
    assert_eq!(status_of(&ctl, &mut f, P1), Some(12));
    // Intro: 0.
    ctl.records[i].not_intro = false;
    assert_eq!(status_of(&ctl, &mut f, P1), Some(0));
    // Reached from the 0x52 list (status ≠ 0, `quests.md` §6.2).
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    ctl.records[i].status = 3;
    ctl.records[i].state = 3;
    ctl.request_quest_data(&mut f, P1).unwrap();
    let list = &f.sent.last().unwrap().1;
    assert_eq!(list[0], 0x52);
    assert_eq!(list[1 + 38], 3);
    assert!(f.log.iter().all(|l| !l.contains("0x58af00")));
}

// Covers: specs/world/quests-act5-2.md §6.1, §6.6, §10, §edge-cases-original-bugs r1
#[test]
fn nihlathak_killed() {
    // Vector: P in the next room with 37.0, lacking 38.* → 38.1, 38.13;
    // sound 82; game 38.13; FX 17; timer 8.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.p(P1).level = Some(123);
    f.p(P1).quests.flags[0].set(37, 0);
    // P2 in Act V elsewhere, in P1's party, with 37.1 pending.
    add_p2(&mut f, 4, 110);
    f.p(P2).quests.flags[0].set(37, 1);
    f.party.insert(P1, vec![P1, P2]);
    f.near = vec![P1];
    monster(&mut f, NIHL_U, 526, Some(60));
    f.pos.insert(NIHL_U, (50, 60, RoomId(7)));
    // Base id 526 gets the chain-34 link (§10); superunique 60 kills
    // are forced.
    let i = rec(&ctl);
    ctl.records[i].active = false;
    f.chains.insert(NIHL_U, QuestChain::default());
    assert!(ctl.add_link(&mut f, NIHL_U, 34, None));
    ctl.monster_killed(&mut f, NIHL_U, Some(P1));
    for p in [P1, P2] {
        let fl = f.flags(p);
        assert!(fl.get(38, 1) && fl.get(38, 13), "{p:?}");
        assert!(!fl.get(38, 14));
    }
    assert!(ctl.game.get(38, 13));
    assert_eq!(f.log, ["sound 1 82", "sound 2 82"]);
    assert_eq!(ctl.fx, 17);
    assert!(f.sent.iter().any(|m| m.1 == [0x89, 17]));
    let x = &ctl.records[i].extra.a5.q4;
    assert_eq!(x.kill_room, Some(RoomId(7)));
    assert!(x.timer);
    assert_eq!(ctl.records[i].state, 4);
    assert_eq!(ctl.timers.len(), 1);
    assert_eq!((ctl.timers[0].chain, ctl.timers[0].period), (34, 8));
    // The timer: status 4 to all once, then removed.
    f.sent.clear();
    for _ in 0..10 {
        ctl.update(&mut f);
    }
    assert!(ctl.timers.is_empty());
    assert_eq!(ctl.records[i].status, 4);
    assert!(!ctl.records[i].extra.a5.q4.timer);
    // Both players are in Act V; F sends to 13 holders.
    assert_eq!(status_msgs(&f), [(P1, 4), (P2, 4)]);
}

// Covers: specs/world/quests-act5-2.md §6.6, §edge-cases-original-bugs r1
#[test]
fn nihlathak_kill_needs_prison_of_ice_and_the_room() {
    let (mut ctl, _) = control();
    let mut f = fake();
    // P1 near but without 37.0 / 37.1; P2 with 37.0 but far, no party.
    add_p2(&mut f, 4, 110);
    f.p(P2).quests.flags[0].set(37, 0);
    f.near = vec![P1];
    monster(&mut f, NIHL_U, 526, Some(60));
    f.pos.insert(NIHL_U, (50, 60, RoomId(7)));
    let i = rec(&ctl);
    let args = EventArgs {
        event: event::MONSTER_KILLED,
        target: Some(NIHL_U),
        player: Some(P1),
        ..EventArgs::default()
    };
    super::callback(&mut ctl, &mut f, i, args, None);
    for p in [P1, P2] {
        assert!(!f.flags(p).get(38, 1) && !f.flags(p).get(38, 13));
        // Completion flag `0x0058B6D0`: 38.14 and `5D 22 00 0C 0000`.
        assert!(f.flags(p).get(38, 14));
        assert!(f
            .sent
            .iter()
            .any(|m| m.0 == p && m.1 == hex("5D 22 00 0C 0000")));
    }
    assert!(f.log.iter().all(|l| !l.starts_with("sound")));
    // An existing timer is not doubled; the game bit and state still
    // change.
    assert_eq!(ctl.timers.len(), 1);
    super::callback(&mut ctl, &mut f, i, args, None);
    assert_eq!(ctl.timers.len(), 1);
    // Intro or no victim room: nothing.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.p(P1).quests.flags[0].set(37, 0);
    f.near = vec![P1];
    let i = rec(&ctl);
    super::callback(&mut ctl, &mut f, i, args, None);
    assert!(!f.flags(P1).get(38, 1) && ctl.timers.is_empty());
    f.pos.insert(NIHL_U, (1, 1, RoomId(1)));
    ctl.records[i].not_intro = false;
    super::callback(&mut ctl, &mut f, i, args, None);
    assert!(!f.flags(P1).get(38, 1) && ctl.timers.is_empty() && !ctl.game.get(38, 13));
}

// Covers: specs/world/quests-act5-2.md §6.8
#[test]
fn intro_game_start_without_halls_waypoint() {
    // Vector: chain 34 state 5; +0x89 = 1.
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    ctl.records[i].not_intro = false;
    super::callback(
        &mut ctl,
        &mut f,
        i,
        ev(event::PLAYER_STARTED_GAME, Some(P1), 0, 0),
        None,
    );
    assert_eq!(ctl.records[i].state, 5);
    assert!(ctl.records[i].extra.a5.q4.anya_portal);
    assert_eq!(f.log, ["waypoint 1 123"]);
    // With the waypoint: no portal wanted.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.a5_waypoints.push((P1, 123));
    ctl.records[i].not_intro = false;
    super::callback(
        &mut ctl,
        &mut f,
        i,
        ev(event::PLAYER_STARTED_GAME, Some(P1), 0, 0),
        None,
    );
    assert!(!ctl.records[i].extra.a5.q4.anya_portal);
}

// Covers: specs/world/quests-act5-2.md §6.8
#[test]
fn game_start_restores_progress() {
    // 38.3: +0x87, status 2 to all with the flags byte kept, state 3.
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    ctl.records[i].flags = 0x40;
    f.p(P1).quests.flags[0].set(38, 3);
    let args = ev(event::PLAYER_STARTED_GAME, Some(P1), 0, 0);
    super::callback(&mut ctl, &mut f, i, args, None);
    let r = &ctl.records[i];
    assert_eq!((r.state, r.status, r.flags), (3, 2, 0x40));
    assert!(r.extra.a5.q4.portal_wanted);
    assert_eq!(f.sent, [(P1, hex("5D 22 40 02 0000"))]);
    // 38.2: +0x87, status 1 to all with the flags byte kept
    // (`0x0058BA86`), state 2.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.records[i].flags = 0x40;
    f.p(P1).quests.flags[0].set(38, 2);
    super::callback(&mut ctl, &mut f, i, args, None);
    let r = &ctl.records[i];
    assert_eq!((r.state, r.status, r.flags), (2, 1, 0x40));
    assert!(r.extra.a5.q4.portal_wanted);
    assert_eq!(f.sent, [(P1, hex("5D 22 40 01 0000"))]);
    // 38.1 set: nothing.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.p(P1).quests.flags[0].set(38, 2);
    f.p(P1).quests.flags[0].set(38, 1);
    super::callback(&mut ctl, &mut f, i, args, None);
    let r = &ctl.records[i];
    assert_eq!((r.state, r.status), (0, 0));
    assert!(!r.extra.a5.q4.portal_wanted);
}

// Covers: specs/world/quests-act5-2.md §6.3
#[test]
fn chat_tables() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    // Without 37.0 / 37.1: nothing.
    ctl.records[i].state = 1;
    assert!(text(&mut ctl, &mut f, ANYA_U).is_empty());
    f.p(P1).quests.flags[0].set(37, 0);
    // drehya, state 1 → table state 0 (20137).
    assert_eq!(strings(&text(&mut ctl, &mut f, ANYA_U)), [20137]);
    // Larzuk in state 1 → index[1] = 0: no Larzuk entry in table state 0.
    assert!(text(&mut ctl, &mut f, LARZUK_U).is_empty());
    // State 2 → table state 1 (Larzuk 20141).
    ctl.records[i].state = 2;
    assert_eq!(strings(&text(&mut ctl, &mut f, LARZUK_U)), [20141]);
    // State 4 needs 38.13.
    ctl.records[i].state = 4;
    assert!(text(&mut ctl, &mut f, LARZUK_U).is_empty());
    // 38.1 → table state 3 (Anya 20148), with 38.4 → 4 (menu 2).
    f.p(P1).quests.flags[0].set(38, 1);
    assert_eq!(text(&mut ctl, &mut f, ANYA_U), [(20148, 0)]);
    f.p(P1).quests.flags[0].set(38, 4);
    assert_eq!(text(&mut ctl, &mut f, ANYA_U), [(20148, 2)]);
    // GUID listed → 4; 38.0 → nothing.
    f.p(P1).quests.flags[0].clear(38, 1);
    ctl.records[i].guids.add(1);
    assert_eq!(strings(&text(&mut ctl, &mut f, LARZUK_U)), [20150]);
    ctl.records[i].guids.remove(1);
    f.p(P1).quests.flags[0].set(38, 0);
    assert!(text(&mut ctl, &mut f, LARZUK_U).is_empty());
    // drehya in state 1 with 38.0: nothing.
    ctl.records[i].state = 1;
    assert!(text(&mut ctl, &mut f, ANYA_U).is_empty());
    // Intro: index[state] is not used.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.p(P1).quests.flags[0].set(37, 0);
    ctl.records[i].state = 2;
    ctl.records[i].not_intro = false;
    assert!(text(&mut ctl, &mut f, LARZUK_U).is_empty());
}

// Covers: specs/world/quests-act5-2.md §6.3
#[test]
fn anya_wants_to_talk() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    let act = |ctl: &QuestControl, f: &mut Fake, c: u16| super::active(ctl, f, i, P1, c, 0);
    ctl.records[i].state = 1;
    assert!(!act(&ctl, &mut f, 512));
    f.p(P1).quests.flags[0].set(37, 1);
    assert!(act(&ctl, &mut f, 512) && !act(&ctl, &mut f, 511));
    f.p(P1).quests.flags[0].set(38, 1);
    assert!(!act(&ctl, &mut f, 512));
    // Other states: 38.13 set and 38.4 clear.
    ctl.records[i].state = 4;
    assert!(!act(&ctl, &mut f, 512));
    f.p(P1).quests.flags[0].set(38, 13);
    assert!(act(&ctl, &mut f, 512));
    f.p(P1).quests.flags[0].set(38, 4);
    assert!(!act(&ctl, &mut f, 512));
    // Through 0x8A (`quests.md` §6.4).
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.picked = true;
    ctl.records[i].state = 1;
    f.p(P1).quests.flags[0].set(37, 0);
    ctl.npc_wants_interact(&mut f, P1, ANYA_U, 512, true)
        .unwrap();
    assert_eq!(f.sent, [(P1, hex("8A 01 40000000"))]);
}

// Covers: specs/world/quests-act5-2.md §6.4, §6.2, §6.7
#[test]
fn anya_starts_the_quest_and_opens_the_portal() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    ctl.records[i].state = 1;
    f.p(P1).quests.flags[0].set(37, 0);
    add_p2(&mut f, 4, 109); // no 37.* → not iterated
    f.pos.insert(ANYA_U, (100, 200, RoomId(3)));
    // 20137 from another NPC: nothing.
    let msg = |n: u16, m: u32| ev(event::SCROLL_MESSAGE, Some(ANYA_U), u32::from(n), m);
    super::callback(&mut ctl, &mut f, i, msg(511, 20137), None);
    assert_eq!(ctl.records[i].state, 1);
    super::callback(&mut ctl, &mut f, i, msg(512, 20137), None);
    let x = &ctl.records[i].extra.a5.q4;
    assert!(x.anya_started && x.portal_wanted);
    assert_eq!(ctl.records[i].state, 2);
    assert!(f.flags(P1).get(38, 2) && !f.flags(P2).get(38, 2));
    // Chat end with Anya: status 1 to all, +0x86 := 0, the portal.
    let end = ev(event::NPC_DEACTIVATE, Some(ANYA_U), 0, 0);
    super::callback(&mut ctl, &mut f, i, end, None);
    assert_eq!(ctl.records[i].status, 1);
    assert_eq!(status_msgs(&f), [(P1, 1), (P2, 1)]);
    assert_eq!(f.log, ["portal 110 205 60 121"]);
    let x = &ctl.records[i].extra.a5.q4;
    assert!(!x.anya_started && !x.portal_wanted && x.portal_made);
    // Again: nothing more.
    f.sent.clear();
    super::callback(&mut ctl, &mut f, i, end, None);
    assert!(f.sent.is_empty() && f.log.len() == 1);
    // Chat end with another NPC: nothing.
    ctl.records[i].extra.a5.q4.anya_started = true;
    super::callback(
        &mut ctl,
        &mut f,
        i,
        ev(event::NPC_DEACTIVATE, Some(LARZUK_U), 0, 0),
        None,
    );
    assert!(f.sent.is_empty());
    // Intro: 20137 does nothing.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.records[i].not_intro = false;
    super::callback(&mut ctl, &mut f, i, msg(512, 20137), None);
    assert_eq!(ctl.records[i].extra.a5.q4, super::Extra::default());
}

// Covers: specs/world/quests-act5-2.md §6.4, §6.10, §edge-cases-original-bugs r2
#[test]
fn anya_reward_line() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    ctl.records[i].state = 4;
    ctl.records[i].status = 4;
    ctl.records[i].flags = 0x55;
    f.p(P1).quests.flags[0].set(38, 13);
    f.p(P1).quests.flags[0].set(38, 1);
    let msg = ev(event::SCROLL_MESSAGE, Some(ANYA_U), 512, 20148);
    super::callback(&mut ctl, &mut f, i, msg, None);
    // Edge case 2: the current status again, flags := 0 (the status
    // function says 4: 38.4 is set after the send).
    assert_eq!(ctl.records[i].flags, 0);
    assert_eq!(f.sent[0], (P1, hex("5D 22 00 04 0000")));
    assert!(f.flags(P1).get(38, 4));
    // State 5 and the own sequence function: chain 35 opens.
    assert_eq!(ctl.records[i].state, 5);
    assert_eq!(ctl.record(35).unwrap().state, 1);
    // Without 38.13 in a not-intro record: only 38.4 and the resend.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.records[i].state = 4;
    f.p(P1).quests.flags[0].set(38, 1);
    super::callback(&mut ctl, &mut f, i, msg, None);
    assert!(f.flags(P1).get(38, 4));
    assert_eq!(ctl.records[i].state, 4);
    assert_eq!(ctl.record(35).unwrap().state, 0);
    // Intro with 38.1: state 5.
    ctl.records[i].not_intro = false;
    super::callback(&mut ctl, &mut f, i, msg, None);
    assert_eq!(ctl.records[i].state, 5);
}

// Covers: specs/world/quests-act5-2.md §6.5, §edge-cases-original-bugs r3
#[test]
fn level_changes_and_leave() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    ctl.records[i].state = 2;
    // Edge case 3: P1 lacks 37.* and still moves the state.
    super::callback(
        &mut ctl,
        &mut f,
        i,
        ev(event::CHANGED_LEVEL, Some(P1), 109, 110),
        None,
    );
    assert_eq!(ctl.records[i].state, 3);
    assert!(!f.flags(P1).get(38, 3));
    // With 37.0 a later leave sets 38.3 through the flag iterate.
    ctl.records[i].state = 2;
    f.p(P1).quests.flags[0].set(37, 0);
    super::callback(
        &mut ctl,
        &mut f,
        i,
        ev(event::CHANGED_LEVEL, Some(P1), 109, 110),
        None,
    );
    assert!(f.flags(P1).get(38, 3));
    // Into the temple with status 1 → status 2 to all; +0x86 := 0.
    ctl.records[i].status = 1;
    ctl.records[i].extra.a5.q4.anya_started = true;
    super::callback(
        &mut ctl,
        &mut f,
        i,
        ev(event::CHANGED_LEVEL, Some(P1), 120, 121),
        None,
    );
    assert_eq!(ctl.records[i].status, 2);
    assert!(!ctl.records[i].extra.a5.q4.anya_started);
    assert_eq!(status_msgs(&f), [(P1, 2)]);
    // Status 2: no second send.
    super::callback(
        &mut ctl,
        &mut f,
        i,
        ev(event::CHANGED_LEVEL, Some(P1), 120, 121),
        None,
    );
    assert_eq!(status_msgs(&f).len(), 1);
    // Event 10: both lists.
    ctl.records[i].guids.add(1);
    ctl.records[i].extra.a5.q4.guids.add(1);
    ctl.records[i].extra.a5.q4.guids.add(9);
    super::callback(
        &mut ctl,
        &mut f,
        i,
        ev(event::PLAYER_LEAVES_GAME, Some(P1), 0, 0),
        None,
    );
    assert!(ctl.records[i].guids.0.is_empty());
    assert_eq!(ctl.records[i].extra.a5.q4.guids.0, [9]);
    // Unhandled events are reported by the caller.
    assert!(!super::callback(
        &mut ctl,
        &mut f,
        i,
        ev(event::PLAYER_JOINED_GAME, Some(P1), 0, 0),
        None,
    ));
}

// Covers: specs/world/quests-act5-2.md §6.7, §10
#[test]
fn temple_portal_callers() {
    // Dummy 459's event 7: nothing without +0x87.
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    f.frame = 500;
    super::temple_portal_event(&mut ctl, &mut f, DUMMY_U);
    assert!(f.log.is_empty());
    // +0x87 set, the dummy without a room: not made → again at + 12.
    ctl.records[i].extra.a5.q4.portal_wanted = true;
    super::temple_portal_event(&mut ctl, &mut f, DUMMY_U);
    assert_eq!(f.log, ["event7 67 512"]);
    // Made already (+0x88): the reschedule repeats (+0x87 stays set).
    f.log.clear();
    f.pos.insert(DUMMY_U, (10, 20, RoomId(2)));
    ctl.records[i].extra.a5.q4.portal_made = true;
    super::temple_portal_event(&mut ctl, &mut f, DUMMY_U);
    assert_eq!(f.log, ["event7 67 512"]);
    // Not made yet: made at the dummy + (10, 5).
    f.log.clear();
    ctl.records[i].extra.a5.q4.portal_made = false;
    super::temple_portal_event(&mut ctl, &mut f, DUMMY_U);
    assert_eq!(f.log, ["portal 20 25 60 121"]);
    let x = &ctl.records[i].extra.a5.q4;
    assert!(x.portal_made && !x.portal_wanted);
    // Anya's AI `0x0058BC80`: +0x89 and Anya in Act V.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.players.insert(
        ANYA_U,
        Player {
            act: Some(4),
            ..Player::default()
        },
    );
    f.pos.insert(ANYA_U, (1, 2, RoomId(2)));
    super::anya_ai_portal(&mut ctl, &mut f, ANYA_U);
    assert!(f.log.is_empty());
    ctl.records[i].extra.a5.q4.anya_portal = true;
    super::anya_ai_portal(&mut ctl, &mut f, ANYA_U);
    assert_eq!(f.log, ["portal 11 7 60 121"]);
    assert!(!ctl.records[i].extra.a5.q4.anya_portal);
    // Outside Act V: nothing.
    ctl.records[i].extra.a5.q4.anya_portal = true;
    f.p(ANYA_U).act = Some(3);
    super::anya_ai_portal(&mut ctl, &mut f, ANYA_U);
    assert_eq!(f.log.len(), 1);
}

// Covers: specs/world/quests-act5-2.md §6.11, §11
#[test]
fn personalize_and_nihlathak_ai() {
    let (mut ctl, _) = control();
    let mut f = fake();
    f.p(P1).quests.flags[0].set(38, 1);
    f.p(P1).quests.flags[0].set(38, 15);
    super::personalize_reward(&mut f, P1);
    let fl = f.flags(P1);
    assert!(fl.get(38, 0) && !fl.get(38, 1) && fl.get(38, 15));
    assert!(f.sent.is_empty());
    // `0x0058BC40`: status 3 to all while < 3, not-intro.
    let i = rec(&ctl);
    ctl.records[i].flags = 9;
    super::nihlathak_ai_status(&mut ctl, &mut f);
    assert_eq!((ctl.records[i].status, ctl.records[i].flags), (3, 0));
    f.sent.clear();
    super::nihlathak_ai_status(&mut ctl, &mut f);
    assert!(f.sent.is_empty());
    ctl.records[i].status = 0;
    ctl.records[i].not_intro = false;
    super::nihlathak_ai_status(&mut ctl, &mut f);
    assert_eq!(ctl.records[i].status, 0);
}

// Covers: specs/world/quests-act5-2.md §6.10
#[test]
fn sequence_function() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    // State 0, not-intro: → 1, returns 1, chain 35 untouched.
    assert!(super::super::sequence(&mut ctl, &mut f, 34));
    assert_eq!(ctl.records[i].state, 1);
    assert_eq!(ctl.record(35).unwrap().state, 0);
    // State 3: holds (1), no change.
    ctl.records[i].state = 3;
    assert!(super::super::sequence(&mut ctl, &mut f, 34));
    assert_eq!(ctl.records[i].state, 3);
    // State 4: seq(35) opens chain 35.
    ctl.records[i].state = 4;
    assert!(super::super::sequence(&mut ctl, &mut f, 34));
    assert_eq!(ctl.record(35).unwrap().state, 1);
}

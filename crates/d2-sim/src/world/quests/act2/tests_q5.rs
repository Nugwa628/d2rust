// Spec: specs/world/quests-act2.md §7 (Edge cases 7, 14)
// Spec: specs/world/quests-act2-2.md §1 items 2, 6, 8
//! Tests for [`super::q5`]: the Summoner record on the quests' fake
//! world.

use super::q5;
use super::Timer;
use crate::units::{RoomId, UnitId};
use crate::world::quests::tests::*;
use crate::world::quests::*;

const P3: UnitId = UnitId(3);
const ATMA_U: UnitId = UnitId(0x20);
const GREIZ_U: UnitId = UnitId(0x21);
const SUMMONER_U: UnitId = UnitId(0x30);

fn kind(class: u16) -> UnitKind {
    UnitKind::Monster {
        class: u32::from(class),
        superunique: None,
        owner: None,
    }
}

/// The fake with Atma, Greiz, the Summoner and `n` players in Lut
/// Gholein (P1 first).
fn fake(n: u32) -> Fake {
    let mut f = Fake::new();
    for (u, class) in [(ATMA_U, 176), (GREIZ_U, 198), (SUMMONER_U, 250)] {
        f.monsters.insert(u, (u.0, class, kind(class)));
    }
    for g in 2..=n {
        f.players.insert(
            UnitId(g),
            Player {
                guid: g,
                ..Player::default()
            },
        );
    }
    for p in f.players() {
        f.p(p).act = Some(1);
        f.p(p).level = Some(40);
    }
    f
}

fn set(f: &mut Fake, p: UnitId, b: u8) {
    f.p(p).quests.flags[0].set(13, b);
}

fn ev(ctl: &mut QuestControl, f: &mut Fake, args: EventArgs) -> TextList {
    let mut list = TextList::new();
    let i = ctl.find(12).unwrap();
    super::callback(ctl, f, i, args, Some(&mut list));
    list
}

fn chat(ctl: &mut QuestControl, f: &mut Fake, npc: UnitId) -> TextList {
    ev(
        ctl,
        f,
        EventArgs {
            event: event::NPC_ACTIVATE,
            target: Some(npc),
            player: Some(P1),
            ..EventArgs::default()
        },
    )
}

fn x(ctl: &QuestControl) -> &q5::Extra {
    &ctl.record(12).unwrap().extra.a2.q5
}

// Covers: specs/world/quests-act2.md §7.2
// Covers: specs/world/quests-act2-2.md §1 r8
#[test]
fn summoner_seen_hook() {
    let (mut ctl, _) = control();
    let mut f = fake(2);
    set(&mut f, P2, 1);
    q5::summoner_seen(&mut ctl, &mut f);
    let r = ctl.record(12).unwrap();
    assert!(x(&ctl).seen);
    assert_eq!((r.state, r.status, r.flags), (1, 2, 0));
    // Status 2 to all (default rule: state 1 < init_no 2 → status 2);
    // P2 has 13.1 (F: 13.0 and 13.15 clear) and gets it too.
    assert_eq!(
        f.sent,
        [(P1, hex("5d 0c 00 02 0000")), (P2, hex("5d 0c 00 02 0000"))]
    );
    // 13.2 only for players without 13.0 and 13.1.
    assert_eq!(f.flags(P1).word(13), 1 << 2);
    assert_eq!(f.flags(P2).word(13), 1 << 1);
    // Status already 2: only seen / state.
    let (mut ctl, _) = control();
    let mut f = fake(1);
    ctl.record_mut(12).unwrap().status = 2;
    q5::summoner_seen(&mut ctl, &mut f);
    assert_eq!(ctl.record(12).unwrap().state, 1);
    assert!(f.sent.is_empty() && f.flags(P1).word(13) == 0);
    // Killed before seen (state 2): status 2 but no 13.2.
    let (mut ctl, _) = control();
    let mut f = fake(1);
    ctl.record_mut(12).unwrap().state = 2;
    q5::summoner_seen(&mut ctl, &mut f);
    assert_eq!(ctl.record(12).unwrap().status, 2);
    assert_eq!(f.flags(P1).word(13), 0);
    // Intro: nothing.
    let (mut ctl, _) = control();
    let mut f = fake(1);
    ctl.record_mut(12).unwrap().not_intro = false;
    q5::summoner_seen(&mut ctl, &mut f);
    assert!(!x(&ctl).seen);
    assert_eq!(ctl.record(12).unwrap().state, 0);
    assert!(f.sent.is_empty());
}

// Covers: specs/world/quests-act2.md §7.2, §edge-cases-original-bugs r7
#[test]
fn chat_and_wants_to_talk() {
    let (mut ctl, _) = control();
    let mut f = fake(1);
    // State 0 → nothing.
    assert!(chat(&mut ctl, &mut f, ATMA_U).is_empty());
    // State 1 → index[1] = 0.
    ctl.record_mut(12).unwrap().state = 1;
    assert_eq!(chat(&mut ctl, &mut f, ATMA_U), [(414, 2)]);
    // State 2 without 13.13 → nothing; with it → 1.
    ctl.record_mut(12).unwrap().state = 2;
    assert!(chat(&mut ctl, &mut f, ATMA_U).is_empty());
    set(&mut f, P1, 13);
    assert_eq!(chat(&mut ctl, &mut f, ATMA_U), [(427, 0)]);
    ctl.record_mut(12).unwrap().state = 3;
    assert_eq!(chat(&mut ctl, &mut f, ATMA_U), [(427, 2)]);
    // Past the table (≤ 3) → nothing.
    ctl.record_mut(12).unwrap().state = 4;
    assert!(chat(&mut ctl, &mut f, ATMA_U).is_empty());
    // 13.0 without 13.13 → nothing.
    let mut f = fake(1);
    ctl.record_mut(12).unwrap().state = 1;
    set(&mut f, P1, 0);
    assert!(chat(&mut ctl, &mut f, ATMA_U).is_empty());
    // GUID listed → 2.
    ctl.record_mut(12).unwrap().guids.add(1);
    assert_eq!(chat(&mut ctl, &mut f, ATMA_U), [(427, 2)]);
    // 13.1 → 1, first.
    set(&mut f, P1, 1);
    assert_eq!(chat(&mut ctl, &mut f, ATMA_U), [(427, 0)]);
    assert_eq!(chat(&mut ctl, &mut f, GREIZ_U), [(419, 0)]);
    // Intro → nothing (after 13.1 and the list).
    let mut f = fake(1);
    ctl.record_mut(12).unwrap().guids.0.clear();
    ctl.record_mut(12).unwrap().not_intro = false;
    assert!(chat(&mut ctl, &mut f, ATMA_U).is_empty());
    // Wants to talk: 13.1 and the list, which omits greiz (edge case 7).
    let i = ctl.find(12).unwrap();
    assert!(!q5::active(&ctl, &mut f, i, P1, 176));
    set(&mut f, P1, 1);
    for n in [176, 175, 199, 177, 202, 244, 210, 201, 200, 178] {
        assert!(q5::active(&ctl, &mut f, i, P1, n), "npc {n}");
    }
    assert!(!q5::active(&ctl, &mut f, i, P1, 198));
}

fn kill(ctl: &mut QuestControl, f: &mut Fake) {
    f.chains.insert(SUMMONER_U, QuestChain(vec![12]));
    f.pos.insert(SUMMONER_U, (10, 20, RoomId(7)));
    ctl.monster_killed(f, SUMMONER_U, Some(P1));
}

// Covers: specs/world/quests-act2.md §7.2, §7.1, §edge-cases-original-bugs r14
// Covers: specs/world/quests-act2-2.md §1 r8
#[test]
fn summoner_killed() {
    let (mut ctl, _) = control();
    let mut f = fake(3);
    // P1 in the kill room; P3 in P1's party (in Act II); P2 elsewhere.
    f.near = vec![P1];
    f.party.insert(P1, vec![P1, P3]);
    ctl.record_mut(12).unwrap().extra.a2.q5.phase = 1;
    kill(&mut ctl, &mut f);
    let r = ctl.record(12).unwrap();
    assert_eq!(r.state, 2);
    assert_eq!(
        *x(&ctl),
        q5::Extra {
            killed: true,
            seen: false,
            kill_room: Some(RoomId(7)),
            timer: true,
            phase: 0,
        }
    );
    assert_eq!(ctl.timers.len(), 1);
    assert_eq!(
        (ctl.timers[0].func, ctl.timers[0].period),
        (TimerFn::Act2(Timer::Summoner), 3)
    );
    // Edge case 14: the party iterate sets A2Q5 (slot 13) bits.
    assert_eq!(f.flags(P1).word(13), 1 << 13 | 1 << 1);
    assert_eq!(f.flags(P3).word(13), 1 << 13 | 1 << 1);
    assert_eq!(f.flags(P1).word(12), 0);
    // Completion flag: P2 gets 13.14 and `5D 0C 00 0C 0000`.
    assert_eq!(f.flags(P2).word(13), 1 << 14);
    assert_eq!(f.sent[0], (P2, hex("5d 0c 00 0c 0000")));
    // FX 7: every player 0x28 then `89 07`.
    let rest: Vec<(UnitId, u8)> = f.sent[1..].iter().map(|m| (m.0, m.1[0])).collect();
    assert_eq!(
        rest,
        [
            (P1, 0x28),
            (P1, 0x89),
            (P2, 0x28),
            (P2, 0x89),
            (P3, 0x28),
            (P3, 0x89)
        ]
    );
    assert_eq!(f.sent[2].1, [0x89, 7]);
    assert_eq!(ctl.fx, 7);
    // A party member outside Act II is skipped.
    let (mut ctl, _) = control();
    let mut f = fake(3);
    f.near = vec![P1];
    f.party.insert(P1, vec![P1, P3]);
    f.p(P3).act = Some(0);
    kill(&mut ctl, &mut f);
    assert_eq!(f.flags(P3).word(13), 1 << 14);
    // A second kill adds no timer, but +0x09 := 0 again (on every
    // not-intro kill, act2-2 §1 item 8).
    ctl.record_mut(12).unwrap().extra.a2.q5.phase = 1;
    kill(&mut ctl, &mut f);
    assert_eq!(ctl.timers.len(), 1);
    assert_eq!(x(&ctl).phase, 0);
}

// Covers: specs/world/quests-act2.md §7.2
#[test]
fn summoner_killed_in_intro_game() {
    let (mut ctl, _) = control();
    let mut f = fake(1);
    f.near = vec![P1];
    ctl.record_mut(12).unwrap().not_intro = false;
    kill(&mut ctl, &mut f);
    assert_eq!(*x(&ctl), q5::Extra::default());
    assert_eq!(ctl.record(12).unwrap().state, 0);
    assert!(ctl.timers.is_empty());
    assert_eq!(f.flags(P1).word(13), 0);
    assert_eq!(f.sent_ids(), [0x28, 0x89]);
}

// Covers: specs/world/quests-act2.md §7.2
#[test]
fn summoner_timer_two_phases() {
    let (mut ctl, _) = control();
    let mut f = fake(3);
    f.near = vec![P1, P2];
    kill(&mut ctl, &mut f);
    // P2 left for the Arcane Sanctuary; P1 is in Lut Gholein.
    f.p(P2).level = Some(74);
    f.p(P3).level = Some(74);
    f.sent.clear();
    // Due 3 ticks later: runs on the 4th update.
    for _ in 0..3 {
        ctl.update(&mut f);
    }
    assert!(f.log.is_empty());
    ctl.update(&mut f);
    assert_eq!(f.log, ["sound 2 51"]);
    assert_eq!(x(&ctl).phase, 1);
    assert_eq!(ctl.timers.len(), 1);
    assert!(f.sent.is_empty());
    for _ in 0..4 {
        ctl.update(&mut f);
    }
    // Status 4 to all: the default rule reports 12 for a state past
    // init_no without 13.13 (P3), the status for 13.13 (P1, P2).
    assert!(ctl.timers.is_empty());
    assert!(!x(&ctl).timer);
    assert_eq!(ctl.record(12).unwrap().status, 4);
    assert_eq!(
        f.sent,
        [
            (P1, hex("5d 0c 00 04 0000")),
            (P2, hex("5d 0c 00 04 0000")),
            (P3, hex("5d 0c 00 0c 0000"))
        ]
    );
}

fn say(ctl: &mut QuestControl, f: &mut Fake, m: u32) {
    ev(
        ctl,
        f,
        EventArgs {
            event: event::SCROLL_MESSAGE,
            target: Some(GREIZ_U),
            player: Some(P1),
            a: 198,
            b: m,
        },
    );
}

// Covers: specs/world/quests-act2.md §7.2
#[test]
fn final_messages() {
    let (mut ctl, _) = control();
    let mut f = fake(1);
    // Outside 419–429: nothing.
    set(&mut f, P1, 1);
    say(&mut ctl, &mut f, 418);
    say(&mut ctl, &mut f, 430);
    assert!(f.sent.is_empty());
    // 13.1 without 13.13: GUID, 13.0, clear 13.1, refresh.
    say(&mut ctl, &mut f, 419);
    assert_eq!(f.flags(P1).word(13), 1);
    assert!(ctl.record(12).unwrap().guids.contains(1));
    assert_eq!(ctl.record(12).unwrap().state, 0);
    assert_eq!(f.sent_ids(), [0x27, 0x29]);
    // 13.1 and 13.13: status 13 to all, game 13.13, state 3.
    let (mut ctl, _) = control();
    let mut f = fake(1);
    set(&mut f, P1, 1);
    set(&mut f, P1, 13);
    ctl.record_mut(12).unwrap().state = 2;
    say(&mut ctl, &mut f, 429);
    let r = ctl.record(12).unwrap();
    assert_eq!((r.status, r.state), (13, 3));
    assert!(ctl.game.get(13, 13));
    assert_eq!(f.flags(P1).word(13), 1 | 1 << 13);
    assert_eq!(f.sent[0], (P1, hex("5d 0c 00 0d 0000")));
    assert_eq!(f.sent_ids(), [0x5D, 0x27, 0x29]);
    // Without 13.1: only the refresh.
    let (mut ctl, _) = control();
    let mut f = fake(1);
    say(&mut ctl, &mut f, 425);
    assert_eq!(f.flags(P1).word(13), 0);
    assert!(ctl.record(12).unwrap().guids.0.is_empty());
    assert_eq!(f.sent_ids(), [0x27, 0x29]);
}

// Covers: specs/world/quests-act2.md §7.2, §7.1
// Covers: specs/world/quests-act2-2.md §1 r2, §1 r6
#[test]
fn level_start_and_leave() {
    let (mut ctl, _) = control();
    let mut f = fake(2);
    ctl.record_mut(12).unwrap().guids.add(1);
    ctl.record_mut(12).unwrap().guids.add(2);
    let lvl = |a, b| EventArgs {
        event: event::CHANGED_LEVEL,
        target: Some(P1),
        player: Some(P1),
        a,
        b,
    };
    // Old 40 and new ≥ 40 only.
    ev(&mut ctl, &mut f, lvl(40, 39));
    ev(&mut ctl, &mut f, lvl(41, 40));
    assert_eq!(ctl.record(12).unwrap().guids.0, [1, 2]);
    ctl.record_mut(12).unwrap().not_intro = false;
    ev(&mut ctl, &mut f, lvl(40, 41));
    assert_eq!(ctl.record(12).unwrap().guids.0, [1, 2]);
    ctl.record_mut(12).unwrap().not_intro = true;
    ev(&mut ctl, &mut f, lvl(40, 41));
    assert_eq!(ctl.record(12).unwrap().guids.0, [2]);
    // Event 10 (`0x00545530` needs 13.0 and 13.1, act2-2 §1 item 2).
    f.p(P2).quests.flags[0].set(13, 0);
    f.p(P2).quests.flags[0].set(13, 1);
    ev(
        &mut ctl,
        &mut f,
        EventArgs {
            event: event::PLAYER_LEAVES_GAME,
            target: Some(P2),
            player: Some(P2),
            ..EventArgs::default()
        },
    );
    assert!(ctl.record(12).unwrap().guids.0.is_empty());
    // Event 13: 13.2 without 13.0, 13.15 → status 2, state 1.
    let start = |ctl: &mut QuestControl, f: &mut Fake| {
        ev(
            ctl,
            f,
            EventArgs {
                event: event::PLAYER_STARTED_GAME,
                target: Some(P1),
                player: Some(P1),
                ..EventArgs::default()
            },
        );
    };
    for (bits, want) in [
        (&[2][..], (2, 1)),
        (&[2, 0][..], (0, 0)),
        (&[2, 15][..], (0, 0)),
        (&[][..], (0, 0)),
    ] {
        let (mut ctl, _) = control();
        let mut f = fake(1);
        for &b in bits {
            set(&mut f, P1, b);
        }
        start(&mut ctl, &mut f);
        let r = ctl.record(12).unwrap();
        assert_eq!((r.status, r.state), want, "{bits:?}");
    }
    assert!(f.log.is_empty());
}

// Spec: specs/world/quests-act5.md §1.3, §3 (Test vectors, Edge cases)
//! A5Q1 Siege on Harrogath and the Act V sequence chain on the quests'
//! fake world.

use super::super::super::tests::*;
use super::super::super::{
    act1, bit, event, EventArgs, QuestChain, QuestControl, QuestError, QuestFlags, TextList,
    UnitKind,
};
use super::{
    larzuk_dummy_init, larzuk_map_ai, shenk_activated, socket_reward, CHAIN, LARZUK, SLOT,
};
use crate::units::{RoomId, UnitId};

pub(super) const LARZUK_U: UnitId = UnitId(0x40);
const MALAH_U: UnitId = UnitId(0x41);
const SHENK_U: UnitId = UnitId(0x42);

fn kind(class: u16, su: Option<u32>) -> UnitKind {
    UnitKind::Monster {
        class: u32::from(class),
        superunique: su,
        owner: None,
    }
}

/// The fake with P1 in Harrogath (Act V) and Larzuk, Malah and Shenk.
fn fake() -> Fake {
    let mut f = Fake::new();
    f.p(P1).act = Some(4);
    f.p(P1).level = Some(109);
    for (u, class, su) in [
        (LARZUK_U, LARZUK, None),
        (MALAH_U, 513, None),
        (SHENK_U, 300, Some(42)),
    ] {
        f.monsters.insert(u, (u.0, class, kind(class, su)));
    }
    f
}

fn add_player(f: &mut Fake, u: UnitId, act: u8) {
    f.players.insert(
        u,
        Player {
            guid: u.0,
            act: Some(act),
            level: Some(110),
            ..Player::default()
        },
    );
}

fn ev(ctl: &mut QuestControl, f: &mut Fake, chain: u8, args: EventArgs) {
    let i = ctl.find(chain).unwrap();
    act1::callback(ctl, f, i, args, None, false);
}

fn text(ctl: &mut QuestControl, f: &mut Fake, npc: UnitId) -> TextList {
    let mut list = TextList::new();
    let args = EventArgs {
        event: event::NPC_ACTIVATE,
        target: Some(npc),
        player: Some(P1),
        ..EventArgs::default()
    };
    let i = ctl.find(CHAIN).unwrap();
    act1::callback(ctl, f, i, args, Some(&mut list), false);
    list
}

fn say(ctl: &mut QuestControl, f: &mut Fake, msg: u32) {
    let args = EventArgs {
        event: event::SCROLL_MESSAGE,
        target: Some(LARZUK_U),
        player: Some(P1),
        a: u32::from(LARZUK),
        b: msg,
    };
    ev(ctl, f, CHAIN, args);
}

fn status(ctl: &QuestControl, f: &mut Fake, pf: &QuestFlags) -> Option<u8> {
    let i = ctl.find(CHAIN).unwrap();
    super::status(ctl, f, i, P1, pf, 0x0058_6CE0)
}

fn fx(bits: &[u8]) -> QuestFlags {
    let mut q = QuestFlags::default();
    for &b in bits {
        q.set(SLOT, b);
    }
    q
}

// ------------------------------------------------------------ §1.3

// Covers: specs/world/quests-act5.md §1.3
#[test]
fn sequence_chain_31_to_34() {
    let (mut ctl, _) = control();
    let mut f = fake();
    // Chain 31 open: 1, no state change, the walk stops.
    assert!(super::super::sequence(&mut ctl, &mut f, 31));
    assert_eq!(ctl.record(31).unwrap().state, 0);
    assert_eq!(ctl.record(32).unwrap().state, 0);
    // Siege done: chain 32 opens (state 0 → 1) and holds.
    ctl.record_mut(31).unwrap().state = 5;
    assert!(super::super::sequence(&mut ctl, &mut f, 31));
    assert_eq!(ctl.record(32).unwrap().state, 1);
    assert_eq!(ctl.record(33).unwrap().state, 0);
    // Rescue done: chain 33 opens; Prison done (state ≥ 5): chain 34.
    ctl.record_mut(32).unwrap().state = 5;
    assert!(super::super::sequence(&mut ctl, &mut f, 31));
    assert_eq!(ctl.record(33).unwrap().state, 1);
    ctl.record_mut(33).unwrap().state = 6;
    assert!(super::super::sequence(&mut ctl, &mut f, 31));
    assert_eq!(ctl.record(34).unwrap().state, 1);
    // Switched off (intro): the walk passes on without state changes.
    let (mut ctl, _) = control();
    ctl.record_mut(31).unwrap().not_intro = false;
    ctl.record_mut(32).unwrap().not_intro = false;
    assert!(super::super::sequence(&mut ctl, &mut f, 31));
    assert_eq!(ctl.record(32).unwrap().state, 0);
    assert_eq!(ctl.record(33).unwrap().state, 1);
    // Chain 32 open but not at state 0: 1, unchanged.
    let (mut ctl, _) = control();
    ctl.record_mut(32).unwrap().state = 3;
    assert!(super::super::sequence(&mut ctl, &mut f, 32));
    assert_eq!(ctl.record(32).unwrap().state, 3);
    // An absent chain ends the walk with 0.
    let (mut ctl, _) = control();
    ctl.record_mut(31).unwrap().state = 5;
    let j = ctl.find(32).unwrap();
    ctl.records.remove(j);
    assert!(!super::super::sequence(&mut ctl, &mut f, 31));
}

// ------------------------------------------------------------ §3

// Covers: specs/world/quests-act5.md §3.4, §3.1
#[test]
fn vector_larzuk_20090_rewards() {
    // Test vector: 35.1 + 35.13, state 3 → bits 2–11 reset, 35.5, state
    // 5, status 13 to qualifying players, GUID added.
    let (mut ctl, _) = control();
    let mut f = fake();
    add_player(&mut f, P2, 4);
    for b in [1, 13, 2, 3, 4, 11] {
        f.p(P1).quests.flags[0].set(SLOT, b);
    }
    f.p(P2).quests.flags[0].set(SLOT, bit::COMPLETED_BEFORE);
    ctl.record_mut(CHAIN).unwrap().state = 3;
    say(&mut ctl, &mut f, 20090);
    let w = f.flags(P1).word(SLOT);
    assert_eq!(w, 1 << 1 | 1 << 5 | 1 << 13);
    let r = ctl.record(CHAIN).unwrap();
    assert_eq!((r.state, r.status, r.flags), (5, 13, 0));
    assert!(r.extra.a5.q1.reward_talk);
    assert_eq!(r.guids.0, [1]);
    // The own sequence function opened chain 32.
    assert_eq!(ctl.record(32).unwrap().state, 1);
    // Status 13 to all: P1 qualifies by 35.13 (its status fn: 4 for
    // 35.5); P2 has 35.15 only and gets nothing. No refresh.
    assert_eq!(f.sent, [(P1, hex("5D 1F 00 04 0000"))]);
    // Without 35.1: nothing at all.
    let (mut ctl, _) = control();
    let mut f = fake();
    say(&mut ctl, &mut f, 20090);
    assert!(f.sent.is_empty() && ctl.record(CHAIN).unwrap().guids.0.is_empty());
    assert!(!ctl.record(CHAIN).unwrap().extra.a5.q1.reward_talk);
    // 35.1 without 35.13: bits only, state kept.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.p(P1).quests.flags[0].set(SLOT, 1);
    ctl.record_mut(CHAIN).unwrap().state = 3;
    say(&mut ctl, &mut f, 20090);
    assert_eq!(ctl.record(CHAIN).unwrap().state, 3);
    assert!(f.flags(P1).get(SLOT, 5) && f.sent.is_empty());
}

// Covers: specs/world/quests-act5.md §3.4, §3.2
#[test]
fn larzuk_20077_and_chat_end() {
    let (mut ctl, _) = control();
    let mut f = fake();
    add_player(&mut f, P2, 4);
    f.p(P2).quests.flags[0].set(SLOT, bit::REWARD_GRANTED);
    ctl.record_mut(CHAIN).unwrap().state = 1;
    say(&mut ctl, &mut f, 20077);
    let r = ctl.record(CHAIN).unwrap();
    assert!(r.state == 2 && r.extra.a5.q1.started);
    // Flag iterate: state 2 → 35.2 (P2 has 35.0: skipped).
    assert!(f.flags(P1).get(SLOT, 2) && !f.flags(P2).get(SLOT, 2));
    // Refresh: the 0x27 list of state 2 for Larzuk, then 0x29.
    assert_eq!(f.sent_ids(), [0x27, 0x29]);
    assert!(f.log.iter().any(|l| l.starts_with("0x27 64 [(20078, 2)")));
    // Chat end with Larzuk: status 1 to all, pending cleared.
    f.sent.clear();
    let end = EventArgs {
        event: event::NPC_DEACTIVATE,
        target: Some(LARZUK_U),
        player: Some(P1),
        ..EventArgs::default()
    };
    ev(&mut ctl, &mut f, CHAIN, end);
    assert_eq!(f.sent, [(P1, hex("5D 1F 00 01 0000"))]);
    assert!(!ctl.record(CHAIN).unwrap().extra.a5.q1.started);
    // Again: nothing; reward talk pending → status 4.
    f.sent.clear();
    ev(&mut ctl, &mut f, CHAIN, end);
    assert!(f.sent.is_empty());
    ctl.record_mut(CHAIN).unwrap().extra.a5.q1.reward_talk = true;
    ev(&mut ctl, &mut f, CHAIN, end);
    assert_eq!(ctl.record(CHAIN).unwrap().status, 4);
    assert!(!ctl.record(CHAIN).unwrap().extra.a5.q1.reward_talk);
    // Another NPC: nothing.
    ctl.record_mut(CHAIN).unwrap().extra.a5.q1.started = true;
    ev(
        &mut ctl,
        &mut f,
        CHAIN,
        EventArgs {
            target: Some(MALAH_U),
            ..end
        },
    );
    assert!(ctl.record(CHAIN).unwrap().extra.a5.q1.started);
    // Flag iterate at state 3: 35.4 with status 2 (Shenk seen).
    let (mut ctl, _) = control();
    let mut g = fake();
    let r = ctl.record_mut(CHAIN).unwrap();
    (r.state, r.status) = (2, 2);
    let args = EventArgs {
        event: event::CHANGED_LEVEL,
        player: Some(P1),
        a: 110,
        b: 111,
        ..EventArgs::default()
    };
    ev(&mut ctl, &mut g, CHAIN, args);
    assert!(g.flags(P1).get(SLOT, 4) && !g.flags(P1).get(SLOT, 3));
}

// Covers: specs/world/quests-act5.md §3.3, §1.2
#[test]
fn chat_tables_and_active() {
    let (mut ctl, _) = control();
    let mut f = fake();
    // State 0: index −1 → nothing.
    assert!(text(&mut ctl, &mut f, LARZUK_U).is_empty());
    ctl.record_mut(CHAIN).unwrap().state = 1;
    assert_eq!(text(&mut ctl, &mut f, LARZUK_U), [(20077, 0)]);
    assert!(text(&mut ctl, &mut f, MALAH_U).is_empty());
    ctl.record_mut(CHAIN).unwrap().state = 2;
    assert_eq!(text(&mut ctl, &mut f, MALAH_U), [(20081, 2)]);
    // States ≥ 4 need 35.13.
    ctl.record_mut(CHAIN).unwrap().state = 4;
    assert!(text(&mut ctl, &mut f, LARZUK_U).is_empty());
    f.p(P1).quests.flags[0].set(SLOT, 13);
    assert_eq!(text(&mut ctl, &mut f, LARZUK_U), [(20090, 0)]);
    ctl.record_mut(CHAIN).unwrap().state = 5;
    assert_eq!(text(&mut ctl, &mut f, LARZUK_U), [(20090, 2)]);
    // Past the index table: nothing.
    ctl.record_mut(CHAIN).unwrap().state = 6;
    assert!(text(&mut ctl, &mut f, LARZUK_U).is_empty());
    // 35.1 set: table state 3 until 35.5.
    f.p(P1).quests.flags[0].set(SLOT, 1);
    assert_eq!(text(&mut ctl, &mut f, LARZUK_U), [(20090, 0)]);
    f.p(P1).quests.flags[0].set(SLOT, 5);
    assert!(text(&mut ctl, &mut f, LARZUK_U).is_empty());
    // 35.0: nothing; intro: nothing.
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().state = 1;
    f.p(P1).quests.flags[0].set(SLOT, 0);
    assert!(text(&mut ctl, &mut f, LARZUK_U).is_empty());
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().not_intro = false;
    assert!(text(&mut ctl, &mut f, LARZUK_U).is_empty());
    // Active: Larzuk at state 1 lacking 35.0 / 35.1, or 35.1 without
    // 35.5.
    let (mut ctl, _) = control();
    let i = ctl.find(CHAIN).unwrap();
    let mut f = fake();
    let act =
        |ctl: &QuestControl, f: &mut Fake, class| super::active(ctl, f, i, P1, class, 0x0058_74E0);
    assert!(!act(&ctl, &mut f, LARZUK));
    ctl.records[i].state = 1;
    assert!(act(&ctl, &mut f, LARZUK) && !act(&ctl, &mut f, 513));
    ctl.records[i].not_intro = false;
    assert!(!act(&ctl, &mut f, LARZUK));
    f.p(P1).quests.flags[0].set(SLOT, 1);
    assert!(act(&ctl, &mut f, LARZUK));
    f.p(P1).quests.flags[0].set(SLOT, 5);
    assert!(!act(&ctl, &mut f, LARZUK));
}

// Covers: specs/world/quests-act5.md §3.5, §1.1
#[test]
fn level_changes() {
    let lvl = |ctl: &mut QuestControl, f: &mut Fake, a, b| {
        let args = EventArgs {
            event: event::CHANGED_LEVEL,
            player: Some(P1),
            a,
            b,
            ..EventArgs::default()
        };
        ev(ctl, f, CHAIN, args);
    };
    // New level 110–112, state 1 → 3 and the flag iterate (35.3).
    for new in [110, 111, 112] {
        let (mut ctl, _) = control();
        let mut f = fake();
        ctl.record_mut(CHAIN).unwrap().state = 1;
        lvl(&mut ctl, &mut f, 113, new);
        assert_eq!(ctl.record(CHAIN).unwrap().state, 3);
        assert!(f.flags(P1).get(SLOT, 3) && f.sent.is_empty());
    }
    // Leaving Harrogath at state 2: quick remove, state 3, status 1 to
    // all, flag iterate.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().state = 2;
    ctl.record_mut(CHAIN).unwrap().guids.add(1);
    lvl(&mut ctl, &mut f, 109, 113);
    let r = ctl.record(CHAIN).unwrap();
    assert_eq!((r.state, r.status), (3, 1));
    assert!(r.guids.0.is_empty() && f.flags(P1).get(SLOT, 3));
    assert_eq!(f.sent, [(P1, hex("5D 1F 00 01 0000"))]);
    // Status already set: no message; a player with 35.1 keeps state 2.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.p(P1).quests.flags[0].set(SLOT, 1);
    let r = ctl.record_mut(CHAIN).unwrap();
    (r.state, r.status) = (2, 1);
    lvl(&mut ctl, &mut f, 109, 113);
    assert_eq!(ctl.record(CHAIN).unwrap().state, 2);
    assert!(f.sent.is_empty());
    // State 0 entering level 111 from 109: the first test fails, the
    // Harrogath branch runs but its tail needs state 2: nothing is sent
    // (REC-3100, a5-wp-34).
    let (mut ctl, _) = control();
    let mut f = fake();
    lvl(&mut ctl, &mut f, 109, 111);
    let r = ctl.record(CHAIN).unwrap();
    assert_eq!((r.state, r.status), (0, 0));
    assert!(f.sent.is_empty());
    // Intro: level 111 does not move the state.
    let (mut ctl, _) = control();
    let mut f = fake();
    let r = ctl.record_mut(CHAIN).unwrap();
    (r.state, r.not_intro) = (1, false);
    lvl(&mut ctl, &mut f, 110, 111);
    assert_eq!(ctl.record(CHAIN).unwrap().state, 1);
    // `0x005873E0` tests the new level 110–112 before it needs a player:
    // without one, state 1 still moves to 3; the 109 branch needs one
    // (REC-3100).
    let no_player = |a, b| EventArgs {
        event: event::CHANGED_LEVEL,
        player: None,
        a,
        b,
        ..EventArgs::default()
    };
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().state = 1;
    ev(&mut ctl, &mut f, CHAIN, no_player(109, 110));
    assert_eq!(ctl.record(CHAIN).unwrap().state, 3);
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().state = 2;
    ev(&mut ctl, &mut f, CHAIN, no_player(109, 113));
    assert_eq!(ctl.record(CHAIN).unwrap().state, 2);
    assert!(f.sent.is_empty());
}

// Covers: specs/world/quests-act5.md §3.6, §edge-cases-original-bugs r1
#[test]
fn shenk_death() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let p3 = UnitId(3);
    add_player(&mut f, P2, 4);
    add_player(&mut f, p3, 4);
    f.pos.insert(SHENK_U, (10, 20, RoomId(7)));
    f.near = vec![P1];
    f.party.insert(P1, vec![P2]);
    // Any victim on the chain-31 link (edge case 1): a Larzuk unit.
    f.chains.insert(LARZUK_U, QuestChain(vec![CHAIN]));
    f.pos.insert(LARZUK_U, (10, 20, RoomId(7)));
    ctl.monster_killed(&mut f, LARZUK_U, Some(P1));
    let r = ctl.record(CHAIN).unwrap();
    assert_eq!(r.extra.a5.q1.kill_room, Some(RoomId(7)));
    for p in [P1, P2] {
        assert!(f.flags(p).get(SLOT, 1) && f.flags(p).get(SLOT, 13), "{p:?}");
    }
    // P3: neither near nor in a party → completion flag (35.14).
    assert!(!f.flags(p3).get(SLOT, 1) && f.flags(p3).get(SLOT, 14));
    assert!(ctl.game.get(SLOT, 13));
    assert_eq!(ctl.fx, 15);
    assert_eq!(f.log, ["sound 1 80", "sound 2 80"]);
    let to3: Vec<Vec<u8>> = f
        .sent
        .iter()
        .filter(|m| m.0 == p3)
        .map(|m| m.1.clone())
        .collect();
    assert_eq!(to3[0], hex("5D 1F 00 0C 0000"));
    assert!(to3.contains(&hex("89 0F")));
    // Status 3 to all (P3 qualifies by 35.14 with the status fn's 12).
    assert_eq!(r.status, 3);
    assert_eq!(to3.last().unwrap(), &hex("5D 1F 00 0C 0000"));
    assert!(f.sent.contains(&(P1, hex("5D 1F 00 03 0000"))));
    // A party member outside Act V gets nothing.
    let (mut ctl, _) = control();
    let mut f = fake();
    add_player(&mut f, P2, 3);
    f.near = vec![P1];
    f.party.insert(P1, vec![P2]);
    f.chains.insert(SHENK_U, QuestChain(vec![CHAIN]));
    f.pos.insert(SHENK_U, (1, 1, RoomId(1)));
    ctl.monster_killed(&mut f, SHENK_U, Some(P1));
    assert!(!f.flags(P2).get(SLOT, 1) && f.flags(P2).get(SLOT, 14));
    // No victim room, or intro: nothing.
    for intro in [false, true] {
        let (mut ctl, _) = control();
        let mut f = fake();
        f.near = vec![P1];
        f.chains.insert(SHENK_U, QuestChain(vec![CHAIN]));
        if intro {
            ctl.record_mut(CHAIN).unwrap().not_intro = false;
            f.pos.insert(SHENK_U, (1, 1, RoomId(1)));
        }
        ctl.monster_killed(&mut f, SHENK_U, Some(P1));
        assert!(f.sent.is_empty() && f.flags(P1) == QuestFlags::default());
    }
    // Status already ≥ 3: not sent again.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().status = 4;
    f.chains.insert(SHENK_U, QuestChain(vec![CHAIN]));
    f.pos.insert(SHENK_U, (1, 1, RoomId(1)));
    ctl.monster_killed(&mut f, SHENK_U, Some(P1));
    assert_eq!(ctl.record(CHAIN).unwrap().status, 4);
}

// Covers: specs/world/quests-act5.md §3.7
#[test]
fn game_start_and_leave() {
    let start = |bits: &[u8], heard: bool, state: u8| {
        let (mut ctl, _) = control();
        let mut f = fake();
        for &b in bits {
            f.p(P1).quests.flags[0].set(SLOT, b);
        }
        if heard {
            f.p(P1).quests.hear(0, 513);
        }
        ctl.record_mut(CHAIN).unwrap().state = state;
        let args = EventArgs {
            event: event::PLAYER_STARTED_GAME,
            player: Some(P1),
            target: Some(P1),
            ..EventArgs::default()
        };
        ev(&mut ctl, &mut f, CHAIN, args);
        let r = ctl.record(CHAIN).unwrap();
        (r.state, r.status)
    };
    assert_eq!(start(&[0], false, 2), (2, 5));
    assert_eq!(start(&[15], false, 0), (0, 5));
    assert_eq!(start(&[1], false, 0), (0, 5));
    assert_eq!(start(&[4, 3], false, 0), (3, 2));
    assert_eq!(start(&[3, 2], false, 0), (3, 1));
    assert_eq!(start(&[2], false, 0), (2, 1));
    assert_eq!(start(&[], true, 0), (1, 0));
    assert_eq!(start(&[], true, 2), (2, 0));
    assert_eq!(start(&[], false, 0), (0, 0));
    // Event 10: the player's GUID leaves the list.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().guids.add(1);
    ctl.record_mut(CHAIN).unwrap().guids.add(9);
    let args = EventArgs {
        event: event::PLAYER_LEAVES_GAME,
        player: Some(P1),
        target: Some(P1),
        ..EventArgs::default()
    };
    ev(&mut ctl, &mut f, CHAIN, args);
    assert_eq!(ctl.record(CHAIN).unwrap().guids.0, [9]);
}

// Covers: specs/world/quests-act5.md §3.8
#[test]
fn larzuk_spawn_and_map_ai() {
    let dummy = UnitId(0x60);
    let (mut ctl, _) = control();
    let mut f = fake();
    f.pos.insert(dummy, (100, 200, RoomId(3)));
    f.spot = Some((1, 2));
    f.spawns = vec![Some(LARZUK_U)];
    f.a5_map_ai = true;
    // A map AI stored before Larzuk exists is only kept.
    larzuk_map_ai(&mut ctl, &mut f, 0x77);
    assert!(f.log.is_empty());
    larzuk_dummy_init(&mut ctl, &mut f, dummy);
    assert_eq!(
        f.log,
        [
            "spot at 100 200 2 0x100 16 100",
            "spawn 511 101 202 mode 1 r 5",
            "flags 64 0x3000000",
            "map ai 64 0x77",
        ]
    );
    let e = &ctl.record(CHAIN).unwrap().extra.a5.q1;
    assert!(e.larzuk_spawned && e.map_ai_applied && e.larzuk_guid == 0x40);
    // Once only.
    f.log.clear();
    larzuk_dummy_init(&mut ctl, &mut f, dummy);
    larzuk_map_ai(&mut ctl, &mut f, 0x78);
    assert!(f.log.is_empty());
    assert_eq!(ctl.record(CHAIN).unwrap().extra.a5.q1.map_ai, 0x78);
    // Store after the spawn: applied then; a record whose +4 is 0 is
    // not marked applied.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.pos.insert(dummy, (100, 200, RoomId(3)));
    f.spot = Some((0, 0));
    f.spawns = vec![Some(LARZUK_U)];
    larzuk_dummy_init(&mut ctl, &mut f, dummy);
    assert_eq!(f.log.len(), 3);
    larzuk_map_ai(&mut ctl, &mut f, 0x55);
    assert_eq!(f.log[3], "map ai 64 0x55");
    assert!(!ctl.record(CHAIN).unwrap().extra.a5.q1.map_ai_applied);
    f.a5_map_ai = true;
    larzuk_map_ai(&mut ctl, &mut f, 0x55);
    assert!(ctl.record(CHAIN).unwrap().extra.a5.q1.map_ai_applied);
    // No spot or no spawn: nothing set.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.pos.insert(dummy, (100, 200, RoomId(3)));
    larzuk_dummy_init(&mut ctl, &mut f, dummy);
    f.spot = Some((0, 0));
    larzuk_dummy_init(&mut ctl, &mut f, dummy);
    assert!(!ctl.record(CHAIN).unwrap().extra.a5.q1.larzuk_spawned);
    // Shenk activated: status 2 to all for superunique 42 below status
    // 2.
    let (mut ctl, _) = control();
    let mut f = fake();
    shenk_activated(&mut ctl, &mut f, LARZUK_U);
    assert!(f.sent.is_empty());
    shenk_activated(&mut ctl, &mut f, SHENK_U);
    assert_eq!(f.sent, [(P1, hex("5D 1F 00 02 0000"))]);
    f.sent.clear();
    shenk_activated(&mut ctl, &mut f, SHENK_U);
    assert!(f.sent.is_empty());
    ctl.record_mut(CHAIN).unwrap().status = 0;
    ctl.record_mut(CHAIN).unwrap().not_intro = false;
    shenk_activated(&mut ctl, &mut f, SHENK_U);
    assert!(f.sent.is_empty());
}

// Covers: specs/world/quests-act5.md §3.9
#[test]
fn socket_reward_bits() {
    let (mut ctl, _) = control();
    let mut f = fake();
    f.p(P1).quests.flags[0].set(SLOT, 1);
    socket_reward(&mut ctl, &mut f, P1);
    assert!(f.flags(P1).get(SLOT, 0) && !f.flags(P1).get(SLOT, 1));
    assert!(f.sent.is_empty() && ctl.faults.is_empty());
    // No chain-31 record: fatal unless 35.15.
    let j = ctl.find(CHAIN).unwrap();
    ctl.records.remove(j);
    socket_reward(&mut ctl, &mut f, P1);
    assert_eq!(ctl.faults, [QuestError::Fatal(0x0058_77C0)]);
    f.p(P1).quests.flags[0].set(SLOT, 15);
    socket_reward(&mut ctl, &mut f, P1);
    assert_eq!(ctl.faults.len(), 1);
}

// Covers: specs/world/quests-act5.md §3.10
#[test]
fn status_function() {
    let (mut ctl, _) = control();
    let mut f = fake();
    // Test vector: 35.1 set, 35.5 clear → 3.
    assert_eq!(status(&ctl, &mut f, &fx(&[1])), Some(3));
    assert_eq!(status(&ctl, &mut f, &fx(&[13])), Some(3));
    assert_eq!(status(&ctl, &mut f, &fx(&[1, 5])), Some(4));
    assert_eq!(status(&ctl, &mut f, &fx(&[0])), Some(0));
    assert_eq!(status(&ctl, &mut f, &fx(&[14])), Some(12));
    ctl.record_mut(CHAIN).unwrap().status = 2;
    assert_eq!(status(&ctl, &mut f, &fx(&[])), Some(2));
    ctl.record_mut(CHAIN).unwrap().state = 5;
    assert_eq!(status(&ctl, &mut f, &fx(&[])), Some(0));
    ctl.record_mut(CHAIN).unwrap().not_intro = false;
    assert_eq!(status(&ctl, &mut f, &fx(&[14])), Some(0));
    assert_eq!(status(&ctl, &mut f, &fx(&[1, 0])), Some(3));
}

// Covers: specs/world/quests-act5.md §2
#[test]
fn records_init() {
    let (ctl, _) = control();
    // (chain, init_no, seq_id) of the §2 table; state 0 and active.
    for (chain, init_no, seq) in [
        (31, 4, Some(32)),
        (32, 4, Some(33)),
        (33, 5, Some(34)),
        (34, 4, Some(35)),
        (35, 4, Some(36)),
        (36, 4, Some(37)),
    ] {
        let r = ctl.record(chain).unwrap();
        assert_eq!(
            (r.state, r.init_no, r.seq_id),
            (0, init_no, seq),
            "chain {chain}"
        );
        assert!(r.active, "chain {chain}");
    }
    let e = &ctl.record(31).unwrap().extra.a5;
    assert_eq!(e.q1, super::Extra::default());
    let e = &ctl.record(32).unwrap().extra.a5;
    assert!(e.q2.guids.0.is_empty());
    let e = &ctl.record(33).unwrap().extra.a5;
    assert!(e.q3.guids.0.is_empty());
    let e = &ctl.record(34).unwrap().extra.a5;
    assert!(e.q4.guids.0.is_empty());
    // Eve of Destruction: +0x94 := 1, +0x9C := 1, list reset.
    let q6 = &ctl.record(36).unwrap().extra.a5.q6;
    assert_eq!((q6.portal_mode, q6.last_portal_mode), (1, 1));
    assert!(q6.guids.0.is_empty());
    // The intro record: state 0, active, no sequence.
    let r = ctl.record(40).unwrap();
    assert!(r.active && r.state == 0 && r.seq_id.is_none());
}

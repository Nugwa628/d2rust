// Spec: specs/world/quests-act4.md §1.3, §3, §7, §8 (Test vectors, Edge cases)
//! The Act IV sequence chain and A4Q1 The Fallen Angel callback by
//! callback, on the quests' fake world.

use super::super::super::tests::*;
use super::super::super::*;
use super::*;

const P3: UnitId = UnitId(3);
const P4: UnitId = UnitId(4);
const TYRAEL_U: UnitId = UnitId(0x20);
const GHOST_U: UnitId = UnitId(0x21);
const IZUAL_U: UnitId = UnitId(0x22);
const CAIN_U: UnitId = UnitId(0x23);

fn kind(class: u16) -> UnitKind {
    UnitKind::Monster {
        class: u32::from(class),
        superunique: None,
        owner: None,
    }
}

/// Four players in The Pandemonium Fortress (Act IV), Tyrael, Cain,
/// Izual's ghost and Izual (linked to chain 22).
fn fake() -> Fake {
    let mut f = Fake::new();
    for g in 1..=4 {
        f.players.insert(
            UnitId(g),
            Player {
                guid: g,
                act: Some(3),
                level: Some(103),
                ..Player::default()
            },
        );
    }
    for (u, class) in [
        (TYRAEL_U, npc::TYRAEL2),
        (GHOST_U, IZUAL_GHOST),
        (IZUAL_U, 256),
        (CAIN_U, 246),
    ] {
        f.monsters.insert(u, (u.0, class, kind(class)));
    }
    f.chains.insert(IZUAL_U, QuestChain(vec![CHAIN]));
    f
}

fn idx(ctl: &QuestControl) -> usize {
    ctl.find(CHAIN).unwrap()
}

fn chat(ctl: &mut QuestControl, f: &mut Fake, p: UnitId, n: UnitId) -> TextList {
    let mut list = TextList::new();
    ctl.npc_activate(f, p, n, &mut list);
    list
}

fn msg(ctl: &mut QuestControl, f: &mut Fake, p: UnitId, n: UnitId, m: u16) {
    let mut b = vec![0x31];
    b.extend(f.guid(n).to_le_bytes());
    b.extend(m.to_le_bytes());
    b.extend([0, 0]);
    assert_eq!(ctl.quest_message(f, p, &b), 0);
}

// Covers: specs/world/quests-act4.md §1.3, §2
#[test]
fn sequence_chain() {
    let (mut ctl, _) = control();
    let mut f = fake();
    // Init: 22 at state 1, 23 and 24 at 0, all active.
    for c in [21, 22, 23, 24, 29] {
        assert!(ctl.record(c).unwrap().active, "chain {c}");
    }
    assert_eq!(ctl.record(22).unwrap().state, 1);
    // 22 not finished: 1, nothing moves.
    assert!(act1::sequence(&mut ctl, &mut f, 22));
    assert_eq!(ctl.record(24).unwrap().state, 0);
    // 22 finished: 24 0 → 1, returns 1; 23 untouched.
    ctl.record_mut(22).unwrap().state = 5;
    assert!(act1::sequence(&mut ctl, &mut f, 22));
    assert_eq!(
        (ctl.record(24).unwrap().state, ctl.record(23).unwrap().state),
        (1, 0)
    );
    // 24 running: stops there.
    assert!(act1::sequence(&mut ctl, &mut f, 22));
    assert_eq!(ctl.record(23).unwrap().state, 0);
    // 24 finished: 23 0 → 1.
    ctl.record_mut(24).unwrap().state = 5;
    assert!(act1::sequence(&mut ctl, &mut f, 22));
    assert_eq!(ctl.record(23).unwrap().state, 1);
    // 23 always returns 1 and moves only from 0.
    ctl.record_mut(23).unwrap().state = 3;
    assert!(act1::sequence(&mut ctl, &mut f, 23));
    assert_eq!(ctl.record(23).unwrap().state, 3);
    // Switched off (not-intro 0): the walk passes 22 and 24 whatever
    // their state, and 23 stays.
    let (mut ctl, _) = control();
    for c in [22, 24, 23] {
        ctl.record_mut(c).unwrap().not_intro = false;
    }
    assert!(act1::sequence(&mut ctl, &mut f, 22));
    assert_eq!(
        (ctl.record(24).unwrap().state, ctl.record(23).unwrap().state),
        (0, 0)
    );
    // An absent target: 0.
    let (mut ctl, _) = control();
    ctl.record_mut(22).unwrap().state = 5;
    ctl.records.retain(|r| r.chain != 24);
    assert!(!act1::sequence(&mut ctl, &mut f, 22));
    assert!(f.log.is_empty() && ctl.faults.is_empty());
}

/// The quest control without the Tyrael gossip and Terror's End records,
/// so Tyrael's text and the callbacks reached are chain 22's alone.
fn act4_ctl() -> QuestControl {
    let (mut ctl, _) = control();
    ctl.records.retain(|r| !matches!(r.chain, 21 | 23));
    ctl
}

// Covers: specs/world/quests-act4.md §3.3 text, §3.3 r1, §3.3 r2, §3.3 r3, §3.3 r4, §1.2, §edge-cases-original-bugs r4
#[test]
fn chat_lines() {
    let mut ctl = act4_ctl();
    let mut f = fake();
    // State 1: table state 0 (670) at Tyrael; nothing at Cain.
    assert_eq!(chat(&mut ctl, &mut f, P1, TYRAEL_U), [(670, 0)]);
    assert_eq!(chat(&mut ctl, &mut f, P1, CAIN_U), []);
    // The ghost with 25.5 clear: 675, whatever else holds.
    f.p(P1).quests.flags[0].set(25, 0);
    assert_eq!(chat(&mut ctl, &mut f, P1, GHOST_U), [(675, 0)]);
    // 25.0 and state 1: nothing at Tyrael.
    assert_eq!(chat(&mut ctl, &mut f, P1, TYRAEL_U), []);
    // 25.1: table state 3 (676 at Tyrael).
    f.p(P2).quests.flags[0].set(25, 1);
    assert_eq!(chat(&mut ctl, &mut f, P2, TYRAEL_U), [(676, 0)]);
    assert_eq!(chat(&mut ctl, &mut f, P2, CAIN_U), [(677, 2)]);
    // The player's GUID listed: table state 4.
    ctl.record_mut(22).unwrap().guids.add(3);
    assert_eq!(chat(&mut ctl, &mut f, P3, TYRAEL_U), [(676, 2)]);
    // State 4 without 25.13: nothing; with 25.13: index[4] = 3.
    ctl.record_mut(22).unwrap().state = 4;
    assert_eq!(chat(&mut ctl, &mut f, P4, TYRAEL_U), []);
    f.p(P4).quests.flags[0].set(25, 13);
    assert_eq!(chat(&mut ctl, &mut f, P4, TYRAEL_U), [(676, 0)]);
    // State 2, 3: index 1, 2.
    ctl.record_mut(22).unwrap().state = 2;
    assert_eq!(chat(&mut ctl, &mut f, P4, TYRAEL_U), [(671, 2)]);
    ctl.record_mut(22).unwrap().state = 3;
    assert_eq!(chat(&mut ctl, &mut f, P4, CAIN_U), [(674, 2)]);
    // State 0 (index −1) and intro games: nothing.
    ctl.record_mut(22).unwrap().state = 0;
    assert_eq!(chat(&mut ctl, &mut f, P4, TYRAEL_U), []);
    ctl.record_mut(22).unwrap().state = 1;
    ctl.record_mut(22).unwrap().not_intro = false;
    assert_eq!(chat(&mut ctl, &mut f, P4, TYRAEL_U), []);
}

// Covers: specs/world/quests-act4.md §3.3 text
#[test]
fn wants_to_talk() {
    let mut ctl = act4_ctl();
    let mut f = fake();
    let i = idx(&ctl);
    let act = |ctl: &QuestControl, f: &mut Fake, p, n| active(ctl, f, i, p, n, 0x005B_37F0);
    assert!(act(&ctl, &mut f, P1, npc::TYRAEL2));
    assert!(act(&ctl, &mut f, P1, IZUAL_GHOST));
    assert!(!act(&ctl, &mut f, P1, 246));
    ctl.records[i].state = 2;
    assert!(!act(&ctl, &mut f, P1, npc::TYRAEL2));
    f.p(P1).quests.flags[0].set(25, 1);
    assert!(act(&ctl, &mut f, P1, npc::TYRAEL2));
    f.p(P1).quests.flags[0].set(25, 5);
    assert!(!act(&ctl, &mut f, P1, IZUAL_GHOST));
    f.p(P2).quests.flags[0].set(25, 0);
    assert!(!act(&ctl, &mut f, P2, IZUAL_GHOST));
    ctl.records[i].not_intro = false;
    assert!(!act(&ctl, &mut f, P1, npc::TYRAEL2));
    // Through 0x8A: the record answers for Act IV players.
    let mut ctl = act4_ctl();
    ctl.picked = true;
    ctl.npc_wants_interact(&mut f, P3, TYRAEL_U, npc::TYRAEL2, true)
        .unwrap();
    assert_eq!(f.sent, [(P3, hex("8A 01 20000000"))]);
    assert!(f.log.is_empty());
}

// Covers: specs/world/quests-act4.md §3.2, §3.4, §edge-cases-original-bugs r1
#[test]
fn tyrael_starts_the_quest() {
    let mut ctl = act4_ctl();
    let mut f = fake();
    f.p(P4).quests.flags[0].set(25, 0);
    ctl.record_mut(22).unwrap().state = 4; // edge case 1: no guard
    msg(&mut ctl, &mut f, P1, TYRAEL_U, 670);
    let r = ctl.record(22).unwrap();
    assert!(r.state == 2 && r.extra.a4.q1.started);
    for p in [P1, P2, P3] {
        assert!(f.flags(p).get(25, 2));
    }
    assert!(!f.flags(P4).get(25, 2));
    // Refresh: Tyrael's text (state 2 → table state 1), then 0x29.
    assert_eq!(f.log, ["0x27 32 [(671, 2)]"]);
    assert_eq!(f.sent_ids(), [0x27, 0x29]);
    // Chat end with Cain: nothing.
    f.sent.clear();
    ctl.npc_deactivate(&mut f, P1, CAIN_U);
    assert!(f.sent.is_empty());
    // Chat end with Tyrael: status 1 to all (0x5D to players without
    // 25.0), +0x11 := 0, callback 2 removed.
    ctl.record_mut(22).unwrap().flags = 7;
    ctl.npc_deactivate(&mut f, P1, TYRAEL_U);
    let r = ctl.record(22).unwrap();
    assert!(r.status == 1 && r.flags == 0 && !r.extra.a4.q1.started);
    assert!(!r.has_callback(event::NPC_DEACTIVATE));
    let five_d: Vec<UnitId> = f.sent.iter().map(|m| m.0).collect();
    assert_eq!(five_d, [P1, P2, P3]);
    assert_eq!(f.sent[0].1, hex("5D 16 00 01 0000"));
    // State 3 iterate: 25.3, or 25.4 with +0x0C.
    let mut ctl = act4_ctl();
    let mut f = fake();
    ctl.record_mut(22).unwrap().state = 3;
    flag_iterate(&ctl, &mut f, idx(&ctl));
    assert!(f.flags(P1).get(25, 3) && !f.flags(P1).get(25, 4));
    ctl.record_mut(22).unwrap().extra.a4.q1.entered = true;
    flag_iterate(&ctl, &mut f, idx(&ctl));
    assert!(f.flags(P2).get(25, 4));
}

// Covers: specs/world/quests-act4.md §3.4, §3.8, §1.3
#[test]
fn tyrael_reward_vector() {
    // Test vector: 676 with 25.1 and 25.13, state 4.
    let mut ctl = act4_ctl();
    let mut f = fake();
    ctl.record_mut(22).unwrap().state = 4;
    ctl.record_mut(22).unwrap().flags = 9;
    f.p(P1).quests.flags[0].set(25, 1);
    f.p(P1).quests.flags[0].set(25, 13);
    f.p(P1).quests.flags[0].set(25, 3);
    msg(&mut ctl, &mut f, P1, TYRAEL_U, 676);
    let r = ctl.record(22).unwrap();
    assert_eq!((r.state, r.status, r.flags), (5, 13, 0));
    assert!(!r.has_callback(event::NPC_DEACTIVATE));
    assert!(ctl.game.get(25, 13));
    let fl = f.flags(P1);
    assert!(fl.get(25, 0) && !fl.get(25, 1) && !fl.get(25, 3) && fl.get(25, 13));
    assert_eq!(f.players[&P1].stats[&5], 2);
    assert_eq!(f.sent[0], (P1, hex("5D 16 02 00 0000")));
    assert!(ctl.record(22).unwrap().guids.contains(1));
    // Hell's Forge starts.
    assert_eq!(ctl.record(24).unwrap().state, 1);
    // Status 13 is silent: only the 5D 02, the refresh's 0x27 and 0x29.
    assert_eq!(f.sent_ids(), [0x5D, 0x27, 0x29]);
    // Listed now: table state 4.
    assert_eq!(f.log, ["0x27 32 [(676, 2)]"]);

    // 25.1 without 25.13: the reward, no state change.
    let mut ctl = act4_ctl();
    let mut f = fake();
    f.p(P2).quests.flags[0].set(25, 1);
    msg(&mut ctl, &mut f, P2, TYRAEL_U, 676);
    let r = ctl.record(22).unwrap();
    assert!(r.state == 1 && r.status == 0 && r.has_callback(event::NPC_DEACTIVATE));
    assert!(!ctl.game.get(25, 13) && f.flags(P2).get(25, 0));
    assert_eq!(f.players[&P2].stats[&5], 2);
    // Without 25.1: nothing at all.
    let mut ctl = act4_ctl();
    let mut f = fake();
    msg(&mut ctl, &mut f, P2, TYRAEL_U, 676);
    assert!(f.sent.is_empty() && f.log.is_empty());
    assert_eq!(f.flags(P2), QuestFlags::default());
    // 676 from Cain: nothing.
    f.p(P2).quests.flags[0].set(25, 1);
    msg(&mut ctl, &mut f, P2, CAIN_U, 676);
    assert!(f.flags(P2).get(25, 1) && f.sent.is_empty());
}

// Covers: specs/world/quests-act4.md §3.4
#[test]
fn ghost_message() {
    let mut ctl = act4_ctl();
    let mut f = fake();
    ctl.record_mut(22).unwrap().flags = 3;
    msg(&mut ctl, &mut f, P2, GHOST_U, 675);
    let r = ctl.record(22).unwrap();
    assert!(r.extra.a4.q1.ghost_talked && r.extra.a4.q1.ghost_talked_0f);
    assert_eq!((r.status, r.flags), (4, 0));
    assert!(f.flags(P2).get(25, 5));
    assert_eq!(f.sent.len(), 4); // status 4 to the four players
                                 // Status 4 already: no second broadcast.
    f.sent.clear();
    msg(&mut ctl, &mut f, P1, GHOST_U, 675);
    assert!(f.sent.is_empty() && f.flags(P1).get(25, 5));
}

// Covers: specs/world/quests-act4.md §3.5 text, §3.5 r1, §3.5 r2, §3.5 r3, §7, §3.8
#[test]
fn izual_killed_credit_vector() {
    // Test vector: killed by A; B adjacent; C in B's party elsewhere in
    // Act IV; D in no party, in Act I.
    let mut ctl = act4_ctl();
    let mut f = fake();
    let (a, b, c, d) = (P1, P2, P3, P4);
    f.p(c).level = Some(105);
    f.p(d).act = Some(0);
    f.p(d).level = Some(1);
    f.near = vec![b];
    f.party.insert(b, vec![b, c]);
    f.party.insert(c, vec![b, c]);
    for p in [a, b, c, d] {
        f.p(p).quests.flags[0].set(25, 2);
    }
    f.pos.insert(IZUAL_U, (5000, 6000, RoomId(7)));
    ctl.monster_killed(&mut f, IZUAL_U, Some(a));
    let r = ctl.record(22).unwrap();
    assert_eq!(r.state, 4);
    assert!(!r.has_callback(event::NPC_DEACTIVATE) && !r.has_callback(event::MONSTER_KILLED));
    let e = &r.extra.a4.q1;
    assert_eq!(
        (e.unit, e.x, e.y, e.ghost_pending, e.timer),
        (Some(IZUAL_U), 5000, 6000, true, true)
    );
    for p in [a, b, c] {
        let fl = f.flags(p);
        assert!(fl.get(25, 13) && fl.get(25, 1) && !fl.get(25, 14), "{p:?}");
    }
    // A and C reset_progress, B (room pass) not.
    assert!(!f.flags(a).get(25, 2) && f.flags(b).get(25, 2) && !f.flags(c).get(25, 2));
    let fd = f.flags(d);
    assert!(fd.get(25, 14) && !fd.get(25, 13) && !fd.get(25, 1) && fd.get(25, 2));
    assert_eq!(f.sent, [(d, hex("5D 16 00 0C 0000"))]);
    assert_eq!(
        ctl.timers,
        [QuestTimer {
            func: TimerFn::Act4(super::super::Timer::Q1(Timer::Ghost)),
            chain: 22,
            due: 3,
            period: 3,
        }]
    );
    // A second linked kill does nothing (edge case 2).
    f.chains.insert(GHOST_U, QuestChain(vec![CHAIN]));
    f.sent.clear();
    ctl.monster_killed(&mut f, GHOST_U, Some(d));
    assert!(f.sent.is_empty() && !f.flags(d).get(25, 13));
    assert_eq!(ctl.timers.len(), 1);
}

// Covers: specs/world/quests-act4.md §3.5 r1, §3.5 r2
#[test]
fn izual_killed_skips_credited_players() {
    let mut ctl = act4_ctl();
    let mut f = fake();
    f.p(P1).quests.flags[0].set(25, 0);
    f.p(P2).quests.flags[0].set(25, 1);
    f.p(P2).quests.flags[0].set(25, 3);
    f.near = vec![P1, P2];
    f.party.insert(P3, vec![P3, P2]);
    f.p(P3).quests.flags[0].set(25, 13); // credited earlier
    ctl.monster_killed(&mut f, IZUAL_U, Some(P2));
    assert!(!f.flags(P1).get(25, 13) && !f.flags(P1).get(25, 14));
    // P2 already pending: no reset, no second credit.
    assert!(f.flags(P2).get(25, 3) && !f.flags(P2).get(25, 13));
    // P3's party pass skips P2; P4 gets 25.14.
    assert!(f.flags(P4).get(25, 14));
    // No position for the victim: +0x04 / +0x08 stay 0.
    let e = &ctl.record(22).unwrap().extra.a4.q1;
    assert_eq!((e.x, e.y), (0, 0));
}

// Covers: specs/world/quests-act4.md §3.5 r3, §edge-cases-original-bugs r2, §edge-cases-original-bugs r3
#[test]
fn izual_killed_intro_and_inactive() {
    // Intro game: no credit, no ghost, but the timer.
    let mut ctl = act4_ctl();
    let mut f = fake();
    ctl.record_mut(22).unwrap().not_intro = false;
    ctl.monster_killed(&mut f, IZUAL_U, Some(P1));
    let r = ctl.record(22).unwrap();
    assert!(r.state == 4 && !r.extra.a4.q1.ghost_pending && r.extra.a4.q1.timer);
    assert_eq!(f.flags(P1), QuestFlags::default());
    assert!(f.sent.is_empty());
    assert_eq!(ctl.timers.len(), 1);
    f.a4_room = Some(RoomId(7));
    for _ in 0..4 {
        ctl.update(&mut f);
    }
    assert!(ctl.timers.is_empty());
    assert!(!f.log.iter().any(|l| l.starts_with("spawn")));
    // An inactive record (switched off): Izual is not a forced kill.
    let mut ctl = act4_ctl();
    let mut f = fake();
    ctl.record_mut(22).unwrap().active = false;
    ctl.monster_killed(&mut f, IZUAL_U, Some(P1));
    assert_eq!(ctl.record(22).unwrap().state, 1);
    assert!(ctl.timers.is_empty());
}

// Covers: specs/world/quests-act4.md §3.6
#[test]
fn ghost_timer_spawns_the_ghost() {
    let mut ctl = act4_ctl();
    let mut f = fake();
    f.pos.insert(IZUAL_U, (5000, 6000, RoomId(7)));
    f.near = vec![P1];
    ctl.monster_killed(&mut f, IZUAL_U, Some(P1));
    f.sent.clear();
    f.a4_room = Some(RoomId(9));
    f.spawns = vec![None, None, Some(UnitId(0x99))];
    for _ in 0..3 {
        ctl.update(&mut f);
    }
    assert!(f.log.is_empty() && ctl.timers.len() == 1);
    ctl.update(&mut f);
    assert!(ctl.timers.is_empty());
    let r = ctl.record(22).unwrap();
    assert!(r.status == 3 && !r.extra.a4.q1.ghost_pending && !r.extra.a4.q1.timer);
    assert_eq!(
        f.log,
        [
            "room act 3 5000 6000",
            "spawn 406 5000 6000 mode 8 r 4294967295",
            "spawn 406 5000 6000 mode 8 r 3",
            "spawn 406 5000 6000 mode 8 r 5",
        ]
    );
    // Status 3 to all: P1 (25.13) and the uncredited P2–P4 get 0x5D.
    assert_eq!(f.sent.len(), 4);
    assert_eq!(f.sent[0], (P1, hex("5D 16 00 03 0000")));

    // Status 4 or 13 kept; no room: no spawn.
    let mut ctl = act4_ctl();
    let mut f = fake();
    ctl.monster_killed(&mut f, IZUAL_U, Some(P1));
    ctl.record_mut(22).unwrap().status = 4;
    f.sent.clear();
    for _ in 0..4 {
        ctl.update(&mut f);
    }
    assert_eq!(ctl.record(22).unwrap().status, 4);
    assert!(f.sent.is_empty());
    assert_eq!(f.log, ["room act 3 0 0"]);
    // State not 4 (676 already): no status.
    let mut ctl = act4_ctl();
    let mut f = fake();
    let i = idx(&ctl);
    ctl.records[i].state = 5;
    run_timer(&mut ctl, &mut f, Timer::Ghost, 22);
    assert_eq!(ctl.records[i].status, 0);
    assert!(f.log.is_empty());
}

// Covers: specs/world/quests-act4.md §3.6, §8
#[test]
fn ghost_and_izual_ai_hooks() {
    let mut ctl = act4_ctl();
    let mut f = fake();
    // Not talked to: false, nothing written.
    assert!(!ghost_may_leave(&mut ctl, &mut f, GHOST_U));
    assert_eq!(ctl.record(22).unwrap().extra.a4.q1.unit, None);
    ctl.record_mut(22).unwrap().extra.a4.q1.ghost_talked = true;
    // A player at distance 4: stays.
    f.a4_dist.insert(P2, 4);
    f.a4_dist.insert(P3, 9);
    assert!(!ghost_may_leave(&mut ctl, &mut f, GHOST_U));
    let e = &ctl.record(22).unwrap().extra.a4.q1;
    assert!(e.near && e.unit == Some(GHOST_U));
    // Distance 5: leaves.
    f.a4_dist.insert(P2, 5);
    assert!(ghost_may_leave(&mut ctl, &mut f, GHOST_U));
    assert!(!ctl.record(22).unwrap().extra.a4.q1.near);
    // Removal: sound 74 to players with 25.13.
    f.p(P2).quests.flags[0].set(25, 13);
    f.p(P4).quests.flags[0].set(25, 13);
    ghost_removed(&mut f);
    assert_eq!(f.log, ["sound 2 74", "sound 4 74"]);
    // Izual's first think: status 2 to all while < 2.
    izual_first_think(&mut ctl, &mut f);
    assert_eq!(ctl.record(22).unwrap().status, 2);
    assert_eq!(f.sent.len(), 4);
    f.sent.clear();
    izual_first_think(&mut ctl, &mut f);
    assert!(f.sent.is_empty());
    ctl.record_mut(22).unwrap().status = 0;
    ctl.record_mut(22).unwrap().not_intro = false;
    izual_first_think(&mut ctl, &mut f);
    assert_eq!(ctl.record(22).unwrap().status, 0);
    // Creation links.
    let u = UnitId(0x50);
    f.chains.insert(u, QuestChain::default());
    assert!(!link_izual(&mut ctl, &mut f, u, UBER_IZUAL));
    assert!(link_izual(&mut ctl, &mut f, u, 256));
    assert!(!link_izual(&mut ctl, &mut f, u, 256)); // already linked
    assert_eq!(f.chains[&u].0, [22]);
    let mss = UnitId(0x51);
    f.chains.insert(mss, QuestChain::default());
    assert!(link_soulstone(&mut ctl, &mut f, mss));
    assert_eq!(f.chains[&mss].0, [22]);
}

// Covers: specs/world/quests-act4.md §3.7, §1.1
#[test]
fn level_change_leave_and_start() {
    let mut ctl = act4_ctl();
    let mut f = fake();
    let i = idx(&ctl);
    ctl.records[i].guids.add(1);
    ctl.records[i].guids.add(2);
    ctl.records[i].state = 2;
    // Leaving another level: nothing.
    ctl.changed_level(&mut f, P1, 104, 103);
    assert_eq!(ctl.records[i].state, 2);
    // Leaving town: quick remove, state 3, iterate, status 1 (silent).
    ctl.changed_level(&mut f, P1, 103, 104);
    let r = &ctl.records[i];
    assert!(!r.guids.contains(1) && r.state == 3 && r.status == 1);
    assert!(f.flags(P2).get(25, 3));
    assert!(f.sent.is_empty());
    // Status ≠ 0: kept.
    ctl.records[i].state = 2;
    ctl.records[i].status = 2;
    ctl.changed_level(&mut f, P2, 103, 104);
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (3, 2));
    // A player with 25.0: no state change.
    ctl.records[i].state = 2;
    f.p(P3).quests.flags[0].set(25, 0);
    ctl.changed_level(&mut f, P3, 103, 104);
    assert_eq!(ctl.records[i].state, 2);
    // Event 10.
    ctl.records[i].guids.add(4);
    ctl.player_leaves(&mut f, P4);
    assert!(!ctl.records[i].guids.contains(4));
    // Event 13: 25.4 first, then 25.3, then 25.2; 25.0 / 25.15: nothing.
    for (b, want) in [(4, (3, 2, true)), (3, (3, 1, false)), (2, (2, 1, false))] {
        // The whole game entry (every record present).
        let (mut ctl, _) = control();
        let mut f = fake();
        f.p(P1).quests.flags[0].set(25, b);
        f.p(P1).quests.flags[0].set(25, 2);
        ctl.player_enters(&mut f, P1, 0).unwrap();
        let r = ctl.record(22).unwrap();
        assert_eq!((r.state, r.status, r.extra.a4.q1.entered), want, "bit {b}");
    }
    for b in [0, 15] {
        let mut ctl = act4_ctl();
        let mut f = fake();
        f.p(P1).quests.flags[0].set(25, b);
        f.p(P1).quests.flags[0].set(25, 4);
        let args = EventArgs {
            event: event::PLAYER_STARTED_GAME,
            player: Some(P1),
            ..EventArgs::default()
        };
        let i = idx(&ctl);
        assert!(callback(&mut ctl, &mut f, i, args, None));
        let r = ctl.record(22).unwrap();
        assert_eq!((r.state, r.status), (1, 0), "bit {b}");
    }
}

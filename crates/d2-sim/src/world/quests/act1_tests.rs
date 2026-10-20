// Spec: specs/world/quests.md §8.1, §9.4 (Test vectors); specs/world/quests-act1.md §10.1, §10.3–§10.8
//! The Act I quests callback by callback, the sequence walk, the intro
//! record and the act transitions, from the spec's test vectors and
//! rules, on the quests' fake world.

use super::tests::*;
use super::*;
use crate::units::RoomId as RoomIdT;

const KASHYA_U: UnitId = UnitId(0x13);
const CHARSI_U: UnitId = UnitId(0x14);
const P3: UnitId = UnitId(3);

fn npc_kind(class: u16) -> UnitKind {
    UnitKind::Monster {
        class: u32::from(class),
        superunique: None,
        owner: None,
    }
}

/// The fake with Kashya and Charsi beside Akara, Flavie and Warriv.
fn fake() -> Fake {
    let mut f = Fake::new();
    for (u, class) in [(KASHYA_U, npc::KASHYA), (CHARSI_U, npc::CHARSI)] {
        f.monsters.insert(u, (u.0, class, npc_kind(class)));
    }
    f
}

fn player(guid: u32, level: Option<u32>) -> Player {
    Player {
        guid,
        act: Some(0),
        level,
        ..Player::default()
    }
}

/// Event 0 to one chain's record only: the lines it adds.
fn text(ctl: &mut QuestControl, f: &mut Fake, chain: u8, npc_u: UnitId) -> TextList {
    let mut list = TextList::new();
    let args = EventArgs {
        event: event::NPC_ACTIVATE,
        target: Some(npc_u),
        player: Some(P1),
        ..EventArgs::default()
    };
    let i = ctl.find(chain).unwrap();
    act1::callback(ctl, f, i, args, Some(&mut list), false);
    list
}

/// C→S 0x31 to the NPC unit `n` (GUID = unit id in the fake).
fn say(ctl: &mut QuestControl, f: &mut Fake, n: UnitId, msg: u16) {
    let mut m = vec![0x31];
    m.extend_from_slice(&n.0.to_le_bytes());
    m.extend_from_slice(&msg.to_le_bytes());
    m.extend_from_slice(&[0, 0]);
    assert_eq!(ctl.quest_message(f, P1, &m), 0);
}

fn sent(f: &Fake) -> Vec<(UnitId, Vec<u8>)> {
    f.sent.clone()
}

fn kill(ctl: &mut QuestControl, f: &mut Fake, chain: u8, victim: UnitId, killer: UnitId) {
    f.chains.insert(victim, QuestChain(vec![chain]));
    ctl.monster_killed(f, victim, Some(killer));
}

// ------------------------------------------------------------ §10.1

// Covers: specs/world/quests-act1.md §10.1 r1, §10.1 r2, §10.1 r3
#[test]
fn sequence_walk() {
    // Vector: from chain 1 (state 5), chain 2 state 5, chain 4 state 0
    // → chain 4 state 1; the walk stops (returns 1).
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(1).unwrap().state = 5;
    ctl.record_mut(2).unwrap().state = 5;
    assert!(act1::sequence(&mut ctl, &mut f, 1));
    assert_eq!(ctl.record(4).unwrap().state, 1);
    assert_eq!(ctl.record(3).unwrap().state, 0);
    // Chain 1 short of its pass state: returns 1, nothing moves.
    let (mut ctl, _) = control();
    assert!(act1::sequence(&mut ctl, &mut f, 1));
    assert_eq!(ctl.record(2).unwrap().state, 0);
    // A switched-off quest passes the call on whatever its state: 1 →
    // 2 (off) → 4 (state 6 = its pass state) → 3 opens.
    let (mut ctl, _) = control();
    ctl.record_mut(1).unwrap().state = 5;
    ctl.record_mut(2).unwrap().not_intro = false;
    ctl.record_mut(4).unwrap().state = 6;
    assert!(act1::sequence(&mut ctl, &mut f, 1));
    assert_eq!(ctl.record(2).unwrap().state, 0);
    assert_eq!(ctl.record(3).unwrap().state, 1);
    // Any other state stops the walk: chain 2 at 3.
    let (mut ctl, _) = control();
    ctl.record_mut(1).unwrap().state = 5;
    ctl.record_mut(2).unwrap().state = 3;
    assert!(act1::sequence(&mut ctl, &mut f, 1));
    assert_eq!(ctl.record(4).unwrap().state, 0);
    // Chain 5 below state 2: returns 1 without passing on; chain 6 never
    // passes on: at state 0 it makes its timer `0x00596580` (period 20),
    // which opens it (state 1) at its first firing and is removed.
    let (mut ctl, _) = control();
    ctl.record_mut(3).unwrap().state = 5;
    assert!(act1::sequence(&mut ctl, &mut f, 3));
    assert_eq!(ctl.timers.len(), 1);
    assert_eq!(ctl.timers[0].func, TimerFn::SlaughterOpen);
    for _ in 0..20 {
        ctl.update(&mut f);
    }
    assert_eq!(ctl.record(6).unwrap().state, 0);
    ctl.update(&mut f);
    assert_eq!(ctl.record(6).unwrap().state, 1);
    assert!(ctl.timers.is_empty());
    assert!(act1::sequence(&mut ctl, &mut f, 5));
    assert!(ctl.faults.is_empty());
}

// ------------------------------------------------------------ §10.4

// Covers: specs/world/quests-act1.md §10.4 r1, §10.4 r2, §10.4 r3, §10.4 r4
#[test]
fn den_npc_text() {
    let (mut ctl, _) = control();
    let mut f = fake();
    // Vector: Akara, state 1, not-intro 1, R slot 1 = 0 → message state 0.
    assert_eq!(text(&mut ctl, &mut f, 1, AKARA_U), [(64, 0)]);
    // Vector: R has 1.1 (any state) → message state 3: Akara 76.
    f.p(P1).quests.flags[0].set(1, bit::REWARD_PENDING);
    for s in [0, 4, 5] {
        ctl.record_mut(1).unwrap().state = s;
        assert_eq!(text(&mut ctl, &mut f, 1, AKARA_U), [(76, 0)]);
    }
    // Vector: state 4, R lacks 1.0, 1.1, 1.13, GUID not in the list.
    f.p(P1).quests.flags[0] = QuestFlags::default();
    ctl.record_mut(1).unwrap().state = 4;
    assert!(text(&mut ctl, &mut f, 1, AKARA_U).is_empty());
    // With 1.13, state 4 → message state 3.
    f.p(P1).quests.flags[0].set(1, bit::PRIMARY_GOAL_DONE);
    assert_eq!(text(&mut ctl, &mut f, 1, AKARA_U), [(76, 0)]);
    // GUID in the record's list → message state 4 (Warriv: 80).
    ctl.record_mut(1).unwrap().guids.add(1);
    assert_eq!(text(&mut ctl, &mut f, 1, WARRIV_U), [(80, 2)]);
    // 1.0, or not-intro 0: nothing.
    let (mut ctl, _) = control();
    f.p(P1).quests.flags[0] = QuestFlags::default();
    f.p(P1).quests.flags[0].set(1, bit::REWARD_GRANTED);
    assert!(text(&mut ctl, &mut f, 1, AKARA_U).is_empty());
    f.p(P1).quests.flags[0] = QuestFlags::default();
    ctl.record_mut(1).unwrap().not_intro = false;
    assert!(text(&mut ctl, &mut f, 1, AKARA_U).is_empty());
}

// Covers: specs/world/quests-act1.md §10.4 l2 r1, §10.4 l2 r2, §10.4 l2 r3
#[test]
fn den_level_changes() {
    // Vector: a = 1, b = 2, state 2, status 0, one player with slot 1 =
    // 0x0004 → state 3, slot 1 = 0x0014 (bit 4, bug kept), status 1 and
    // `5d 01 00 01 0000`.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.p(P1).quests.flags[0].set(1, bit::STARTED);
    ctl.record_mut(1).unwrap().state = 2;
    ctl.record_mut(1).unwrap().guids.add(1);
    ctl.changed_level(&mut f, P1, 1, 2);
    let r = ctl.record(1).unwrap();
    assert_eq!((r.state, r.status), (3, 1));
    assert!(!r.has_callback(event::NPC_DEACTIVATE));
    assert!(!r.guids.contains(1));
    assert_eq!(f.flags(P1).word(1), 0x0014);
    assert_eq!(sent(&f), [(P1, hex("5d 01 00 01 0000"))]);
    // Status already 1: bit 3, nothing sent.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(1).unwrap().state = 2;
    ctl.record_mut(1).unwrap().status = 1;
    ctl.changed_level(&mut f, P1, 1, 2);
    assert_eq!(f.flags(P1).word(1), 0x0008);
    assert!(f.sent.is_empty());
    // Entering level 8 from state 1, status 0: state 3, status 2 sent,
    // then I2 sets bit 4; a second entry changes nothing.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.changed_level(&mut f, P1, 2, 8);
    let r = ctl.record(1).unwrap();
    assert_eq!((r.state, r.status), (3, 2));
    assert!(r.extra.entered && !r.has_callback(event::NPC_DEACTIVATE));
    assert_eq!(f.flags(P1).word(1), 0x0010);
    assert_eq!(sent(&f), [(P1, hex("5d 01 00 02 0000"))]);
    f.sent.clear();
    ctl.changed_level(&mut f, P1, 2, 8);
    assert!(f.sent.is_empty());
    // not-intro 0: entering level 8 does nothing.
    let (mut ctl, _) = control();
    ctl.record_mut(1).unwrap().not_intro = false;
    ctl.changed_level(&mut f, P1, 2, 8);
    assert_eq!(ctl.record(1).unwrap().state, 1);
}

// Covers: specs/world/quests-act1.md §10.4 l3 r1, §10.4 l3 r2, §10.4 l3 r3, §10.4 l3 r4, §10.4 l3 r6, §10.4 l3 r7
#[test]
fn den_kills_before_the_clear() {
    // Vector: P = 10, V = 10, spawned 40, killed 37, state 3 → left 3;
    // `5d 01 20 04 0300`; callback 2 null.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(1).unwrap().state = 3;
    f.den = (40, 37, 10, 10);
    kill(&mut ctl, &mut f, 1, UnitId(0x40), P1);
    let r = ctl.record(1).unwrap();
    assert_eq!((r.extra.monsters_left, r.flags, r.status), (3, 0x20, 4));
    assert!(!r.has_callback(event::NPC_DEACTIVATE));
    assert!(r.extra.guids.contains(1));
    assert_eq!(sent(&f), [(P1, hex("5d 01 20 04 0300"))]);
    // Step 7: P > V, status 4 and left > 5: again, one 0x5D per kill.
    f.sent.clear();
    f.den = (40, 30, 5, 10);
    kill(&mut ctl, &mut f, 1, UnitId(0x41), P1);
    assert_eq!(sent(&f), [(P1, hex("5d 01 20 04 0a00"))]);
    // P > V with status 0: nothing sent.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.den = (40, 39, 5, 10);
    kill(&mut ctl, &mut f, 1, UnitId(0x40), P1);
    assert!(f.sent.is_empty());
    assert_eq!(ctl.record(1).unwrap().extra.monsters_left, 1);
    // not-intro 0: nothing at all.
    let (mut ctl, _) = control();
    ctl.record_mut(1).unwrap().not_intro = false;
    kill(&mut ctl, &mut f, 1, UnitId(0x40), P1);
    assert!(ctl.record(1).unwrap().extra.guids.0.is_empty());
}

// Covers: specs/world/quests.md §5 text; specs/world/quests-act1.md §10.4 l3 r5
#[test]
fn den_clear_iterates_and_timer() {
    // Vector: P = V = 10, spawned = killed = 40 → state 4; game 1.13;
    // killers get 13, 1; others 14 + `5d 01 00 0c 0000` + 0x28; `89 00`;
    // timer 8. P2 is the killer's party member in Act I (I3), P3 has no
    // room level (0: no 0x5D).
    let (mut ctl, _) = control();
    let mut f = fake();
    f.players.insert(P2, player(2, Some(8)));
    f.players.insert(P3, player(3, Some(0)));
    f.party.insert(P1, vec![P1, P2]);
    f.den = (40, 40, 10, 10);
    ctl.tick = 100;
    kill(&mut ctl, &mut f, 1, UnitId(0x40), P1);
    let r = ctl.record(1).unwrap();
    assert_eq!(r.state, 4);
    assert!(r.extra.done && r.extra.timer);
    assert!(!r.has_callback(event::MONSTER_KILLED));
    assert!(ctl.game.get(1, bit::PRIMARY_GOAL_DONE));
    assert_eq!(f.flags(P1).word(1), 0x2002);
    assert_eq!(f.flags(P2).word(1), 0x2002);
    assert_eq!(f.flags(P3).word(1), 0x4000);
    assert_eq!(f.log, ["sound 1 35", "sound 2 35"]);
    let ids: Vec<(UnitId, u8)> = f.sent.iter().map(|m| (m.0, m.1[0])).collect();
    assert_eq!(
        ids,
        [
            (P3, 0x28),
            (P1, 0x28),
            (P1, 0x89),
            (P2, 0x28),
            (P2, 0x89),
            (P3, 0x28),
            (P3, 0x89),
        ]
    );
    assert_eq!(ctl.timers.len(), 1);
    // Vector: timer made at updater tick 100, state still 4 → tick 109:
    // broadcast(5, 0), timer removed.
    f.sent.clear();
    for _ in 0..8 {
        ctl.update(&mut f);
    }
    assert!(f.sent.is_empty());
    ctl.update(&mut f);
    assert_eq!(ctl.tick, 109);
    assert_eq!(
        sent(&f),
        [
            (P1, hex("5d 01 00 05 0000")),
            (P2, hex("5d 01 00 05 0000")),
            (P3, hex("5d 01 00 0c 0000")),
        ]
    );
    assert!(ctl.timers.is_empty() && !ctl.record(1).unwrap().extra.timer);
}

// Covers: specs/world/quests-act1.md §10.4 l4 r1, §10.4 l4 r2, §10.4 l4 r3, §10.4 text
#[test]
fn den_akara_messages() {
    // Vector: msg 76 with R slot 1 = 0x2002, state 4, chain 2 state 0
    // not-intro 1 → chain 1 state 5, status 13; chain 2 state 1; R slot 1
    // = 0x2001; slot 41 = 0x2002; stat 5 + 1.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.p(P1).quests.flags[0].set(1, bit::PRIMARY_GOAL_DONE);
    f.p(P1).quests.flags[0].set(1, bit::REWARD_PENDING);
    f.p(P1).quests.flags[0].set(1, bit::ENTER_AREA);
    ctl.record_mut(1).unwrap().state = 4;
    ctl.record_mut(1).unwrap().flags = 0x20;
    say(&mut ctl, &mut f, AKARA_U, 76);
    let r = ctl.record(1).unwrap();
    assert_eq!((r.state, r.status, r.flags), (5, 13, 0));
    assert!(!r.has_callback(event::NPC_DEACTIVATE) && r.guids.contains(1));
    assert_eq!(ctl.record(2).unwrap().state, 1);
    assert_eq!(f.flags(P1).word(1), 0x2001);
    assert_eq!(f.flags(P1).word(41), 0x2002);
    assert_eq!(f.players[&P1].stats[&5], 1);
    // The refresh: 0x27 then 0x29.
    assert_eq!(f.sent_ids(), [0x27, 0x29]);
    // 76 from another NPC, or without 1.1: nothing.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.p(P1).quests.flags[0].set(1, bit::REWARD_PENDING);
    say(&mut ctl, &mut f, KASHYA_U, 76);
    assert_eq!(f.flags(P1).word(1), 0x0002);
    f.p(P1).quests.flags[0] = QuestFlags::default();
    say(&mut ctl, &mut f, AKARA_U, 76);
    assert_eq!(f.flags(P1).word(1), 0);
    assert!(f.sent.is_empty());
    // 1.1 without 1.13: bits and the skill point, state unchanged.
    f.p(P1).quests.flags[0].set(1, bit::REWARD_PENDING);
    say(&mut ctl, &mut f, AKARA_U, 76);
    assert_eq!(ctl.record(1).unwrap().state, 1);
    assert_eq!(f.flags(P1).word(1), 0x0001);
    assert_eq!(f.players[&P1].stats[&5], 1);
    // Message 64 from any state (no guard): state 2, I2 bit 2.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(1).unwrap().state = 4;
    say(&mut ctl, &mut f, AKARA_U, 64);
    let r = ctl.record(1).unwrap();
    assert!(r.state == 2 && r.extra.talked);
    assert_eq!(f.flags(P1).word(1), 0x0004);
    // Chat end with another NPC: nothing; with Akara: status 1 and
    // callback 2 cleared.
    f.sent.clear();
    ctl.npc_deactivate(&mut f, P1, KASHYA_U);
    assert!(f.sent.is_empty());
    ctl.npc_deactivate(&mut f, P1, AKARA_U);
    assert_eq!(sent(&f), [(P1, hex("5d 01 00 01 0000"))]);
    assert!(!ctl.record(1).unwrap().has_callback(event::NPC_DEACTIVATE));
    // Event 10: the player's GUID leaves both lists.
    ctl.record_mut(1).unwrap().guids.add(1);
    ctl.record_mut(1).unwrap().extra.guids.add(1);
    ctl.player_leaves(&mut f, P1);
    assert!(!ctl.record(1).unwrap().guids.contains(1));
    assert!(!ctl.record(1).unwrap().extra.guids.contains(1));
}

// Covers: specs/world/quests.md §6.4; specs/world/quests-act1.md §10.4 text
#[test]
fn den_active_fn() {
    let (ctl, _) = control();
    let mut f = fake();
    let i = ctl.find(1).unwrap();
    let f0 = ctl.records[i].active_fn.unwrap();
    assert!(act1::active_fn(&ctl, &mut f, i, P1, npc::AKARA, f0));
    assert!(!act1::active_fn(&ctl, &mut f, i, P1, npc::KASHYA, f0));
    let (mut ctl, _) = control();
    ctl.record_mut(1).unwrap().state = 2;
    assert!(!act1::active_fn(&ctl, &mut f, i, P1, npc::AKARA, f0));
    f.p(P1).quests.flags[0].set(1, bit::REWARD_PENDING);
    assert!(act1::active_fn(&ctl, &mut f, i, P1, npc::AKARA, f0));
    f.p(P1).quests.flags[0].set(1, bit::REWARD_GRANTED);
    assert!(!act1::active_fn(&ctl, &mut f, i, P1, npc::AKARA, f0));
}

// ------------------------------------------------------------ §10.5 A1Q2

// Covers: specs/world/quests-act1.md §10.5 r1, §10.5 r9
#[test]
fn burial_npc_text_and_active() {
    let (mut ctl, _) = control();
    let mut f = fake();
    assert_eq!(ctl.record(2).unwrap().state, 0);
    assert!(text(&mut ctl, &mut f, 2, KASHYA_U).is_empty());
    for (state, want) in [(1, vec![(81, 0)]), (2, vec![(82, 2)]), (3, vec![(87, 2)])] {
        ctl.record_mut(2).unwrap().state = state;
        assert_eq!(text(&mut ctl, &mut f, 2, KASHYA_U), want);
    }
    ctl.record_mut(2).unwrap().state = 4;
    assert!(text(&mut ctl, &mut f, 2, KASHYA_U).is_empty());
    // No not-intro test.
    ctl.record_mut(2).unwrap().state = 1;
    ctl.record_mut(2).unwrap().not_intro = false;
    assert_eq!(text(&mut ctl, &mut f, 2, KASHYA_U), [(81, 0)]);
    ctl.record_mut(2).unwrap().guids.add(1);
    assert_eq!(text(&mut ctl, &mut f, 2, KASHYA_U), [(92, 2)]);
    f.p(P1).quests.flags[0].set(2, bit::REWARD_PENDING);
    assert_eq!(text(&mut ctl, &mut f, 2, KASHYA_U), [(92, 0)]);
    // Active: Kashya, R lacks 2.0, and 2.1 or (not-intro, state 1).
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = ctl.find(2).unwrap();
    let f0 = ctl.records[i].active_fn.unwrap();
    assert!(!act1::active_fn(&ctl, &mut f, i, P1, npc::KASHYA, f0));
    ctl.records[i].state = 1;
    assert!(act1::active_fn(&ctl, &mut f, i, P1, npc::KASHYA, f0));
    assert!(!act1::active_fn(&ctl, &mut f, i, P1, npc::AKARA, f0));
    f.p(P1).quests.flags[0].set(2, bit::REWARD_GRANTED);
    assert!(!act1::active_fn(&ctl, &mut f, i, P1, npc::KASHYA, f0));
}

// Covers: specs/world/quests-act1.md §10.5 r2, §10.5 r3, §10.5 r6, §10.5 r8
#[test]
fn burial_start_and_area() {
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(2).unwrap().state = 1;
    say(&mut ctl, &mut f, KASHYA_U, 81);
    assert!(ctl.record(2).unwrap().extra.talked);
    assert_eq!(f.flags(P1).word(2), 0x0004);
    f.sent.clear();
    ctl.npc_deactivate(&mut f, P1, KASHYA_U);
    assert_eq!(sent(&f), [(P1, hex("5d 02 00 01 0000"))]);
    let r = ctl.record(2).unwrap();
    assert!(!r.extra.talked && !r.has_callback(event::NPC_DEACTIVATE));
    // Leaving town in state 2: state 3, J2 (status 1 → bit 3), nothing
    // sent.
    f.sent.clear();
    ctl.changed_level(&mut f, P1, 1, 2);
    assert_eq!(ctl.record(2).unwrap().state, 3);
    assert_eq!(f.flags(P1).word(2), 0x000C);
    assert!(f.sent.is_empty());
    // Entering the Burial Grounds (17) with status 1: status 2 through
    // J1, flags as they are, then J2 (bit 4).
    ctl.record_mut(2).unwrap().flags = 0x20;
    ctl.changed_level(&mut f, P1, 2, 17);
    let r = ctl.record(2).unwrap();
    assert_eq!((r.state, r.status, r.flags), (3, 2, 0x20));
    assert_eq!(sent(&f), [(P1, hex("5d 02 20 02 0000"))]);
    assert_eq!(f.flags(P1).word(2), 0x001C);
    // From state 0 (not opened yet): state 3, flags 0.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(2).unwrap().flags = 0x20;
    ctl.changed_level(&mut f, P1, 2, 17);
    let r = ctl.record(2).unwrap();
    assert_eq!((r.state, r.status, r.flags), (3, 2, 0));
    // Event 13 restores from bit 4.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.p(P1).quests.flags[0].set(2, bit::ENTER_AREA);
    ctl.player_enters(&mut f, P1, 0).unwrap();
    let r = ctl.record(2).unwrap();
    assert_eq!((r.state, r.status), (3, 2));
    // Event 10 removes the player's GUID.
    ctl.record_mut(2).unwrap().guids.add(1);
    ctl.player_leaves(&mut f, P1);
    assert!(!ctl.record(2).unwrap().guids.contains(1));
}

// Covers: specs/world/quests-act1.md §10.5 r4, §10.5 r5
#[test]
fn burial_kill_and_timer() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let raven = UnitId(0x40);
    f.monsters.insert(raven, (0x40, 267, npc_kind(267)));
    f.players.insert(P2, player(2, Some(17)));
    f.players.insert(P3, player(3, Some(1)));
    // P1 near Blood Raven; P2 not near but P3's party member... P3 is in
    // P1's party: J7 gives it 13, 1 (room level ≠ 0, Act I).
    f.near = vec![P1];
    f.party.insert(P1, vec![P1, P3]);
    ctl.record_mut(2).unwrap().state = 3;
    ctl.tick = 10;
    kill(&mut ctl, &mut f, 2, raven, P2);
    let r = ctl.record(2).unwrap();
    assert_eq!(r.state, 4);
    assert!(r.extra.killed && r.extra.kill_b1 && r.extra.kill_b2);
    assert_eq!((r.extra.kill_d4, r.extra.victim), (1, 0x40));
    assert!(!r.has_callback(event::NPC_DEACTIVATE) && r.has_callback(event::MONSTER_KILLED));
    assert!(ctl.game.get(2, bit::PRIMARY_GOAL_DONE));
    assert_eq!(f.flags(P1).word(2), 0x2002);
    assert_eq!(f.flags(P3).word(2), 0x2002);
    assert_eq!(f.flags(P2).word(2), 0x4000);
    // J5: 0x5D to P2 only, no 0x28; J6: sound 34 for 13-holders.
    assert_eq!(sent(&f), [(P2, hex("5d 02 00 0c 0000"))]);
    assert_eq!(f.log, ["sound 1 34", "sound 3 34"]);
    assert_eq!(ctl.timers.len(), 1);
    // Timer 15: broadcast(3, 0) at tick 10 + 16, once.
    f.sent.clear();
    for _ in 0..15 {
        ctl.update(&mut f);
    }
    assert!(f.sent.is_empty());
    ctl.update(&mut f);
    assert_eq!(
        sent(&f),
        [
            (P1, hex("5d 02 00 03 0000")),
            (P2, hex("5d 02 00 0c 0000")),
            (P3, hex("5d 02 00 03 0000")),
        ]
    );
    assert!(ctl.timers.is_empty());
    // Callback 8 stays: a second linked death repeats it (a new timer).
    kill(&mut ctl, &mut f, 2, raven, P2);
    assert_eq!(ctl.timers.len(), 1);
    assert!(ctl.faults.is_empty());
    // The victim's GUID must resolve (fatal otherwise).
    let (mut ctl, _) = control();
    let mut f = fake();
    kill(&mut ctl, &mut f, 2, UnitId(0x99), P1);
    assert_eq!(ctl.faults, [QuestError::Fatal(0x0059_0C40)]);
    // not-intro 0: nothing.
    let (mut ctl, _) = control();
    ctl.record_mut(2).unwrap().not_intro = false;
    kill(&mut ctl, &mut f, 2, raven, P1);
    assert_eq!(ctl.record(2).unwrap().state, 0);
}

// Covers: specs/world/quests-act1.md §10.5 r7, §10.2
#[test]
fn burial_reward() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let fl = &mut f.p(P1).quests.flags[0];
    fl.set(2, bit::PRIMARY_GOAL_DONE);
    fl.set(2, bit::REWARD_PENDING);
    fl.set(2, bit::STARTED);
    ctl.record_mut(2).unwrap().state = 4;
    ctl.record_mut(2).unwrap().flags = 0x20;
    say(&mut ctl, &mut f, KASHYA_U, 92);
    let r = ctl.record(2).unwrap();
    assert_eq!((r.state, r.status, r.flags), (5, 13, 0));
    assert!(r.guids.contains(1) && r.has_callback(event::NPC_DEACTIVATE));
    // Sequence: chain 2 at 5 passes to chain 4 (state 0 → 1).
    assert_eq!(ctl.record(4).unwrap().state, 1);
    // Bits 2–11 stay.
    assert_eq!(f.flags(P1).word(2), 0x2005);
    assert_eq!(f.sent_ids(), [0x28, 0x27, 0x29]);
    assert_eq!(f.log.iter().filter(|l| *l == "merc 150").count(), 1);
    // Without 2.13: bits and the mercenary, state unchanged.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.p(P1).quests.flags[0].set(2, bit::REWARD_PENDING);
    say(&mut ctl, &mut f, KASHYA_U, 92);
    assert_eq!(ctl.record(2).unwrap().state, 0);
    assert_eq!(f.flags(P1).word(2), 0x0001);
    assert!(f.log.contains(&"merc 150".to_string()));
}

// ------------------------------------------------------------ §10.5 A1Q3

fn malus_fake() -> (QuestControl, Fake, UnitId) {
    let (ctl, _) = control();
    let mut f = fake();
    let malus = UnitId(0x71);
    f.objects.insert(malus, (0x71, q3::MALUS_OBJECT, 0));
    (ctl, f, malus)
}

use act1::q3;

// Covers: specs/world/quests-act1.md §10.5 l2 r1, §10.5 l2 r2
#[test]
fn malus_object_init_and_operate() {
    let (mut ctl, mut f, malus) = malus_fake();
    act1::malus_init(&mut ctl, &mut f, malus);
    let r = ctl.record(3).unwrap();
    assert!(r.extra.malus_known && r.extra.malus_guid == 0x71);
    assert_eq!(f.log, ["mode 113 0"]);
    // Switched off: the object is shown taken; operating it only sounds.
    let (mut ctl, mut f, malus) = malus_fake();
    ctl.record_mut(3).unwrap().not_intro = false;
    act1::malus_init(&mut ctl, &mut f, malus);
    assert_eq!(ctl.record(3).unwrap().extra.malus_mode, 2);
    assert_eq!(f.objects[&malus].2, 2);
    f.log.clear();
    act1::malus_operate(&mut ctl, &mut f, malus, P1);
    assert_eq!(f.log, ["mode 113 2", "sound 1 19"]);
    // Mode ≠ 0: nothing.
    let (mut ctl, mut f, malus) = malus_fake();
    f.p(P1).stats.insert(12, 8);
    f.objects.get_mut(&malus).unwrap().2 = 2;
    act1::malus_operate(&mut ctl, &mut f, malus, P1);
    assert!(f.log.is_empty());
    // Status already 1: flags kept.
    let (mut ctl, mut f, malus) = malus_fake();
    f.p(P1).stats.insert(12, 8);
    ctl.record_mut(3).unwrap().status = 1;
    ctl.record_mut(3).unwrap().flags = 0x20;
    ctl.record_mut(3).unwrap().state = 4;
    act1::malus_operate(&mut ctl, &mut f, malus, P1);
    assert_eq!(ctl.record(3).unwrap().flags, 0x20);
    // State already 4: no K2.
    assert_eq!(f.flags(P1).word(3), 0);
}

// Covers: specs/world/quests-act1.md §10.5 l2 r3, §10.5 l2 r14
#[test]
fn malus_npc_text_and_active() {
    let (mut ctl, mut f, _) = malus_fake();
    let i = ctl.find(3).unwrap();
    let f0 = ctl.records[i].active_fn.unwrap();
    // State 0: nothing; state 1 → message state 0 (146, menu 1 → 0).
    assert!(text(&mut ctl, &mut f, 3, CHARSI_U).is_empty());
    ctl.record_mut(3).unwrap().state = 1;
    assert_eq!(text(&mut ctl, &mut f, 3, CHARSI_U), [(146, 0)]);
    assert!(act1::active_fn(&ctl, &mut f, i, P1, npc::CHARSI, f0));
    ctl.record_mut(3).unwrap().state = 4;
    assert!(text(&mut ctl, &mut f, 3, CHARSI_U).is_empty());
    assert!(!act1::active_fn(&ctl, &mut f, i, P1, npc::CHARSI, f0));
    // Carrying the Malus at level 8 → message state 3, active.
    f.p(P1).items.push(*b"hdm ");
    f.p(P1).stats.insert(12, 8);
    let want = {
        let mut l = TextList::new();
        let t = ctl.records[i].msgs.unwrap();
        ctl.add_messages(t, &mut l, npc::CHARSI, 3);
        l
    };
    assert!(!want.is_empty());
    assert_eq!(text(&mut ctl, &mut f, 3, CHARSI_U), want);
    assert!(act1::active_fn(&ctl, &mut f, i, P1, npc::CHARSI, f0));
    // Level 7 with the Malus: nothing, not active.
    f.p(P1).stats.insert(12, 7);
    assert!(text(&mut ctl, &mut f, 3, CHARSI_U).is_empty());
    assert!(!act1::active_fn(&ctl, &mut f, i, P1, npc::CHARSI, f0));
    // R has 3.0 but not 3.13: nothing.
    f.p(P1).stats.insert(12, 8);
    f.p(P1).quests.flags[0].set(3, bit::REWARD_GRANTED);
    assert!(text(&mut ctl, &mut f, 3, CHARSI_U).is_empty());
    assert!(!act1::active_fn(&ctl, &mut f, i, P1, npc::CHARSI, f0));
}

// Covers: specs/world/quests-act1.md §10.5 l2 r4, §10.5 l2 r5, §10.5 l2 r6, §10.5 l2 r9
#[test]
fn malus_start_chat_end_and_pick_up() {
    let (mut ctl, mut f, _) = malus_fake();
    say(&mut ctl, &mut f, CHARSI_U, 146);
    assert!(ctl.record(3).unwrap().state == 2 && ctl.record(3).unwrap().extra.talked);
    // No K2 at the message: bit 2 comes with the chat end.
    assert_eq!(f.flags(P1).word(3), 0);
    f.sent.clear();
    ctl.npc_deactivate(&mut f, P1, CHARSI_U);
    assert_eq!(f.flags(P1).word(3), 0x0004);
    assert_eq!(sent(&f), [(P1, hex("5d 03 00 01 0000"))]);
    assert!(!ctl.record(3).unwrap().extra.talked);
    // Leaving town in state 2: state 3, K2 bit 3, callback 3 cleared;
    // status already 1, nothing sent.
    f.sent.clear();
    ctl.changed_level(&mut f, P1, 1, 2);
    let r = ctl.record(3).unwrap();
    assert!(r.state == 3 && !r.has_callback(event::CHANGED_LEVEL));
    assert_eq!(f.flags(P1).word(3), 0x000C);
    assert!(f.sent.is_empty());
    // Picking up the Malus (event 4, active record): 3.6 and sound 36
    // once, status 2 each time.
    let item = UnitId(0x80);
    f.chains.insert(item, QuestChain(vec![3]));
    ctl.item_event(&mut f, event::ITEM_PICKED_UP, P1, item);
    ctl.item_event(&mut f, event::ITEM_PICKED_UP, P1, item);
    assert!(f.flags(P1).get(3, 6));
    assert_eq!(f.log.iter().filter(|l| *l == "sound 1 36").count(), 1);
    assert_eq!(
        sent(&f),
        [(P1, hex("5d 03 00 02 0000")), (P1, hex("5d 03 00 02 0000"))]
    );
    // Event 10: the +0x14 list loses the player.
    ctl.record_mut(3).unwrap().extra.guids.add(1);
    ctl.player_leaves(&mut f, P1);
    assert!(!ctl.record(3).unwrap().extra.guids.contains(1));
    // not-intro 0: event 3 clears callback 3.
    let (mut ctl, mut f, _) = malus_fake();
    ctl.record_mut(3).unwrap().not_intro = false;
    ctl.changed_level(&mut f, P1, 2, 3);
    assert!(!ctl.record(3).unwrap().has_callback(event::CHANGED_LEVEL));
    // The reward's chat end: status 13.
    let (mut ctl, mut f, _) = malus_fake();
    ctl.record_mut(3).unwrap().extra.rewarded = true;
    f.p(P1).quests.flags[0].set(3, bit::PRIMARY_GOAL_DONE);
    ctl.npc_deactivate(&mut f, P1, CHARSI_U);
    assert_eq!(sent(&f), [(P1, hex("5d 03 00 0d 0000"))]);
    assert!(!ctl.record(3).unwrap().extra.rewarded);
}

// Covers: specs/world/quests-act1.md §10.5 l2 r10, §10.5 l2 r17
#[test]
fn malus_brought_to_charsi() {
    let (mut ctl, mut f, malus) = malus_fake();
    f.p(P1).stats.insert(12, 8);
    f.players.insert(P2, player(2, Some(1)));
    f.p(P2).stats.insert(12, 8);
    f.players.insert(P3, player(3, Some(1)));
    f.p(P3).stats.insert(12, 7);
    f.party.insert(P1, vec![P1, P2, P3]);
    act1::malus_operate(&mut ctl, &mut f, malus, P1);
    f.p(P1).items.push(*b"hdm ");
    f.sent.clear();
    f.log.clear();
    say(&mut ctl, &mut f, CHARSI_U, 163);
    let r = ctl.record(3).unwrap();
    assert!(r.state == 5 && r.extra.rewarded && r.extra.guids.contains(1));
    assert_eq!(r.extra.malus_items, 0);
    assert!(ctl.game.get(3, bit::PRIMARY_GOAL_DONE));
    assert!(f.flags(P1).get(3, 13) && f.flags(P1).get(3, 1));
    // K3: the level-8 member gets 13, 1; the level-7 one nothing.
    assert_eq!(f.flags(P2).word(3) & 0x2002, 0x2002);
    assert_eq!(f.flags(P3).word(3) & 0x2002, 0);
    assert!(!f.players[&P1].items.contains(b"hdm "));
    assert_eq!(f.sent_ids(), [0x28, 0x27, 0x29]);
    // Sequence: chain 3 at 5 passes to chain 6: its opening timer.
    assert_eq!(ctl.timers.last().unwrap().func, TimerFn::SlaughterOpen);
    // Without the Malus: listed, text refreshed, nothing else.
    let (mut ctl, mut f, _) = malus_fake();
    say(&mut ctl, &mut f, CHARSI_U, 163);
    assert!(ctl.record(3).unwrap().extra.guids.contains(1));
    assert_eq!(f.flags(P1).word(3), 0);
    assert_eq!(f.sent_ids(), [0x27, 0x29]);
    // With 3.0: nothing, no refresh.
    let (mut ctl, mut f, _) = malus_fake();
    f.p(P1).quests.flags[0].set(3, bit::REWARD_GRANTED);
    say(&mut ctl, &mut f, CHARSI_U, 163);
    assert!(f.sent.is_empty() && ctl.record(3).unwrap().extra.guids.0.is_empty());
    // Not at state 4 but started with the Malus: chain 6's sequence.
    let (mut ctl, mut f, _) = malus_fake();
    f.p(P1).items.push(*b"hdm ");
    ctl.record_mut(3).unwrap().state = 2;
    ctl.record_mut(3).unwrap().extra.started_with_malus = true;
    say(&mut ctl, &mut f, CHARSI_U, 163);
    assert_eq!(ctl.record(3).unwrap().state, 2);
    assert_eq!(ctl.timers.last().unwrap().func, TimerFn::SlaughterOpen);
}

// Covers: specs/world/quests-act1.md §10.5 l2 r7, §10.5 l2 r8, §10.5 l2 r11, §10.5 l2 r12, §10.5 l2 r15
#[test]
fn malus_counts_and_reset() {
    // Event 13 with the Malus: counted, +0xA1; bit 2 → state 2 first.
    let (mut ctl, mut f, malus) = malus_fake();
    f.p(P1).items.push(*b"hdm ");
    f.p(P1).quests.flags[0].set(3, bit::STARTED);
    f.p(P1).quests.flags[0].set(3, bit::LEAVE_TOWN);
    ctl.player_enters(&mut f, P1, 0).unwrap();
    let r = ctl.record(3).unwrap();
    assert_eq!((r.state, r.status), (2, 1));
    assert!(r.extra.started_with_malus && r.extra.malus_items == 1);
    // Event 14: another carrier joins; the count reaching 1 with the
    // Malus taken sets status 2 (nothing sent).
    let (mut ctl, mut f, _) = malus_fake();
    ctl.player_enters(&mut f, P1, 0).unwrap();
    {
        let x = &mut ctl.record_mut(3).unwrap().extra;
        x.malus_known = true;
        x.malus_mode = 2;
    }
    f.p(P1).items.push(*b"hdm ");
    f.sent.clear();
    ctl.player_enters(&mut f, P1, 0).unwrap();
    let r = ctl.record(3).unwrap();
    assert_eq!((r.extra.malus_items, r.status), (1, 2));
    // Event 9: the carrier leaves; the count reaching 0 → status 3.
    f.p(P1).act = Some(0);
    f.sent.clear();
    let args = EventArgs {
        event: event::PLAYER_DROPPED_WITH_QUEST_ITEM,
        target: Some(UnitId(0x80)),
        player: Some(P1),
        ..EventArgs::default()
    };
    let i = ctl.find(3).unwrap();
    act1::callback(&mut ctl, &mut f, i, args, None, false);
    // Status 3 is set; the 0x5D reports the status function's value: the
    // leaving player still carries the Malus (K4) → 2.
    let r = ctl.record(3).unwrap();
    assert_eq!((r.extra.malus_items, r.status), (0, 3));
    assert_eq!(sent(&f), [(P1, hex("5d 03 00 02 0000"))]);
    // Event 6 → reset: state 3, flags 1, status 1 through K1, the object
    // back to mode 0.
    let (mut ctl, mut f, _) = malus_fake();
    f.p(P1).stats.insert(12, 8);
    act1::malus_operate(&mut ctl, &mut f, malus, P1);
    f.sent.clear();
    let args = EventArgs {
        event: event::EVENT6,
        player: Some(P1),
        ..EventArgs::default()
    };
    let i = ctl.find(3).unwrap();
    act1::callback(&mut ctl, &mut f, i, args, None, false);
    let r = ctl.record(3).unwrap();
    assert_eq!((r.state, r.flags, r.status), (3, 1, 1));
    assert_eq!((r.extra.malus_items, r.extra.malus_mode), (0, 0));
    assert!(r.extra.malus_known);
    assert_eq!(f.objects[&malus].2, 0);
    assert_eq!(sent(&f), [(P1, hex("5d 03 01 01 0000"))]);
    // Reset with the object gone: Malus no longer known.
    let (mut ctl, mut f, _) = malus_fake();
    f.p(P1).stats.insert(12, 8);
    act1::malus_operate(&mut ctl, &mut f, malus, P1);
    f.objects.clear();
    act1::callback(&mut ctl, &mut f, i, args, None, false);
    assert!(!ctl.record(3).unwrap().extra.malus_known);
}

// Covers: specs/world/quests-act1.md §10.5 l2 r13
#[test]
fn malus_status_fn() {
    let (mut ctl, mut f, _) = malus_fake();
    let i = ctl.find(3).unwrap();
    let st = |ctl: &QuestControl, f: &mut Fake| {
        let pf = f.flags(P1);
        act1::status_fn(ctl, f, i, P1, &pf, 0x0059_1D30)
    };
    ctl.records[i].status = 7;
    assert_eq!(st(&ctl, &mut f), Some(7));
    ctl.records[i].state = 5;
    assert_eq!(st(&ctl, &mut f), Some(0));
    ctl.game.set(3, bit::PRIMARY_GOAL_DONE);
    assert_eq!(st(&ctl, &mut f), Some(4));
    f.p(P1).stats.insert(12, 8);
    assert_eq!(st(&ctl, &mut f), Some(12));
    f.p(P1).quests.flags[0].set(3, bit::COMPLETED_NOW);
    assert_eq!(st(&ctl, &mut f), Some(12));
    f.p(P1).quests.flags[0].set(3, bit::PRIMARY_GOAL_DONE);
    assert_eq!(st(&ctl, &mut f), Some(13));
    ctl.records[i].not_intro = false;
    assert_eq!(st(&ctl, &mut f), Some(0));
    // A party member with the Malus: 2, or 0 with 3.0.
    f.players.insert(P2, player(2, Some(1)));
    f.p(P2).items.push(*b"hdm ");
    f.party.insert(P1, vec![P1, P2]);
    assert_eq!(st(&ctl, &mut f), Some(2));
    f.p(P1).quests.flags[0].set(3, bit::REWARD_GRANTED);
    assert_eq!(st(&ctl, &mut f), Some(0));
    f.p(P1).quests.flags[0].set(3, bit::REWARD_PENDING);
    assert_eq!(st(&ctl, &mut f), Some(10));
    // In 0x52 (§6.2): written to slot 3.
    let (mut ctl, mut f, _) = malus_fake();
    ctl.record_mut(3).unwrap().status = 1;
    ctl.request_quest_data(&mut f, P1).unwrap();
    assert_eq!(f.sent.last().unwrap().1[1 + 3], 1);
}

// Covers: specs/world/quests-act1.md §10.5 l2 r16
#[test]
fn malus_imbue_granted() {
    let (mut ctl, mut f, _) = malus_fake();
    f.p(P1).quests.flags[0].set(3, bit::REWARD_PENDING);
    act1::imbue_granted(&mut ctl, &mut f, P1);
    assert_eq!(f.flags(P1).word(3), 0x0001);
    assert!(!ctl.record(3).unwrap().active);
    // With 3.15 the record stays active.
    let (mut ctl, mut f, _) = malus_fake();
    f.p(P1).quests.flags[0].set(3, bit::COMPLETED_BEFORE);
    act1::imbue_granted(&mut ctl, &mut f, P1);
    assert!(ctl.record(3).unwrap().active);
}

// ------------------------------------------------------------ §10.6 A1Q4

const CAIN_T: UnitId = UnitId(0x15);
const CAIN5_U: UnitId = UnitId(0x16);

/// The fake with the Tristram Cain (146) and Cain in town (cain5).
fn cain_fake() -> Fake {
    let mut f = fake();
    for (u, class) in [(CAIN_T, 146), (CAIN5_U, npc::CAIN5)] {
        f.monsters.insert(u, (u.0, class, npc_kind(class)));
    }
    f
}

fn q4(ctl: &QuestControl) -> &act1::q4::Extra4 {
    &ctl.record(4).unwrap().extra.q4
}

fn q4_mut(ctl: &mut QuestControl) -> &mut act1::q4::Extra4 {
    &mut ctl.record_mut(4).unwrap().extra.q4
}

fn flags_mut(f: &mut Fake, p: UnitId) -> &mut QuestFlags {
    &mut f.p(p).quests.flags[0]
}

/// Calls one callback of `chain` directly.
fn call(ctl: &mut QuestControl, f: &mut Fake, chain: u8, args: EventArgs) {
    let i = ctl.find(chain).unwrap();
    act1::callback(ctl, f, i, args, None, false);
}

// Covers: specs/world/quests-act1.md §10.6 r1
#[test]
fn cain_npc_text() {
    let (mut ctl, _) = control();
    let mut f = cain_fake();
    assert!(text(&mut ctl, &mut f, 4, AKARA_U).is_empty());
    ctl.record_mut(4).unwrap().state = 1;
    assert_eq!(text(&mut ctl, &mut f, 4, AKARA_U), [(97, 0)]);
    // The scroll in the inventory → message state 3.
    f.p(P1).items.push(*b"bks ");
    assert_eq!(text(&mut ctl, &mut f, 4, AKARA_U), [(112, 0)]);
    f.p(P1).items.clear();
    // State 4 without the scroll → message state 2.
    ctl.record_mut(4).unwrap().state = 4;
    assert_eq!(text(&mut ctl, &mut f, 4, AKARA_U), [(104, 2)]);
    // The Tristram Cain: state 9 first, then the rest (state 4: nothing
    // for class 146).
    assert_eq!(text(&mut ctl, &mut f, 4, CAIN_T), [(124, 0)]);
    // 4.13 and Cain not heard → 5; heard with 4.1 → 7; 4.1 → 5.
    flags_mut(&mut f, P1).set(4, bit::PRIMARY_GOAL_DONE);
    assert_eq!(text(&mut ctl, &mut f, 4, CAIN5_U), [(123, 0)]);
    flags_mut(&mut f, P1).set(4, bit::REWARD_PENDING);
    q4_mut(&mut ctl).heard.add(1);
    assert_eq!(text(&mut ctl, &mut f, 4, CAIN5_U), [(123, 2)]);
    assert_eq!(text(&mut ctl, &mut f, 4, AKARA_U), [(118, 0)]);
    // Credited (+0xB4) → 6.
    *flags_mut(&mut f, P1) = QuestFlags::default();
    q4_mut(&mut ctl).credited.add(1);
    assert_eq!(text(&mut ctl, &mut f, 4, CAIN5_U), [(125, 0)]);
    // In the record list: heard Cain with 4.14 → 8.
    q4_mut(&mut ctl).credited.remove(1);
    ctl.record_mut(4).unwrap().guids.add(1);
    flags_mut(&mut f, P1).set(4, bit::COMPLETED_NOW);
    assert_eq!(text(&mut ctl, &mut f, 4, CAIN5_U), [(125, 2)]);
    // +0x4F with `bkd `: the scroll is deleted, one fewer in the game.
    let (mut ctl, _) = control();
    let mut f = cain_fake();
    q4_mut(&mut ctl).b4f = true;
    q4_mut(&mut ctl).scrolls = 1;
    f.p(P1).items.push(*b"bkd ");
    text(&mut ctl, &mut f, 4, AKARA_U);
    assert_eq!(f.log, ["delete bkd "]);
    assert_eq!(q4(&ctl).scrolls, 0);
}

// Covers: specs/world/quests-act1.md §10.6 r2, §10.6 r3, §10.6 r9, §10.6 r14, §10.6 r18
#[test]
fn cain_through_akara_and_act2() {
    let (mut ctl, _) = control();
    let mut f = cain_fake();
    ctl.record_mut(4).unwrap().state = 1;
    say(&mut ctl, &mut f, AKARA_U, 97);
    assert!(ctl.record(4).unwrap().state == 2 && ctl.record(4).unwrap().extra.talked);
    f.sent.clear();
    ctl.npc_deactivate(&mut f, P1, AKARA_U);
    assert_eq!(sent(&f), [(P1, hex("5d 04 00 01 0000"))]);
    assert_eq!(f.flags(P1).word(4), 0x0004);
    ctl.changed_level(&mut f, P1, 1, 2);
    assert_eq!(ctl.record(4).unwrap().state, 3);
    // The scroll deciphered (112): state 5, status 3, then the chat end
    // sends status 3 and L2 gives bit 3.
    f.p(P1).items.push(*b"bks ");
    f.sent.clear();
    say(&mut ctl, &mut f, AKARA_U, 112);
    let r = ctl.record(4).unwrap();
    assert_eq!((r.state, r.status, r.flags), (5, 3, 0));
    assert!(r.extra.q4.deciphered && r.extra.q4.b4b && r.extra.q4.b4e);
    assert_eq!(r.extra.q4.scroll_guid, 500);
    f.sent.clear();
    ctl.npc_deactivate(&mut f, P1, AKARA_U);
    assert_eq!(sent(&f), [(P1, hex("5d 04 00 03 0000"))]);
    assert_eq!(f.flags(P1).word(4), 0x000C);
    assert!(!q4(&ctl).deciphered);
    // Lut Gholein before freeing Cain: the cleanup (gibbet mode 3, tree
    // mode 1, the Tristram Cain removed), state 7, status 5, game 4.13,
    // L3 (4.14, credited).
    let (gibbet, tree) = (UnitId(0x60), UnitId(0x61));
    f.objects.insert(gibbet, (0x60, 26, 0));
    f.objects.insert(tree, (0x61, 30, 0));
    {
        let x = q4_mut(&mut ctl);
        (x.gibbet_known, x.gibbet_guid) = (true, 0x60);
        (x.tree_known, x.tree_guid) = (true, 0x61);
    }
    f.sent.clear();
    f.log.clear();
    ctl.changed_level(&mut f, P1, 2, 40);
    f.log.retain(|l| !l.starts_with("unhandled")); // Acts II–V event 3
    let r = ctl.record(4).unwrap();
    assert_eq!((r.state, r.status), (7, 5));
    let x = &r.extra.q4;
    assert!(x.cain_gone && x.cain_removed && x.town_cain_due && x.progress == 1);
    assert_eq!(x.gibbet_open, 3);
    assert!(x.credited.contains(1));
    assert!(ctl.game.get(4, bit::PRIMARY_GOAL_DONE));
    assert_eq!((f.objects[&gibbet].2, f.objects[&tree].2), (3, 1));
    assert_eq!(f.log, ["mode 96 3", "mode 97 1", "remove 21"]);
    assert_eq!(sent(&f), [(P1, hex("5d 04 00 0c 0000"))]);
    assert!(f.flags(P1).get(4, bit::COMPLETED_NOW));
    // Reward (118) with 4.1 and 4.13: state 6, status 13, the sequence
    // opens chain 3 (chain 4's pass state is 6).
    let (mut ctl, _) = control();
    let mut f = cain_fake();
    flags_mut(&mut f, P1).set(4, bit::REWARD_PENDING);
    flags_mut(&mut f, P1).set(4, bit::PRIMARY_GOAL_DONE);
    ctl.record_mut(4).unwrap().state = 5;
    say(&mut ctl, &mut f, AKARA_U, 118);
    let r = ctl.record(4).unwrap();
    assert_eq!((r.state, r.status), (6, 13));
    assert!(r.guids.contains(1) && ctl.game.get(4, bit::PRIMARY_GOAL_DONE));
    assert_eq!(ctl.record(3).unwrap().state, 1);
    assert_eq!(f.sent_ids(), [0x28, 0x5D, 0x27, 0x29]);
    // Cain 125 and 123: the record list, the heard list.
    q4_mut(&mut ctl).credited.add(1);
    say(&mut ctl, &mut f, CAIN5_U, 125);
    assert!(!q4(&ctl).credited.contains(1));
    say(&mut ctl, &mut f, CAIN5_U, 123);
    assert!(q4(&ctl).heard.contains(1));
    // Back in Tristram after state 6 with Cain still there: state 5,
    // status 4, nothing sent.
    let (mut ctl, _) = control();
    let mut f = cain_fake();
    ctl.record_mut(4).unwrap().state = 6;
    ctl.changed_level(&mut f, P1, 4, 38);
    let r = ctl.record(4).unwrap();
    assert_eq!((r.state, r.status), (5, 4));
    assert!(f.sent.is_empty());
}

// Covers: specs/world/quests-act1.md §10.6 r14
#[test]
fn cain_removal_waits_for_an_open_chat() {
    let (mut ctl, _) = control();
    let mut f = cain_fake();
    f.chats.insert(CAIN_T, vec![P1]);
    ctl.changed_level(&mut f, P1, 2, 40);
    // The chat stays: `5D 04 01 00 0000` to its client, the stored preset
    // dropped (Cain not removed).
    assert_eq!(f.sent[0], (P1, hex("5d 04 01 00 0000")));
    assert!(f.log.contains(&"preset 0 146".to_string()));
    assert!(!q4(&ctl).cain_removed && q4(&ctl).cain_gone);
}

// Covers: specs/world/quests-act1.md §10.6 r15
// Covers: specs/world/quests-act1-rest.md §8 r3
#[test]
fn town_cain_spawn() {
    let room = RoomIdT(1);
    let beside = UnitId(0x30);
    let setup = |pos: (i32, i32), spawns: Vec<Option<UnitId>>, spot| {
        let (mut ctl, _) = control();
        let mut f = cain_fake();
        // The anchor is the town-Cain marker object (class 385).
        f.objects.insert(beside, (0x30, 385, 0));
        f.pos.insert(beside, (pos.0, pos.1, room));
        f.rooms.insert(room, (0, 0, 12, 12));
        f.spawns = spawns;
        f.spot = spot;
        let x = q4_mut(&mut ctl);
        (x.marker_known, x.marker_guid, x.town_cain_due) = (true, 0x30, true);
        ctl.changed_level(&mut f, P1, 2, 1);
        f.log.retain(|l| !l.starts_with("unhandled")); // Acts II–V event 3
        (ctl, f)
    };
    // The first spawn fails: one retry one tile on, r 10.
    let (ctl, f) = setup((10, 10), vec![None, Some(UnitId(0x50))], None);
    assert_eq!(
        f.log,
        [
            "spot at 10 10 2 0x100 1 100",
            "spawn 265 10 10 mode 1 r 5",
            "spot at 11 11 2 0x100 2 100",
            "spawn 265 10 10 mode 1 r 10",
            "flags 80 0x3000000",
        ]
    );
    let x = q4(&ctl);
    assert!(x.town_cain && !x.town_cain_due && x.town_cain_guid == 0x50);
    // No point inside the room: (y, y + 21) (bug kept), then the spot.
    let (_, f) = setup((50, 7), vec![Some(UnitId(0x50))], Some((1, 1)));
    assert_eq!(
        f.log[..2],
        ["spot at 7 28 2 0x100 1 100", "spawn 265 8 29 mode 1 r 5"]
    );
    // Every try fails: 20 retries, then (x, y, R0) with r 15.
    let (ctl, f) = setup((10, 10), vec![], None);
    assert_eq!(f.log.iter().filter(|l| l.starts_with("spawn")).count(), 22);
    assert_eq!(f.log.last().unwrap(), "spawn 265 10 10 mode 1 r 15");
    assert!(!q4(&ctl).town_cain);
}

// Covers: specs/world/quests-act1.md §10.6 r4, §10.6 r5, §10.6 r7, §10.6 r8, §10.6 r13
#[test]
fn cain_items_and_leaving() {
    let (mut ctl, _) = control();
    let mut f = cain_fake();
    // Event 4 (active record): L2.
    ctl.record_mut(4).unwrap().state = 2;
    let item = UnitId(0x80);
    f.chains.insert(item, QuestChain(vec![4]));
    ctl.item_event(&mut f, event::ITEM_PICKED_UP, P1, item);
    assert_eq!(f.flags(P1).word(4), 0x0004);
    // Event 9: the last scroll leaves with its carrier: tree reset.
    let tree = UnitId(0x61);
    f.objects.insert(tree, (0x61, 30, 1));
    f.item_codes.insert(item, *b"bks ");
    {
        let x = q4_mut(&mut ctl);
        (x.tree_known, x.tree_guid, x.scrolls) = (true, 0x61, 1);
    }
    ctl.record_mut(4).unwrap().state = 4;
    f.sent.clear();
    let args = EventArgs {
        event: event::PLAYER_DROPPED_WITH_QUEST_ITEM,
        target: Some(item),
        player: Some(P1),
        ..EventArgs::default()
    };
    call(&mut ctl, &mut f, 4, args);
    assert_eq!(q4(&ctl).scrolls, 0);
    assert_eq!(ctl.record(4).unwrap().state, 3);
    assert_eq!(f.objects[&tree].2, 0);
    assert_eq!(sent(&f), [(P1, hex("5d 04 00 01 0000"))]);
    // Event 6: the same reset unless +0x4F.
    ctl.record_mut(4).unwrap().state = 5;
    q4_mut(&mut ctl).b4f = true;
    call(
        &mut ctl,
        &mut f,
        4,
        EventArgs {
            event: event::EVENT6,
            ..EventArgs::default()
        },
    );
    assert_eq!(ctl.record(4).unwrap().state, 5);
    // Event 10: with 4.0 and 4.1 the record list loses the player; the
    // +0xB4 and +0x138 lists always.
    flags_mut(&mut f, P1).set(4, bit::REWARD_GRANTED);
    flags_mut(&mut f, P1).set(4, bit::REWARD_PENDING);
    ctl.record_mut(4).unwrap().guids.add(1);
    q4_mut(&mut ctl).credited.add(1);
    q4_mut(&mut ctl).heard.add(1);
    ctl.player_leaves(&mut f, P1);
    let r = ctl.record(4).unwrap();
    assert!(!r.guids.contains(1) && !r.extra.q4.credited.contains(1));
    assert!(!r.extra.q4.heard.contains(1));
}

// Covers: specs/world/quests-act1.md §10.6 r6
#[test]
fn cow_king_needs_the_cow_level_access() {
    // Vector: killed by a classic-game player lacking 26.0 → nothing.
    let (mut ctl, _) = control();
    let mut f = cain_fake();
    f.expansion = false;
    f.p(P1).level = Some(39);
    let args = EventArgs {
        event: event::MONSTER_KILLED,
        target: Some(UnitId(0x40)),
        player: Some(P1),
        ..EventArgs::default()
    };
    call(&mut ctl, &mut f, 4, args);
    assert!(f.log.is_empty() && f.flags(P1) == QuestFlags::default());
    // With 26.0: 4.10 for the killer and every player in level 39; 8 `vps `
    // drops (quality argument 0).
    flags_mut(&mut f, P1).set(26, bit::REWARD_GRANTED);
    f.players.insert(P2, player(2, Some(39)));
    call(&mut ctl, &mut f, 4, args);
    assert!(f.flags(P1).get(4, 10) && f.flags(P2).get(4, 10));
    assert_eq!(f.log, vec!["drop vps  0"; 8]);
    // Already 4.10: nothing.
    f.log.clear();
    call(&mut ctl, &mut f, 4, args);
    assert!(f.log.is_empty());
}

// Covers: specs/world/quests-act1.md §10.6 r10, §10.6 r11, §10.6 r12
#[test]
fn cain_start_join_and_active() {
    let state = |bits: &[u8], items: &[[u8; 4]]| {
        let (mut ctl, _) = control();
        let mut f = cain_fake();
        for &b in bits {
            flags_mut(&mut f, P1).set(4, b);
        }
        f.p(P1).items = items.to_vec();
        let args = EventArgs {
            event: event::PLAYER_STARTED_GAME,
            player: Some(P1),
            ..EventArgs::default()
        };
        call(&mut ctl, &mut f, 4, args);
        let r = ctl.record(4).unwrap().clone();
        (r, ctl.game.get(4, bit::PRIMARY_GOAL_DONE))
    };
    let (r, game) = state(&[bit::REWARD_GRANTED], &[]);
    assert!(game && r.extra.q4.town_cain_due && r.extra.q4.gibbet_open == 3);
    let (r, game) = state(&[bit::COMPLETED_BEFORE], &[]);
    assert!(!game && r.extra.q4.game_done_due);
    let (r, _) = state(&[bit::ENTER_AREA], &[]);
    assert_eq!((r.state, r.status, r.extra.q4.progress), (5, 4, 1));
    let (r, _) = state(&[bit::STARTED], &[]);
    assert_eq!((r.state, r.status), (2, 1));
    let (r, _) = state(&[], &[*b"bkd "]);
    assert_eq!((r.state, r.status, r.extra.q4.scrolls), (5, 3, 1));
    let (r, _) = state(&[], &[*b"bks "]);
    assert_eq!((r.state, r.status, r.extra.q4.scrolls), (4, 2, 1));
    // Event 14: counted.
    let (mut ctl, _) = control();
    let mut f = cain_fake();
    f.p(P1).items = vec![*b"bks ", *b"bkd "];
    call(
        &mut ctl,
        &mut f,
        4,
        EventArgs {
            event: event::PLAYER_JOINED_GAME,
            player: Some(P1),
            ..EventArgs::default()
        },
    );
    assert_eq!(q4(&ctl).scrolls, 2);
    // Active.
    let i = ctl.find(4).unwrap();
    let f0 = ctl.records[i].active_fn.unwrap();
    assert!(!act1::active_fn(&ctl, &mut f, i, P1, npc::AKARA, f0));
    ctl.records[i].state = 4;
    assert!(act1::active_fn(&ctl, &mut f, i, P1, npc::AKARA, f0));
    flags_mut(&mut f, P1).set(4, bit::PRIMARY_GOAL_DONE);
    assert!(act1::active_fn(&ctl, &mut f, i, P1, npc::CAIN5, f0));
    ctl.records[i].extra.q4.heard.add(1);
    assert!(!act1::active_fn(&ctl, &mut f, i, P1, npc::CAIN5, f0));
    assert!(!act1::active_fn(&ctl, &mut f, i, P1, npc::KASHYA, f0));
}

// Covers: specs/world/quests.md §9.4; specs/world/quests-act1.md §10.6 text, §10.6 r17
#[test]
fn stone_order_tree_and_stones() {
    // Vector: order [18, 20, 17, 21, 19] → `50 0400 0100 0300 0000 0400
    // 0200` (+ two bytes never written; 0 here).
    let (mut ctl, _) = control();
    let mut f = cain_fake();
    ctl.record_mut(4).unwrap().extra.stone_order = Some([18, 20, 17, 21, 19]);
    act1::send_stone_order(&mut ctl, &mut f, P1);
    assert_eq!(
        sent(&f),
        [(P1, hex("50 0400 0100 0300 0000 0400 0200 0000"))]
    );
    // The tree: the scroll drops, state 4, status 2, callback 9.
    let tree = UnitId(0x61);
    f.objects.insert(tree, (0x61, 30, 0));
    ctl.record_mut(4).unwrap().state = 1;
    ctl.record_mut(4)
        .unwrap()
        .clear_callback(event::PLAYER_DROPPED_WITH_QUEST_ITEM);
    f.sent.clear();
    act1::q4::tree_operate(&mut ctl, &mut f, tree, P1);
    assert_eq!(f.log, ["sound 1 45", "drop bks  2", "mode 97 1"]);
    let r = ctl.record(4).unwrap();
    assert_eq!((r.state, r.status), (4, 2));
    assert!(r.has_callback(event::PLAYER_DROPPED_WITH_QUEST_ITEM));
    assert!(r.extra.q4.tree_known && r.extra.q4.scrolls == 1);
    assert_eq!(sent(&f), [(P1, hex("5d 04 00 02 0000"))]);
    // Operated again (mode 1): nothing.
    f.log.clear();
    act1::q4::tree_operate(&mut ctl, &mut f, tree, P1);
    assert!(f.log.is_empty());
    // The linked class-61 object (§4.6).
    let linked = UnitId(0x62);
    f.objects.insert(linked, (0x62, 61, 0));
    f.chains.insert(linked, QuestChain::default());
    assert!(ctl.add_link(&mut f, linked, 4, Some(0x0059_2F80)));
    assert!(q4(&ctl).linked && q4(&ctl).linked_guid == 0x62);
    // Stones without `bkd `: sound 39 on every 64th touch.
    let stones: Vec<UnitId> = (0..5).map(|k| UnitId(0x70 + k)).collect();
    for (k, &s) in stones.iter().enumerate() {
        let class = if k == 3 { 21 } else { 17 };
        f.objects.insert(s, (s.0, class, 0));
    }
    f.pos.insert(stones[3], (100, 100, RoomIdT(1)));
    f.log.clear();
    act1::q4::stone_operate(&mut ctl, &mut f, stones[0], P1, 18);
    act1::q4::stone_operate(&mut ctl, &mut f, stones[0], P1, 18);
    assert_eq!(f.log, ["sound 1 39"]);
    // With `bkd `: the order 18, 20, 17, 21, 19 (a wrong value does
    // nothing); the linked object's mode follows the count.
    f.p(P1).items.push(*b"bkd ");
    f.sent.clear();
    f.log.clear();
    act1::q4::stone_operate(&mut ctl, &mut f, stones[1], P1, 20);
    assert_eq!(q4(&ctl).stones, 0);
    for (k, v) in [(0, 18), (1, 20), (2, 17), (3, 21)] {
        act1::q4::stone_operate(&mut ctl, &mut f, stones[k], P1, v);
        assert_eq!(f.objects[&linked].2, k as i32 + 2);
    }
    assert_eq!(ctl.record(4).unwrap().state, 5);
    act1::q4::stone_operate(&mut ctl, &mut f, stones[4], P1, 19);
    let x = q4(&ctl);
    assert!(x.b4f && x.stones == 5 && x.scrolls == 0);
    assert_eq!(f.objects[&linked].2, 6);
    assert!(f.log.contains(&"delete bkd ".to_string()));
    // The portal beside the stone of value 21, status 4, 4.4, `89 01`.
    // The cairnstones missile (owner the player, skill 0, level 1), its
    // room refreshed.
    let at = f
        .log
        .iter()
        .position(|l| l == "missile 288 owner 1 skill 0 level 1 106 97");
    assert_eq!(f.log[at.unwrap() + 1], "refresh 36865");
    assert!(f.flags(P1).get(4, bit::ENTER_AREA));
    assert_eq!(f.sent[0], (P1, hex("5d 04 00 04 0000")));
    assert_eq!(f.sent.last().unwrap().1, [0x89, 1]);
}

// ------------------------------------------------------------ §10.7 A1Q5

fn q5(ctl: &QuestControl) -> &act1::q5::Extra5 {
    &ctl.record(5).unwrap().extra.q5
}

// Covers: specs/world/quests-act1.md §10.7 r1, §10.7 r8
#[test]
fn tower_npc_text_and_active() {
    let (mut ctl, _) = control();
    let mut f = cain_fake();
    assert!(text(&mut ctl, &mut f, 5, AKARA_U).is_empty());
    ctl.record_mut(5).unwrap().state = 2;
    assert_eq!(text(&mut ctl, &mut f, 5, AKARA_U), [(130, 2)]);
    ctl.record_mut(5).unwrap().state = 4;
    assert!(text(&mut ctl, &mut f, 5, AKARA_U).is_empty());
    ctl.record_mut(5).unwrap().extra.q5.credited.push(1);
    assert_eq!(text(&mut ctl, &mut f, 5, AKARA_U), [(143, 0)]);
    let i = ctl.find(5).unwrap();
    let f0 = ctl.records[i].active_fn.unwrap();
    assert!(act1::active_fn(&ctl, &mut f, i, P1, npc::AKARA, f0));
    assert!(!act1::active_fn(&ctl, &mut f, i, P1, npc::WARRIV1, f0));
    assert!(!act1::active_fn(&ctl, &mut f, i, P1, 147, f0));
    // Reported (A) with 5.0 and 5.13 → message state 3.
    ctl.records[i].extra.q5.credited.clear();
    ctl.records[i].extra.q5.reported.push(1);
    flags_mut(&mut f, P1).set(5, bit::REWARD_GRANTED);
    flags_mut(&mut f, P1).set(5, bit::PRIMARY_GOAL_DONE);
    assert_eq!(text(&mut ctl, &mut f, 5, AKARA_U), [(143, 2)]);
}

// Covers: specs/world/quests-act1.md §10.7 r2, §10.7 r6, §10.7 r9, §10.7 r11
#[test]
fn tower_tome_levels_and_report() {
    let (mut ctl, _) = control();
    let mut f = cain_fake();
    // The tome before any status: mode 1, event 1 at frame + 16, message
    // 127 opened, state 2, +0x11B.
    let tome = UnitId(0x71);
    f.objects.insert(tome, (0x71, 0x9F, 0));
    act1::q5::tome_operate(&mut ctl, &mut f, tome, P1);
    assert_eq!(f.log, ["mode 113 1", "event1 113 16", "message 1 113 127"]);
    assert!(ctl.record(5).unwrap().state == 2 && q5(&ctl).tome_early);
    // Message 127: status 1, M2 (bit 2).
    say(&mut ctl, &mut f, AKARA_U, 127);
    assert_eq!(f.sent[0], (P1, hex("5d 05 00 01 0000")));
    assert_eq!(f.flags(P1).word(5), 0x0004);
    // The Forgotten Tower with status 1: status 4, M2 (state 2: bit 2).
    f.sent.clear();
    ctl.changed_level(&mut f, P1, 3, 20);
    assert_eq!(sent(&f), [(P1, hex("5d 05 00 04 0000"))]);
    // Tower Cellar 5: state 3, status 2, M2 → bit 4 (status 2).
    f.sent.clear();
    ctl.changed_level(&mut f, P1, 24, 25);
    let r = ctl.record(5).unwrap();
    assert_eq!((r.state, r.status), (3, 2));
    assert_eq!(sent(&f), [(P1, hex("5d 05 00 02 0000"))]);
    assert_eq!(f.flags(P1).word(5), 0x0014);
    // From state 0, the tower: state 2, status 3.
    let (mut ctl, _) = control();
    let mut f = cain_fake();
    ctl.changed_level(&mut f, P1, 3, 20);
    let r = ctl.record(5).unwrap();
    assert_eq!((r.state, r.status), (2, 3));
    // The report (140–145) after the kill: state 5, the sequence (chain
    // 5 → chain 3 opens), B → A.
    ctl.record_mut(5).unwrap().extra.q5.report_due = true;
    ctl.record_mut(5).unwrap().extra.q5.credited.push(1);
    flags_mut(&mut f, P1).set(5, bit::PRIMARY_GOAL_DONE);
    say(&mut ctl, &mut f, KASHYA_U, 142);
    assert_eq!(ctl.record(5).unwrap().state, 5);
    assert_eq!(ctl.record(3).unwrap().state, 1);
    assert_eq!(
        (q5(&ctl).credited.len(), q5(&ctl).reported.clone()),
        (0, vec![1])
    );
    // Leaving town at state 5 from A with B empty: inactive.
    ctl.changed_level(&mut f, P1, 1, 2);
    assert!(!ctl.record(5).unwrap().active);
}

// Covers: specs/world/quests-act1.md §10.7 r3, §10.7 r4, §10.7 r5
#[test]
fn countess_kill_and_timer() {
    let (mut ctl, _) = control();
    let mut f = cain_fake();
    f.p(P1).level = Some(25);
    f.players.insert(P2, player(2, Some(20)));
    f.players.insert(P3, player(3, Some(3)));
    f.party.insert(P1, vec![P1, P3]);
    let countess = UnitId(0x40);
    f.pos.insert(countess, (7, 8, RoomIdT(1)));
    ctl.record_mut(5).unwrap().state = 3;
    ctl.tick = 20;
    kill(&mut ctl, &mut f, 5, countess, P1);
    let r = ctl.record(5).unwrap();
    assert_eq!(r.state, 5);
    assert!(!r.has_callback(event::MONSTER_KILLED) && r.active);
    let x = &r.extra.q5;
    assert!(x.killed && x.report_due && x.death_pos == (7, 8));
    assert_eq!(x.credited, [1]);
    assert!(ctl.game.get(5, bit::PRIMARY_GOAL_DONE));
    // M4: the cellar player 13, 0; the other 14. M5: the party member
    // 13, 0. M6: 14 and `5D 05 00 0C` to the one lacking 5.0.
    assert_eq!(f.flags(P1).word(5), 0x2001);
    // P3 is out of the cellar: M4's 14 first, then M5's 13, 0.
    assert_eq!(f.flags(P3).word(5), 0x6001);
    assert_eq!(f.flags(P2).word(5), 0x4000);
    assert_eq!(sent(&f), [(P2, hex("5d 05 00 0c 0000"))]);
    // The trap step: no chest listed, no trap; the victim's event 7.
    assert_eq!(f.log, ["sound 1 37", "event7 64 10"]);
    // Timer 7: status 13 at tick 28 (M1: everyone here).
    f.sent.clear();
    for _ in 0..7 {
        ctl.update(&mut f);
    }
    assert!(f.sent.is_empty());
    ctl.update(&mut f);
    assert_eq!(
        sent(&f),
        [
            (P1, hex("5d 05 00 0d 0000")),
            (P2, hex("5d 05 00 0c 0000")),
            (P3, hex("5d 05 00 0d 0000")),
        ]
    );
    // Event 10 (kept by the kill): out of the lists.
    ctl.player_leaves(&mut f, P1);
    assert!(q5(&ctl).credited.is_empty());
}

// Covers: specs/world/quests.md §9.5; specs/world/quests-act1.md §10.7 r7, §10.7 r10
#[test]
fn tower_restore_and_objects() {
    for (bits, want) in [
        (vec![4u8], (3, 1)),
        (vec![6], (3, 4)),
        (vec![5], (2, 3)),
        (vec![3], (3, 1)),
        (vec![2], (2, 1)),
        (vec![2, 0], (0, 0)),
    ] {
        let (mut ctl, _) = control();
        let mut f = cain_fake();
        for b in bits {
            flags_mut(&mut f, P1).set(5, b);
        }
        call(
            &mut ctl,
            &mut f,
            5,
            EventArgs {
                event: event::PLAYER_STARTED_GAME,
                player: Some(P1),
                ..EventArgs::default()
            },
        );
        let r = ctl.record(5).unwrap();
        assert_eq!((r.state, r.status), want);
    }
    // Chests: listed (8 at most); after the kill each run reschedules.
    let (mut ctl, _) = control();
    let mut f = cain_fake();
    let chest = UnitId(0x72);
    act1::q5::chest_init(&mut ctl, &mut f, chest);
    assert_eq!(q5(&ctl).chests, [0x72]);
    // Not killed: the trap step does nothing.
    assert!(f.log.is_empty());
    ctl.record_mut(5).unwrap().extra.q5.killed = true;
    f.log.clear();
    // Killed, the chest is gone: no trap, the reschedule.
    object_event(&mut ctl, &mut f, chest, 0x173);
    assert_eq!(f.log, ["event7 114 10"]);
    // Object init `0x00595A00`: switched off → mode 3.
    ctl.record_mut(5).unwrap().not_intro = false;
    f.log.clear();
    act1::q5::object_init(&mut ctl, &mut f, chest);
    assert_eq!(f.log, ["mode 114 3"]);
}

// ------------------------------------------------------------ §10.8 A1Q6

fn q6(ctl: &QuestControl) -> &act1::q6::Extra6 {
    &ctl.record(6).unwrap().extra.q6
}

// Covers: specs/world/quests-act1.md §10.8 r1, §10.8 r9
#[test]
fn slaughter_npc_text_and_active() {
    let (mut ctl, _) = control();
    let mut f = cain_fake();
    let i = ctl.find(6).unwrap();
    let f0 = ctl.records[i].active_fn.unwrap();
    assert!(text(&mut ctl, &mut f, 6, CAIN5_U).is_empty());
    ctl.records[i].state = 1;
    assert_eq!(text(&mut ctl, &mut f, 6, CAIN5_U), [(166, 0)]);
    assert!(act1::active_fn(&ctl, &mut f, i, P1, npc::CAIN5, f0));
    // Message state 0 has Cain's line only.
    assert!(text(&mut ctl, &mut f, 6, AKARA_U).is_empty());
    ctl.records[i].state = 2;
    assert_eq!(text(&mut ctl, &mut f, 6, AKARA_U), [(168, 2)]);
    // Listed for Akara → message state 3.
    ctl.records[i].extra.q6.akara.add(1);
    assert_eq!(text(&mut ctl, &mut f, 6, AKARA_U), [(179, 0)]);
    assert!(act1::active_fn(&ctl, &mut f, i, P1, npc::AKARA, f0));
    // 6.1: Cain → 4, Warriv → 3; Warriv active.
    flags_mut(&mut f, P1).set(6, bit::REWARD_PENDING);
    assert_eq!(text(&mut ctl, &mut f, 6, CAIN5_U), [(184, 2)]);
    assert_eq!(text(&mut ctl, &mut f, 6, WARRIV_U), [(183, 0)]);
    assert!(act1::active_fn(&ctl, &mut f, i, P1, npc::WARRIV1, f0));
    assert!(!act1::active_fn(&ctl, &mut f, i, P1, npc::CAIN5, f0));
}

// Covers: specs/world/quests-act1.md §10.8 r2, §10.8 r3, §10.8 r7, §10.8 r8
#[test]
fn slaughter_start_catacombs_and_reward() {
    let (mut ctl, _) = control();
    let mut f = cain_fake();
    ctl.record_mut(6).unwrap().state = 1;
    say(&mut ctl, &mut f, CAIN5_U, 166);
    assert_eq!(f.flags(P1).word(6), 0x0004);
    f.sent.clear();
    ctl.npc_deactivate(&mut f, P1, CAIN5_U);
    assert_eq!(sent(&f), [(P1, hex("5d 06 00 01 0000"))]);
    assert!(!ctl.record(6).unwrap().has_callback(event::NPC_DEACTIVATE));
    // Catacombs 1 with status 1: state 3, O2 → bit 3.
    f.sent.clear();
    ctl.changed_level(&mut f, P1, 33, 34);
    assert_eq!(ctl.record(6).unwrap().state, 3);
    assert_eq!(f.flags(P1).word(6), 0x000C);
    // Catacombs 4: status 2 sent, O2 → bit 4.
    ctl.changed_level(&mut f, P1, 36, 37);
    assert_eq!(sent(&f), [(P1, hex("5d 06 00 02 0000"))]);
    assert_eq!(f.flags(P1).word(6), 0x001C);
    // Catacombs 2 from state 1 with status 0: status 1, nothing sent.
    let (mut ctl, _) = control();
    let mut f = cain_fake();
    ctl.record_mut(6).unwrap().state = 1;
    ctl.changed_level(&mut f, P1, 34, 35);
    let r = ctl.record(6).unwrap();
    assert_eq!((r.state, r.status), (3, 1));
    assert!(f.sent.is_empty());
    // Warriv 183 with 6.1 and 6.13: refresh first, then status 13, state
    // 5, game 6.13, 6.0, the record list, 0x28.
    flags_mut(&mut f, P1).set(6, bit::REWARD_PENDING);
    flags_mut(&mut f, P1).set(6, bit::PRIMARY_GOAL_DONE);
    say(&mut ctl, &mut f, WARRIV_U, 183);
    let r = ctl.record(6).unwrap();
    assert_eq!((r.state, r.status), (5, 13));
    assert!(r.guids.contains(1) && ctl.game.get(6, bit::PRIMARY_GOAL_DONE));
    assert_eq!(f.flags(P1).word(6), 0x2009);
    assert_eq!(f.sent_ids(), [0x27, 0x29, 0x28]);
    // Lut Gholein at state 4 → 5.
    ctl.record_mut(6).unwrap().state = 4;
    ctl.changed_level(&mut f, P1, 1, 40);
    assert_eq!(ctl.record(6).unwrap().state, 5);
    // Catacombs entry keeps states 4 and 5 (not changed: no O2 with
    // status ≠ 0 below Catacombs 4).
    for s in [4, 5] {
        ctl.record_mut(6).unwrap().state = s;
        ctl.record_mut(6).unwrap().status = 1;
        f.sent.clear();
        ctl.changed_level(&mut f, P1, 33, 34);
        assert_eq!(ctl.record(6).unwrap().state, s);
        assert!(f.sent.is_empty());
    }
    // Event 13: §10.1 restore.
    let (mut ctl, _) = control();
    let mut f = cain_fake();
    flags_mut(&mut f, P1).set(6, bit::LEAVE_TOWN);
    ctl.player_enters(&mut f, P1, 0).unwrap();
    let r = ctl.record(6).unwrap();
    assert_eq!((r.state, r.status), (3, 1));
}

// Covers: specs/world/quests.md §5 text; specs/world/quests-act1.md §10.8 r4, §10.8 r5, §10.8 r6, §10.8 text
// Covers: specs/world/quests-act1-rest.md §5 r1, §5 r2, §5 r3
#[test]
fn andariel_kill_and_portal_timer() {
    let (mut ctl, _) = control();
    let mut f = cain_fake();
    // Expansion characters at progression 0 (save flags bit 5).
    f.client_flags.insert(P1, 0x0020);
    f.client_flags.insert(P2, 0x0020);
    f.p(P1).level = Some(37);
    f.pos.insert(P1, (30, 40, RoomIdT(2)));
    f.players.insert(P2, player(2, Some(37)));
    let andariel = UnitId(0x40);
    f.monsters.insert(andariel, (0x40, 156, npc_kind(156)));
    ctl.record_mut(6).unwrap().state = 3;
    ctl.tick = 50;
    let mut s = ctl.seed;
    let gems: Vec<String> = [&act1::CHIPPED_GEMS, &act1::CHIPPED_GEMS, &act1::NORMAL_GEMS]
        .iter()
        .map(|l| {
            let c = act1::gem_code(l, s.step());
            format!("drop {} 2", String::from_utf8_lossy(&c))
        })
        .collect();
    kill(&mut ctl, &mut f, 6, andariel, P1);
    let r = ctl.record(6).unwrap();
    assert_eq!(r.state, 4);
    assert!(r.has_callback(event::PLAYER_LEAVES_GAME) && !r.has_callback(event::MONSTER_KILLED));
    let x = &r.extra.q6;
    assert!(x.killed && x.victim == 0x40 && x.counter == 1);
    assert!(x.cain.contains(1) && x.akara.contains(2) && x.kashya.contains(1));
    assert_eq!(f.flags(P1).word(6), 0x2002);
    assert_eq!(f.flags(P2).word(6), 0x2002);
    // The killer's credit, the gem draws, O3 credits both (each credit
    // raises the progression to 5 · 0 + 1), O6 sound 33.
    let mut want = vec!["progression 1 0x0120".to_string()];
    want.extend(gems);
    want.extend(["progression 1 0x0120", "progression 2 0x0120"].map(String::from));
    want.extend(["sound 1 33", "sound 2 33"].map(String::from));
    assert_eq!(f.log, want);
    assert!(f.sent.is_empty());
    // Vector: firings at T + 2, T + 4, …; the 9th (counter 10, T + 18)
    // opens the portal at the first player in Catacombs 4; the 11th
    // (counter 12, T + 22) sends status 3 and removes the timer.
    f.log.clear();
    while ctl.tick < 67 {
        ctl.update(&mut f);
    }
    assert!(f.log.is_empty());
    ctl.update(&mut f);
    assert_eq!(ctl.tick, 68);
    assert_eq!(f.log, ["portal 30 40 59 1"]);
    assert_eq!(q6(&ctl).counter, 10);
    while ctl.tick < 71 {
        ctl.update(&mut f);
    }
    assert!(f.sent.is_empty());
    ctl.update(&mut f);
    assert_eq!(ctl.tick, 72);
    assert_eq!(
        sent(&f),
        [(P1, hex("5d 06 00 03 0000")), (P2, hex("5d 06 00 03 0000"))]
    );
    assert!(ctl.timers.is_empty());
    // Event 10 after the kill: out of every list.
    ctl.player_leaves(&mut f, P1);
    let x = q6(&ctl);
    assert!(!x.cain.contains(1) && !x.akara.contains(1) && !x.kashya.contains(1));
    assert!(act1::q6::killed(&ctl));
}

// ------------------------------------------------------------ §10.3, §8.1

// Covers: specs/world/quests-act1.md §10.3
#[test]
fn act1_intro_first_talk() {
    let (mut ctl, _) = control();
    let mut f = cain_fake();
    f.p(P1).class = 1; // sorceress: Akara's special text
    assert_eq!(text(&mut ctl, &mut f, 37, AKARA_U), [(12, 0)]);
    assert_eq!(text(&mut ctl, &mut f, 37, KASHYA_U), [(24, 0)]);
    assert!(text(&mut ctl, &mut f, 37, WARRIV_U).is_empty());
    let i = ctl.find(37).unwrap();
    let f0 = ctl.records[i].active_fn.unwrap();
    assert!(act1::active_fn(&ctl, &mut f, i, P1, npc::AKARA, f0));
    // Message 11 (or 12) from Akara sets the intro bit; no refresh.
    say(&mut ctl, &mut f, AKARA_U, 11);
    assert!(f.players[&P1].quests.heard(0, npc::AKARA));
    assert!(f.sent.is_empty());
    assert!(text(&mut ctl, &mut f, 37, AKARA_U).is_empty());
    assert!(!act1::active_fn(&ctl, &mut f, i, P1, npc::AKARA, f0));
    // Kashya's 11 is not hers.
    say(&mut ctl, &mut f, KASHYA_U, 11);
    assert!(!f.players[&P1].quests.heard(0, npc::KASHYA));
}

// Covers: specs/world/quests.md §8.1
#[test]
fn act_transition_send_order() {
    // Meshif: intro flags (Act II list), 0x28, `61 03`.
    let (mut ctl, _) = control();
    let mut f = cain_fake();
    ctl.act_completion(&mut f, P1, npc::MESHIF1).unwrap();
    assert_eq!(f.sent_ids(), [0x28, 0x61]);
    assert_eq!(f.sent[1].1, [0x61, 3]);
    assert!(f.players[&P1].quests.intro[0].contains(&INTRO_NPCS[1][0]));
    // The Durance: intro flags (Act III list), 0x28, `61 04`.
    let mut f = cain_fake();
    ctl.object_warp(&mut f, P1, 102);
    assert_eq!(f.sent_ids(), [0x28, 0x61]);
    assert_eq!(f.sent[1].1, [0x61, 4]);
    assert!(f.players[&P1].quests.intro[0].contains(&INTRO_NPCS[2][0]));
    // +0x4C already 1: no 0x61.
    let mut f = cain_fake();
    f.p(P1).byte4c = 1;
    ctl.act_completion(&mut f, P1, npc::MESHIF1).unwrap();
    assert_eq!(f.sent_ids(), [0x28]);
}

// Covers: specs/world/quests-act1.md §10.5 text, §10.7 text
#[test]
fn chain_2_3_5_init_records() {
    // Init `0x00591210` (A1Q2), `0x00591F70` (A1Q3), `0x00595920` (A1Q5):
    // active 1, state 0, the init_no / seq_id / filter and the function
    // addresses of each record, the extra zeroed.
    let (ctl, _) = control();
    for (chain, init_no, seq_id, filter, status_fn, active_fn, seq_fn) in [
        (2u8, 4u8, 4u8, 2u8, None, 0x0059_1080u32, 0x0059_10F0u32),
        (3, 5, 6, 3, Some(0x0059_1D30), 0x0059_1C30, 0x0059_1E40),
        (5, 4, 3, 5, None, 0x0059_52C0, 0x0059_5240),
    ] {
        let r = ctl.record(chain).unwrap();
        assert!(r.active, "chain {chain}");
        assert_eq!((r.state, r.init_no, r.seq_id), (0, init_no, Some(seq_id)));
        assert_eq!(r.filter, filter, "chain {chain}");
        assert_eq!(r.status_fn, status_fn, "chain {chain}");
        assert_eq!(r.active_fn, Some(active_fn), "chain {chain}");
        assert_eq!(r.seq_fn, Some(seq_fn), "chain {chain}");
        assert!(r.guids.0.is_empty(), "chain {chain}");
    }
    let e = &ctl.record(5).unwrap().extra.q5;
    assert!(e.reported.is_empty() && e.credited.is_empty() && e.chests.is_empty());
    assert!(e.in_cellar.0.is_empty() && e.death_pos == (0, 0) && !e.killed && !e.trap_spawned);
}

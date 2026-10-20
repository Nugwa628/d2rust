// Spec: specs/world/quests-act2.md §3 (A2Q1 Radament's Lair), Test vectors
//! A2Q1 callback by callback on the quests' fake world: chat, Atma's
//! messages, chat end, leaving town, Radament's AI hook, the kill and
//! its timer, game start and the Book of Skill.

use super::q1;
use crate::units::{RoomId, UnitId};
use crate::world::quests::tests::*;
use crate::world::quests::*;

const ATMA_U: UnitId = UnitId(0x20);
const WARRIV2_U: UnitId = UnitId(0x21);
const RAD_U: UnitId = UnitId(0x30);
const P3: UnitId = UnitId(3);
const P4: UnitId = UnitId(4);
const KILL_ROOM: RoomId = RoomId(7);
const SLOT: u8 = 9;

fn npc_kind(class: u16) -> UnitKind {
    UnitKind::Monster {
        class: u32::from(class),
        superunique: None,
        owner: None,
    }
}

fn player(guid: u32) -> Player {
    Player {
        guid,
        act: Some(1),
        level: Some(40),
        ..Player::default()
    }
}

/// P1 in Lut Gholein; Atma, warriv2 and Radament (chain 8 linked, in
/// Sewers Level 3).
fn fake() -> Fake {
    let mut f = Fake::new();
    f.players.insert(P1, player(1));
    for (u, class) in [(ATMA_U, 176), (WARRIV2_U, 175), (RAD_U, 229)] {
        f.monsters.insert(u, (u.0, class, npc_kind(class)));
    }
    f.unit_levels.insert(RAD_U, 49);
    f.pos.insert(RAD_U, (10, 10, KILL_ROOM));
    f
}

fn word(f: &Fake, p: UnitId) -> u16 {
    f.flags(p).word(SLOT)
}

fn set(f: &mut Fake, p: UnitId, bits: &[u8]) {
    for &b in bits {
        f.p(p).quests.flags[0].set(SLOT, b);
    }
}

fn rec(ctl: &QuestControl) -> &QuestRecord {
    ctl.record(8).unwrap()
}

fn rec_mut(ctl: &mut QuestControl) -> &mut QuestRecord {
    ctl.record_mut(8).unwrap()
}

fn x(ctl: &QuestControl) -> &q1::Extra {
    &rec(ctl).extra.a2.q1
}

fn x_mut(ctl: &mut QuestControl) -> &mut q1::Extra {
    &mut rec_mut(ctl).extra.a2.q1
}

/// One callback of chain 8 only.
fn call(ctl: &mut QuestControl, f: &mut Fake, args: EventArgs, list: Option<&mut TextList>) {
    let i = ctl.find(8).unwrap();
    act1::callback(ctl, f, i, args, list, false);
}

/// Event 0 to chain 8 only: the lines it adds for P1.
fn text(ctl: &mut QuestControl, f: &mut Fake, n: UnitId) -> TextList {
    let mut list = TextList::new();
    let args = EventArgs {
        event: event::NPC_ACTIVATE,
        target: Some(n),
        player: Some(P1),
        ..EventArgs::default()
    };
    call(ctl, f, args, Some(&mut list));
    list
}

/// C→S 0x31 from `p` to the NPC unit `n` (GUID = unit id in the fake).
fn say(ctl: &mut QuestControl, f: &mut Fake, p: UnitId, n: UnitId, msg: u16) {
    let mut m = vec![0x31];
    m.extend_from_slice(&n.0.to_le_bytes());
    m.extend_from_slice(&msg.to_le_bytes());
    m.extend_from_slice(&[0, 0]);
    assert_eq!(ctl.quest_message(f, p, &m), 0);
}

fn active(ctl: &QuestControl, f: &mut Fake, npc: u16) -> bool {
    let i = ctl.find(8).unwrap();
    act2::active_fn(ctl, f, i, P1, npc, 0x0059_8910)
}

/// The 0x5D messages sent, as (player, bytes).
fn status_msgs(f: &Fake) -> Vec<(UnitId, Vec<u8>)> {
    f.sent.iter().filter(|m| m.1[0] == 0x5D).cloned().collect()
}

fn ids(f: &Fake) -> Vec<(UnitId, u8)> {
    f.sent.iter().map(|m| (m.0, m.1[0])).collect()
}

// Covers: specs/world/quests-act2.md §3.2
#[test]
fn radament_chat_text() {
    let (mut ctl, _) = control();
    let mut f = fake();
    // Record state 1 (init): index 0 → Atma's start message 304.
    assert_eq!(rec(&ctl).state, 1);
    assert_eq!(text(&mut ctl, &mut f, ATMA_U), [(304, 0)]);
    // Vector: state 3, 9.0/9.1/9.13 clear, not listed → table state 2
    // (Atma 317, warriv2 315, menu 2).
    rec_mut(&mut ctl).state = 3;
    assert_eq!(text(&mut ctl, &mut f, ATMA_U), [(317, 2)]);
    assert_eq!(text(&mut ctl, &mut f, WARRIV2_U), [(315, 2)]);
    // Vector: state 4 with 9.13 clear → nothing; with 9.13 → table
    // state 3 (Atma 334, menu 1 sent as 0).
    rec_mut(&mut ctl).state = 4;
    assert_eq!(text(&mut ctl, &mut f, ATMA_U), []);
    set(&mut f, P1, &[bit::PRIMARY_GOAL_DONE]);
    assert_eq!(text(&mut ctl, &mut f, ATMA_U), [(334, 0)]);
    // State 5 → table state 4; state 6 → table state 0.
    rec_mut(&mut ctl).state = 5;
    assert_eq!(text(&mut ctl, &mut f, ATMA_U), [(334, 2)]);
    rec_mut(&mut ctl).state = 6;
    assert_eq!(text(&mut ctl, &mut f, ATMA_U), [(304, 0)]);
    // States past the 7-entry index table, and state 0: nothing.
    for s in [0, 7, 8, 9] {
        rec_mut(&mut ctl).state = s;
        assert_eq!(text(&mut ctl, &mut f, ATMA_U), [], "state {s}");
    }
    // 9.0 set: nothing.
    rec_mut(&mut ctl).state = 3;
    set(&mut f, P1, &[bit::REWARD_GRANTED]);
    assert_eq!(text(&mut ctl, &mut f, ATMA_U), []);
    // GUID listed → table state 4 (Atma 334, menu 2), whatever the bits.
    rec_mut(&mut ctl).guids.add(1);
    assert_eq!(text(&mut ctl, &mut f, ATMA_U), [(334, 2)]);
    // 9.1 → table state 3 first, even at state 0.
    rec_mut(&mut ctl).state = 0;
    set(&mut f, P1, &[bit::REWARD_PENDING]);
    assert_eq!(text(&mut ctl, &mut f, ATMA_U), [(334, 0)]);
    assert!(ctl.faults.is_empty());
}

// Covers: specs/world/quests-act2.md §3.2
#[test]
fn radament_wants_to_talk() {
    let (mut ctl, _) = control();
    let mut f = fake();
    // not-intro, state 1, 9.15 clear, Atma only.
    assert!(active(&ctl, &mut f, 176));
    assert!(!active(&ctl, &mut f, 175));
    set(&mut f, P1, &[bit::COMPLETED_BEFORE]);
    assert!(!active(&ctl, &mut f, 176));
    // 9.1 → true at any state.
    set(&mut f, P1, &[bit::REWARD_PENDING]);
    rec_mut(&mut ctl).state = 4;
    assert!(active(&ctl, &mut f, 176));
    // 9.0 → false.
    set(&mut f, P1, &[bit::REWARD_GRANTED]);
    assert!(!active(&ctl, &mut f, 176));
    // State 2 or an intro record without 9.1 → false.
    f.p(P1).quests.flags[0] = QuestFlags::default();
    rec_mut(&mut ctl).state = 2;
    assert!(!active(&ctl, &mut f, 176));
    rec_mut(&mut ctl).state = 1;
    rec_mut(&mut ctl).not_intro = false;
    assert!(!active(&ctl, &mut f, 176));
}

// Covers: specs/world/quests-act2.md §3.3, §3.4, §3.5, §3.1
#[test]
fn radament_start_chat_end_and_leaving_town() {
    let (mut ctl, _) = control();
    let mut f = fake();
    f.players.insert(P2, player(2));
    set(&mut f, P2, &[bit::REWARD_GRANTED]);
    // 304 from Atma: started, state 2, 9.2 for players without 9.0/9.1,
    // then the refresh (0x27, 0x29) to P1.
    say(&mut ctl, &mut f, P1, ATMA_U, 304);
    assert!(x(&ctl).atma_started);
    assert_eq!(rec(&ctl).state, 2);
    assert_eq!(word(&f, P1), 1 << bit::STARTED);
    assert_eq!(word(&f, P2), 1 << bit::REWARD_GRANTED);
    assert_eq!(ids(&f), [(P1, 0x27), (P1, 0x29)]);
    // 304 from another NPC: nothing.
    f.sent.clear();
    rec_mut(&mut ctl).state = 1;
    say(&mut ctl, &mut f, P1, WARRIV2_U, 304);
    assert_eq!(rec(&ctl).state, 1);
    assert!(f.sent.is_empty());
    rec_mut(&mut ctl).state = 2;
    // Chat end with another NPC: nothing.
    ctl.npc_deactivate(&mut f, P1, WARRIV2_U);
    assert!(f.sent.is_empty() && x(&ctl).atma_started);
    // Chat end with Atma: status 1 to all (flags 0; F: P1 qualifies, P2
    // has 9.0), callback 2 cleared.
    rec_mut(&mut ctl).flags = 0x20;
    ctl.npc_deactivate(&mut f, P1, ATMA_U);
    assert_eq!(status_msgs(&f), [(P1, hex("5d 08 00 01 0000"))]);
    assert_eq!(rec(&ctl).status, 1);
    assert_eq!(rec(&ctl).flags, 0);
    assert!(!x(&ctl).atma_started);
    assert!(!rec(&ctl).has_callback(event::NPC_DEACTIVATE));
    // Leaving town (old level 40): quick remove; state 2 → 3, iterate
    // with status 1 → 9.3.
    rec_mut(&mut ctl).guids.add(1);
    rec_mut(&mut ctl).guids.add(9);
    ctl.changed_level(&mut f, P1, 40, 41);
    assert_eq!(rec(&ctl).guids.0, [9]);
    assert_eq!(rec(&ctl).state, 3);
    assert_eq!(word(&f, P1), 1 << bit::STARTED | 1 << bit::LEAVE_TOWN);
    // Another old level, or state ≠ 2: nothing.
    ctl.changed_level(&mut f, P1, 41, 40);
    rec_mut(&mut ctl).state = 2;
    set(&mut f, P1, &[bit::REWARD_PENDING]);
    ctl.changed_level(&mut f, P1, 40, 41);
    assert_eq!(rec(&ctl).state, 2);
    assert!(ctl.faults.is_empty());
}

// Covers: specs/world/quests-act2.md §3.6
// Covers: specs/world/quests-act2-2.md §1 r19
#[test]
fn radament_ai_hook() {
    let (mut ctl, _) = control();
    let mut f = fake();
    // Wrong level or an intro record: nothing.
    f.unit_levels.insert(RAD_U, 48);
    q1::radament_ai(&mut ctl, &mut f, RAD_U);
    assert_eq!((rec(&ctl).state, rec(&ctl).status), (1, 0));
    f.unit_levels.insert(RAD_U, 49);
    rec_mut(&mut ctl).not_intro = false;
    q1::radament_ai(&mut ctl, &mut f, RAD_U);
    assert_eq!((rec(&ctl).state, rec(&ctl).status), (1, 0));
    rec_mut(&mut ctl).not_intro = true;
    // First entry: state 1 → 3, callback 2 cleared, status 2 sent (+0x09
    // becomes 1), then the iterate → 9.4.
    rec_mut(&mut ctl).flags = 0x20;
    q1::radament_ai(&mut ctl, &mut f, RAD_U);
    assert_eq!((rec(&ctl).state, rec(&ctl).status), (3, 2));
    assert_eq!(rec(&ctl).flags, 0);
    assert!(!rec(&ctl).has_callback(event::NPC_DEACTIVATE));
    assert!(x(&ctl).entry_sent);
    assert_eq!(status_msgs(&f), [(P1, hex("5d 08 00 02 0000"))]);
    assert_eq!(word(&f, P1), 1 << bit::ENTER_AREA);
    // State ≥ 3 and status ≥ 2: nothing more.
    f.sent.clear();
    q1::radament_ai(&mut ctl, &mut f, RAD_U);
    assert!(f.sent.is_empty());
    // +0x09 already set: status 2 silently (no 0x5D), iterate still runs.
    f.p(P1).quests.flags[0] = QuestFlags::default();
    rec_mut(&mut ctl).state = 2;
    rec_mut(&mut ctl).status = 1;
    q1::radament_ai(&mut ctl, &mut f, RAD_U);
    assert_eq!((rec(&ctl).state, rec(&ctl).status), (3, 2));
    assert!(f.sent.is_empty());
    assert_eq!(word(&f, P1), 1 << bit::ENTER_AREA);
    // Status ≥ 2 with a state below 3 (act2-2 §1 item 19): state := 3,
    // then the flag iterate only (status ≠ 1 → 9.4); no 0x5D.
    f.p(P1).quests.flags[0] = QuestFlags::default();
    rec_mut(&mut ctl).state = 2;
    rec_mut(&mut ctl).status = 3;
    q1::radament_ai(&mut ctl, &mut f, RAD_U);
    assert_eq!((rec(&ctl).state, rec(&ctl).status), (3, 3));
    assert!(f.sent.is_empty());
    assert_eq!(word(&f, P1), 1 << bit::ENTER_AREA);
    assert!(!f.log.iter().any(|l| l.starts_with("unhandled")));
}

/// Radament with chain 8 dies, killed by P1.
fn kill(ctl: &mut QuestControl, f: &mut Fake) {
    f.chains.insert(RAD_U, QuestChain(vec![8]));
    ctl.monster_killed(f, RAD_U, Some(P1));
}

// Covers: specs/world/quests-act2.md §3.7 text, §3.7 r1, §3.7 r2, §3.7 r3, §3.1
#[test]
fn radament_kill_vector() {
    // Vector: A in the kill room, B adjacent, C in town (party of A); all
    // lack 9.0/9.1, none holds `ass ` → A, B, C get 9.13, 9.1, 9.5; 3
    // books drop; A, B, C get sound 50.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.players.insert(P2, player(2));
    f.players.insert(P3, player(3));
    f.near = vec![P1, P2];
    f.party.insert(P1, vec![P1, P3]);
    rec_mut(&mut ctl).state = 3;
    kill(&mut ctl, &mut f);
    let goal = 1 << bit::PRIMARY_GOAL_DONE | 1 << bit::REWARD_PENDING | 1 << bit::CUSTOM1;
    for p in [P1, P2, P3] {
        assert_eq!(word(&f, p), goal, "{p:?}");
    }
    // 0x28 to A, then A's party member C, then B; no completion flag
    // (all have 9.1).
    assert_eq!(ids(&f), [(P1, 0x28), (P3, 0x28), (P2, 0x28)]);
    let books: Vec<&String> = f.log.iter().filter(|l| l.starts_with("drop")).collect();
    assert_eq!(books, ["drop ass  2"; 3]);
    let sounds: Vec<&String> = f.log.iter().filter(|l| l.starts_with("sound")).collect();
    assert_eq!(sounds, ["sound 1 50", "sound 2 50", "sound 3 50"]);
    assert_eq!(rec(&ctl).state, 4);
    assert!(ctl.game.get(SLOT, bit::PRIMARY_GOAL_DONE));
    assert!(!rec(&ctl).has_callback(event::NPC_DEACTIVATE));
    let e = x(&ctl);
    assert!(e.killed && e.timer);
    assert_eq!((e.kill_room, e.books), (Some(KILL_ROOM), 3));
    assert_eq!(ctl.timers.len(), 1);
    assert_eq!(
        (ctl.timers[0].func, ctl.timers[0].period),
        (TimerFn::Act2(act2::Timer::RadamentStatus), 12)
    );
    // The timer fires 13 updater ticks later: status 3 to all (state 4,
    // each has 9.13), +0x0A := 0, removed.
    f.sent.clear();
    for _ in 0..12 {
        ctl.update(&mut f);
    }
    assert!(f.sent.is_empty());
    ctl.update(&mut f);
    let want = hex("5d 08 00 03 0000");
    assert_eq!(
        status_msgs(&f),
        [(P1, want.clone()), (P2, want.clone()), (P3, want)]
    );
    assert_eq!(rec(&ctl).status, 3);
    assert!(ctl.timers.is_empty() && !x(&ctl).timer);
    assert!(ctl.faults.is_empty());
}

// Covers: specs/world/quests-act2.md §3.7 r2, §3.7 r3, §edge-cases-original-bugs r2
#[test]
fn radament_kill_far_players_and_completion_flag() {
    let (mut ctl, _) = control();
    let mut f = fake();
    // P2 far away with a pending book from an earlier game (9.1, 9.5);
    // P3 holds a book with 9.5; P4 far, no bits; P5 in Act I in P1's
    // party.
    let p5 = UnitId(5);
    for (p, g) in [(P2, 2), (P3, 3), (P4, 4), (p5, 5)] {
        f.players.insert(p, player(g));
    }
    f.p(p5).act = Some(0);
    set(&mut f, P2, &[bit::REWARD_PENDING, bit::CUSTOM1]);
    set(&mut f, P3, &[bit::REWARD_GRANTED, bit::CUSTOM1]);
    f.p(P3).items.push(*b"ass ");
    f.near = vec![P1];
    f.party.insert(P1, vec![P1, p5]);
    kill(&mut ctl, &mut f);
    // P1 near: goal bits; P5 out of Act II: nothing from the party step,
    // then the completion flag (9.14). P4: completion flag.
    let goal = 1 << bit::PRIMARY_GOAL_DONE | 1 << bit::REWARD_PENDING | 1 << bit::CUSTOM1;
    assert_eq!(word(&f, P1), goal);
    assert_eq!(word(&f, P4), 1 << bit::COMPLETED_NOW);
    assert_eq!(word(&f, p5), 1 << bit::COMPLETED_NOW);
    assert_eq!(word(&f, P2), 1 << bit::REWARD_PENDING | 1 << bit::CUSTOM1);
    let done = hex("5d 08 00 0c 0000");
    assert_eq!(
        f.sent.iter().map(|m| (m.0, m.1[0])).collect::<Vec<_>>(),
        [(P1, 0x28), (P4, 0x5D), (p5, 0x5D)]
    );
    assert_eq!(f.sent[1].1, done);
    // Books: P1 and the far P2 (edge case 2); P3 holds one. All drop at
    // Radament. Sound 50 only for 9.13 (P1).
    assert_eq!(x(&ctl).books, 2);
    let books = f.log.iter().filter(|l| l.starts_with("drop")).count();
    assert_eq!(books, 2);
    let sounds: Vec<&String> = f.log.iter().filter(|l| l.starts_with("sound")).collect();
    assert_eq!(sounds, ["sound 1 50"]);
    // A second linked death: the count is reset and recounted (P1 and P2
    // still lack a book: 2); no second timer.
    f.log.clear();
    kill(&mut ctl, &mut f);
    assert_eq!(x(&ctl).books, 2);
    assert_eq!(ctl.timers.len(), 1);
    // Intro record: the kill does nothing.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.near = vec![P1];
    rec_mut(&mut ctl).not_intro = false;
    kill(&mut ctl, &mut f);
    assert_eq!(word(&f, P1), 0);
    assert_eq!(rec(&ctl).state, 1);
    assert!(ctl.timers.is_empty() && f.sent.is_empty());
}

// Covers: specs/world/quests-act2.md §3.3, §1.4
#[test]
fn radament_reward_message() {
    let (mut ctl, _) = control();
    let mut f = fake();
    f.players.insert(P2, player(2));
    f.near = vec![P1, P2];
    rec_mut(&mut ctl).state = 3;
    kill(&mut ctl, &mut f);
    f.sent.clear();
    // 334 from P1 (9.1, 9.13), state 4: status 13 to all, callback 2
    // cleared, state 5, own seq fn → seq(13): Seven Tombs state 0 → 1.
    rec_mut(&mut ctl).callbacks |= 1 << event::NPC_DEACTIVATE;
    say(&mut ctl, &mut f, P1, ATMA_U, 334);
    let want = hex("5d 08 00 0d 0000");
    assert_eq!(status_msgs(&f), [(P1, want.clone()), (P2, want)]);
    assert_eq!((rec(&ctl).state, rec(&ctl).status), (5, 13));
    assert!(!rec(&ctl).has_callback(event::NPC_DEACTIVATE));
    assert_eq!(ctl.record(13).unwrap().state, 1);
    // P1: 9.0 set, 9.1 cleared, GUID added, refresh (0x27, 0x29).
    let granted = 1 << bit::REWARD_GRANTED | 1 << bit::PRIMARY_GOAL_DONE | 1 << bit::CUSTOM1;
    assert_eq!(word(&f, P1), granted);
    assert_eq!(rec(&ctl).guids.0, [1]);
    assert_eq!(ids(&f)[2..], [(P1, 0x27), (P1, 0x29)], "after the two 0x5D");
    // P2 next: state already 5, nothing resent.
    f.sent.clear();
    say(&mut ctl, &mut f, P2, ATMA_U, 334);
    assert_eq!(ids(&f), [(P2, 0x27), (P2, 0x29)]);
    assert_eq!(word(&f, P2), granted);
    assert_eq!(rec(&ctl).guids.0, [1, 2]);
    // Without 9.1: nothing at all.
    f.sent.clear();
    say(&mut ctl, &mut f, P2, ATMA_U, 334);
    assert!(f.sent.is_empty());
    assert!(ctl.faults.is_empty());
}

// Covers: specs/world/quests-act2.md §3.3, §3.1
#[test]
fn radament_reward_in_intro_and_pending_games() {
    // Intro record, P1 with 9.13 and 9.1: status 13, state 5, game 9.13.
    let (mut ctl, _) = control();
    let mut f = fake();
    rec_mut(&mut ctl).not_intro = false;
    rec_mut(&mut ctl).state = 0;
    set(&mut f, P1, &[bit::PRIMARY_GOAL_DONE, bit::REWARD_PENDING]);
    say(&mut ctl, &mut f, P1, ATMA_U, 334);
    assert_eq!((rec(&ctl).state, rec(&ctl).status), (5, 13));
    assert!(ctl.game.get(SLOT, bit::PRIMARY_GOAL_DONE));
    assert_eq!(ctl.record(13).unwrap().state, 1);
    // Not-intro, 9.13 already granted state 5: game 9.13 not set here.
    let (mut ctl, _) = control();
    let mut f = fake();
    rec_mut(&mut ctl).state = 5;
    set(&mut f, P1, &[bit::PRIMARY_GOAL_DONE, bit::REWARD_PENDING]);
    say(&mut ctl, &mut f, P1, ATMA_U, 334);
    assert!(!ctl.game.get(SLOT, bit::PRIMARY_GOAL_DONE));
    assert_eq!(status_msgs(&f), []);
    // 9.1 without 9.13 and +0x10 set: seq(13) only.
    let (mut ctl, _) = control();
    let mut f = fake();
    rec_mut(&mut ctl).not_intro = false;
    rec_mut(&mut ctl).state = 0;
    x_mut(&mut ctl).reward_pending = true;
    set(&mut f, P1, &[bit::REWARD_PENDING]);
    say(&mut ctl, &mut f, P1, ATMA_U, 334);
    assert_eq!((rec(&ctl).state, rec(&ctl).status), (0, 0));
    assert_eq!(ctl.record(13).unwrap().state, 1);
    assert_eq!(word(&f, P1), 1 << bit::REWARD_GRANTED);
    // Same without +0x10: chain 13 stays at 0.
    let (mut ctl, _) = control();
    let mut f = fake();
    set(&mut f, P1, &[bit::REWARD_PENDING]);
    say(&mut ctl, &mut f, P1, ATMA_U, 334);
    assert_eq!(ctl.record(13).unwrap().state, 0);
    assert_eq!(word(&f, P1), 1 << bit::REWARD_GRANTED);
}

fn start(ctl: &mut QuestControl, f: &mut Fake) {
    let args = EventArgs {
        event: event::PLAYER_STARTED_GAME,
        target: Some(P1),
        player: Some(P1),
        ..EventArgs::default()
    };
    call(ctl, f, args, None);
}

// Covers: specs/world/quests-act2.md §3.9, §edge-cases-original-bugs r1
#[test]
fn radament_game_start() {
    // Edge case 1: 9.0 and 9.5 without a book → 9.0, 9.1, 9.15, 9.5
    // cleared; game 9.13; not-intro 0; state 0.
    let (mut ctl, _) = control();
    let mut f = fake();
    set(
        &mut f,
        P1,
        &[
            bit::REWARD_GRANTED,
            bit::REWARD_PENDING,
            bit::COMPLETED_BEFORE,
            bit::CUSTOM1,
            bit::PRIMARY_GOAL_DONE,
        ],
    );
    start(&mut ctl, &mut f);
    assert_eq!(word(&f, P1), 1 << bit::PRIMARY_GOAL_DONE);
    assert!(ctl.game.get(SLOT, bit::PRIMARY_GOAL_DONE));
    assert!(!rec(&ctl).not_intro);
    assert_eq!(rec(&ctl).state, 0);
    // With the book held: bits kept, the rest the same.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.p(P1).items.push(*b"ass ");
    set(&mut f, P1, &[bit::REWARD_GRANTED, bit::CUSTOM1]);
    start(&mut ctl, &mut f);
    assert_eq!(word(&f, P1), 1 << bit::REWARD_GRANTED | 1 << bit::CUSTOM1);
    assert!(ctl.game.get(SLOT, bit::PRIMARY_GOAL_DONE));
    assert_eq!((rec(&ctl).not_intro, rec(&ctl).state), (false, 0));
    // 9.15 with 9.1: +0x10 := 1; state 0, not-intro 0; no game bit.
    for (pending, want) in [(true, true), (false, false)] {
        let (mut ctl, _) = control();
        let mut f = fake();
        set(&mut f, P1, &[bit::COMPLETED_BEFORE]);
        if pending {
            set(&mut f, P1, &[bit::REWARD_PENDING]);
        }
        start(&mut ctl, &mut f);
        assert_eq!(x(&ctl).reward_pending, want);
        assert_eq!((rec(&ctl).not_intro, rec(&ctl).state), (false, 0));
        assert!(!ctl.game.get(SLOT, bit::PRIMARY_GOAL_DONE));
    }
    // Progress bits restore (state, status).
    for (b, want) in [
        (Some(bit::ENTER_AREA), (3, 2)),
        (Some(bit::LEAVE_TOWN), (3, 1)),
        (Some(bit::STARTED), (2, 1)),
        (None, (1, 0)),
    ] {
        let (mut ctl, _) = control();
        let mut f = fake();
        if let Some(b) = b {
            set(&mut f, P1, &[b]);
        }
        start(&mut ctl, &mut f);
        assert_eq!((rec(&ctl).state, rec(&ctl).status), want, "{b:?}");
        assert!(rec(&ctl).not_intro);
    }
}

// Covers: specs/world/quests-act2.md §1.1
// Covers: specs/world/quests-act2-2.md §1 r2
#[test]
fn radament_player_leaves() {
    // `0x00598980` → `0x00545530`: removed only when P1 has 9.0 and 9.1.
    for (bits, kept) in [
        (&[][..], vec![1, 7]),
        (&[bit::REWARD_GRANTED][..], vec![1, 7]),
        (&[bit::REWARD_PENDING][..], vec![1, 7]),
        (&[bit::REWARD_GRANTED, bit::REWARD_PENDING][..], vec![7]),
    ] {
        let (mut ctl, _) = control();
        let mut f = fake();
        set(&mut f, P1, bits);
        rec_mut(&mut ctl).guids.add(1);
        rec_mut(&mut ctl).guids.add(7);
        ctl.player_leaves(&mut f, P1);
        assert_eq!(rec(&ctl).guids.0, kept, "{bits:?}");
        assert!(!f.log.iter().any(|l| l.starts_with("unhandled 8 ")));
    }
}

// Covers: specs/world/quests-act2.md §3.8
#[test]
fn book_of_skill_use() {
    let mut f = fake();
    // Without 9.5: sound 19, the book stays.
    assert!(!q1::use_book_of_skill(&mut f, P1));
    assert_eq!(f.log, ["sound 1 19"]);
    assert!(f.sent.is_empty());
    assert_eq!(f.p(P1).stats.get(&5), None);
    // With 9.5: cleared, +1 stat 5, `5D 08 02 00 0000`, consumed.
    set(&mut f, P1, &[bit::REWARD_GRANTED, bit::CUSTOM1]);
    assert!(q1::use_book_of_skill(&mut f, P1));
    assert_eq!(word(&f, P1), 1 << bit::REWARD_GRANTED);
    assert_eq!(f.p(P1).stats[&5], 1);
    assert_eq!(f.sent, [(P1, hex("5d 08 02 00 0000"))]);
    assert_eq!(f.log, ["sound 1 19"]);
    // A second book: refused.
    assert!(!q1::use_book_of_skill(&mut f, P1));
    assert_eq!(f.p(P1).stats[&5], 1);
}

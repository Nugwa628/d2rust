// Spec: specs/world/quests-act2.md §9 (A2Q0, A2Q7, A2Q8, the Act II intro)
// Spec: specs/world/quests-act2-2.md §1 items 2, 3, 18
//! The Act II gossip and intro records on the quests' fake world.

use crate::rng::Seed;
use crate::units::UnitId;
use crate::world::quests::tests::*;
use crate::world::quests::*;

const JERHYN_U: UnitId = UnitId(0x21);
const CAIN_U: UnitId = UnitId(0x22);
const GUARD4_U: UnitId = UnitId(0x23);
const GUARD5_U: UnitId = UnitId(0x24);
const ELZIX_U: UnitId = UnitId(0x25);
const GREIZ_U: UnitId = UnitId(0x26);
const ATMA_U: UnitId = UnitId(0x27);
const MESHIF_U: UnitId = UnitId(0x28);

fn npc_kind(class: u16) -> UnitKind {
    UnitKind::Monster {
        class: u32::from(class),
        superunique: None,
        owner: None,
    }
}

/// P1 in Lut Gholein with seed {12345, 666}; the Act II gossip NPCs.
fn fake() -> Fake {
    let mut f = Fake::new();
    f.p(P1).act = Some(1);
    f.p(P1).level = Some(40);
    f.p(P1).seed = Seed::new(12345, 666);
    for (u, class) in [
        (JERHYN_U, 201),
        (CAIN_U, 244),
        (GUARD4_U, 377),
        (GUARD5_U, 378),
        (ELZIX_U, 199),
        (GREIZ_U, 198),
        (ATMA_U, 176),
        (MESHIF_U, 210),
    ] {
        f.monsters.insert(u, (u.0, class, npc_kind(class)));
    }
    f
}

fn set(f: &mut Fake, slot: u8, b: u8) {
    f.p(P1).quests.flags[0].set(slot, b);
}

fn has(f: &Fake, slot: u8, b: u8) -> bool {
    f.flags(P1).get(slot, b)
}

/// Event 0 to one chain's record only: the lines it adds for P1.
fn text(ctl: &mut QuestControl, f: &mut Fake, chain: u8, n: UnitId) -> TextList {
    let mut list = TextList::new();
    let args = EventArgs {
        event: event::NPC_ACTIVATE,
        target: Some(n),
        player: Some(P1),
        ..EventArgs::default()
    };
    let i = ctl.find(chain).unwrap();
    act1::callback(ctl, f, i, args, Some(&mut list), false);
    list
}

/// One callback of one chain only.
fn call(ctl: &mut QuestControl, f: &mut Fake, chain: u8, ev: u8, target: UnitId) {
    let args = EventArgs {
        event: ev,
        target: Some(target),
        player: Some(P1),
        ..EventArgs::default()
    };
    let i = ctl.find(chain).unwrap();
    act1::callback(ctl, f, i, args, None, false);
}

/// C→S 0x31 from P1 to the NPC unit `n` (GUID = unit id in the fake).
fn say(ctl: &mut QuestControl, f: &mut Fake, n: UnitId, msg: u16) {
    let mut m = vec![0x31];
    m.extend_from_slice(&n.0.to_le_bytes());
    m.extend_from_slice(&msg.to_le_bytes());
    m.extend_from_slice(&[0, 0]);
    assert_eq!(ctl.quest_message(f, P1, &m), 0);
}

fn active(ctl: &QuestControl, f: &mut Fake, chain: u8, npc: u16) -> bool {
    let i = ctl.find(chain).unwrap();
    let a = ctl.records[i].active_fn.unwrap();
    act2::active_fn(ctl, f, i, P1, npc, a)
}

// ------------------------------------------------------------ chain 7

// Covers: specs/world/quests-act2.md §9
// Covers: specs/world/quests-act2-2.md §1 r2, §1 r18
#[test]
fn jerhyn_gossip() {
    let (mut ctl, _) = control();
    let mut f = fake();
    // Jerhyn with 8.0 clear → table state 0 (253); Cain needs 4.14.
    assert_eq!(text(&mut ctl, &mut f, 7, JERHYN_U), [(253, 0)]);
    assert_eq!(text(&mut ctl, &mut f, 7, CAIN_U), []);
    assert!(active(&ctl, &mut f, 7, 201));
    assert!(!active(&ctl, &mut f, 7, 244));
    // Cain with 4.14, not listed → table state 1 (125) and `0x005940A0`:
    // P1 leaves chain 4's +0xB4 list (act2-2 §1 item 18).
    set(&mut f, 4, bit::COMPLETED_NOW);
    ctl.record_mut(4).unwrap().extra.q4.credited.add(1);
    ctl.record_mut(4).unwrap().extra.q4.credited.add(9);
    assert_eq!(text(&mut ctl, &mut f, 7, CAIN_U), [(125, 0)]);
    assert_eq!(ctl.record(4).unwrap().extra.q4.credited.0, [9]);
    assert!(f.log.is_empty());
    // 125 from another NPC: nothing; from Cain: P1 joins the extra list.
    say(&mut ctl, &mut f, JERHYN_U, 125);
    assert!(ctl.record(7).unwrap().extra.a2.q0.0.is_empty());
    say(&mut ctl, &mut f, CAIN_U, 125);
    assert_eq!(ctl.record(7).unwrap().extra.a2.q0.0, [1]);
    f.log.clear();
    assert_eq!(text(&mut ctl, &mut f, 7, CAIN_U), []);
    assert!(f.log.is_empty());
    // Event 10: P1 leaves the extra list.
    ctl.player_leaves(&mut f, P1);
    assert!(ctl.record(7).unwrap().extra.a2.q0.0.is_empty());
    // 253 from Cain: nothing; from Jerhyn: 8.0, game 8.13.
    say(&mut ctl, &mut f, CAIN_U, 253);
    assert!(!has(&f, 8, bit::REWARD_GRANTED));
    say(&mut ctl, &mut f, JERHYN_U, 253);
    assert!(has(&f, 8, bit::REWARD_GRANTED));
    assert_eq!(f.flags(P1).word(8), 1);
    assert!(ctl.game.get(8, bit::PRIMARY_GOAL_DONE));
    assert_eq!(text(&mut ctl, &mut f, 7, JERHYN_U), []);
    assert!(!active(&ctl, &mut f, 7, 201));
    // Event 13: 8.0 → game 8.13; without it nothing.
    let (mut ctl, _) = control();
    call(&mut ctl, &mut f, 7, event::PLAYER_STARTED_GAME, P1);
    assert!(ctl.game.get(8, bit::PRIMARY_GOAL_DONE));
    let (mut ctl, _) = control();
    let mut f = fake();
    call(&mut ctl, &mut f, 7, event::PLAYER_STARTED_GAME, P1);
    assert_eq!(ctl.game, QuestFlags::default());
    assert!(ctl.faults.is_empty());
}

// ------------------------------------------------------------ chain 26

/// The guard's line for P1 and the draws it made.
fn guard(ctl: &mut QuestControl, f: &mut Fake) -> (TextList, Seed) {
    let l = text(ctl, f, 26, GUARD4_U);
    (l, f.p(P1).seed)
}

// Covers: specs/world/quests-act2.md §9
#[test]
fn guard4_lines_draw_from_the_player_seed() {
    let s0 = Seed::new(12345, 666);
    let mut one = s0;
    assert_eq!(one.step(), 22_752_887);
    // lo' mod 3 = 2 → inline and roll(3) both give table state 4 (63);
    // roll(2) = 1 → table state 1 (60).
    assert_eq!(22_752_887 % 3, 2);
    let (mut ctl, _) = control();
    let mut f = fake();
    // 30.0, 30.13 clear, no Radament progress, chain 8 not intro → roll(2).
    assert_eq!(guard(&mut ctl, &mut f), (vec![(60, 0)], one));
    // Every other branch draws once too: (setup, line).
    type Setup = fn(&mut QuestControl, &mut Fake);
    let cases: [(Setup, u16); 8] = [
        (|_, f| set(f, 30, bit::PRIMARY_GOAL_DONE), 63),
        (|c, _| c.record_mut(8).unwrap().not_intro = false, 63),
        (|_, f| set(f, 9, bit::REWARD_GRANTED), 63),
        (|_, f| set(f, 9, bit::PRIMARY_GOAL_DONE), 63),
        (|_, f| set(f, 9, bit::REWARD_PENDING), 63),
        (|c, _| c.game.set(9, bit::PRIMARY_GOAL_DONE), 63),
        (|_, f| set(f, 30, bit::REWARD_GRANTED), 63),
        (
            |c, f| {
                // 30.0 wins over everything else.
                set(f, 30, bit::REWARD_GRANTED);
                c.record_mut(8).unwrap().not_intro = false;
            },
            63,
        ),
    ];
    for (k, (setup, line)) in cases.into_iter().enumerate() {
        let (mut ctl, _) = control();
        let mut f = fake();
        setup(&mut ctl, &mut f);
        assert_eq!(guard(&mut ctl, &mut f), (vec![(line, 0)], one), "case {k}");
    }
    // A second chat continues the same seed: roll(2) of the next lo'.
    let mut two = one;
    let want = two.step() % 2;
    let (mut ctl, _) = control();
    let mut f = fake();
    guard(&mut ctl, &mut f);
    let (l, s) = guard(&mut ctl, &mut f);
    assert_eq!((l, s), (vec![(59 + want as u16, 0)], two));
    // Another NPC: no draw, no line.
    let mut f = fake();
    assert_eq!(text(&mut ctl, &mut f, 26, ATMA_U), []);
    assert_eq!(f.p(P1).seed, s0);
}

// Covers: specs/world/quests-act2.md §9
// Covers: specs/world/quests-act2-2.md §1 r3
#[test]
fn guard4_messages_kill_and_active() {
    let (mut ctl, _) = control();
    let mut f = fake();
    assert!(active(&ctl, &mut f, 26, 377));
    assert!(!active(&ctl, &mut f, 26, 378));
    // 59, 60 → 30.13; any other message from 377 → 30.0 (act2-2 §1
    // item 3).
    for (msg, slot_word) in [(58, 1), (59, 0x2000), (60, 0x2000), (64, 1)] {
        f.p(P1).quests.flags[0] = QuestFlags::default();
        say(&mut ctl, &mut f, GUARD4_U, msg);
        assert_eq!(f.flags(P1).word(30), slot_word, "msg {msg}");
    }
    f.p(P1).quests.flags[0] = QuestFlags::default();
    // 30.13 set: no talk unless 9.13.
    set(&mut f, 30, bit::PRIMARY_GOAL_DONE);
    assert!(!active(&ctl, &mut f, 26, 377));
    set(&mut f, 9, bit::PRIMARY_GOAL_DONE);
    assert!(active(&ctl, &mut f, 26, 377));
    for msg in [61, 62, 63] {
        f.p(P1).quests.flags[0] = QuestFlags::default();
        say(&mut ctl, &mut f, GUARD4_U, msg);
        assert_eq!(f.flags(P1).word(30), 1, "msg {msg}");
    }
    // 30.0: never.
    set(&mut f, 9, bit::PRIMARY_GOAL_DONE);
    assert!(!active(&ctl, &mut f, 26, 377));
    // Event 8 is a bare `ret`: handled, nothing changes.
    f.log.clear();
    f.sent.clear();
    call(&mut ctl, &mut f, 26, event::MONSTER_KILLED, GUARD4_U);
    assert!(f.log.is_empty() && f.sent.is_empty());
}

// ------------------------------------------------------------ chain 27

// Covers: specs/world/quests-act2.md §9
#[test]
fn guard5_gossip() {
    let (mut ctl, _) = control();
    let mut f = fake();
    // Chain 13 not-intro state 0, chain 10 not-intro state 0 (< 4): no.
    assert_eq!(text(&mut ctl, &mut f, 27, GUARD5_U), []);
    // Chain 10 at state 4 → table state 0 (303).
    ctl.record_mut(10).unwrap().state = 4;
    assert_eq!(text(&mut ctl, &mut f, 27, GUARD5_U), [(303, 0)]);
    assert_eq!(text(&mut ctl, &mut f, 27, ATMA_U), []);
    // Chain 10 an intro record, at any state → yes.
    ctl.record_mut(10).unwrap().state = 0;
    ctl.record_mut(10).unwrap().not_intro = false;
    assert_eq!(text(&mut ctl, &mut f, 27, GUARD5_U), [(303, 0)]);
    // Chain 13 at state 2, or an intro record → no.
    ctl.record_mut(13).unwrap().state = 2;
    assert_eq!(text(&mut ctl, &mut f, 27, GUARD5_U), []);
    ctl.record_mut(13).unwrap().state = 1;
    assert_eq!(text(&mut ctl, &mut f, 27, GUARD5_U), [(303, 0)]);
    ctl.record_mut(13).unwrap().not_intro = false;
    assert_eq!(text(&mut ctl, &mut f, 27, GUARD5_U), []);
    ctl.record_mut(13).unwrap().not_intro = true;
    // 14.1 or 14.0 → no.
    for b in [bit::REWARD_PENDING, bit::REWARD_GRANTED] {
        f.p(P1).quests.flags[0] = QuestFlags::default();
        set(&mut f, 14, b);
        assert_eq!(text(&mut ctl, &mut f, 27, GUARD5_U), []);
    }
    // Wants to talk: also needs game 11.13 (not chain 10's state).
    f.p(P1).quests.flags[0] = QuestFlags::default();
    assert!(!active(&ctl, &mut f, 27, 378));
    ctl.game.set(11, bit::PRIMARY_GOAL_DONE);
    assert!(active(&ctl, &mut f, 27, 378));
    assert!(!active(&ctl, &mut f, 27, 377));
    ctl.record_mut(13).unwrap().state = 2;
    assert!(!active(&ctl, &mut f, 27, 378));
    ctl.record_mut(13).unwrap().state = 0;
    set(&mut f, 14, bit::REWARD_PENDING);
    assert!(!active(&ctl, &mut f, 27, 378));
    // 303 → 31.0; another message nothing.
    f.p(P1).quests.flags[0] = QuestFlags::default();
    say(&mut ctl, &mut f, GUARD5_U, 302);
    assert_eq!(f.flags(P1).word(31), 0);
    say(&mut ctl, &mut f, GUARD5_U, 303);
    assert_eq!(f.flags(P1).word(31), 1);
}

// ------------------------------------------------------------ chain 38

// Covers: specs/world/quests-act2.md §9, §edge-cases-original-bugs r6
#[test]
fn act2_intro_record() {
    let (mut ctl, _) = control();
    let mut f = fake();
    // Elzix: special class necromancer (2).
    f.p(P1).class = 2;
    assert_eq!(text(&mut ctl, &mut f, 38, ELZIX_U), [(204, 0)]);
    f.p(P1).class = 0;
    assert_eq!(text(&mut ctl, &mut f, 38, ELZIX_U), [(203, 0)]);
    // Meshif: amazon (0).
    assert_eq!(text(&mut ctl, &mut f, 38, MESHIF_U), [(242, 0)]);
    // Greiz: no special class, always table state 0 (190).
    for c in 0..7 {
        f.p(P1).class = c;
        assert_eq!(text(&mut ctl, &mut f, 38, GREIZ_U), [(190, 0)]);
    }
    // Edge case 6: Atma has rows (221) but the record ignores her.
    assert_eq!(text(&mut ctl, &mut f, 38, ATMA_U), []);
    say(&mut ctl, &mut f, ATMA_U, 221);
    assert!(f.p(P1).quests.intro[0].is_empty());
    // A message from the wrong NPC sets nothing.
    say(&mut ctl, &mut f, ELZIX_U, 190);
    assert!(f.p(P1).quests.intro[0].is_empty());
    // 204 from Elzix sets Elzix's bit; his intro is then silent.
    say(&mut ctl, &mut f, ELZIX_U, 204);
    assert_eq!(
        f.p(P1).quests.intro[0].iter().copied().collect::<Vec<_>>(),
        [199]
    );
    assert_eq!(text(&mut ctl, &mut f, 38, ELZIX_U), []);
    assert_eq!(text(&mut ctl, &mut f, 38, GREIZ_U), [(190, 0)]);
    say(&mut ctl, &mut f, GREIZ_U, 190);
    say(&mut ctl, &mut f, MESHIF_U, 241);
    assert_eq!(
        f.p(P1).quests.intro[0].iter().copied().collect::<Vec<_>>(),
        [198, 199, 210]
    );
    // The active function `0x005985C0` returns false (act2-2 §1 item 18).
    f.log.clear();
    for npc in [199, 198, 210, 176] {
        assert!(!active(&ctl, &mut f, 38, npc));
    }
    assert!(f.log.is_empty());
}

// Covers: specs/world/quests-act2-2.md §1 r3, §edge-cases-original-bugs r1
#[test]
fn guard4_chat_end_callback() {
    // Vector: msg 61 from NPC 377 → 30.0; callback 2 unchanged.
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = ctl.find(26).unwrap();
    assert!(!ctl.records[i].has_callback(event::NPC_DEACTIVATE));
    say(&mut ctl, &mut f, GUARD4_U, 61);
    assert_eq!(f.flags(P1).word(30), 1);
    assert!(!ctl.records[i].has_callback(event::NPC_DEACTIVATE));
    // The same messages from another NPC: nothing (NPC 377 only).
    f.p(P1).quests.flags[0] = QuestFlags::default();
    for msg in [59, 61] {
        say(&mut ctl, &mut f, GUARD5_U, msg);
    }
    assert_eq!(f.flags(P1).word(30), 0);
    assert!(!ctl.records[i].has_callback(event::NPC_DEACTIVATE));
    // Vector: msg 60 from 377 → 30.13 and callback 2 = `0x0059E0B0`; the
    // chat end leaves it set (extra +0x00 is never 1).
    say(&mut ctl, &mut f, GUARD4_U, 60);
    assert_eq!(f.flags(P1).word(30), 0x2000);
    assert!(ctl.records[i].has_callback(event::NPC_DEACTIVATE));
    f.log.clear();
    call(&mut ctl, &mut f, 26, event::NPC_DEACTIVATE, GUARD4_U);
    assert!(ctl.records[i].has_callback(event::NPC_DEACTIVATE));
    assert!(f.log.is_empty() && f.sent.iter().all(|m| m.1[0] != 0x5D));
    // With +0x00 = 1 (unreachable in 1.14d) the body clears both.
    ctl.records[i].extra.a2.q7_chat = 1;
    call(&mut ctl, &mut f, 26, event::NPC_DEACTIVATE, ATMA_U);
    assert_eq!(ctl.records[i].extra.a2.q7_chat, 1);
    call(&mut ctl, &mut f, 26, event::NPC_DEACTIVATE, GUARD4_U);
    assert_eq!(ctl.records[i].extra.a2.q7_chat, 0);
    assert!(!ctl.records[i].has_callback(event::NPC_DEACTIVATE));
}

// Covers: specs/world/quests-act2-2.md §1 r3
#[test]
fn guard5_message_needs_its_npc() {
    let (mut ctl, _) = control();
    let mut f = fake();
    say(&mut ctl, &mut f, GUARD4_U, 303);
    say(&mut ctl, &mut f, ATMA_U, 303);
    assert_eq!(f.flags(P1).word(31), 0);
    say(&mut ctl, &mut f, GUARD5_U, 303);
    assert_eq!(f.flags(P1).word(31), 1);
}

// Covers: specs/world/quests-act2-2.md §1 r18
#[test]
fn gossip_status_functions_report_nothing() {
    let (ctl, _) = control();
    let mut f = fake();
    let pf = QuestFlags::default();
    for chain in [7, 26, 27] {
        let i = ctl.find(chain).unwrap();
        let Some(sf) = ctl.records[i].status_fn else {
            continue;
        };
        assert_eq!(act2::status_fn(&ctl, &mut f, i, P1, &pf, sf), None);
    }
    assert!(f.log.is_empty());
}

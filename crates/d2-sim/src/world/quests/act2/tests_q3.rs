// Spec: specs/world/quests-act2.md §5 (Test vectors, Edge cases 7–9)
// Spec: specs/world/quests-act2-2.md §1 items 2, 4, 5, 6, 7, 20
//! Tests for [`super::q3`] (A2Q3 Tainted Sun, chain 10, slot 11).

use super::q3;
use crate::rng::Seed;
use crate::units::{RoomId, UnitId};
use crate::world::quests::tests::*;
use crate::world::quests::*;

const DROGNAN_U: UnitId = UnitId(0x20);
const FARA_U: UnitId = UnitId(0x21);
const ATMA_U: UnitId = UnitId(0x22);
const ALTAR: UnitId = UnitId(0x50);
const P3: UnitId = UnitId(3);
/// Claw Viper Temple level 2 (the altar's level in these tests).
const TEMPLE: u32 = 61;

fn npc(f: &mut Fake, u: UnitId, class: u16) {
    f.monsters.insert(
        u,
        (
            u.0,
            class,
            UnitKind::Monster {
                class: u32::from(class),
                superunique: None,
                owner: None,
            },
        ),
    );
}

/// Chain 10's record and a fake with P1 in Lut Gholein (Act II).
fn setup() -> (QuestControl, Fake, usize) {
    let (ctl, _) = control();
    let mut f = Fake::new();
    f.p(P1).act = Some(1);
    f.p(P1).level = Some(40);
    npc(&mut f, DROGNAN_U, 177);
    npc(&mut f, FARA_U, 178);
    npc(&mut f, ATMA_U, 176);
    let i = ctl.find(10).unwrap();
    (ctl, f, i)
}

fn add_player(f: &mut Fake, u: UnitId, level: u32) {
    f.players.insert(
        u,
        Player {
            guid: u.0,
            act: Some(1),
            level: Some(level),
            ..Player::default()
        },
    );
}

fn ev(ctl: &mut QuestControl, f: &mut Fake, i: usize, args: EventArgs) -> bool {
    q3::callback(ctl, f, i, args, None)
}

fn level(ctl: &mut QuestControl, f: &mut Fake, i: usize, old: u32, new: u32) {
    let args = EventArgs {
        event: event::CHANGED_LEVEL,
        player: Some(P1),
        target: Some(P1),
        a: old,
        b: new,
    };
    assert!(ev(ctl, f, i, args));
}

fn say(ctl: &mut QuestControl, f: &mut Fake, i: usize, n: UnitId, class: u16, msg: u32) {
    let args = EventArgs {
        event: event::SCROLL_MESSAGE,
        player: Some(P1),
        target: Some(n),
        a: u32::from(class),
        b: msg,
    };
    assert!(ev(ctl, f, i, args));
}

fn chat(ctl: &mut QuestControl, f: &mut Fake, i: usize, n: UnitId) -> TextList {
    let mut list = TextList::new();
    let args = EventArgs {
        event: event::NPC_ACTIVATE,
        player: Some(P1),
        target: Some(n),
        ..EventArgs::default()
    };
    assert!(q3::callback(ctl, f, i, args, Some(&mut list)));
    list
}

fn x3(ctl: &QuestControl, i: usize) -> &q3::Extra {
    &ctl.records[i].extra.a2.q3
}

const DARK: &str = "53 05000000 00000000 01";
const LIGHT: &str = "53 02000000 00000000 00";

// Covers: specs/world/quests-act2.md §5.2, §5.3, §5.4
#[test]
fn darken_timer_seed_vector() {
    let (mut ctl, mut f, i) = setup();
    ctl.seed = Seed::new(12345, 666);
    level(&mut ctl, &mut f, i, 40, 44);
    // lo' 22752887: & 1 = 1 → period 16.
    assert_eq!(ctl.seed.lo, 22_752_887);
    assert_eq!(ctl.timers.len(), 1);
    let t = ctl.timers[0];
    assert_eq!(
        (t.func, t.chain, t.period),
        (TimerFn::Act2(super::Timer::Darken), 10, 16)
    );
    assert!(x3(&ctl, i).darken_timer);
    // Old level 40: quick remove (empty list), state 0 stays.
    assert_eq!(ctl.records[i].state, 0);
    // A second entry (45) while the timer exists draws nothing.
    let seed = ctl.seed;
    level(&mut ctl, &mut f, i, 44, 45);
    assert_eq!((ctl.seed, ctl.timers.len()), (seed, 1));
    // First fires 17 updater ticks later.
    f.sent.clear();
    for _ in 0..16 {
        ctl.update(&mut f);
    }
    assert!(f.sent.is_empty() && x3(&ctl, i).darken_timer);
    ctl.update(&mut f);
    assert!(ctl.timers.is_empty());
    // Darken: status 1 to all (F), Act II clients get 0x53 and 5D 0A 10.
    let msgs: Vec<Vec<u8>> = f.sent.iter().map(|m| m.1.clone()).collect();
    assert_eq!(
        msgs,
        [hex("5d 0a 00 01 0000"), hex(DARK), hex("5d 0a 10 00 0000")]
    );
    assert_eq!(f.log, ["sun start 1"]);
    let r = &ctl.records[i];
    assert_eq!((r.state, r.status), (1, 1));
    assert!(x3(&ctl, i).dark && !x3(&ctl, i).darken_timer);
    // Flag iterate: state 1 → 11.2 only.
    assert_eq!(f.flags(P1).word(11), 1 << 2);
}

// Covers: specs/world/quests-act2.md §5.3, §5.4
#[test]
fn darken_triggers_and_act_load() {
    // No timer in an intro game, past state 0, or for other levels.
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].not_intro = false;
    level(&mut ctl, &mut f, i, 40, 45);
    ctl.records[i].not_intro = true;
    ctl.records[i].state = 1;
    level(&mut ctl, &mut f, i, 40, 44);
    ctl.records[i].state = 0;
    level(&mut ctl, &mut f, i, 40, 41);
    assert!(ctl.timers.is_empty());
    // Back in town with darkness pending: darken, flag iterate, pending
    // cleared (state 2 after darken: 11.2 and 11.3).
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].state = 2;
    ctl.records[i].extra.a2.q3.dark_pending = true;
    level(&mut ctl, &mut f, i, 41, 40);
    assert!(!x3(&ctl, i).dark_pending);
    assert_eq!(f.flags(P1).word(11), 0b1100);
    assert_eq!(f.log, ["sun start 1"]);
    // Leaving town: quick remove; state 2 → 3.
    ctl.records[i].guids.add(1);
    level(&mut ctl, &mut f, i, 40, 41);
    assert_eq!(ctl.records[i].state, 3);
    assert!(ctl.records[i].guids.0.is_empty());
    // The timer with darkness already applied does nothing but clear +0x01.
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].extra.a2.q3.dark = true;
    ctl.records[i].extra.a2.q3.darken_timer = true;
    assert!(q3::darken_timer(&mut ctl, &mut f, i));
    assert!(!x3(&ctl, i).darken_timer && f.sent.is_empty());
    // Act load `0x0059AC40`: only n = 1 with darkness pending.
    let (mut ctl, mut f, i) = setup();
    q3::act_load(&mut ctl, &mut f, 1, 1);
    assert!(f.log.is_empty());
    ctl.records[i].extra.a2.q3.dark_pending = true;
    q3::act_load(&mut ctl, &mut f, 1, 0);
    assert!(f.log.is_empty());
    q3::act_load(&mut ctl, &mut f, 1, 1);
    assert_eq!(f.log, ["sun start 1"]);
    assert!(x3(&ctl, i).dark && !x3(&ctl, i).dark_pending);
    assert!(f.sent.is_empty());
}

// Covers: specs/world/quests-act2.md §5.5, §edge-cases-original-bugs r7
#[test]
fn chat_and_wants_to_talk() {
    let (mut ctl, mut f, i) = setup();
    // State 0: nothing.
    assert!(chat(&mut ctl, &mut f, i, DROGNAN_U).is_empty());
    // index[state] (`0x0073A620`: −1, 0, 1, 2, 3, 0).
    for (state, want) in [(1, (348, 0)), (2, (348, 2)), (3, (358, 2))] {
        ctl.records[i].state = state;
        assert_eq!(
            chat(&mut ctl, &mut f, i, DROGNAN_U),
            [want],
            "state {state}"
        );
    }
    ctl.records[i].state = 4;
    assert!(chat(&mut ctl, &mut f, i, DROGNAN_U).is_empty());
    // GUID listed → 4; 11.14 hides it; 11.1 → 3.
    ctl.records[i].state = 1;
    ctl.records[i].guids.add(1);
    assert_eq!(chat(&mut ctl, &mut f, i, DROGNAN_U), [(371, 2)]);
    f.p(P1).quests.flags[0].set(11, 14);
    assert!(chat(&mut ctl, &mut f, i, DROGNAN_U).is_empty());
    f.p(P1).quests.flags[0].set(11, 1);
    assert_eq!(chat(&mut ctl, &mut f, i, DROGNAN_U), [(371, 0)]);
    // Intro or 11.0: nothing from the index.
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].state = 1;
    ctl.records[i].not_intro = false;
    assert!(chat(&mut ctl, &mut f, i, DROGNAN_U).is_empty());
    ctl.records[i].not_intro = true;
    f.p(P1).quests.flags[0].set(11, 0);
    assert!(chat(&mut ctl, &mut f, i, DROGNAN_U).is_empty());
    // Wants to talk: drognan at state 1; with 11.1 the list, not fara.
    let (mut ctl, mut f, i) = setup();
    assert!(!q3::active(&ctl, &mut f, i, P1, 177));
    ctl.records[i].state = 1;
    assert!(q3::active(&ctl, &mut f, i, P1, 177));
    assert!(!q3::active(&ctl, &mut f, i, P1, 176));
    f.p(P1).quests.flags[0].set(11, 1);
    for n in [176, 175, 198, 199, 177, 202, 244, 210, 200, 201] {
        assert!(q3::active(&ctl, &mut f, i, P1, n), "npc {n}");
    }
    assert!(!q3::active(&ctl, &mut f, i, P1, 178)); // fara (edge case 7)
    f.p(P1).quests.flags[0].set(11, 0);
    assert!(!q3::active(&ctl, &mut f, i, P1, 177));
}

// Covers: specs/world/quests-act2.md §5.6
// Covers: specs/world/quests-act2-2.md §1 r7
#[test]
fn messages_start_and_reward() {
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].state = 1;
    ctl.records[i].status = 1;
    // 348 from another NPC: nothing.
    say(&mut ctl, &mut f, i, ATMA_U, 176, 348);
    assert_eq!(ctl.records[i].state, 1);
    say(&mut ctl, &mut f, i, DROGNAN_U, 177, 348);
    let r = &ctl.records[i];
    assert_eq!((r.state, r.status, r.flags), (2, 2, 0));
    assert_eq!(f.flags(P1).word(11), 0b1100);
    assert!(f.sent.is_empty()); // status 2 is silent
                                // 362–372 with 11.13 and 11.1: status 13, state 5, own seq fn, refresh,
                                // 11.0 set and 11.1 cleared, GUID added.
    f.p(P1).quests.flags[0].set(11, 13);
    f.p(P1).quests.flags[0].set(11, 1);
    say(&mut ctl, &mut f, i, ATMA_U, 176, 365);
    let r = &ctl.records[i];
    assert_eq!((r.state, r.status), (5, 13));
    assert!(r.guids.contains(1));
    // seq(10) at state 5 → seq(11): chain 11 state 0 → 1 → seq(13).
    assert_eq!(ctl.record(11).unwrap().state, 1);
    assert_eq!(ctl.record(13).unwrap().state, 1);
    let fl = f.flags(P1);
    assert!(fl.get(11, 0) && !fl.get(11, 1));
    assert_eq!(f.sent_ids(), [0x27, 0x29]);
    assert!(!ctl.game.get(11, 13));
    // A reward message in an intro game: game 11.13 only inside the 11.1
    // block (act2-2 §1 item 7), no state change.
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].not_intro = false;
    f.p(P1).quests.flags[0].set(11, 13);
    say(&mut ctl, &mut f, i, ATMA_U, 176, 362);
    assert_eq!(ctl.records[i].state, 0);
    assert!(!ctl.game.get(11, 13));
    assert!(!f.flags(P1).get(11, 0));
    f.p(P1).quests.flags[0].set(11, 1);
    say(&mut ctl, &mut f, i, ATMA_U, 176, 362);
    assert_eq!(ctl.records[i].state, 0);
    assert!(ctl.game.get(11, 13));
    assert!(f.flags(P1).get(11, 0));
    // 373 is not a Tainted Sun message.
    let (mut ctl, mut f, i) = setup();
    say(&mut ctl, &mut f, i, ATMA_U, 176, 373);
    assert!(f.sent.is_empty() && f.log.is_empty());
}

// Covers: specs/world/quests-act2.md §5.6, §5.2
// Covers: specs/world/quests-act2-2.md §1 r2, §1 r6
#[test]
fn game_start_and_leave() {
    // 11.0 → game 11.13 only.
    let (mut ctl, mut f, i) = setup();
    f.p(P1).quests.flags[0].set(11, 0);
    let start = EventArgs {
        event: event::PLAYER_STARTED_GAME,
        player: Some(P1),
        target: Some(P1),
        ..EventArgs::default()
    };
    ev(&mut ctl, &mut f, i, start);
    assert!(ctl.game.get(11, 13));
    assert!(f.log.is_empty());
    // No 11.2: nothing.
    let (mut ctl, mut f, i) = setup();
    ev(&mut ctl, &mut f, i, start);
    assert!(f.sent.is_empty() && ctl.records[i].state == 0);
    // 11.2 / 11.3 / 11.4: darken, then the status and state.
    for (bits, want) in [(&[2][..], (1, 1)), (&[2, 3], (2, 2)), (&[2, 4], (2, 3))] {
        let (mut ctl, mut f, i) = setup();
        for &b in bits {
            f.p(P1).quests.flags[0].set(11, b);
        }
        ev(&mut ctl, &mut f, i, start);
        let r = &ctl.records[i];
        assert_eq!((r.status, r.state), want, "bits {bits:?}");
        assert!(x3(&ctl, i).dark);
        assert_eq!(f.log, ["sun start 1"]);
        assert_eq!(f.sent[1].1, hex(DARK));
    }
    // Event 10: both lists (the record list needs 11.0 and 11.1, act2-2
    // §1 item 2).
    let (mut ctl, mut f, i) = setup();
    f.p(P1).quests.flags[0].set(11, 0);
    f.p(P1).quests.flags[0].set(11, 1);
    ctl.records[i].guids.add(1);
    ctl.records[i].extra.a2.q3.list.add(1);
    ctl.records[i].extra.a2.q3.list.add(9);
    let leave = EventArgs {
        event: event::PLAYER_LEAVES_GAME,
        ..start
    };
    ev(&mut ctl, &mut f, i, leave);
    assert!(ctl.records[i].guids.0.is_empty());
    assert_eq!(x3(&ctl, i).list.0, [9]);
    // Event 2 has no body.
    let end = EventArgs {
        event: event::NPC_DEACTIVATE,
        ..start
    };
    assert!(!ev(&mut ctl, &mut f, i, end));
}

/// The altar at (10, 20) in room 7 of the temple; P2 on the same level,
/// P3 (P1's party) in town.
fn altar_setup() -> (QuestControl, Fake, usize) {
    let (mut ctl, mut f, i) = setup();
    ctl.seed = Seed::new(12345, 666);
    f.objects.insert(ALTAR, (0x50, 149, 0));
    f.unit_levels.insert(ALTAR, TEMPLE);
    f.pos.insert(ALTAR, (10, 20, RoomId(7)));
    f.p(P1).level = Some(TEMPLE);
    add_player(&mut f, P2, TEMPLE);
    add_player(&mut f, P3, 40);
    f.party.insert(P1, vec![P1, P3]);
    ctl.records[i].state = 3;
    ctl.records[i].status = 2;
    (ctl, f, i)
}

// Covers: specs/world/quests-act2.md §5.7 r3, §1.3, §edge-cases-original-bugs r8
// Covers: specs/world/quests-act2-2.md §1 r4, §1 r20
#[test]
fn altar_operate_vector() {
    let (mut ctl, mut f, i) = altar_setup();
    ctl.records[i].extra.a2.q3.dark = true;
    f.p(P2).items.push(*b"hst "); // no amulet for P2
                                  // The altar never calls the quest-chest gate (act2-2 §1 item 4).
    f.gate_closed = true;
    assert_eq!(q3::altar_operate(&mut ctl, &mut f, ALTAR, P1), 0);
    // Act II clients get the 0x53; every player 0x28 then `89 06`.
    let light = hex(LIGHT);
    let msgs: Vec<(u32, Vec<u8>)> = f
        .sent
        .iter()
        .map(|m| {
            (
                m.0 .0,
                if m.1[0] == 0x28 {
                    vec![0x28]
                } else {
                    m.1.clone()
                },
            )
        })
        .collect();
    assert_eq!(
        msgs,
        [
            (1, light.clone()),
            (2, light.clone()),
            (3, light),
            (1, vec![0x28]),
            (1, hex("89 06")),
            (2, vec![0x28]),
            (2, hex("89 06")),
            (3, vec![0x28]),
            (3, hex("89 06")),
        ]
    );
    // Amulets: P1 and P3 qualify; quality 7, the level computed by the
    // drop (`&level` is an out parameter, edge case 8 and act2-2 §1 item
    // 20), identified; no quest-chest gate (act2-2 §1 item 4); treasure;
    // 7 gold piles (lo' 22752887 mod 5 = 2).
    let mut want = vec![
        "mode 80 1".to_string(),
        "sun end".into(),
        "sound 1 52".into(),
        "sound 2 52".into(),
        "sound 3 52".into(),
        "qdrop 80 vip  7 None false".into(),
        "identify 600".into(),
        "qdrop 80 vip  7 None false".into(),
        "identify 601".into(),
        "treasure 80 4".into(),
    ];
    want.extend(vec!["gold 80".to_string(); 7]);
    assert_eq!(f.log, want);
    let x = x3(&ctl, i);
    assert_eq!(
        (
            x.altar_room,
            x.altar_level,
            x.altar_mode,
            x.altar_guid,
            x.amulets
        ),
        (Some(RoomId(7)), TEMPLE, 2, 0x50, 2)
    );
    assert!(x.altar_seen && x.altar_destroyed && !x.dark);
    assert_eq!(ctl.record(9).unwrap().extra.a2.q2.amulet_count, 2);
    assert_eq!((ctl.records[i].state, ctl.fx), (4, 6));
    assert!(ctl.game.get(11, 13));
    // P1 (operator), P2 (same level), P3 (P1's party in Act II): 11.13, 11.1.
    for p in [P1, P2, P3] {
        assert_eq!(f.flags(p).word(11), 1 << 13 | 1 << 1, "player {}", p.0);
    }
    // The status timer, period 10: state 4 → status 3 to all.
    let t = ctl.timers[0];
    assert_eq!(
        (t.func, t.period),
        (TimerFn::Act2(super::Timer::AltarStatus), 10)
    );
    f.sent.clear();
    ctl.records[i].extra.a2.q3.status_timer = true;
    assert!(q3::altar_timer(&mut ctl, &mut f, i));
    assert_eq!(ctl.records[i].status, 3);
    assert!(!x3(&ctl, i).status_timer);
    // F: 11.13 → each player gets 5D (default status at state 4 = init_no,
    // 11.13 set → the status byte).
    assert_eq!(f.sent.len(), 3);
    assert_eq!(f.sent[0].1, hex("5d 0a 00 03 0000"));
}

// Covers: specs/world/quests-act2.md §5.7 r1, §5.7 r2, §5.7 r4
#[test]
fn altar_refusals_and_intro() {
    // Step 1: 11.1 and holding `vip ` → sound 19, nothing else.
    let (mut ctl, mut f, i) = altar_setup();
    f.p(P1).quests.flags[0].set(11, 1);
    f.p(P1).items.push(*b"vip ");
    assert_eq!(q3::altar_operate(&mut ctl, &mut f, ALTAR, P1), 0);
    assert_eq!(f.log, ["sound 1 19"]);
    assert_eq!(ctl.records[i].state, 3);
    // 11.0 with 10.0 → refused too; 11.0 alone is not.
    let (mut ctl, mut f, _) = altar_setup();
    f.p(P1).quests.flags[0].set(11, 0);
    f.p(P1).quests.flags[0].set(10, 0);
    q3::altar_operate(&mut ctl, &mut f, ALTAR, P1);
    assert_eq!(f.log, ["sound 1 19"]);
    // Step 2: object mode ≠ 0 → nothing.
    let (mut ctl, mut f, i) = altar_setup();
    f.objects.get_mut(&ALTAR).unwrap().2 = 1;
    q3::altar_operate(&mut ctl, &mut f, ALTAR, P1);
    assert!(f.log.is_empty() && f.sent.is_empty() && ctl.records[i].state == 3);
    // Step 4: intro → mode, drops, treasure, gold; no state, flags, FX or
    // timer.
    let (mut ctl, mut f, i) = altar_setup();
    ctl.records[i].not_intro = false;
    f.p(P1).quests.flags[0].set(10, 0); // P1 lacks nothing but has 10.0
    q3::altar_operate(&mut ctl, &mut f, ALTAR, P1);
    assert_eq!(f.log[0], "mode 80 1");
    assert_eq!(
        f.log
            .iter()
            .filter(|l| l.starts_with("qdrop 80 vip  7 None"))
            .count(),
        2
    );
    assert_eq!(f.log.iter().filter(|l| *l == "gold 80").count(), 7);
    assert!(f.log.contains(&"treasure 80 4".to_string()));
    assert!(f.sent.is_empty() && ctl.timers.is_empty());
    assert_eq!((ctl.records[i].state, ctl.fx), (3, 0));
    let x = x3(&ctl, i);
    assert!(x.altar_destroyed && x.altar_mode == 2 && !x.altar_seen);
    assert_eq!(ctl.record(9).unwrap().extra.a2.q2.amulet_count, 2);
    assert_eq!(f.flags(P2).word(11), 0);
    assert!(!ctl.game.get(11, 13));
}

// Covers: specs/world/quests-act2-2.md §1 r5
#[test]
fn altar_init_status_test_outside_the_state_block() {
    // State ≥ 2 with status 0 (unreachable in 1.14d, kept): the status
    // test still runs inside not-intro → status 2 to all, no darken.
    let (mut ctl, mut f, i) = altar_setup();
    ctl.records[i].state = 3;
    ctl.records[i].status = 0;
    ctl.records[i].flags = 5;
    q3::altar_init(&mut ctl, &mut f, ALTAR);
    let r = &ctl.records[i];
    assert_eq!((r.state, r.status, r.flags), (3, 2, 0));
    assert!(!f.log.iter().any(|l| l.starts_with("sun")));
    assert!(f.sent.iter().any(|m| m.1 == hex("5d 0a 00 02 0000")));
    // An intro record: neither block.
    let (mut ctl, mut f, i) = altar_setup();
    ctl.records[i].not_intro = false;
    ctl.records[i].state = 0;
    ctl.records[i].status = 0;
    q3::altar_init(&mut ctl, &mut f, ALTAR);
    let r = &ctl.records[i];
    assert_eq!((r.state, r.status), (0, 0));
    assert!(f.sent.is_empty());
}

// Covers: specs/world/quests-act2.md §5.8, §edge-cases-original-bugs r9
#[test]
fn altar_init_reaches_state_3() {
    let (mut ctl, mut f, i) = altar_setup();
    ctl.records[i].state = 0;
    ctl.records[i].status = 0;
    ctl.records[i].flags = 5;
    q3::altar_init(&mut ctl, &mut f, ALTAR);
    let r = &ctl.records[i];
    // Darken (status 1 to all) makes the status-0 test false.
    assert_eq!((r.state, r.status, r.flags), (3, 1, 0));
    let x = x3(&ctl, i);
    assert!(x.altar_seen && x.dark && x.altar_guid == 0x50);
    assert_eq!(f.log, ["sun start 1", "mode 80 0"]);
    // Again (state 3): only the mode from +0x08.
    ctl.records[i].extra.a2.q3.altar_mode = 2;
    f.log.clear();
    f.sent.clear();
    q3::altar_init(&mut ctl, &mut f, ALTAR);
    assert_eq!(f.log, ["mode 80 2"]);
    assert!(f.sent.is_empty());
    // No chain 10: mode 2 unless already 2.
    let (mut ctl, mut f, i) = altar_setup();
    ctl.records.remove(i);
    q3::altar_init(&mut ctl, &mut f, ALTAR);
    assert_eq!(f.log, ["mode 80 2"]);
    f.log.clear();
    q3::altar_init(&mut ctl, &mut f, ALTAR);
    assert!(f.log.is_empty());
}

// Covers: specs/world/quests-act2.md §5.1
#[test]
fn extra_data_at_init_and_the_event_10_list() {
    let (mut ctl, mut f, i) = setup();
    // Init: everything off, altar mode 0 (neutral), the list empty.
    let e = &ctl.records[i].extra.a2.q3;
    assert_eq!(e, &q3::Extra::default());
    assert_eq!((e.altar_mode, e.list.0.len()), (0, 0));
    add_player(&mut f, P3, 40);
    for g in [P1.0, P3.0] {
        ctl.records[i].extra.a2.q3.list.add(g);
    }
    // Other events leave the +0x14 list alone.
    level(&mut ctl, &mut f, i, 40, 41);
    level(&mut ctl, &mut f, i, 41, 40);
    let start = EventArgs {
        event: event::PLAYER_STARTED_GAME,
        player: Some(P3),
        target: Some(P3),
        ..EventArgs::default()
    };
    assert!(ev(&mut ctl, &mut f, i, start));
    assert_eq!(ctl.records[i].extra.a2.q3.list.0, [P1.0, P3.0]);
    // Event 10 removes the leaving player.
    let leave = EventArgs {
        event: event::PLAYER_LEAVES_GAME,
        player: Some(P1),
        target: Some(P1),
        ..EventArgs::default()
    };
    assert!(ev(&mut ctl, &mut f, i, leave));
    assert_eq!(ctl.records[i].extra.a2.q3.list.0, [P3.0]);
}

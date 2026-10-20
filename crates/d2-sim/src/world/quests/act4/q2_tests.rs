// Spec: specs/world/quests-act4.md §5 (Test vectors, Edge cases)
//! A4Q2 Terror's End callback by callback, from the spec's test vectors
//! and rules, on the quests' fake world. Callbacks are called on chain
//! 23's record directly so the other Act IV records stay out of the
//! logs.

use super::super::super::tests::*;
use super::super::super::{act1, object_event, HostRequest, QuestChain, UnitKind};
use super::*;
use crate::units::RoomId;

const TYRAEL_U: UnitId = UnitId(0x20);
const CAIN_U: UnitId = UnitId(0x21);
const DIABLO_U: UnitId = UnitId(0x22);
const P3: UnitId = UnitId(3);
const ROOM: RoomId = RoomId(1);

fn kind(class: u16, superunique: Option<u32>) -> UnitKind {
    UnitKind::Monster {
        class: u32::from(class),
        superunique,
        owner: None,
    }
}

fn act4_player(guid: u32) -> Player {
    Player {
        guid,
        act: Some(3),
        level: Some(FORTRESS),
        ..Player::default()
    }
}

/// The fake with P1 in the Fortress, Tyrael, Cain and Diablo.
fn fake() -> Fake {
    let mut f = Fake::new();
    f.players.insert(P1, act4_player(1));
    for (u, class) in [
        (TYRAEL_U, npc::TYRAEL2),
        (CAIN_U, CAIN4),
        (DIABLO_U, DIABLO),
    ] {
        f.monsters.insert(u, (u.0, class, kind(class, None)));
    }
    f.chains.insert(DIABLO_U, QuestChain(vec![CHAIN]));
    f.rooms.insert(ROOM, (0, 0, 2000, 2000));
    f
}

fn ix(ctl: &QuestControl) -> usize {
    ctl.find(CHAIN).unwrap()
}

fn xd(ctl: &QuestControl) -> &Extra {
    &ctl.records[ix(ctl)].extra.a4.q2
}

/// One event to chain 23's record only.
fn ev(ctl: &mut QuestControl, f: &mut Fake, args: EventArgs) {
    let i = ix(ctl);
    act1::callback(ctl, f, i, args, None, false);
}

/// Event 11 from NPC unit `n` of class `class`.
fn say(ctl: &mut QuestControl, f: &mut Fake, p: UnitId, n: UnitId, msg: u32) {
    let class = f.monsters[&n].1;
    let args = EventArgs {
        event: event::SCROLL_MESSAGE,
        target: Some(n),
        player: Some(p),
        a: u32::from(class),
        b: msg,
    };
    ev(ctl, f, args);
}

/// Event 0: the lines chain 23 adds for NPC `n`.
fn text(ctl: &mut QuestControl, f: &mut Fake, n: UnitId) -> TextList {
    let mut list = TextList::new();
    let args = EventArgs {
        event: event::NPC_ACTIVATE,
        target: Some(n),
        player: Some(P1),
        ..EventArgs::default()
    };
    let i = ix(ctl);
    act1::callback(ctl, f, i, args, Some(&mut list), false);
    list
}

fn set(f: &mut Fake, p: UnitId, slot: u8, bits: &[u8]) {
    let d = usize::from(f.difficulty);
    for &b in bits {
        f.p(p).quests.flags[d].set(slot, b);
    }
}

// ------------------------------------------------------------ §5.2

// Covers: specs/world/quests-act4.md §5.2 text, §5.2 r1, §5.2 r2, §5.2 r3
#[test]
fn chat_lists_by_bits_and_game_type() {
    let (mut ctl, _) = control();
    // Classic, 26.7: Tyrael state 2, Cain 3, others nothing.
    let mut f = fake();
    f.expansion = false;
    set(&mut f, P1, SLOT, &[7]);
    assert_eq!(text(&mut ctl, &mut f, TYRAEL_U), [(684, 0)]);
    assert_eq!(text(&mut ctl, &mut f, CAIN_U), [(685, 2)]);
    assert_eq!(text(&mut ctl, &mut f, AKARA_U), []);
    // Classic, 26.6: Cain state 2, Tyrael 3.
    let mut f = fake();
    f.expansion = false;
    set(&mut f, P1, SLOT, &[6]);
    assert_eq!(text(&mut ctl, &mut f, CAIN_U), [(685, 0)]);
    assert_eq!(text(&mut ctl, &mut f, TYRAEL_U), [(684, 2)]);
    // Classic with 26.0 only: nothing.
    let mut f = fake();
    f.expansion = false;
    set(&mut f, P1, SLOT, &[0]);
    assert_eq!(text(&mut ctl, &mut f, TYRAEL_U), []);
    // Expansion, 26.0 with 26.8 and 26.9 clear: Tyrael 4 then 5, Cain 5
    // then 4.
    let mut f = fake();
    set(&mut f, P1, SLOT, &[0]);
    assert_eq!(text(&mut ctl, &mut f, TYRAEL_U), [(20000, 0), (20000, 2)]);
    assert_eq!(text(&mut ctl, &mut f, CAIN_U), [(20001, 2), (20001, 0)]);
    // 26.9 set: only the 26.8 step.
    set(&mut f, P1, SLOT, &[9]);
    assert_eq!(text(&mut ctl, &mut f, TYRAEL_U), [(20000, 2)]);
    assert_eq!(text(&mut ctl, &mut f, CAIN_U), [(20001, 0)]);
    // 26.0 clear: the index table 0x0073D56C by state.
    let mut f = fake();
    let i = ix(&ctl);
    for (state, tyrael, cain) in [
        (0, vec![], vec![]),
        (1, vec![(681, 0)], vec![]),
        (2, vec![(683, 2)], vec![(682, 2)]),
        (3, vec![], vec![]),
    ] {
        ctl.records[i].state = state;
        assert_eq!(text(&mut ctl, &mut f, TYRAEL_U), tyrael, "state {state}");
        assert_eq!(text(&mut ctl, &mut f, CAIN_U), cain, "state {state}");
    }
    // Not-intro clear: nothing.
    ctl.records[i].state = 1;
    ctl.records[i].not_intro = false;
    assert_eq!(text(&mut ctl, &mut f, TYRAEL_U), []);
}

// Covers: specs/world/quests-act4.md §5.2 text
#[test]
fn wants_to_talk() {
    let (mut ctl, _) = control();
    let i = ix(&ctl);
    let mut f = fake();
    let act = |ctl: &QuestControl, f: &mut Fake, c: u16| active(ctl, f, i, P1, c, 0x005B_4450);
    assert!(!act(&ctl, &mut f, npc::TYRAEL2)); // state 0
    ctl.records[i].state = 1;
    assert!(act(&ctl, &mut f, npc::TYRAEL2));
    assert!(!act(&ctl, &mut f, CAIN4));
    set(&mut f, P1, SLOT, &[15]);
    assert!(!act(&ctl, &mut f, npc::TYRAEL2));
    // Expansion, 26.0: Tyrael until 26.9, Cain until 26.8.
    let mut f = fake();
    set(&mut f, P1, SLOT, &[0]);
    assert!(act(&ctl, &mut f, npc::TYRAEL2) && act(&ctl, &mut f, CAIN4));
    set(&mut f, P1, SLOT, &[8, 9]);
    assert!(!act(&ctl, &mut f, npc::TYRAEL2) && !act(&ctl, &mut f, CAIN4));
    // Classic, 26.0: Tyrael with 26.7, Cain with 26.6.
    let mut f = fake();
    f.expansion = false;
    set(&mut f, P1, SLOT, &[0]);
    assert!(!act(&ctl, &mut f, npc::TYRAEL2) && !act(&ctl, &mut f, CAIN4));
    set(&mut f, P1, SLOT, &[6, 7]);
    assert!(act(&ctl, &mut f, npc::TYRAEL2) && act(&ctl, &mut f, CAIN4));
    assert!(!act(&ctl, &mut f, npc::AKARA));
}

// Covers: specs/world/quests-act4.md §5.2 text, §5.3, §edge-cases-original-bugs r1
#[test]
fn tyrael_starts_the_quest() {
    let (mut ctl, _) = control();
    let i = ix(&ctl);
    let mut f = fake();
    f.players.insert(P2, act4_player(2));
    set(&mut f, P2, SLOT, &[0]);
    ctl.records[i].state = 3; // not state-guarded (edge case 1)
    say(&mut ctl, &mut f, P1, TYRAEL_U, 681);
    assert_eq!(ctl.records[i].state, 2);
    assert!(xd(&ctl).started);
    // The flag iterate: P1 gets 26.2, P2 (26.0) nothing.
    assert!(f.flags(P1).get(SLOT, 2) && !f.flags(P2).get(SLOT, 2));
    // Refresh: Tyrael's text again (0x27, then 0x29).
    assert_eq!(f.sent_ids(), [0x27, 0x29]);
    // Chat end with Cain: nothing; with Tyrael: status 1 to all, once.
    f.sent.clear();
    let end = |n| EventArgs {
        event: event::NPC_DEACTIVATE,
        target: Some(n),
        player: Some(P1),
        ..EventArgs::default()
    };
    ev(&mut ctl, &mut f, end(CAIN_U));
    assert!(f.sent.is_empty() && xd(&ctl).started);
    ev(&mut ctl, &mut f, end(TYRAEL_U));
    assert_eq!(ctl.records[i].status, 1);
    assert!(!xd(&ctl).started);
    // F: P1 (no 26.0, no 26.15) gets the 0x5D; P2 has 26.0.
    assert_eq!(f.sent, [(P1, hex("5D 17 00 01 0000"))]);
    f.sent.clear();
    ev(&mut ctl, &mut f, end(TYRAEL_U));
    assert!(f.sent.is_empty());
}

// Covers: specs/world/quests-act4.md §5.2 text
#[test]
fn completion_talk_messages() {
    let (mut ctl, _) = control();
    let mut f = fake();
    f.expansion = false;
    set(&mut f, P1, SLOT, &[6, 7]);
    say(&mut ctl, &mut f, P1, TYRAEL_U, 684);
    assert!(!f.flags(P1).get(SLOT, 7) && f.flags(P1).get(SLOT, 6));
    say(&mut ctl, &mut f, P1, CAIN_U, 685);
    assert!(!f.flags(P1).get(SLOT, 6));
    // 20001 sets 26.8 in any game type; 20000 is expansion only.
    say(&mut ctl, &mut f, P1, CAIN_U, 20001);
    assert!(f.flags(P1).get(SLOT, 8));
    say(&mut ctl, &mut f, P1, TYRAEL_U, 20000);
    assert!(!f.flags(P1).get(SLOT, 9));
    assert!(f.log.is_empty() && f.sent.is_empty());
}

// ------------------------------------------------------------ §5.3

// Covers: specs/world/quests-act4.md §5.3, §edge-cases-original-bugs r13
#[test]
fn level_changes_move_the_state() {
    let (mut ctl, _) = control();
    let i = ix(&ctl);
    let lv = |a, b| EventArgs {
        event: event::CHANGED_LEVEL,
        target: Some(P1),
        player: Some(P1),
        a,
        b,
    };
    // Leaving the Fortress at state 2: state 3, status 1 (silent), 26.3.
    let mut f = fake();
    ctl.records[i].state = 2;
    ctl.records[i].flags = 7;
    ev(&mut ctl, &mut f, lv(FORTRESS, 104));
    let r = &ctl.records[i];
    assert_eq!((r.state, r.status, r.flags), (3, 1, 0));
    assert!(f.flags(P1).get(SLOT, 3));
    assert!(f.sent.is_empty());
    // A player with 26.0 stops the walk.
    let mut f = fake();
    set(&mut f, P1, SLOT, &[0]);
    ctl.records[i].state = 2;
    ctl.records[i].status = 0;
    ev(&mut ctl, &mut f, lv(FORTRESS, 104));
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (2, 0));
    // Entering the Chaos Sanctum: state 3, status 1 to all, 26.3; the
    // state never passes 3.
    let mut f = fake();
    ev(&mut ctl, &mut f, lv(107, SANCTUM));
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (3, 1));
    assert_eq!(f.sent, [(P1, hex("5D 17 00 01 0000"))]);
    assert!(f.flags(P1).get(SLOT, 3));
    // Status 2 already: no 0x5D; status ≠ 1 → 26.4.
    let mut f = fake();
    ctl.records[i].status = 2;
    ev(&mut ctl, &mut f, lv(107, SANCTUM));
    assert!(f.sent.is_empty() && f.flags(P1).get(SLOT, 4));
    assert_eq!(ctl.records[i].state, 3);
    // Not-intro clear: nothing.
    let mut f = fake();
    ctl.records[i].not_intro = false;
    ev(&mut ctl, &mut f, lv(107, SANCTUM));
    assert_eq!(f.flags(P1).word(SLOT), 0);
}

// Covers: specs/world/quests-act4.md §5.3, §edge-cases-original-bugs r15
#[test]
fn start_and_join_restore() {
    let at = |e| EventArgs {
        event: e,
        target: Some(P1),
        player: Some(P1),
        ..EventArgs::default()
    };
    let (mut ctl, _) = control();
    let i = ix(&ctl);
    // 26.9 without 28.0 is forgotten (expansion), in events 13 and 14.
    for e in [event::PLAYER_STARTED_GAME, event::PLAYER_JOINED_GAME] {
        let mut f = fake();
        set(&mut f, P1, SLOT, &[9]);
        ev(&mut ctl, &mut f, at(e));
        assert!(!f.flags(P1).get(SLOT, 9));
        let mut f = fake();
        set(&mut f, P1, SLOT, &[9]);
        set(&mut f, P1, 28, &[0]);
        ev(&mut ctl, &mut f, at(e));
        assert!(f.flags(P1).get(SLOT, 9));
        let mut f = fake();
        f.expansion = false;
        set(&mut f, P1, SLOT, &[9]);
        ev(&mut ctl, &mut f, at(e));
        assert!(f.flags(P1).get(SLOT, 9));
    }
    // Event 13 restores from the first of 26.4, 26.3, 26.2.
    for (bits, want) in [
        (vec![2, 3, 4], (3, 2)),
        (vec![2, 3], (3, 1)),
        (vec![2], (2, 1)),
        (vec![0, 4], (0, 0)),
        (vec![15, 4], (0, 0)),
    ] {
        let mut f = fake();
        ctl.records[i].state = 0;
        ctl.records[i].status = 0;
        set(&mut f, P1, SLOT, &bits);
        ev(&mut ctl, &mut f, at(event::PLAYER_STARTED_GAME));
        let r = &ctl.records[i];
        assert_eq!((r.state, r.status), want, "{bits:?}");
    }
    // Event 14 restores nothing.
    let mut f = fake();
    ctl.records[i].state = 0;
    set(&mut f, P1, SLOT, &[4]);
    ev(&mut ctl, &mut f, at(event::PLAYER_JOINED_GAME));
    assert_eq!(ctl.records[i].state, 0);
}

// ------------------------------------------------------------ §5.4

const SEAL: UnitId = UnitId(0x40);
const DUMMY_U: UnitId = UnitId(0x41);

fn seal_fake(class: u16) -> Fake {
    let mut f = fake();
    f.frame = 100;
    f.q2_fc1 = 20;
    f.objects.insert(SEAL, (0x40, class, 0));
    f.pos.insert(SEAL, (1000, 1000, ROOM));
    f
}

// Covers: specs/world/quests-act4.md §5.4, §1.4, §edge-cases-original-bugs r20
#[test]
fn seal_392_spawns_its_boss() {
    // Test vector: seal 392 at (1000, 1000), free → dummy 131 at (988,
    // 948); its event 7 spawns superunique 36.
    let (mut ctl, _) = control();
    let mut f = seal_fake(392);
    f.spot = Some((0, 0));
    f.object_spawns = vec![Some(DUMMY_U)];
    infector_seal_operate(&mut ctl, &mut f, SEAL, P1, 392);
    assert_eq!(
        f.log,
        [
            "spot at 988 948 3 0x3f11 13 100",
            "spawn object 131 988 948",
            "room portal RoomId(1) false",
            "mode 64 1",
            "event1 64 140",
        ]
    );
    assert_eq!(xd(&ctl).bosses, [(988, 948), (0, 0), (0, 0)]);
    assert_eq!(xd(&ctl).seals, [true, false, false, false, false]);
    // A second operate: the seal is in mode 1.
    f.log.clear();
    infector_seal_operate(&mut ctl, &mut f, SEAL, P1, 392);
    assert!(f.log.is_empty());
    // The dummy's event 7 (class 131 in level 108, `quests.md` §9.5).
    let mut f = fake();
    f.frame = 500;
    f.objects.insert(DUMMY_U, (0x41, DUMMY, 1));
    f.pos.insert(DUMMY_U, (988, 948, ROOM));
    f.players.insert(
        DUMMY_U,
        Player {
            level: Some(SANCTUM),
            ..Player::default()
        },
    );
    f.q2_superuniques = vec![Some(UnitId(0x50))];
    object_event(&mut ctl, &mut f, DUMMY_U, DUMMY);
    assert_eq!(f.log, ["mode 65 2", "superunique 36 988 948 at 65 arg 2"]);
    // Spawn fails: event 7 again at f + 10.
    f.log.clear();
    dummy_event(&mut ctl, &mut f, DUMMY_U);
    assert_eq!(
        f.log,
        ["superunique 36 988 948 at 65 arg 2", "event7 65 510"]
    );
    // Not at a pair: nothing.
    f.log.clear();
    f.pos.insert(DUMMY_U, (988, 949, ROOM));
    dummy_event(&mut ctl, &mut f, DUMMY_U);
    assert!(f.log.is_empty());
}

// Covers: specs/world/quests-act4.md §5.4
#[test]
fn boss_seals_offsets_and_failures() {
    for (op, class, k, spot, r, su) in [
        (
            de_seis_seal_operate::<Fake> as fn(&mut QuestControl, &mut Fake, UnitId, UnitId, u16),
            394,
            1,
            "spot at 961 1033 3 0x3f11 14 100",
            (961, 1033),
            37,
        ),
        (
            vizier_seal_operate::<Fake>,
            396,
            2,
            "spot at 1032 1016 3 0x3f11 15 100",
            (1032, 1016),
            38,
        ),
    ] {
        let (mut ctl, _) = control();
        // No free spot: the seal stays in mode 0, the pair keeps seal +
        // offset.
        let mut f = seal_fake(class);
        op(&mut ctl, &mut f, SEAL, P1, class);
        assert_eq!(f.log, [spot]);
        assert_eq!(xd(&ctl).bosses[k], r);
        assert_eq!(f.object_mode(SEAL), 0);
        // The free spot moved by (+2, −1): the pair follows; no object →
        // still mode 0.
        f.log.clear();
        f.spot = Some((2, -1));
        op(&mut ctl, &mut f, SEAL, P1, class);
        let moved = (r.0 + 2, r.1 - 1);
        assert_eq!(xd(&ctl).bosses[k], moved);
        assert_eq!(f.object_mode(SEAL), 0);
        // Created: refresh and the activation; its dummy spawns `su`.
        f.object_spawns = vec![Some(DUMMY_U)];
        op(&mut ctl, &mut f, SEAL, P1, class);
        assert_eq!(f.object_mode(SEAL), 1);
        assert!(xd(&ctl).seals[usize::from(class - 392)]);
        f.log.clear();
        f.pos.insert(DUMMY_U, (moved.0, moved.1, ROOM));
        f.q2_superuniques = vec![Some(UnitId(0x50))];
        dummy_event(&mut ctl, &mut f, DUMMY_U);
        assert_eq!(
            f.log,
            [format!(
                "superunique {su} {} {} at 65 arg 2",
                moved.0, moved.1
            )]
        );
    }
    // No chain 23: the boss seal does nothing at all.
    let (mut ctl, _) = control();
    ctl.records.retain(|r| r.chain != CHAIN);
    let mut f = seal_fake(392);
    f.spot = Some((0, 0));
    infector_seal_operate(&mut ctl, &mut f, SEAL, P1, 392);
    assert!(f.log.is_empty());
    dummy_event(&mut ctl, &mut f, SEAL);
    assert!(f.log.is_empty());
}

// Covers: specs/world/quests-act4.md §5.4
#[test]
fn plain_seal_activation() {
    let (mut ctl, _) = control();
    let mut f = seal_fake(393);
    seal_operate(&mut ctl, &mut f, SEAL, P1, 393);
    assert_eq!(f.log, ["mode 64 1", "event1 64 140"]);
    assert_eq!(xd(&ctl).seals, [false, true, false, false, false]);
    f.log.clear();
    seal_operate(&mut ctl, &mut f, SEAL, P1, 393);
    assert!(f.log.is_empty());
    // Without chain 23 the seal still animates.
    let (mut ctl, _) = control();
    ctl.records.retain(|r| r.chain != CHAIN);
    let mut f = seal_fake(395);
    seal_operate(&mut ctl, &mut f, SEAL, P1, 395);
    assert_eq!(f.log, ["mode 64 1", "event1 64 140"]);
}

// ------------------------------------------------------------ §5.5–§5.6

const START: UnitId = UnitId(0x60);

/// Opens the five seals (plain activations of fresh seal objects).
fn open_seals(ctl: &mut QuestControl, f: &mut Fake) {
    for class in 392..=396u16 {
        let o = UnitId(0x80 + u32::from(class - 392));
        f.objects.insert(o, (o.0, class, 0));
        seal_operate(ctl, f, o, P1, class);
    }
}

/// Kills the seal boss number `n` (a linked, forced superunique).
fn kill_boss(ctl: &mut QuestControl, f: &mut Fake, n: u32) {
    let u = UnitId(0x90 + n);
    f.monsters
        .insert(u, (u.0, 700 + n as u16, kind(700, Some(36 + n))));
    f.chains.insert(u, QuestChain(vec![CHAIN]));
    ctl.monster_killed(f, u, Some(P1));
}

fn sanctum_fake() -> Fake {
    let mut f = fake();
    for (u, class) in [(0xA0, 300), (0xA1, 301), (0xA2, 302)] {
        let u = UnitId(u);
        f.monsters.insert(u, (u.0, class, kind(class, None)));
    }
    f.q2_level_monsters = vec![UnitId(0xA0), UnitId(0xA1), UnitId(0xA2), DIABLO_U];
    f.q2_dead = vec![UnitId(0xA1)];
    f.q2_align.insert(UnitId(0xA2), 1);
    f.objects.insert(START, (0x60, 255, 0));
    f.pos.insert(START, (700, 800, ROOM));
    f
}

// Covers: specs/world/quests-act4.md §5.5, §5.6, §5.7, §8
#[test]
fn third_boss_kill_clears_and_spawns_diablo() {
    // Test vector: seals 392–396 opened, then the 3 seal bosses killed →
    // 3rd kill: FX 12, evil level-108 monsters killed, timer; Diablo at
    // the 10th firing (T + 20 updater ticks).
    let (mut ctl, _) = control();
    let mut f = sanctum_fake();
    open_seals(&mut ctl, &mut f);
    start_point_init(&mut ctl, &mut f, START);
    assert!(xd(&ctl).start_known && xd(&ctl).start_guid == 0x60);
    kill_boss(&mut ctl, &mut f, 0);
    kill_boss(&mut ctl, &mut f, 1);
    assert!(ctl.timers.is_empty() && !sanctum_cleared(&ctl));
    f.log.clear();
    f.sent.clear();
    kill_boss(&mut ctl, &mut f, 2);
    assert_eq!(xd(&ctl).kills, 3);
    assert!(sanctum_cleared(&ctl));
    assert_eq!(f.log, ["level monsters 108", "remove 160"]);
    assert_eq!(f.sent_ids(), [0x28, 0x89]);
    assert_eq!(f.sent[1].1, [0x89, 12]);
    assert_eq!(ctl.timers.len(), 1);
    let x = xd(&ctl);
    assert!(x.timer && x.spawn_pending && !x.ending && x.counter == 0);
    // Diablo at the 10th firing: T + 20.
    let t = ctl.tick;
    f.log.clear();
    f.spawns = vec![Some(DIABLO_U)];
    for _ in 0..19 {
        ctl.update(&mut f);
    }
    assert!(f.log.is_empty());
    assert_eq!(xd(&ctl).counter, 9);
    ctl.update(&mut f);
    assert_eq!(ctl.tick, t + 20);
    assert_eq!(
        f.log,
        [
            "spawn 243 700 800 mode 1 r 4294967295",
            "flags 34 0x3000000"
        ]
    );
    assert!(ctl.timers.is_empty());
    let x = xd(&ctl);
    assert!(x.spawned && !x.spawn_pending && !x.timer);
    // The start point initialises no more once Diablo is spawned.
    let mut f = sanctum_fake();
    f.objects.insert(UnitId(0x61), (0x61, 255, 0));
    start_point_init(&mut ctl, &mut f, UnitId(0x61));
    assert_eq!(xd(&ctl).start_guid, 0x60);
}

// Covers: specs/world/quests-act4.md §5.6
#[test]
fn diablo_spawn_waits_for_the_start_point_and_retries() {
    let (mut ctl, _) = control();
    let mut f = sanctum_fake();
    for n in 0..3 {
        kill_boss(&mut ctl, &mut f, n);
    }
    assert!(ctl.timers.is_empty()); // the seals are closed
    open_seals(&mut ctl, &mut f); // the last seal triggers
    assert_eq!(ctl.timers.len(), 1);
    // Twelve firings without a start point: nothing spawned.
    f.log.clear();
    for _ in 0..24 {
        ctl.update(&mut f);
    }
    assert!(f.log.is_empty() && xd(&ctl).counter == 12);
    // The start point initialises: the trigger finds the timer, the
    // Sanctum already cleared (no second FX).
    f.sent.clear();
    start_point_init(&mut ctl, &mut f, START);
    assert!(f.sent.is_empty() && ctl.timers.len() == 1);
    // The next firing spawns: r −1, 5, 10, all failing → retried.
    ctl.update(&mut f);
    ctl.update(&mut f);
    assert_eq!(
        f.log,
        [
            "spawn 243 700 800 mode 1 r 4294967295",
            "spawn 243 700 800 mode 1 r 5",
            "spawn 243 700 800 mode 1 r 10",
        ]
    );
    f.log.clear();
    f.spawns = vec![None, Some(DIABLO_U)];
    ctl.update(&mut f);
    ctl.update(&mut f);
    assert_eq!(
        f.log,
        [
            "spawn 243 700 800 mode 1 r 4294967295",
            "spawn 243 700 800 mode 1 r 5",
            "flags 34 0x3000000",
        ]
    );
    assert!(ctl.timers.is_empty());
}

// Covers: specs/world/quests-act4.md §5.6, §5.7, §edge-cases-original-bugs r10
#[test]
fn a_fourth_kill_blocks_diablo() {
    let (mut ctl, _) = control();
    let mut f = sanctum_fake();
    for n in 0..4 {
        kill_boss(&mut ctl, &mut f, n);
    }
    open_seals(&mut ctl, &mut f);
    start_point_init(&mut ctl, &mut f, START);
    assert_eq!(xd(&ctl).kills, 4);
    assert!(ctl.timers.is_empty() && !sanctum_cleared(&ctl));
}

// ------------------------------------------------------------ §5.7–§5.8

fn kill_diablo(ctl: &mut QuestControl, f: &mut Fake, killer: Option<UnitId>) {
    f.pos.insert(DIABLO_U, (700, 800, ROOM));
    ctl.monster_killed(f, DIABLO_U, killer);
}

// Covers: specs/world/quests-act4.md §5.7, §5.8 text, §5.8 r1, §5.8 r2, §5.8 r3, §7, §edge-cases-original-bugs r12, §edge-cases-original-bugs r22
#[test]
fn classic_diablo_kill_and_end_of_game() {
    // Test vector: Diablo killed, classic, killer in his room → killer
    // credited (26.13, 26.0, 26.6, 26.7), status 13, sound 75, FX 13; 75
    // s later save pass, 90 s warp to 103, 95 s game end.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.expansion = false;
    f.frame = 1000;
    f.players.insert(P2, act4_player(2));
    f.near = vec![P1];
    set(&mut f, P1, SLOT, &[2, 3]);
    f.client_flags.insert(P1, 0);
    kill_diablo(&mut ctl, &mut f, Some(P1));
    let r1 = f.flags(P1);
    assert!([13, 0, 6, 7].iter().all(|&b| r1.get(SLOT, b)));
    assert!(!r1.get(SLOT, 2) && !r1.get(SLOT, 3)); // reset_progress
    let r2 = f.flags(P2);
    assert!(r2.get(SLOT, 14) && !r2.get(SLOT, 13) && !r2.get(SLOT, 0));
    let x = xd(&ctl);
    assert!(x.killed && x.diablo_room == Some(ROOM) && x.credited == 1);
    assert!(x.timer && x.ending && x.end_pending && x.warp_pending && !x.saved);
    assert_eq!(x.counter, 1000);
    assert_eq!(ctl.records[ix(&ctl)].status, 13);
    let mut want = vec![
        (P1, hex("89 0D")),
        (P2, hex("89 0D")),
        (P1, hex("5D 17 00 0D 0000")),
        (P2, hex("5D 17 00 0D 0000")),
        (P2, hex("5D 17 00 0C 0000")),
        (P1, hex("5D 17 02 00 0000")),
    ];
    let sent: Vec<_> = f.sent.iter().filter(|m| m.1[0] != 0x28).cloned().collect();
    assert_eq!(sent, want);
    // `0x00538680(client, 4, difficulty 0)`: a classic character's
    // progression 4 (bits 8–12) at `0x005B4D77`.
    assert_eq!(
        f.log,
        [
            format!("progression {} 0x0400", P1.0),
            "sound 1 75".to_owned()
        ]
    );
    // The schedule: elapsed = 40 ms × frames since the kill (open
    // question 2), tested at each firing.
    f.log.clear();
    f.sent.clear();
    let fire = |ctl: &mut QuestControl, f: &mut Fake, frame: i32| {
        f.frame = frame;
        run_timer(ctl, f, Timer::Diablo, CHAIN)
    };
    // The save pass and the game end are host requests
    // (`quests-helpers.md` §6).
    assert!(!fire(&mut ctl, &mut f, 1000 + 1875)); // 75000 ms: not yet
    assert!(f.log.is_empty() && ctl.host_requests.is_empty());
    assert!(!fire(&mut ctl, &mut f, 1000 + 1876));
    assert_eq!(ctl.take_host_requests(), [HostRequest::SavePass]);
    assert!(!fire(&mut ctl, &mut f, 1000 + 2250)); // once only
    assert!(ctl.host_requests.is_empty());
    f.log.clear();
    assert!(!fire(&mut ctl, &mut f, 1000 + 2251));
    assert_eq!(
        f.log,
        ["end interaction 1", "warp 1 103 0", "end interaction 2"]
    );
    want = vec![(P1, hex("5D 17 01 00 0000")), (P2, hex("50 1700"))];
    want[1].1.extend([0u8; 12]);
    assert_eq!(f.sent, want);
    assert!(f.players[&P1].byte4c == 1 && f.players[&P2].byte4c == 1);
    assert_eq!(xd(&ctl).last_warped, Some(P2));
    f.log.clear();
    assert!(!fire(&mut ctl, &mut f, 1000 + 2375)); // warp done once
    assert!(f.log.is_empty());
    assert!(fire(&mut ctl, &mut f, 1000 + 2376));
    assert!(f.log.is_empty());
    assert_eq!(ctl.take_host_requests(), [HostRequest::EndGame]);
    let x = xd(&ctl);
    assert!(!x.timer && !x.ending && !x.end_pending);
}

// Covers: specs/world/quests-act4.md §5.8 r1, §5.8 r2, §5.7
#[test]
fn expansion_diablo_kill_runs_the_timer_without_ending() {
    let (mut ctl, _) = control();
    let mut f = fake();
    f.near = vec![P1];
    kill_diablo(&mut ctl, &mut f, Some(P1));
    let r1 = f.flags(P1);
    assert!(r1.get(SLOT, 13) && r1.get(SLOT, 0) && !r1.get(SLOT, 6) && !r1.get(SLOT, 7));
    // No FX 13 in expansion games; no 0x00538680.
    assert!(!f.sent.iter().any(|m| m.1[0] == 0x89));
    assert_eq!(f.log, ["sound 1 75"]);
    f.log.clear();
    f.sent.clear();
    for (frame, done) in [(1876, false), (2251, false), (2376, true)] {
        f.frame = frame;
        assert_eq!(run_timer(&mut ctl, &mut f, Timer::Diablo, CHAIN), done);
    }
    assert!(f.log.is_empty());
    assert_eq!(ctl.take_host_requests(), [HostRequest::SavePass]);
    assert!(f.sent.is_empty());
}

// Covers: specs/world/quests-act4.md §5.7, §7, §edge-cases-original-bugs r9
#[test]
fn credit_by_room_then_party() {
    // P1 kills from two rooms away (26.14 only); P2 is in Diablo's room;
    // P3 is in P2's party in Act IV.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.players.insert(P2, act4_player(2));
    f.players.insert(P3, act4_player(3));
    f.near = vec![P2];
    f.party.insert(P2, vec![P3]);
    kill_diablo(&mut ctl, &mut f, Some(P1));
    assert!(f.flags(P1).get(SLOT, 14) && !f.flags(P1).get(SLOT, 13));
    for p in [P2, P3] {
        assert!(f.flags(p).get(SLOT, 13) && f.flags(p).get(SLOT, 0));
    }
    assert_eq!(xd(&ctl).credited, 2);
    // The killer in the credited player's party, in Act IV: credited by
    // the party pass.
    let (mut ctl1, _) = control();
    let mut e = fake();
    e.players.insert(P2, act4_player(2));
    e.near = vec![P2];
    e.party.insert(P2, vec![P1, P2]);
    kill_diablo(&mut ctl1, &mut e, Some(P1));
    assert!(e.flags(P1).get(SLOT, 13) && !e.flags(P1).get(SLOT, 14));
    assert_eq!(xd(&ctl1).credited, 2); // P2 itself has 26.0 by then
                                       // A party member outside Act IV is not credited.
    let (mut ctl2, _) = control();
    let mut g = fake();
    g.players.insert(P2, act4_player(2));
    g.p(P1).act = Some(0); // not in Act IV
    g.near = vec![P2];
    g.party.insert(P2, vec![P1]);
    kill_diablo(&mut ctl2, &mut g, Some(P1));
    assert!(!g.flags(P1).get(SLOT, 13) && g.flags(P1).get(SLOT, 14));
    assert_eq!(xd(&ctl2).credited, 1);
    // A player with 26.1 (or 26.0) is not credited by the room pass.
    let (mut ctl3, _) = control();
    let mut h = fake();
    h.near = vec![P1];
    set(&mut h, P1, SLOT, &[1]);
    kill_diablo(&mut ctl3, &mut h, Some(P1));
    assert!(!h.flags(P1).get(SLOT, 13) && h.flags(P1).get(SLOT, 14));
    assert_eq!(xd(&ctl3).credited, 0);
}

// Covers: specs/world/quests-act4.md §5.7, §edge-cases-original-bugs r11
#[test]
fn diablo_death_in_an_intro_game_and_without_killer() {
    let (mut ctl, _) = control();
    let i = ix(&ctl);
    ctl.records[i].not_intro = false;
    let mut f = fake();
    f.expansion = false;
    f.near = vec![P1];
    kill_diablo(&mut ctl, &mut f, Some(P1));
    // +0x14, FX 13 and the end timer, but no credit and no status.
    assert!(xd(&ctl).killed && xd(&ctl).timer);
    assert_eq!(ctl.timers.len(), 1);
    assert_eq!(f.flags(P1).word(SLOT), 0);
    assert_eq!(f.sent_ids(), [0x28, 0x89]);
    // No killing player: no timer. No room: nothing past +0x14.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.game_type = 1;
    kill_diablo(&mut ctl, &mut f, None);
    assert!(xd(&ctl).killed && !xd(&ctl).timer && ctl.timers.is_empty());
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.monster_killed(&mut f, DIABLO_U, Some(P1));
    assert!(xd(&ctl).killed && xd(&ctl).diablo_room.is_none() && !xd(&ctl).timer);
    assert!(f.sent.is_empty());
}

// ------------------------------------------------------------ §5.9

const PORTAL_U: UnitId = UnitId(0x70);

// Covers: specs/world/quests-act4.md §5.9, §5.2 text
#[test]
fn tyrael_20000_opens_the_portal() {
    // Test vector: expansion, Tyrael 20000 → 26.9 set; portal 566 at
    // Tyrael + (5, 0) or the nearest free spot.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.pos.insert(TYRAEL_U, (500, 600, ROOM));
    f.spot = Some((1, 2));
    f.a5_places = vec![Some(PORTAL_U)];
    say(&mut ctl, &mut f, P1, TYRAEL_U, 20000);
    assert!(f.flags(P1).get(SLOT, 9));
    assert_eq!(
        f.log,
        [
            "spot at 505 600 2 0x400 12 100",
            "place 566 506 602 room 1 [1, 1, 0]",
            "flags 112 0x3000000",
        ]
    );
    assert!(xd(&ctl).portal_spawned && !xd(&ctl).portal_request);
    // At most one per game.
    f.log.clear();
    say(&mut ctl, &mut f, P1, TYRAEL_U, 20000);
    assert!(f.log.is_empty());
    // A failed spawn is retried by the next 20000 only.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.pos.insert(TYRAEL_U, (500, 600, ROOM));
    say(&mut ctl, &mut f, P1, TYRAEL_U, 20000);
    assert!(!xd(&ctl).portal_spawned);
    f.spot = Some((0, 0));
    say(&mut ctl, &mut f, P1, TYRAEL_U, 20000);
    assert!(!xd(&ctl).portal_spawned); // no object
    f.a5_places = vec![Some(PORTAL_U)];
    say(&mut ctl, &mut f, P1, TYRAEL_U, 20000);
    assert!(xd(&ctl).portal_spawned);
}

// Covers: specs/world/quests-act4.md §5.9
#[test]
fn portal_init_modes() {
    let (mut ctl, _) = control();
    let mut f = fake();
    portal_init(&mut ctl, &mut f, PORTAL_U);
    portal_init(&mut ctl, &mut f, PORTAL_U);
    assert_eq!(f.log, ["mode 112 1", "mode 112 2"]);
    assert_eq!(xd(&ctl).portal_mode, 2);
    let (mut ctl, _) = control();
    ctl.records.retain(|r| r.chain != CHAIN);
    let mut f = fake();
    portal_init(&mut ctl, &mut f, PORTAL_U);
    assert!(f.log.is_empty());
}

// Covers: specs/world/quests-act4.md §5.9, §edge-cases-original-bugs r14
#[test]
fn portal_operate_moves_to_harrogath() {
    let (ctl, _) = control();
    // Classic: nothing.
    let mut f = fake();
    f.expansion = false;
    set(&mut f, P1, SLOT, &[0]);
    portal_operate(&ctl, &mut f, PORTAL_U, P1);
    assert!(f.log.is_empty() && f.sent.is_empty());
    // Neither 26.0 nor 26.13: sound 19.
    let mut f = fake();
    portal_operate(&ctl, &mut f, PORTAL_U, P1);
    assert_eq!(f.log, ["sound 1 19"]);
    // 26.13 without 26.0, outside Act IV: nothing.
    let mut f = fake();
    set(&mut f, P1, SLOT, &[13]);
    f.p(P1).act = Some(4);
    portal_operate(&ctl, &mut f, PORTAL_U, P1);
    assert!(f.log.is_empty() && f.sent.is_empty());
    // In Act IV, first use: 28.0, 28.13, the client steps, then the act
    // change, send flags and the waypoint.
    f.p(P1).act = Some(3);
    f.difficulty = 1;
    set(&mut f, P1, SLOT, &[13]);
    portal_operate(&ctl, &mut f, PORTAL_U, P1);
    assert!(f.flags(P1).get(28, 0) && f.flags(P1).get(28, 13));
    assert_eq!(f.players[&P1].byte4c, 1);
    assert_eq!(
        f.log,
        ["interact 1 None", "act change 1 109 5", "waypoint 1 109 1"]
    );
    assert_eq!(f.sent_ids(), [0x5D, 0x61, 0x28]);
    assert_eq!(f.sent[0].1, hex("5D 17 02 00 0000"));
    assert_eq!(f.sent[1].1, [0x61, 5]);
    // Again (28.0 set): only the act change, flags and waypoint.
    f.log.clear();
    f.sent.clear();
    portal_operate(&ctl, &mut f, PORTAL_U, P1);
    assert_eq!(f.log, ["act change 1 109 5", "waypoint 1 109 1"]);
    assert_eq!(f.sent_ids(), [0x28]);
    // A busy client: the bits, no client steps.
    let mut f = fake();
    set(&mut f, P1, SLOT, &[0]);
    f.q2_busy = vec![P1];
    portal_operate(&ctl, &mut f, PORTAL_U, P1);
    assert!(f.flags(P1).get(28, 0));
    assert_eq!(f.log, ["act change 1 109 5", "waypoint 1 109 0"]);
    assert_eq!(f.sent_ids(), [0x28]);
}

// ------------------------------------------------------------ §5.10, §8

// Covers: specs/world/quests-act4.md §5.10, §8
#[test]
fn classic_interaction_gate() {
    let (mut ctl, _) = control();
    let mut f = fake();
    assert!(!interaction_refused(&ctl, &f));
    f.expansion = false;
    assert!(!interaction_refused(&ctl, &f));
    let i = ix(&ctl);
    ctl.records[i].extra.a4.q2.killed = true;
    assert!(interaction_refused(&ctl, &f));
    f.expansion = true;
    assert!(!interaction_refused(&ctl, &f));
    f.expansion = false;
    ctl.records.retain(|r| r.chain != CHAIN);
    assert!(interaction_refused(&ctl, &f));
    assert!(!sanctum_cleared(&ctl));
}

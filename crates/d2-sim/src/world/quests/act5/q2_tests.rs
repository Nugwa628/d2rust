// Spec: specs/world/quests-act5.md §4 (Test vectors, Edge cases); specs/world/quests.md §6.2
//! A5Q2 Rescue on Mount Arreat on the quests' fake world.

use super::super::super::tests::*;
use super::super::super::{
    act1, event, EventArgs, QuestChain, QuestControl, QuestWorld, TextList, UnitKind,
};
use super::{
    barbarians_left, cage_init, door_open_near, group_count, group_counting, group_portal,
    portal_event, rescue, rescue_status, BARBARIAN, CHAIN, PRISON_DOOR, QUAL_KEHK, SLOT,
};
use crate::units::{RoomId, UnitId};

const QUAL_U: UnitId = UnitId(0x40);
const LARZUK_U: UnitId = UnitId(0x41);
const DOOR_U: UnitId = UnitId(0x50);
const CAGE_U: UnitId = UnitId(0x60);
const PORTAL_U: UnitId = UnitId(0x61);
/// Barbarian units: 0x100 + 8g + n.
fn barb(g: u32, n: u32) -> UnitId {
    UnitId(0x100 + 8 * g + n)
}

fn kind(class: u16) -> UnitKind {
    UnitKind::Monster {
        class: u32::from(class),
        superunique: None,
        owner: None,
    }
}

fn fake() -> Fake {
    let mut f = Fake::new();
    f.p(P1).act = Some(4);
    f.p(P1).level = Some(111);
    for (u, class) in [(QUAL_U, QUAL_KEHK), (LARZUK_U, 511), (DOOR_U, PRISON_DOOR)] {
        f.monsters.insert(u, (u.0, class, kind(class)));
    }
    for g in 0..3 {
        for n in 0..5 {
            let b = barb(g, n);
            f.monsters.insert(b, (b.0, BARBARIAN, kind(BARBARIAN)));
        }
    }
    f
}

fn ev(ctl: &mut QuestControl, f: &mut Fake, args: EventArgs) {
    let i = ctl.find(CHAIN).unwrap();
    act1::callback(ctl, f, i, args, None, false);
}

fn say(ctl: &mut QuestControl, f: &mut Fake, msg: u32) {
    ev(
        ctl,
        f,
        EventArgs {
            event: event::SCROLL_MESSAGE,
            target: Some(QUAL_U),
            player: Some(P1),
            a: u32::from(QUAL_KEHK),
            b: msg,
        },
    );
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

fn kill(ctl: &mut QuestControl, f: &mut Fake, victim: UnitId) {
    f.chains.insert(victim, QuestChain(vec![CHAIN]));
    ctl.monster_killed(f, victim, Some(P1));
}

/// All three groups spawned with GUIDs 0x100 + 8g + n.
fn groups(ctl: &mut QuestControl) {
    let e = &mut ctl.record_mut(CHAIN).unwrap().extra.a5.q2;
    for g in 0..3 {
        e.cage_spawned[g] = true;
        for n in 0..5 {
            e.barbarians[g][n] = barb(g as u32, n as u32).0;
        }
    }
    e.spawned = 15;
}

fn x(ctl: &mut QuestControl) -> &mut super::Extra {
    &mut ctl.record_mut(CHAIN).unwrap().extra.a5.q2
}

// Covers: specs/world/quests-act5.md §4.4
#[test]
fn vector_rune_rewards() {
    // 36.1 + 36.7 → one rune; 36.0 set, 36.1 clear; 5D 20 02 00 0000.
    let (mut ctl, _) = control();
    let mut f = fake();
    for b in [1, 7] {
        f.p(P1).quests.flags[0].set(SLOT, b);
    }
    say(&mut ctl, &mut f, 20110);
    assert_eq!(f.log[0], "reward r07  0 2");
    assert!(!f.log.iter().any(|l| l.starts_with("reward r08")));
    let fl = f.flags(P1);
    assert!(fl.get(SLOT, 0) && !fl.get(SLOT, 1) && !fl.get(SLOT, 7));
    assert_eq!(f.sent[0], (P1, hex("5D 20 02 00 0000")));
    assert_eq!(ctl.record(CHAIN).unwrap().guids.0, [1]);
    // Refresh follows (0x27, 0x29).
    assert_eq!(f.sent_ids()[1..], [0x27, 0x29]);
    // 36.1 + 36.5 → all three runes; 36.6 → two.
    for (bit, want) in [(5, 3), (6, 2)] {
        let (mut ctl, _) = control();
        let mut f = fake();
        for b in [1, bit] {
            f.p(P1).quests.flags[0].set(SLOT, b);
        }
        say(&mut ctl, &mut f, 20110);
        let runes: Vec<&String> = f.log.iter().filter(|l| l.starts_with("reward")).collect();
        let codes = ["reward r07  0 2", "reward r08  0 2", "reward r09  0 2"];
        assert_eq!(runes, codes[..want].iter().collect::<Vec<_>>());
    }
    // 36.13: state 5, the own sequence function, status 13 silently,
    // callback 2 cleared.
    let (mut ctl, _) = control();
    let mut f = fake();
    for b in [1, 13, 5] {
        f.p(P1).quests.flags[0].set(SLOT, b);
    }
    ctl.record_mut(CHAIN).unwrap().state = 4;
    say(&mut ctl, &mut f, 20110);
    let r = ctl.record(CHAIN).unwrap();
    assert_eq!((r.state, r.status), (5, 13));
    assert!(!r.has_callback(event::NPC_DEACTIVATE));
    assert_eq!(ctl.record(33).unwrap().state, 1);
    assert_eq!(f.sent[0], (P1, hex("5D 20 02 00 0000")));
    // Without 36.1: nothing at all (no refresh).
    let (mut ctl, _) = control();
    let mut f = fake();
    say(&mut ctl, &mut f, 20110);
    assert!(f.sent.is_empty() && f.log.is_empty());
}

// Covers: specs/world/quests-act5.md §4.4, §4.2
#[test]
fn qual_kehk_start_and_chat_end() {
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().state = 1;
    say(&mut ctl, &mut f, 20096);
    assert_eq!(ctl.record(CHAIN).unwrap().state, 2);
    assert!(x(&mut ctl).started && f.flags(P1).get(SLOT, 2));
    assert_eq!(f.sent_ids(), [0x27, 0x29]);
    f.sent.clear();
    let end = EventArgs {
        event: event::NPC_DEACTIVATE,
        target: Some(QUAL_U),
        player: Some(P1),
        ..EventArgs::default()
    };
    // Killed ≥ 5: nothing.
    x(&mut ctl).killed = 5;
    ev(&mut ctl, &mut f, end);
    assert!(f.sent.is_empty() && x(&mut ctl).started);
    x(&mut ctl).killed = 4;
    ev(&mut ctl, &mut f, end);
    // Barbarians left: 15 − 4 killed.
    assert_eq!(f.sent, [(P1, hex("5D 20 00 01 0B00"))]);
    assert!(!x(&mut ctl).started);
    assert!(!ctl
        .record(CHAIN)
        .unwrap()
        .has_callback(event::NPC_DEACTIVATE));
    // Flag iterate at state 3 → 36.3; players with 36.1 skipped.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().state = 2;
    let lvl = EventArgs {
        event: event::CHANGED_LEVEL,
        player: Some(P1),
        a: 110,
        b: 111,
        ..EventArgs::default()
    };
    ev(&mut ctl, &mut f, lvl);
    assert!(f.flags(P1).get(SLOT, 3));
}

// Covers: specs/world/quests-act5.md §4.3
#[test]
fn chat_tables_and_active() {
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().state = 1;
    assert_eq!(text(&mut ctl, &mut f, QUAL_U), [(20096, 0)]);
    assert!(text(&mut ctl, &mut f, LARZUK_U).is_empty());
    ctl.record_mut(CHAIN).unwrap().state = 3;
    assert_eq!(text(&mut ctl, &mut f, QUAL_U), [(20103, 2)]);
    // Index 2 with 36.4 → table state 5 (Qual-Kehk only).
    f.p(P1).quests.flags[0].set(SLOT, 4);
    assert_eq!(text(&mut ctl, &mut f, QUAL_U), [(20104, 2)]);
    assert_eq!(text(&mut ctl, &mut f, LARZUK_U), [(20107, 2)]);
    // State > 3 needs 36.13.
    ctl.record_mut(CHAIN).unwrap().state = 4;
    assert!(text(&mut ctl, &mut f, QUAL_U).is_empty());
    f.p(P1).quests.flags[0].set(SLOT, 13);
    assert_eq!(text(&mut ctl, &mut f, QUAL_U), [(20110, 0)]);
    // 36.1 → 3 always; a listed GUID → 4.
    let mut f = fake();
    f.p(P1).quests.flags[0].set(SLOT, 1);
    ctl.record_mut(CHAIN).unwrap().state = 0;
    assert_eq!(text(&mut ctl, &mut f, QUAL_U), [(20110, 0)]);
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().guids.add(1);
    f.p(P1).quests.flags[0].set(SLOT, 0);
    assert_eq!(text(&mut ctl, &mut f, QUAL_U), [(20110, 2)]);
    // 36.0 or intro: nothing.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().state = 1;
    f.p(P1).quests.flags[0].set(SLOT, 0);
    assert!(text(&mut ctl, &mut f, QUAL_U).is_empty());
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().not_intro = false;
    assert!(text(&mut ctl, &mut f, QUAL_U).is_empty());
    // Active.
    let (mut ctl, _) = control();
    let i = ctl.find(CHAIN).unwrap();
    let mut f = fake();
    let act = |ctl: &QuestControl, f: &mut Fake, c| super::active(ctl, f, i, P1, c, 0x0058_8340);
    assert!(!act(&ctl, &mut f, QUAL_KEHK) && act(&ctl, &mut f, BARBARIAN));
    ctl.records[i].state = 1;
    assert!(act(&ctl, &mut f, QUAL_KEHK) && !act(&ctl, &mut f, 511));
    ctl.records[i].state = 3;
    f.p(P1).quests.flags[0].set(SLOT, 1);
    assert!(act(&ctl, &mut f, QUAL_KEHK) && !act(&ctl, &mut f, BARBARIAN));
}

// Covers: specs/world/quests-act5.md §4.5
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
        ev(ctl, f, args);
    };
    // Level 111 at state 1, status 0: state 3, status 1 to all,
    // callback 2 cleared, flag iterate.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().state = 1;
    lvl(&mut ctl, &mut f, 110, 111);
    let r = ctl.record(CHAIN).unwrap();
    assert_eq!((r.state, r.status), (3, 1));
    assert!(!r.has_callback(event::NPC_DEACTIVATE) && f.flags(P1).get(SLOT, 3));
    assert_eq!(f.sent, [(P1, hex("5D 20 00 01 0F00"))]);
    // Status set, state 0: no iterate (b false).
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().status = 2;
    ctl.record_mut(CHAIN).unwrap().state = 0;
    lvl(&mut ctl, &mut f, 110, 112);
    assert!(f.sent.is_empty() && f.flags(P1).word(SLOT) == 0);
    // Killed ≥ 5: the first branch fails; old level 109 then.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().state = 2;
    x(&mut ctl).killed = 5;
    ctl.record_mut(CHAIN).unwrap().guids.add(1);
    lvl(&mut ctl, &mut f, 109, 111);
    let r = ctl.record(CHAIN).unwrap();
    assert_eq!((r.state, r.status), (3, 0));
    assert!(r.guids.0.is_empty() && f.flags(P1).get(SLOT, 3));
    assert!(f.sent.is_empty());
    // Old level 109 at state 0: the tail needs state 2, nothing is sent.
    let (mut ctl, _) = control();
    let mut f = fake();
    lvl(&mut ctl, &mut f, 109, 110);
    let r = ctl.record(CHAIN).unwrap();
    assert_eq!((r.state, r.status), (0, 0));
    assert!(f.sent.is_empty());
    // At state 2, killed < 5, status 0: status 1 to all, callback 2 gone.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().state = 2;
    lvl(&mut ctl, &mut f, 109, 110);
    let r = ctl.record(CHAIN).unwrap();
    assert_eq!((r.state, r.status), (3, 1));
    assert!(!r.has_callback(event::NPC_DEACTIVATE));
}

// Covers: specs/world/quests-act5.md §4.6 text, §4.6 r1, §4.6 r2, §4.6 r3, §edge-cases-original-bugs r2
#[test]
fn kills_doors_and_barbarians() {
    let (mut ctl, _) = control();
    let mut f = fake();
    groups(&mut ctl);
    // A door: portal flag cleared; live barbarians of class 534 closer
    // than 15 are freed.
    f.pos.insert(DOOR_U, (5, 5, RoomId(9)));
    f.a5_adjacent.insert(
        RoomId(9),
        vec![barb(0, 0), barb(0, 1), barb(0, 2), barb(0, 3), QUAL_U],
    );
    f.a5_modes.insert(barb(0, 0), 1);
    f.a5_modes.insert(barb(0, 1), 12); // dead
    f.a5_modes.insert(barb(0, 2), 1);
    f.a5_modes.insert(barb(0, 3), 0); // mode 0
    f.a5_modes.insert(QUAL_U, 1);
    f.a5_dist.insert((barb(0, 0), DOOR_U), 14);
    f.a5_dist.insert((barb(0, 1), DOOR_U), 3);
    f.a5_dist.insert((barb(0, 2), DOOR_U), 15); // not closer
    f.a5_dist.insert((barb(0, 3), DOOR_U), 3);
    f.a5_dist.insert((QUAL_U, DOOR_U), 3);
    kill(&mut ctl, &mut f, DOOR_U);
    assert_eq!(f.log, ["room portal RoomId(9) false"]);
    assert_eq!(x(&mut ctl).freed_guids, [barb(0, 0).0]);
    assert_eq!((x(&mut ctl).freed, x(&mut ctl).killed), (1, 0));
    // A freed barbarian dying counts only toward its group (edge case 2).
    kill(&mut ctl, &mut f, barb(0, 0));
    assert_eq!((x(&mut ctl).killed, x(&mut ctl).counter[0]), (0, 1));
    // Other kills: killed += 1 and the group counter.
    for n in 1..5 {
        kill(&mut ctl, &mut f, barb(1, n));
    }
    assert_eq!((x(&mut ctl).killed, x(&mut ctl).counter[1]), (4, 4));
    assert!(!x(&mut ctl).accounted[1] && f.sent.is_empty());
    // Vector: killed reaches 5 → status 12 to all, completion flag.
    x(&mut ctl).started = true;
    kill(&mut ctl, &mut f, barb(1, 0));
    assert_eq!(x(&mut ctl).killed, 5);
    assert!(x(&mut ctl).accounted[1] && !x(&mut ctl).started);
    assert_eq!(ctl.record(CHAIN).unwrap().status, 12);
    assert!(f.flags(P1).get(SLOT, 14));
    assert_eq!(
        f.sent,
        [(P1, hex("5D 20 00 0C 0900")), (P1, hex("5D 20 00 0C 0000"))]
    );
    // Later kills: no second failure message.
    f.sent.clear();
    kill(&mut ctl, &mut f, barb(2, 0));
    assert!(f.sent.is_empty());
    // Intro: nothing.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().not_intro = false;
    kill(&mut ctl, &mut f, barb(0, 0));
    assert_eq!(x(&mut ctl).killed, 0);
}

// Covers: specs/world/quests-act5.md §4.7, §4.1
#[test]
fn cages_spawn_groups() {
    let (mut ctl, _) = control();
    let mut f = fake();
    f.pos.insert(CAGE_U, (40, 50, RoomId(2)));
    f.spawns = vec![
        Some(barb(0, 0)),
        None,
        Some(barb(0, 1)),
        Some(barb(0, 2)),
        Some(barb(0, 3)),
        Some(barb(0, 4)),
        Some(barb(1, 0)),
    ];
    cage_init(&mut ctl, &mut f, CAGE_U);
    let r = ctl.record(CHAIN).unwrap();
    assert_eq!((r.state, r.status), (2, 1));
    assert_eq!(f.sent, [(P1, hex("5D 20 00 01 0F00"))]);
    let e = &r.extra.a5.q2;
    assert_eq!(e.spawned, 5);
    assert_eq!(e.cage_spawned, [true, false, false]);
    assert_eq!(e.cage_pos[0], (40, 50));
    let want: Vec<u32> = (0..5).map(|n| barb(0, n).0).collect();
    assert_eq!(e.barbarians[0].to_vec(), want);
    // Six spawn calls (one failed), stop after 5.
    assert_eq!(
        f.log
            .iter()
            .filter(|l| l.starts_with("spawn 534 40 50"))
            .count(),
        6
    );
    assert_eq!(f.spawns, [Some(barb(1, 0))]);
    // Each barbarian is linked to chain 32 (none have a chain in the
    // fake: the link is refused; flags are set).
    assert_eq!(
        f.log
            .iter()
            .filter(|l| l.as_str() == "flags 256 0x3000000")
            .count(),
        1
    );
    // The same cage again: stop (same position as group 0).
    f.log.clear();
    cage_init(&mut ctl, &mut f, CAGE_U);
    assert!(f.log.is_empty());
    // Barbarians left: 2 × 5 unspawned + 5 spawned.
    assert_eq!(barbarians_left(&ctl, &mut f), 15);
    // A second cage: group 1; 25 tries at most.
    let cage2 = UnitId(0x62);
    f.pos.insert(cage2, (80, 50, RoomId(2)));
    f.spawns = vec![None; 30];
    cage_init(&mut ctl, &mut f, cage2);
    assert_eq!(f.spawns.len(), 5);
    let e = &ctl.record(CHAIN).unwrap().extra.a5.q2;
    assert_eq!((e.spawned, e.cage_spawned[1]), (5, true));
    // Status already set: not sent again; intro: no state change.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.pos.insert(CAGE_U, (40, 50, RoomId(2)));
    ctl.record_mut(CHAIN).unwrap().not_intro = false;
    cage_init(&mut ctl, &mut f, CAGE_U);
    assert_eq!(ctl.record(CHAIN).unwrap().state, 0);
    assert!(f.sent.is_empty() && x(&mut ctl).cage_spawned[0]);
}

// Covers: specs/world/quests.md §6.2, §6.3
#[test]
fn barbarians_left_count() {
    let (mut ctl, _) = control();
    let mut f = fake();
    assert_eq!(barbarians_left(&ctl, &mut f), 15);
    groups(&mut ctl);
    x(&mut ctl).killed = 2;
    x(&mut ctl).freed = 4;
    assert_eq!(barbarians_left(&ctl, &mut f), 9);
    x(&mut ctl).freed = 20;
    assert_eq!(barbarians_left(&ctl, &mut f), 0);
    // 0x50 carries it when the quest has a status (list[36] ≠ 0).
    x(&mut ctl).freed = 3;
    ctl.record_mut(CHAIN).unwrap().status = 1;
    ctl.request_quest_data(&mut f, P1).unwrap();
    let m = f.sent.iter().find(|m| m.1[0] == 0x50).unwrap();
    assert_eq!(m.1[7..9], [10, 0]);
    // No chain-32 record: 0.
    let j = ctl.find(CHAIN).unwrap();
    ctl.records.remove(j);
    assert_eq!(barbarians_left(&ctl, &mut f), 0);
}

/// The rescue set-up: groups spawned, P1 in room 4 with a dead door
/// next to barbarian (2, 0).
fn rescue_world() -> (QuestControl, Fake) {
    let (mut ctl, _) = control();
    let mut f = fake();
    groups(&mut ctl);
    f.pos.insert(P1, (0, 0, RoomId(4)));
    f.pos.insert(DOOR_U, (30, 40, RoomId(5)));
    f.a5_adjacent.insert(RoomId(4), vec![QUAL_U, DOOR_U]);
    f.a5_modes.insert(DOOR_U, 12);
    f.a5_dist.insert((DOOR_U, barb(2, 0)), 14);
    f.objects.insert(PORTAL_U, (0x61, 189, 1));
    f.frame = 1000;
    (ctl, f)
}

// Covers: specs/world/quests-act5.md §4.7
#[test]
fn rescue_portal_and_completion() {
    // Vector: 15 spawned, killed 2, freed 13, last rescue → status 3; P
    // gets 36.13, 36.1, 36.7.
    let (mut ctl, mut f) = rescue_world();
    x(&mut ctl).killed = 2;
    x(&mut ctl).freed = 13;
    f.a5_places = vec![None, Some(PORTAL_U)];
    rescue(&mut ctl, &mut f, P1, barb(2, 0));
    assert_eq!(
        f.log[..4],
        [
            "place 189 32 40 room 5 [1, 1, 0]",
            "place 189 30 40 room 5 [1, 1, 0]",
            "event7 97 1025",
            "room portal RoomId(5) false",
        ]
    );
    let e = &ctl.record(CHAIN).unwrap().extra.a5.q2;
    assert!(e.portal_made[2] && e.portal_spawned[2] && e.portal_guid[2] == 0x61);
    let fl = f.flags(P1);
    assert!(fl.get(SLOT, 13) && fl.get(SLOT, 1) && fl.get(SLOT, 7));
    assert!(!fl.get(SLOT, 5) && !fl.get(SLOT, 6));
    assert_eq!(ctl.record(CHAIN).unwrap().status, 3);
    assert_eq!(f.sent[0], (P1, hex("5D 20 00 03 0000")));
    assert_eq!(f.log.last().unwrap(), "sound 1 81");
    // The same group again: stop (portal made).
    f.log.clear();
    rescue(&mut ctl, &mut f, P1, barb(2, 0));
    assert!(f.log.is_empty());
    // Freed 15 / 14: bits 36.5 / 36.6; a party member in Act V too.
    for (freed, killed, bit) in [(15, 0, 5), (14, 1, 6)] {
        let (mut ctl, mut f) = rescue_world();
        f.players.insert(
            P2,
            Player {
                guid: 2,
                act: Some(4),
                level: Some(111),
                ..Player::default()
            },
        );
        f.party.insert(P1, vec![P2]);
        x(&mut ctl).killed = killed;
        x(&mut ctl).freed = freed;
        rescue(&mut ctl, &mut f, P1, barb(2, 0));
        for p in [P1, P2] {
            assert!(f.flags(p).get(SLOT, bit) && f.flags(p).get(SLOT, 1));
        }
        assert_eq!(
            f.log[f.log.len() - 2..],
            ["sound 1 81".to_string(), "sound 2 81".to_string()]
        );
    }
    // All accounted but freed < 12: the completion flag only.
    let (mut ctl, mut f) = rescue_world();
    x(&mut ctl).killed = 4;
    x(&mut ctl).freed = 11;
    rescue(&mut ctl, &mut f, P1, barb(2, 0));
    assert!(f.flags(P1).get(SLOT, 14) && !f.flags(P1).get(SLOT, 1));
    assert_eq!(ctl.record(CHAIN).unwrap().status, 0);
    // Not all accounted: 36.4 on P, flags byte 0x20, status 2 to all.
    let (mut ctl, mut f) = rescue_world();
    x(&mut ctl).freed = 5;
    x(&mut ctl).started = true;
    rescue(&mut ctl, &mut f, P1, barb(2, 0));
    assert!(f.flags(P1).get(SLOT, 4) && !x(&mut ctl).started);
    let r = ctl.record(CHAIN).unwrap();
    assert_eq!((r.status, r.flags), (2, 0x20));
    assert_eq!(f.sent, [(P1, hex("5D 20 20 02 0A00"))]);
    // Failing all three placements: the free spot (mask 0x8000, radius
    // 17); nothing made → no portal fields, the check still runs.
    let (mut ctl, mut f) = rescue_world();
    f.spot = Some((1, 1));
    x(&mut ctl).killed = 5;
    rescue(&mut ctl, &mut f, P1, barb(2, 0));
    assert_eq!(f.log[2], "spot at 32 40 2 0x8000 17 100");
    assert_eq!(f.log[3], "place 189 33 41 room 5 [1, 1, 0]");
    assert!(x(&mut ctl).portal_made[2] && !x(&mut ctl).portal_spawned[2]);
    assert!(f.flags(P1).get(SLOT, 4));
    assert_eq!(ctl.record(CHAIN).unwrap().status, 0); // killed ≥ 5
                                                      // No dead door close enough, or B in no group: nothing.
    let (mut ctl, mut f) = rescue_world();
    f.a5_dist.insert((DOOR_U, barb(2, 0)), 15);
    rescue(&mut ctl, &mut f, P1, barb(2, 0));
    rescue(&mut ctl, &mut f, P1, QUAL_U);
    assert!(f.log.is_empty() && !x(&mut ctl).portal_made[2]);
}

// Covers: specs/world/quests-act5.md §4.8
#[test]
fn rescue_portal_modes() {
    let (mut ctl, mut f) = rescue_world();
    x(&mut ctl).portal_guid[1] = 0x61;
    portal_event(&mut ctl, &mut f, PORTAL_U);
    assert_eq!(f.log, ["mode 97 2", "event7 97 1025"]);
    // Mode 2, group not accounted: stays.
    f.log.clear();
    portal_event(&mut ctl, &mut f, PORTAL_U);
    assert_eq!(f.log, ["event7 97 1025"]);
    // Accounted: the counter must pass 5.
    x(&mut ctl).accounted[1] = true;
    for _ in 0..5 {
        portal_event(&mut ctl, &mut f, PORTAL_U);
    }
    assert_eq!(f.object_mode(PORTAL_U), 2);
    portal_event(&mut ctl, &mut f, PORTAL_U);
    assert_eq!(f.object_mode(PORTAL_U), 3);
    assert!(x(&mut ctl).may_close[1] && x(&mut ctl).close_counter[1] == 6);
    portal_event(&mut ctl, &mut f, PORTAL_U);
    assert_eq!(f.object_mode(PORTAL_U), 4);
    f.log.clear();
    portal_event(&mut ctl, &mut f, PORTAL_U);
    assert_eq!(f.log, ["event7 97 1025"]);
    // Not a group's portal: nothing.
    let (mut ctl, mut f) = rescue_world();
    portal_event(&mut ctl, &mut f, PORTAL_U);
    assert!(f.log.is_empty());
}

// Covers: specs/world/quests-act5.md §4.9
#[test]
fn start_join_leave() {
    let start = |bits: &[u8]| {
        let (mut ctl, _) = control();
        let mut f = fake();
        for &b in bits {
            f.p(P1).quests.flags[0].set(SLOT, b);
        }
        f.p(P1).quests.flags[0].set(SLOT, 4);
        let args = EventArgs {
            event: event::PLAYER_STARTED_GAME,
            player: Some(P1),
            target: Some(P1),
            ..EventArgs::default()
        };
        ev(&mut ctl, &mut f, args);
        assert!(!f.flags(P1).get(SLOT, 4));
        let r = ctl.record(CHAIN).unwrap();
        (r.state, r.status)
    };
    assert_eq!(start(&[0, 3]), (0, 0));
    assert_eq!(start(&[15, 3]), (0, 0));
    assert_eq!(start(&[1, 3]), (0, 0));
    assert_eq!(start(&[3, 2]), (3, 1));
    assert_eq!(start(&[2]), (2, 1));
    assert_eq!(start(&[]), (0, 0));
    // Join clears 36.4; leave removes the GUID from both lists.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.p(P1).quests.flags[0].set(SLOT, 4);
    let mut args = EventArgs {
        event: event::PLAYER_JOINED_GAME,
        player: Some(P1),
        target: Some(P1),
        ..EventArgs::default()
    };
    ev(&mut ctl, &mut f, args);
    assert!(!f.flags(P1).get(SLOT, 4));
    ctl.record_mut(CHAIN).unwrap().guids.add(1);
    x(&mut ctl).guids.add(1);
    x(&mut ctl).guids.add(7);
    args.event = event::PLAYER_LEAVES_GAME;
    ev(&mut ctl, &mut f, args);
    assert!(ctl.record(CHAIN).unwrap().guids.0.is_empty());
    assert_eq!(x(&mut ctl).guids.0, [7]);
}

// Covers: specs/world/quests-act5.md §4.10
#[test]
fn barbarian_ai_hooks() {
    let (mut ctl, mut f) = rescue_world();
    let b = barb(1, 2);
    // 0x00588830 / 0x00588880.
    assert!(!group_counting(&ctl, &mut f, b));
    for _ in 0..4 {
        group_count(&mut ctl, &mut f, b);
    }
    assert!(group_counting(&ctl, &mut f, b) && !x(&mut ctl).accounted[1]);
    group_count(&mut ctl, &mut f, b);
    assert!(x(&mut ctl).accounted[1] && x(&mut ctl).counter[1] == 5);
    assert!(!group_counting(&ctl, &mut f, QUAL_U));
    ctl.record_mut(CHAIN).unwrap().not_intro = false;
    group_count(&mut ctl, &mut f, b);
    assert!(x(&mut ctl).counter[1] == 5 && !group_counting(&ctl, &mut f, b));
    // 0x00588D60: the group's portal when spawned and existing.
    let (mut ctl, mut f) = rescue_world();
    x(&mut ctl).portal_guid[1] = 0x61;
    assert_eq!(group_portal(&ctl, &mut f, b), None);
    x(&mut ctl).portal_spawned[1] = true;
    assert_eq!(group_portal(&ctl, &mut f, b), Some(PORTAL_U));
    f.objects.clear();
    assert_eq!(group_portal(&ctl, &mut f, b), None);
    // 0x00588DD0: status 5 to all once, while killed < 5.
    let (mut ctl, mut f) = rescue_world();
    rescue_status(&mut ctl, &mut f);
    assert_eq!(ctl.record(CHAIN).unwrap().status, 5);
    assert_eq!(f.sent.len(), 1);
    rescue_status(&mut ctl, &mut f);
    assert_eq!(f.sent.len(), 1);
    let (mut ctl, mut f) = rescue_world();
    x(&mut ctl).killed = 5;
    rescue_status(&mut ctl, &mut f);
    assert_eq!(ctl.record(CHAIN).unwrap().status, 0);
    // 0x00588E10: a dead door near the player, else the fallbacks.
    let (_, mut f) = rescue_world();
    assert!(door_open_near(&mut f, Some(P1), b));
    f.a5_modes.insert(DOOR_U, 1);
    assert!(!door_open_near(&mut f, Some(P1), b));
    f.a5_modes.insert(DOOR_U, 12);
    // No player: B's room unless game type 3 (first client's player).
    f.pos.insert(b, (0, 0, RoomId(6)));
    assert!(!door_open_near(&mut f, None, b));
    f.game_type = 3;
    assert!(door_open_near(&mut f, None, b));
}

// Covers: specs/world/quests-act5.md §4.7
#[test]
fn freed_bit_is_an_equality_test() {
    // 15 → 36.5, 14 → 36.6, any other count (16 included) → 36.7
    // (`0x00588B96`–`0x00588BA8`).
    assert_eq!(super::freed_bit(15), 5);
    assert_eq!(super::freed_bit(14), 6);
    assert_eq!(super::freed_bit(16), 7);
    assert_eq!(super::freed_bit(12), 7);
}

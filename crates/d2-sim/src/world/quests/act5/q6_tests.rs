// Spec: specs/world/quests-act5-2.md §8 (Test vectors, Edge cases), §11
//! A5Q6 Eve of Destruction callback by callback on the quests' fake
//! world, and the Act V sequence chain through chains 34–36.

use super::super::super::tests::*;
use super::super::super::*;
use crate::units::RoomId;

const LARZUK_U: UnitId = UnitId(0x40);
const CAIN_U: UnitId = UnitId(0x41);
const TYRAEL_U: UnitId = UnitId(0x42);
const DREHYA_U: UnitId = UnitId(0x43);
const STATUE_U: UnitId = UnitId(0x44);
const BAAL_U: UnitId = UnitId(0x45);
const PORTAL_U: UnitId = UnitId(0x46);

fn monster(f: &mut Fake, u: UnitId, class: u16) {
    let kind = UnitKind::Monster {
        class: u32::from(class),
        superunique: None,
        owner: None,
    };
    f.monsters.insert(u, (u.0, class, kind));
}

/// P1 in the Worldstone Chamber (Act V); the post-Baal NPCs, a statue,
/// Baal.
fn fake() -> Fake {
    let mut f = Fake::new();
    f.p(P1).act = Some(4);
    f.p(P1).level = Some(132);
    for (u, c) in [
        (LARZUK_U, 511),
        (CAIN_U, 520),
        (TYRAEL_U, 521),
        (DREHYA_U, 512),
        (STATUE_U, 537),
        (BAAL_U, 544),
    ] {
        monster(&mut f, u, c);
    }
    f.pos.insert(BAAL_U, (70, 80, RoomId(9)));
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
    ctl.find(36).unwrap()
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

fn call(ctl: &mut QuestControl, f: &mut Fake, args: EventArgs) -> bool {
    let i = rec(ctl);
    super::callback(ctl, f, i, args, None)
}

fn text(ctl: &mut QuestControl, f: &mut Fake, n: UnitId) -> TextList {
    let mut list = TextList::new();
    let i = rec(ctl);
    super::callback(
        ctl,
        f,
        i,
        ev(event::NPC_ACTIVATE, Some(n), 0, 0),
        Some(&mut list),
    );
    list
}

fn status_msgs(f: &Fake) -> Vec<(UnitId, u8)> {
    f.sent
        .iter()
        .filter(|m| m.1[0] == 0x5D && m.1[1] == 36)
        .map(|m| (m.0, m.1[3]))
        .collect()
}

fn kill_baal(ctl: &mut QuestControl, f: &mut Fake, killer: Option<UnitId>) {
    f.chains.insert(BAAL_U, QuestChain(vec![36]));
    ctl.monster_killed(f, BAAL_U, killer);
}

// Covers: specs/world/quests-act5-2.md §8.1, §8.5 text, §8.5 r1, §8.5 r2, §11, §edge-cases-original-bugs r8
#[test]
fn baal_gold_hell() {
    // Vector: hell (d = 2), 2 credited players, quest-seed rolls r1, r2 →
    // piles 13500 + r1 mod 1500, 13500 + r2 mod 1500.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.difficulty = 2;
    add_p2(&mut f, 4, 132);
    // P3 is P2's party member in Harrogath: credited, not counted.
    let p3 = UnitId(3);
    f.players.insert(
        p3,
        Player {
            guid: 3,
            act: Some(4),
            level: Some(109),
            ..Player::default()
        },
    );
    f.party.insert(P2, vec![P2, p3]);
    f.a5_created = Some(UnitId(0x99));
    for p in [P1, P2, p3] {
        f.client_flags.insert(p, 0x20);
    }
    let mut seed = ctl.seed;
    let (r1, r2) = (seed.step() % 1500, seed.step() % 1500);
    kill_baal(&mut ctl, &mut f, Some(P1));
    let i = rec(&ctl);
    assert_eq!(ctl.records[i].extra.a5.q6.credited, 2);
    assert_eq!(ctl.seed, seed);
    for p in [P1, P2, p3] {
        let fl = f.players[&p].quests.flags[2];
        assert!(fl.get(40, 0) && fl.get(40, 13), "{p:?}");
    }
    assert_eq!(
        f.log,
        [
            // n = 5 * 2 + 5 = 15 for an expansion character (bit 5).
            "progression 1 0x0f20",
            "progression 2 0x0f20",
            "progression 3 0x0f20",
            "sound 1 83",
            "sound 2 83",
            "sound 3 83",
            &format!("gold 69 {}", 13500 + r1),
            &format!("gold 69 {}", 13500 + r2),
            "missile at 69 625",
            "refresh 153",
        ]
    );
    // The save pass is a host request (`quests-helpers.md` §6).
    assert_eq!(ctl.take_host_requests(), [HostRequest::SavePass]);
    // FX 19 first, then status 4, S5D(36, 2, 0) to each credited player.
    assert_eq!(f.sent[0].1[0], 0x28);
    assert_eq!(f.sent[1].1, [0x89, 19]);
    assert!(f
        .sent
        .iter()
        .any(|m| m.0 == p3 && m.1 == hex("5D 24 02 00 0000")));
    assert_eq!(ctl.records[i].state, 5);
    assert_eq!(ctl.records[i].status, 4);
    assert_eq!(ctl.records[i].extra.a5.q6.kill_room, Some(RoomId(9)));
}

// Covers: specs/world/quests-act5-2.md §8.5 text, §8.5 r1, §8.5 r2, §edge-cases-original-bugs r8
#[test]
fn baal_without_killer_or_credit() {
    // No killing player: status 4, no credits, no gold, the hook.
    let (mut ctl, _) = control();
    let mut f = fake();
    kill_baal(&mut ctl, &mut f, None);
    assert!(!f.flags(P1).get(40, 0));
    assert_eq!(f.log, ["missile at 69 625"]);
    assert_eq!(ctl.take_host_requests(), [HostRequest::SavePass]);
    assert_eq!(ctl.records[rec(&ctl)].state, 5);
    // A killer that already has 40.0: credits for the others, no gold.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.p(P1).quests.flags[0].set(40, 0);
    add_p2(&mut f, 4, 132);
    let seed = ctl.seed;
    kill_baal(&mut ctl, &mut f, Some(P1));
    assert!(f.flags(P2).get(40, 13));
    assert_eq!(ctl.seed, seed);
    assert!(f.log.iter().all(|l| !l.starts_with("gold")));
    // Intro (and inactive: class 544 kills are forced): only FX 19 and
    // the missile.
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    ctl.records[i].not_intro = false;
    ctl.records[i].active = false;
    kill_baal(&mut ctl, &mut f, Some(P1));
    assert_eq!(f.log, ["missile at 69 625"]);
    assert_eq!(f.sent_ids(), [0x28, 0x89]);
    assert_eq!(ctl.records[i].state, 0);
    // Uber Baal (class 709) has no link: nothing (monster creation, §10).
}

// Covers: specs/world/quests-act5-2.md §8.4
#[test]
fn post_baal_messages() {
    // Vector: msg 20179 → 40.6.
    let (mut ctl, _) = control();
    let mut f = fake();
    call(&mut ctl, &mut f, ev(event::SCROLL_MESSAGE, None, 0, 20179));
    assert_eq!(f.flags(P1).word(40), 1 << 6);
    for (m, b) in [(20178, 4), (20177, 5), (20175, 7), (20180, 8), (20176, 9)] {
        call(&mut ctl, &mut f, ev(event::SCROLL_MESSAGE, None, 0, m));
        assert!(f.flags(P1).get(40, b), "{m}");
    }
    let w = f.flags(P1).word(40);
    call(&mut ctl, &mut f, ev(event::SCROLL_MESSAGE, None, 0, 20174));
    call(&mut ctl, &mut f, ev(event::SCROLL_MESSAGE, None, 0, 20181));
    assert_eq!(f.flags(P1).word(40), w);
}

// Covers: specs/world/quests-act5-2.md §8.3 text, §8.3 r1, §8.3 r2, §8.3 r3, §edge-cases-original-bugs r7
#[test]
fn chat_tables_and_wants_to_talk() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    let a5 = ctl.find(35).unwrap();
    // Step 1: the statue after the fight (state 1, +0x90 = 0): table
    // state 3, no statue entries; the statue wants to talk.
    ctl.records[a5].extra.a5.q5.fight_started = true;
    ctl.records[i].state = 1;
    assert!(text(&mut ctl, &mut f, STATUE_U).is_empty());
    assert!(super::active(&ctl, &mut f, i, P1, 537, 0));
    // State 2 with +0x90: the same text test, but no wish to talk.
    ctl.records[i].state = 2;
    ctl.records[i].extra.a5.q6.by_sequence = true;
    assert!(!super::active(&ctl, &mut f, i, P1, 537, 0));
    // Ancients alive: no.
    ctl.records[a5].extra.a5.q5.alive = 1;
    ctl.records[i].state = 1;
    ctl.records[i].extra.a5.q6.by_sequence = false;
    assert!(!super::active(&ctl, &mut f, i, P1, 537, 0));
    // Step 2: 40.0 clear, only state 3 adds table state 0.
    for s in 0..3 {
        ctl.records[i].state = s;
        assert!(text(&mut ctl, &mut f, LARZUK_U).is_empty());
    }
    ctl.records[i].state = 3;
    assert_eq!(text(&mut ctl, &mut f, LARZUK_U), [(20171, 2)]);
    ctl.records[i].state = 4;
    assert!(text(&mut ctl, &mut f, LARZUK_U).is_empty());
    assert!(!super::active(&ctl, &mut f, i, P1, 511, 0));
    // Step 3: 40.0 set.
    f.p(P1).quests.flags[0].set(40, 0);
    assert_eq!(text(&mut ctl, &mut f, LARZUK_U), [(20178, 0)]);
    assert!(super::active(&ctl, &mut f, i, P1, 511, 0));
    f.p(P1).quests.flags[0].set(40, 4);
    assert!(text(&mut ctl, &mut f, LARZUK_U).is_empty());
    assert!(!super::active(&ctl, &mut f, i, P1, 511, 0));
    f.p(P1).quests.flags[0].set(40, 13);
    assert_eq!(text(&mut ctl, &mut f, LARZUK_U), [(20178, 2)]);
    // drehya as larzuk with 40.9.
    assert_eq!(text(&mut ctl, &mut f, DREHYA_U), [(20176, 0)]);
    f.p(P1).quests.flags[0].set(40, 9);
    assert_eq!(text(&mut ctl, &mut f, DREHYA_U), [(20176, 2)]);
    // tyrael3: 40.13 → table state 2; wants to talk while 40.7 clear.
    assert_eq!(text(&mut ctl, &mut f, TYRAEL_U), [(20175, 0)]);
    assert!(super::active(&ctl, &mut f, i, P1, 521, 0));
    // cain6: 40.5 clear → 4 (menu 0); set with 40.13 → 5; 40.10 → none.
    assert_eq!(text(&mut ctl, &mut f, CAIN_U), [(20177, 0)]);
    assert!(super::active(&ctl, &mut f, i, P1, 520, 0));
    f.p(P1).quests.flags[0].set(40, 5);
    assert_eq!(text(&mut ctl, &mut f, CAIN_U), [(20177, 2)]);
    assert!(!super::active(&ctl, &mut f, i, P1, 520, 0));
    f.p(P1).quests.flags[0].clear(40, 5);
    f.p(P1).quests.flags[0].set(40, 10);
    assert!(text(&mut ctl, &mut f, CAIN_U).is_empty());
    assert!(!super::active(&ctl, &mut f, i, P1, 520, 0));
}

// Covers: specs/world/quests-act5-2.md §8.4
#[test]
fn tyrael_chat_end_makes_the_last_portal() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    // Another NPC: nothing.
    call(
        &mut ctl,
        &mut f,
        ev(event::NPC_DEACTIVATE, Some(LARZUK_U), 0, 0),
    );
    assert!(!ctl.records[i].extra.a5.q6.last_portal_made);
    // Tyrael: `0x0058D7D0` for the first player in level 132: object 565
    // (flags 1, 1, 0) at a free spot from its position + (5, 0) in its
    // room (size 5, mask 0x400, limit 100); +0x98 := 1.
    f.pos.insert(P1, (10, 20, RoomId(3)));
    f.spot = Some((0, 1));
    f.a5_places = vec![Some(UnitId(0x95))];
    call(
        &mut ctl,
        &mut f,
        ev(event::NPC_DEACTIVATE, Some(TYRAEL_U), 0, 0),
    );
    assert_eq!(
        f.log,
        [
            "spot at 15 20 5 0x400 18 100",
            "place 565 15 21 room 3 [1, 1, 0]"
        ]
    );
    assert!(ctl.records[i].extra.a5.q6.last_portal_made);
    call(
        &mut ctl,
        &mut f,
        ev(event::NPC_DEACTIVATE, Some(TYRAEL_U), 0, 0),
    );
    assert_eq!(f.log.len(), 2);
    // Nobody in the Chamber: only +0x98.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.p(P1).level = Some(131);
    call(
        &mut ctl,
        &mut f,
        ev(event::NPC_DEACTIVATE, Some(TYRAEL_U), 0, 0),
    );
    assert!(f.log.is_empty() && ctl.records[i].extra.a5.q6.last_portal_made);
}

// Covers: specs/world/quests-act5-2.md §8.6 r1, §8.6 r2, §8.6 r3, §8.2, §edge-cases-original-bugs r9
#[test]
fn level_changes() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    ctl.records[i].extra.a5.q6.zoo_id = 0x1234;
    // Into Harrogath: the zoo id.
    call(
        &mut ctl,
        &mut f,
        ev(event::CHANGED_LEVEL, Some(P1), 110, 109),
    );
    let mut want = hex("50 2400 3412");
    want.resize(15, 0);
    assert_eq!(f.sent, [(P1, want)]);
    // Out of Harrogath in state 2: quick remove, state 3, status 1, 40.3.
    f.sent.clear();
    ctl.records[i].state = 2;
    ctl.records[i].guids.add(1);
    call(
        &mut ctl,
        &mut f,
        ev(event::CHANGED_LEVEL, Some(P1), 109, 110),
    );
    let r = &ctl.records[i];
    assert!(r.guids.0.is_empty());
    assert_eq!((r.state, r.status), (3, 1));
    assert!(f.flags(P1).get(40, 3));
    assert_eq!(status_msgs(&f), [(P1, 1)]);
    // Edge case 9: Pandemonium (133) → status 2 to all.
    f.sent.clear();
    call(&mut ctl, &mut f, ev(event::CHANGED_LEVEL, Some(P1), 1, 133));
    assert_eq!(ctl.records[i].status, 2);
    assert_eq!(status_msgs(&f), [(P1, 2)]);
    // Below 131: nothing.
    f.sent.clear();
    ctl.records[i].status = 1;
    call(
        &mut ctl,
        &mut f,
        ev(event::CHANGED_LEVEL, Some(P1), 120, 130),
    );
    assert!(f.sent.is_empty());
    // A player outside Act V is not flag-iterated.
    let (mut ctl, _) = control();
    let mut f = fake();
    add_p2(&mut f, 3, 103);
    ctl.records[i].state = 2;
    call(
        &mut ctl,
        &mut f,
        ev(event::CHANGED_LEVEL, Some(P1), 120, 131),
    );
    assert!(f.flags(P1).get(40, 2) && !f.flags(P2).get(40, 2));
    // Intro: no status, the iterate still runs.
    ctl.records[i].not_intro = false;
    ctl.records[i].status = 0;
    call(
        &mut ctl,
        &mut f,
        ev(event::CHANGED_LEVEL, Some(P1), 120, 131),
    );
    assert_eq!(ctl.records[i].status, 0);
}

// Covers: specs/world/quests-act5-2.md §8.7, §11
#[test]
fn game_start_join_and_leave() {
    for (b, want) in [(3, (3, 1, false)), (2, (3, 1, true)), (1, (0, 0, false))] {
        let (mut ctl, _) = control();
        let mut f = fake();
        f.p(P1).quests.flags[0].set(40, b);
        call(
            &mut ctl,
            &mut f,
            ev(event::PLAYER_STARTED_GAME, Some(P1), 0, 0),
        );
        let r = &ctl.records[rec(&ctl)];
        assert_eq!(
            (r.state, r.status, r.extra.a5.q6.by_sequence),
            want,
            "bit {b}"
        );
        assert!(f.log.is_empty() && f.sent.is_empty());
    }
    // 40.0 or 40.15: 40.10 and the progression call.
    for b in [0, 15] {
        let (mut ctl, _) = control();
        let mut f = fake();
        f.p(P1).quests.flags[0].set(40, b);
        f.client_flags.insert(P1, 0x20);
        call(
            &mut ctl,
            &mut f,
            ev(event::PLAYER_STARTED_GAME, Some(P1), 0, 0),
        );
        assert!(f.flags(P1).get(40, 10));
        assert_eq!(f.log, ["progression 1 0x0520"]);
    }
    // Join: 40.0 → 40.10.
    let (mut ctl, _) = control();
    let mut f = fake();
    call(
        &mut ctl,
        &mut f,
        ev(event::PLAYER_JOINED_GAME, Some(P1), 0, 0),
    );
    assert!(!f.flags(P1).get(40, 10));
    f.p(P1).quests.flags[0].set(40, 0);
    call(
        &mut ctl,
        &mut f,
        ev(event::PLAYER_JOINED_GAME, Some(P1), 0, 0),
    );
    assert!(f.flags(P1).get(40, 10));
    // Leave: both lists.
    let i = rec(&ctl);
    ctl.records[i].guids.add(1);
    ctl.records[i].extra.a5.q6.guids.add(1);
    call(
        &mut ctl,
        &mut f,
        ev(event::PLAYER_LEAVES_GAME, Some(P1), 0, 0),
    );
    assert!(ctl.records[i].guids.0.is_empty());
    assert!(ctl.records[i].extra.a5.q6.guids.0.is_empty());
    // The cow portal needs 40.0 in an expansion game (`quests.md` §8.4).
    f.p(P1).level = Some(1);
    f.spot = Some((5, 5));
    assert!(cow_portal(&mut ctl, &mut f, P1));
}

// Covers: specs/world/quests-act5-2.md §8.8, §10
#[test]
fn throne_and_chamber_portals() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    assert_eq!(ctl.records[i].extra.a5.q6.portal_mode, 1);
    assert_eq!(ctl.records[i].extra.a5.q6.last_portal_mode, 1);
    super::portal_init(&mut ctl, &mut f, PORTAL_U);
    super::portal_init(&mut ctl, &mut f, PORTAL_U);
    assert_eq!(f.log, ["mode 70 1", "mode 70 2"]);
    // In the Throne, closed until the Chamber opens.
    f.log.clear();
    f.p(P1).level = Some(131);
    assert_eq!(super::portal_operate(&ctl, &mut f, P1), 0);
    assert!(f.log.is_empty());
    assert!(!super::chamber_warp_open(&ctl));
    assert_eq!(warp_check(131, 132), WarpCheck::Delegate(0x0058_E640));
    // `0x0058E600` from the throne AI: open, status 3 to all.
    super::chamber_open(&mut ctl, &mut f);
    assert!(super::chamber_warp_open(&ctl));
    assert_eq!(ctl.records[i].status, 3);
    assert_eq!(status_msgs(&f), [(P1, 3)]);
    f.sent.clear();
    super::chamber_open(&mut ctl, &mut f);
    assert!(f.sent.is_empty());
    super::portal_operate(&ctl, &mut f, P1);
    // Elsewhere: to the Throne, entry 0.
    f.p(P1).level = Some(129);
    super::portal_operate(&ctl, &mut f, P1);
    assert_eq!(f.log, ["warp 1 132 11", "warp 1 131 0"]);
    // Tyrael's spawn `0x0058E920(game, room, unit)` (§8.8): a free spot
    // from the unit's position − (5, 5) in the given room (size 5, mask
    // 0x400, limit 100), then tyrael3 there (mode 1, spread 4, 0x42).
    f.log.clear();
    f.pos.insert(BAAL_U, (40, 50, RoomId(7)));
    f.spot = Some((1, 2));
    f.spawns = vec![Some(UnitId(0x77))];
    assert_eq!(
        super::spawn_tyrael(&mut f, RoomId(9), BAAL_U),
        Some(UnitId(0x77))
    );
    assert_eq!(
        f.log,
        [
            "spot at 35 45 5 0x400 19 100",
            "spawn 521 36 47 room 9 mode 1 spread 4 flags 0x42"
        ]
    );
    // No spot: nothing.
    f.log.clear();
    f.spot = None;
    assert_eq!(super::spawn_tyrael(&mut f, RoomId(9), BAAL_U), None);
    assert_eq!(f.log, ["spot at 35 45 5 0x400 19 100"]);
}

// Covers: specs/world/quests-act5-2.md §8.8, §11
#[test]
fn last_portal() {
    let (mut ctl, _) = control();
    let mut f = fake();
    // Before its init (+0x9C = 1): nothing.
    assert_eq!(super::last_portal_operate(&mut ctl, &mut f, P1), 0);
    assert!(f.log.is_empty());
    super::last_portal_init(&mut ctl, &mut f, PORTAL_U);
    assert_eq!(f.log, ["mode 70 1"]);
    // Without 40.13: refused.
    f.log.clear();
    super::last_portal_operate(&mut ctl, &mut f, P1);
    assert_eq!(f.log, ["sound 1 19"]);
    // With 40.13, busy: warp and the save pass only.
    f.p(P1).quests.flags[0].set(40, 13);
    f.q2_busy.push(P1);
    f.log.clear();
    super::last_portal_operate(&mut ctl, &mut f, P1);
    assert_eq!(f.log, ["warp 1 109 0"]);
    assert_eq!(ctl.take_host_requests(), [HostRequest::SavePass]);
    assert!(!f.flags(P1).get(40, 10));
    // Not busy: interaction reset, +0x4C := 1, `61 07`, 40.10.
    f.q2_busy.clear();
    f.log.clear();
    super::last_portal_operate(&mut ctl, &mut f, P1);
    assert_eq!(f.log, ["warp 1 109 0", "interact 1 None"]);
    assert_eq!(ctl.take_host_requests(), [HostRequest::SavePass]);
    assert_eq!(f.players[&P1].byte4c, 1);
    assert_eq!(f.sent, [(P1, vec![0x61, 0x07])]);
    assert!(f.flags(P1).get(40, 10));
}

// Covers: specs/world/quests-act5-2.md §8.8
#[test]
fn zoo() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    f.a5_monstats_rows = 700;
    let mut seed = ctl.seed;
    let draws: Vec<u32> = (0..10).map(|_| 1 + seed.roll(699)).collect();
    // The third draw is zoo-flagged.
    f.a5_zoo = vec![draws[2]];
    let mut want_seed = ctl.seed;
    for _ in 0..3 {
        want_seed.step();
    }
    add_p2(&mut f, 4, 109);
    super::zoo_init(&mut ctl, &mut f);
    assert_eq!(ctl.seed, want_seed);
    let x = &ctl.records[i].extra.a5.q6;
    assert!(x.zoo_chosen && x.zoo_id == draws[2]);
    let mut m = vec![0x50, 36, 0];
    m.extend((draws[2] as u16).to_le_bytes());
    m.resize(15, 0);
    assert_eq!(f.sent, [(P1, m.clone()), (P2, m)]);
    // Once per game.
    f.sent.clear();
    super::zoo_init(&mut ctl, &mut f);
    assert!(f.sent.is_empty());
    // No hit in 10 draws: id 0, ten steps.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.a5_monstats_rows = 700;
    let mut want_seed = ctl.seed;
    for _ in 0..10 {
        want_seed.step();
    }
    super::zoo_init(&mut ctl, &mut f);
    assert_eq!(ctl.seed, want_seed);
    assert_eq!(ctl.records[i].extra.a5.q6.zoo_id, 0);
}

// Covers: specs/world/quests-act5-2.md §8.9, §6.10, §7.10
#[test]
fn sequence_chain_34_to_36() {
    let (mut ctl, _) = control();
    let mut f = fake();
    add_p2(&mut f, 3, 103); // not in Act V
                            // Chains 31–33 finished: the walk reaches 34 and opens it.
    for c in [31, 32, 33] {
        ctl.record_mut(c).unwrap().state = 5;
    }
    assert!(super::super::sequence(&mut ctl, &mut f, 31));
    assert_eq!(ctl.record(34).unwrap().state, 1);
    assert_eq!(ctl.record(35).unwrap().state, 0);
    // 34 done (state ≥ 4) → 35 opens.
    ctl.record_mut(34).unwrap().state = 4;
    assert!(super::super::sequence(&mut ctl, &mut f, 31));
    assert_eq!(ctl.record(35).unwrap().state, 1);
    assert_eq!(ctl.record(36).unwrap().state, 0);
    // 35 done (state 5) → 36: +0x90, state 2, status 1 to all, 40.2 for
    // players in Act V.
    ctl.record_mut(35).unwrap().state = 5;
    assert!(super::super::sequence(&mut ctl, &mut f, 31));
    let r = ctl.record(36).unwrap();
    assert_eq!((r.state, r.status), (2, 1));
    assert!(r.extra.a5.q6.by_sequence);
    assert!(f.flags(P1).get(40, 2) && !f.flags(P2).get(40, 2));
    assert_eq!(status_msgs(&f), [(P1, 1)]);
    // Chain 36 always returns 1; past state 0 it changes nothing.
    f.sent.clear();
    assert!(super::super::sequence(&mut ctl, &mut f, 36));
    assert!(f.sent.is_empty());
    // Intro 36: returns 1, nothing.
    let (mut ctl, _) = control();
    let i = rec(&ctl);
    ctl.records[i].not_intro = false;
    assert!(super::sequence(&mut ctl, &mut f, i));
    assert_eq!(ctl.records[i].state, 0);
}

// Covers: specs/world/quests-act5-2.md §edge-cases-original-bugs r11
#[test]
fn baal_without_a_victim_room_sends_only_fx_19() {
    let (mut ctl, _) = control();
    let mut f = fake();
    f.pos.remove(&BAAL_U);
    let i = rec(&ctl);
    let before = (ctl.records[i].state, ctl.records[i].status);
    kill_baal(&mut ctl, &mut f, Some(P1));
    assert!(f.log.is_empty(), "no credit, gold or missile 625");
    assert!(f.sent.iter().any(|m| m.1 == [0x89, 19]));
    assert!(!f.flags(P1).get(40, 0));
    assert_eq!((ctl.records[i].state, ctl.records[i].status), before);
    assert!(ctl.take_host_requests().is_empty());
}

// Covers: specs/world/quests-act5-2.md §edge-cases-original-bugs r13
#[test]
fn last_portal_is_tried_once_even_without_a_free_spot() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    f.pos.insert(P1, (10, 20, RoomId(3)));
    f.spot = None;
    call(
        &mut ctl,
        &mut f,
        ev(event::NPC_DEACTIVATE, Some(TYRAEL_U), 0, 0),
    );
    // One search, no object, and +0x98 is set: the walk ended at the
    // first player in level 132 and nothing retries.
    assert_eq!(f.log, ["spot at 15 20 5 0x400 18 100"]);
    assert!(ctl.records[i].extra.a5.q6.last_portal_made);
    call(
        &mut ctl,
        &mut f,
        ev(event::NPC_DEACTIVATE, Some(TYRAEL_U), 0, 0),
    );
    assert_eq!(f.log.len(), 1);
}

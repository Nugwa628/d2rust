// Spec: specs/world/quests-act1-rest.md
//! A1Q4 remainders on the quests' fake world: the gibbet operate and its
//! event 7 (§1), the Cairn stone init and the Tristram-portal timer
//! (§2), the town-Cain marker and "Cain leaves Tristram" (§3), the party
//! step (§6) and the stone-order 0x50 (§7), from the spec's test vectors.
use super::tests::*;
use super::*;
use crate::units::RoomId as RoomIdT;

const P3: UnitId = UnitId(3);
const P4: UnitId = UnitId(4);
const P5: UnitId = UnitId(5);
const P6: UnitId = UnitId(6);
const GIBBET: UnitId = UnitId(0x50);
const GIBBET_ROOM: RoomIdT = RoomIdT(3);
const CAIN_T: UnitId = UnitId(0x15);
const CAIN5_U: UnitId = UnitId(0x16);
const STONE: UnitId = UnitId(0x70);
const MARKER: UnitId = UnitId(0x30);

fn npc_kind(class: u16) -> UnitKind {
    UnitKind::Monster {
        class: u32::from(class),
        superunique: None,
        owner: None,
    }
}

fn player(guid: u32, act: Option<u8>, level: Option<u32>) -> Player {
    Player {
        guid,
        act,
        level,
        ..Player::default()
    }
}

fn q4(ctl: &QuestControl) -> &act1::q4::Extra4 {
    &ctl.record(4).unwrap().extra.q4
}

fn q4_mut(ctl: &mut QuestControl) -> &mut act1::q4::Extra4 {
    &mut ctl.record_mut(4).unwrap().extra.q4
}

/// The expected S→C 0x28 (`28 06 00000000 00` + the 96-byte record)
/// for one player as it stands now.
fn msg28(f: &Fake, p: UnitId) -> Vec<u8> {
    let mut m = hex("28 06 00000000 00");
    m.extend_from_slice(&f.flags(p).0);
    m
}

/// A fake with the gibbet (object 26, mode 0) at (20, 30) in room 3,
/// frame 100.
fn gibbet_fake() -> Fake {
    let mut f = Fake::new();
    f.frame = 100;
    f.objects.insert(GIBBET, (0x50, 26, 0));
    f.pos.insert(GIBBET, (20, 30, GIBBET_ROOM));
    f
}

fn drop_chain4(ctl: &mut QuestControl) {
    ctl.records.retain(|r| r.chain != 4);
    assert!(ctl.find(4).is_none());
}

// ------------------------------------------------------------ §1.1

// Covers: specs/world/quests-act1-rest.md §1 text, §1.1 r1, §1.1 r2, §1.1 r3, §1.1 r4, §1.1 r5
// Covers: specs/world/quests-act1-rest.md §1.1 r6, §1.1 r7, §1.1 r8, §1.1 r9
#[test]
fn gibbet_operate_vectors() {
    // Vector: R slot 4 = 0, object mode 0, single player → object mode
    // 1; event 1 at frame + (0x1000 >> 8) and event 7 at frame + 17;
    // X +0x54 = 3; R slot 4 = 0x2002; 0x28.
    let (mut ctl, _) = control();
    let mut f = gibbet_fake();
    assert!(ctl.record(4).unwrap().not_intro);
    act1::q4::gibbet_operate(&mut ctl, &mut f, GIBBET, P1);
    assert_eq!(
        f.log,
        ["mode 80 1", "event1 80 116", "event7 80 117", "refresh 80"]
    );
    assert_eq!(f.flags(P1).word(4), 0x2002);
    assert_eq!(f.sent, [(P1, msg28(&f, P1))]);
    assert_eq!(f.sent[0].1.len(), 103);
    assert_eq!(f.sent[0].1[15..17], [0x02, 0x20]);
    let x = q4(&ctl);
    assert_eq!((x.gibbet_open, x.gibbet_player), (3, 1));
    assert_eq!(f.objects[&GIBBET].2, 1);

    // Vector: R slot 4 = 0x0002 (4.1) → sound 19; nothing else. 4.0
    // alone the same.
    for b in [bit::REWARD_PENDING, bit::REWARD_GRANTED] {
        let (mut ctl, _) = control();
        let mut f = gibbet_fake();
        f.p(P1).quests.flags[0].set(4, b);
        let before = f.flags(P1);
        act1::q4::gibbet_operate(&mut ctl, &mut f, GIBBET, P1);
        assert_eq!(f.log, ["sound 1 19"]);
        assert!(f.sent.is_empty());
        assert_eq!(f.flags(P1), before);
        assert_eq!(f.objects[&GIBBET].2, 0);
        assert_eq!(q4(&ctl).gibbet_open, 0);
    }

    // Vector: state 6 → nothing. Cain gone (+0x50) and an intro record
    // the same, even with 4.1 (no sound).
    for case in 0..3 {
        let (mut ctl, _) = control();
        let mut f = gibbet_fake();
        f.p(P1).quests.flags[0].set(4, bit::REWARD_PENDING);
        match case {
            0 => ctl.record_mut(4).unwrap().state = 6,
            1 => q4_mut(&mut ctl).cain_gone = true,
            _ => ctl.record_mut(4).unwrap().not_intro = false,
        }
        act1::q4::gibbet_operate(&mut ctl, &mut f, GIBBET, P1);
        assert!(f.log.is_empty() && f.sent.is_empty(), "case {case}");
    }
    // State 5 is below 6: the gibbet opens.
    let (mut ctl, _) = control();
    let mut f = gibbet_fake();
    ctl.record_mut(4).unwrap().state = 5;
    act1::q4::gibbet_operate(&mut ctl, &mut f, GIBBET, P1);
    assert_eq!(f.log[0], "mode 80 1");

    // Object mode ≠ 0 (already open): nothing after the flag test.
    let (mut ctl, _) = control();
    let mut f = gibbet_fake();
    f.objects.get_mut(&GIBBET).unwrap().2 = 1;
    act1::q4::gibbet_operate(&mut ctl, &mut f, GIBBET, P1);
    assert!(f.log.is_empty() && f.sent.is_empty());
    assert_eq!(f.flags(P1).word(4), 0);

    // No chain 4 record: X is none and the steps go on (no extra write).
    let (mut ctl, _) = control();
    drop_chain4(&mut ctl);
    let mut f = gibbet_fake();
    act1::q4::gibbet_operate(&mut ctl, &mut f, GIBBET, P1);
    assert_eq!(
        f.log,
        ["mode 80 1", "event1 80 116", "event7 80 117", "refresh 80"]
    );
    assert_eq!(f.sent, [(P1, msg28(&f, P1))]);
    assert!(ctl.faults.is_empty());
}

// Covers: specs/world/quests-act1-rest.md §1.1 r9, §6 text, §6.1, §6.2
#[test]
fn gibbet_operate_party_step() {
    let (mut ctl, _) = control();
    let mut f = gibbet_fake();
    // P2: Act I, level 2 → credited. P3: Act II → no. P4: level 0 → no.
    // P5: already 4.0 → no. P6: no room → no. P1 itself is listed and
    // already has 4.1.
    f.players.insert(P2, player(2, Some(0), Some(2)));
    f.players.insert(P3, player(3, Some(1), Some(40)));
    f.players.insert(P4, player(4, Some(0), Some(0)));
    f.players.insert(P5, player(5, Some(0), Some(3)));
    f.players.insert(P6, player(6, Some(0), None));
    f.p(P5).quests.flags[0].set(4, bit::REWARD_GRANTED);
    f.party.insert(P1, vec![P1, P2, P3, P4, P5, P6]);
    act1::q4::gibbet_operate(&mut ctl, &mut f, GIBBET, P1);
    assert_eq!(f.sent, [(P1, msg28(&f, P1)), (P2, msg28(&f, P2))]);
    let words: Vec<u16> = [P1, P2, P3, P4, P5, P6]
        .iter()
        .map(|&p| f.flags(p).word(4))
        .collect();
    assert_eq!(words, [0x2002, 0x2002, 0, 0, 0x0001, 0]);
    // The operating player in no party (party id 0xFFFF): no member step
    // even though another player would qualify.
    let (mut ctl, _) = control();
    let mut f = gibbet_fake();
    f.players.insert(P2, player(2, Some(0), Some(2)));
    act1::q4::gibbet_operate(&mut ctl, &mut f, GIBBET, P1);
    assert_eq!(f.sent, [(P1, msg28(&f, P1))]);
    assert_eq!(f.flags(P2).word(4), 0);
}

// ------------------------------------------------------------ §1.2

// Covers: specs/world/quests-act1-rest.md §1.2 text, §1.2 r1, §1.2 r2, §1.2 r3, §1.2 r5, §1.2 r6
#[test]
fn gibbet_event_cain_spawned() {
    // Vector: Cain spawned, P in Tristram lacking 4.0/4.1, state 5,
    // init_no 6 → P: 4.13, 4.1, 0x28; then `5d 04 00 06 0000`. Run
    // through the object event dispatcher (class 0x1A).
    let (mut ctl, _) = control();
    let mut f = gibbet_fake();
    f.p(P1).level = Some(38);
    f.spawns = vec![Some(CAIN_T)];
    ctl.record_mut(4).unwrap().state = 5;
    ctl.record_mut(4).unwrap().flags = 3;
    q4_mut(&mut ctl).gibbet_player = 1;
    assert_eq!(ctl.record(4).unwrap().init_no, 6);
    object_event(&mut ctl, &mut f, GIBBET, 0x1A);
    assert_eq!(
        f.log,
        [
            "mode 80 3",
            "spawn 146 23 33 mode 1 r 4294967295",
            "flags 21 0x3000000",
            "sound 1 48",
        ]
    );
    assert_eq!(f.flags(P1).word(4), 0x2002);
    assert_eq!(f.sent, [(P1, msg28(&f, P1)), (P1, hex("5d 04 00 06 0000"))]);
    let r = ctl.record(4).unwrap();
    assert_eq!((r.state, r.status, r.flags), (5, 6, 0));
    assert_eq!(r.extra.q4.gibbet_open, 3);
    assert!(!r.extra.q4.cain_failed && !r.extra.q4.town_cain_due);
    assert!(ctl.faults.is_empty());

    // Same, a second player Q in Act I outside Tristram → Q: 4.14,
    // `5d 04 00 0c 0000` (L5), then `5d 04 00 0c 0000` (L1, now → 12).
    let (mut ctl, _) = control();
    let mut f = gibbet_fake();
    f.p(P1).level = Some(38);
    f.players.insert(P2, player(2, Some(0), Some(2)));
    f.spawns = vec![Some(CAIN_T)];
    ctl.record_mut(4).unwrap().state = 5;
    q4_mut(&mut ctl).gibbet_player = 1;
    act1::q4::gibbet_event(&mut ctl, &mut f, GIBBET);
    assert_eq!(f.flags(P2).word(4), 0x4000);
    assert_eq!(
        f.sent,
        [
            (P1, msg28(&f, P1)),
            (P2, hex("5d 04 00 0c 0000")),
            (P1, hex("5d 04 00 06 0000")),
            (P2, hex("5d 04 00 0c 0000")),
        ]
    );

    // The first spawn fails, the free spot (size 2, mask 0x100, radius
    // 3, limit 100) is found and the second spawn there succeeds; the
    // opener's GUID is absent: no sound 48.
    let (mut ctl, _) = control();
    let mut f = gibbet_fake();
    f.spawns = vec![None, Some(CAIN_T)];
    f.spot = Some((2, -1));
    ctl.record_mut(4).unwrap().state = 5;
    q4_mut(&mut ctl).gibbet_player = u32::MAX;
    act1::q4::gibbet_event(&mut ctl, &mut f, GIBBET);
    assert_eq!(
        f.log,
        [
            "mode 80 3",
            "spawn 146 23 33 mode 1 r 4294967295",
            "spot at 23 33 2 0x100 3 100",
            "spawn 146 25 32 mode 1 r 4294967295",
            "flags 21 0x3000000",
        ]
    );
    assert!(!q4(&ctl).cain_failed);

    // An intro record or Cain gone: nothing.
    for gone in [false, true] {
        let (mut ctl, _) = control();
        let mut f = gibbet_fake();
        f.spawns = vec![Some(CAIN_T)];
        if gone {
            q4_mut(&mut ctl).cain_gone = true;
        } else {
            ctl.record_mut(4).unwrap().not_intro = false;
        }
        act1::q4::gibbet_event(&mut ctl, &mut f, GIBBET);
        assert!(f.log.is_empty() && f.sent.is_empty());
        assert_eq!(q4(&ctl).gibbet_open, 0);
        assert!(ctl.faults.is_empty());
    }

    // No chain 4 record: the fatal assert.
    let (mut ctl, _) = control();
    drop_chain4(&mut ctl);
    let mut f = gibbet_fake();
    object_event(&mut ctl, &mut f, GIBBET, 0x1A);
    assert_eq!(ctl.faults, [QuestError::Fatal(0x0059_3290)]);
    assert!(f.log.is_empty() && f.sent.is_empty());
}

// Covers: specs/world/quests-act1-rest.md §1.2 r3, §1.2 r4, §1.2 r6, §edge-cases-original-bugs r5
#[test]
fn gibbet_event_cain_spawn_fails() {
    // Both spawns fail. P1 in the Rogue Encampment, P2 and P3 in
    // Tristram: the portal to town is owned by P2 (first in walk order),
    // at gibbet + (6, 6), class 59, level 1, free-spot search.
    let setup = || {
        let (mut ctl, _) = control();
        let mut f = gibbet_fake();
        f.players.insert(P2, player(2, Some(0), Some(38)));
        f.players.insert(P3, player(3, Some(0), Some(38)));
        f.spot = Some((1, 1));
        ctl.record_mut(4).unwrap().state = 5;
        q4_mut(&mut ctl).gibbet_player = 2;
        (ctl, f)
    };
    let (mut ctl, mut f) = setup();
    f.portals = vec![Some(UnitId(0x99))];
    act1::q4::gibbet_event(&mut ctl, &mut f, GIBBET);
    assert_eq!(
        f.log,
        [
            "mode 80 3",
            "spawn 146 23 33 mode 1 r 4294967295",
            "spot at 23 33 2 0x100 3 100",
            "spawn 146 24 34 mode 1 r 4294967295",
            "portal Some(2) room 3 26 36 59 1 exact false",
        ]
    );
    let x = q4(&ctl);
    assert_eq!(x.found_player, Some(P2));
    assert!(x.out_portal && x.town_cain_due && x.cain_failed && !x.town_cain);
    // L4 (P2, P3 in Tristram), L5 (P1), then L1 with status 6.
    assert_eq!(
        f.sent,
        [
            (P2, msg28(&f, P2)),
            (P3, msg28(&f, P3)),
            (P1, hex("5d 04 00 0c 0000")),
            (P1, hex("5d 04 00 0c 0000")),
            (P2, hex("5d 04 00 06 0000")),
            (P3, hex("5d 04 00 06 0000")),
        ]
    );
    let r = ctl.record(4).unwrap();
    assert_eq!((r.status, r.flags), (6, 0));

    // The portal creation fails: +0x66 stays 0; the rest as before.
    let (mut ctl, mut f) = setup();
    act1::q4::gibbet_event(&mut ctl, &mut f, GIBBET);
    assert_eq!(f.log[4], "portal Some(2) room 3 26 36 59 1 exact false");
    let x = q4(&ctl);
    assert!(!x.out_portal && x.town_cain_due && x.cain_failed);

    // +0x66 already set: no second portal.
    let (mut ctl, mut f) = setup();
    q4_mut(&mut ctl).out_portal = true;
    act1::q4::gibbet_event(&mut ctl, &mut f, GIBBET);
    assert_eq!(f.log.len(), 4);
    assert!(!f.log.iter().any(|l| l.starts_with("portal")));

    // No free spot (one spawn try), nobody in Tristram, Cain already in
    // town (+0x51): no portal, +0x52 untouched, +0x62 set.
    let (mut ctl, mut f) = setup();
    f.spot = None;
    f.p(P2).level = Some(2);
    f.p(P3).level = None;
    f.portals = vec![Some(UnitId(0x99))];
    q4_mut(&mut ctl).town_cain = true;
    act1::q4::gibbet_event(&mut ctl, &mut f, GIBBET);
    assert_eq!(
        f.log,
        [
            "mode 80 3",
            "spawn 146 23 33 mode 1 r 4294967295",
            "spot at 23 33 2 0x100 3 100",
        ]
    );
    let x = q4(&ctl);
    assert_eq!(x.found_player, None);
    assert!(!x.out_portal && !x.town_cain_due && x.cain_failed);
    assert_eq!(f.portals, [Some(UnitId(0x99))]);
}

// ------------------------------------------------------------ §2

fn stone_fake() -> Fake {
    let mut f = Fake::new();
    f.objects.insert(STONE, (0x70, 17, 0));
    f.pos.insert(STONE, (40, 50, RoomIdT(4)));
    f
}

// Covers: specs/world/quests-act1-rest.md §2 text, §2.2 text, §2.2 r1, §2.2 r2, §2.2 r3, §edge-cases-original-bugs r3
#[test]
fn stone_init_vectors() {
    // Vector: not-intro 1, X +0x4C 0, +0x4D 0, +0x50 0 → mode unchanged.
    let (mut ctl, _) = control();
    let mut f = stone_fake();
    act1::q4::stone_init(&mut ctl, &mut f, STONE, 17);
    assert!(f.log.is_empty() && ctl.timers.is_empty());
    assert_eq!(q4(&ctl).portal_stone, 0);

    // Vector: stone 17, X +0x4C 1, +0x45 0, +0x44 0 → +0x4C 0; +0x40 =
    // GUID; +0x44 1; timer period 1; mode 2.
    let (mut ctl, _) = control();
    let mut f = stone_fake();
    q4_mut(&mut ctl).b4c = true;
    act1::q4::stone_init(&mut ctl, &mut f, STONE, 17);
    assert_eq!(f.log, ["mode 112 2"]);
    let x = q4(&ctl);
    assert!(!x.b4c && x.portal_timer && !x.portal_made);
    assert_eq!(x.portal_stone, 0x70);
    assert_eq!(
        ctl.timers,
        [QuestTimer {
            func: TimerFn::TristramPortal,
            chain: 4,
            due: 1,
            period: 1,
        }]
    );
    // A second stone 17 with the timer pending: +0x40 moves, no timer.
    let other = UnitId(0x71);
    f.objects.insert(other, (0x71, 17, 0));
    q4_mut(&mut ctl).b4c = true;
    act1::q4::stone_init(&mut ctl, &mut f, other, 17);
    assert_eq!(q4(&ctl).portal_stone, 0x71);
    assert_eq!(ctl.timers.len(), 1);
    // Only the first stone after "done" takes step 3: +0x4C is now 0.
    f.log.clear();
    act1::q4::stone_init(&mut ctl, &mut f, STONE, 17);
    assert!(f.log.is_empty());

    // Not-intro 0, a stone other than 17: mode 2, no timer.
    let (mut ctl, _) = control();
    let mut f = stone_fake();
    ctl.record_mut(4).unwrap().not_intro = false;
    act1::q4::stone_init(&mut ctl, &mut f, STONE, 18);
    assert_eq!(f.log, ["mode 112 2"]);
    assert!(ctl.timers.is_empty() && q4(&ctl).portal_stone == 0);
    // The portal already made (+0x45): stone 17 only gets mode 2.
    let (mut ctl, _) = control();
    let mut f = stone_fake();
    {
        let x = q4_mut(&mut ctl);
        (x.b4c, x.portal_made) = (true, true);
    }
    act1::q4::stone_init(&mut ctl, &mut f, STONE, 17);
    assert_eq!(f.log, ["mode 112 2"]);
    assert!(ctl.timers.is_empty() && q4(&ctl).portal_stone == 0);

    // Quest live with +0x4D or +0x50: mode 2.
    for case in 0..2 {
        let (mut ctl, _) = control();
        let mut f = stone_fake();
        if case == 0 {
            q4_mut(&mut ctl).b4d = true;
        } else {
            q4_mut(&mut ctl).cain_gone = true;
        }
        act1::q4::stone_init(&mut ctl, &mut f, STONE, 19);
        assert_eq!(f.log, ["mode 112 2"]);
        assert!(ctl.timers.is_empty());
    }
    // The per-stone reset byte +0x4B + c (never set by the original):
    // when 1 it is cleared and the stone goes back to mode 0.
    let (mut ctl, _) = control();
    let mut f = stone_fake();
    q4_mut(&mut ctl).stone_reset[1] = true;
    act1::q4::stone_init(&mut ctl, &mut f, STONE, 17);
    assert!(f.log.is_empty());
    act1::q4::stone_init(&mut ctl, &mut f, STONE, 18);
    assert_eq!(f.log, ["mode 112 0"]);
    assert_eq!(q4(&ctl).stone_reset, [false; 5]);

    // No chain 4 record: mode 2 unless it is 2 already.
    let (mut ctl, _) = control();
    drop_chain4(&mut ctl);
    let mut f = stone_fake();
    act1::q4::stone_init(&mut ctl, &mut f, STONE, 17);
    assert_eq!(f.log, ["mode 112 2"]);
    act1::q4::stone_init(&mut ctl, &mut f, STONE, 17);
    assert_eq!(f.log, ["mode 112 2"]);
    assert!(ctl.timers.is_empty());
}

// Covers: specs/world/quests-act1-rest.md §2.3 text, §2.3 r1, §2.3 r2, §2.3 r3, §edge-cases-original-bugs r4
#[test]
fn tristram_portal_timer() {
    let setup = || {
        let (mut ctl, _) = control();
        let mut f = stone_fake();
        q4_mut(&mut ctl).b4c = true;
        act1::q4::stone_init(&mut ctl, &mut f, STONE, 17);
        f.log.clear();
        (ctl, f)
    };
    // Creation fails: the timer stays and runs again every 2 updates
    // (due < tick), at the stone + (4, 4), owner none, exactly there.
    let (mut ctl, mut f) = setup();
    let want = "portal None room 4 44 54 60 38 exact true";
    ctl.update(&mut f);
    assert!(f.log.is_empty());
    ctl.update(&mut f);
    assert_eq!((ctl.tick, f.log.clone()), (2, vec![want.to_string()]));
    assert_eq!(ctl.timers.len(), 1);
    assert_eq!(ctl.timers[0].due, 3);
    ctl.update(&mut f);
    assert_eq!(f.log.len(), 1);
    ctl.update(&mut f);
    assert_eq!(f.log, [want, want]);
    assert!(q4(&ctl).portal_timer && !q4(&ctl).portal_made);
    // Created: +0x45 1, +0x44 0, the timer removed.
    f.portals = vec![Some(UnitId(0x99))];
    ctl.update(&mut f);
    ctl.update(&mut f);
    assert_eq!(f.log, [want, want, want]);
    assert!(ctl.timers.is_empty());
    let x = q4(&ctl);
    assert!(x.portal_made && !x.portal_timer);

    // The stone gone: +0x44 0, removed, no portal.
    let (mut ctl, mut f) = setup();
    f.objects.remove(&STONE);
    f.portals = vec![Some(UnitId(0x99))];
    ctl.update(&mut f);
    ctl.update(&mut f);
    assert!(f.log.is_empty() && ctl.timers.is_empty());
    let x = q4(&ctl);
    assert!(!x.portal_timer && !x.portal_made);
}

// ------------------------------------------------------------ §3

fn marker_fake() -> Fake {
    let mut f = Fake::new();
    f.objects.insert(MARKER, (0x30, 385, 0));
    f.pos.insert(MARKER, (10, 10, RoomIdT(1)));
    f.rooms.insert(RoomIdT(1), (0, 0, 12, 12));
    f
}

// Covers: specs/world/quests-act1-rest.md §3 text
#[test]
fn marker_init_registers_and_spawns() {
    // No town Cain due: the marker is registered only.
    let (mut ctl, _) = control();
    let mut f = marker_fake();
    act1::q4::marker_init(&mut ctl, &mut f, MARKER, Some(RoomIdT(1)), 7, 8);
    assert!(f.log.is_empty());
    let x = q4(&ctl);
    assert!(x.marker_known && x.marker_guid == 0x30 && x.marker_pos == (7, 8));
    assert!(!x.town_cain);
    // +0x52 = 1, +0x51 = 0: the town Cain spawns at the init args' (x,
    // y) in their room.
    let (mut ctl, _) = control();
    let mut f = marker_fake();
    f.spawns = vec![Some(CAIN5_U)];
    q4_mut(&mut ctl).town_cain_due = true;
    act1::q4::marker_init(&mut ctl, &mut f, MARKER, Some(RoomIdT(1)), 10, 10);
    assert_eq!(
        f.log,
        [
            "spot at 10 10 2 0x100 1 100",
            "spawn 265 10 10 mode 1 r 5",
            "flags 22 0x3000000",
        ]
    );
    let x = q4(&ctl);
    assert!(x.town_cain && !x.town_cain_due && x.town_cain_guid == 0x16);
    assert!(x.marker_known && x.marker_pos == (10, 10));
    // +0x51 = 1 already: no spawn.
    let (mut ctl, _) = control();
    let mut f = marker_fake();
    f.spawns = vec![Some(CAIN5_U)];
    {
        let x = q4_mut(&mut ctl);
        (x.town_cain_due, x.town_cain) = (true, true);
    }
    act1::q4::marker_init(&mut ctl, &mut f, MARKER, Some(RoomIdT(1)), 10, 10);
    assert!(f.log.is_empty());
    // No chain 4 record: nothing.
    let (mut ctl, _) = control();
    drop_chain4(&mut ctl);
    let mut f = marker_fake();
    act1::q4::marker_init(&mut ctl, &mut f, MARKER, Some(RoomIdT(1)), 10, 10);
    assert!(f.log.is_empty() && ctl.faults.is_empty());
}

// Covers: specs/world/quests-act1-rest.md §3 r1, §3 r2, §3 r3
#[test]
fn cain_leaves_tristram() {
    // The marker never registered: +0x91 and +0x52 only.
    let (mut ctl, _) = control();
    let mut f = marker_fake();
    f.spawns = vec![Some(CAIN5_U)];
    act1::q4::cain_leaves_tristram(&mut ctl, &mut f);
    assert!(f.log.is_empty());
    let x = q4(&ctl);
    assert!(x.b91 && x.town_cain_due && !x.town_cain && !x.cain_portal);

    // Registered, the marker gone: nothing more.
    let (mut ctl, _) = control();
    let mut f = marker_fake();
    f.objects.remove(&MARKER);
    f.spawns = vec![Some(CAIN5_U)];
    {
        let x = q4_mut(&mut ctl);
        (x.marker_known, x.marker_guid) = (true, 0x30);
    }
    act1::q4::cain_leaves_tristram(&mut ctl, &mut f);
    assert!(f.log.is_empty());
    assert!(q4(&ctl).b91 && q4(&ctl).town_cain_due);

    // Registered, +0x51 = 0: the marker init runs again at the marker's
    // own position, the town Cain spawns (+0x51 = 1), so the cain portal
    // (object 189, mode 1) follows at Cain's position in its room.
    let (mut ctl, _) = control();
    let mut f = marker_fake();
    f.monsters
        .insert(CAIN5_U, (0x16, npc::CAIN5, npc_kind(npc::CAIN5)));
    f.pos.insert(CAIN5_U, (10, 10, RoomIdT(1)));
    f.spawns = vec![Some(CAIN5_U)];
    {
        let x = q4_mut(&mut ctl);
        (x.marker_known, x.marker_guid, x.marker_pos) = (true, 0x30, (0, 0));
    }
    act1::q4::cain_leaves_tristram(&mut ctl, &mut f);
    assert_eq!(
        f.log,
        [
            "spot at 10 10 2 0x100 1 100",
            "spawn 265 10 10 mode 1 r 5",
            "flags 22 0x3000000",
            "object 189 10 10 room 1 mode 1",
        ]
    );
    let x = q4(&ctl);
    assert!(x.b91 && x.town_cain && !x.town_cain_due);
    assert_eq!(x.marker_pos, (10, 10));
    assert!(x.cain_portal && x.cain_portal_guid == 0xA001);
    assert_eq!(f.objects[&UnitId(0xA001)], (0xA001, 189, 1));

    // Cain already in town (+0x51 = 1): no spawn, +0x52 stays 1, the
    // portal at Cain's position (room found from Cain's room).
    let (mut ctl, _) = control();
    let mut f = marker_fake();
    f.monsters
        .insert(CAIN5_U, (0x16, npc::CAIN5, npc_kind(npc::CAIN5)));
    f.pos.insert(CAIN5_U, (5, 6, RoomIdT(1)));
    {
        let x = q4_mut(&mut ctl);
        (x.marker_known, x.marker_guid) = (true, 0x30);
        (x.town_cain, x.town_cain_guid) = (true, 0x16);
    }
    act1::q4::cain_leaves_tristram(&mut ctl, &mut f);
    assert_eq!(f.log, ["object 189 5 6 room 1 mode 1"]);
    let x = q4(&ctl);
    assert!(x.town_cain_due && x.cain_portal && x.cain_portal_guid == 0xA001);

    // +0x51 = 1 but the Cain monster is gone: no portal.
    let (mut ctl, _) = control();
    let mut f = marker_fake();
    {
        let x = q4_mut(&mut ctl);
        (x.marker_known, x.marker_guid) = (true, 0x30);
        (x.town_cain, x.town_cain_guid) = (true, 0x16);
    }
    act1::q4::cain_leaves_tristram(&mut ctl, &mut f);
    assert!(f.log.is_empty());
    assert!(!q4(&ctl).cain_portal);

    // No chain 4 record: nothing.
    let (mut ctl, _) = control();
    drop_chain4(&mut ctl);
    let mut f = marker_fake();
    act1::q4::cain_leaves_tristram(&mut ctl, &mut f);
    assert!(f.log.is_empty() && ctl.faults.is_empty());
}

// ------------------------------------------------------------ §7

// Covers: specs/world/quests-act1-rest.md §7, §2.1, §edge-cases-original-bugs r6
#[test]
fn stone_order_message_bytes() {
    // Vector: order [18, 20, 17, 21, 19] → `50 0400 0100 0300 0000 0400
    // 0200 ?? ??`; D2RUST writes 0 in bytes 13–14 (masked when compared).
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    ctl.record_mut(4).unwrap().extra.stone_order = Some([18, 20, 17, 21, 19]);
    let seed = ctl.seed;
    act1::send_stone_order(&mut ctl, &mut f, P1);
    assert_eq!(f.sent, [(P1, hex("50 0400 0100 0300 0000 0400 0200 0000"))]);
    assert_eq!(f.sent[0].1.len(), 15);
    assert_eq!(f.sent[0].1[13..], [0, 0]);
    // The order was already computed: no quest-seed draw.
    assert_eq!(ctl.seed, seed);
    assert!(ctl.faults.is_empty());
}

// Covers: specs/world/quests-act1-rest.md §3 text
#[test]
fn cain_out_of_town_portal() {
    let (mut ctl, _) = control();
    let mut f = marker_fake();
    f.pos.insert(CAIN5_U, (4, 5, RoomIdT(1)));
    // Before the set-up and the portal: the coordinates are Cain's own.
    assert_eq!(
        act1::q4::cain_portal_coords(&mut ctl, &mut f, CAIN5_U),
        Some((4, 5))
    );
    // The set-up takes Cain's position once; a second one fails.
    assert!(act1::q4::cain_portal_setup(&mut ctl, &mut f, CAIN5_U));
    assert_eq!(q4(&ctl).portal_point, (4, 5));
    assert!(!act1::q4::cain_portal_setup(&mut ctl, &mut f, CAIN5_U));
    // Cain walked on: the portal goes at the stored point, mode 1.
    f.pos.insert(CAIN5_U, (7, 8, RoomIdT(1)));
    assert!(act1::q4::cain_spawn_outside_portal(
        &mut ctl, &mut f, CAIN5_U
    ));
    assert_eq!(f.log, ["object 189 4 5 room 1 mode 1"]);
    let x = q4(&ctl);
    assert!(x.out_cain_portal && x.out_cain_portal_guid == 0xA001);
    assert_eq!(
        act1::q4::cain_portal_coords(&mut ctl, &mut f, CAIN5_U),
        Some((4, 5))
    );
    // A second spawn does nothing and still answers 1.
    assert!(act1::q4::cain_spawn_outside_portal(
        &mut ctl, &mut f, CAIN5_U
    ));
    assert_eq!(f.log.len(), 1);

    // The point outside every room: it moves by (+3, +3), no object, 1.
    let (mut ctl, _) = control();
    let mut f = marker_fake();
    f.pos.insert(CAIN5_U, (20, 20, RoomIdT(1)));
    assert!(act1::q4::cain_portal_setup(&mut ctl, &mut f, CAIN5_U));
    assert!(act1::q4::cain_spawn_outside_portal(
        &mut ctl, &mut f, CAIN5_U
    ));
    assert!(f.log.is_empty() && !q4(&ctl).out_cain_portal);
    assert_eq!(q4(&ctl).portal_point, (23, 23));

    // No chain 4 record: set-up fails, no coordinates, the spawn answers 1.
    let (mut ctl, _) = control();
    drop_chain4(&mut ctl);
    let mut f = marker_fake();
    f.pos.insert(CAIN5_U, (4, 5, RoomIdT(1)));
    assert!(!act1::q4::cain_portal_setup(&mut ctl, &mut f, CAIN5_U));
    assert_eq!(
        act1::q4::cain_portal_coords(&mut ctl, &mut f, CAIN5_U),
        None
    );
    assert!(act1::q4::cain_spawn_outside_portal(
        &mut ctl, &mut f, CAIN5_U
    ));
    assert!(f.log.is_empty());
}

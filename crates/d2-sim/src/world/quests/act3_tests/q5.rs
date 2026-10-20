// Spec: specs/world/quests-act3.md §7 (A3Q5 The Blackened Temple tests)
//! A3Q5 (chain 19, slot 21) callback by callback.

use super::*;

const CH: u8 = 19;
const S: u8 = 21;
/// A council member (superunique) and the orb's monster.
const COUNCIL_U: UnitId = UnitId(0x40);
const ORB_MON_U: UnitId = UnitId(0x41);
/// The orb object, the stairs R object.
const ORB_U: UnitId = UnitId(0x50);
const STAIRS_U: UnitId = UnitId(0x51);

fn setup() -> (QuestControl, Fake3, usize) {
    let (ctl, _) = control();
    let i = ctl.find(CH).unwrap();
    (ctl, Fake3::new(), i)
}

fn ev(e: u8, target: Option<UnitId>, player: Option<UnitId>, a: u32, b: u32) -> EventArgs {
    EventArgs {
        event: e,
        target,
        player,
        a,
        b,
    }
}

fn x(ctl: &QuestControl) -> &act3::q5::Extra {
    &ctl.record(CH).unwrap().extra.act3.q5
}

fn xm(ctl: &mut QuestControl) -> &mut act3::q5::Extra {
    &mut ctl.record_mut(CH).unwrap().extra.act3.q5
}

fn setf(f: &mut Fake3, p: UnitId, slot: u8, b: u8) {
    f.p(p).quests.flags[0].set(slot, b);
}

fn add_monster(f: &mut Fake3, u: UnitId, class: u16) {
    f.f.monsters.insert(
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

fn kill(ctl: &mut QuestControl, f: &mut Fake3, victim: UnitId) {
    call(
        ctl,
        f,
        CH,
        ev(event::MONSTER_KILLED, Some(victim), Some(P1), 0, 0),
    );
}

// ------------------------------------------------------------ §7.2, §7.3

// Covers: specs/world/quests-act3.md §7.2, §7.3, §2
#[test]
fn chat_end_status_and_flag_iterate() {
    let (mut ctl, mut f, i) = setup();
    f.add_player(P2, 75);
    setf(&mut f, P2, S, 0);
    ctl.records[i].state = 2;
    ctl.records[i].flags = 9;
    xm(&mut ctl).ormus_started = true;
    xm(&mut ctl).council_seen = true;
    act3::install(&mut ctl, i, event::NPC_DEACTIVATE);
    // Another NPC: nothing.
    call(
        &mut ctl,
        &mut f,
        CH,
        ev(event::NPC_DEACTIVATE, Some(CAIN3_U), Some(P1), 0, 0),
    );
    assert!(x(&ctl).ormus_started);
    assert!(f.f.sent.is_empty());
    call(
        &mut ctl,
        &mut f,
        CH,
        ev(event::NPC_DEACTIVATE, Some(ORMUS_U), Some(P1), 0, 0),
    );
    let r = ctl.record(CH).unwrap();
    assert_eq!((r.status, r.flags), (2, 0));
    assert!(!x(&ctl).ormus_started);
    assert!(!r.has_callback(event::NPC_DEACTIVATE));
    // F sends to P1 only (P2 has 21.0).
    assert_eq!(sent_5d(&f), [(P1, vec![0x5D, CH, 0, 2, 0, 0])]);
    // Flag iterate: P1 gets 21.2 and 21.3; P2 (21.0) nothing.
    assert!(f.flags(P1).get(S, 2) && f.flags(P1).get(S, 3));
    assert!(!f.flags(P2).get(S, 2) && !f.flags(P2).get(S, 3));
    // Pending flag clear: a second chat end does nothing.
    f.f.sent.clear();
    call(
        &mut ctl,
        &mut f,
        CH,
        ev(event::NPC_DEACTIVATE, Some(ORMUS_U), Some(P1), 0, 0),
    );
    assert!(f.f.sent.is_empty());
}

// Covers: specs/world/quests-act3.md §7.3, §1.2
#[test]
fn event0_message_selection() {
    let (mut ctl, mut f, i) = setup();
    // State 0 → index −1: nothing.
    assert!(text(&mut ctl, &mut f, CH, P1, ORMUS_U).is_empty());
    // State 1 → table state 0: Ormus 594 (menu 1 → 0).
    ctl.records[i].state = 1;
    assert_eq!(text(&mut ctl, &mut f, CH, P1, ORMUS_U), [(594, 0)]);
    // State 3 → table state 2.
    ctl.records[i].state = 3;
    assert_eq!(text(&mut ctl, &mut f, CH, P1, ORMUS_U), [(598, 2)]);
    assert_eq!(text(&mut ctl, &mut f, CH, P1, ALKOR_U), [(596, 2)]);
    // State 5 → table state 4.
    ctl.records[i].state = 5;
    assert_eq!(text(&mut ctl, &mut f, CH, P1, NATALYA_U), [(620, 2)]);
    // Intro: nothing.
    ctl.records[i].not_intro = false;
    assert!(text(&mut ctl, &mut f, CH, P1, NATALYA_U).is_empty());
    ctl.records[i].not_intro = true;
    // State > 5: table state 6 only with 21.13.
    ctl.records[i].state = 6;
    assert!(text(&mut ctl, &mut f, CH, P1, CAIN3_U).is_empty());
    setf(&mut f, P1, S, 13);
    assert_eq!(text(&mut ctl, &mut f, CH, P1, CAIN3_U), [(626, 2)]);
    // 21.0 and not listed: nothing; listed: falls through to state > 5.
    setf(&mut f, P1, S, 0);
    assert!(text(&mut ctl, &mut f, CH, P1, CAIN3_U).is_empty());
    ctl.records[i].guids.add(1);
    assert_eq!(text(&mut ctl, &mut f, CH, P1, CAIN3_U), [(626, 2)]);
    // 21.4: table state 5 (Cain 626, menu 1 → 0), even in an intro
    // record; nothing once the orb is smashed.
    setf(&mut f, P1, S, 4);
    ctl.records[i].not_intro = false;
    assert_eq!(text(&mut ctl, &mut f, CH, P1, CAIN3_U), [(626, 0)]);
    xm(&mut ctl).orb_smashed = true;
    assert!(text(&mut ctl, &mut f, CH, P1, CAIN3_U).is_empty());
}

// Covers: specs/world/quests-act3.md §7.3
#[test]
fn active_function() {
    let (mut ctl, mut f, i) = setup();
    let a = |ctl: &QuestControl, f: &mut Fake3, c| act3::active(ctl, f, i, P1, c);
    assert!(!a(&ctl, &mut f, act3::npc::ORMUS));
    ctl.records[i].state = 1;
    assert!(a(&ctl, &mut f, act3::npc::ORMUS));
    assert!(!a(&ctl, &mut f, act3::npc::CAIN3));
    setf(&mut f, P1, S, 4);
    assert!(a(&ctl, &mut f, act3::npc::CAIN3));
    assert!(!a(&ctl, &mut f, act3::npc::ALKOR));
    xm(&mut ctl).orb_smashed = true;
    assert!(!a(&ctl, &mut f, act3::npc::CAIN3));
    // 21.0: never.
    xm(&mut ctl).orb_smashed = false;
    setf(&mut f, P1, S, 0);
    assert!(!a(&ctl, &mut f, act3::npc::ORMUS));
    assert!(!a(&ctl, &mut f, act3::npc::CAIN3));
}

// Covers: specs/world/quests-act3.md §7.3, §7.1
#[test]
fn ormus_594_vector() {
    // Test vector: Lam Esen not done, Ormus 594 → state 3 (table state 2,
    // msgs 596…).
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].state = 1;
    say(&mut ctl, &mut f, P1, ORMUS_U, 594);
    assert_eq!(ctl.records[i].state, 3);
    assert!(x(&ctl).ormus_started);
    // Refreshed: 0x27 then 0x29.
    assert!(f
        .log()
        .iter()
        .any(|l| l.starts_with("0x27 36 ") && l.contains("(598, 2)")));
    assert!(f.f.sent_ids().contains(&0x29));
    assert_eq!(text(&mut ctl, &mut f, CH, P1, ALKOR_U), [(596, 2)]);
    // Lam Esen done (game 17.13): state 2.
    let (mut ctl, mut f, i) = setup();
    ctl.game.set(17, 13);
    say(&mut ctl, &mut f, P1, ORMUS_U, 594);
    assert_eq!(ctl.records[i].state, 2);
    // 21.0: ignored.
    let (mut ctl, mut f, i) = setup();
    setf(&mut f, P1, S, 0);
    say(&mut ctl, &mut f, P1, ORMUS_U, 594);
    assert_eq!(ctl.records[i].state, 0);
    assert!(!x(&ctl).ormus_started);
    // From another NPC: ignored.
    let (mut ctl, mut f, i) = setup();
    say(&mut ctl, &mut f, P1, ALKOR_U, 594);
    assert_eq!(ctl.records[i].state, 0);
}

// Covers: specs/world/quests-act3.md §7.3, §4.10, §1.3
#[test]
fn cain_626_completes() {
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].state = 6;
    setf(&mut f, P1, S, 4);
    setf(&mut f, P1, S, 13);
    setf(&mut f, P1, 18, 0);
    f.p(P1).items = vec![*b"qey ", *b"qf2 ", *b"hp1 "];
    say(&mut ctl, &mut f, P1, CAIN3_U, 626);
    for c in ["qey ", "qhr ", "qbr ", "qf1 ", "qf2 "] {
        assert!(f.log().contains(&format!("delete {c}")));
    }
    assert_eq!(f.p(P1).items, [*b"hp1 "]);
    assert!(ctl.game.get(S, 13));
    let r = ctl.record(CH).unwrap();
    assert_eq!((r.state, r.status), (7, 13));
    assert!(r.guids.contains(1));
    let fl = f.flags(P1);
    assert!(fl.get(S, 0) && !fl.get(S, 4));
    // The own sequence fn: state 7 → seq(20) starts the Guardian.
    let g = ctl.record(20).unwrap();
    assert_eq!(g.state, 1);
    assert!(g.has_callback(event::NPC_DEACTIVATE));
    // Status 13 is silent.
    assert!(sent_5d(&f).is_empty());
    assert!(f.log().iter().any(|l| l.starts_with("0x27 32 ")));
}

// Covers: specs/world/quests-act3.md §7.3
#[test]
fn cain_626_without_goal_or_items() {
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].state = 6;
    // Without 21.4: nothing.
    say(&mut ctl, &mut f, P1, CAIN3_U, 626);
    assert!(!f.flags(P1).get(S, 0));
    // With 21.4 only: no deletes, no game bit, state kept; still 21.0.
    setf(&mut f, P1, S, 4);
    say(&mut ctl, &mut f, P1, CAIN3_U, 626);
    assert!(!f.log().iter().any(|l| l.starts_with("delete")));
    assert!(!ctl.game.get(S, 13));
    assert_eq!(ctl.records[i].state, 6);
    assert!(f.flags(P1).get(S, 0) && !f.flags(P1).get(S, 4));
    // The sequence fn ran: state 6 ≠ 7 and not-intro → 1, chain 20
    // untouched.
    assert_eq!(ctl.record(20).unwrap().state, 0);
    // State 7 already: no status change.
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].state = 7;
    ctl.records[i].status = 4;
    setf(&mut f, P1, S, 4);
    setf(&mut f, P1, S, 13);
    say(&mut ctl, &mut f, P1, CAIN3_U, 626);
    assert_eq!(ctl.records[i].status, 4);
    assert!(!ctl.game.get(S, 13));
}

// ------------------------------------------------------------ §7.4

// Covers: specs/world/quests-act3.md §7.4
#[test]
fn causeway_starts_quest() {
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
    call(
        &mut ctl,
        &mut f,
        CH,
        ev(event::CHANGED_LEVEL, Some(P1), Some(P1), 79, 82),
    );
    assert_eq!(ctl.records[i].state, 1);
    assert!(ctl.records[i].has_callback(event::NPC_DEACTIVATE));
    // Not state 0, or intro: nothing.
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].not_intro = false;
    call(
        &mut ctl,
        &mut f,
        CH,
        ev(event::CHANGED_LEVEL, Some(P1), Some(P1), 79, 82),
    );
    assert_eq!(ctl.records[i].state, 0);
    assert!(!ctl.records[i].has_callback(event::NPC_DEACTIVATE));
    ctl.records[i].not_intro = true;
    ctl.records[i].state = 2;
    call(
        &mut ctl,
        &mut f,
        CH,
        ev(event::CHANGED_LEVEL, Some(P1), Some(P1), 79, 82),
    );
    assert_eq!(ctl.records[i].state, 2);
}

// Covers: specs/world/quests-act3.md §7.4, §7.2
#[test]
fn leaving_docks() {
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].guids.add(1);
    ctl.records[i].guids.add(7);
    ctl.records[i].state = 3;
    ctl.records[i].status = 1;
    xm(&mut ctl).council_seen = true;
    call(
        &mut ctl,
        &mut f,
        CH,
        ev(event::CHANGED_LEVEL, Some(P1), Some(P1), 75, 76),
    );
    let r = ctl.record(CH).unwrap();
    assert_eq!(r.guids.0, [7]);
    // Silent status 2; state S(4, 5) = 5 (Lam Esen not done).
    assert_eq!((r.status, r.state), (2, 5));
    assert!(sent_5d(&f).is_empty());
    // Flag iterate with state 5: only the council bit.
    assert!(!f.flags(P1).get(S, 2) && f.flags(P1).get(S, 3));
    // Lam Esen done, status already 3: state 4, status kept.
    let (mut ctl, mut f, i) = setup();
    ctl.game.set(17, 13);
    ctl.records[i].state = 2;
    ctl.records[i].status = 3;
    call(
        &mut ctl,
        &mut f,
        CH,
        ev(event::CHANGED_LEVEL, Some(P1), Some(P1), 75, 76),
    );
    assert_eq!((ctl.records[i].status, ctl.records[i].state), (3, 4));
    // The player has 21.4: nothing.
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].state = 2;
    setf(&mut f, P1, S, 4);
    call(
        &mut ctl,
        &mut f,
        CH,
        ev(event::CHANGED_LEVEL, Some(P1), Some(P1), 75, 76),
    );
    assert_eq!((ctl.records[i].status, ctl.records[i].state), (0, 2));
}

// Covers: specs/world/quests-act3.md §7.4, §edge-cases-original-bugs r13
#[test]
fn game_start() {
    // 18.0 → the orb counts as smashed: the Durance opens (edge case 13).
    let (mut ctl, mut f, i) = setup();
    setf(&mut f, P1, 18, 0);
    assert!(!act3::durance_open(&ctl, 83));
    call(
        &mut ctl,
        &mut f,
        CH,
        ev(event::PLAYER_STARTED_GAME, Some(P1), Some(P1), 0, 0),
    );
    assert!(x(&ctl).orb_smashed);
    assert!(act3::durance_open(&ctl, 83));
    assert!(ctl.records[i].not_intro);
    // 21.0 → game 21.13, intro.
    let (mut ctl, mut f, i) = setup();
    setf(&mut f, P1, S, 0);
    call(
        &mut ctl,
        &mut f,
        CH,
        ev(event::PLAYER_STARTED_GAME, Some(P1), Some(P1), 0, 0),
    );
    assert!(ctl.game.get(S, 13) && !ctl.records[i].not_intro);
    // 21.4 likewise.
    let (mut ctl, mut f, i) = setup();
    setf(&mut f, P1, S, 4);
    call(
        &mut ctl,
        &mut f,
        CH,
        ev(event::PLAYER_STARTED_GAME, Some(P1), Some(P1), 0, 0),
    );
    assert!(ctl.game.get(S, 13) && !ctl.records[i].not_intro);
    // 21.3 with 17.0 → status 3, state 4.
    let (mut ctl, mut f, i) = setup();
    setf(&mut f, P1, S, 3);
    setf(&mut f, P1, S, 2);
    setf(&mut f, P1, 17, 0);
    call(
        &mut ctl,
        &mut f,
        CH,
        ev(event::PLAYER_STARTED_GAME, Some(P1), Some(P1), 0, 0),
    );
    assert!(x(&ctl).had_lam);
    assert_eq!((ctl.records[i].status, ctl.records[i].state), (3, 4));
    // 21.3 without 17.0 → state 5; 21.2 → status 2, state 2 / 3.
    for (bits, lam, want) in [
        (&[3u8][..], false, (3, 5)),
        (&[2][..], true, (2, 2)),
        (&[2][..], false, (2, 3)),
        (&[][..], true, (0, 0)),
    ] {
        let (mut ctl, mut f, i) = setup();
        for &b in bits {
            setf(&mut f, P1, S, b);
        }
        if lam {
            setf(&mut f, P1, 17, 0);
        }
        call(
            &mut ctl,
            &mut f,
            CH,
            ev(event::PLAYER_STARTED_GAME, Some(P1), Some(P1), 0, 0),
        );
        assert_eq!(x(&ctl).had_lam, lam);
        assert_eq!((ctl.records[i].status, ctl.records[i].state), want);
        assert!(f.f.sent.is_empty());
    }
}

// Covers: specs/world/quests-act3.md §7.4
#[test]
fn leave_game_removes_guid() {
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].guids.add(1);
    ctl.records[i].guids.add(5);
    call(
        &mut ctl,
        &mut f,
        CH,
        ev(event::PLAYER_LEAVES_GAME, Some(P1), Some(P1), 0, 0),
    );
    assert_eq!(ctl.records[i].guids.0, [5]);
}

// ------------------------------------------------------------ §7.5

// Covers: specs/world/quests-act3.md §7.5, §10
#[test]
fn council_registration() {
    let (mut ctl, mut f, i) = setup();
    assert_eq!(act3::superunique_link(27), Some(CH));
    ctl.records[i].state = 3;
    ctl.records[i].status = 2;
    act3::council_preset(&mut ctl, &mut f, UnitId(0x60));
    let e = x(&ctl);
    assert!(e.council_seen);
    assert_eq!(
        (e.council.clone(), e.registered, e.left),
        (vec![0x60], 1, 1)
    );
    let r = ctl.record(CH).unwrap();
    assert_eq!((r.status, r.state), (3, 5));
    assert_eq!(sent_5d(&f), [(P1, vec![0x5D, CH, 0, 3, 0, 0])]);
    assert!(f.flags(P1).get(S, 3));
    // A duplicate: no new entry; status 3 with state ≥ 2: nothing sent.
    f.f.sent.clear();
    act3::council_preset(&mut ctl, &mut f, UnitId(0x60));
    assert_eq!(x(&ctl).registered, 1);
    assert!(f.f.sent.is_empty());
    // Up to six.
    for u in 0x61..0x68 {
        act3::council_preset(&mut ctl, &mut f, UnitId(u));
    }
    let e = x(&ctl);
    assert_eq!(e.council, [0x60, 0x61, 0x62, 0x63, 0x64, 0x65]);
    assert_eq!((e.registered, e.left), (6, 6));
    // State < 2 with status 1: nothing.
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].state = 1;
    ctl.records[i].status = 1;
    act3::council_preset(&mut ctl, &mut f, UnitId(0x60));
    assert_eq!((ctl.records[i].status, ctl.records[i].state), (1, 1));
    assert_eq!(x(&ctl).registered, 1);
    // Lam Esen done: state 4.
    let (mut ctl, mut f, i) = setup();
    ctl.game.set(17, 13);
    act3::council_preset(&mut ctl, &mut f, UnitId(0x60));
    assert_eq!((ctl.records[i].status, ctl.records[i].state), (3, 4));
    // Intro: nothing at all.
    let (mut ctl, mut f, i) = setup();
    ctl.records[i].not_intro = false;
    act3::council_preset(&mut ctl, &mut f, UnitId(0x60));
    assert_eq!(*x(&ctl), act3::q5::Extra::default());
}

// ------------------------------------------------------------ §7.6

// Covers: specs/world/quests-act3.md §7.6 text, §10
#[test]
fn orb_monster_kill_only_flags() {
    let (mut ctl, mut f, _) = setup();
    assert_eq!(act3::monster_link(366, 366), Some(CH));
    add_monster(&mut f, ORB_MON_U, act3::npc::COMPELLING_ORB);
    xm(&mut ctl).left = 1;
    kill(&mut ctl, &mut f, ORB_MON_U);
    assert_eq!(f.log(), ["flags 65 0x20000"]);
    assert_eq!(x(&ctl).left, 1);
}

// Covers: specs/world/quests-act3.md §7.6 r1, §7.6 r2
#[test]
fn flail_then_cube() {
    let (mut ctl, mut f, _) = setup();
    add_monster(&mut f, COUNCIL_U, 700);
    f.add_player(P2, 80);
    f.add_player(UnitId(3), 80);
    // P2 holds the flail; player 3 has 18.0 and a cube; player 4 is in
    // Act I.
    f.p(P2).items.push(*b"qf1 ");
    setf(&mut f, UnitId(3), 18, 0);
    f.p(UnitId(3)).items.push(*b"box ");
    f.add_player(UnitId(4), 80);
    f.p(UnitId(4)).act = Some(0);
    f.drops = vec![false, true];
    kill(&mut ctl, &mut f, COUNCIL_U);
    let e = x(&ctl);
    assert_eq!((e.flails_to_drop, e.cubes_to_drop), (1, 2));
    // One flail asked (P1), created only on the second try.
    assert_eq!(f.log(), ["qdrop 64 qf1  7 false"]);
    assert!(!x(&ctl).flail_dropped);
    assert_eq!(ctl.record(16).unwrap().extra.act3.q2.flails, 0);
    // Edge case 14: the flail drops again on the next kill.
    kill(&mut ctl, &mut f, COUNCIL_U);
    assert!(x(&ctl).flail_dropped);
    let q2 = &ctl.record(16).unwrap().extra.act3.q2;
    assert_eq!((q2.flails, q2.flail_dropped), (1, true));
    // Then the cube: one per player without one (recounted).
    f.f.log.clear();
    kill(&mut ctl, &mut f, COUNCIL_U);
    assert!(x(&ctl).cube_dropped);
    assert_eq!(f.log(), ["qdrop 64 box  2 false", "qdrop 64 box  2 false"]);
    // Both dropped: no recount, no drops.
    f.f.log.clear();
    f.p(P1).items.push(*b"box ");
    kill(&mut ctl, &mut f, COUNCIL_U);
    assert_eq!(x(&ctl).cubes_to_drop, 2);
    assert!(f.log().is_empty());
}

// Covers: specs/world/quests-act3.md §edge-cases-original-bugs r14, §7.6 r2
#[test]
fn no_cube_while_everyone_has_a_flail() {
    let (mut ctl, mut f, _) = setup();
    add_monster(&mut f, COUNCIL_U, 700);
    f.add_player(P2, 80);
    f.p(P1).items.push(*b"qf2 ");
    f.p(P2).items.push(*b"qf1 ");
    for _ in 0..3 {
        kill(&mut ctl, &mut f, COUNCIL_U);
    }
    let e = x(&ctl);
    assert_eq!((e.flails_to_drop, e.cubes_to_drop), (0, 2));
    assert!(!e.flail_dropped && !e.cube_dropped);
    assert!(f.log().is_empty());
}

// Covers: specs/world/quests-act3.md §7.6 r3, §1.1
#[test]
fn last_council_kill() {
    let (mut ctl, mut f, i) = setup();
    add_monster(&mut f, COUNCIL_U, 700);
    // Nothing to drop: P1 holds a flail and a cube.
    f.p(P1).items = vec![*b"qf1 ", *b"box "];
    xm(&mut ctl).flail_dropped = true;
    xm(&mut ctl).cube_dropped = true;
    xm(&mut ctl).left = 2;
    ctl.records[i].state = 5;
    ctl.records[i].status = 3;
    // P2 near (in Act III), P3 near with 18.0, P4 P2's party member, P5 a
    // party member in Act I, P6 far away.
    let (p3, p4, p5, p6) = (UnitId(3), UnitId(4), UnitId(5), UnitId(6));
    for p in [P2, p3, p4, p5, p6] {
        f.add_player(p, 80);
    }
    f.p(p5).act = Some(0);
    setf(&mut f, p3, 18, 0);
    f.f.near = vec![P2, p3, p5];
    f.f.party.insert(P2, vec![P2, p4, p5]);
    kill(&mut ctl, &mut f, COUNCIL_U);
    assert_eq!(x(&ctl).left, 1);
    assert!(f.f.sent.is_empty());
    kill(&mut ctl, &mut f, COUNCIL_U);
    assert_eq!(x(&ctl).left, 0);
    assert_eq!(x(&ctl).last_council, 0x40);
    let r = ctl.record(CH).unwrap();
    assert_eq!((r.state, r.status), (6, 4));
    assert!(ctl.game.get(S, 13));
    let fl = |f: &Fake3, p| f.flags(p);
    // Near: P2 21.4 + 21.13; P3 21.0 + 21.13 (18.0); P5 is in Act I.
    assert!(fl(&f, P2).get(S, 4) && fl(&f, P2).get(S, 13) && !fl(&f, P2).get(S, 0));
    assert!(fl(&f, p3).get(S, 0) && fl(&f, p3).get(S, 13) && !fl(&f, p3).get(S, 4));
    // Party of P2: P4 credited; P5 (Act I) not.
    assert!(fl(&f, p4).get(S, 4) && fl(&f, p4).get(S, 13));
    assert!(!fl(&f, p5).get(S, 4) && !fl(&f, p5).get(S, 13));
    // Completion flag: players lacking 21.0 and 21.4 (P1, P5, P6).
    for p in [P1, p5, p6] {
        assert!(fl(&f, p).get(S, 14));
    }
    for p in [P2, p3, p4] {
        assert!(!fl(&f, p).get(S, 14));
    }
    // Status 4 to all: the default status rule (init_no 6) reports 12
    // in state 6 to players without 21.13, and nobody has it yet; P5's
    // room is not in Act III. Then the completion messages.
    let s: Vec<(UnitId, u8)> = sent_5d(&f).into_iter().map(|m| (m.0, m.1[3])).collect();
    assert_eq!(
        s,
        [
            (P1, 12),
            (P2, 12),
            (p3, 12),
            (p4, 12),
            (p6, 12),
            (P1, 12),
            (p5, 12),
            (p6, 12)
        ]
    );
    assert!(sent_5d(&f).iter().all(|m| m.1[2] == 0 && m.1[1] == CH));
    // Sound 64 to the 21.13 holders.
    let sounds: Vec<String> = f
        .log()
        .into_iter()
        .filter(|l| l.starts_with("sound"))
        .collect();
    assert_eq!(sounds, ["sound 2 64", "sound 3 64", "sound 4 64"]);
    // A further kill: count stays 0.
    kill(&mut ctl, &mut f, COUNCIL_U);
    assert_eq!(x(&ctl).left, 0);
}

// Covers: specs/world/quests-act3.md §7.6 r3
#[test]
fn council_kill_orb_smashed_and_intro() {
    let (mut ctl, mut f, i) = setup();
    add_monster(&mut f, COUNCIL_U, 700);
    f.p(P1).items = vec![*b"qf1 ", *b"box "];
    xm(&mut ctl).left = 1;
    xm(&mut ctl).orb_smashed = true;
    kill(&mut ctl, &mut f, COUNCIL_U);
    assert_eq!(ctl.records[i].state, 7);
    // Intro: the count is untouched.
    let (mut ctl, mut f, i) = setup();
    add_monster(&mut f, COUNCIL_U, 700);
    f.p(P1).items = vec![*b"qf1 ", *b"box "];
    xm(&mut ctl).left = 1;
    ctl.records[i].not_intro = false;
    kill(&mut ctl, &mut f, COUNCIL_U);
    assert_eq!(x(&ctl).left, 1);
    assert!(!ctl.game.get(S, 13));
}

// ------------------------------------------------------------ §7.7

// Covers: specs/world/quests-act3.md §7.7
#[test]
fn orb_init_spawns_monster() {
    let (mut ctl, mut f, _) = setup();
    f.f.spawns = vec![None, Some(ORB_MON_U)];
    add_monster(&mut f, ORB_MON_U, act3::npc::COMPELLING_ORB);
    // Spawn fails: tried again at the next init.
    act3::orb_init(&mut ctl, &mut f, ORB_U);
    assert!(!x(&ctl).orb_spawned);
    act3::orb_init(&mut ctl, &mut f, ORB_U);
    assert_eq!(
        f.log(),
        [
            "spawn at 80 366 mode 1",
            "spawn at 80 366 mode 1",
            "flags 65 0x20000"
        ]
    );
    assert!(x(&ctl).orb_spawned);
    assert_eq!(x(&ctl).orb_guid, 0x41);
    // Spawned already; smashed → mode 2.
    f.f.log.clear();
    xm(&mut ctl).orb_smashed = true;
    act3::orb_init(&mut ctl, &mut f, ORB_U);
    assert_eq!(f.log(), ["mode 80 2"]);
}

// Covers: specs/world/quests-act3.md §7.7, §4.10, §edge-cases-original-bugs r12
#[test]
fn orb_operate_two_hits() {
    let (mut ctl, mut f, i) = setup();
    f.f.objects.insert(ORB_U, (0x50, 404, 0));
    add_monster(&mut f, ORB_MON_U, act3::npc::COMPELLING_ORB);
    xm(&mut ctl).orb_guid = 0x41;
    f.f.frame = 100;
    // No `qf2 ` in hand: sound 19.
    act3::orb_operate(&mut ctl, &mut f, ORB_U, P1);
    assert_eq!(f.log(), ["sound 1 19"]);
    assert_eq!(x(&ctl).hits, 0);
    f.weapons.insert(P1, *b"qf2 ");
    f.p(P1).items = vec![*b"qf2 "];
    setf(&mut f, P1, S, 4);
    // The first valid hit does nothing.
    f.f.log.clear();
    act3::orb_operate(&mut ctl, &mut f, ORB_U, P1);
    assert_eq!(x(&ctl).hits, 1);
    assert!(f.log().is_empty() && f.f.sent.is_empty());
    // The second smashes it. The orb kill (`quests-helpers.md` §4.3):
    // mode 0 at (0, 0), then missile 368 from the orb to the first
    // class-386 object of its room list, data +0x28 := its GUID, its
    // room refreshed.
    let stairs = UnitId(0x81);
    f.f.objects.insert(stairs, (0x81, 386, 0));
    f.f.pos.insert(ORB_MON_U, (10, 20, RoomId(4)));
    f.f.pos.insert(stairs, (12, 25, RoomId(4)));
    f.f.a5_adjacent.insert(RoomId(4), vec![ORB_MON_U, stairs]);
    ctl.records[i].state = 7;
    act3::orb_operate(&mut ctl, &mut f, ORB_U, P1);
    let fl = f.flags(P1);
    assert!(fl.get(18, 0) && fl.get(18, 13) && fl.get(S, 0));
    assert!(f.p(P1).items.is_empty());
    let log = f.log();
    assert_eq!(
        log[..7],
        [
            "delete qf2 ",
            "mode request 65 0 0 0",
            "spawn missile 368 owner 65 origin Some(65) flags 0x420 at 0 0 target 12 25 skill 0 level 1",
            "missile guid 36865 0x81",
            "refresh 129",
            "mode 80 1",
            "event1 80 116"
        ]
    );
    assert!(ctl.faults.is_empty());
    assert!(x(&ctl).orb_smashed);
    assert_eq!(ctl.fx, 10);
    assert!(f.f.sent.iter().any(|m| m.1 == [0x89, 10]));
    // No 0x28 beyond the FX's own, no 0x5D.
    assert!(sent_5d(&f).is_empty());
    // Chain 19's seq fn (state 7) → the Guardian starts.
    assert_eq!(ctl.record(20).unwrap().state, 1);
    // Object now in mode 1: no further operate.
    f.f.log.clear();
    act3::orb_operate(&mut ctl, &mut f, ORB_U, P1);
    assert!(f.log().is_empty());
}

// Covers: specs/world/quests-act3.md §7.7
#[test]
fn orb_operate_party() {
    let (mut ctl, mut f, _) = setup();
    f.f.objects.insert(ORB_U, (0x50, 404, 0));
    f.weapons.insert(P1, *b"qf2 ");
    xm(&mut ctl).hits = 1;
    let (p3, p4, p5) = (UnitId(3), UnitId(4), UnitId(5));
    for p in [P2, p3, p4, p5] {
        f.add_player(p, 80);
        f.p(p).items = vec![*b"qey ", *b"qf1 "];
    }
    // P2 has 18.0; P3 in Act III with 21.4; P4 trading; P5 in Act I.
    setf(&mut f, P2, 18, 0);
    setf(&mut f, p3, S, 4);
    f.trading.insert(p4);
    f.p(p5).act = Some(0);
    f.f.party.insert(P1, vec![P2, p3, p4, p5]);
    act3::orb_operate(&mut ctl, &mut f, ORB_U, P1);
    // No orb monster known: nothing killed.
    assert!(!f.log().iter().any(|l| l.starts_with("kill")));
    assert!(f.p(P2).items.is_empty());
    assert!(f.p(p3).items.is_empty());
    let f3 = f.flags(p3);
    assert!(f3.get(18, 0) && f3.get(18, 13) && f3.get(S, 0));
    let f4 = f.flags(p4);
    assert!(f4.get(18, 0) && f4.get(18, 13) && !f4.get(S, 0));
    assert_eq!(f.p(p4).items.len(), 2);
    assert!(!f.flags(p5).get(18, 0));
    assert_eq!(f.p(p5).items.len(), 2);
    // The operator lacked 21.4: no 21.0.
    assert!(!f.flags(P1).get(S, 0));
}

// Covers: specs/world/quests-act3.md §7.7, §1.4
#[test]
fn stairs_r_and_durance_check() {
    let (mut ctl, mut f, _) = setup();
    act3::stairs_r_init(&mut ctl, &mut f, STAIRS_U);
    assert!(f.log().is_empty());
    // Closed until the orb is smashed, except from Durance of Hate 2.
    assert!(!act3::durance_open(&ctl, 83));
    assert!(act3::durance_open(&ctl, 101));
    assert_eq!(warp_check(83, 100), WarpCheck::Delegate(0x005B_BFA0));
    xm(&mut ctl).orb_smashed = true;
    assert!(act3::durance_open(&ctl, 83));
    act3::stairs_r_init(&mut ctl, &mut f, STAIRS_U);
    assert_eq!(f.log(), ["mode 81 2"]);
}

// Covers: specs/world/quests-act3.md §4.10
#[test]
fn khalims_will_completes_by_the_durance_warp() {
    // The third way (`quests.md` §8.1): the Durance warp sets 18.0 and
    // 18.13 and deletes the five Khalim items; the orb smash and Cain's
    // 626 are `orb_operate_two_hits` and `cain_626_completes`.
    let (mut ctl, _) = control();
    let mut f = crate::world::quests::tests::Fake::new();
    ctl.object_warp(&mut f, P1, 102);
    let fl = f.flags(P1);
    assert!(fl.get(18, 0) && fl.get(18, 13));
    for c in ["qey ", "qhr ", "qbr ", "qf1 ", "qf2 "] {
        assert!(f.log.contains(&format!("delete {c}")), "{c}: {:?}", f.log);
    }
}

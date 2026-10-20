// Spec: specs/world/quests-act5-2.md §9 (Test vectors)
//! The Act V intro record (chain 40) on the quests' fake world.

use super::super::super::tests::*;
use super::super::super::*;

const DREHYA_U: UnitId = UnitId(0x40);
const MALAH_U: UnitId = UnitId(0x41);
const NIHL_U: UnitId = UnitId(0x42);
const QUAL_U: UnitId = UnitId(0x43);
const CAIN_U: UnitId = UnitId(0x44);
const LARZUK_U: UnitId = UnitId(0x45);

fn fake() -> Fake {
    let mut f = Fake::new();
    f.p(P1).act = Some(4);
    f.p(P1).level = Some(109);
    for (u, c) in [
        (DREHYA_U, 512),
        (MALAH_U, 513),
        (NIHL_U, 514),
        (QUAL_U, 515),
        (CAIN_U, 520),
        (LARZUK_U, 511),
    ] {
        let kind = UnitKind::Monster {
            class: u32::from(c),
            superunique: None,
            owner: None,
        };
        f.monsters.insert(u, (u.0, c, kind));
    }
    f
}

/// NPC chat start through the dispatch (Act V records only answer).
fn text(ctl: &mut QuestControl, f: &mut Fake, n: UnitId) -> Vec<u16> {
    let mut list = TextList::new();
    let i = ctl.find(40).unwrap();
    let args = EventArgs {
        event: event::NPC_ACTIVATE,
        target: Some(n),
        player: Some(P1),
        ..EventArgs::default()
    };
    assert!(super::callback(ctl, f, i, args, Some(&mut list)));
    list.iter().map(|e| e.0).collect()
}

/// C→S 0x31 to the NPC unit (GUID = unit id in the fake).
fn say(ctl: &mut QuestControl, f: &mut Fake, n: UnitId, msg: u16) {
    let mut m = vec![0x31];
    m.extend_from_slice(&n.0.to_le_bytes());
    m.extend_from_slice(&msg.to_le_bytes());
    m.extend_from_slice(&[0, 0]);
    assert_eq!(ctl.quest_message(f, P1, &m), 0);
}

fn heard(f: &Fake, class: u16) -> bool {
    f.players[&P1]
        .quests
        .heard(usize::from(f.difficulty), class)
}

// Covers: specs/world/quests-act5-2.md §9
#[test]
fn record_init() {
    let (ctl, _) = control();
    let r = ctl.record(40).unwrap();
    assert!(r.active && !r.not_intro);
    assert_eq!((r.state, r.status, r.filter), (0, 0, 42));
    assert!(r.has_callback(event::NPC_ACTIVATE) && r.has_callback(event::SCROLL_MESSAGE));
    assert!(!r.has_callback(event::NPC_DEACTIVATE));
    assert_eq!(r.status_fn, Some(0x0058_6C40));
    assert_eq!(r.active_fn, Some(0x0058_6C50));
    assert_eq!(r.msgs, Some(0x0073_2FF8));
    // The table: 3 states × 5 NPCs, every menu 0.
    assert_eq!(super::lines(1, 514), [(20054, 0)]);
    assert!(super::lines(3, 514).is_empty());
    assert!(super::lines(0, 511).is_empty());
}

// Covers: specs/world/quests-act5-2.md §9
#[test]
fn event0_by_player_class() {
    // Vector: nihlathak, assassin, intro bit clear → table state 1
    // (20054).
    let (mut ctl, _) = control();
    let mut f = fake();
    f.p(P1).class = 6;
    assert_eq!(text(&mut ctl, &mut f, NIHL_U), [20054]);
    // The rest of the class table.
    for (class, npc, want) in [
        (2, NIHL_U, 20055),
        (0, NIHL_U, 20053),
        (4, MALAH_U, 20039),
        (1, MALAH_U, 20038),
        (3, MALAH_U, 20037),
        (5, QUAL_U, 20067),
        (3, QUAL_U, 20066),
        (6, QUAL_U, 20065),
        (4, DREHYA_U, 20014),
        (1, CAIN_U, 20003),
    ] {
        let mut f = fake();
        f.p(P1).class = class;
        assert_eq!(text(&mut ctl, &mut f, npc), [want], "class {class}");
    }
    // Other NPCs: nothing; intro bit set: nothing.
    assert!(text(&mut ctl, &mut f, LARZUK_U).is_empty());
    f.p(P1).quests.hear(0, 514);
    assert!(text(&mut ctl, &mut f, NIHL_U).is_empty());
    // Reached through the dispatch for a player in Act V.
    let mut list = TextList::new();
    let mut f = fake();
    ctl.npc_activate(&mut f, P1, CAIN_U, &mut list);
    assert!(list.contains(&(20003, 0)));
}

// Covers: specs/world/quests-act5-2.md §9
#[test]
fn malah_starts_the_siege() {
    // Vector: malah 20038 with chain 31 not-intro, state 0 → malah intro
    // bit; chain 31 state 1.
    let (mut ctl, _) = control();
    let mut f = fake();
    say(&mut ctl, &mut f, MALAH_U, 20038);
    assert!(heard(&f, 513));
    assert_eq!(ctl.record(31).unwrap().state, 1);
    // Chain 31 past state 0, or intro: the bit only.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(31).unwrap().not_intro = false;
    say(&mut ctl, &mut f, MALAH_U, 20037);
    assert!(heard(&f, 513));
    assert_eq!(ctl.record(31).unwrap().state, 0);
    // A message not in Malah's range: nothing.
    let (mut ctl, _) = control();
    let mut f = fake();
    say(&mut ctl, &mut f, MALAH_U, 20040);
    assert!(!heard(&f, 513));
    assert_eq!(ctl.record(31).unwrap().state, 0);
}

// Covers: specs/world/quests-act5-2.md §9
#[test]
fn event11_sets_intro_bits() {
    let (mut ctl, _) = control();
    let mut f = fake();
    for (n, class, msg) in [
        (DREHYA_U, 512, 20014),
        (CAIN_U, 520, 20003),
        (NIHL_U, 514, 20055),
        (QUAL_U, 515, 20065),
    ] {
        assert!(!heard(&f, class));
        // Another NPC's line does not count.
        say(&mut ctl, &mut f, LARZUK_U, msg);
        assert!(!heard(&f, class));
        say(&mut ctl, &mut f, n, msg);
        assert!(heard(&f, class), "{class}");
    }
    assert!(!heard(&f, 511));
    // Wrong message for the NPC: nothing.
    let (mut ctl, _) = control();
    let mut f = fake();
    say(&mut ctl, &mut f, DREHYA_U, 20015);
    assert!(!heard(&f, 512));
}

// Covers: specs/world/quests-act5-2.md §9
#[test]
fn active_and_status_functions() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = ctl.find(40).unwrap();
    assert!(super::active(&ctl, &mut f, i, P1, 513, 0));
    assert!(!super::active(&ctl, &mut f, i, P1, 514, 0));
    f.p(P1).quests.hear(0, 513);
    assert!(!super::active(&ctl, &mut f, i, P1, 513, 0));
    let pf = f.flags(P1);
    assert_eq!(super::status(&ctl, &mut f, i, P1, &pf, 0x0058_6C40), None);
    assert!(f.log.is_empty());
    // 0x8A for Malah through the record's active function.
    let mut f = fake();
    ctl.picked = true;
    ctl.npc_wants_interact(&mut f, P1, MALAH_U, 513, true)
        .unwrap();
    assert_eq!(f.sent, [(P1, hex("8A 01 41000000"))]);
}

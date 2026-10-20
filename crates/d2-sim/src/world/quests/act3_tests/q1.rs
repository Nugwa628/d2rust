// Spec: specs/world/quests-act3.md §3 (A3Q1 Lam Esen's Tome, chain 15)
//! Chain 15 callback by callback, the status and active functions and
//! the tome object.

use super::*;

const C: u8 = 15;
const S: u8 = 17;
const P3: UnitId = UnitId(3);
const TOME_U: UnitId = UnitId(0x60);
const ITEM_U: UnitId = UnitId(0x300);

fn setf(f: &mut Fake3, u: UnitId, slot: u8, b: u8) {
    f.p(u).quests.flags[0].set(slot, b);
}

fn idx(ctl: &QuestControl) -> usize {
    ctl.find(C).unwrap()
}

fn x(ctl: &QuestControl) -> &act3::q1::Extra {
    &ctl.record(C).unwrap().extra.act3.q1
}

fn ev(e: u8, p: UnitId, target: Option<UnitId>, a: u32, b: u32) -> EventArgs {
    EventArgs {
        event: e,
        target,
        player: Some(p),
        a,
        b,
    }
}

fn alkor(p: UnitId, msg: u32) -> EventArgs {
    ev(
        event::SCROLL_MESSAGE,
        p,
        Some(ALKOR_U),
        u32::from(act3::npc::ALKOR),
        msg,
    )
}

fn status_of(ctl: &QuestControl, f: &mut Fake3, p: UnitId) -> u8 {
    let pf = f.flags(p);
    act3::status(ctl, f, idx(ctl), p, &pf).unwrap()
}

fn active(ctl: &QuestControl, f: &mut Fake3, p: UnitId, class: u16) -> bool {
    act3::active(ctl, f, idx(ctl), p, class)
}

fn no_unhandled(f: &Fake3) {
    assert!(
        !f.log().iter().any(|l| l.starts_with("unhandled 15 ")),
        "{:?}",
        f.log()
    );
}

// Covers: specs/world/quests-act3.md §3.2, §1.2
#[test]
fn chat_by_state() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    // State 0: index −1, nothing.
    assert!(text(&mut ctl, &mut f, C, P1, ALKOR_U).is_empty());
    // State 1 → table state 0 (549, menu 1 → 0); only Alkor has a line.
    ctl.records[i].state = 1;
    assert_eq!(text(&mut ctl, &mut f, C, P1, ALKOR_U), vec![(549, 0)]);
    assert!(text(&mut ctl, &mut f, C, P1, ORMUS_U).is_empty());
    // State 2 → 1, state 3 → 2.
    ctl.records[i].state = 2;
    assert_eq!(text(&mut ctl, &mut f, C, P1, ALKOR_U), vec![(550, 2)]);
    ctl.records[i].state = 3;
    assert_eq!(text(&mut ctl, &mut f, C, P1, CAIN3_U), vec![(562, 2)]);
    // State 4 or 5 without the tome: past the table, nothing.
    ctl.records[i].state = 4;
    assert!(text(&mut ctl, &mut f, C, P1, ALKOR_U).is_empty());
    // Intro: nothing.
    ctl.records[i].state = 1;
    ctl.records[i].not_intro = false;
    assert!(text(&mut ctl, &mut f, C, P1, ALKOR_U).is_empty());
    no_unhandled(&f);
}

// Covers: specs/world/quests-act3.md §3.2
#[test]
fn chat_tome_and_done() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    // Holding the tome → table state 3 at any NPC, even in intro state.
    f.p(P1).items.push(*b"bbb ");
    ctl.records[i].not_intro = false;
    assert_eq!(text(&mut ctl, &mut f, C, P1, ALKOR_U), vec![(564, 0)]);
    assert_eq!(text(&mut ctl, &mut f, C, P1, NATALYA_U), vec![(570, 2)]);
    // 17.0 set, 17.13 clear → nothing even with the tome.
    setf(&mut f, P1, S, 0);
    assert!(text(&mut ctl, &mut f, C, P1, ALKOR_U).is_empty());
    // 17.0 and 17.13, no tome: table state 4 when GUID listed.
    f.p(P1).items.clear();
    setf(&mut f, P1, S, 13);
    assert!(text(&mut ctl, &mut f, C, P1, CAIN3_U).is_empty());
    ctl.records[i].guids.add(1);
    assert_eq!(text(&mut ctl, &mut f, C, P1, CAIN3_U), vec![(569, 2)]);
    assert_eq!(text(&mut ctl, &mut f, C, P1, NATALYA_U), vec![(570, 2)]);
    assert!(text(&mut ctl, &mut f, C, P1, ALKOR_U).is_empty());
}

// Covers: specs/world/quests-act3.md §3.2
#[test]
fn wants_to_talk() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    let alk = act3::npc::ALKOR;
    assert!(!active(&ctl, &mut f, P1, alk));
    ctl.records[i].state = 1;
    assert!(active(&ctl, &mut f, P1, alk));
    assert!(!active(&ctl, &mut f, P1, act3::npc::ORMUS));
    setf(&mut f, P1, S, 15);
    assert!(!active(&ctl, &mut f, P1, alk));
    f.p(P1).items.push(*b"bbb ");
    assert!(active(&ctl, &mut f, P1, alk));
    assert!(!active(&ctl, &mut f, P1, act3::npc::CAIN3));
    f.p(P1).items.clear();
    ctl.records[i].state = 2;
    let mut g = Fake3::new();
    assert!(!active(&ctl, &mut g, P1, alk));
    setf(&mut g, P1, S, 0);
    ctl.records[i].state = 1;
    assert!(!active(&ctl, &mut g, P1, alk));
}

// Covers: specs/world/quests-act3.md §3.3
#[test]
fn alkor_549_starts() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    f.add_player(P2, 76);
    setf(&mut f, P2, S, 1);
    call(&mut ctl, &mut f, C, alkor(P1, 549));
    let r = ctl.record(C).unwrap();
    assert_eq!((r.state, r.status, r.flags), (2, 1, 0));
    // Status 1 to all: F sends to both (neither has 17.0 / 17.15).
    let s = sent_5d(&f);
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].0, P1);
    assert_eq!(s[1].0, P2);
    // The flag iterate: P1 gets 17.2; P2 (17.1) does not.
    assert!(f.flags(P1).get(S, 2));
    assert!(!f.flags(P2).get(S, 2));
    // Refresh: 0x27 for Alkor.
    assert!(f.log().iter().any(|l| l.starts_with("0x27 35")));
    no_unhandled(&f);

    // Other messages, other NPCs, and players with 17.0: nothing.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    call(&mut ctl, &mut f, C, alkor(P1, 550));
    let mut a = alkor(P1, 549);
    a.a = u32::from(act3::npc::ORMUS);
    call(&mut ctl, &mut f, C, a);
    setf(&mut f, P1, S, 0);
    call(&mut ctl, &mut f, C, alkor(P1, 549));
    assert_eq!(ctl.record(C).unwrap().state, 0);
    assert!(f.f.sent.is_empty());
}

// Covers: specs/world/quests-act3.md §3.3, §edge-cases-original-bugs r1
#[test]
fn alkor_564_vector() {
    // Test vector: A (sender, Act III, holding the tome), B in Act III,
    // C in Act I; none has 17.0 / 17.1; +0x00 = 1.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    f.add_player(P2, 80);
    f.add_player(P3, 1);
    f.p(P3).act = Some(0);
    f.p(P1).items.push(*b"bbb ");
    let i = idx(&ctl);
    ctl.records[i].state = 4;
    assert!(!ctl.records[idx(&ctl)].has_callback(event::NPC_DEACTIVATE));
    call(&mut ctl, &mut f, C, alkor(P1, 564));
    for p in [P1, P2] {
        let fl = f.flags(p);
        assert!(fl.get(S, 13) && fl.get(S, 0), "{p:?}");
        assert!(!fl.get(S, 1) && !fl.get(S, 14));
        assert_eq!(f.f.players[&p].stats.get(&4), Some(&5));
        assert!(f.f.sent.contains(&(p, vec![0x5D, 0x0F, 2, 0, 0, 0])));
        assert!(f.f.sent.iter().any(|m| m.0 == p && m.1[0] == 0x28));
    }
    let fc = f.flags(P3);
    assert!(fc.get(S, 14));
    assert!(!fc.get(S, 0) && !fc.get(S, 1) && !fc.get(S, 13));
    assert_eq!(f.f.players[&P3].stats.get(&4), None);
    assert!(!f.f.sent.iter().any(|m| m.0 == P3));
    // The tome handed in: deleted, chat end pending, callback 2 stored.
    assert!(!f.p(P1).items.contains(b"bbb "));
    assert!(f.log().contains(&"delete bbb ".to_string()));
    assert!(x(&ctl).brought);
    assert_eq!(x(&ctl).brought_by, 1);
    assert!(!x(&ctl).reward_open);
    let r = ctl.record(C).unwrap();
    assert!(r.has_callback(event::NPC_DEACTIVATE));
    // Tail: status 13 silent, GUID added, game 17.13, state 5.
    assert_eq!((r.status, r.state), (13, 5));
    assert!(r.guids.contains(1));
    assert!(ctl.game.get(S, 13));
    // The refresh comes first (0x27 before any 0x28).
    let first = f.f.sent.iter().position(|m| m.1[0] == 0x27).unwrap();
    let flags = f.f.sent.iter().position(|m| m.1[0] == 0x28).unwrap();
    assert!(first < flags);
    no_unhandled(&f);
}

// Covers: specs/world/quests-act3.md §3.3, §edge-cases-original-bugs r1
#[test]
fn alkor_564_without_tome() {
    // Edge case 1: the reward goes out whether or not a tome was handed
    // in; no chat-end callback is stored.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    f.add_player(P2, 75);
    call(&mut ctl, &mut f, C, alkor(P2, 564));
    for p in [P1, P2] {
        assert!(f.flags(p).get(S, 0));
        assert_eq!(f.f.players[&p].stats.get(&4), Some(&5));
    }
    assert!(!x(&ctl).brought);
    let r = ctl.record(C).unwrap();
    assert!(!r.has_callback(event::NPC_DEACTIVATE));
    assert!(r.guids.contains(2));
    assert!(ctl.game.get(S, 13));
    assert_eq!(r.state, 5);

    // A later 564 (reward handed out): no reward, the tail still runs.
    f.add_player(P3, 75);
    f.f.sent.clear();
    let i = idx(&ctl);
    ctl.records[i].status = 0;
    call(&mut ctl, &mut f, C, alkor(P3, 564));
    assert!(!f.flags(P3).get(S, 0) && !f.flags(P3).get(S, 14));
    assert_eq!(f.f.players[&P3].stats.get(&4), None);
    assert!(!sent_5d(&f).iter().any(|m| m.1[1] == 0x0F && m.1[2] == 2));
    let r = ctl.record(C).unwrap();
    assert_eq!(r.status, 13);
    assert!(r.guids.contains(3));
}

// Covers: specs/world/quests-act3.md §3.3
#[test]
fn alkor_564_intro_and_state5() {
    // Intro: rewards, but no game 17.13 and no state change.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    ctl.records[i].not_intro = false;
    call(&mut ctl, &mut f, C, alkor(P1, 564));
    assert!(f.flags(P1).get(S, 13));
    assert!(!ctl.game.get(S, 13));
    assert_eq!(ctl.records[i].state, 0);
    // Already state 5: game 17.13, state unchanged.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    ctl.records[i].state = 5;
    call(&mut ctl, &mut f, C, alkor(P1, 564));
    assert!(ctl.game.get(S, 13));
    assert_eq!(ctl.records[i].state, 5);
}

// Covers: specs/world/quests-act3.md §3.7, §edge-cases-original-bugs r6
#[test]
fn chat_end_sound() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    f.add_player(P2, 75);
    f.p(P1).items.push(*b"bbb ");
    call(&mut ctl, &mut f, C, alkor(P1, 564));
    let end = |p: UnitId, n: UnitId| ev(event::NPC_DEACTIVATE, p, Some(n), 0, 0);
    // Another NPC, another player: nothing.
    call(&mut ctl, &mut f, C, end(P1, ORMUS_U));
    call(&mut ctl, &mut f, C, end(P2, ALKOR_U));
    assert!(x(&ctl).brought);
    assert!(!f.log().iter().any(|l| l.starts_with("sound")));
    // The bringer at Alkor: sound 67, pending cleared, callback stays.
    call(&mut ctl, &mut f, C, end(P1, ALKOR_U));
    assert!(f.log().contains(&"sound 1 67".to_string()));
    assert!(!x(&ctl).brought);
    assert!(ctl.record(C).unwrap().has_callback(event::NPC_DEACTIVATE));
    // Once only.
    call(&mut ctl, &mut f, C, end(P1, ALKOR_U));
    assert_eq!(f.log().iter().filter(|l| l.starts_with("sound")).count(), 1);
    no_unhandled(&f);
}

// Covers: specs/world/quests-act3.md §3.7
#[test]
fn chat_end_druid_assassin_silent() {
    for class in [5u8, 6] {
        let (mut ctl, _) = control();
        let mut f = Fake3::new();
        f.p(P1).class = class;
        f.p(P1).items.push(*b"bbb ");
        call(&mut ctl, &mut f, C, alkor(P1, 564));
        call(
            &mut ctl,
            &mut f,
            C,
            ev(event::NPC_DEACTIVATE, P1, Some(ALKOR_U), 0, 0),
        );
        assert!(!f.log().iter().any(|l| l.starts_with("sound")));
        assert!(!x(&ctl).brought);
    }
}

// Covers: specs/world/quests-act3.md §3.4
#[test]
fn level_changes() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    let lvl = |old: u32, new: u32| ev(event::CHANGED_LEVEL, P1, Some(P1), old, new);
    // Entering Lower Kurast at state 0: state 1, no status.
    call(&mut ctl, &mut f, C, lvl(78, 79));
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (1, 0));
    assert!(f.f.sent.is_empty());
    // Not at state 0, or intro: nothing.
    ctl.records[i].state = 2;
    call(&mut ctl, &mut f, C, lvl(78, 79));
    assert_eq!(ctl.records[i].state, 2);
    // Leaving the Docks at state 2 (17.1 set but not tested): quick
    // remove, status 1 to all, state 3, flag iterate.
    ctl.records[i].guids.add(1);
    ctl.records[i].guids.add(9);
    setf(&mut f, P1, S, 1);
    f.add_player(P2, 76);
    call(&mut ctl, &mut f, C, lvl(75, 76));
    let r = &ctl.records[i];
    assert_eq!((r.state, r.status), (3, 1));
    assert_eq!(r.guids.0, vec![9]);
    assert_eq!(sent_5d(&f).len(), 2);
    assert!(!f.flags(P1).get(S, 2));
    assert!(f.flags(P2).get(S, 2));
    // Status already 1: no send.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    ctl.records[i].state = 2;
    ctl.records[i].status = 1;
    call(&mut ctl, &mut f, C, lvl(75, 76));
    assert_eq!(ctl.records[i].state, 3);
    assert!(sent_5d(&f).is_empty());
    assert!(f.flags(P1).get(S, 2));
    // With 17.0: nothing; intro at state 0: no start.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    ctl.records[i].state = 2;
    setf(&mut f, P1, S, 0);
    call(&mut ctl, &mut f, C, lvl(75, 76));
    assert_eq!(ctl.records[i].state, 2);
    ctl.records[i].state = 0;
    ctl.records[i].not_intro = false;
    call(&mut ctl, &mut f, C, lvl(78, 79));
    assert_eq!(ctl.records[i].state, 0);
    no_unhandled(&f);
}

// Covers: specs/world/quests-act3.md §3.5
#[test]
fn pick_up_and_drop() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::ITEM_PICKED_UP, P1, Some(ITEM_U), 0, 0),
    );
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (4, 2));
    assert_eq!(sent_5d(&f).len(), 1);
    assert_eq!(x(&ctl).holders.0, vec![1]);
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::ITEM_DROPPED, P1, Some(ITEM_U), 0, 0),
    );
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (3, 1));
    assert!(x(&ctl).holders.0.is_empty());
    // Intro: only the holder list moves.
    ctl.records[i].not_intro = false;
    f.f.sent.clear();
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::ITEM_PICKED_UP, P1, Some(ITEM_U), 0, 0),
    );
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (3, 1));
    assert_eq!(x(&ctl).holders.0, vec![1]);
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::ITEM_DROPPED, P1, Some(ITEM_U), 0, 0),
    );
    assert!(x(&ctl).holders.0.is_empty());
    assert!(f.f.sent.is_empty());
    no_unhandled(&f);
}

// Covers: specs/world/quests-act3.md §3.5
#[test]
fn leave_and_join() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    {
        let x = &mut ctl.records[i].extra.act3.q1;
        x.tomes = 1;
        x.tome_active = true;
    }
    ctl.records[i].state = 3;
    call(
        &mut ctl,
        &mut f,
        C,
        ev(
            event::PLAYER_DROPPED_WITH_QUEST_ITEM,
            P1,
            Some(ITEM_U),
            0,
            0,
        ),
    );
    assert_eq!(x(&ctl).tomes, 0);
    assert!(x(&ctl).holder_left);
    assert_eq!(ctl.records[i].status, 8);
    assert_eq!(sent_5d(&f).len(), 1);
    // A joining player without the tome: nothing.
    f.add_player(P2, 75);
    f.f.sent.clear();
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::PLAYER_JOINED_GAME, P2, Some(P2), 0, 0),
    );
    assert_eq!(x(&ctl).tomes, 0);
    assert!(x(&ctl).holder_left);
    // With the tome: count 1, holder_left cleared, status 2 to all.
    f.p(P2).items.push(*b"bbb ");
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::PLAYER_JOINED_GAME, P2, Some(P2), 0, 0),
    );
    assert_eq!(x(&ctl).tomes, 1);
    assert!(!x(&ctl).holder_left);
    assert_eq!(ctl.records[i].status, 2);
    assert_eq!(sent_5d(&f).len(), 2);
    // A second holder: count 2, no send.
    f.f.sent.clear();
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::PLAYER_JOINED_GAME, P2, Some(P2), 0, 0),
    );
    assert_eq!(x(&ctl).tomes, 2);
    assert!(f.f.sent.is_empty());
    // Leaving at state 4, or with the tome inactive: count only.
    ctl.records[i].state = 4;
    call(
        &mut ctl,
        &mut f,
        C,
        ev(
            event::PLAYER_DROPPED_WITH_QUEST_ITEM,
            P1,
            Some(ITEM_U),
            0,
            0,
        ),
    );
    call(
        &mut ctl,
        &mut f,
        C,
        ev(
            event::PLAYER_DROPPED_WITH_QUEST_ITEM,
            P1,
            Some(ITEM_U),
            0,
            0,
        ),
    );
    assert_eq!(x(&ctl).tomes, 0);
    assert!(!x(&ctl).holder_left);
    assert!(f.f.sent.is_empty());
    no_unhandled(&f);
}

// Covers: specs/world/quests-act3.md §3.5, §1.1
#[test]
fn leaving_game_removes_guid() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    ctl.records[i].guids.add(1);
    ctl.records[i].guids.add(4);
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::PLAYER_LEAVES_GAME, P1, Some(P1), 0, 0),
    );
    assert_eq!(ctl.records[i].guids.0, vec![4]);
    no_unhandled(&f);
}

// Covers: specs/world/quests-act3.md §3.6, §3.1
#[test]
fn tome_operate_drops() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    f.f.objects.insert(TOME_U, (0x77, 193, 0));
    ctl.records[i].clear_callback(event::NPC_ACTIVATE);
    ctl.records[i].clear_callback(event::SCROLL_MESSAGE);
    act3::tome_operate(&mut ctl, &mut f, TOME_U, P1);
    assert_eq!(
        f.log(),
        vec![
            format!("qdrop {} bbb  2 false", TOME_U.0),
            format!("mode {} 2", TOME_U.0)
        ]
    );
    let r = &ctl.records[i];
    let x = &r.extra.act3.q1;
    assert_eq!((x.tomes, x.tome_mode, x.tome_guid), (1, 2, 0x77));
    assert!(x.tome_active && x.tome_dropped);
    assert!(r.has_callback(event::NPC_ACTIVATE) && r.has_callback(event::SCROLL_MESSAGE));
    assert_eq!((r.state, r.status), (3, 1));
    assert!(f.f.sent.is_empty());
    // Mode now 2: a second operate does nothing.
    act3::tome_operate(&mut ctl, &mut f, TOME_U, P1);
    assert_eq!(x_tomes(&ctl), 1);
    assert_eq!(f.log().len(), 2);
}

fn x_tomes(ctl: &QuestControl) -> i32 {
    x(ctl).tomes
}

// Covers: specs/world/quests-act3.md §3.6
#[test]
fn tome_operate_refused_and_failed() {
    // 17.0: sound 19, nothing dropped.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    f.f.objects.insert(TOME_U, (0x77, 193, 0));
    setf(&mut f, P1, S, 0);
    act3::tome_operate(&mut ctl, &mut f, TOME_U, P1);
    assert_eq!(f.log(), vec!["sound 1 19".to_string()]);
    // Intro: nothing at all (not even the sound).
    let mut g = Fake3::new();
    g.f.objects.insert(TOME_U, (0x77, 193, 0));
    setf(&mut g, P1, S, 0);
    ctl.records[i].not_intro = false;
    act3::tome_operate(&mut ctl, &mut g, TOME_U, P1);
    assert!(g.log().is_empty());
    // Not created: no state change.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    f.f.objects.insert(TOME_U, (0x77, 193, 0));
    f.drops = vec![false];
    ctl.records[i].status = 5;
    act3::tome_operate(&mut ctl, &mut f, TOME_U, P1);
    assert_eq!(x_tomes(&ctl), 0);
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (0, 5));
    assert_eq!(f.f.objects[&TOME_U].2, 0);
    // State already 3, status 2: state stays, status := 1 silently.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    f.f.objects.insert(TOME_U, (0x77, 193, 0));
    ctl.records[i].state = 3;
    ctl.records[i].status = 2;
    ctl.records[i].flags = 9;
    act3::tome_operate(&mut ctl, &mut f, TOME_U, P1);
    let r = &ctl.records[i];
    assert_eq!((r.state, r.status, r.flags), (3, 1, 0));
}

// Covers: specs/world/quests-act3.md §3.6
#[test]
fn tome_init_mode() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    f.f.objects.insert(TOME_U, (0x77, 193, 0));
    act3::tome_init(&mut ctl, &mut f, TOME_U);
    assert_eq!(f.f.objects[&TOME_U].2, 0);
    ctl.records[i].extra.act3.q1.tome_mode = 2;
    act3::tome_init(&mut ctl, &mut f, TOME_U);
    assert_eq!(f.f.objects[&TOME_U].2, 2);
    ctl.records[i].extra.act3.q1.tome_mode = 0;
    ctl.records[i].not_intro = false;
    f.f.objects.insert(TOME_U, (0x77, 193, 0));
    act3::tome_init(&mut ctl, &mut f, TOME_U);
    assert_eq!(f.f.objects[&TOME_U].2, 2);
    assert!(ctl.faults.is_empty());
    // No chain 15: fatal.
    ctl.records.remove(i);
    act3::tome_init(&mut ctl, &mut f, TOME_U);
    assert_eq!(ctl.faults, vec![QuestError::Fatal(0x0054_4E30)]);
}

// Covers: specs/world/quests-act3.md §3.7
#[test]
fn game_start() {
    let start = |ctl: &mut QuestControl, f: &mut Fake3| {
        call(
            ctl,
            f,
            C,
            ev(event::PLAYER_STARTED_GAME, P1, Some(P1), 0, 0),
        );
    };
    // 17.0 → game 17.13 only.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    setf(&mut f, P1, S, 0);
    f.p(P1).items.push(*b"bbb ");
    start(&mut ctl, &mut f);
    assert!(ctl.game.get(S, 13));
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (0, 0));
    // Holding the tome.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    f.p(P1).items.push(*b"bbb ");
    setf(&mut f, P1, S, 3);
    start(&mut ctl, &mut f);
    let r = &ctl.records[i];
    assert_eq!((r.state, r.status), (4, 2));
    let x = &r.extra.act3.q1;
    assert_eq!((x.tomes, x.tome_mode), (1, 2));
    assert!(x.tome_active);
    assert_eq!(x.holders.0, vec![1]);
    assert!(!ctl.game.get(S, 13));
    // 17.3 (before 17.2) → state 3; 17.2 → state 2; status 1.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    setf(&mut f, P1, S, 3);
    setf(&mut f, P1, S, 2);
    start(&mut ctl, &mut f);
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (3, 1));
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    setf(&mut f, P1, S, 2);
    start(&mut ctl, &mut f);
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (2, 1));
    // Nothing set: nothing; nothing sent in any case.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    start(&mut ctl, &mut f);
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (0, 0));
    assert!(f.f.sent.is_empty());
    no_unhandled(&f);
}

// Covers: specs/world/quests-act3.md §3.8
#[test]
fn status_function() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    // 15.0 clear → 0.
    ctl.records[i].state = 2;
    assert_eq!(status_of(&ctl, &mut f, P1), 0);
    setf(&mut f, P1, 15, 0);
    // Not-intro, state < 4 → 1.
    assert_eq!(status_of(&ctl, &mut f, P1), 1);
    // Tome active, none in the game → 8 + (game type ≠ 3).
    ctl.records[i].extra.act3.q1.tome_active = true;
    assert_eq!(status_of(&ctl, &mut f, P1), 9);
    f.f.game_type = 3;
    assert_eq!(status_of(&ctl, &mut f, P1), 8);
    ctl.records[i].extra.act3.q1.tomes = 1;
    assert_eq!(status_of(&ctl, &mut f, P1), 1);
    // State ≥ 4: 1 in Act III with a tome in the game, else 12.
    ctl.records[i].state = 4;
    assert_eq!(status_of(&ctl, &mut f, P1), 1);
    f.p(P1).act = Some(1);
    assert_eq!(status_of(&ctl, &mut f, P1), 12);
    f.p(P1).act = Some(2);
    ctl.records[i].extra.act3.q1.tomes = 0;
    assert_eq!(status_of(&ctl, &mut f, P1), 12);
    // A party member holding the tome → 2; without → falls through.
    f.add_player(P2, 75);
    f.f.party.insert(P1, vec![P1, P2]);
    assert_eq!(status_of(&ctl, &mut f, P1), 12);
    f.p(P2).items.push(*b"bbb ");
    assert_eq!(status_of(&ctl, &mut f, P1), 2);
    // Holding it → 2.
    f.f.party.clear();
    f.p(P1).items.push(*b"bbb ");
    assert_eq!(status_of(&ctl, &mut f, P1), 2);
    // 17.0 → 11 + 2 × 17.13.
    setf(&mut f, P1, S, 0);
    assert_eq!(status_of(&ctl, &mut f, P1), 11);
    setf(&mut f, P1, S, 13);
    assert_eq!(status_of(&ctl, &mut f, P1), 13);
    // Intro without tome or bits → 0.
    let mut g = Fake3::new();
    setf(&mut g, P1, 15, 0);
    ctl.records[i].not_intro = false;
    assert_eq!(status_of(&ctl, &mut g, P1), 0);
}

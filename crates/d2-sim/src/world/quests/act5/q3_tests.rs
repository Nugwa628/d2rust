// Spec: specs/world/quests-act5.md §5 (Test vectors, Randomness, Edge cases)
//! A5Q3 Prison of Ice on the quests' fake world.

use super::super::super::tests::*;
use super::super::super::{
    act1, event, EventArgs, QuestControl, QuestFlags, QuestWorld, TextList, UnitKind,
};
use super::{
    anya_dummy_event, anya_dummy_init, anya_items, anya_tier, anya_town_dummy_init,
    apply_resist_scroll, frozen_anya_init, frozen_anya_operate, map_ai_store,
    nihlathak_dummy_event, nihlathak_temple_dummy_init, nihlathak_town_dummy_init, portal_event,
    town_cleanup, use_resist_scroll, CHAIN, DREHYA, DREHYAICED, MALAH, NIHLATHAK, SLOT,
};
use crate::rng::Seed;
use crate::units::{RoomId, UnitId};

const MALAH_U: UnitId = UnitId(0x40);
const DREHYA_U: UnitId = UnitId(0x41);
const ICED_U: UnitId = UnitId(0x42);
const NIHL_U: UnitId = UnitId(0x43);
const LARZUK_U: UnitId = UnitId(0x44);
const DUMMY_U: UnitId = UnitId(0x60);
const FANA_U: UnitId = UnitId(0x61);
const PORTAL_U: UnitId = UnitId(0x62);
const ICE: [u8; 4] = *b"ice ";

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
    f.p(P1).level = Some(109);
    for (u, class) in [
        (MALAH_U, MALAH),
        (DREHYA_U, DREHYA),
        (ICED_U, DREHYAICED),
        (LARZUK_U, 511),
    ] {
        f.monsters.insert(u, (u.0, class, kind(class)));
    }
    f.frame = 500;
    f
}

fn x(ctl: &mut QuestControl) -> &mut super::Extra {
    &mut ctl.record_mut(CHAIN).unwrap().extra.a5.q3
}

fn bits(f: &mut Fake, d: usize, b: &[u8]) {
    for &b in b {
        f.p(P1).quests.flags[d].set(SLOT, b);
    }
}

fn ev(ctl: &mut QuestControl, f: &mut Fake, args: EventArgs) {
    let i = ctl.find(CHAIN).unwrap();
    act1::callback(ctl, f, i, args, None, false);
}

fn say(ctl: &mut QuestControl, f: &mut Fake, npc: Option<(UnitId, u16)>, msg: u32) {
    ev(
        ctl,
        f,
        EventArgs {
            event: event::SCROLL_MESSAGE,
            target: npc.map(|n| n.0),
            player: Some(P1),
            a: u32::from(npc.map_or(0, |n| n.1)),
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

fn status(ctl: &QuestControl, f: &mut Fake, pf: &QuestFlags) -> Option<u8> {
    let i = ctl.find(CHAIN).unwrap();
    super::status(ctl, f, i, P1, pf, 0x0058_A300)
}

fn fx(b: &[u8]) -> QuestFlags {
    let mut q = QuestFlags::default();
    for &b in b {
        q.set(SLOT, b);
    }
    q
}

/// A quest seed whose first roll(n) is `want`.
fn seed_rolling(n: i32, want: u32) -> Seed {
    (0..)
        .map(Seed::init_low)
        .find(|s| {
            let mut c = *s;
            c.roll(n) == want
        })
        .unwrap()
}

// ------------------------------------------------------------ §5.7

// Covers: specs/world/quests-act5.md §5.7
#[test]
fn vector_anya_item_hell_assassin() {
    // Hell, level 70, assassin, quest-seed roll(7) = 3 → `7cs` rare at
    // the level of `0x00558200`.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.difficulty = 2;
    f.p(P1).class = 6;
    f.p(P1).stats.insert(12, 70);
    f.a5_item_level = 77;
    f.a5_drop_sound = 5;
    bits(&mut f, 2, &[1]);
    ctl.seed = seed_rolling(7, 3);
    let mut after = ctl.seed;
    after.roll(7);
    say(&mut ctl, &mut f, Some((DREHYA_U, DREHYA)), 20136);
    assert_eq!(f.log, ["reward 7cs  77 6"]);
    assert_eq!(ctl.seed, after);
    assert_eq!(f.sent, [(P1, hex("5D 21 10 00 0500"))]);
    let fl = f.flags(P1);
    assert!(fl.get(SLOT, 9) && fl.get(SLOT, 10) && fl.get(SLOT, 1) && !fl.get(SLOT, 0));
    // Taken once per difficulty: a second message does nothing.
    f.log.clear();
    say(&mut ctl, &mut f, Some((DREHYA_U, DREHYA)), 20136);
    assert!(f.log.is_empty() && ctl.seed == after);
}

// Covers: specs/world/quests-act5.md §5.7
#[test]
fn vector_anya_item_nightmare_normal_list() {
    // Nightmare, level 40, game type ≠ 3, barbarian, roll(5) = 0 → ba1.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.difficulty = 1;
    f.p(P1).class = 4;
    f.p(P1).stats.insert(12, 40);
    f.a5_drop_sound = -1;
    bits(&mut f, 1, &[1, 8]);
    ctl.seed = seed_rolling(5, 0);
    say(&mut ctl, &mut f, Some((DREHYA_U, DREHYA)), 20136);
    assert_eq!(f.log, ["reward ba1  0 6"]);
    // A drop sound ≤ 0 sends 0; with 37.8 the reward is complete.
    assert_eq!(f.sent, [(P1, hex("5D 21 10 00 0000"))]);
    let fl = f.flags(P1);
    assert!(fl.get(SLOT, 0) && !fl.get(SLOT, 1) && fl.get(SLOT, 9) && fl.get(SLOT, 10));
    // Tiers.
    assert_eq!(anya_tier(1, 0, 40), 0);
    assert_eq!(anya_tier(1, 0, 46), 1);
    assert_eq!(anya_tier(1, 3, 1), 1);
    assert_eq!(anya_tier(2, 0, 65), 0);
    assert_eq!(anya_tier(2, 0, 66), 2);
    assert_eq!(anya_tier(2, 3, 1), 2);
    assert_eq!(anya_tier(0, 3, 99), 0);
    // Lists.
    let s = |l: Vec<[u8; 4]>| -> Vec<String> {
        l.iter()
            .map(|c| String::from_utf8_lossy(c).trim_end().to_string())
            .collect()
    };
    assert_eq!(
        s(anya_items(0, 0).unwrap()),
        ["am1", "am2", "am3", "am4", "am5"]
    );
    assert_eq!(
        s(anya_items(1, 1).unwrap()),
        ["ob6", "ob7", "ob8", "ob9", "oba"]
    );
    assert_eq!(
        s(anya_items(2, 5).unwrap()),
        ["drb", "drc", "drd", "dre", "drf"]
    );
    assert_eq!(
        s(anya_items(0, 6).unwrap()),
        ["ktr", "wrb", "axf", "ces", "clw", "btl", "skr"]
    );
    assert_eq!(
        s(anya_items(1, 6).unwrap()),
        ["9ar", "9wb", "9xf", "9cs", "9lw", "9tw", "9qr"]
    );
    assert_eq!(anya_items(0, 7), None);
    // Needs 37.1 and not 37.0.
    let (mut ctl, _) = control();
    let mut f = fake();
    say(&mut ctl, &mut f, Some((DREHYA_U, DREHYA)), 20136);
    bits(&mut f, 0, &[1, 0]);
    say(&mut ctl, &mut f, Some((DREHYA_U, DREHYA)), 20136);
    assert!(f.log.is_empty() && f.sent.is_empty());
}

// Covers: specs/world/quests-act5.md §5.7, §edge-cases-original-bugs r8
#[test]
fn vector_resist_scroll() {
    // Scroll used after the normal one (37.7 in two records) → 20.
    let mut f = fake();
    f.difficulty = 1;
    bits(&mut f, 0, &[7, 8]);
    bits(&mut f, 1, &[8]);
    assert!(use_resist_scroll(&mut f, P1));
    assert!(f.flags(P1).get(SLOT, 7));
    assert_eq!(f.log, ["resist 1 20"]);
    assert_eq!(f.sent, [(P1, hex("5D 21 02 00 0000"))]);
    // Used: not again; without 37.8: refused.
    assert!(!use_resist_scroll(&mut f, P1));
    let mut g = fake();
    assert!(!use_resist_scroll(&mut g, P1));
    // At load: the sum over all three difficulties, added again (open
    // question 3: stacking); nothing for v = 0.
    bits(&mut f, 2, &[7]);
    apply_resist_scroll(&mut f, P1);
    apply_resist_scroll(&mut g, P1);
    assert_eq!(f.log, ["resist 1 20", "resist 1 30"]);
    assert!(g.log.is_empty());
}

// Covers: specs/world/quests-act5.md §5.7, §5.9, §edge-cases-original-bugs r9
#[test]
fn scroll_reward() {
    // 37.1 without 37.9: the scroll, S5D, status 6 to all.
    let (mut ctl, _) = control();
    let mut f = fake();
    bits(&mut f, 0, &[1]);
    say(&mut ctl, &mut f, Some((MALAH_U, MALAH)), 20132);
    assert_eq!(f.log, ["reward tr2  0 2"]);
    assert!(f.flags(P1).get(SLOT, 8) && f.flags(P1).get(SLOT, 1));
    assert_eq!(ctl.record(CHAIN).unwrap().status, 6);
    assert_eq!(f.sent_ids(), [0x5D, 0x5D, 0x28]);
    assert_eq!(f.sent[0].1, hex("5D 21 02 00 0000"));
    assert_eq!(f.sent[1].1, hex("5D 21 00 06 0000"));
    // With 37.9: done (37.0); Anya thawed → the iced Anya is killed,
    // Anya goes to town.
    let (mut ctl, _) = control();
    let mut f = fake();
    bits(&mut f, 0, &[1, 9]);
    x(&mut ctl).anya = 1;
    x(&mut ctl).town_dummy_seen = true;
    x(&mut ctl).town_dummy_guid = 0x60;
    f.objects.insert(DUMMY_U, (0x60, 459, 0));
    f.pos.insert(DUMMY_U, (7, 8, RoomId(2)));
    f.pos.insert(DREHYA_U, (7, 8, RoomId(2)));
    // The room covering her position (`0x00463740`).
    f.rooms.insert(RoomId(2), (0, 0, 100, 100));
    f.a5_crits = vec![Some(DREHYA_U)];
    f.a5_places = vec![Some(PORTAL_U)];
    f.objects.insert(PORTAL_U, (0x62, 189, 1));
    ctl.record_mut(CHAIN).unwrap().state = 5;
    say(&mut ctl, &mut f, Some((MALAH_U, MALAH)), 20132);
    let fl = f.flags(P1);
    assert!(fl.get(SLOT, 0) && !fl.get(SLOT, 1) && fl.get(SLOT, 8));
    assert_eq!(
        f.log,
        [
            "reward tr2  0 2",
            "kill 66",
            "critical 512 7 8 room 2",
            "place 189 7 8 room 2 [1, 1, 0]",
        ]
    );
    let e = x(&mut ctl);
    assert!(e.iced_found && e.outside_may_close && e.anya == 2);
    assert!(e.anya_in_town && e.anya_guid == 0x41);
    assert!(e.town_portal && e.town_portal_guid == 0x62);
    // Status 6 is not sent with 37.9.
    assert_eq!(ctl.record(CHAIN).unwrap().status, 0);
    // No frozen Anya monster: its inactive node is dropped. An iced Anya
    // someone talks to: `0x00573180` with `0x00589070` (0x62 (1, her
    // GUID), interaction cleared, S5D(33, 0x20, 0) to each chatting
    // player, the list freed); +0xE3 stays 0, so the drop step runs too.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.monsters.remove(&ICED_U);
    bits(&mut f, 0, &[1, 9]);
    x(&mut ctl).anya = 1;
    say(&mut ctl, &mut f, Some((MALAH_U, MALAH)), 20132);
    assert_eq!(f.log, ["reward tr2  0 2", "preset 4 527"]);
    assert!(!x(&mut ctl).iced_found && x(&mut ctl).anya == 2);
    let (mut ctl, _) = control();
    let mut f = fake();
    f.chats.insert(ICED_U, vec![P1]);
    bits(&mut f, 0, &[1, 9]);
    x(&mut ctl).anya = 1;
    say(&mut ctl, &mut f, Some((MALAH_U, MALAH)), 20132);
    assert_eq!(
        f.log,
        [
            "reward tr2  0 2",
            "interact 1 None",
            "clear chats 66",
            "preset 4 527"
        ]
    );
    assert!(!x(&mut ctl).iced_found);
    let tail: Vec<_> = f
        .sent
        .iter()
        .filter(|m| m.1[0] != 0x28)
        .skip(1)
        .cloned()
        .collect();
    assert_eq!(
        tail,
        [
            (P1, hex("62 01 42000000 00")),
            (P1, hex("5D 21 20 00 0000"))
        ]
    );
    // Again later (37.0, 37.8, not used, none held): once per game.
    let (mut ctl, _) = control();
    let mut f = fake();
    bits(&mut f, 0, &[0, 8]);
    say(&mut ctl, &mut f, Some((MALAH_U, MALAH)), 20132);
    say(&mut ctl, &mut f, Some((MALAH_U, MALAH)), 20132);
    assert_eq!(f.log, ["reward tr2  0 2"]);
    assert!(x(&mut ctl).scroll_again && f.sent.is_empty());
    // Used (37.7) or never given (37.8 clear): no second scroll.
    for b in [&[15u8, 8, 7][..], &[15]] {
        let (mut ctl, _) = control();
        let mut f = fake();
        bits(&mut f, 0, b);
        say(&mut ctl, &mut f, Some((MALAH_U, MALAH)), 20132);
        assert!(f.log.is_empty());
    }
}

// ------------------------------------------------------------ §5.3, §5.4

// Covers: specs/world/quests-act5.md §5.3 text, §5.3 r1, §5.3 r2, §5.3 r3, §5.3 r4, §edge-cases-original-bugs r3
#[test]
fn chat_steps_and_active() {
    let (mut ctl, _) = control();
    let mut f = fake();
    // r1: the iced Anya at state 6 is killed, nothing added.
    ctl.record_mut(CHAIN).unwrap().state = 6;
    assert!(text(&mut ctl, &mut f, ICED_U).is_empty());
    assert_eq!(f.log, ["kill 66"]);
    // r3: state 1 → table 0 (Malah's start).
    ctl.record_mut(CHAIN).unwrap().state = 1;
    assert_eq!(text(&mut ctl, &mut f, MALAH_U), [(20116, 0)]);
    // r2: Malah at state 4, no potion anywhere → table 3.
    ctl.record_mut(CHAIN).unwrap().state = 4;
    assert_eq!(text(&mut ctl, &mut f, MALAH_U), [(20127, 0)]);
    // A potion in the game: index[4] = 3 again from step 3.
    x(&mut ctl).potions = 1;
    assert_eq!(text(&mut ctl, &mut f, MALAH_U), [(20127, 0)]);
    assert_eq!(text(&mut ctl, &mut f, LARZUK_U), [(20129, 2)]);
    // State > 4 needs 37.13; past the table → nothing.
    ctl.record_mut(CHAIN).unwrap().state = 5;
    assert!(text(&mut ctl, &mut f, LARZUK_U).is_empty());
    bits(&mut f, 0, &[13]);
    assert_eq!(text(&mut ctl, &mut f, LARZUK_U), [(20129, 2)]);
    ctl.record_mut(CHAIN).unwrap().state = 7;
    assert!(text(&mut ctl, &mut f, LARZUK_U).is_empty());
    // GUID listed → 6; 37.0 → nothing; intro → nothing.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().guids.add(1);
    assert_eq!(text(&mut ctl, &mut f, LARZUK_U), [(20134, 2)]);
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().state = 1;
    bits(&mut f, 0, &[0]);
    assert!(text(&mut ctl, &mut f, MALAH_U).is_empty());
    ctl.record_mut(CHAIN).unwrap().not_intro = false;
    let mut f = fake();
    assert!(text(&mut ctl, &mut f, MALAH_U).is_empty());
    // r4: 37.1 set → table 5, except Anya with 37.9 or Malah with 37.8
    // (edge case 3: Malah's re-offer never comes from table 5).
    let (mut ctl, _) = control();
    let mut f = fake();
    bits(&mut f, 0, &[1]);
    assert_eq!(text(&mut ctl, &mut f, DREHYA_U), [(20136, 0)]);
    assert_eq!(text(&mut ctl, &mut f, MALAH_U), [(20132, 0)]);
    bits(&mut f, 0, &[8, 9]);
    assert!(text(&mut ctl, &mut f, DREHYA_U).is_empty());
    assert!(text(&mut ctl, &mut f, MALAH_U).is_empty());
    assert_eq!(text(&mut ctl, &mut f, LARZUK_U), [(20134, 2)]);
    // Active.
    let (mut ctl, _) = control();
    let i = ctl.find(CHAIN).unwrap();
    let mut f = fake();
    let act = |ctl: &QuestControl, f: &mut Fake, c| super::active(ctl, f, i, P1, c, 0x0058_9F10);
    assert!(!act(&ctl, &mut f, MALAH) && act(&ctl, &mut f, DREHYAICED));
    ctl.records[i].state = 1;
    assert!(act(&ctl, &mut f, MALAH) && !act(&ctl, &mut f, 511));
    ctl.records[i].state = 4;
    assert!(act(&ctl, &mut f, MALAH));
    ctl.records[i].extra.a5.q3.potions = 1;
    assert!(!act(&ctl, &mut f, MALAH));
    f.p(P1).items.push(ICE);
    assert!(!act(&ctl, &mut f, DREHYAICED));
    ctl.records[i].state = 5;
    f.p(P1).items.clear();
    assert!(!act(&ctl, &mut f, DREHYAICED) && !act(&ctl, &mut f, DREHYA));
    bits(&mut f, 0, &[1]);
    assert!(act(&ctl, &mut f, MALAH) && act(&ctl, &mut f, DREHYA));
    bits(&mut f, 0, &[8, 9]);
    assert!(!act(&ctl, &mut f, MALAH) && !act(&ctl, &mut f, DREHYA));
}

// Covers: specs/world/quests-act5.md §5.4, §5.2
#[test]
fn malah_messages_and_chat_end() {
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().state = 1;
    say(&mut ctl, &mut f, Some((MALAH_U, MALAH)), 20116);
    assert!(x(&mut ctl).started && f.flags(P1).get(SLOT, 2));
    assert_eq!(ctl.record(CHAIN).unwrap().state, 2);
    assert_eq!(f.sent_ids(), [0x27, 0x29]);
    // 20127: the potion once.
    f.sent.clear();
    f.log.clear();
    say(&mut ctl, &mut f, Some((MALAH_U, MALAH)), 20127);
    say(&mut ctl, &mut f, Some((MALAH_U, MALAH)), 20127);
    assert_eq!(f.log, ["reward ice  0 2"]);
    assert_eq!(f.sent, [(P1, hex("5D 21 01 00 0000"))]);
    assert!(x(&mut ctl).potions == 1 && x(&mut ctl).potion_given);
    // A message from another NPC: nothing.
    say(&mut ctl, &mut f, Some((LARZUK_U, 511)), 20116);
    assert_eq!(ctl.record(CHAIN).unwrap().state, 2);
    // Chat end with Malah: status 1, then status 4 (potion given).
    f.sent.clear();
    let end = EventArgs {
        event: event::NPC_DEACTIVATE,
        target: Some(MALAH_U),
        player: Some(P1),
        ..EventArgs::default()
    };
    ev(&mut ctl, &mut f, end);
    // P1 holds the potion: its status fn says 4 both times.
    assert_eq!(
        f.sent,
        [(P1, hex("5D 21 00 04 0000")), (P1, hex("5D 21 00 04 0000"))]
    );
    let e = x(&mut ctl);
    assert!(!e.started && !e.potion_given);
    assert_eq!(ctl.record(CHAIN).unwrap().status, 4);
    // Status ≥ 4: the potion flag stays.
    x(&mut ctl).potion_given = true;
    ev(&mut ctl, &mut f, end);
    assert!(x(&mut ctl).potion_given);
}

// Covers: specs/world/quests-act5.md §5.4, §5.9
#[test]
fn statue_text_20131() {
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().state = 3;
    x(&mut ctl).nihlathak_in_town = true;
    x(&mut ctl).nihlathak_guid = 0x43;
    x(&mut ctl).nihlathak_dummy_guid = 0x70;
    f.monsters
        .insert(NIHL_U, (0x43, NIHLATHAK, kind(NIHLATHAK)));
    f.objects.insert(UnitId(0x70), (0x70, 461, 0));
    say(&mut ctl, &mut f, None, 20131);
    assert_eq!(ctl.record(CHAIN).unwrap().state, 4);
    assert_eq!(ctl.record(CHAIN).unwrap().status, 3);
    assert_eq!(f.log, ["kill in town 67", "event7 112 501"]);
    let e = x(&mut ctl);
    assert!(!e.nihlathak_in_town && e.nihlathak_gone);
    // Intro: nothing.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().not_intro = false;
    say(&mut ctl, &mut f, Some((MALAH_U, MALAH)), 20131);
    assert_eq!(ctl.record(CHAIN).unwrap().state, 0);
    assert!(f.sent.is_empty());
}

// ------------------------------------------------------------ §5.5

// Covers: specs/world/quests-act5.md §5.5 r1, §5.5 r2, §5.5 r3, §edge-cases-original-bugs r6
#[test]
fn level_changes() {
    let lvl = |ctl: &mut QuestControl, f: &mut Fake, a, b| {
        ev(
            ctl,
            f,
            EventArgs {
                event: event::CHANGED_LEVEL,
                player: Some(P1),
                target: Some(P1),
                a,
                b,
            },
        );
    };
    // r1: level 112 at state 0 → 1 and the town cleanup.
    let (mut ctl, _) = control();
    let mut f = fake();
    lvl(&mut ctl, &mut f, 111, 112);
    assert_eq!(ctl.record(CHAIN).unwrap().state, 1);
    assert!(x(&mut ctl).nihlathak_gone);
    // Level 113 at state 2: state 3, status 1 to all, flag iterate.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().state = 2;
    lvl(&mut ctl, &mut f, 112, 113);
    let r = ctl.record(CHAIN).unwrap();
    assert_eq!((r.state, r.status), (3, 1));
    assert!(f.flags(P1).get(SLOT, 3) && x(&mut ctl).nihlathak_gone);
    assert_eq!(f.sent, [(P1, hex("5D 21 00 01 0000"))]);
    // State 4, status set: no change, no iterate.
    let (mut ctl, _) = control();
    let mut f = fake();
    let r = ctl.record_mut(CHAIN).unwrap();
    (r.state, r.status) = (4, 3);
    lvl(&mut ctl, &mut f, 113, 114);
    assert_eq!(ctl.record(CHAIN).unwrap().state, 4);
    assert!(f.flags(P1).word(SLOT) == 0 && f.sent.is_empty());
    // r2: leaving Harrogath at state 2.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().state = 2;
    ctl.record_mut(CHAIN).unwrap().guids.add(1);
    lvl(&mut ctl, &mut f, 109, 110);
    let r = ctl.record(CHAIN).unwrap();
    assert_eq!((r.state, r.status), (3, 1));
    assert!(r.guids.0.is_empty() && f.flags(P1).get(SLOT, 3));
    // r1 and r2 both: Harrogath → Arreat Plateau by waypoint at state 0
    // (r1 sets state 1; r2's tail needs state 2: no status).
    let (mut ctl, _) = control();
    let mut f = fake();
    lvl(&mut ctl, &mut f, 109, 112);
    let r = ctl.record(CHAIN).unwrap();
    assert_eq!((r.state, r.status), (1, 0));
    // r3 (edge case 6): the temple before Anya is freed.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().state = 3;
    lvl(&mut ctl, &mut f, 110, 121);
    assert_eq!(ctl.record(CHAIN).unwrap().state, 6);
    let e = x(&mut ctl);
    assert!(e.anya == 2 && e.nihlathak_gone);
    assert!(f.flags(P1).get(SLOT, 14));
    assert_eq!(f.sent, [(P1, hex("5D 21 00 0C 0000"))]);
    // After the freeing (state 5): nothing.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().state = 5;
    lvl(&mut ctl, &mut f, 120, 124);
    assert_eq!(ctl.record(CHAIN).unwrap().state, 5);
    assert_eq!(x(&mut ctl).anya, 0);
}

// ------------------------------------------------------------ §5.6

// Covers: specs/world/quests-act5.md §5.6
#[test]
fn frozen_anya_object() {
    // Dummy 460: init schedules event 7; the event makes object 558.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.pos.insert(DUMMY_U, (20, 30, RoomId(8)));
    anya_dummy_init(&mut ctl, &mut f, DUMMY_U);
    assert_eq!(x(&mut ctl).outside_dummy_guid, 0x60);
    anya_dummy_event(&mut ctl, &mut f, DUMMY_U);
    f.a5_places = vec![Some(FANA_U)];
    f.objects.insert(FANA_U, (0x61, 558, 1));
    anya_dummy_event(&mut ctl, &mut f, DUMMY_U);
    assert_eq!(
        f.log,
        [
            "event7 96 525",
            "place 558 20 30 room 8 [1, 0, 0]",
            "event7 96 525",
            "place 558 20 30 room 8 [1, 0, 0]",
            "flags 97 0x3000000",
        ]
    );
    let e = x(&mut ctl);
    assert!(e.frozen_spawned && e.frozen_guid == 0x61);
    // Spawned: neither init nor event acts again.
    f.log.clear();
    anya_dummy_init(&mut ctl, &mut f, DUMMY_U);
    anya_dummy_event(&mut ctl, &mut f, DUMMY_U);
    assert!(f.log.is_empty());
    // Object 558's init: GUID, status 2 to all below 2.
    frozen_anya_init(&mut ctl, &mut f, FANA_U);
    assert_eq!(x(&mut ctl).frozen_object_guid, 0x61);
    assert_eq!(f.sent, [(P1, hex("5D 21 00 02 0000"))]);
    // Intro: the dummy does nothing.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.record_mut(CHAIN).unwrap().not_intro = false;
    anya_dummy_init(&mut ctl, &mut f, DUMMY_U);
    anya_dummy_event(&mut ctl, &mut f, DUMMY_U);
    assert!(f.log.is_empty() && x(&mut ctl).outside_dummy_guid == 0);
}

// Covers: specs/world/quests-act5.md §5.6, §edge-cases-original-bugs r5
#[test]
fn operate_and_thaw() {
    let (mut ctl, _) = control();
    let mut f = fake();
    f.objects.insert(FANA_U, (0x61, 558, 1));
    f.pos.insert(FANA_U, (50, 60, RoomId(3)));
    x(&mut ctl).frozen_object_guid = 0x61;
    // Without the potion: the statue's text; status 1 → 3.
    ctl.record_mut(CHAIN).unwrap().status = 1;
    assert_eq!(frozen_anya_operate(&mut ctl, &mut f, FANA_U, P1), 0);
    assert_eq!(f.log, ["message 1 97 20131"]);
    assert_eq!(ctl.record(CHAIN).unwrap().status, 3);
    // With it: Anya freed.
    f.log.clear();
    f.sent.clear();
    f.players.insert(
        P2,
        Player {
            guid: 2,
            act: Some(4),
            level: Some(113),
            ..Player::default()
        },
    );
    f.party.insert(P1, vec![P2]);
    f.p(P1).items.push(ICE);
    x(&mut ctl).potions = 1;
    frozen_anya_operate(&mut ctl, &mut f, FANA_U, P1);
    assert_eq!(f.log, ["delete ice ", "room portal RoomId(3) false"]);
    let r = ctl.record(CHAIN).unwrap();
    assert_eq!((r.state, r.status), (5, 5));
    let e = &r.extra.a5.q3;
    assert!(e.anya == 1 && e.potions == 0 && e.thaw_timer && e.thaw_pos == (50, 60));
    for p in [P1, P2] {
        assert!(f.flags(p).get(SLOT, 13) && f.flags(p).get(SLOT, 1));
    }
    assert_eq!(ctl.fx, 16);
    assert_eq!(ctl.timers.len(), 1);
    assert_eq!(ctl.timers[0].period, 1);
    // A holder with 37.1 gets nothing.
    f.p(P1).items.push(ICE);
    f.log.clear();
    frozen_anya_operate(&mut ctl, &mut f, FANA_U, P1);
    assert!(f.log.is_empty() && ctl.timers.len() == 1);
    // The thaw: step 0 frees the statue, step 1 spawns the iced Anya.
    f.a5_crits = vec![Some(ICED_U)];
    f.a5_map_ai = true;
    x(&mut ctl).anya_map_ai = 0x99;
    ctl.update(&mut f);
    ctl.update(&mut f);
    assert_eq!(f.log, ["mode 97 2", "free collision 97"]);
    assert_eq!(x(&mut ctl).thaw_step, 1);
    // Period 1: due once tick passes due (every second update).
    ctl.update(&mut f);
    assert_eq!(f.log.len(), 2);
    ctl.update(&mut f);
    assert_eq!(
        f.log[2..],
        [
            "critical 527 50 60 room 3",
            "leave room 97",
            "map ai 66 0x99"
        ]
    );
    let e = x(&mut ctl);
    assert!(e.thawed_spawned && e.thawed_guid == 0x42 && !e.thaw_timer);
    assert!(e.anya_map_ai_applied);
    assert!(ctl.timers.is_empty());
    // Edge case 5: the object gone at step 1 → 0 forever.
    let (mut ctl, _) = control();
    let mut f = fake();
    x(&mut ctl).thaw_step = 1;
    ctl.add_timer(CHAIN, act5_thaw(), 1).unwrap();
    for _ in 0..5 {
        ctl.update(&mut f);
    }
    assert_eq!(ctl.timers.len(), 1);
    assert_eq!(x(&mut ctl).thaw_step, 2);
    assert!(f.log.is_empty());
}

fn act5_thaw() -> super::super::super::TimerFn {
    super::super::super::TimerFn::Act5(super::super::Timer::Q3(super::Timer::Thaw))
}

// ------------------------------------------------------------ §5.8

// Covers: specs/world/quests-act5.md §5.8
#[test]
fn anya_portal_modes() {
    // In Harrogath: the close counter must pass 5.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.objects.insert(PORTAL_U, (0x62, 189, 1));
    f.players.insert(
        PORTAL_U,
        Player {
            level: Some(109),
            act: Some(4),
            ..Player::default()
        },
    );
    portal_event(&mut ctl, &mut f, PORTAL_U);
    assert_eq!(f.log, ["mode 98 2", "event7 98 525"]);
    for _ in 0..5 {
        portal_event(&mut ctl, &mut f, PORTAL_U);
    }
    assert_eq!(f.object_mode(PORTAL_U), 2);
    portal_event(&mut ctl, &mut f, PORTAL_U);
    assert_eq!(f.object_mode(PORTAL_U), 3);
    assert!(x(&mut ctl).town_may_close);
    portal_event(&mut ctl, &mut f, PORTAL_U);
    assert_eq!(f.object_mode(PORTAL_U), 4);
    // Elsewhere: +0xAD decides.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.objects.insert(PORTAL_U, (0x62, 189, 2));
    f.players.insert(
        PORTAL_U,
        Player {
            level: Some(113),
            act: Some(4),
            ..Player::default()
        },
    );
    portal_event(&mut ctl, &mut f, PORTAL_U);
    assert_eq!(f.object_mode(PORTAL_U), 2);
    x(&mut ctl).outside_may_close = true;
    portal_event(&mut ctl, &mut f, PORTAL_U);
    assert_eq!(f.object_mode(PORTAL_U), 3);
    assert_eq!(x(&mut ctl).town_close_counter, 0);
}

// ------------------------------------------------------------ §5.9

// Covers: specs/world/quests-act5.md §5.8
#[test]
fn map_ai_store_applies_once_when_in_town() {
    let (mut ctl, _) = control();
    let mut f = fake();
    f.a5_map_ai = true;
    // Anya not in town: the handle is only kept.
    map_ai_store(&mut ctl, &mut f, 0x10, false);
    assert!(f.log.is_empty() && x(&mut ctl).anya_map_ai == 0x10);
    // In town with her unit alive: applied once.
    f.monsters.insert(DREHYA_U, (0x41, DREHYA, kind(DREHYA)));
    let e = x(&mut ctl);
    e.anya_in_town = true;
    e.anya_guid = 0x41;
    map_ai_store(&mut ctl, &mut f, 0x11, false);
    assert_eq!(f.log, ["map ai 65 0x11"]);
    assert!(x(&mut ctl).anya_map_ai_applied);
    map_ai_store(&mut ctl, &mut f, 0x12, false);
    assert_eq!(f.log.len(), 1);
    // Nihlathak's store keeps his own handle.
    map_ai_store(&mut ctl, &mut f, 0x20, true);
    assert!(x(&mut ctl).nihlathak_map_ai == 0x20 && x(&mut ctl).anya_map_ai == 0x12);
}

// Covers: specs/world/quests-act5.md §5.9, §edge-cases-original-bugs r10
#[test]
fn town_dummies_and_cleanup() {
    // Dummy 459: Anya spawns there once she is back (+0x84 = 2).
    let (mut ctl, _) = control();
    let mut f = fake();
    f.pos.insert(DUMMY_U, (5, 6, RoomId(1)));
    anya_town_dummy_init(&mut ctl, &mut f, DUMMY_U);
    let e = x(&mut ctl);
    assert!(e.town_dummy_seen && e.town_dummy_guid == 0x60 && e.town_dummy_pos == (5, 6));
    assert!(f.log.is_empty());
    x(&mut ctl).anya = 2;
    x(&mut ctl).anya_map_ai = 0x10;
    f.a5_crits = vec![Some(DREHYA_U)];
    anya_town_dummy_init(&mut ctl, &mut f, DUMMY_U);
    assert_eq!(f.log, ["critical 512 5 6 room 1", "map ai 65 0x10"]);
    assert!(x(&mut ctl).anya_in_town && x(&mut ctl).anya_guid == 0x41);
    // The own sequence function ran (chain 33 open at state 0 → 1).
    assert_eq!(ctl.record(CHAIN).unwrap().state, 1);
    // Chain 34 wants the temple portal (+0x87): event 7 at frame + 12.
    ctl.record_mut(34).unwrap().extra.a5.q4.portal_wanted = true;
    f.frame = 100;
    f.log.clear();
    anya_town_dummy_init(&mut ctl, &mut f, DUMMY_U);
    assert_eq!(f.log, ["event7 96 112"]);
    ctl.record_mut(34).unwrap().extra.a5.q4.portal_wanted = false;
    // Dummy 461: Nihlathak in town unless gone.
    let nd = UnitId(0x70);
    f.pos.insert(nd, (9, 9, RoomId(1)));
    f.log.clear();
    f.a5_crits = vec![Some(NIHL_U)];
    nihlathak_town_dummy_init(&mut ctl, &mut f, nd);
    assert_eq!(f.log, ["critical 514 9 9 room 1"]);
    let e = x(&mut ctl);
    assert!(e.nihlathak_in_town && e.nihlathak_guid == 0x43 && e.nihlathak_dummy_guid == 0x70);
    // The cleanup: Anya stays while +0x84 = 2; Nihlathak gone → node
    // dropped.
    f.log.clear();
    let i = ctl.find(CHAIN).unwrap();
    town_cleanup(&mut ctl, &mut f, i);
    assert_eq!(f.log, ["preset 4 514"]);
    let e = x(&mut ctl);
    assert!(e.anya_in_town && !e.nihlathak_in_town && e.nihlathak_gone);
    // Anya present and not back: she leaves town; absent: node dropped.
    x(&mut ctl).anya = 0;
    f.log.clear();
    town_cleanup(&mut ctl, &mut f, i);
    assert_eq!(f.log, ["leave town 65"]);
    x(&mut ctl).anya_in_town = true;
    f.monsters.remove(&DREHYA_U);
    f.log.clear();
    town_cleanup(&mut ctl, &mut f, i);
    assert_eq!(f.log, ["preset 4 512"]);
    assert!(!x(&mut ctl).anya_in_town);
    // Gone from town: dummy 461 spawns nothing; dummy 462 spawns the
    // boss (superunique 60) once.
    f.log.clear();
    nihlathak_town_dummy_init(&mut ctl, &mut f, nd);
    assert!(f.log.is_empty());
    f.a5_crits = vec![Some(NIHL_U)];
    nihlathak_temple_dummy_init(&mut ctl, &mut f, nd);
    nihlathak_temple_dummy_init(&mut ctl, &mut f, nd);
    assert_eq!(f.log, ["superunique 60 9 9 room 1"]);
    assert!(x(&mut ctl).boss_spawned && x(&mut ctl).boss_guid == 0x43);
    // Not gone yet: no boss.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.pos.insert(nd, (9, 9, RoomId(1)));
    nihlathak_temple_dummy_init(&mut ctl, &mut f, nd);
    assert!(f.log.is_empty());
    // Dummy 461's event 7 (`0x00589540`): "back" = a monster with GUID
    // +0x9C exists; then `0x00589340` again. +0x9C = 0 here: nothing.
    nihlathak_dummy_event(&mut ctl, &mut f, nd);
    assert!(f.log.is_empty());
    x(&mut ctl).nihlathak_guid = 0x43;
    f.monsters
        .insert(NIHL_U, (0x43, NIHLATHAK, kind(NIHLATHAK)));
    nihlathak_dummy_event(&mut ctl, &mut f, nd);
    assert_eq!(f.log, ["kill in town 67"]);
}

// ------------------------------------------------------------ §5.10

// Covers: specs/world/quests-act5.md §5.10
#[test]
fn start_join_leave() {
    let start = |b: &[u8], ice: bool, intro: bool| {
        let (mut ctl, _) = control();
        let mut f = fake();
        bits(&mut f, 0, b);
        if ice {
            f.p(P1).items.push(ICE);
        }
        ctl.record_mut(CHAIN).unwrap().not_intro = !intro;
        ev(
            &mut ctl,
            &mut f,
            EventArgs {
                event: event::PLAYER_STARTED_GAME,
                player: Some(P1),
                target: Some(P1),
                ..EventArgs::default()
            },
        );
        let r = ctl.record(CHAIN).unwrap();
        let e = &r.extra.a5.q3;
        (r.state, r.status, e.potions, e.anya, e.nihlathak_gone)
    };
    assert_eq!(start(&[0], true, false), (0, 0, 1, 2, true));
    assert_eq!(start(&[15], false, false), (0, 0, 0, 2, true));
    assert_eq!(start(&[3, 2], false, true), (0, 0, 0, 0, false));
    assert_eq!(start(&[3, 2], false, false), (3, 1, 0, 0, false));
    assert_eq!(start(&[2], false, false), (2, 1, 0, 0, false));
    assert_eq!(start(&[2], true, false), (3, 4, 1, 0, false));
    // Join: a potion counts; once the prison is over, 38.14 for players
    // lacking 37.0, 37.1, 37.6.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.p(P1).items.push(ICE);
    let join = EventArgs {
        event: event::PLAYER_JOINED_GAME,
        player: Some(P1),
        target: Some(P1),
        ..EventArgs::default()
    };
    ev(&mut ctl, &mut f, join);
    assert_eq!(x(&mut ctl).potions, 1);
    assert!(!f.flags(P1).get(38, 14));
    ctl.record_mut(CHAIN).unwrap().state = 6;
    ev(&mut ctl, &mut f, join);
    assert!(f.flags(P1).get(38, 14));
    let mut f = fake();
    bits(&mut f, 0, &[6]);
    ev(&mut ctl, &mut f, join);
    assert!(!f.flags(P1).get(38, 14));
    ctl.record_mut(34).unwrap().not_intro = false;
    let mut f = fake();
    ev(&mut ctl, &mut f, join);
    assert!(!f.flags(P1).get(38, 14));
    // Leave: both lists; a potion leaves with its holder.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.p(P1).items.push(ICE);
    x(&mut ctl).potions = 2;
    x(&mut ctl).guids.add(1);
    ctl.record_mut(CHAIN).unwrap().guids.add(1);
    ev(
        &mut ctl,
        &mut f,
        EventArgs {
            event: event::PLAYER_LEAVES_GAME,
            ..join
        },
    );
    assert_eq!(x(&mut ctl).potions, 1);
    assert!(x(&mut ctl).guids.0.is_empty());
    assert!(ctl.record(CHAIN).unwrap().guids.0.is_empty());
}

// ------------------------------------------------------------ §5.11

// Covers: specs/world/quests-act5.md §5.11, §edge-cases-original-bugs r4
#[test]
fn status_function() {
    let (mut ctl, _) = control();
    let mut f = fake();
    // Test vector: 37.1, 37.8, not 37.9 → 6.
    assert_eq!(status(&ctl, &mut f, &fx(&[1, 8])), Some(6));
    assert_eq!(status(&ctl, &mut f, &fx(&[1])), Some(5));
    assert_eq!(status(&ctl, &mut f, &fx(&[13, 9])), Some(5));
    assert_eq!(status(&ctl, &mut f, &fx(&[1, 8, 9])), Some(13));
    assert_eq!(status(&ctl, &mut f, &fx(&[0, 1])), Some(0));
    assert_eq!(status(&ctl, &mut f, &fx(&[14])), Some(12));
    ctl.record_mut(CHAIN).unwrap().status = 2;
    assert_eq!(status(&ctl, &mut f, &fx(&[])), Some(2));
    // Edge case 4: a party member's potion never gives 4.
    f.players.insert(
        P2,
        Player {
            guid: 2,
            items: vec![ICE],
            ..Player::default()
        },
    );
    f.party.insert(P1, vec![P2]);
    assert_eq!(status(&ctl, &mut f, &fx(&[])), Some(2));
    f.p(P1).items.push(ICE);
    assert_eq!(status(&ctl, &mut f, &fx(&[14])), Some(4));
    f.p(P1).items.clear();
    ctl.record_mut(CHAIN).unwrap().state = 5;
    assert_eq!(status(&ctl, &mut f, &fx(&[])), Some(0));
    ctl.record_mut(CHAIN).unwrap().not_intro = false;
    assert_eq!(status(&ctl, &mut f, &fx(&[14])), Some(0));
}

// Covers: specs/world/quests-act5.md §5.1
#[test]
fn extra_data_at_init() {
    // Anya frozen (0), Nihlathak in town, no thaw step, the GUID list
    // empty; init clears stale data.
    let (mut ctl, _) = control();
    let r = ctl.record_mut(33).unwrap();
    assert_eq!(r.extra.a5.q3, super::Extra::default());
    assert_eq!(r.extra.a5.q3.anya, 0);
    r.extra.a5.q3.anya = 2;
    r.extra.a5.q3.guids.add(7);
    super::init(r);
    assert_eq!(r.extra.a5.q3, super::Extra::default());
}

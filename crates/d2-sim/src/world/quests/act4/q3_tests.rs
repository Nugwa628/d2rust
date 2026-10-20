// Spec: specs/world/quests-act4.md §4, §7, §8 (Test vectors, Randomness, Edge cases)
//! A4Q3 Hell's Forge callback by callback, the Hellforge and its drops,
//! on the quests' fake world.

use super::super::super::tests::*;
use super::super::super::*;
use super::*;
use crate::rng::Seed;

const P3: UnitId = UnitId(3);
const CAIN_U: UnitId = UnitId(0x23);
const TYRAEL_U: UnitId = UnitId(0x20);
const FORGE: UnitId = UnitId(0x40);
const HEPH_U: UnitId = UnitId(0x41);

/// Three players in Act IV, Cain, Tyrael, the Hellforge (mode `mode`).
fn fake(mode: i32) -> Fake {
    let mut f = Fake::new();
    for g in 1..=3 {
        f.players.insert(
            UnitId(g),
            Player {
                guid: g,
                act: Some(3),
                level: Some(103),
                ..Player::default()
            },
        );
    }
    for (u, class) in [(CAIN_U, CAIN4), (TYRAEL_U, npc::TYRAEL2), (HEPH_U, 409)] {
        let kind = UnitKind::Monster {
            class: u32::from(class),
            superunique: None,
            owner: None,
        };
        f.monsters.insert(u, (u.0, class, kind));
    }
    f.objects.insert(FORGE, (FORGE.0, HELLFORGE, mode));
    f
}

fn idx(ctl: &QuestControl) -> usize {
    ctl.find(CHAIN).unwrap()
}

fn chat(ctl: &mut QuestControl, f: &mut Fake, p: UnitId) -> TextList {
    let mut list = TextList::new();
    ctl.npc_activate(f, p, CAIN_U, &mut list);
    list
}

fn msg(ctl: &mut QuestControl, f: &mut Fake, p: UnitId, n: UnitId, m: u16) {
    let mut b = vec![0x31];
    b.extend(f.guid(n).to_le_bytes());
    b.extend(m.to_le_bytes());
    b.extend([0, 0]);
    assert_eq!(ctl.quest_message(f, p, &b), 0);
}

fn drops(f: &Fake) -> Vec<String> {
    f.log
        .iter()
        .filter_map(|l| l.strip_prefix("drop "))
        .map(|l| l[..3].to_string())
        .collect()
}

// Covers: specs/world/quests-act4.md §4.3 text, §4.3 r1, §4.3 r2, §4.3 r3, §4.3 r4
#[test]
fn cain_lines() {
    let (mut ctl, _) = control();
    let mut f = fake(0);
    let i = idx(&ctl);
    // State 0: nothing.
    assert_eq!(chat(&mut ctl, &mut f, P1), []);
    // State 1: soulstone held → 678; not held → 679.
    ctl.records[i].state = 1;
    assert_eq!(chat(&mut ctl, &mut f, P1), [(679, 0)]);
    f.p(P1).items.push(SOULSTONE);
    assert_eq!(chat(&mut ctl, &mut f, P1), [(678, 0)]);
    // +0x1C or status ≥ 3: no 679.
    ctl.records[i].extra.a4.q3.gave_stone = true;
    assert_eq!(chat(&mut ctl, &mut f, P2), []);
    ctl.records[i].extra.a4.q3.gave_stone = false;
    ctl.records[i].status = 3;
    assert_eq!(chat(&mut ctl, &mut f, P2), []);
    // Other states: 679 only without the stone.
    ctl.records[i].status = 2;
    ctl.records[i].state = 3;
    assert_eq!(chat(&mut ctl, &mut f, P2), [(679, 0)]);
    assert_eq!(chat(&mut ctl, &mut f, P1), []);
    // State ≥ 4 without 27.13: nothing; with it, the state rules.
    ctl.records[i].state = 4;
    assert_eq!(chat(&mut ctl, &mut f, P2), []);
    f.p(P2).quests.flags[0].set(27, 13);
    assert_eq!(chat(&mut ctl, &mut f, P2), [(679, 0)]);
    // 27.1 → table state 2; GUID listed → 3.
    f.p(P2).quests.flags[0].set(27, 1);
    assert_eq!(chat(&mut ctl, &mut f, P2), [(680, 0)]);
    ctl.records[i].guids.add(3);
    assert_eq!(chat(&mut ctl, &mut f, P3), [(680, 2)]);
    // 27.0 or intro: nothing.
    f.p(P3).quests.flags[0].set(27, 0);
    ctl.records[i].guids.remove(3);
    assert_eq!(chat(&mut ctl, &mut f, P3), []);
    ctl.records[i].not_intro = false;
    assert_eq!(chat(&mut ctl, &mut f, P2), [(680, 0)]); // 27.1 first
    f.p(P2).quests.flags[0].clear(27, 1);
    assert_eq!(chat(&mut ctl, &mut f, P2), []);
    // Tyrael has no Hell's Forge text.
    let mut list = TextList::new();
    ctl.npc_activate(&mut f, P2, TYRAEL_U, &mut list);
    assert!(!list.iter().any(|l| (678..=680).contains(&l.0)));
}

// Covers: specs/world/quests-act4.md §4.3 text
#[test]
fn wants_to_talk() {
    let (mut ctl, _) = control();
    let mut f = fake(0);
    let i = idx(&ctl);
    let act = |ctl: &QuestControl, f: &mut Fake, p, n| active(ctl, f, i, p, n, 0x005B_5EE0);
    assert!(!act(&ctl, &mut f, P1, CAIN4));
    ctl.records[i].state = 1;
    assert!(act(&ctl, &mut f, P1, CAIN4));
    assert!(!act(&ctl, &mut f, P1, npc::TYRAEL2));
    f.p(P1).quests.flags[0].set(27, 1);
    assert!(!act(&ctl, &mut f, P1, CAIN4));
    f.p(P2).quests.flags[0].set(27, 0);
    assert!(!act(&ctl, &mut f, P2, CAIN4));
    ctl.records[i].extra.a4.q3.gave_stone = true;
    assert!(!act(&ctl, &mut f, P3, CAIN4));
    ctl.records[i].extra.a4.q3.gave_stone = false;
    ctl.records[i].not_intro = false;
    assert!(!act(&ctl, &mut f, P3, CAIN4));
}

// Covers: specs/world/quests-act4.md §4.4, §4.5, §edge-cases-original-bugs r7
#[test]
fn cain_messages_and_chat_end() {
    let (mut ctl, _) = control();
    let mut f = fake(0);
    let i = idx(&ctl);
    ctl.records[i].state = 1;
    // 678: +0x02, state 2, refresh.
    msg(&mut ctl, &mut f, P1, CAIN_U, 678);
    assert!(ctl.records[i].extra.a4.q3.started && ctl.records[i].state == 2);
    assert_eq!(f.sent_ids(), [0x27, 0x29]);
    // 678 from Tyrael: nothing.
    ctl.records[i].state = 1;
    msg(&mut ctl, &mut f, P1, TYRAEL_U, 678);
    assert_eq!(ctl.records[i].state, 1);
    // 679: the soulstone, state 2, +0x1C.
    f.log.clear();
    msg(&mut ctl, &mut f, P2, CAIN_U, 679);
    assert_eq!(f.log[0], "reward mss  0 2");
    assert!(f.players[&P2].items.contains(&SOULSTONE));
    let e = &ctl.records[i].extra.a4.q3;
    assert!(e.gave_stone && e.started && ctl.records[i].state == 2);
    // 679 at state 3 keeps the state.
    ctl.records[i].state = 3;
    msg(&mut ctl, &mut f, P2, CAIN_U, 679);
    assert_eq!(ctl.records[i].state, 3);
    // Chat end: status 4 (+0x1C) with the flags byte as it was, then
    // flags := 0, +0x02 := 0, callback removed, flag iterate (state 3:
    // 27.3).
    ctl.records[i].flags = 5;
    f.sent.clear();
    ctl.npc_deactivate(&mut f, P1, CAIN_U);
    let r = &ctl.records[i];
    assert!(r.status == 4 && r.flags == 0 && !r.extra.a4.q3.started);
    assert!(!r.has_callback(event::NPC_DEACTIVATE));
    assert_eq!(f.sent.len(), 3);
    assert_eq!(f.sent[0], (P1, hex("5D 18 05 04 0000")));
    assert!(f.flags(P1).get(27, 3) && f.flags(P3).get(27, 3));
    // Never re-installed: a later chat end does nothing.
    ctl.records[i].extra.a4.q3.started = true;
    f.sent.clear();
    ctl.npc_deactivate(&mut f, P1, CAIN_U);
    assert!(f.sent.is_empty());
    // Without +0x1C: status 1; flag iterate at state 2: 27.2, or 27.5
    // with +0x1C.
    let (mut ctl, _) = control();
    let mut f = fake(0);
    let i = idx(&ctl);
    msg(&mut ctl, &mut f, P1, CAIN_U, 678);
    ctl.npc_deactivate(&mut f, P1, CAIN_U);
    assert_eq!(ctl.records[i].status, 1);
    assert!(f.flags(P2).get(27, 2) && !f.flags(P2).get(27, 5));
    ctl.records[i].extra.a4.q3.gave_stone = true;
    f.p(P3).quests.flags[0].set(27, 1);
    flag_iterate(&ctl, &mut f, i);
    assert!(f.flags(P1).get(27, 5) && !f.flags(P3).get(27, 5));
}

// Covers: specs/world/quests-act4.md §4.4, §1.3
#[test]
fn cain_reward() {
    let (mut ctl, _) = control();
    let mut f = fake(0);
    let i = idx(&ctl);
    ctl.records[i].state = 4;
    ctl.records[i].flags = 3;
    // Without 27.1: nothing.
    msg(&mut ctl, &mut f, P1, CAIN_U, 680);
    assert!(f.sent.is_empty() && ctl.records[i].state == 4);
    // With 27.1 and 27.13: status 13 (silent), state 5, Terror's End
    // starts; 27.0 set, 27.1 cleared, progress kept; GUID listed.
    for b in [1, 13, 2] {
        f.p(P1).quests.flags[0].set(27, b);
    }
    msg(&mut ctl, &mut f, P1, CAIN_U, 680);
    let r = &ctl.records[i];
    assert_eq!((r.state, r.status, r.flags), (5, 13, 0));
    assert_eq!(ctl.record(23).unwrap().state, 1);
    let fl = f.flags(P1);
    assert!(fl.get(27, 0) && !fl.get(27, 1) && fl.get(27, 2));
    assert!(ctl.records[i].guids.contains(1));
    assert_eq!(f.sent_ids(), [0x27, 0x29]);
    // 27.1 alone, state 5: the reward only.
    f.p(P2).quests.flags[0].set(27, 1);
    msg(&mut ctl, &mut f, P2, CAIN_U, 680);
    assert!(f.flags(P2).get(27, 0) && ctl.records[i].guids.contains(2));
}

// Covers: specs/world/quests-act4.md §4.5, §4.1
#[test]
fn pickup_level_change_and_hammer_count() {
    let (mut ctl, _) = control();
    let mut f = fake(0);
    let i = idx(&ctl);
    let hammer = UnitId(0x70);
    f.chains.insert(hammer, QuestChain(vec![CHAIN]));
    f.item_codes.insert(hammer, HAMMER);
    // Pick-up: state 0 → 1.
    ctl.item_event(&mut f, event::ITEM_PICKED_UP, P1, hammer);
    assert_eq!(ctl.records[i].state, 1);
    // Not at state ≠ 0, status 13, or with 27.13.
    for setup in 0..3 {
        let (mut ctl, _) = control();
        let mut f = fake(0);
        f.chains.insert(hammer, QuestChain(vec![CHAIN]));
        match setup {
            0 => ctl.record_mut(24).unwrap().status = 13,
            1 => f.p(P1).quests.flags[0].set(27, 13),
            _ => ctl.record_mut(24).unwrap().not_intro = false,
        }
        ctl.item_event(&mut f, event::ITEM_PICKED_UP, P1, hammer);
        assert_eq!(ctl.record(24).unwrap().state, 0, "setup {setup}");
    }
    // Event 3 from town at state 2: quick remove, state 3, 27.3.
    ctl.records[i].state = 2;
    ctl.records[i].guids.add(1);
    ctl.changed_level(&mut f, P1, 103, 104);
    assert!(ctl.records[i].state == 3 && !ctl.records[i].guids.contains(1));
    assert!(f.flags(P2).get(27, 3));
    // A credited player leaving: no state change.
    ctl.records[i].state = 2;
    f.p(P3).quests.flags[0].set(27, 1);
    ctl.changed_level(&mut f, P3, 103, 104);
    assert_eq!(ctl.records[i].state, 2);
    // Event 14 (holding hfh) +1; event 9 (an hfh item) −1; event 10.
    f.p(P2).items.push(HAMMER);
    ctl.player_enters(&mut f, P2, 1).unwrap(); // first entry: no event 14
    ctl.player_enters(&mut f, P2, 1).unwrap();
    assert_eq!(ctl.records[i].extra.a4.q3.hammers, 1);
    ctl.records[i].guids.add(2);
    let args = EventArgs {
        event: event::PLAYER_DROPPED_WITH_QUEST_ITEM,
        target: Some(hammer),
        player: Some(P2),
        ..EventArgs::default()
    };
    assert!(callback(&mut ctl, &mut f, i, args, None));
    assert_eq!(ctl.records[i].extra.a4.q3.hammers, 0);
    let args = EventArgs {
        target: Some(UnitId(0x71)),
        ..args
    };
    assert!(callback(&mut ctl, &mut f, i, args, None));
    assert_eq!(ctl.records[i].extra.a4.q3.hammers, 0);
    ctl.player_leaves(&mut f, P2);
    assert!(!ctl.records[i].guids.contains(2));
}

// Covers: specs/world/quests-act4.md §4.5
#[test]
fn game_start_restores() {
    let start = |bits: &[u8], items: &[[u8; 4]]| {
        let (mut ctl, _) = control();
        let mut f = fake(0);
        for &b in bits {
            f.p(P1).quests.flags[0].set(27, b);
        }
        f.p(P1).items.extend_from_slice(items);
        let i = idx(&ctl);
        let args = EventArgs {
            event: event::PLAYER_STARTED_GAME,
            player: Some(P1),
            ..EventArgs::default()
        };
        assert!(callback(&mut ctl, &mut f, i, args, None));
        let r = &ctl.records[i];
        (
            r.state,
            r.status,
            r.extra.a4.q3.gave_stone,
            r.extra.a4.q3.hammers,
        )
    };
    assert_eq!(start(&[5, 3, 2], &[SOULSTONE]), (2, 4, true, 0));
    assert_eq!(start(&[3, 2], &[SOULSTONE]), (3, 1, false, 0));
    assert_eq!(start(&[2], &[SOULSTONE, HAMMER]), (2, 1, false, 1));
    assert_eq!(start(&[], &[SOULSTONE]), (0, 0, false, 0));
    // Without the soulstone: state 1, whatever the bits.
    assert_eq!(start(&[5, 2], &[]), (1, 0, false, 0));
    // 27.0 / 27.15: only the hammer count.
    assert_eq!(start(&[0], &[HAMMER]), (0, 0, false, 1));
    assert_eq!(start(&[15], &[]), (0, 0, false, 0));
}

// Covers: specs/world/quests-act4.md §4.6
#[test]
fn forge_init_modes() {
    let (mut ctl, _) = control();
    let mut f = fake(0);
    ctl.record_mut(24).unwrap().extra.a4.q3.forge_mode = 2;
    forge_init(&mut ctl, &mut f, FORGE);
    assert_eq!(f.log, ["mode 64 2"]);
    assert_eq!(ctl.record(24).unwrap().status, 2);
    assert_eq!(f.sent.len(), 3);
    // Status 13, 2, 3 kept.
    for s in [13, 3] {
        ctl.record_mut(24).unwrap().status = s;
        f.sent.clear();
        forge_init(&mut ctl, &mut f, FORGE);
        assert!(f.sent.is_empty() && ctl.record(24).unwrap().status == s);
    }
    // Intro or no record: mode 3.
    ctl.record_mut(24).unwrap().not_intro = false;
    f.log.clear();
    forge_init(&mut ctl, &mut f, FORGE);
    assert_eq!(f.log, ["mode 64 3"]);
    ctl.records.retain(|r| r.chain != 24);
    f.log.clear();
    forge_init(&mut ctl, &mut f, FORGE);
    assert_eq!(f.log, ["mode 64 3"]);
}

// Covers: specs/world/quests-act4.md §4.6
#[test]
fn forge_takes_the_soulstone() {
    let (mut ctl, _) = control();
    let mut f = fake(0);
    f.frame = 100;
    let i = idx(&ctl);
    // Not holding it: sound 19, state 0 → 1.
    forge_operate(&mut ctl, &mut f, FORGE, P1);
    assert_eq!(f.log, ["sound 1 19"]);
    assert_eq!(ctl.records[i].state, 1);
    // Holding it: mode 1, end event at f + 22, +0x00 := 2, status 3,
    // soulstone deleted.
    f.log.clear();
    f.p(P1).items.push(SOULSTONE);
    ctl.records[i].flags = 4;
    forge_operate(&mut ctl, &mut f, FORGE, P1);
    assert_eq!(f.log, ["mode 64 1", "event1 64 122", "delete mss "]);
    let r = &ctl.records[i];
    assert_eq!((r.status, r.flags, r.extra.a4.q3.forge_mode), (3, 0, 2));
    assert_eq!(f.sent.len(), 3);
    // Mode 1: nothing.
    f.log.clear();
    forge_operate(&mut ctl, &mut f, FORGE, P2);
    assert!(f.log.is_empty());
    // A player with 27.0 or 27.1: sound 19 in any mode.
    f.p(P3).quests.flags[0].set(27, 1);
    forge_operate(&mut ctl, &mut f, FORGE, P3);
    assert_eq!(f.log, ["sound 3 19"]);
    // No record: nothing.
    ctl.records.retain(|r| r.chain != 24);
    f.log.clear();
    forge_operate(&mut ctl, &mut f, FORGE, P1);
    assert!(f.log.is_empty());
}

// Covers: specs/world/quests-act4.md §4.6, §7, §edge-cases-original-bugs r5
#[test]
fn forge_three_hits_vector() {
    // Test vector: mode 2, three valid hits by two players.
    let (mut ctl, _) = control();
    let mut f = fake(2);
    f.frame = 500;
    let i = idx(&ctl);
    ctl.records[i].state = 3;
    for p in [P1, P2] {
        f.p(p).items.extend([HAMMER, SOULSTONE]);
        f.a4_wield.insert(p, HAMMER);
    }
    f.party.insert(P2, vec![P1, P2]);
    // P3 holds a hammer but wields something else: refused.
    f.p(P3).items.push(HAMMER);
    f.a4_wield.insert(P3, *b"axe ");
    forge_operate(&mut ctl, &mut f, FORGE, P3);
    // P2 not wielding: refused.
    f.a4_wield.remove(&P2);
    forge_operate(&mut ctl, &mut f, FORGE, P2);
    f.a4_wield.insert(P2, HAMMER);
    assert_eq!(f.log, ["sound 3 19", "sound 2 19"]);
    assert_eq!(ctl.records[i].extra.a4.q3.hits, 0);
    forge_operate(&mut ctl, &mut f, FORGE, P1);
    forge_operate(&mut ctl, &mut f, FORGE, P1);
    assert_eq!(ctl.records[i].extra.a4.q3.hits, 2);
    assert_eq!(f.objects[&FORGE].2, 2);
    f.log.clear();
    // The third hit, by P2.
    forge_operate(&mut ctl, &mut f, FORGE, P2);
    assert_eq!(
        f.log,
        [
            "mode 64 3",
            "delete hfh ",
            "delete hfh ",
            "delete mss ",
            "event7 64 522",
        ]
    );
    let r = &ctl.records[i];
    let e = &r.extra.a4.q3;
    assert_eq!((r.state, r.status, r.flags), (4, 13, 0));
    assert_eq!(
        (
            e.forge_mode,
            e.smashed,
            e.gems_pending,
            e.tier,
            e.sets,
            e.hits
        ),
        (3, true, true, 4, 2, 3)
    );
    for p in [P1, P2] {
        let fl = f.flags(p);
        assert!(fl.get(27, 13) && fl.get(27, 1) && !fl.get(27, 14));
    }
    assert!(f.players[&P2].items.contains(&SOULSTONE)); // the hitter keeps it
    assert!(f.flags(P3).get(27, 14) && !f.flags(P3).get(27, 13));
    // Status 13 to all, the completion flag to P3, FX 14 (0x28, 89 0E).
    let to_p3: Vec<Vec<u8>> = f
        .sent
        .iter()
        .filter(|m| m.0 == P3)
        .map(|m| m.1.clone())
        .collect();
    // (the default status rule gives P3, without 27.13, status 12).
    assert_eq!(to_p3[0], hex("5D 18 00 0C 0000"));
    assert_eq!(to_p3[1], hex("5D 18 00 0C 0000"));
    assert_eq!(to_p3[2][0], 0x28);
    assert_eq!(to_p3[3], hex("89 0E"));
    assert_eq!(ctl.fx, 14);
    // Mode 3: further hits do nothing.
    f.log.clear();
    forge_operate(&mut ctl, &mut f, FORGE, P1); // credited: refused
    f.a4_wield.insert(P3, HAMMER);
    forge_operate(&mut ctl, &mut f, FORGE, P3); // mode 3: nothing
    assert_eq!(f.log, ["sound 1 19"]);
    assert_eq!(ctl.records[i].extra.a4.q3.hits, 3);
}

/// Runs the forge's event 7 until no further event is scheduled.
fn run_drops(ctl: &mut QuestControl, f: &mut Fake) {
    for _ in 0..8 {
        f.log.retain(|l| !l.starts_with("event7"));
        forge_event(ctl, f, FORGE);
        if !f.log.iter().any(|l| l.starts_with("event7")) {
            return;
        }
    }
}

fn smashed(ctl: &mut QuestControl, sets: i32) {
    let e = &mut ctl.record_mut(24).unwrap().extra.a4.q3;
    (e.smashed, e.gems_pending, e.tier, e.sets) = (true, true, 4, sets);
    ctl.seed = Seed::new(12345, 666);
}

// Covers: specs/world/quests-act4.md §4.7 text, §4.7 r1, §4.7 r2, §4.7 r3, §4.7 r4, §edge-cases-original-bugs r16
#[test]
fn gem_and_rune_vector_one_player() {
    let (mut ctl, _) = control();
    let mut f = fake(3);
    f.frame = 40;
    smashed(&mut ctl, 1);
    let mut seed = Seed::new(12345, 666);
    let los: Vec<u32> = (0..5).map(|_| seed.step()).collect();
    assert_eq!(
        los,
        [22752887, 2337785264, 1617882871, 4008788125, 3081094168]
    );
    forge_event(&mut ctl, &mut f, FORGE);
    assert_eq!(f.log, ["mode 64 4", "drop gpy  2", "event7 64 60"]);
    run_drops(&mut ctl, &mut f);
    assert_eq!(drops(&f), ["gpy", "gly", "gly", "sku", "r10"]);
    let e = &ctl.record(24).unwrap().extra.a4.q3;
    assert!(!e.gems_pending && e.tier == 0);
    assert_eq!(ctl.seed, seed);
    // A further event: mode 4 only.
    f.log.clear();
    forge_event(&mut ctl, &mut f, FORGE);
    assert_eq!(f.log, ["mode 64 4"]);
}

// Covers: specs/world/quests-act4.md §4.7 r3, §4.7 r4, §7
#[test]
fn gem_and_rune_vector_two_players() {
    let (mut ctl, _) = control();
    let mut f = fake(3);
    smashed(&mut ctl, 2);
    run_drops(&mut ctl, &mut f);
    assert_eq!(
        drops(&f),
        ["gpy", "gpy", "gly", "skl", "glw", "gly", "sku", "gsr", "r09"]
    );
}

// Covers: specs/world/quests-act4.md §4.7 r4
#[test]
fn rune_tables_by_difficulty_and_classic() {
    // Hell: index 9 → r24; nightmare: r21.
    for (d, want) in [(2, "r24"), (1, "r21")] {
        let (mut ctl, _) = control();
        let mut f = fake(3);
        f.difficulty = d;
        smashed(&mut ctl, 1);
        run_drops(&mut ctl, &mut f);
        assert_eq!(drops(&f).last().unwrap(), want);
    }
    assert_eq!(rune_code(0, 0), *b"r01 ");
    assert_eq!(rune_code(0, 10), *b"r11 ");
    assert_eq!(rune_code(1, 0), *b"r12 ");
    assert_eq!(rune_code(1, 10), *b"r22 ");
    assert_eq!(rune_code(2, 0), *b"r15 ");
    assert_eq!(rune_code(2, 10), *b"r25 ");
    // Classic: four gems, no rune, one step fewer.
    let (mut ctl, _) = control();
    let mut f = fake(3);
    f.expansion = false;
    smashed(&mut ctl, 1);
    run_drops(&mut ctl, &mut f);
    assert_eq!(drops(&f), ["gpy", "gly", "gly", "sku"]);
    let mut seed = Seed::new(12345, 666);
    (0..4).for_each(|_| {
        seed.step();
    });
    assert_eq!(ctl.seed, seed);
}

// Covers: specs/world/quests-act4.md §4.7 r2, §4.7 r3
#[test]
fn gem_round_guards() {
    // No sets, or no drops pending: nothing but the mode; no draw.
    for (pending, sets) in [(false, 1), (true, 0)] {
        let (mut ctl, _) = control();
        let mut f = fake(3);
        smashed(&mut ctl, sets);
        ctl.record_mut(24).unwrap().extra.a4.q3.gems_pending = pending;
        forge_event(&mut ctl, &mut f, FORGE);
        assert_eq!(f.log, ["mode 64 4"]);
        assert_eq!(ctl.seed, Seed::new(12345, 666));
    }
    // A tier outside 1–4 stops the whole call before any draw.
    let (mut ctl, _) = control();
    let mut f = fake(3);
    smashed(&mut ctl, 1);
    ctl.record_mut(24).unwrap().extra.a4.q3.tier = 5;
    forge_event(&mut ctl, &mut f, FORGE);
    assert_eq!(f.log, ["mode 64 4"]);
    assert_eq!(ctl.seed, Seed::new(12345, 666));
    // Not smashed: no mode change.
    let (mut ctl, _) = control();
    let mut f = fake(3);
    smashed(&mut ctl, 1);
    ctl.record_mut(24).unwrap().extra.a4.q3.smashed = false;
    forge_event(&mut ctl, &mut f, FORGE);
    assert_eq!(f.log[0], "drop gpy  2");
    // Through the object-event dispatcher (class 0x178).
    let (mut ctl, _) = control();
    let mut f = fake(3);
    smashed(&mut ctl, 1);
    object_event(&mut ctl, &mut f, FORGE, HELLFORGE);
    assert_eq!(f.log, ["mode 64 4", "drop gpy  2", "event7 64 20"]);
}

// Covers: specs/world/quests-act4.md §4.8, §8, §edge-cases-original-bugs r8
#[test]
fn hephasto_drops_a_hammer_per_kill() {
    let (mut ctl, _) = control();
    let mut f = fake(0);
    f.chains.insert(HEPH_U, QuestChain::default());
    assert!(link_hephasto(&mut ctl, &mut f, HEPH_U));
    for n in 1..=2 {
        ctl.monster_killed(&mut f, HEPH_U, Some(P1));
        assert_eq!(ctl.record(24).unwrap().extra.a4.q3.hammers, n);
    }
    assert_eq!(f.log, ["drop hfh  7", "drop hfh  7"]);
    assert!(ctl.record(24).unwrap().has_callback(event::MONSTER_KILLED));
    // Intro: nothing.
    ctl.record_mut(24).unwrap().not_intro = false;
    ctl.monster_killed(&mut f, HEPH_U, Some(P1));
    assert_eq!(f.log.len(), 2);
    // The hammer item's link; the Mephisto stub does nothing.
    let hfh = UnitId(0x72);
    f.chains.insert(hfh, QuestChain::default());
    assert!(link_hammer(&mut ctl, &mut f, hfh));
    assert_eq!(f.chains[&hfh].0, [24]);
    mephisto_killed();
}

// Covers: specs/world/quests-act4.md §edge-cases-original-bugs r6
#[test]
fn a_round_without_gems_ends_all_drops() {
    // Every drop of the first round fails: count 0 → stop, no event 7
    // rescheduled, +0x03 stays 1, the tier is kept and no rune follows.
    let (mut ctl, _) = control();
    let mut f = fake(3);
    f.drop_at_fails = true;
    smashed(&mut ctl, 2);
    forge_event(&mut ctl, &mut f, FORGE);
    assert_eq!(drops(&f), ["gpy", "gpy"]);
    assert!(!f.log.iter().any(|l| l.starts_with("event7")));
    let e = &ctl.record(24).unwrap().extra.a4.q3;
    assert!(e.gems_pending);
    assert_eq!(e.tier, 4);
}

// Covers: specs/world/quests-act4.md §3.1, §4.2, §5.1
#[test]
fn extra_data_zeroed_at_init() {
    // The three records' extra data start at 0 (the Hellforge mode to
    // restore 0, no hits, no sets, no timers) and init clears stale data.
    let (mut ctl, _) = control();
    for chain in [22, 23, 24] {
        let r = ctl.record(chain).unwrap();
        assert_eq!(r.extra.a4.q1, super::super::q1::Extra::default());
        assert_eq!(r.extra.a4.q2, super::super::q2::Extra::default());
        assert_eq!(r.extra.a4.q3, Extra::default());
    }
    let r = ctl.record_mut(24).unwrap();
    (
        r.extra.a4.q3.hits,
        r.extra.a4.q3.sets,
        r.extra.a4.q3.smashed,
    ) = (3, 2, true);
    super::super::init(r);
    assert_eq!(r.extra.a4.q3, Extra::default());
}

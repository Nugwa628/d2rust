//! NPC ids through the host frame: the real `NpcControl` and
//! `QuestControl` on the seam fake ([`super::fake`]).
//!
//! Part of the world handler's test suite: covers talk, chat, menu actions,
//! Cain's identify, hire refusals and the resurrect message, which
//! `world::handler` routes into the sim's NPC module.

use d2_sim::units::UnitId;
use d2_sim::world::npc::{self, talk, InteractionList, InvEntry, NpcWorld, Place};

use super::fake::*;
use super::*;

fn npc_msg(id: u8, ty: u32, guid: u32) -> Vec<u8> {
    let mut m = vec![id];
    m.extend_from_slice(&ty.to_le_bytes());
    m.extend_from_slice(&guid.to_le_bytes());
    m
}

/// S→C 0x2A kind 0 with GUID −1 and the player's gold (bytes 3–6 zero,
/// the NPC module's refusal layout).
fn refusal(code: u8, gold: u32) -> Vec<u8> {
    npc::transaction(0, code, u32::MAX, gold).to_vec()
}

fn state(h: &TestHost<FakeSim>, npc: UnitId, p: UnitId) -> Option<u8> {
    h.game.world.w.lists[&npc].state(p)
}

fn assert_clean(h: &TestHost<FakeSim>) {
    assert!(h.game.world.faults.is_empty(), "{:?}", h.game.world.faults);
    assert!(h.game.unhandled.is_empty(), "{:?}", h.game.unhandled);
}

// Talk, then chat, then trade at Charsi: message sequence and list states.
#[test]
fn talk_to_charsi_then_chat_and_trade() {
    // The talk sends 0x27, 0x29, 0x28 in one frame; the chat open sends
    // nothing (no healer); the trade action sends no 0x2A.
    let (s, p) = sim();
    let mut h = host(s);
    let (code, got) = send(&mut h, &hex("13 01000000 06000000"));
    assert_eq!(code, ResultCode::Done);
    let ids: Vec<u8> = got.iter().map(|m| m[0]).collect();
    assert_eq!(ids, [0x27, 0x29, 0x28]);
    // 0x27: 27 01 <NPC GUID> + the 34 list bytes (the fake encodes zeros).
    assert_eq!(got[0][..6], hex("27 01 06000000"));
    assert_eq!(got[0].len(), 40);
    let ctl = h.game.world.w.ctl.as_ref().unwrap();
    assert_eq!(got[1], [&[0x29u8][..], &ctl.game.0[..]].concat());
    let record = h.game.world.w.quests[&p].flags[0].0;
    assert_eq!(
        got[2],
        [&hex("28 01 06000000 00")[..], &record[..]].concat()
    );
    assert_eq!(state(&h, CHARSI_U, p), Some(talk::TALKING));
    assert_eq!(h.game.world.w.interact.get(&p), Some(&(1, CHARSI)));

    let (code, got) = send(&mut h, &hex("2f 01000000 06000000"));
    assert_eq!((code, got), (ResultCode::Done, vec![]));
    assert_eq!(state(&h, CHARSI_U, p), Some(talk::CHATTING));

    let (code, got) = send(&mut h, &hex("38 01000000 06000000 00000000"));
    assert_eq!((code, got), (ResultCode::Done, vec![]));
    assert_eq!(state(&h, CHARSI_U, p), Some(talk::TRADING));
    let log = &h.game.world.w.log;
    assert_eq!(
        log.last().unwrap(),
        &format!("open trade {} 1006 true false", p.0)
    );
    assert_clean(&h);
}

// Interact range limits and unit-type validation of 0x13.
#[test]
fn interact_distances_and_unit_types() {
    let (mut s, p) = sim();
    s.world.w.units.get_mut(&CHARSI_U).unwrap().x = 108; // distance 8
    s.world.w.units.get_mut(&AKARA_U).unwrap().x = 151; // distance 51
    let mut h = host(s);
    // 7..8: approach, result 0, no interaction.
    let (code, got) = send(&mut h, &npc_msg(0x13, 1, CHARSI));
    assert_eq!((code, got), (ResultCode::Done, vec![]));
    assert!(h
        .game
        .world
        .w
        .log
        .contains(&format!("approach {} 1006", p.0)));
    assert_eq!(state(&h, CHARSI_U, p), None);
    // > 50 → 1.
    let (code, _) = send(&mut h, &npc_msg(0x13, 1, AKARA));
    assert_eq!(code, ResultCode::Refused);
    // Missing monster → 1.
    let (code, _) = send(&mut h, &npc_msg(0x13, 1, 0x99));
    assert_eq!(code, ResultCode::Refused);
    // Unit type > 5 → 2.
    let (code, _) = send(&mut h, &npc_msg(0x13, 6, CHARSI));
    assert_eq!(code, ResultCode::Invalid);
    assert_clean(&h);
    // Unit type 2 (object): this host has no object state: stub.
    let (code, got) = send(&mut h, &npc_msg(0x13, 2, CHARSI));
    assert_eq!((code, got), (ResultCode::Done, vec![]));
    assert_eq!(h.game.unhandled, vec![(0, 0x13, 9)]);
}

// Refusal codes of chat open (0x2F) and close (0x30).
#[test]
fn chat_open_and_close_refusals() {
    let (mut s, p) = sim();
    s.world.w.units.get_mut(&AKARA_U).unwrap().act = 1;
    let pguid = s.world.w.units[&p].guid;
    let mut h = host(s);
    // Missing → 1; a unit that is no monster with a list → 3; other act → 2.
    let (code, _) = send(&mut h, &npc_msg(0x2F, 1, 0x99));
    assert_eq!(code, ResultCode::Refused);
    let (code, _) = send(&mut h, &npc_msg(0x2F, 1, pguid));
    assert_eq!(code, ResultCode::Malformed);
    let (code, _) = send(&mut h, &npc_msg(0x30, 1, AKARA));
    assert_eq!(code, ResultCode::Invalid);
    // Bytes 1–4 are not read: the NPC is the GUID at +5.
    let (code, got) = send(&mut h, &npc_msg(0x2F, 0xFFFF_FFFF, CHARSI));
    assert_eq!((code, got), (ResultCode::Done, vec![]));
    assert_clean(&h);
}

// Chat close unlinks the player from the NPC's list and drops the gamble list.
#[test]
fn chat_close_unlinks_and_drops_the_gamble_list() {
    // `30 01000000 10000000` (Akara) sends nothing.
    let (mut s, p) = sim();
    s.world.w.lists.insert(
        AKARA_U,
        InteractionList {
            nodes: vec![(p, talk::TRADING)],
        },
    );
    s.world.w.interact.insert(p, (1, AKARA));
    let mut h = host(s);
    let (code, got) = send(&mut h, &hex("30 01000000 10000000"));
    assert_eq!((code, got), (ResultCode::Done, vec![]));
    assert_eq!(state(&h, AKARA_U, p), None);
    assert!(h.game.world.w.interact.is_empty());
    assert_eq!(
        h.game.world.w.log.last().unwrap(),
        &format!("drop gamble list {} 1016", p.0)
    );
    assert_clean(&h);
}

// Menu action (0x38): unknown unit refusal and the non-gambler gamble case.
#[test]
fn menu_action_unit_check_and_actions() {
    let (s, p) = sim();
    let mut h = host(s);
    // A failing unit check returns its result code.
    let (code, _) = send(&mut h, &hex("38 01000000 99000000 00000000"));
    assert_eq!(code, ResultCode::Refused);
    // Gamble at Charsi (not a gambler): nothing.
    let (code, got) = send(&mut h, &hex("38 02000000 06000000 00000000"));
    assert_eq!((code, got), (ResultCode::Done, vec![]));
    assert!(!h
        .game
        .world
        .w
        .log
        .iter()
        .any(|l| l.starts_with("open trade")));
    assert_eq!(state(&h, CHARSI_U, p), None);
    assert_clean(&h);
}

fn cain_setup(gold: u32) -> (TestHost<FakeSim>, UnitId, Vec<UnitId>) {
    let (mut s, p) = sim();
    let w = &mut s.world.w;
    NpcWorld::set_stat(w, p, npc::stat::GOLD, gold);
    w.interact.insert(p, (1, CAIN));
    let mut items = Vec::new();
    for (i, place) in [Place::Grid(0), Place::Equipped, Place::Grid(3)]
        .into_iter()
        .enumerate()
    {
        let u = UnitId(2000 + i as u32);
        w.add(
            u,
            FUnit {
                guid: 0x40 + i as u32,
                ty: 4,
                owner: Some(p),
                ..FUnit::default()
            },
        );
        w.inventory.entry(p).or_default().push(InvEntry {
            item: u,
            place,
            flags: 0,
        });
        items.push(u);
    }
    (host(s), p, items)
}

// Cain identifies all three items for 300 gold.
#[test]
fn cain_identifies_three_items_for_300() {
    let (mut h, _, items) = cain_setup(500);
    let (code, got) = send(&mut h, &hex("34 20000000"));
    assert_eq!(code, ResultCode::Done);
    assert_eq!(got, vec![hex("2a 00 03 00000000 ffffffff c8000000")]);
    for u in items {
        assert_eq!(h.game.world.w.units[&u].flags & 0x10, 0x10);
    }
    assert_clean(&h);
}

// Cain's refusals: too little gold, not the interact unit, missing NPC.
#[test]
fn cain_refusals() {
    let (mut h, _, _) = cain_setup(100);
    // Not enough gold → code 12.
    let (code, got) = send(&mut h, &hex("34 20000000"));
    assert_eq!((code, got), (ResultCode::Done, vec![refusal(12, 100)]));
    // Not the interact unit → code 9.
    let (code, got) = send(&mut h, &hex("34 06000000"));
    assert_eq!((code, got), (ResultCode::Done, vec![refusal(9, 100)]));
    // Missing NPC → code 9.
    let (code, got) = send(&mut h, &hex("34 99000000"));
    assert_eq!((code, got), (ResultCode::Done, vec![refusal(9, 100)]));
    assert_clean(&h);
}

// Hire refusals: not the interact unit, and Kashya's level/quest requirement.
#[test]
fn hire_refusals() {
    let (mut s, p) = sim();
    let kashya = UnitId(1030);
    s.world.w.add(
        kashya,
        FUnit {
            guid: 0x30,
            ty: 1,
            class: npc::class::KASHYA,
            mode: 1,
            x: 100,
            y: 100,
            ..FUnit::default()
        },
    );
    s.world.w.lists.insert(kashya, InteractionList::default());
    let mut h = host(s);
    // Not the interact unit → code 9.
    let (code, got) = send(&mut h, &hex("36 30000000 0100 0000"));
    assert_eq!((code, got), (ResultCode::Done, vec![refusal(9, 500)]));
    // Kashya below level 8 without Sisters' Burial Grounds → code 11.
    h.game.world.w.interact.insert(p, (1, 0x30));
    let (code, got) = send(&mut h, &hex("36 30000000 0200 0000"));
    assert_eq!((code, got), (ResultCode::Done, vec![refusal(11, 500)]));
    assert_clean(&h);
}

// Resurrect (0x62) is malformed outside the expansion.
#[test]
fn resurrect_outside_the_expansion_is_malformed() {
    let (s, _) = sim();
    let mut h = host(s);
    let (code, got) = send(&mut h, &hex("62 30000000"));
    assert_eq!((code, got), (ResultCode::Malformed, vec![]));
    assert_clean(&h);
}

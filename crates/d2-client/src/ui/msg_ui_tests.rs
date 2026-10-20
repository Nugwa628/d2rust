//! The UI dispatch of the 0x5D, 0x63 and 0x77 outputs (test
//! vectors, UI side; "A" and "B" are two reference sessions).

use super::*;
use crate::bridge::world::{ClientUnit, UnitKey, PLAYER};
use crate::ui::layout::Screen;
use crate::ui::original::{UiConfig, UiOutcome};
use crate::ui::states::UiEffect;

fn world(expansion: bool) -> ClientWorld {
    let mut w = ClientWorld::default();
    let key = UnitKey::new(PLAYER, 1);
    let mut u = ClientUnit::new(key);
    u.mode = 1;
    w.units.insert(key, u);
    w.local_player = Some(key);
    w.expansion = u32::from(expansion);
    w
}

fn ui() -> OriginalUi {
    let config = UiConfig {
        screen: Screen::R800,
        expansion_installed: true,
    };
    OriginalUi::new(config, None).unwrap()
}

fn quest(c: u8, f: u8, s: u8, v: i16) -> Output {
    Output::QuestUi {
        chain: c,
        flags: f,
        status: s,
        extra: v,
    }
}

fn ui_sounds(ids: &[i32]) -> Vec<SoundRequest> {
    ids.iter().map(|&i| SoundRequest::Ui(i)).collect()
}

#[test]
fn quest_sounds() {
    let w = world(false);
    let mut u = ui();
    for (c, f, v) in [
        (8, 2, 0),
        (0x21, 0x10, 5),
        (33, 1, 0),
        (4, 2, 0),
        (10, 0x10, 0),
    ] {
        u.apply_output(&quest(c, f, 0, v), &w).unwrap();
    }
    assert_eq!(
        u.take_outcome().sounds,
        ui_sounds(&[7, 5, 237, 241, 2456, 2474])
    );
    // f = 3, c = 8: the bit-0 table has no 8 (edge case 1).
    u.apply_output(&quest(8, 3, 0, 0), &w).unwrap();
    assert_eq!(u.take_outcome(), UiOutcome::default());
    // f bit 0 with c = 4 is a model row: nothing at delivery.
    u.apply_output(&quest(4, 1, 0, 0), &w).unwrap();
    assert_eq!(u.take_outcome(), UiOutcome::default());
}

#[test]
fn quest_log_tail_a129118() {
    let w = world(false);
    let mut u = ui();
    u.apply_output(&quest(1, 0, 1, 0), &w).unwrap();
    assert!(u.is_open(UI_QUESTLOG as u8));
    assert!(u.msg_state().quest_log_latch);
    let o = u.take_outcome();
    assert!(o.effects.contains(&UiEffect::Opened(UI_QUESTLOG as u8)));
    // The entry of c in the quest-log table: not in the specs.
    assert_eq!(o.skipped, [skip::QUEST_LOG_TABLE]);
    // Latch set, quest screen closed: nothing.
    u.apply_output(&quest(1, 0, 2, 0), &w).unwrap();
    assert_eq!(u.take_outcome(), UiOutcome::default());
    // f bit 5, c 32: the quest log's B:= v, then the tail.
    u.apply_output(&quest(32, 0x20, 0, -3), &w).unwrap();
    assert_eq!(u.msg_state().quest_7bf2ac, Some(-3));
}

#[test]
fn quest_parts_without_a_spec_are_skipped() {
    let w = world(false);
    let mut u = ui();
    for (c, f) in [(3, 1), (13, 1), (15, 1), (36, 2), (5, 0x20)] {
        u.apply_output(&quest(c, f, 0, 0), &w).unwrap();
    }
    assert_eq!(
        u.take_outcome().skipped,
        [skip::MONSTER_EFFECT, skip::VIDEO_7, skip::DEN_COUNTER]
    );
    // f bit 1, c 23: expansion → the expansion flag; classic → the classic
    // flag. No video plays, so classic reports no skipped part.
    u.apply_output(&quest(23, 2, 0, 0), &world(true)).unwrap();
    assert!(u.msg_state().act_end_7bc9d8 && !u.msg_state().act_end_7bc9d4);
    let mut u = ui();
    u.apply_output(&quest(23, 2, 0, 0), &w).unwrap();
    assert!(!u.msg_state().act_end_7bc9d8 && u.msg_state().act_end_7bc9d4);
    assert!(u.take_outcome().skipped.is_empty());
}

const B7852: [u8; 16] = [2, 1, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

#[test]
fn waypoint_menu_b7852() {
    let w = world(false);
    let mut u = ui();
    let o = Output::WaypointMenu {
        guid: 0x0A,
        record: B7852,
    };
    u.apply_output(&o, &w).unwrap();
    assert!(u.is_open(UI_WAYPOINT as u8));
    let st = u.msg_state().waypoint.unwrap();
    assert_eq!((st.guid, st.close_latch), (0x0A, false));
    assert_eq!(st.record.0, B7852, "magic 0x0102 kept, bit 0 already set");
    // No room in the model: tab 0.
    assert_eq!(st.tab, Some(0));
    let out = u.take_outcome();
    // r2.2: the input reset is asked of the host, not skipped.
    assert_eq!(out.skipped, [skip::WAYPOINT_ROWS]);
    assert!(u.take_input_reset());
    assert!(!u.take_input_reset());
    // Jump 1 with the left slot: the cursor effect is reported.
    assert!(out
        .effects
        .iter()
        .any(|e| matches!(e, UiEffect::CursorX(_))));
    // Magic 0x0000: wiped to index 0 only.
    let mut u = ui();
    let mut r = B7852;
    r[..2].copy_from_slice(&[0, 0]);
    u.apply_output(&Output::WaypointMenu { guid: 1, record: r }, &w)
        .unwrap();
    let mut want = [0u8; 16];
    want[2] = 1;
    assert_eq!(u.msg_state().waypoint.unwrap().record.0, want);
    // Another magic: fatal in the game.
    let mut u = ui();
    r[..2].copy_from_slice(&[7, 7]);
    assert!(matches!(
        u.apply_output(&Output::WaypointMenu { guid: 1, record: r }, &w),
        Err(OriginalUiError::Waypoint(_))
    ));
}

#[test]
fn waypoint_menu_refused_stores_nothing() {
    let w = world(false);
    let mut u = ui();
    let menu = |guid| Output::WaypointMenu {
        guid,
        record: B7852,
    };
    u.apply_output(&menu(0x0A), &w).unwrap();
    // Open already: the gate refuses (`ui-states.tsv` row 20), so the
    // second menu's GUID and record are not stored.
    let mut r = B7852;
    r[3] = 0x80;
    u.apply_output(
        &Output::WaypointMenu {
            guid: 0x0B,
            record: r,
        },
        &w,
    )
    .unwrap();
    let st = u.msg_state().waypoint.unwrap();
    assert_eq!((st.guid, st.record.0), (0x0A, B7852));
    // A dead player: the call returns 1 without setting the flag, so the record
    // is stored.
    let mut dead = world(false);
    dead.units.get_mut(&UnitKey::new(PLAYER, 1)).unwrap().mode = 0;
    let mut u = ui();
    u.apply_output(&menu(0x0C), &dead).unwrap();
    assert!(!u.is_open(UI_WAYPOINT as u8));
    assert_eq!(u.msg_state().waypoint.map(|s| s.guid), Some(0x0C));
}

#[test]
fn stash_and_cube_a104179() {
    let w = world(false);
    let mut u = ui();
    u.apply_output(
        &Output::TradeAction {
            code: 0x10,
            dead_or_absent: false,
        },
        &w,
    )
    .unwrap();
    assert!(u.is_open(UI_STASH as u8));
    assert_eq!(u.msg_state().inventory_mode, MODE_STASH);
    // 0x11: the stash closes (inventory mode 0x0C).
    u.apply_output(
        &Output::TradeAction {
            code: 0x11,
            dead_or_absent: false,
        },
        &w,
    )
    .unwrap();
    assert!(!u.is_open(UI_STASH as u8));
    assert_eq!(u.msg_state().inventory_mode, 0);
    // 0x15: the cube.
    u.apply_output(
        &Output::TradeAction {
            code: 0x15,
            dead_or_absent: false,
        },
        &w,
    )
    .unwrap();
    assert!(u.is_open(UI_CUBE as u8));
    assert_eq!(u.msg_state().inventory_mode, MODE_CUBE);
    // 0x11 with the cube's mode: nothing.
    u.apply_output(
        &Output::TradeAction {
            code: 0x11,
            dead_or_absent: false,
        },
        &w,
    )
    .unwrap();
    assert!(u.is_open(UI_CUBE as u8));
}

#[test]
fn trade_codes() {
    let w = world(false);
    let mut u = ui();
    // 0x0C with trade state 0: close trade(0), no decline.
    u.apply_output(
        &Output::TradeAction {
            code: 0x0C,
            dead_or_absent: false,
        },
        &w,
    )
    .unwrap();
    assert_eq!(u.take_outcome(), UiOutcome::default());
    u.apply_output(
        &Output::TradeAction {
            code: 0x0E,
            dead_or_absent: false,
        },
        &w,
    )
    .unwrap();
    assert!(u.msg_state().trade_7bce28);
    u.apply_output(
        &Output::TradeAction {
            code: 0x0F,
            dead_or_absent: false,
        },
        &w,
    )
    .unwrap();
    assert!(!u.msg_state().trade_7bce28);
    // Code 9: player event sound 23 on the local player.
    u.apply_output(
        &Output::TradeAction {
            code: 0x09,
            dead_or_absent: false,
        },
        &w,
    )
    .unwrap();
    assert_eq!(
        u.take_outcome().sounds,
        [SoundRequest::PlayerEvent {
            unit: UnitKey::new(PLAYER, 1),
            event: 23
        }]
    );
    // Past 0x15 and the "nothing" codes.
    for code in [0x16, 0xFF, 0x03, 0x12] {
        u.apply_output(
            &Output::TradeAction {
                code,
                dead_or_absent: false,
            },
            &w,
        )
        .unwrap();
    }
    assert_eq!(u.take_outcome(), UiOutcome::default());
    // The trade codes need the trade helpers: skipped.
    u.apply_output(
        &Output::TradeAction {
            code: 0x00,
            dead_or_absent: false,
        },
        &w,
    )
    .unwrap();
    assert_eq!(u.take_outcome().skipped, [skip::TRADE]);
    // A sound output is the audio layer's.
    let s = Output::ServerSound {
        unit: UnitKey::new(PLAYER, 1),
        class: 0,
        at: None,
        event: 2,
    };
    u.apply_output(&s, &w).unwrap();
    assert_eq!(u.take_outcome(), UiOutcome::default());
}

fn npc_text(t: u8, count: u8, kind0: u8, str0: u16) -> Output {
    let mut bytes = [0u8; 40];
    bytes[0] = 0x27;
    bytes[1] = t;
    bytes[2] = 6;
    bytes[6] = count;
    bytes[8] = kind0;
    bytes[10..12].copy_from_slice(&str0.to_le_bytes());
    Output::NpcText {
        bytes,
        present: true,
        object_class: 0,
    }
}

fn npc_dialog(cursor_item: bool) -> NpcDialog {
    NpcDialog {
        kind: 1,
        guid: 6,
        quest_flags: [0; 96],
        unit: UnitKey::new(1, 6),
        class: 148,
        interact: true,
        f4b1a10: None,
        cursor_item,
        level: 1,
        unidentified: 0,
        expansion: false,
        npc_monsters: vec![UnitKey::new(1, 6)],
    }
}

#[test]
fn npc_text_list() {
    let w = world(false);
    let mut u = ui();
    assert_eq!(u.npc_text(), None);
    // A seq 37351: type 1, one entry (kind 0, string 0x25): rebuilt.
    u.apply_output(&npc_text(1, 1, 0, 0x25), &w).unwrap();
    let l = *u.npc_text().unwrap();
    assert_eq!((l.count(), l.kind(0), l.string(0)), (1, 0, 0x25));
    assert_eq!(l.m(), 0x25);
    // r2.1 overhead number and r2.2 type 2: the list stays.
    u.apply_output(&npc_text(1, 1, 3, 37), &w).unwrap();
    u.apply_output(&npc_text(2, 1, 0, 9), &w).unwrap();
    assert_eq!(u.npc_text(), Some(&l));
    assert_eq!(u.take_outcome().skipped, [skip::NPC_TEXT_SHOW; 3]);
    // r2.3: any other type frees it.
    u.apply_output(&npc_text(4, 1, 0, 9), &w).unwrap();
    assert_eq!(u.npc_text(), None);
    // r3: freeing with no list is "nothing to free", not a fault (the
    // game's null read ends the process; D2RUST treats it as none).
    u.apply_output(&npc_text(5, 1, 0, 9), &w).unwrap();
    assert_eq!(u.npc_text(), None);
}

#[test]
fn npc_dialog_case() {
    use crate::bridge::msg::ui_npc::DialogCase;
    let list = |count, kind0, str0| match npc_text(1, count, kind0, str0) {
        Output::NpcText { bytes, .. } => NpcTextList::from_record(&bytes),
        _ => unreachable!(),
    };
    let d = npc_dialog(false);
    let l = list(1, 0, 0x25);
    // B0 first, even without a list.
    assert_eq!(dialog_case(1, None, &d).unwrap(), Some(DialogCase::B0));
    // No list: fatal 0x1060.
    assert!(matches!(
        dialog_case(0, None, &d),
        Err(OriginalUiError::NoNpcText)
    ));
    // B1 before B2; B2 with m; m = 0xFFFF → B3–B6.
    assert_eq!(
        dialog_case(0, Some(&l), &npc_dialog(true)).unwrap(),
        Some(DialogCase::B1)
    );
    assert_eq!(
        dialog_case(0, Some(&l), &d).unwrap(),
        Some(DialogCase::B2 { m: 0x25 })
    );
    assert_eq!(
        dialog_case(0, Some(&list(1, 0, 0xFFFF)), &d).unwrap(),
        Some(DialogCase::Rest)
    );
    // M is the smallest kind-0 string id of the list (a second
    // entry of the helper is (kind 0, id 0)); B1 does not need m; an
    // empty list gives 0xFFFF.
    let l2 = list(2, 0, 0x25);
    assert_eq!(
        dialog_case(0, Some(&l2), &d).unwrap(),
        Some(DialogCase::B2 { m: 0 })
    );
    assert_eq!(list(0, 0, 0x25).m(), 0xFFFF);
    assert_eq!(
        dialog_case(0, Some(&l2), &npc_dialog(true)).unwrap(),
        Some(DialogCase::B1)
    );
}

#[test]
fn npc_dialog_answer_is_kept_for_the_bridge() {
    use crate::bridge::msg::ui_npc::DialogCase;
    let w = world(false);
    let mut u = ui();
    let d = npc_dialog(false);
    // Without a 0x27 first: fatal 0x1060.
    assert!(matches!(
        u.apply_output(&Output::NpcDialog(Box::new(d.clone())), &w),
        Err(OriginalUiError::NoNpcText)
    ));
    u.apply_output(&npc_text(1, 1, 0, 0x25), &w).unwrap();
    u.take_outcome();
    u.apply_output(&Output::NpcDialog(Box::new(d.clone())), &w)
        .unwrap();
    assert_eq!(
        u.take_dialog_answer(),
        Some((Box::new(d.clone()), DialogCase::B2 { m: 0x25 }))
    );
    assert_eq!(u.take_dialog_answer(), None);
    assert_eq!(u.take_outcome().skipped, [skip::NPC_DIALOG_UI]);
    // A 2-entry list (helper: second entry kind 0, id 0) gives m = 0.
    u.apply_output(&npc_text(1, 2, 0, 0x25), &w).unwrap();
    u.take_outcome();
    u.apply_output(&Output::NpcDialog(Box::new(d.clone())), &w)
        .unwrap();
    assert_eq!(
        u.take_dialog_answer(),
        Some((Box::new(d), DialogCase::B2 { m: 0 }))
    );
    assert_eq!(u.take_outcome().skipped, [skip::NPC_DIALOG_UI]);
}

#[test]
fn npc_dialog_branches_ask_for_speech() {
    let w = world(false);
    let run = |text: Output, d: NpcDialog| {
        let mut u = ui();
        u.apply_output(&text, &w).unwrap();
        u.take_outcome();
        u.apply_output(&Output::NpcDialog(Box::new(d)), &w).unwrap();
        u.take_outcome().sounds
    };
    let npc = UnitKey::new(1, 6);
    // B2: the dialog line of m.
    assert_eq!(
        run(npc_text(1, 1, 0, 0x25), npc_dialog(false)),
        [SoundRequest::NpcDialogLine {
            npc,
            class: 148,
            key: 0x25
        }]
    );
    // B1 (a cursor item): none.
    assert_eq!(run(npc_text(1, 1, 0, 0x25), npc_dialog(true)), []);
    // B6: no text line, an `interact` class: the NPC's greeting.
    let greeting = SoundRequest::NpcGreeting { npc, class: 148 };
    assert_eq!(
        run(npc_text(1, 1, 0, 0xFFFF), npc_dialog(false)),
        [greeting]
    );
    // B4 (no `interact`) and B5 (an interaction running): none.
    let mut d = npc_dialog(false);
    d.interact = false;
    assert_eq!(run(npc_text(1, 1, 0, 0xFFFF), d), []);
    let mut d = npc_dialog(false);
    d.f4b1a10 = Some(1);
    assert_eq!(run(npc_text(1, 1, 0, 0xFFFF), d), []);
}

fn record(t: u8, guid: u32, entries: &[(u8, u16)]) -> Output {
    let mut bytes = [0u8; 40];
    bytes[0] = 0x27;
    bytes[1] = t;
    bytes[2..6].copy_from_slice(&guid.to_le_bytes());
    bytes[6] = entries.len() as u8;
    for (k, &(kind, id)) in entries.iter().enumerate() {
        bytes[8 + 4 * k] = kind;
        bytes[10 + 4 * k..12 + 4 * k].copy_from_slice(&id.to_le_bytes());
    }
    Output::NpcText {
        bytes,
        present: true,
        object_class: 0,
    }
}

#[test]
fn npc_text_list_walk() {
    let w = world(false);
    let mut u = ui();
    // The spec's example: (kind 0, 300), (kind 1, 50), (kind 0, 120).
    u.apply_output(&record(1, 6, &[(0, 300), (1, 50), (0, 120)]), &w)
        .unwrap();
    let l = *u.npc_text().unwrap();
    // Built by prepending (reverse message order), then sorted by id.
    assert_eq!(l.nodes(), [(1, 50), (0, 120), (0, 300)]);
    assert_eq!((l.m(), l.m2()), (120, 50));
    // Kinds other than 0 and 1 are skipped; none → 0xFFFF.
    u.apply_output(&record(1, 6, &[(2, 7), (3, 8)]), &w)
        .unwrap();
    let l = *u.npc_text().unwrap();
    assert_eq!((l.m(), l.m2()), (0xFFFF, 0xFFFF));
    // Ids compare as unsigned u16.
    u.apply_output(&record(1, 6, &[(0, 0x9000), (0, 0x0100)]), &w)
        .unwrap();
    assert_eq!(u.npc_text().unwrap().m(), 0x0100);
    // The 1-entry kind-3 overhead case does not rebuild the list.
    u.apply_output(&record(1, 6, &[(3, 37)]), &w).unwrap();
    assert_eq!(u.npc_text().unwrap().m(), 0x0100);
    // A count of 8 or more is the build's fatal assertion.
    let eight = [(0u8, 1u16); 8];
    assert!(matches!(
        u.apply_output(&record(1, 6, &eight), &w),
        Err(OriginalUiError::NpcTextCount(8))
    ));
    // The 0x7C0C68 flag has no writer: it stays 0, so B0 never holds.
    assert_eq!(u.msg_state().ui_7c0c68, 0);
}

#[test]
fn quest_special_hire_popup_and_code_3() {
    let w = world(true);
    let mut u = ui();
    let sp = |code: u16, a: u16| Output::QuestSpecial {
        code,
        words: [a, 0, 0, 0, 0, 0],
    };
    // Code 2: the name word has no effect; the popup opens UI 0x23 and, with
    // `PopupHireling` missing (0), the hireling pop-up flag:= 1.
    u.more_mut().hire_7beecc = 9;
    u.apply_output(&sp(2, 0x1234), &w).unwrap();
    assert!(u.is_open(0x23));
    assert_eq!(u.more().hire_7beecc, 0);
    assert_eq!(u.more().hire_7beee4, 1);
    // Another name, popup already configured: no one-time open.
    let mut u = ui();
    u.more_mut().popup_hireling = 1;
    u.apply_output(&sp(2, 0), &w).unwrap();
    assert!(u.is_open(0x23));
    assert_eq!(u.more().hire_7beee4, 0);
    // Code 3 has no observable effect (the two fields have no reader).
    let before = u.more().clone();
    u.apply_output(&sp(3, 5), &w).unwrap();
    assert_eq!(*u.more(), before);
}

#[test]
fn npc_interact_sounds_overlay_and_the_interact_npc_test() {
    let w = world(false);
    let mut u = ui();
    let k = UnitKey::new(1, 7);
    let mk = |unit: UnitKey, present, class, m3c, blocker| Output::NpcInteract {
        unit,
        present,
        class,
        mdata_3c: m3c,
        blocker_open: blocker,
    };
    // Absent: nothing.
    u.apply_output(&mk(k, false, 148, None, false), &w).unwrap();
    assert!(u.more().unit_sounds.is_empty() && u.more().overlays.is_empty());
    // act5pow (534): sound 4603 when +0x3C ≠ −1, else 4607; no overlay.
    u.apply_output(&mk(k, true, 534, Some(5), false), &w)
        .unwrap();
    u.apply_output(&mk(k, true, 534, Some(-1), false), &w)
        .unwrap();
    assert_eq!(u.more().unit_sounds, [(4603, k), (4607, k)]);
    assert!(u.more().overlays.is_empty());
    // Another class: overlay 72 unless the unit is the interact NPC.
    let mut u = ui();
    u.apply_output(&mk(k, true, 148, Some(-1), false), &w)
        .unwrap();
    assert_eq!(
        u.more().overlays,
        [OverlayCall {
            unit: k,
            id: 72,
            on: true
        }]
    );
    u.more_mut().interact_active = true;
    u.more_mut().interact_npc = 7;
    u.apply_output(&mk(k, true, 148, Some(-1), false), &w)
        .unwrap();
    assert_eq!(u.more().overlays.len(), 1);
    // The test needs the interaction flag set: not active, same GUID → overlay.
    u.more_mut().interact_active = false;
    u.apply_output(&mk(k, true, 148, Some(-1), false), &w)
        .unwrap();
    assert_eq!(u.more().overlays.len(), 2);
    // act2guard2 (331): sound 3983 unless a blocker is open, the flag
    // function is set, or quest 12 bit 8 or bit 1 is set.
    let mut u = ui();
    u.apply_output(&mk(k, true, 331, Some(-1), false), &w)
        .unwrap();
    assert_eq!(u.more().unit_sounds, [(3983, k)]);
    u.apply_output(&mk(k, true, 331, Some(-1), true), &w)
        .unwrap();
    assert_eq!(u.more().unit_sounds.len(), 1);
    u.more_mut().f4b1620 = true;
    u.apply_output(&mk(k, true, 331, Some(-1), false), &w)
        .unwrap();
    assert_eq!(u.more().unit_sounds.len(), 1);
    u.more_mut().f4b1620 = false;
    for bit in [8usize, 1] {
        let mut q = [0u8; 96];
        let n = 16 * 12 + bit;
        q[n >> 3] |= 1 << (n & 7);
        u.apply_output(&Output::QuestFlags { record: q }, &w)
            .unwrap();
        u.apply_output(&mk(k, true, 331, Some(-1), false), &w)
            .unwrap();
        assert_eq!(u.more().unit_sounds.len(), 1, "bit {bit}");
    }
}

#[test]
fn small_ui_globals() {
    let w = world(false);
    let mut u = ui();
    // 0x91: flags of entries whose class equals a slot below the row
    // count; nothing is cleared.
    u.more_mut().monstats_rows = 600;
    u.more_mut().intro_table = [(148, 0), (148, 0), (150, 0), (700, 0), (5, 1)]
        .map(|(class, flag)| IntroEntry { class, flag })
        .to_vec();
    let mut slots = [0xFFFFu16; 12];
    slots[0] = 148;
    slots[3] = 700; // past the row count
    u.apply_output(&Output::NpcIntro { slots }, &w).unwrap();
    let flags: Vec<u8> = u.more().intro_table.iter().map(|e| e.flag).collect();
    assert_eq!(flags, [1, 1, 0, 0, 1]);
    // 0x78: the name with byte 15 forced to 0, and the partner GUID.
    u.apply_output(
        &Output::TradePartner {
            name: [b'x'; 16],
            guid: 77,
        },
        &w,
    )
    .unwrap();
    assert_eq!(u.more().partner_name[14..], [b'x', 0]);
    assert_eq!(u.more().partner_guid, 77);
    // 0x29: the game quest record.
    u.apply_output(&Output::GameQuestFlags { record: [7; 96] }, &w)
        .unwrap();
    assert_eq!(u.more().game_quest_record, [7; 96]);
    // 0x52: status bytes copied, the quest log flag:= 0.
    let mut st = [0u8; 41];
    st[5] = 9;
    u.more_mut().quest_7bf2b0 = 4;
    u.apply_output(&Output::QuestLog { status: st }, &w)
        .unwrap();
    assert_eq!(u.more().quest_log_status, st);
    assert_eq!(u.more().quest_7bf2b0, 0);
    // 0x5E: the 37 bytes and the availability flag:= 1.
    u.apply_output(&Output::QuestAvailability { bytes: [3; 37] }, &w)
        .unwrap();
    assert_eq!(u.more().quest_avail, [3; 37]);
    assert!(u.more().quest_avail_set);
    // 0x9B: alive (0xFFFF) runs the NPC-menu edit with 11, 8, 24, 21, 43.
    u.apply_output(&Output::MercRevive { state: 5, value: 6 }, &w)
        .unwrap();
    assert_eq!((u.more().merc_state, u.more().merc_7c0dd0), (5, 6));
    assert!(u.more().merc_menu_calls.is_empty());
    u.apply_output(
        &Output::MercRevive {
            state: 0xFFFF,
            value: 0,
        },
        &w,
    )
    .unwrap();
    assert_eq!(u.more().merc_menu_calls, [11, 8, 24, 21, 43]);
    // 0x28 T = 6 copies Q; the record is overwritten only by 0x28.
    assert_eq!(u.more().client_quest, [0; 96]);
    u.apply_output(&Output::QuestFlags { record: [1; 96] }, &w)
        .unwrap();
    assert_eq!(u.more().client_quest, [1; 96]);
    u.apply_output(&npc_text(1, 1, 0, 0x25), &w).unwrap();
    let mut d = npc_dialog(false);
    d.quest_flags = [2; 96];
    u.apply_output(&Output::NpcDialog(Box::new(d)), &w).unwrap();
    assert_eq!(u.more().client_quest, [2; 96]);
}

#[test]
fn trade_code_0a_plays_on_the_partner() {
    let mut w = world(false);
    let mut u = ui();
    let p = UnitKey::new(PLAYER, 9);
    w.units.insert(p, ClientUnit::new(p));
    u.apply_output(
        &Output::TradeAction {
            code: 0x0A,
            dead_or_absent: false,
        },
        &w,
    )
    .unwrap();
    assert!(u.take_outcome().sounds.is_empty());
    u.apply_output(
        &Output::TradePartner {
            name: [0; 16],
            guid: 9,
        },
        &w,
    )
    .unwrap();
    u.apply_output(
        &Output::TradeAction {
            code: 0x0A,
            dead_or_absent: false,
        },
        &w,
    )
    .unwrap();
    assert_eq!(
        u.take_outcome().sounds,
        [SoundRequest::PlayerEvent { unit: p, event: 23 }]
    );
}

fn chat(kind: u8, lang: u8, ut: u8, b8: u8, name: &str, text: &[u8], present: bool) -> Output {
    Output::ChatLine {
        kind,
        lang,
        unit: UnitKey::new(ut, 4),
        b8,
        b9: 0,
        name: name.as_bytes().to_vec(),
        text: text.to_vec(),
        present,
        player_name: None,
    }
}

#[test]
fn chat_lines_by_type_and_overhead_records() {
    let mut u = ui();
    let t = b"hi".to_vec();
    // By type.
    assert_eq!(
        u.chat_line(&chat(4, 0, 0, 0, "", b"hi", false), false),
        Some(ChatAction::ScreenMessage(t.clone()))
    );
    assert_eq!(
        u.chat_line(&chat(6, 0, 0, 0, "", b"hi", false), false),
        Some(ChatAction::Formatted(t.clone()))
    );
    // 1 with u8@3 in {0, 1}: a plain screen message; otherwise named.
    assert_eq!(
        u.chat_line(&chat(1, 0, 1, 0, "n", b"hi", false), false),
        Some(ChatAction::ScreenMessage(t.clone()))
    );
    assert_eq!(
        u.chat_line(&chat(1, 0, 2, 0, "n", b"hi", false), false),
        Some(ChatAction::Named {
            name: b"n".to_vec(),
            text: t.clone()
        })
    );
    assert_eq!(
        u.chat_line(&chat(2, 0, 0, 0, "n", b"hi", false), false),
        Some(ChatAction::Named {
            name: b"n".to_vec(),
            text: t.clone()
        })
    );
    assert_eq!(
        u.chat_line(&chat(7, 0, 0, 5, "", b"hi", false), false),
        Some(ChatAction::Type7(5))
    );
    assert_eq!(
        u.chat_line(&chat(3, 0, 0, 0, "", b"hi", false), false),
        None
    );
    // 5: overhead text only when the unit was present.
    let k = UnitKey::new(1, 4);
    assert_eq!(
        u.chat_line(&chat(5, 0, 1, 0, "", b"hi", false), false),
        None
    );
    assert!(u.more().overhead.is_empty());
    assert_eq!(
        u.chat_line(&chat(5, 0, 1, 0, "", b"hi", true), false),
        Some(ChatAction::Overhead(k, t.clone()))
    );
    // r4: d = 8 · min(n, 254) + 125; end = counter + d.
    let r = u.more().overhead[&k].clone();
    assert_eq!((r.duration, r.end, r.text), (141, 141, t));
    let long = vec![b'a'; 300];
    u.set_overhead(k, &long, 0);
    let r = u.more().overhead[&k].clone();
    assert_eq!((r.duration, r.text.len()), (8 * 254 + 125, 254));
    // The counter steps per draw; a record is freed once its end passed.
    u.set_overhead(k, b"hi", 0);
    for _ in 0..141 {
        u.overhead_draw();
    }
    assert!(u.more().overhead.contains_key(&k));
    u.overhead_draw();
    assert!(!u.more().overhead.contains_key(&k));
    // An empty text frees the record; so does OverheadClear.
    u.set_overhead(k, b"x", 0);
    u.set_overhead(k, b"", 0);
    assert!(u.more().overhead.is_empty());
    // r3.1: a squelched player line shows nothing; r3.3: a text with a
    // character >= 0x80 in a foreign language is refused unless {7, 12}.
    assert_eq!(u.chat_line(&chat(4, 0, 0, 0, "", b"hi", false), true), None);
    u.more_mut().own_lang = 0;
    assert_eq!(
        u.chat_line(&chat(4, 9, 1, 0, "", &[0xE9], false), false),
        None
    );
    assert!(u
        .chat_line(&chat(4, 0, 1, 0, "", &[0xE9], false), false)
        .is_some());
    u.more_mut().own_lang = 12;
    assert!(u
        .chat_line(&chat(4, 7, 1, 0, "", &[0xE9], false), false)
        .is_some());
}

#[test]
fn close_trade_one_toggles_the_inventory_only_for_a_live_player() {
    let w = world(false);
    // Code 0x0D = close trade(1): a live local player gets the toggle.
    let mut u = ui();
    u.apply_output(
        &Output::TradeAction {
            code: 0x0D,
            dead_or_absent: false,
        },
        &w,
    )
    .unwrap();
    assert!(u.is_open(1));
    // Dead or absent (captured at receive): no toggle.
    let mut u = ui();
    u.apply_output(
        &Output::TradeAction {
            code: 0x0D,
            dead_or_absent: true,
        },
        &w,
    )
    .unwrap();
    assert!(!u.is_open(1));
    // Close trade(0) never toggles.
    let mut u = ui();
    u.apply_output(
        &Output::TradeAction {
            code: 0x0C,
            dead_or_absent: false,
        },
        &w,
    )
    .unwrap();
    assert!(!u.is_open(1));
}

#[test]
fn npc_text_freed_at_interaction_end() {
    let w = world(false);
    let mut u = ui();
    u.apply_output(&npc_text(1, 1, 0, 0x25), &w).unwrap();
    assert!(u.npc_text().is_some());
    // The interaction ends: freed.
    u.free_npc_text();
    assert_eq!(u.npc_text(), None);
    // Game exit: freed too, and freeing none is harmless.
    u.apply_output(&npc_text(1, 1, 0, 0x25), &w).unwrap();
    u.free_npc_text();
    u.free_npc_text();
    assert_eq!(u.npc_text(), None);
}

// The message edge cases, each reproduced.
#[test]
fn messages_edge_cases_reproduced() {
    use crate::ui::messages::chat::ScreenMessages;
    use crate::ui::messages::dialog::{DialogOpen, DialogUi, Scroll};
    use crate::ui::messages::hire::{HirePopup, StoneAnim};
    use crate::ui::messages::overhead::{
        BubbleBox, BubbleResult, OverheadPass, TimedBox, CAND_H, CAND_W, MOVE_TABLE,
    };
    use crate::ui::messages::{testutil::Fixed, Ltrb};
    let m = Fixed;
    let wide = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
    // Timing runs on the wall clock in milliseconds, fed from the frame
    // clock: records, the timed box, the scroll and the stones take
    // millisecond ticks.
    let mut l = ScreenMessages::new();
    l.add(&wide("a"), 0, 5000, 800, false, &m);
    assert_eq!(l.records()[0].expiry, 15_000);
    assert_eq!(TimedBox::open(&wide("x"), 1000, &m).expiry, 1000 + 26 * 200);
    let mut sc = Scroll::new(4);
    sc.update(100);
    sc.update(108);
    assert_eq!(sc.p, (108 - 100) / 4 * 4);
    let mut st = StoneAnim::default();
    st.step(10);
    st.step(61);
    assert_eq!(st.counter, 1);
    // A refused bubble placement leaves font 13 current.
    let mut p = OverheadPass::default();
    p.begin((0, 0), None);
    let b = |x| BubbleBox::new(&wide("hello"), &m).at(x, 300);
    p.bubble(b(40), 100, 100, 0);
    assert_eq!(
        p.bubble(b(40), 100, 100, 0),
        BubbleResult::Refused {
            font_restored: false
        }
    );
    // Placement candidates are 400 × 280 and only diagonal.
    assert_eq!((CAND_W, CAND_H), (400, 280));
    assert_eq!(MOVE_TABLE, [1, 1, -1, 1, 1, -1, -1, -1]);
    let mut p = OverheadPass::default();
    p.begin((0, 0), None);
    p.place(Ltrb::new(40, 40, 140, 61), 800, 600);
    p.place(Ltrb::new(40, 40, 140, 61), 800, 600);
    assert_eq!(p.slots()[1], Ltrb::xywh(90, 75, 400, 280));
    // A bubble of more than 10 lines draws its first 10 lines at the
    // positions of the last 10.
    let mut big = BubbleBox {
        x: 0,
        y: 50,
        w: 50,
        h: 15 * 12 + 6,
        lines: (0..12).map(|i| wide(&i.to_string())).collect(),
        flag: 0,
    };
    let d = crate::ui::messages::overhead::bubble_draw(&mut big, 800, 600, 0).unwrap();
    assert_eq!((d.lines.len(), d.lines[0].y), (10, 50 + 18 + 30));
    // The timed box is centred on x = 320 at every resolution.
    let t = TimedBox::open(&wide("0123456789"), 0, &m);
    assert_eq!(t.bx.x, 320 - t.bx.w / 2);
    // A second dialog open keeps the old text.
    let mut d = DialogUi::default();
    d.open(
        DialogOpen::Npc { unit: None, id: 1 },
        &wide("5\nold"),
        800,
        600,
        0,
        0,
        1,
    );
    d.open(
        DialogOpen::Npc { unit: None, id: 2 },
        &wide("5\nnew"),
        800,
        600,
        0,
        0,
        2,
    );
    assert_eq!(d.panel.as_ref().unwrap().lines[0], wide("old"));
    // 0x27 with a type other than 1 or 2 frees the list without a null
    // test: with no list it is "no list" (not a crash).
    let w = world(false);
    let mut u = ui();
    u.apply_output(&npc_text(4, 1, 0, 9), &w).unwrap();
    assert_eq!(u.npc_text(), None);
    u.apply_output(&npc_text(1, 1, 0, 9), &w).unwrap();
    u.apply_output(&npc_text(0xFF, 1, 0, 9), &w).unwrap();
    assert_eq!(u.npc_text(), None);
    // 0x50 code 2 passes a hire-table entry the popup never reads: the
    // popup takes no entry.
    let mut h = HirePopup::default();
    h.open(None);
    assert!(h.popup);
    // The per-frame bubble box field +0x16 starts at 0 (drawn): D2RUST uses
    // 0 (open question 2).
    assert_eq!(BubbleBox::new(&wide("x"), &m).flag, 0);
}

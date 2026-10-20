//! Quest-log tests from the spec's synthetic test vectors and the rule text.

use super::icons::*;
use super::state::*;
use super::tables::*;
use super::*;

fn ctx<'a>(p: &'a QuestFlags, g: Option<&'a QuestFlags>) -> RowCtx<'a> {
    RowCtx {
        p,
        g,
        multiplayer: false,
        den: 0,
        barbarians: 0,
    }
}

fn row(q: u8, l: u8, p: &QuestFlags, g: Option<&QuestFlags>) -> Row {
    derive_row(q, l, &ctx(p, g), &mut 0)
}

fn all_loaded(_: u8) -> bool {
    true
}

fn gate(exp: bool) -> TabGate<'static> {
    TabGate {
        expansion_installed: exp,
        expansion_game: exp,
        cel_loaded: &all_loaded,
    }
}

#[test]
fn msg_0x52_copies_status_and_opens_tab() {
    let mut log = QuestLog::new();
    let mut bytes = [0u8; 42];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = i as u8;
    }
    log.flag_2b0 = 9;
    log.latch = 2;
    let fx = log.receive_0x52(&bytes, true, 3);
    assert_eq!(log.status[0], 1);
    assert_eq!(log.status[40], 41);
    assert_eq!(log.flag_2b0, 0);
    assert_eq!(
        fx,
        vec![
            LogEffect::LoadPanelCels,
            LogEffect::OpenTab {
                tab: 3,
                reset: true
            }
        ]
    );
    log.latch = 1;
    assert!(log.receive_0x52(&bytes, true, 3).is_empty());
    log.latch = 2;
    assert!(log.receive_0x52(&bytes, false, 3).is_empty());
}

#[test]
fn msg_0x50_counters() {
    let mut log = QuestLog::new();
    log.receive_0x50(1, 3, 4, 5);
    assert_eq!((log.den, log.tomb, log.barbarians), (3, 4, 5));
    log.receive_0x50(2, 9, 9, 9);
    assert_eq!((log.den, log.tomb, log.barbarians), (3, 4, 5));
}

#[test]
fn msg_0x5d_writes_by_chain_only_while_open() {
    let mut log = QuestLog::new();
    // Chain 8 is entry 9 (Radament).
    assert_eq!(log.receive_0x5d(8, 2, false), None);
    assert_eq!(log.status[9], 0);
    assert_eq!(log.receive_0x5d(8, 2, true), Some(9));
    assert_eq!(log.status[9], 2);
    // Chain 30 (respec) writes the never-shown S[34]; chain 14 has no entry.
    assert_eq!(log.receive_0x5d(30, 1, true), Some(34));
    assert_eq!(log.status[34], 1);
    assert_eq!(log.receive_0x5d(14, 1, true), None);
    assert_eq!(log.receive_0x5d(21, 1, true), None);
}

#[test]
fn flags_kept_as_received() {
    let p = QuestFlags::new().with(1, 0xE001).with(2, 0x8000);
    assert!(p.bit(1, 13) && p.bit(1, 14) && p.bit(1, 15) && p.bit(1, 0));
    assert!(!p.bit(1, 1));
    assert_eq!(p.word(1), 0xE001);
    assert!(p.bit(2, 15));
}

#[test]
fn resets() {
    let mut log = QuestLog::new();
    log.status = [7; ENTRY_COUNT];
    log.last = [7; ENTRY_COUNT];
    log.den = 3;
    log.latch = 2;
    log.selected = Some(4);
    log.reset_status();
    assert!(log.status.iter().all(|&s| s == 0) && log.last.iter().all(|&s| s == 0));
    assert_eq!(log.den, 3);
    log.status = [7; ENTRY_COUNT];
    log.reset_game();
    assert_eq!((log.den, log.latch, log.selected), (0, 0, Some(0)));
    assert!(log.status.iter().all(|&s| s == 0));
}

#[test]
fn entry_table() {
    let enabled: Vec<u8> = (0..41).filter(|&i| entry(i).enabled).collect();
    assert_eq!(enabled.len(), 27);
    for i in [0, 7, 8, 15, 16, 23, 24, 28, 29, 30, 31, 32, 33, 34] {
        assert!(!entry(i).enabled, "{i}");
    }
    assert_eq!(entry(34).chain, 30);
    assert_eq!(entry(34).tab, 4);
    assert_eq!(entry(0).chain, CHAIN_NONE);
    assert_eq!(entry(0).tab, TAB_NONE);
    // Slot grid, (tab, slot) -> entry.
    let grid: [[u8; 6]; 5] = [
        [1, 2, 4, 5, 3, 6],
        [9, 10, 11, 12, 13, 14],
        [20, 19, 18, 17, 21, 22],
        [25, 27, 26, 0, 0, 0],
        [35, 36, 37, 38, 39, 40],
    ];
    for (tab, slots) in grid.iter().enumerate() {
        for (slot, &e) in slots.iter().enumerate() {
            if e != 0 {
                let en = entry(e);
                assert_eq!((en.tab, en.slot), (tab as u8, slot as u8), "entry {e}");
            }
        }
    }
    // Icons and names.
    assert_eq!(ICON_NAMES[entry(1).icon as usize], "a1q1");
    assert_eq!(ICON_NAMES[entry(26).icon as usize], "a4q3");
    assert_eq!(ICON_NAMES[entry(27).icon as usize], "a4q2");
    assert_eq!(ICON_NAMES[entry(40).icon as usize], "a5q6");
    assert_eq!(ICON_NAMES.len(), 27);
    assert_eq!(
        TAB_ICON_RANGE,
        [(0, 5), (6, 11), (12, 17), (18, 20), (21, 26)]
    );
    // Chains; quest id = index.
    assert_eq!(entry(9).chain, 8);
    assert_eq!(entry(35).chain, 31);
    assert_eq!(chain_entry(8), Some(9));
    assert_eq!(chain_entry(30), Some(34));
    assert_eq!(chain_entry(14), None);
    assert!((0..41).all(|i| entry(i).filter == i));
}

#[test]
fn status_table_layout() {
    assert_eq!(TABLES.len(), 27);
    let t = table_of(1).unwrap();
    assert_eq!(t.rows.len(), 15);
    assert_eq!(t.rows[0], (NULL_STRING, NULL_STRING));
    assert_eq!(t.title, 3714);
    assert_eq!(t.completed, 76);
    assert_eq!(t.pending, 4);
    assert_eq!(table_of(3).unwrap().pending, 0xFFFF);
    assert_eq!(t.row(1), (3735, 64));
    // Status 6..9 of Den of Evil are null rows.
    assert_eq!(t.row(7), (3725, 3725));
}

#[test]
fn tab_build_filters_and_orders() {
    let mut log = QuestLog::new();
    let p = QuestFlags::new();
    let rows = log.build_tab(0, &p, None, false, &gate(false));
    let q: Vec<u8> = rows.iter().map(|r| r.row.quest).collect();
    assert_eq!(q, vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(rows[2].slot, 4);
    // A cel that is not loaded hides its entry.
    let none = |k: u8| k != 1;
    let g = TabGate {
        expansion_installed: false,
        expansion_game: false,
        cel_loaded: &none,
    };
    let rows = log.build_tab(0, &p, None, false, &g);
    assert_eq!(rows.len(), 5);
    assert!(rows.iter().all(|r| r.row.quest != 2));
    // Act IV has three rows.
    assert_eq!(log.build_tab(3, &p, None, false, &gate(false)).len(), 3);
}

#[test]
fn tab_open_selection() {
    let mut log = QuestLog::new();
    let p = QuestFlags::new();
    // Act V needs the expansion and an expansion game.
    assert_eq!(log.open_tab(4, false, &p, None, false, &gate(false)), None);
    assert!(log.build_tab(4, &p, None, false, &gate(false)).is_empty());
    assert_eq!(log.build_tab(4, &p, None, false, &gate(true)).len(), 6);
    // A changed row wins: Den of Evil status 1 (changed, last 0).
    log.status[2] = 1;
    assert_eq!(
        log.open_tab(0, false, &p, None, false, &gate(false)),
        Some(1)
    );
    // A remembered slot is kept unless reset.
    log.remembered[0] = Some(5);
    assert_eq!(
        log.open_tab(0, false, &p, None, false, &gate(false)),
        Some(5)
    );
    // (the remembered slot still wins on the call that resets; the slots are
    // cleared after the selection was made.)
    assert_eq!(
        log.open_tab(0, true, &p, None, false, &gate(false)),
        Some(5)
    );
    assert_eq!(log.remembered, [None; 5]);
    assert_eq!(log.selected, Some(5));
    assert_eq!(
        log.open_tab(0, false, &p, None, false, &gate(false)),
        Some(1)
    );
    // A refused Act V tab: selection none, remembered untouched.
    log.remembered[2] = Some(3);
    assert_eq!(log.open_tab(4, true, &p, None, false, &gate(false)), None);
    assert_eq!(log.selected, None);
    assert_eq!(log.remembered[2], Some(3));
    // No changed row and no state 0: the clicked slot, else the first state 3.
    let mut log = QuestLog::new();
    log.status[3] = 1;
    log.last[3] = 1; // not changed
    let _ = log.open_tab(0, false, &p, None, false, &gate(false));
    let mut log2 = QuestLog::new();
    log2.status[3] = 1;
    log2.last[3] = 1;
    log2.clicked[0] = Some(2);
    let mut lp = log2.clone();
    // Entries other than 3 are state 2 (not available) with status 0.
    assert_eq!(
        lp.open_tab(0, false, &p, None, false, &gate(false)),
        Some(2)
    );
    // The scan visits all 41 entries: a changed row past slot 5 still wins
    // (Act I has only 6 entries; Act II selection comes from its own tab).
    let mut log3 = QuestLog::new();
    log3.status[14] = 1;
    assert_eq!(
        log3.open_tab(1, false, &p, None, false, &gate(false)),
        Some(5)
    );
}

#[test]
fn description_pane_rules() {
    assert!(!title_drawn(3724));
    assert!(title_drawn(3714));
    assert_eq!(TEXT_WRAP_PX, 270);
    assert!(text_drawn(false));
    assert!(!text_drawn(true));
    assert!(!replay_plays(3724));
    assert!(!replay_plays(3725));
    assert!(replay_plays(64));
}

#[test]
fn siege_with_p1_reads_zero_as_four() {
    // Test vector: q 35, L 0, P {35: 0x0002} -> text 21786, speech 20090,
    // shown 0, state 3.
    let p = QuestFlags::new().with(35, 0x0002);
    let r = row(35, 0, &p, None);
    assert_eq!(r.text, RowText::Id(21786));
    assert_eq!(r.speech, 20090);
    assert_eq!(r.shown, 0);
    assert_eq!(r.icon, IconState::InProgress);
    assert_eq!(r.title, 22618);
    // Siege without P.1 takes the generic rules.
    let r = row(35, 1, &QuestFlags::new(), None);
    assert_eq!(r.text, RowText::Id(22619));
}

#[test]
fn seven_tombs_rows() {
    // Test vector: q 14, L 1, P {12: 0x0001} -> shown 7, text 961, speech
    // 396, state 3.
    let p = QuestFlags::new().with(12, 0x0001);
    let mut last = 0;
    let r = derive_row(14, 1, &ctx(&p, None), &mut last);
    assert_eq!((r.shown, r.text, r.speech), (7, RowText::Id(961), 396));
    assert_eq!(r.icon, IconState::InProgress);
    assert!(!r.changed);
    assert_eq!(last, 7);
    // P[14].3 -> 5, P[14].4 -> 6.
    let p = QuestFlags::new().with(14, 0x0008);
    assert_eq!(row(14, 0, &p, None).shown, 5);
    let p = QuestFlags::new().with(14, 0x0010);
    assert_eq!(row(14, 0, &p, None).shown, 6);
    // P.13 -> reward pending (pending 4, P.1 needed).
    let p = QuestFlags::new().with(14, 0x2002);
    let r = row(14, 0, &p, None);
    assert_eq!(r.shown, 5);
    assert!(r.changed);
    // Otherwise rule 5 / 6: P.1 clear, received status shown.
    let r = row(14, 2, &QuestFlags::new(), None);
    assert_eq!((r.shown, r.text), (2, RowText::Id(962)));
}

#[test]
fn generic_completed_rows() {
    // Test vector: q 1, L 0, P {1: 0x2001} -> shown 13, text 3726, speech 76,
    // state 0.
    let p = QuestFlags::new().with(1, 0x2001);
    let mut last = 0;
    let r = derive_row(1, 0, &ctx(&p, None), &mut last);
    assert_eq!((r.shown, r.text, r.speech), (13, RowText::Id(3726), 76));
    assert_eq!(r.icon, IconState::JustCompleted);
    assert_eq!(last, 13);
    assert_eq!(r.title, 3714);
    // P.12 set: completed icon.
    let p = QuestFlags::new().with(1, 0x3001);
    assert_eq!(row(1, 0, &p, None).icon, IconState::Completed);
    // P.0 without P.13: C(11, 3728).
    let p = QuestFlags::new().with(1, 0x0001);
    let r = row(1, 0, &p, None);
    assert_eq!((r.shown, r.text), (11, RowText::Id(3728)));
}

#[test]
fn reward_pending_row() {
    // Test vector: q 2, L 0, P {2: 0x2002} -> shown 3, text 3743, speech 81,
    // changed 1, state 3.
    let p = QuestFlags::new().with(2, 0x2002);
    let mut last = 0;
    let r = derive_row(2, 0, &ctx(&p, None), &mut last);
    assert_eq!((r.shown, r.text, r.speech), (3, RowText::Id(3743), 81));
    assert!(r.changed);
    assert_eq!(r.icon, IconState::InProgress);
    assert_eq!(last, 3);
    // Same step again: not changed.
    let r = derive_row(2, 0, &ctx(&p, None), &mut last);
    assert!(!r.changed);
    // Fallen Angel skips rule 4: P.13 + P.1 with word 2 none -> rule 5.
    let p = QuestFlags::new().with(25, 0x2002);
    let r = row(25, 0, &p, None);
    assert_eq!(r.icon, IconState::NotAvailable);
    // P.1 clear -> rule 6 (status 0: Z, state 2 here).
    let p = QuestFlags::new().with(2, 0x2000);
    assert_eq!(row(2, 0, &p, None).icon, IconState::NotAvailable);
}

#[test]
fn reward_still_pending_rows() {
    // q 38, L 0, P {38: 0x8012} -> shown 5, text 21790, speech 20148.
    let p = QuestFlags::new().with(38, 0x8012);
    let r = row(38, 0, &p, None);
    assert_eq!((r.shown, r.text, r.speech), (5, RowText::Id(21790), 20148));
    assert_eq!(r.icon, IconState::InProgress);
    // Without P.4 the row is 4.
    let p = QuestFlags::new().with(38, 0x8002);
    assert_eq!(row(38, 0, &p, None).shown, 4);
    // q 27: row 10, completed icon.
    let p = QuestFlags::new().with(27, 0x8002);
    let r = row(27, 0, &p, None);
    assert_eq!(
        (r.shown, r.text, r.icon),
        (10, RowText::Id(3728), IconState::Completed)
    );
    // q 37: P.8 set, P.9 clear -> 6; P.8 clear -> 5; both -> 10.
    let p = QuestFlags::new().with(37, 0x8102);
    assert_eq!(row(37, 0, &p, None).shown, 6);
    let p = QuestFlags::new().with(37, 0x8002);
    assert_eq!(row(37, 0, &p, None).shown, 5);
    let p = QuestFlags::new().with(37, 0x8302);
    assert_eq!(row(37, 0, &p, None).shown, 10);
    // A null row 10 is state 2 (Den of Evil row 10 is real; Terror's End is null).
    let p = QuestFlags::new().with(26, 0x8002);
    let r = row(26, 0, &p, None);
    assert_eq!((r.shown, r.icon), (10, IconState::NotAvailable));
    // P.15 clear -> rule 6.
    let p = QuestFlags::new().with(38, 0x0002);
    assert_eq!(row(38, 3, &p, None).shown, 3);
}

#[test]
fn received_status_or_zero() {
    let p = QuestFlags::new();
    // L != 0 -> M.
    assert_eq!(row(9, 2, &p, None).text, RowText::Id(942));
    // L == 0 -> Z: test vector q 9, L 0, P {}, G {} -> state 2.
    let g = QuestFlags::new();
    assert_eq!(row(9, 0, &p, Some(&g)).icon, IconState::NotAvailable);
    assert_eq!(row(9, 0, &p, None).icon, IconState::NotAvailable);
}

#[test]
fn received_status_rows() {
    let p = QuestFlags::new();
    // Test vector: q 1, L 1 -> title 3714, text 3735, speech 64, changed 1,
    // state 3.
    let r = row(1, 1, &p, None);
    assert_eq!(
        (r.title, r.text, r.speech, r.changed, r.icon),
        (3714, RowText::Id(3735), 64, true, IconState::InProgress)
    );
    // Rule 7.1: q 1, L 4, D 3 -> text 3738 + "3"; D 1 -> 3739.
    let p1 = QuestFlags::new().with(1, 0x0004);
    let mut c = ctx(&p1, None);
    c.den = 3;
    let r = derive_row(1, 4, &c, &mut 0);
    assert_eq!((r.shown, r.text), (4, RowText::Append(3738, 3)));
    c.den = 1;
    assert_eq!(derive_row(1, 4, &c, &mut 0).text, RowText::Id(3739));
    // L 1 is not a count row.
    assert_eq!(derive_row(1, 1, &c, &mut 0).text, RowText::Id(3735));
    // Rule 7.2: q 36, L 2, B 3 -> 22624 formatted with 3, speech 20104.
    let mut c = ctx(&p, None);
    c.barbarians = 3;
    let r = derive_row(36, 2, &c, &mut 0);
    assert_eq!((r.text, r.speech), (RowText::Format(22624, 3), 20104));
    assert_eq!(r.icon, IconState::InProgress);
    // B = 0 -> 3729.
    c.barbarians = 0;
    assert_eq!(derive_row(36, 2, &c, &mut 0).text, RowText::Id(3729));
    // Test vector: q 5, L 12 -> 3729 (3727 replaced in single player).
    assert_eq!(row(5, 12, &p, None).text, RowText::Id(3729));
    let mut c = ctx(&p, None);
    c.multiplayer = true;
    assert_eq!(derive_row(5, 12, &c, &mut 0).text, RowText::Id(3727));
    // "Done by the game" (G.13, P.0/13/1 clear, q != 19) and P.14.
    let g = QuestFlags::new().with(5, 0x2000);
    assert_eq!(row(5, 1, &p, Some(&g)).text, RowText::Id(3729));
    let g19 = QuestFlags::new().with(19, 0x2000);
    assert_eq!(row(19, 2, &p, Some(&g19)).text, RowText::Id(977));
    let p14 = QuestFlags::new().with(5, 0x4000);
    let mut c = ctx(&p14, None);
    c.multiplayer = true;
    assert_eq!(derive_row(5, 1, &c, &mut 0).text, RowText::Id(3730));
    // L == 13 with P.0 clear: icon 0, or 1 with P.12.
    let r = row(2, 13, &QuestFlags::new(), None);
    assert_eq!(r.icon, IconState::JustCompleted);
    let r = row(2, 13, &QuestFlags::new().with(2, 0x1000), None);
    assert_eq!(r.icon, IconState::Completed);
    // changed / last.
    let mut last = 2;
    let r = derive_row(2, 2, &ctx(&p, None), &mut last);
    assert!(!r.changed);
}

#[test]
fn status_zero_rows() {
    let p = QuestFlags::new();
    // Test vector: q 3, L 0, G {3: 0x2000}, multiplayer -> title 3724, text
    // 3730, speech 3725, state 3.
    let g = QuestFlags::new().with(3, 0x2000);
    let mut c = ctx(&p, Some(&g));
    c.multiplayer = true;
    let r = derive_row(3, 0, &c, &mut 0);
    assert_eq!(
        (r.title, r.text, r.speech, r.icon),
        (3724, RowText::Id(3730), 3725, IconState::InProgress)
    );
    // Single player: 3729.
    assert_eq!(row(3, 0, &p, Some(&g)).text, RowText::Id(3729));
    // G.13 with P.1 or P.13 -> state 2.
    let p1 = QuestFlags::new().with(3, 0x0002);
    assert_eq!(row(3, 0, &p1, Some(&g)).icon, IconState::NotAvailable);
    // q 21, P.0 clear, P.4 set -> text 989.
    let g21 = QuestFlags::new().with(21, 0x2000);
    let p21 = QuestFlags::new().with(21, 0x0010);
    assert_eq!(row(21, 0, &p21, Some(&g21)).text, RowText::Id(989));
    // G.13 clear: G.15 clear -> state 2; G.15 set and P clear -> text.
    let g15 = QuestFlags::new().with(3, 0x8000);
    assert_eq!(row(3, 0, &p, Some(&g15)).text, RowText::Id(3729));
    assert_eq!(row(3, 0, &p1, Some(&g15)).icon, IconState::NotAvailable);
    // A quest with no table is state 2.
    assert_eq!(row(7, 1, &p, None).icon, IconState::NotAvailable);
}

#[test]
fn just_completed_animation() {
    let mut a = IconAnim::default();
    // The first draw only stamps.
    let e = a.step(1, 5000, true);
    assert_eq!((e.frame, e.sound, e.acknowledge), (0, false, None));
    assert_eq!((a.stamp, a.counter), (5000, 0));
    // Not more than 100 ms: no step.
    let e = a.step(1, 5100, true);
    assert_eq!((e.frame, e.sound, a.counter), (0, false, 0));
    // The frame is the counter before the step; the sound is the step's.
    let e = a.step(1, 5101, true);
    assert_eq!((e.frame, e.sound, a.counter), (0, true, 1));
    let e = a.step(1, 5150, true);
    assert_eq!((e.frame, e.sound, e.acknowledge), (1, false, None));
    // With counter 24 a stepping draw still shows frame 24 and does not
    // acknowledge; the next draw does.
    let mut c = IconAnim {
        counter: 24,
        stamp: 1000,
    };
    let e = c.step(7, 1200, true);
    assert_eq!((e.frame, e.acknowledge, c.counter), (24, None, 25));
    let e = c.step(7, 1201, true);
    assert_eq!((e.frame, e.acknowledge), (24, Some(7)));
    // No sound without the expansion.
    let mut b = IconAnim::default();
    b.step(1, 500, false);
    assert!(!b.step(1, 700, false).sound);
    // Set P.12 and send for every state-0 row.
    let p = QuestFlags::new().with(1, 0x2001);
    let rows = [row(1, 0, &p, None), row(2, 1, &QuestFlags::new(), None)];
    let mut pm = p.clone();
    assert_eq!(acknowledge_all(&rows, &mut pm), vec![1]);
    assert!(pm.bit(1, 12));
}

#[test]
fn completed_icon() {
    assert_eq!(
        icon_frame(IconState::Completed, false, 3, 0),
        (IconCel::Icon, 24)
    );
    assert_eq!(
        icon_frame(IconState::Completed, true, 3, 0),
        (IconCel::QuestDone, 3)
    );
}

#[test]
fn not_available_icon() {
    assert_eq!(
        icon_frame(IconState::NotAvailable, false, 3, 0),
        (IconCel::Icon, 26)
    );
    assert!(!draws_title_and_text(IconState::NotAvailable));
    assert!(draws_title_and_text(IconState::InProgress));
}

#[test]
fn in_progress_icon() {
    assert_eq!(
        icon_frame(IconState::InProgress, false, 3, 0),
        (IconCel::Icon, 0)
    );
    assert_eq!(
        icon_frame(IconState::InProgress, true, 3, 0),
        (IconCel::Icon, 25)
    );
}

#[test]
fn selection_frames() {
    assert_eq!(selection_frame(true), 1);
    assert_eq!(selection_frame(false), 0);
}

#[test]
fn tomb_symbol_placement() {
    let p = QuestFlags::new().with(12, 1);
    let r = row(14, 1, &p, None);
    assert_eq!((r.title, r.shown), (928, 7));
    assert_eq!(
        tomb_symbol(&r, 5, 3, 10, 400, 600),
        Some((3, 10 + 0x6E, 400 - 0x6E + 600))
    );
    assert_eq!(tomb_symbol(&r, 5, 9, 10, 400, 600).unwrap().0, 0);
    assert_eq!(tomb_symbol(&r, 4, 3, 10, 400, 600), None);
    let r2 = row(14, 2, &QuestFlags::new(), None);
    assert_eq!(tomb_symbol(&r2, 5, 3, 10, 400, 600), None);
}

fn avail(set: &[u8]) -> [u8; 37] {
    let mut a = [0u8; 37];
    for &c in set {
        a[c as usize] = 1;
    }
    a
}

fn check(
    c: u8,
    a: Option<&[u8; 37]>,
    p: Option<&QuestFlags>,
    g: Option<&QuestFlags>,
    l1: u8,
) -> Result<bool, QuestCheckError> {
    let mut status = [0u8; ENTRY_COUNT];
    status[1] = l1;
    let mut last = [0u8; ENTRY_COUNT];
    quest_check(
        c,
        &QuestCheckCtx {
            availability: a,
            p,
            g,
            multiplayer: false,
            den: 0,
            barbarians: 0,
        },
        &status,
        &mut last,
    )
}

#[test]
fn check_availability_byte() {
    let g = QuestFlags::new();
    let p = QuestFlags::new();
    assert_eq!(
        check(2, None, Some(&p), Some(&g), 0),
        Err(QuestCheckError::NoAvailability)
    );
    let a = avail(&[2]);
    assert_eq!(
        check(37, Some(&a), Some(&p), Some(&g), 0),
        Err(QuestCheckError::OutOfRange)
    );
    // Test vector: c 2, 0x5E byte 2 = 0 -> 0.
    let none = avail(&[]);
    assert_eq!(check(2, Some(&none), Some(&p), Some(&g), 0), Ok(false));
    assert_eq!(check(2, Some(&a), Some(&p), Some(&g), 0), Ok(true));
    // The byte is indexed by c directly: c 12 reads byte 12.
    let a12 = avail(&[12]);
    assert_eq!(check(12, Some(&a12), Some(&p), Some(&g), 0), Ok(true));
    assert_eq!(check(13, Some(&a12), Some(&p), Some(&g), 0), Ok(false));
}

#[test]
fn check_entry_by_chain() {
    let g = QuestFlags::new();
    let p = QuestFlags::new();
    // c 12 tests entry 13 (chain 12); c 13 tests entry 14.
    let a = avail(&[12, 13]);
    let p13 = QuestFlags::new().with(13, 1);
    assert_eq!(check(12, Some(&a), Some(&p13), Some(&g), 0), Ok(false));
    assert_eq!(check(13, Some(&a), Some(&p13), Some(&g), 0), Ok(true));
    // A chain with no entry (c 14) gives 0.
    let a14 = avail(&[14]);
    assert_eq!(check(14, Some(&a14), Some(&p), Some(&g), 0), Ok(false));
}

#[test]
fn check_game_record() {
    let p = QuestFlags::new();
    let a = avail(&[34]);
    // Test vector: c 34, G {38: 0x2000} -> 0 (G.13).
    let g = QuestFlags::new().with(38, 0x2000);
    assert_eq!(check(34, Some(&a), Some(&p), Some(&g), 0), Ok(false));
    assert_eq!(
        check(34, Some(&a), Some(&p), Some(&QuestFlags::new()), 0),
        Ok(true)
    );
    // G not received -> 0.
    assert_eq!(check(34, Some(&a), Some(&p), None, 0), Ok(false));
}

#[test]
fn check_player_record() {
    let g = QuestFlags::new();
    // Test vector: c 5, P {5: 0x4000} -> 0 (P.14).
    let a = avail(&[5, 12, 2]);
    let p = QuestFlags::new().with(5, 0x4000);
    assert_eq!(check(5, Some(&a), Some(&p), Some(&g), 0), Ok(false));
    let p = QuestFlags::new().with(13, 0x0001);
    assert_eq!(check(12, Some(&a), Some(&p), Some(&g), 0), Ok(false));
    let p = QuestFlags::new().with(2, 0x0002);
    assert_eq!(check(2, Some(&a), Some(&p), Some(&g), 0), Ok(false));
    assert_eq!(check(2, Some(&a), None, Some(&g), 0), Ok(false));
}

#[test]
fn check_den_of_evil_row() {
    let g = QuestFlags::new();
    let a = avail(&[1]);
    let p = QuestFlags::new().with(1, 0x0004);
    // Test vectors: shown 1 -> 1; shown 5 -> 0.
    assert_eq!(check(1, Some(&a), Some(&p), Some(&g), 1), Ok(true));
    assert_eq!(check(1, Some(&a), Some(&p), Some(&g), 5), Ok(false));
    // The row build rewrites last[1].
    let mut status = [0u8; ENTRY_COUNT];
    status[1] = 4;
    let mut last = [0u8; ENTRY_COUNT];
    let c = QuestCheckCtx {
        availability: Some(&a),
        p: Some(&p),
        g: Some(&g),
        multiplayer: false,
        den: 3,
        barbarians: 0,
    };
    assert_eq!(quest_check(1, &c, &status, &mut last), Ok(true));
    assert_eq!(last[1], 4);
    // Other c skip the row build.
    let a2 = avail(&[2]);
    let mut last = [0u8; ENTRY_COUNT];
    let c2 = QuestCheckCtx {
        availability: Some(&a2),
        ..c
    };
    assert_eq!(quest_check(2, &c2, &[5; ENTRY_COUNT], &mut last), Ok(true));
    assert_eq!(last[2], 0);
}

#[test]
fn check_returns_one() {
    let g = QuestFlags::new();
    let p = QuestFlags::new();
    for c in [1u8, 2, 3, 5, 6, 12, 13, 31, 34, 35, 36] {
        let a = avail(&[c]);
        assert_eq!(check(c, Some(&a), Some(&p), Some(&g), 0), Ok(true), "c {c}");
    }
}

#[test]
fn edge_siege_shown_status_is_received() {
    let p = QuestFlags::new().with(35, 0x0002);
    let mut last = 0;
    let r = derive_row(35, 0, &ctx(&p, None), &mut last);
    assert_eq!(r.shown, 0);
    assert_eq!(last, 4);
    // Icon 6 (tomb) reads the shown status only; Siege's stays received.
    assert_eq!(tomb_symbol(&r, 5, 0, 0, 0, 600), None);
}

#[test]
fn edge_37_and_38_tests_independent() {
    // Quest 37 never takes the 38 test and vice versa.
    let p = QuestFlags::new().with(37, 0x8012).with(38, 0x8102);
    assert_eq!(row(37, 0, &p, None).shown, 5);
    assert_eq!(row(38, 0, &p, None).shown, 4);
    assert_eq!(
        row(37, 0, &QuestFlags::new().with(37, 0x8012), None).shown,
        5
    );
    assert_eq!(
        row(38, 0, &QuestFlags::new().with(38, 0x8102), None).shown,
        4
    );
}

#[test]
fn edge_rescue_zero_barbarians() {
    let p = QuestFlags::new();
    let mut c = ctx(&p, None);
    c.multiplayer = true;
    c.barbarians = 0;
    assert_eq!(derive_row(36, 2, &c, &mut 0).text, RowText::Id(3729));
}

#[test]
fn edge_blade_table_shifted() {
    let t = table_of(19).unwrap();
    assert_eq!(t.row(12).0, 3728);
    assert_eq!(t.row(13).0, 3727);
    assert_eq!(t.row(14).0, 3726);
    // A received 11 with P.0 clear reaches the table; 11 itself is null.
    assert_eq!(t.row(11), (NULL_STRING, NULL_STRING));
    // With P.0 set the generic rule wins.
    let p = QuestFlags::new().with(19, 0x0001);
    assert_eq!(row(19, 13, &p, None).text, RowText::Id(3728));
}

#[test]
fn edge_3727_replacement_only_rule_7_2() {
    let p = QuestFlags::new();
    // Den of Evil status 12 keeps 3727 in single player.
    assert_eq!(row(1, 12, &p, None).text, RowText::Id(3727));
    assert_eq!(row(2, 12, &p, None).text, RowText::Id(3729));
}

#[test]
fn edge_respec_chain_writes_hidden_slot() {
    let mut log = QuestLog::new();
    assert_eq!(log.receive_0x5d(30, 3, true), Some(34));
    let p = QuestFlags::new();
    for tab in 0..5 {
        let rows = log.build_tab(tab, &p, None, false, &gate(true));
        assert!(rows.iter().all(|r| r.row.quest != 34));
    }
}

#[test]
fn edge_status_past_table_is_null_row() {
    let p = QuestFlags::new();
    let r = row(1, 15, &p, None);
    assert_eq!(r.text, RowText::Id(3725));
    assert_eq!(r.speech, 3725);
}

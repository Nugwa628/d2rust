use super::*;
use crate::bridge::output::Output;
use crate::bridge::world::{ClientUnit, MONSTER, PLAYER};
use crate::ui::layout::Screen;
use crate::ui::original::UiConfig;
use crate::ui::{NoPanelRules, UiRoot};

const AKARA: u32 = 148;
const CHARSI: u32 = 154;
const NPC: u32 = 77;
const ME: UnitKey = UnitKey {
    unit_type: PLAYER,
    guid: 1,
};
/// The gossip text: speed line "4", then two lines.
const GOSSIP: u16 = 900;
/// A second kind-0 line.
const NEXT_LINE: u16 = 901;

struct Strs(Vec<(u16, Vec<u16>)>);

fn w16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

impl Strs {
    fn new() -> Self {
        let mut v = vec![
            (3381, w16("Talk")),
            (3399, w16("introduction")),
            (3395, w16("gossip")),
            (3400, w16("cancel")),
            (3724, w16("Invalid Quest Value")),
            (GOSSIP, w16("4\nHello\nThere")),
            (NEXT_LINE, w16("4\nNext")),
        ];
        // Gossip plays a text record of Akara's intro entry (facts): every
        // one of them reads as the same two lines here.
        for r in &crate::ui::messages::npc_facts::intro_table().entries[0].records {
            v.push((r.text(), w16("4\nHello\nThere")));
        }
        Strs(v)
    }
}

impl StringLookup for Strs {
    fn get(&self, _: &str) -> Option<&[u16]> {
        None
    }
    fn get_id(&self, id: u16) -> Option<&[u16]> {
        self.0.iter().find(|(i, _)| *i == id).map(|(_, t)| &t[..])
    }
}

fn world(class: u32) -> ClientWorld {
    let mut w = ClientWorld::default();
    let mut p = ClientUnit::new(ME);
    p.position = Some((1000, 1000));
    p.stats.insert(12, 1);
    // Mode 1 (neutral): alive for the gate.
    p.mode = 1;
    w.units.insert(ME, p);
    w.local_player = Some(ME);
    let mut n = ClientUnit::new(UnitKey::new(MONSTER, NPC));
    n.class = class;
    n.position = Some((1004, 1000));
    w.units.insert(n.key, n);
    w.frames = 100;
    w.drlg_updates = 100;
    w
}

fn setup() -> (OriginalUi, UiRoot) {
    let config = UiConfig {
        screen: Screen::R800,
        expansion_installed: true,
    };
    let ui = OriginalUi::new(config, None).unwrap();
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    ui.install(&mut root).unwrap();
    (ui, root)
}

fn send(ui: &mut OriginalUi, root: &mut UiRoot, w: &ClientWorld, e: UiEvent) {
    let s = Strs::new();
    let ctx = UiCtx {
        tick: w.frames,
        world: w,
        strings: &s,
    };
    ui.before_event(e, w);
    let r = root.dispatch(e, &ctx);
    ui.after_event(root, e, r).unwrap();
}

fn click(ui: &mut OriginalUi, root: &mut UiRoot, w: &ClientWorld, at: Point) {
    let button = PointerButton::Left;
    send(ui, root, w, UiEvent::Press { button, at });
    send(ui, root, w, UiEvent::Release { button, at });
}

fn poll(ui: &mut OriginalUi, root: &mut UiRoot, w: &ClientWorld) {
    ui.npc_menu_poll(w, root, &Strs::new());
}

/// S→C 0x27 type 1 for the NPC: entries (kind, string id).
fn text_list(entries: &[(u8, u16)]) -> Output {
    let mut b = [0u8; 40];
    b[0] = 0x27;
    b[1] = 1;
    b[2..6].copy_from_slice(&NPC.to_le_bytes());
    b[6] = entries.len() as u8;
    for (k, &(kind, id)) in entries.iter().enumerate() {
        b[6 + 2 + 4 * k] = kind;
        b[6 + 4 + 4 * k..6 + 6 + 4 * k].copy_from_slice(&id.to_le_bytes());
    }
    Output::NpcText {
        bytes: b,
        present: true,
        object_class: 0,
    }
}

fn texts(v: &[Vec<u16>]) -> Vec<String> {
    v.iter().map(|t| String::from_utf16_lossy(t)).collect()
}

/// Akara's menu with a text list of one kind-0 and one kind-2 entry,
/// then Talk.
fn talk_to_akara() -> (OriginalUi, UiRoot, ClientWorld) {
    let (mut ui, mut root) = setup();
    let w = world(AKARA);
    ui.apply_output(&text_list(&[(0, GOSSIP), (2, 501)]), &w)
        .unwrap();
    ui.open_npc_menu(NPC, AKARA, 1, &w);
    poll(&mut ui, &mut root, &w);
    let p = ui.npc_menu_row_point(0).unwrap();
    click(&mut ui, &mut root, &w, p);
    (ui, root, w)
}

// Talk opens the topic box: the caption, "introduction" (Akara
// has one), "gossip", one item per kind-2 entry (captioned through the
// 527-pair table: not in the repository, so the not-found 3724), cancel.
#[test]
fn talk_opens_the_topic_box_of_the_text_list() {
    let (ui, mut root, _) = talk_to_akara();
    assert!(ui.npc_menu_box().is_none(), "the menu box closed");
    assert!(root.take_intents().is_empty(), "talk sends nothing");
    let topics = ui.npc_topics().expect("the topic box");
    assert_eq!(
        texts(&topics),
        [
            "Talk",
            "introduction",
            "gossip",
            "Invalid Quest Value",
            "cancel"
        ]
    );
}

// A topic plays in the dialog panel: top centre ((W − 325) / 2, 12),
// the first line is the speed and is not shown; its backing rectangle;
// the scroll advances with the clock; a button press skips and
// the end callback brings the topic box back.
#[test]
fn a_topic_plays_in_the_dialog_panel_and_a_press_skips_it() {
    let (mut ui, mut root, mut w) = talk_to_akara();
    // "gossip" (the second selectable item) plays a text record of the
    // NPC's intro entry, not the list's kind-0 entry.
    let p = ui.npc_topic_point(1).unwrap();
    click(&mut ui, &mut root, &w, p);
    assert!(ui.npc_topics().is_none(), "the topic box is freed");
    let lines = ui.npc_dialog_lines().expect("the dialog panel");
    assert_eq!(texts(&lines), ["Hello", "There"]);
    let s = Strs::new();
    let draw = |ui: &OriginalUi, root: &UiRoot, w: &ClientWorld| {
        let _ = ui;
        let ctx = UiCtx {
            tick: w.frames,
            world: w,
            strings: &s,
        };
        let mut out: Vec<UiDraw> = Vec::new();
        root.draw(&ctx, &mut out);
        out
    };
    let x = (800 - 325) / 2;
    let out = draw(&ui, &root, &w);
    assert!(out.contains(&UiDraw::Rect(RectRequest::sized(x, 12 - 5, 325, 122, 0, 1))));
    // The scroll runs on the clock: no whole line at first, then a line.
    poll(&mut ui, &mut root, &w);
    for _ in 0..80 {
        w.frames += 1;
        w.drlg_updates += 1;
        poll(&mut ui, &mut root, &w);
    }
    let out = draw(&ui, &root, &w);
    assert!(
        out.iter().any(|d| matches!(d, UiDraw::Text(t)
            if String::from_utf16_lossy(&t.text) == "Hello" && t.style.font == PANEL_FONT)),
        "the first line scrolled in"
    );
    // A press skips (after the 100 ms grace) and the topic box is back.
    click(&mut ui, &mut root, &w, Point::new(400, 300));
    poll(&mut ui, &mut root, &w);
    assert!(ui.npc_dialog_lines().is_none());
    assert!(ui.npc_topics().is_some(), "the topic box again");
}

// B013 (playtest: heavy lag while an NPC text box is open). While the dialog
// scrolls, its partly visible lines are vertical-window text draws; the play
// preview drops a frame whose text does not lay out, and the vertical window
// was unanswered, so nearly every frame of a scroll was lost. Every frame's
// text must lay out.
#[test]
fn every_frame_of_a_scrolling_dialog_lays_out() {
    use crate::ui::text::{layout_text, OriginalText};
    use d2_formats::font::{FontTable, Glyph};
    let glyphs = (0..256u16)
        .map(|i| Glyph {
            code: i,
            unknown1: 0,
            width: 6,
            height: 10,
            unknown2: 1,
            unknown3: 0,
            frame: i,
            unknown5: 0,
        })
        .collect();
    let font = FontTable {
        version: 1,
        unknown: 0,
        count: 256,
        height: 10,
        width: 0,
        glyphs,
    };
    let (mut ui, mut root, mut w) = talk_to_akara();
    let p = ui.npc_topic_point(1).unwrap();
    click(&mut ui, &mut root, &w, p);
    let s = Strs::new();
    let (mut frames, mut windows, mut failed) = (0, 0, 0);
    while ui.npc_dialog_lines().is_some() && frames < 2000 {
        w.frames += 1;
        w.drlg_updates += 1;
        frames += 1;
        poll(&mut ui, &mut root, &w);
        let ctx = UiCtx {
            tick: w.frames,
            world: &w,
            strings: &s,
        };
        let mut out: Vec<UiDraw> = Vec::new();
        root.draw(&ctx, &mut out);
        for d in &out {
            let UiDraw::Text(t) = d else { continue };
            if matches!(t.opts, TextOpts::Vertical { skip, .. } if skip >= 0) {
                windows += 1;
            }
            if layout_text(&font, &t.text, t.at, t.style, &t.opts, &OriginalText).is_err() {
                failed += 1;
            }
        }
    }
    assert!(windows > 0, "the scroll draws partly visible lines");
    assert_eq!(
        failed, 0,
        "text draws that do not lay out ({frames} frames)"
    );
}

// Charsi's Imbue row opens the item-socket dialog (NPC mode; ui 0x0E; ui 8 off
// first); with an item placed, the imbue button sends C→S 0x38 [0][NPC][item]
// and the note opens; S→C 0x58 code 6 closes it and ends the interaction
// (0x30).
#[test]
fn imbue_runs_the_item_socket_dialog() {
    let (mut ui, mut root) = setup();
    let mut w = world(CHARSI);
    ui.open_npc_menu(NPC, CHARSI, 10, &w);
    poll(&mut ui, &mut root, &w);
    let rows = ui.npc_menu().unwrap().rows;
    let k = rows
        .iter()
        .position(|r| r.kind == Some(crate::ui::layout::OptionKind::Imbue))
        .unwrap();
    let p = ui.npc_menu_row_point(k).unwrap();
    click(&mut ui, &mut root, &w, p);
    assert!(!ui.is_open(8) && ui.is_open(UI_SOCKET));
    assert!(root.open_panels().contains(&SOCKET_PANEL));
    assert_eq!(ui.socket_step(), Some(Step::Open));
    // Its text is clipped to the configured screen.
    {
        let s = Strs::new();
        let ctx = UiCtx {
            tick: w.frames,
            world: &w,
            strings: &s,
        };
        let mut out: Vec<UiDraw> = Vec::new();
        root.draw(&ctx, &mut out);
        assert!(out
            .iter()
            .filter_map(|d| match d {
                UiDraw::Text(t) => Some(t.clip),
                _ => None,
            })
            .all(|c| c == Screen::R800.rect()));
    }
    ui.imbue_place(555);
    // No click within 400 ms of the open.
    w.frames += 10;
    // Button 0 (hit 122–154 × 224–256).
    click(&mut ui, &mut root, &w, Point::new(130, 230));
    let mut want = vec![0x38, 0, 0, 0, 0];
    want.extend_from_slice(&NPC.to_le_bytes());
    want.extend_from_slice(&555u32.to_le_bytes());
    assert_eq!(
        root.take_intents()
            .iter()
            .map(|i| i.0.clone())
            .collect::<Vec<_>>(),
        vec![want]
    );
    assert_eq!(ui.socket_step(), Some(Step::Waiting));
    ui.apply_output(
        &Output::OpenUi {
            guid: NPC,
            code: 6,
            arg: 0,
        },
        &w,
    )
    .unwrap();
    poll(&mut ui, &mut root, &w);
    assert_eq!(ui.socket_step(), None);
    assert!(!ui.is_open(UI_SOCKET));
    assert_eq!(root.take_intents()[0].0[0], 0x30, "the interaction ends");
}

// A unit dialog whose NPC is present in mode 12 (dead) closes at the text pass
// (the unit looked up as a monster, mode at +0x10).
#[test]
fn a_dialog_closes_when_its_npc_is_in_mode_12() {
    let (mut ui, mut root, mut w) = talk_to_akara();
    let p = ui.npc_topic_point(1).unwrap();
    click(&mut ui, &mut root, &w, p);
    assert!(ui.npc_dialog_lines().is_some(), "the dialog is up");
    w.frames += 1;
    w.drlg_updates += 1;
    poll(&mut ui, &mut root, &w);
    assert!(ui.npc_dialog_lines().is_some(), "an alive NPC keeps it up");
    w.units.get_mut(&UnitKey::new(MONSTER, NPC)).unwrap().mode =
        crate::bridge::modes::monster_mode::DEAD;
    w.frames += 1;
    w.drlg_updates += 1;
    poll(&mut ui, &mut root, &w);
    assert!(ui.npc_dialog_lines().is_none(), "mode 12 closes it");
}

// 0x28's dialog branch B2 plays the NPC's line before ui 8 goes on. The game
// draws the dialog panel in the text pass and skips it from the game window's
// handlers: with ui 8 closed the panel is drawn, a press skips it, and the end
// callback opens the menu (Akara's intro at y
// 123, a click at frame 60 skips it, her second talk opens the menu).
#[test]
fn the_b2_line_draws_and_skips_without_the_npc_menu() {
    let (mut ui, mut root) = setup();
    let mut w = world(AKARA);
    ui.apply_output(&text_list(&[(0, GOSSIP)]), &w).unwrap();
    ui.apply_output(
        &Output::NpcDialog(Box::new(crate::bridge::output::NpcDialog {
            kind: 1,
            guid: NPC,
            quest_flags: [0; 96],
            unit: UnitKey::new(MONSTER, NPC),
            class: AKARA,
            interact: true,
            f4b1a10: None,
            cursor_item: false,
            level: 1,
            unidentified: 0,
            expansion: false,
            npc_monsters: vec![UnitKey::new(MONSTER, NPC)],
        })),
        &w,
    )
    .unwrap();
    poll(&mut ui, &mut root, &w);
    assert!(!ui.is_open(8), "B2: no menu while the line plays");
    let lines = ui.npc_dialog_lines().expect("the dialog panel");
    assert_eq!(texts(&lines), ["Hello", "There"]);
    assert!(root.open_panels().contains(&DIALOG_PANEL));
    let s = Strs::new();
    let ctx = UiCtx {
        tick: w.frames,
        world: &w,
        strings: &s,
    };
    let mut out: Vec<UiDraw> = Vec::new();
    root.draw(&ctx, &mut out);
    let x = (800 - 325) / 2;
    assert!(
        out.contains(&UiDraw::Rect(RectRequest::sized(x, 12 - 5, 325, 122, 0, 1))),
        "the dialog's backing is drawn without ui 8"
    );
    // Past the 100 ms grace a press anywhere skips it; with no next line a
    // mouse skip ends the talk (the end callback's stack argument is 1).
    w.frames += 5;
    w.drlg_updates += 5;
    poll(&mut ui, &mut root, &w);
    root.take_intents();
    click(&mut ui, &mut root, &w, Point::new(400, 300));
    poll(&mut ui, &mut root, &w);
    assert!(ui.npc_dialog_lines().is_none(), "skipped");
    poll(&mut ui, &mut root, &w);
    assert!(!ui.is_open(8), "a mouse skip opens no menu");
    let sent = root.take_intents();
    assert!(
        sent.iter().any(|i| i.0[..2] == [0x30, 1]),
        "the interaction ends: {sent:?}"
    );
}

/// 0x28 for Akara: the B2 case (the text list's first kind-0 entry).
fn b2_dialog(ui: &mut OriginalUi, w: &ClientWorld) {
    b2_dialog_for(ui, w, AKARA);
}

/// 0x28 for an NPC class: the B2 case.
fn b2_dialog_for(ui: &mut OriginalUi, w: &ClientWorld, class: u32) {
    ui.apply_output(
        &Output::NpcDialog(Box::new(crate::bridge::output::NpcDialog {
            kind: 1,
            guid: NPC,
            quest_flags: [0; 96],
            unit: UnitKey::new(MONSTER, NPC),
            class,
            interact: true,
            f4b1a10: None,
            cursor_item: false,
            level: 1,
            unidentified: 0,
            expansion: false,
            npc_monsters: vec![UnitKey::new(MONSTER, NPC)],
        })),
        w,
    )
    .unwrap();
}

// The greeting is a B2 line before the menu: its key skip opens the menu
// even for a class whose talk-topic end ends the interaction (266: Talk
// only). Ending the topic still ends the interaction (the topic rule is
// unchanged; `panels::npc::talk_end`).
#[test]
fn a_talk_only_npcs_greeting_opens_the_menu_after_a_key_skip() {
    const TALK_ONLY: u32 = 266;
    let (mut ui, mut root) = setup();
    let mut w = world(TALK_ONLY);
    ui.apply_output(&text_list(&[(0, GOSSIP)]), &w).unwrap();
    b2_dialog_for(&mut ui, &w, TALK_ONLY);
    for _ in 0..5 {
        w.frames += 1;
        w.drlg_updates += 1;
        poll(&mut ui, &mut root, &w);
    }
    assert!(ui.npc_dialog_lines().is_some(), "the greeting plays");
    assert!(!ui.is_open(8), "no menu while the greeting plays");
    let _ = root.take_intents();
    let key = UiEvent::Action(ActionId(Action::ClearScreen.index() as u16));
    send(&mut ui, &mut root, &w, key);
    poll(&mut ui, &mut root, &w);
    poll(&mut ui, &mut root, &w);
    assert!(ui.is_open(8), "the key skip opens the menu");
    assert!(ui.npc_menu_box().is_some());
    assert!(
        !root
            .take_intents()
            .iter()
            .any(|i| i.0.first() == Some(&0x30)),
        "the interaction stays open"
    );
}

// The B2 line's end callback plays the next unplayed kind-0
// line with C→S 0x31 (Warriv's intro, then his
// quest line); an Esc skip of the last line rebuilds the menu.
#[test]
fn the_b2_talk_plays_the_next_line_then_a_key_skip_opens_the_menu() {
    let (mut ui, mut root) = setup();
    let mut w = world(AKARA);
    ui.apply_output(&text_list(&[(0, GOSSIP), (0, NEXT_LINE)]), &w)
        .unwrap();
    b2_dialog(&mut ui, &w);
    poll(&mut ui, &mut root, &w);
    assert_eq!(texts(&ui.npc_dialog_lines().unwrap()), ["Hello", "There"]);
    w.frames += 5;
    w.drlg_updates += 5;
    poll(&mut ui, &mut root, &w);
    root.take_intents();
    click(&mut ui, &mut root, &w, Point::new(400, 300));
    let lines = ui.npc_dialog_lines().expect("the next line opens at once");
    assert_eq!(texts(&lines), ["Next"]);
    let sent = root.take_intents();
    let mut msg = vec![0x31];
    msg.extend(NPC.to_le_bytes());
    msg.extend(u32::from(NEXT_LINE).to_le_bytes());
    assert!(sent.iter().any(|i| i.0 == msg), "C→S 0x31: {sent:?}");
    poll(&mut ui, &mut root, &w);
    assert!(!ui.is_open(8), "no menu while the next line plays");
    w.frames += 5;
    w.drlg_updates += 5;
    poll(&mut ui, &mut root, &w);
    let esc = UiEvent::Action(ActionId(Action::GameMenu.index() as u16));
    send(&mut ui, &mut root, &w, esc);
    poll(&mut ui, &mut root, &w);
    poll(&mut ui, &mut root, &w);
    assert!(ui.npc_dialog_lines().is_none(), "skipped");
    assert!(ui.is_open(8), "a key skip of the last line opens the menu");
}

// 0x28 B2 plays a dialog line before the menu. The skip handlers sit on the
// game window, so a click skips it with no talk up. The button skip passes EDX
// 1 and a stack argument of 1 to the end callback: no menu; the interaction
// ends (C→S 0x30).
#[test]
fn a_click_skipping_the_b2_line_ends_the_interaction() {
    let (mut ui, mut root) = setup();
    let mut w = world(AKARA);
    {
        let mut st = ui.npcm.borrow_mut();
        st.talk.pending_open = Some((NPC, GOSSIP));
        st.talk.menu_after = Some((NPC, AKARA, 1, 0));
    }
    poll(&mut ui, &mut root, &w);
    assert!(ui.npc_dialog_lines().is_some(), "the B2 line plays");
    assert!(ui.npc_menu_box().is_none(), "no menu while it plays");
    for _ in 0..10 {
        w.frames += 1;
        w.drlg_updates += 1;
        poll(&mut ui, &mut root, &w);
    }
    let _ = root.take_intents();
    click(&mut ui, &mut root, &w, Point::new(400, 250));
    poll(&mut ui, &mut root, &w);
    assert!(ui.npc_dialog_lines().is_none(), "the click skipped it");
    assert!(ui.npc_menu_box().is_none(), "no menu after a button skip");
    let sent: Vec<Vec<u8>> = root.take_intents().into_iter().map(|i| i.0).collect();
    assert!(
        sent.contains(&crate::ui::panels::npc::msg_chat_end(NPC).to_vec()),
        "{sent:?}"
    );
}

// Scripts click the talk by id: the topic box's choices are
// `npc.topic(N)` (the caption is not one), the dialog is `dialog.skip`
// over the whole screen, and a click on each by id runs as a press there.
#[test]
fn the_topics_and_the_dialog_skip_have_ids() {
    let (mut ui, mut root, mut w) = talk_to_akara();
    let s = Strs::new();
    let ids = |root: &UiRoot, w: &ClientWorld| {
        let ctx = UiCtx {
            tick: w.frames,
            world: w,
            strings: &s,
        };
        root.ui_elements(&ctx)
    };
    let els = ids(&root, &w);
    let names: Vec<&str> = els
        .iter()
        .map(|e| e.id.as_str())
        .filter(|id| id.starts_with("npc.topic("))
        .collect();
    assert_eq!(
        names,
        [
            "npc.topic(0)",
            "npc.topic(1)",
            "npc.topic(2)",
            "npc.topic(3)"
        ]
    );
    let p = ui.npc_topic_point(1).unwrap();
    assert!(els[1].rect.contains(p), "gossip's point is in npc.topic(1)");
    click(&mut ui, &mut root, &w, els[1].at);
    assert!(ui.npc_dialog_lines().is_some(), "gossip plays");
    let els = ids(&root, &w);
    assert_eq!(els.len(), 1);
    assert_eq!(els[0].id, "dialog.skip");
    assert_eq!(els[0].at, Point::new(400, 300));
    poll(&mut ui, &mut root, &w);
    for _ in 0..80 {
        w.frames += 1;
        w.drlg_updates += 1;
        poll(&mut ui, &mut root, &w);
    }
    click(&mut ui, &mut root, &w, els[0].at);
    poll(&mut ui, &mut root, &w);
    assert!(ui.npc_dialog_lines().is_none(), "skipped");
    assert_eq!(
        ids(&root, &w)
            .iter()
            .filter(|e| e.id.starts_with("npc.topic("))
            .count(),
        4,
        "the topic box again"
    );
}

#[test]
fn resizing_talk_preserves_text_scroll_and_speech_state() {
    use crate::rules::{adaptive_integer::FrameSelection, camera::FrameSize};
    let mut st = TalkState::default();
    let _ = st.dialog.open(
        DialogOpen::Npc {
            id: 42,
            unit: Some(77),
        },
        &w16("4\nFirst\nSecond"),
        800,
        600,
        80,
        -60,
        1200,
    );
    st.dialog.speech_pending = true;
    st.dialog.mouse_down = true;
    let panel = st.dialog.panel.clone();
    for (w, h) in [
        (1920, 1080),
        (3840, 2160),
        (2560, 1440),
        (1920, 1200),
        (2560, 1080),
        (3440, 1440),
        (5120, 2160),
        (1365, 767),
        (1919, 1079),
        (800, 1600),
        (7680, 2160),
        (801, 541),
    ] {
        let frame =
            FrameSelection::for_window(w, h, FrameSize::WIDE, crate::ui::ScaleMode::Integer).frame;
        let screen = Screen::for_frame(frame);
        st.resize_screen(screen, &Measure(None));
        assert_eq!(st.dialog.panel, panel);
        assert_eq!(st.dialog.t_open, 1200);
        assert!(st.dialog.speech_pending);
        assert!(!st.dialog.mouse_down);
        assert!(st.dialog.x >= 0 && st.dialog.x + 325 <= screen.w);
        assert!(st.dialog.y >= 0 && st.dialog.y + 182 <= screen.h - 53);
    }
}

/// The dialog-line stops the UI asked for since the last take.
fn speech_closes(ui: &mut OriginalUi) -> usize {
    use crate::audio::driver::SoundRequest;
    ui.take_outcome()
        .sounds
        .iter()
        .filter(|r| **r == SoundRequest::NpcDialogClose)
        .count()
}

// Cancelling the NPC text stops the voice line it was playing: an Esc
// skip of the B2 line asks the audio layer to stop the dialog line.
#[test]
fn an_esc_skip_of_the_b2_line_stops_its_speech() {
    let (mut ui, mut root) = setup();
    let mut w = world(AKARA);
    ui.apply_output(&text_list(&[(0, GOSSIP)]), &w).unwrap();
    b2_dialog(&mut ui, &w);
    poll(&mut ui, &mut root, &w);
    assert!(ui.npc_dialog_lines().is_some(), "the B2 line plays");
    w.frames += 5;
    w.drlg_updates += 5;
    poll(&mut ui, &mut root, &w);
    assert_eq!(speech_closes(&mut ui), 0, "nothing stops while it plays");
    let esc = UiEvent::Action(ActionId(Action::GameMenu.index() as u16));
    send(&mut ui, &mut root, &w, esc);
    poll(&mut ui, &mut root, &w);
    assert!(ui.npc_dialog_lines().is_none(), "skipped");
    assert_eq!(speech_closes(&mut ui), 1, "the Esc skip stops the line");
}

// A click skip of the B2 line (which ends the interaction) stops the line.
#[test]
fn a_click_skip_of_the_b2_line_stops_its_speech() {
    let (mut ui, mut root) = setup();
    let mut w = world(AKARA);
    ui.apply_output(&text_list(&[(0, GOSSIP)]), &w).unwrap();
    b2_dialog(&mut ui, &w);
    poll(&mut ui, &mut root, &w);
    w.frames += 5;
    w.drlg_updates += 5;
    poll(&mut ui, &mut root, &w);
    let _ = speech_closes(&mut ui);
    click(&mut ui, &mut root, &w, Point::new(400, 300));
    poll(&mut ui, &mut root, &w);
    assert!(ui.npc_dialog_lines().is_none(), "skipped");
    assert_eq!(speech_closes(&mut ui), 1, "the click skip stops the line");
}

// A press inside the 100 ms grace only eats the event: the text and its
// line go on.
#[test]
fn a_press_inside_the_grace_keeps_the_speech() {
    let (mut ui, mut root) = setup();
    let w = world(AKARA);
    ui.apply_output(&text_list(&[(0, GOSSIP)]), &w).unwrap();
    b2_dialog(&mut ui, &w);
    poll(&mut ui, &mut root, &w);
    let _ = speech_closes(&mut ui);
    click(&mut ui, &mut root, &w, Point::new(400, 300));
    assert!(ui.npc_dialog_lines().is_some(), "still up");
    assert_eq!(speech_closes(&mut ui), 0);
}

// The interaction ends while a topic's text is up (the NPC menu goes off:
// walking away, another panel taking its place): the text closes and its
// line stops with it.
#[test]
fn the_interaction_ending_mid_text_stops_its_speech() {
    let (mut ui, mut root, mut w) = talk_to_akara();
    let p = ui.npc_topic_point(1).unwrap();
    click(&mut ui, &mut root, &w, p);
    assert!(ui.npc_dialog_lines().is_some(), "the dialog is up");
    let _ = speech_closes(&mut ui);
    ui.set_ui(8, 1, false).unwrap();
    w.frames += 1;
    w.drlg_updates += 1;
    poll(&mut ui, &mut root, &w);
    assert!(ui.npc_dialog_lines().is_none(), "the text goes");
    assert_eq!(speech_closes(&mut ui), 1, "and its line stops");
}

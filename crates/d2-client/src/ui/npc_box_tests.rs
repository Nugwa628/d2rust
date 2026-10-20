use super::*;
use crate::bridge::output::Output;
use crate::bridge::world::{ClientUnit, MONSTER, PLAYER};
use crate::controls::Action;
use crate::ui::layout::Screen;
use crate::ui::original::UiConfig;
use crate::ui::panel::ActionId;
use crate::ui::{NoPanelRules, Routed};

const KASHYA: u32 = 150;
const AKARA: u32 = 148;
const CAIN: u32 = 244;
const NPC: u32 = 77;
const ME: UnitKey = UnitKey {
    unit_type: PLAYER,
    guid: 1,
};

struct Strs(Vec<(u16, Vec<u16>)>);

impl Strs {
    fn new() -> Self {
        let w = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        Strs(vec![
            (3381, w("Talk")),
            (3396, w("Trade")),
            (3397, w("Hire")),
            (4142, w("cancel")),
            (4021, w("Identify Items: ")),
            (22696, w("Resurrect %s: %d")),
            (11021, w("Rogue")),
        ])
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

fn utf(t: &[u16]) -> String {
    String::from_utf16_lossy(t)
}

/// A player at (1000, 1000) level `level` and the NPC of `class` beside.
fn world(class: u32, level: i32) -> ClientWorld {
    let mut w = ClientWorld::default();
    let mut p = ClientUnit::new(ME);
    p.position = Some((1000, 1000));
    p.stats.insert(12, level);
    w.units.insert(ME, p);
    w.local_player = Some(ME);
    let mut n = ClientUnit::new(UnitKey::new(MONSTER, NPC));
    n.class = class;
    n.position = Some((1004, 1000));
    w.units.insert(n.key, n);
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

/// One event through the root and the play UI.
fn send(ui: &mut OriginalUi, root: &mut UiRoot, w: &ClientWorld, e: UiEvent) -> Routed {
    let s = Strs::new();
    let ctx = UiCtx {
        tick: 0,
        world: w,
        strings: &s,
    };
    ui.before_event(e, w);
    let r = root.dispatch(e, &ctx);
    ui.after_event(root, e, r).unwrap();
    r
}

fn click(ui: &mut OriginalUi, root: &mut UiRoot, w: &ClientWorld, at: Point) {
    let button = PointerButton::Left;
    send(ui, root, w, UiEvent::Press { button, at });
    send(ui, root, w, UiEvent::Release { button, at });
}

fn open(ui: &mut OriginalUi, root: &mut UiRoot, w: &ClientWorld, class: u32, level: i32) {
    ui.open_npc_menu(NPC, class, level, w);
    ui.npc_menu_poll(w, root, &Strs::new());
}

fn bytes(i: &ClientIntent) -> Vec<u8> {
    i.0.clone()
}

// The menu is ui 8 (the root mirrors it), Kashya's build sends the
// hire-list request 0x38 [3][NPC][player], and the box is the
// auto-laid-out spec box above the NPC: the name item (color 4, not
// selectable), the options, then `lowercasecancel`.
#[test]
fn kashyas_menu_is_ui_8_with_the_hire_request_and_the_box_above_her() {
    let (mut ui, mut root) = setup();
    let w = world(KASHYA, 10);
    open(&mut ui, &mut root, &w, KASHYA, 10);
    assert!(ui.is_open(8), "SetUIState(8, on)");
    assert!(root.open_panels().contains(&NPC_MENU_PANEL));
    let mut want = vec![0x38, 3, 0, 0, 0];
    want.extend_from_slice(&NPC.to_le_bytes());
    want.extend_from_slice(&ME.guid.to_le_bytes());
    assert_eq!(
        root.take_intents().iter().map(bytes).collect::<Vec<_>>(),
        vec![want]
    );
    let m = ui.npc_menu().expect("the box is built");
    let kinds: Vec<_> = m.rows.iter().map(|r| r.kind).collect();
    // Level 10 > 7: hire in slot 1.
    assert_eq!(
        kinds,
        vec![Some(OptionKind::Talk), Some(OptionKind::Hire), None]
    );
    let (r, items) = ui.npc_menu_box().unwrap();
    let texts: Vec<(String, u8)> = items.iter().map(|(t, c)| (utf(t), *c)).collect();
    assert_eq!(
        texts,
        vec![
            (String::new(), 4),
            ("Talk".into(), 0),
            ("Hire".into(), 0),
            ("cancel".into(), 0)
        ]
    );
    // With no fonts (8 px a unit): w = 8 × 6 + 20, h = 21 + 3 × 15 +
    // 15; x = anchor x − w / 2, y = anchor y − h / 3.
    let (ax, ay) = npc_anchor(&w, NPC, 0).unwrap();
    assert_eq!((r.r - r.l, r.b - r.t), (68, 81));
    assert_eq!((r.l, r.t), (ax - 34, ay - 27));
}

// Cancel ends the interaction: C→S 0x30 [1][GUID] and SetUIState(8, off).
#[test]
fn cancel_sends_0x30_and_turns_ui_8_off() {
    let (mut ui, mut root) = setup();
    let w = world(AKARA, 1);
    open(&mut ui, &mut root, &w, AKARA, 1);
    assert!(root.take_intents().is_empty(), "no hire request for Akara");
    let p = ui.npc_menu_row_point(2).unwrap();
    click(&mut ui, &mut root, &w, p);
    let mut want = vec![0x30, 1, 0, 0, 0];
    want.extend_from_slice(&NPC.to_le_bytes());
    assert_eq!(
        root.take_intents().iter().map(bytes).collect::<Vec<_>>(),
        vec![want]
    );
    assert!(!ui.is_open(8));
    assert!(ui.npc_menu().is_none());
}

// Esc (and Space) with the NPC menu up: the box consumes the key and runs
// p1, the interaction ends (C->S 0x30), and the game menu stays shut.
#[test]
fn esc_ends_the_interaction_and_does_not_open_the_game_menu() {
    for key in [Action::GameMenu, Action::ClearScreen] {
        let (mut ui, mut root) = setup();
        let w = world(AKARA, 1);
        open(&mut ui, &mut root, &w, AKARA, 1);
        root.take_intents();
        send(
            &mut ui,
            &mut root,
            &w,
            UiEvent::Action(ActionId(key.index() as u16)),
        );
        let mut want = vec![0x30, 1, 0, 0, 0];
        want.extend_from_slice(&NPC.to_le_bytes());
        assert_eq!(
            root.take_intents().iter().map(bytes).collect::<Vec<_>>(),
            vec![want],
            "{key:?}"
        );
        assert!(!ui.is_open(8));
        assert!(!ui.is_open(9), "the game menu stays shut");
        assert!(ui.npc_menu().is_none());
    }
}

// With ui 8 open the gate refuses the inventory hot key (C[8][1] = 2).
#[test]
fn the_inventory_key_is_refused_while_the_menu_is_up() {
    let (mut ui, mut root) = setup();
    let w = world(AKARA, 1);
    open(&mut ui, &mut root, &w, AKARA, 1);
    let a = UiEvent::Action(ActionId(Action::ToggleInventory.index() as u16));
    send(&mut ui, &mut root, &w, a);
    assert!(!ui.is_open(1), "refused by the NPC menu");
}

// An NPC class without a record gets record 0 (Akara's talk, trade).
#[test]
fn a_class_without_a_record_gets_record_0() {
    let (mut ui, mut root) = setup();
    let w = world(999, 1);
    open(&mut ui, &mut root, &w, 999, 1);
    let kinds: Vec<_> = ui.npc_menu().unwrap().rows.iter().map(|r| r.kind).collect();
    assert_eq!(
        kinds,
        vec![Some(OptionKind::Talk), Some(OptionKind::Trade), None]
    );
}

// Without the NPC in the model the build ends the interaction.
#[test]
fn no_npc_ends_the_interaction() {
    let (mut ui, mut root) = setup();
    let mut w = world(AKARA, 1);
    w.units.remove(&UnitKey::new(MONSTER, NPC));
    open(&mut ui, &mut root, &w, AKARA, 1);
    assert!(ui.npc_menu().is_none());
    assert!(!ui.is_open(8));
    assert_eq!(root.take_intents()[0].0[0], 0x30);
}

// The Resurrect slot reads `hireresurrect2` with the mercenary's name
// (id 0x421 → string 11021) and the cost.
#[test]
fn resurrect_row_is_captioned_by_hireresurrect2_with_the_name() {
    let (mut ui, mut root) = setup();
    let mut w = world(KASHYA, 10);
    w.expansion = 1;
    {
        let mut st = ui.npcm.borrow_mut();
        st.resurrect = Some(500);
        st.merc_name = 0x421;
    }
    open(&mut ui, &mut root, &w, KASHYA, 10);
    let (_, items) = ui.npc_menu_box().unwrap();
    let texts: Vec<String> = items.iter().map(|(t, _)| utf(t)).collect();
    assert!(
        texts.contains(&"Resurrect Rogue: 500".to_string()),
        "{texts:?}"
    );
    let m = ui.npc_menu().unwrap();
    let r = m
        .rows
        .iter()
        .find(|r| r.kind == Some(OptionKind::Resurrect))
        .unwrap();
    assert_eq!(r.cost, Some(500));
}

// Identify sends C→S 0x34 and waits (state 10); S→C 0x2A result 3 resets
// Cain's counts and rebuilds the menu, whose identify caption reads the
// count again.
#[test]
fn identify_waits_for_0x2a_and_the_result_rebuilds_the_menu() {
    let (mut ui, mut root) = setup();
    let w = world(CAIN, 1);
    ui.open_npc_menu_with(NPC, CAIN, 1, 2, &w);
    ui.npc_menu_poll(&w, &mut root, &Strs::new());
    let m = ui.npc_menu().unwrap();
    let k = m
        .rows
        .iter()
        .position(|r| r.kind == Some(OptionKind::Identify))
        .unwrap();
    assert_eq!(m.rows[k].cost, Some(200));
    let (_, items) = ui.npc_menu_box().unwrap();
    assert!(items.iter().any(|(t, _)| utf(t) == "Identify Items: 200"));
    let p = ui.npc_menu_row_point(k).unwrap();
    click(&mut ui, &mut root, &w, p);
    let mut want = vec![0x34];
    want.extend_from_slice(&NPC.to_le_bytes());
    assert_eq!(
        root.take_intents().iter().map(bytes).collect::<Vec<_>>(),
        vec![want]
    );
    assert_eq!(ui.npc_menu_state(), MENU_WAITING);
    assert!(ui.npc_menu().is_none(), "the box closed for the send");
    let mut b = [0u8; 15];
    b[0] = 0x2A;
    b[2] = 3;
    ui.apply_output(&Output::NpcTransaction { bytes: b, gold: 0 }, &w)
        .unwrap();
    ui.npc_menu_poll(&w, &mut root, &Strs::new());
    assert_eq!(ui.npc_menu_state(), 1);
    assert!(ui.npc_menu().is_some(), "the menu is built again");
    assert!(ui.is_open(8));
}

// Talk closes the box; the talk's end with flag 1 builds the menu
// again.
#[test]
fn talk_closes_the_box_and_its_end_rebuilds_the_menu() {
    let (mut ui, mut root) = setup();
    let w = world(AKARA, 1);
    open(&mut ui, &mut root, &w, AKARA, 1);
    let p = ui.npc_menu_row_point(0).unwrap();
    click(&mut ui, &mut root, &w, p);
    let m = ui.npc_menu().unwrap();
    assert!(m.talking);
    assert!(ui.npc_menu_box().is_none());
    assert!(root.take_intents().is_empty(), "talk sends nothing");
    let p = ui.npc_topic_cancel_point().expect("the topic box");
    click(&mut ui, &mut root, &w, p);
    ui.npc_menu_poll(&w, &mut root, &Strs::new());
    assert!(ui.npc_menu_box().is_some());
    assert!(!ui.npc_menu().unwrap().talking);
}

// Hire opens the hire list (the box closes); its Back builds the NPC
// menu again.
#[test]
fn hire_opens_the_list_and_back_rebuilds_the_menu() {
    let (mut ui, mut root) = setup();
    let w = world(KASHYA, 10);
    open(&mut ui, &mut root, &w, KASHYA, 10);
    root.take_intents();
    let p = ui.npc_menu_row_point(1).unwrap();
    click(&mut ui, &mut root, &w, p);
    assert_eq!(ui.hire_list().up, Some(NPC));
    assert!(ui.npc_menu_box().is_none());
    // Back: the box's Back item at (W − 490) / 2, y = (H − 40) / 2 − 195 +
    // 21 + 315.
    click(&mut ui, &mut root, &w, Point::new(400, 85 + 21 + 315 - 2));
    assert_eq!(ui.hire_list().up, None);
    assert!(root.take_intents().is_empty());
    ui.npc_menu_poll(&w, &mut root, &Strs::new());
    assert!(ui.npc_menu_box().is_some(), "Back built the menu again");
}

// A press outside the box ends the interaction (D2RUST's own until the
// window handlers are specified) and the selection follows the pointer.
#[test]
fn the_pointer_selects_and_an_outside_press_ends_the_chat() {
    let (mut ui, mut root) = setup();
    let w = world(AKARA, 1);
    open(&mut ui, &mut root, &w, AKARA, 1);
    assert_eq!(
        ui.npcm.borrow().bx.as_ref().unwrap().selected,
        1,
        "the first selectable item"
    );
    let p = ui.npc_menu_row_point(1).unwrap();
    send(&mut ui, &mut root, &w, UiEvent::CursorMoved(p));
    assert_eq!(ui.npcm.borrow().bx.as_ref().unwrap().selected, 2);
    // Style 1: the selected item draws in color 3.
    let s = Strs::new();
    let ctx = UiCtx {
        tick: 0,
        world: &w,
        strings: &s,
    };
    let mut out: Vec<UiDraw> = Vec::new();
    root.draw(&ctx, &mut out);
    let trade = out
        .iter()
        .find_map(|d| match d {
            UiDraw::Text(t) if utf(&t.text) == "Trade" => Some(t.style.color),
            _ => None,
        })
        .unwrap();
    assert_eq!(trade, 3);
    click(&mut ui, &mut root, &w, Point::new(5, 5));
    assert_eq!(root.take_intents()[0].0[0], 0x30);
    assert!(!ui.is_open(8));
}

// A player who has a hireling: the hire row opens the confirm dialog
// (kind 5, `VerifyTransaction9`, Yes / No) instead of sending; No brings
// the hire list back, Yes sends C→S 0x36 and opens the waiting note.
#[test]
fn hiring_over_a_hireling_asks_first() {
    let (mut ui, mut root) = setup();
    let mut w = world(KASHYA, 10);
    w.expansion = 1;
    w.pets.push(crate::bridge::world::PetRecord {
        class: 271,
        pet_type: crate::bridge::world::PET_HIRELING,
        pet: 900,
        owner: ME.guid,
        f1c: 100,
        gone: false,
        extra: None,
    });
    open(&mut ui, &mut root, &w, KASHYA, 10);
    root.take_intents();
    ui.apply_output(&Output::HireListReset, &w).unwrap();
    ui.apply_output(
        &Output::HireOffer {
            name: 3000,
            seed: 5,
        },
        &w,
    )
    .unwrap();
    let p = ui.npc_menu_row_point(1).unwrap();
    click(&mut ui, &mut root, &w, p);
    assert_eq!(ui.hire_list().up, Some(NPC));
    // Row 0 of the list at 800 × 600: list y 120.
    click(&mut ui, &mut root, &w, Point::new(200, 120 + 8));
    assert!(root.take_intents().is_empty(), "nothing sent yet");
    ui.npc_menu_poll(&w, &mut root, &Strs::new());
    let (kind, _) = ui.shop_state().confirm().expect("the confirm dialog");
    assert_eq!(kind, TxKind::Hire);
    let no = ui.shop_confirm_point(false).unwrap();
    click(&mut ui, &mut root, &w, no);
    assert!(ui.shop_state().confirm().is_none());
    assert_eq!(ui.hire_list().up, Some(NPC), "No: the hire list again");
    assert!(root.take_intents().is_empty());
    click(&mut ui, &mut root, &w, Point::new(200, 120 + 8));
    ui.npc_menu_poll(&w, &mut root, &Strs::new());
    let yes = ui.shop_confirm_point(true).unwrap();
    click(&mut ui, &mut root, &w, yes);
    assert_eq!(
        root.take_intents().iter().map(bytes).collect::<Vec<_>>(),
        vec![crate::ui::hire_list::hire_intent(NPC, 3000).0]
    );
    ui.npc_menu_poll(&w, &mut root, &Strs::new());
    assert_eq!(ui.npc_menu_state(), MENU_WAITING);
    assert!(ui.npc_waiting_note());
}

// The box background is the rectangle `(x, y, w, h, 0, 1)`: the sink's
// rectangle over the box, color 0, mode 1 (no cel).
#[test]
fn the_box_frame_is_the_dark_rectangle() {
    let (mut ui, mut root) = setup();
    let w = world(AKARA, 1);
    open(&mut ui, &mut root, &w, AKARA, 1);
    let (r, _) = ui.npc_menu_box().unwrap();
    let s = Strs::new();
    let ctx = UiCtx {
        tick: 0,
        world: &w,
        strings: &s,
    };
    let mut out: Vec<UiDraw> = Vec::new();
    root.draw(&ctx, &mut out);
    let want = RectRequest::sized(r.l, r.t, r.r - r.l, r.b - r.t, 0, 1);
    assert!(out.contains(&UiDraw::Rect(want)), "{out:?}");
}

// The box background is followed by the `menu\boxpieces` border
// Around (x, y, x + w − 1, y + h − 1): one cel
// draw per piece, in the routine's order (playtest B014).
#[test]
fn the_box_frame_is_followed_by_the_boxpieces_border() {
    let (mut ui, mut root) = setup();
    let w = world(AKARA, 1);
    open(&mut ui, &mut root, &w, AKARA, 1);
    let (r, _) = ui.npc_menu_box().unwrap();
    let file = ui
        .files()
        .id(crate::ui::box_border::BOXPIECES_FILE)
        .expect("boxpieces is registered");
    let s = Strs::new();
    let ctx = UiCtx {
        tick: 0,
        world: &w,
        strings: &s,
    };
    let mut out: Vec<UiDraw> = Vec::new();
    root.draw(&ctx, &mut out);
    let got: Vec<(u32, i32, i32)> = out
        .iter()
        .filter_map(|d| match d {
            UiDraw::Image(i) if i.image.file == file => Some((i.image.frame, i.at.x, i.at.y)),
            _ => None,
        })
        .collect();
    let want = crate::ui::box_border::border_pieces(r.l, r.t, r.r - 1, r.b - 1);
    assert_eq!(got, want);
    // The backing rectangle comes first.
    let frame = out
        .iter()
        .position(|d| matches!(d, UiDraw::Rect(q) if q.x0 == r.l && q.y0 == r.t))
        .expect("the backing rectangle");
    let first = out
        .iter()
        .position(|d| matches!(d, UiDraw::Image(i) if i.image.file == file))
        .expect("a border piece");
    assert!(frame < first);
}

// Playtest B016: Esc (and Space) with the shop up closes the shop (the
// close-all of command 56 / 38 takes ui 0x0C), the shop's close ends the
// interaction (C->S 0x30, ui 8 off) and the game menu stays shut.
#[test]
fn esc_closes_the_shop_and_ends_the_interaction() {
    for key in [Action::GameMenu, Action::ClearScreen] {
        let (mut ui, mut root) = setup();
        let w = world(AKARA, 1);
        open(&mut ui, &mut root, &w, AKARA, 1);
        ui.open_shop(NPC, AKARA);
        ui.shop_poll(&w, &mut root);
        assert!(ui.is_open(0x0C), "the shop is up");
        root.take_intents();
        send(
            &mut ui,
            &mut root,
            &w,
            UiEvent::Action(ActionId(key.index() as u16)),
        );
        ui.shop_poll(&w, &mut root);
        assert!(!ui.is_open(0x0C), "{key:?}: the shop closes");
        assert!(!ui.is_open(9), "{key:?}: the game menu stays shut");
        assert!(!ui.is_open(8), "{key:?}: the interaction ends");
        let mut want = vec![0x30, 1, 0, 0, 0];
        want.extend_from_slice(&NPC.to_le_bytes());
        assert_eq!(
            root.take_intents().iter().map(bytes).collect::<Vec<_>>(),
            vec![want],
            "{key:?}"
        );
    }
}

// Playtest B001 (not reproduced; probe): after a trade closed by the shop's own close, Esc
// opens the game menu (no NPC interaction is left to eat the key).
#[test]
fn esc_opens_the_game_menu_after_a_trade_ended() {
    let (mut ui, mut root) = setup();
    let mut w = world(AKARA, 1);
    w.units.get_mut(&ME).unwrap().mode = 1;
    open(&mut ui, &mut root, &w, AKARA, 1);
    ui.open_shop(NPC, AKARA);
    ui.shop_poll(&w, &mut root);
    ui.set_ui(0x0C, 1, false).unwrap();
    ui.shop_poll(&w, &mut root);
    ui.npc_menu_poll(&w, &mut root, &Strs::new());
    assert!(!ui.is_open(8));
    let before: Vec<u8> = (0..38u8).filter(|&i| ui.is_open(i)).collect();
    let r = send(
        &mut ui,
        &mut root,
        &w,
        UiEvent::Action(ActionId(Action::GameMenu.index() as u16)),
    );
    assert!(
        ui.is_open(9),
        "the game menu opens ({r:?}, open before {before:?})"
    );
}

/// S→C 0x27 type 1 with one kind-0 entry (string `str0`).
fn npc_text_out(str0: u16) -> Output {
    let mut bytes = [0u8; 40];
    bytes[0] = 0x27;
    bytes[1] = 1;
    bytes[2] = 6;
    bytes[6] = 1;
    bytes[10..12].copy_from_slice(&str0.to_le_bytes());
    Output::NpcText {
        bytes,
        present: true,
        object_class: 0,
    }
}

fn npc_dialog_out(class: u32) -> Output {
    Output::NpcDialog(Box::new(crate::bridge::output::NpcDialog {
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
    }))
}

// 0x62 (1, G) arrives while the B2 line
// still talks (menu state 0, the talk flag = 1): the client ends the
// interaction with C→S 0x30 [1][G] and no menu opens afterwards.
#[test]
fn npc_dialog_end_during_the_b2_line_ends_the_chat() {
    let (mut ui, mut root) = setup();
    let w = world(AKARA, 1);
    ui.apply_output(&npc_text_out(0x25), &w).unwrap();
    ui.apply_output(&npc_dialog_out(AKARA), &w).unwrap();
    ui.npc_menu_poll(&w, &mut root, &Strs::new());
    root.take_intents();
    assert_eq!(ui.npc_menu_state(), 0, "B2: the line talks, no menu yet");
    ui.apply_output(&Output::NpcDialogEnd { kind: 1 }, &w)
        .unwrap();
    for _ in 0..3 {
        ui.npc_menu_poll(&w, &mut root, &Strs::new());
    }
    let mut want = vec![0x30, 1, 0, 0, 0];
    want.extend_from_slice(&NPC.to_le_bytes());
    let sent = root.take_intents().iter().map(bytes).collect::<Vec<_>>();
    assert!(sent.contains(&want), "{sent:02x?}");
    assert_eq!(ui.npc_menu_state(), 0);
    assert!(ui.npc_menu().is_none(), "the end callback opens no menu");
    // A second 0x62 finds no interaction: nothing more is sent.
    ui.apply_output(&Output::NpcDialogEnd { kind: 1 }, &w)
        .unwrap();
    ui.npc_menu_poll(&w, &mut root, &Strs::new());
    let again = root.take_intents().iter().map(bytes).collect::<Vec<_>>();
    assert!(!again.contains(&want), "{again:02x?}");
}

// The menu's choices have ids, top down, and clicking one by its id runs
// it: Akara's third choice is Cancel, which ends the interaction.
#[test]
fn the_menu_choices_have_ids_and_a_click_by_id_runs_them() {
    let (mut ui, mut root) = setup();
    let w = world(AKARA, 1);
    open(&mut ui, &mut root, &w, AKARA, 1);
    let s = Strs::new();
    let ctx = UiCtx {
        tick: 0,
        world: &w,
        strings: &s,
    };
    let els = root.ui_elements(&ctx);
    let ids: Vec<&str> = els.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["npc.menu(0)", "npc.menu(1)", "npc.menu(2)"]);
    let p = ui.npc_menu_row_point(2).unwrap();
    assert!(els[2].rect.contains(p), "Cancel's point is in npc.menu(2)");
    let at = crate::ui::ui_ids::find(&els, "npc.menu(2)").unwrap().at;
    click(&mut ui, &mut root, &w, at);
    assert_eq!(root.take_intents().first().map(|i| i.0[0]), Some(0x30));
    assert!(!ui.is_open(8));
    let ctx = UiCtx {
        tick: 0,
        world: &w,
        strings: &s,
    };
    assert!(root.ui_elements(&ctx).is_empty(), "the box is gone");
}

#[test]
fn resizing_an_open_npc_menu_preserves_rows_selection_and_waiting_deadline() {
    use crate::rules::{adaptive_integer::FrameSelection, camera::FrameSize};
    let (mut ui, mut root) = setup();
    let world = world(KASHYA, 10);
    open(&mut ui, &mut root, &world, KASHYA, 10);
    ui.open_waiting_note(&Strs::new(), 1234);
    let (rows, selected, deadline, pending) = {
        let st = ui.npcm.borrow();
        (
            st.rows.clone(),
            st.bx.as_ref().unwrap().selected,
            st.note.as_ref().unwrap().deadline,
            st.pending.len(),
        )
    };
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
        ui.resize_screen(screen);
        {
            let st = ui.npcm.borrow();
            assert_eq!(st.rows, rows);
            assert_eq!(st.bx.as_ref().unwrap().selected, selected);
            assert_eq!(st.note.as_ref().unwrap().deadline, deadline);
            assert_eq!(
                st.pending.len(),
                pending,
                "resize must not replay hire requests"
            );
            let bx = st.bx.as_ref().unwrap();
            assert!(bx.pos.0 >= 0 && bx.pos.0 + bx.size.0 <= screen.w);
            assert!(bx.pos.1 >= 0 && bx.pos.1 + bx.size.1 <= screen.h - 48);
            let note = &st.note.as_ref().unwrap().bx;
            assert!(note.pos.1 >= 0 && note.pos.1 + note.size.1 <= screen.h - 48);
        }
        let at = ui.npc_menu_row_point(0).unwrap();
        assert!(at.y >= 0 && at.y < screen.h);
        ui.resize_screen(Screen::R800);
    }
}

// The preview's automatic chat close must not end an NPC interaction while
// its B2 greeting is still playing (the menu is not built yet then).
#[test]
fn an_open_interaction_includes_the_b2_greeting() {
    let (mut ui, mut root) = setup();
    let w = world(AKARA, 1);
    open(&mut ui, &mut root, &w, AKARA, 1);
    assert!(ui.npc_interaction_up());
    assert!(ui.npc_menu_up());
    let (ui, _root) = setup();
    {
        let mut st = ui.npcm.borrow_mut();
        st.talk.menu_after = Some((NPC, AKARA, 1, 0));
        st.talk.pending_open = Some((NPC, 900));
    }
    assert!(!ui.npc_menu_up(), "the greeting is not the built menu");
    assert!(ui.npc_interaction_up(), "the interaction is still open");
}

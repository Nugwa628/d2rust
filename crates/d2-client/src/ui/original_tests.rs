//! The play UI wired into a root: hotkeys, the gate and open mode,
//! the panel adapters' draws and clicks (spec vectors where the
//! client model holds the inputs).

use super::*;
use crate::bridge::output::Output;
use crate::bridge::world::{ClientUnit, UnitKey};
use crate::ui::draw::UiDraw;
use crate::ui::{ClientIntent, NoPanelRules, NoStrings};

const AMAZON: u32 = 0;

fn areas() -> Vec<InvArea> {
    let mut v = vec![InvArea::default(); 32];
    // Vectors: `inventory.bin` records 0 and 16 (amazon).
    v[0] = InvArea {
        left: 320,
        right: 640,
        top: 0,
        bottom: 441,
    };
    v[16] = InvArea {
        left: 400,
        right: 720,
        top: 60,
        bottom: 501,
    };
    v
}

fn world(class: u32, mode: u32, expansion: bool) -> ClientWorld {
    let mut w = ClientWorld::default();
    let key = UnitKey::new(PLAYER, 1);
    let mut u = ClientUnit::new(key);
    u.class = class;
    u.mode = mode;
    w.units.insert(key, u);
    w.local_player = Some(key);
    w.expansion = u32::from(expansion);
    w
}

struct Ui {
    ui: OriginalUi,
    root: UiRoot,
}

// The right panel's area: right exclusive, bottom inclusive (record 0
// at 640 × 480: x 320–639, y 0–441).
#[test]
fn inv_area_rect_has_an_inclusive_bottom() {
    let r = areas()[0].rect();
    assert!(r.contains(Point::new(639, 441)));
    assert!(r.contains(Point::new(320, 0)));
    assert!(!r.contains(Point::new(640, 100)));
    assert!(!r.contains(Point::new(400, 442)));
}

// An expansion game's waypoint tabs step by 64 over five tabs: a press
// at (300, 80) at 800 × 600 (x' = 220, y' = 20) selects tab 3,
// drawn as `expwaygatetabs` frame 2t = 6. The click walks
// down the setter's quest records, so the client holds records 7,
// 15, 23, 26 and 28 bit 0 (S→C 0x29).
#[test]
fn expansion_waypoint_tab_click_uses_five_tabs() {
    let mut u = ui(Some(areas()), true);
    let w = world(AMAZON, 1, true);
    let mut quest = [0u8; 96];
    for q in [7usize, 15, 23, 26, 28] {
        quest[2 * q] |= 1;
    }
    u.ui.apply_output(&Output::QuestFlags { record: quest }, &w)
        .unwrap();
    let record = [2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    u.ui.apply_output(&Output::WaypointMenu { guid: 0x0A, record }, &w)
        .unwrap();
    u.root.sync_states(&u.ui.shared.borrow().states);
    assert!(u.ui.is_open(0x14));
    let tabs = |u: &Ui| -> Vec<u32> {
        panel_images(&u.images(&w), "menu\\expwaygatetabs")
            .into_iter()
            .map(|(f, ..)| f)
            .collect()
    };
    assert!(!tabs(&u).contains(&6));
    u.send(
        &w,
        UiEvent::Press {
            button: PointerButton::Left,
            at: Point::new(300, 80),
        },
    );
    assert!(tabs(&u).contains(&6), "{:?}", tabs(&u));
}

// A release over the belt clicks only after a press there (the press flag
// is kept): a press in the world dragged onto the belt picks nothing.
#[test]
fn a_belt_release_without_a_belt_press_does_not_click() {
    use crate::bridge::items::mode;
    use crate::ui::panels::control::belt::{BeltBox, BeltRecord};
    use crate::ui::panels::inv_items::tests::world as item_world;
    let mut u = ui(Some(areas()), true);
    // Four boxes in the strip (y 562..590, x 430 + 31 i), in every
    // record (whichever the belt type picks).
    let boxes: Vec<BeltBox> = (0..4)
        .map(|i| BeltBox {
            left: 430 + 31 * i,
            right: 458 + 31 * i,
            top: 562,
            bottom: 590,
        })
        .collect();
    let records = vec![BeltRecord { boxes }; 14];
    u.ui.set_belt_parts(hud_belt::BeltParts {
        records,
        types: BTreeMap::new(),
        beltable: [*b"hp1 "].into(),
    });
    let mut w = item_world(&[(7, mode::BELT, (0, 0, 0, 0), b"hp1 ")], None);
    let me = w.local_player.unwrap();
    w.units.get_mut(&me).unwrap().mode = 1;
    let (b, box0, field) = (
        PointerButton::Left,
        Point::new(440, 570),
        Point::new(300, 200),
    );
    u.send(
        &w,
        UiEvent::Press {
            button: b,
            at: field,
        },
    );
    u.send(
        &w,
        UiEvent::Release {
            button: b,
            at: box0,
        },
    );
    assert!(u.root.take_intents().is_empty());
    u.send(
        &w,
        UiEvent::Press {
            button: b,
            at: box0,
        },
    );
    u.send(
        &w,
        UiEvent::Release {
            button: b,
            at: box0,
        },
    );
    assert!(!u.root.take_intents().is_empty());
}

fn ui(inv: Option<Vec<InvArea>>, installed: bool) -> Ui {
    ui_at(Screen::R800, inv, installed)
}

fn ui_at(screen: Screen, inv: Option<Vec<InvArea>>, installed: bool) -> Ui {
    let config = UiConfig {
        screen,
        expansion_installed: installed,
    };
    let ui = OriginalUi::new(config, inv).unwrap();
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    ui.install(&mut root).unwrap();
    Ui { ui, root }
}

impl Ui {
    fn send(&mut self, w: &ClientWorld, e: UiEvent) -> Routed {
        let ctx = UiCtx {
            tick: 0,
            world: w,
            strings: &NoStrings,
        };
        self.ui.before_event(e, w);
        let r = self.root.dispatch(e, &ctx);
        self.ui.after_event(&mut self.root, e, r).unwrap();
        r
    }

    fn root_char(&mut self, w: &ClientWorld, c: u16) -> Routed {
        self.send(w, UiEvent::Char(c))
    }

    fn key(&mut self, w: &ClientWorld, a: Action) -> Routed {
        self.send(w, UiEvent::Action(ActionId(a.index() as u16)))
    }

    fn click(&mut self, w: &ClientWorld, at: Point) -> (Routed, Routed) {
        let b = PointerButton::Left;
        (
            self.send(w, UiEvent::Press { button: b, at }),
            self.send(w, UiEvent::Release { button: b, at }),
        )
    }

    /// (file name, frame, x, y) of every image the open panels draw.
    fn images(&self, w: &ClientWorld) -> Vec<(String, u32, i32, i32)> {
        let ctx = UiCtx {
            tick: 0,
            world: w,
            strings: &NoStrings,
        };
        let mut out: Vec<UiDraw> = Vec::new();
        self.root.draw(&ctx, &mut out);
        let files = self.ui.files();
        out.iter()
            .filter_map(|d| match d {
                UiDraw::Image(i) => Some((
                    files.name(i.image.file).unwrap().to_string(),
                    i.image.frame,
                    i.at.x,
                    i.at.y,
                )),
                _ => None,
            })
            .collect()
    }
}

fn panel_images(v: &[(String, u32, i32, i32)], prefix: &str) -> Vec<(u32, i32, i32)> {
    v.iter()
        .filter(|(n, ..)| n.starts_with(prefix))
        .map(|(_, f, x, y)| (*f, *x, *y))
        .collect()
}

// (the cursor item and the step-10 tips in the last panel, after the HUD of
// step 7; the HUD before the NPC menu family)
#[test]
fn install_mirrors_the_flags_and_keeps_the_border_open() {
    let u = ui(Some(areas()), true);
    assert_eq!(
        u.root.open_panels(),
        vec![
            // Step 2: the equipment warning icons, before every panel.
            hud::WARN_PANEL,
            BORDER_PANEL,
            hud::HUD_PANEL,
            crate::ui::hire_list::HIRE_PANEL,
            crate::ui::original::gold_dialog::GOLD_PANEL,
            crate::ui::original::game_messages::MESSAGES_PANEL,
            crate::ui::original::overhead_ui::OVERHEAD_PANEL,
            crate::ui::original::npc_talk::DIALOG_PANEL,
            crate::ui::original::TOP_PANEL
        ]
    );
    let w = world(AMAZON, 1, true);
    let img = u.images(&w);
    // Mode 0: no border; the 800 × 600 control panel base, six frames.
    assert!(panel_images(&img, "panel\\800borderframe").is_empty());
    let ctrl = panel_images(&img, "panel\\800ctrlpnl7");
    assert_eq!(ctrl.len(), 6);
    assert_eq!(ctrl[0], (0, 0, 600));
    assert_eq!(ctrl[5], (5, 800 - 117, 600));
}

#[test]
fn hotkeys_toggle_their_state_with_the_specs_jump() {
    let mut u = ui(Some(areas()), true);
    let w = world(AMAZON, 1, true);
    // (corrected 2026-10-07, vector "key I at 800 × 600, mouse x 500"):
    // the Inventory key passes jump 1, so the cursor goes to x 300.
    u.send(&w, UiEvent::CursorMoved(Point::new(500, 300)));
    assert_eq!(u.key(&w, Action::ToggleInventory), Routed::Unhandled);
    assert!(u.ui.is_open(UI_INVENTORY));
    assert_eq!(u.ui.open_mode().get(), 1);
    assert_eq!(
        u.ui.take_outcome().effects,
        vec![
            UiEffect::Opened(1),
            UiEffect::InventoryHook,
            UiEffect::OpenMode(OpenMode::new(1).unwrap()),
            UiEffect::CursorX(300)
        ]
    );
    assert_eq!(
        u.root.open_panels(),
        vec![
            hud::WARN_PANEL,
            PanelId(1),
            BORDER_PANEL,
            hud::HUD_PANEL,
            crate::ui::hire_list::HIRE_PANEL,
            crate::ui::original::gold_dialog::GOLD_PANEL,
            crate::ui::original::game_messages::MESSAGES_PANEL,
            crate::ui::original::overhead_ui::OVERHEAD_PANEL,
            crate::ui::original::npc_talk::DIALOG_PANEL,
            crate::ui::original::TOP_PANEL
        ],
        "the root mirrors the flag"
    );
    // The border shows on the right (mode 1): frames 5–9.
    let border = panel_images(&u.images(&w), "panel\\800borderframe");
    assert_eq!(
        border.iter().map(|b| b.0).collect::<Vec<_>>(),
        vec![5, 6, 7, 8, 9]
    );
    u.key(&w, Action::ToggleInventory);
    assert!(!u.ui.is_open(UI_INVENTORY));
    assert_eq!(u.ui.open_mode().get(), 0);
    // An action without a state does nothing.
    u.key(&w, Action::ToggleRun);
    assert_eq!(
        u.root.open_panels(),
        vec![
            hud::WARN_PANEL,
            BORDER_PANEL,
            hud::HUD_PANEL,
            crate::ui::hire_list::HIRE_PANEL,
            crate::ui::original::gold_dialog::GOLD_PANEL,
            crate::ui::original::game_messages::MESSAGES_PANEL,
            crate::ui::original::overhead_ui::OVERHEAD_PANEL,
            crate::ui::original::npc_talk::DIALOG_PANEL,
            crate::ui::original::TOP_PANEL
        ]
    );
}

// The Quest Log key passes jump 1 on the open and the close (measured:
// `a1-panel-waypoint`, mouse (320, 240) → (520, 237) on the open, → (320, 234)
// on the close at 800 × 600).
#[test]
fn the_quest_log_key_moves_the_cursor_both_ways() {
    let mut u = ui(Some(areas()), true);
    let w = world(AMAZON, 1, true);
    u.send(&w, UiEvent::CursorMoved(Point::new(320, 240)));
    u.key(&w, Action::ToggleQuests);
    assert!(u.ui.is_open(0x0F));
    assert_eq!(
        u.ui.take_outcome().effects.last(),
        Some(&UiEffect::CursorX(520))
    );
    u.send(&w, UiEvent::CursorMoved(Point::new(520, 237)));
    u.key(&w, Action::ToggleQuests);
    assert!(!u.ui.is_open(0x0F));
    assert_eq!(
        u.ui.take_outcome().effects.last(),
        Some(&UiEffect::CursorX(320))
    );
}

#[test]
fn conflict_table_vectors() {
    let mut u = ui(Some(areas()), true);
    let w = world(AMAZON, 1, true);
    // Inventory open, character on: C[1][2] = 0, both open, mode 3.
    u.key(&w, Action::ToggleInventory);
    u.key(&w, Action::ToggleCharacter);
    assert!(u.ui.is_open(UI_INVENTORY) && u.ui.is_open(UI_CHARACTER));
    assert_eq!(u.ui.open_mode().get(), 3);
    // Character off, then the skill tree: C[1][4] = 1 closes the
    // inventory (mode 0), then the skill tree opens (mode 1).
    u.key(&w, Action::ToggleCharacter);
    u.ui.take_outcome();
    u.key(&w, Action::ToggleSkillTree);
    assert!(!u.ui.is_open(UI_INVENTORY) && u.ui.is_open(UI_SKILLTREE));
    assert_eq!(u.ui.open_mode().get(), 1);
    let fx = u.ui.take_outcome().effects;
    assert_eq!(fx[0], UiEffect::Closed(1));
    // The mode change, then only the cursor jump (the mouse
    // starts at the cursor init's (320, 240)).
    let mode = fx
        .iter()
        .rposition(|e| *e == UiEffect::OpenMode(OpenMode::new(1).unwrap()))
        .expect("mode 1");
    assert!(
        matches!(fx[mode + 1..], [] | [UiEffect::CursorX(_)]),
        "{fx:?}"
    );
}

#[test]
fn a_dead_player_cannot_toggle_a_panel() {
    let mut u = ui(Some(areas()), true);
    let dead = world(AMAZON, 0x11, true);
    u.key(&dead, Action::ToggleInventory);
    assert!(!u.ui.is_open(UI_INVENTORY));
    // No local player: allowed.
    u.key(&ClientWorld::default(), Action::ToggleInventory);
    assert!(u.ui.is_open(UI_INVENTORY));
}

#[test]
fn inventory_draws_its_art_and_its_close_button_closes_it() {
    let mut u = ui(Some(areas()), true);
    let w = world(AMAZON, 1, true);
    u.key(&w, Action::ToggleInventory);
    // Vectors, 800 × 600: frames 4–7 at (400, 316), (656, 316),
    // (400, 492), (656, 492); close button (418, 476).
    let img = u.images(&w);
    assert_eq!(
        panel_images(&img, "panel\\invchar6"),
        vec![(4, 400, 316), (5, 656, 316), (6, 400, 492), (7, 656, 492)]
    );
    assert_eq!(
        panel_images(&img, "panel\\buysellbtn"),
        vec![(10, 418, 476)]
    );
    // Press: frame 11; release in the close rectangle: SetUIState(1, off).
    let at = Point::new(430, 460);
    let b = PointerButton::Left;
    assert_eq!(
        u.send(&w, UiEvent::Press { button: b, at }),
        Routed::Panel(PanelId(1))
    );
    assert_eq!(
        panel_images(&u.images(&w), "panel\\buysellbtn"),
        vec![(11, 418, 476)]
    );
    u.send(&w, UiEvent::Release { button: b, at });
    assert!(!u.ui.is_open(UI_INVENTORY));
    assert_eq!(u.ui.open_mode().get(), 0);
    // A click elsewhere in the area is consumed and closes nothing.
    u.key(&w, Action::ToggleInventory);
    assert_eq!(
        u.click(&w, Point::new(600, 200)),
        (Routed::Panel(PanelId(1)), Routed::Panel(PanelId(1)))
    );
    assert!(u.ui.is_open(UI_INVENTORY));
    // Outside the `inv` rectangle: not the panel's.
    assert_eq!(u.click(&w, Point::new(100, 200)).0, Routed::Unhandled);
}

#[test]
fn without_the_inventory_table_the_right_panels_take_no_click() {
    let mut u = ui(None, true);
    let w = world(AMAZON, 1, true);
    u.key(&w, Action::ToggleInventory);
    assert_eq!(
        u.click(&w, Point::new(430, 460)),
        (Routed::Unhandled, Routed::Unhandled)
    );
    assert!(u.ui.is_open(UI_INVENTORY));
}

#[test]
fn skill_tree_art_tabs_and_close() {
    let mut u = ui(Some(areas()), true);
    let w = world(AMAZON, 1, true);
    u.key(&w, Action::ToggleSkillTree);
    // Vectors: amazon tab 1, frames 0–3 then 4–7 at (400, 316) …;
    // close button at (571, 477).
    let img = u.images(&w);
    let art = panel_images(&img, "spells\\skltree_a_back");
    assert_eq!(
        art,
        vec![
            (0, 400, 316),
            (1, 656, 316),
            (2, 400, 492),
            (3, 656, 492),
            (4, 400, 316),
            (5, 656, 316),
            (6, 400, 492),
            (7, 656, 492)
        ]
    );
    assert_eq!(
        panel_images(&img, "panel\\buysellbtn"),
        vec![(10, 571, 477)]
    );
    // Tab 2 (mouse down in its rectangle): sound 6, frames 8–11.
    let tab2 = Point::new(650, 300);
    assert_eq!(skilltree_tab(tab2), Some(2));
    u.click(&w, tab2);
    assert_eq!(
        u.ui.take_outcome().sounds,
        vec![crate::audio::driver::SoundRequest::Ui(6)]
    );
    let art = panel_images(&u.images(&w), "spells\\skltree_a_back");
    assert_eq!(
        art[4..].iter().map(|a| a.0).collect::<Vec<_>>(),
        vec![8, 9, 10, 11]
    );
    // The same tab again: no sound.
    u.click(&w, tab2);
    assert!(u.ui.take_outcome().sounds.is_empty());
    // Close: amazon tab 2 offset −220 → button at (500, 477); release in
    // its rectangle toggles ui 4 off.
    u.click(&w, Point::new(510, 460));
    assert!(!u.ui.is_open(UI_SKILLTREE));
}

fn skilltree_tab(p: Point) -> Option<u8> {
    super::super::panels::skilltree::tab_at(&Screen::R800, p)
}

#[test]
fn character_art_and_close_button() {
    let mut u = ui(Some(areas()), true);
    let w = world(AMAZON, 1, true);
    u.key(&w, Action::ToggleCharacter);
    assert_eq!(u.ui.open_mode().get(), 2);
    // Vectors: quads at (80, 316), (336, 316), (80, 492), (336, 492).
    let img = u.images(&w);
    assert_eq!(
        panel_images(&img, "panel\\invchar6"),
        vec![(0, 80, 316), (1, 336, 316), (2, 80, 492), (3, 336, 492)]
    );
    assert_eq!(
        panel_images(&img, "panel\\buysellbtn"),
        vec![(10, 208, 480)]
    );
    // No stat-point box or add buttons (`PENDING`).
    assert!(panel_images(&img, "panel\\skillpoints").is_empty());
    // Only the control panel's: the help button's socket and button (state 2
    // does not hide it: `a1-panel-character`; the filter's prefix takes
    // `levelsocket` too) and the two closed level buttons (frame 2).
    assert_eq!(
        panel_images(&img, "panel\\level"),
        vec![(0, 725, 440), (0, 728, 436), (2, 206, 592), (2, 563, 592)]
    );
    // A classic install draws `InvChar`.
    let mut c = ui(Some(areas()), false);
    c.key(&w, Action::ToggleCharacter);
    assert_eq!(panel_images(&c.images(&w), "panel\\invchar").len(), 4);
    assert!(panel_images(&c.images(&w), "panel\\invchar6").is_empty());
    // Release in the close rectangle: SetUIState(2, off).
    u.click(&w, Point::new(215, 460));
    assert!(!u.ui.is_open(UI_CHARACTER));
    assert_eq!(u.ui.open_mode().get(), 0);
}

#[test]
fn panel_intents_leave_through_the_root_in_order() {
    let mut u = ui(Some(areas()), true);
    let w = world(AMAZON, 1, true);
    let a = ClientIntent(vec![0x3A, 0, 0]);
    let b = ClientIntent(vec![0x3B, 6, 0]);
    u.ui.shared.borrow_mut().outputs.extend([
        PanelOutput::Intent(a.clone()),
        PanelOutput::SetUi {
            ui: UI_INVENTORY,
            mode: 0,
            jump: false,
        },
        PanelOutput::Intent(b.clone()),
    ]);
    u.ui.after_event(&mut u.root, UiEvent::CursorLeft, Routed::Unhandled)
        .unwrap();
    assert_eq!(u.root.intents(), &[a, b]);
    assert!(u.ui.is_open(UI_INVENTORY));
    let _ = w;
}

#[test]
fn a_bad_state_request_is_an_error() {
    let mut u = ui(None, true);
    assert!(matches!(
        u.ui.set_ui(0x26, 0, false),
        Err(UiStateError::BadState(0x26))
    ));
    u.ui.shared.borrow_mut().outputs.push(PanelOutput::SetUi {
        ui: 1,
        mode: 3,
        jump: false,
    });
    assert!(u
        .ui
        .after_event(&mut u.root, UiEvent::CursorLeft, Routed::Unhandled)
        .is_err());
}

#[test]
fn hotkey_states_and_records() {
    assert_eq!(
        hotkey_state(ActionId(Action::ToggleInventory.index() as u16)),
        Some(1)
    );
    assert_eq!(
        hotkey_state(ActionId(Action::ToggleCharacter.index() as u16)),
        Some(2)
    );
    assert_eq!(
        hotkey_state(ActionId(Action::ToggleSkillTree.index() as u16)),
        Some(4)
    );
    assert_eq!(
        hotkey_state(ActionId(Action::GameMenu.index() as u16)),
        None
    );
    // The inventory class records, + 16 at 800 × 600.
    assert_eq!(inventory_record(0, &Screen::R800), Some(16));
    assert_eq!(inventory_record(5, &Screen::R640), Some(14));
    assert_eq!(inventory_record(6, &Screen::R800), Some(31));
    assert_eq!(inventory_record(7, &Screen::R800), None);
    assert!(PENDING
        .iter()
        .all(|(what, why)| !what.is_empty() && !why.is_empty()));
}

#[test]
fn inventory_draws_the_gold_value_and_button_from_the_model() {
    let mut u = ui(Some(areas()), true);
    let mut w = world(AMAZON, 1, true);
    let key = w.local_player.unwrap();
    w.units.get_mut(&key).unwrap().stats.insert(14, 4321);
    // A state list adds 9: the line shows the full value.
    w.units
        .get_mut(&key)
        .unwrap()
        .state_lists
        .insert(1, [((14u16, 0u16), 9)].into_iter().collect());
    u.key(&w, Action::ToggleInventory);
    // The button needs no player: (W − sx − 236, H + sy − 71).
    assert_eq!(
        panel_images(&u.images(&w), "panel\\goldcoinbtn"),
        vec![(0, 484, 469)]
    );
    let gold_texts = |u: &Ui| -> Vec<(String, i32, i32, u16, u16)> {
        let ctx = UiCtx {
            tick: 0,
            world: &w,
            strings: &NoStrings,
        };
        let mut out: Vec<UiDraw> = Vec::new();
        u.root.draw(&ctx, &mut out);
        out.iter()
            .filter_map(|d| match d {
                UiDraw::Text(t) => Some((
                    String::from_utf16_lossy(&t.text),
                    t.at.x,
                    t.at.y,
                    t.style.font,
                    t.style.color,
                )),
                _ => None,
            })
            .collect()
    };
    // Without the fonts bound no text is drawn.
    assert!(gold_texts(&u).is_empty());
    let mut f = FontMeasure::default();
    f.insert(
        1,
        FontTable::parse(&character_bind_tests::tbl(6)).expect("tbl"),
    );
    u.ui.set_fonts(f);
    // `%d` of stat 14 total, Font16, color 0, at (W − sx − 212, H + sy − 72).
    assert_eq!(gold_texts(&u), vec![("4330".to_string(), 508, 468, 1, 0)]);
}

// D2RUST's own, unverified: Esc order
#[test]
fn esc_opens_the_game_menu_closes_panels_first_and_closes_it_again() {
    let mut u = ui(Some(areas()), true);
    let w = world(AMAZON, 1, true);
    assert!(!u.ui.is_open(9));
    // Nothing open: Esc opens the menu.
    u.key(&w, Action::GameMenu);
    assert!(u.ui.is_open(9));
    assert!(u.root.open_panels().contains(&esc_menu::ESC_PANEL));
    // Esc again closes it.
    u.key(&w, Action::GameMenu);
    assert!(!u.ui.is_open(9));
    // A panel open: Esc closes the panel only; the next Esc opens the menu.
    u.key(&w, Action::ToggleInventory);
    assert!(u.ui.is_open(1));
    u.key(&w, Action::GameMenu);
    assert!(!u.ui.is_open(1) && !u.ui.is_open(9));
    u.key(&w, Action::GameMenu);
    assert!(u.ui.is_open(9));
}

// The close-all closes only Esc-closable states (flag 1): the automap
// (0x0A, flag 0) does not block the menu; the menu's open closes it and
// remembers it (keep = 1), the menu's close reopens it. Chat (5, flag 1)
// is closed by the first Esc and the menu stays shut.
#[test]
fn esc_closes_closable_states_and_the_menu_restores_the_kept_ones() {
    let mut u = ui(Some(areas()), true);
    let w = world(AMAZON, 1, true);
    u.key(&w, Action::ToggleInventory);
    assert!(u.ui.is_open(1));
    u.ui.set_ui(0x0A, 0, false).expect("automap on");
    assert!(u.ui.is_open(0x0A));
    // First Esc: the inventory (flag 1) closes; the automap stays.
    u.key(&w, Action::GameMenu);
    assert!(!u.ui.is_open(1) && !u.ui.is_open(9) && u.ui.is_open(0x0A));
    // Second Esc: nothing closable is open, the menu opens and closes
    // the automap.
    u.key(&w, Action::GameMenu);
    assert!(u.ui.is_open(9) && !u.ui.is_open(0x0A));
    // Esc with the menu open: the menu closes, the automap reopens.
    u.key(&w, Action::GameMenu);
    assert!(!u.ui.is_open(9) && u.ui.is_open(0x0A));
    // Chat is Esc-closable: the first Esc closes it, the menu stays shut.
    if u.ui.set_ui(5, 0, false).expect("chat on") {
        u.key(&w, Action::GameMenu);
        assert!(!u.ui.is_open(5) && !u.ui.is_open(9));
    }
}

#[test]
fn esc_without_a_player_opens_nothing() {
    let mut u = ui(Some(areas()), true);
    u.key(&ClientWorld::default(), Action::GameMenu);
    assert!(!u.ui.is_open(9));
}

// The rows of the tree replace the old three-entry / Options page; the
// Game menu rows are now at the spec's y tops 185 / 235 / 285.
#[test]
fn the_menu_tree_returns_saves_exits_and_swallows_clicks() {
    let mut u = ui(Some(areas()), true);
    let w = world(AMAZON, 1, true);
    let texts = |u: &Ui| -> Vec<String> {
        let ctx = UiCtx {
            tick: 0,
            world: &w,
            strings: &NoStrings,
        };
        let mut out: Vec<UiDraw> = Vec::new();
        u.root.draw(&ctx, &mut out);
        out.iter()
            .filter_map(|d| match d {
                UiDraw::Text(t) => Some(String::from_utf16_lossy(&t.text)),
                _ => None,
            })
            .collect()
    };
    let art_names = |u: &Ui, w: &ClientWorld| -> Vec<String> {
        let ctx = UiCtx {
            tick: 0,
            world: w,
            strings: &NoStrings,
        };
        let mut out: Vec<UiDraw> = Vec::new();
        u.root.draw(&ctx, &mut out);
        let sh = u.ui.shared.borrow();
        // Menu art only (not the HUD), one entry per image (multi-frame ones tile).
        let mut v: Vec<String> = out
            .iter()
            .filter_map(|d| match d {
                UiDraw::Image(i) => sh.tables.files.name(i.image.file).map(str::to_string),
                _ => None,
            })
            .filter(|n| n.starts_with("*local") || n.starts_with("cursor\\pentspin"))
            .collect();
        v.dedup();
        v
    };
    assert!(texts(&u).is_empty());
    u.key(&w, Action::GameMenu);
    // The labels are DC6 images now (spec: no font): no text.
    assert!(texts(&u).is_empty());
    assert_eq!(
        art_names(&u, &w),
        vec![
            "*local\\options",
            "*local\\exit",
            "*local\\returntogame",
            "cursor\\pentspin"
        ]
    );
    // Rows: Game menu tops 185 / 235 / 285 (click inside the 50 px row).
    let row = |i: i32| Point::new(400, 185 + 50 * i + 20);
    let (_, r) = u.click(&w, row(0));
    assert_ne!(r, Routed::Unhandled);
    assert!(u.ui.is_open(9) && !u.ui.take_exit_request());
    // An action entry activated: sound 2.
    use crate::audio::driver::SoundRequest;
    let sounds = u.ui.take_outcome().sounds;
    assert_eq!(sounds.last(), Some(&SoundRequest::Ui(2)), "{sounds:?}");
    assert_eq!(
        art_names(&u, &w)[..5],
        [
            "*local\\soundoptions",
            "*local\\videooptions",
            "*local\\automapoptions",
            "*local\\cfgoptions",
            "*local\\previous"
        ]
    );
    // Options rows (tops 135, 185,...): Video Options.
    let orow = |i: i32| Point::new(400, 135 + 50 * i + 20);
    u.click(&w, orow(1));
    let t = texts(&u);
    assert_eq!(art_names(&u, &w)[0], "*local\\videooptions");
    // D2RUST's own display rows draw as text.
    assert!(t.contains(&"Window".to_string()));
    assert!(t.contains(&"Window Size".to_string()));
    assert!(t.contains(&"VSync".to_string()));
    assert!(t.contains(&"Frame Limit".to_string()));
    assert!(t.contains(&"Animation frames".to_string()));
    // View draws as text too, 16:9 by default.
    assert!(t.contains(&"View (next start)".to_string()));
    assert!(t.contains(&"16:9 Widescreen".to_string()));
    // The info lines have no full-width dark backing strip.
    assert!(u.images(&w).iter().all(|(file, frame, _, _)| {
        file != super::hud::FILL_FILE || *frame != super::esc_menu::DARK
    }));
    // The info lines above the rows.
    assert!(t.iter().any(|s| s.starts_with("Display: ")), "{t:?}");
    assert!(t.iter().any(|s| s.starts_with("Picture: ")), "{t:?}");
    // Video rows: title, View, Window, Window Size, Scale, VSync,
    // Frame Limit, Animation frames, Light Quality.
    let m = u.ui.shared.borrow().esc.menu.clone();
    let wm = Point::new(400, m.y_top(8) + 20);
    // Light Quality cycles and reports the change once; the value round-trips
    // through the config text.
    assert!(u.ui.take_settings_change().is_none());
    u.ui.take_outcome();
    u.click(&w, wm);
    // A choice entry activated: sound 1.
    assert_eq!(u.ui.take_outcome().sounds, [SoundRequest::Ui(1)]);
    let s = u.ui.take_settings_change().unwrap();
    assert_eq!(s.light_quality, 0);
    assert!(u.ui.take_settings_change().is_none());
    let back = crate::app::config::parse_settings(&crate::app::config::write_settings(&s));
    assert_eq!(back.unwrap(), s);
    // Previous Menu (last row) twice: Options, then the Game menu.
    for _ in 0..2 {
        let m = u.ui.shared.borrow().esc.menu.clone();
        let last = m.rows().len() - 1;
        u.click(&w, Point::new(400, m.y_top(last) + 20));
    }
    assert_eq!(art_names(&u, &w)[0], "*local\\options");
    // A click outside the rows does not reach the world either.
    let (_, r) = u.click(&w, Point::new(10, 10));
    assert_ne!(r, Routed::Unhandled);
    // Arrow keys and Enter: Down wraps to Options; Right on a choice row.
    u.root_char(&w, 0xF028);
    u.root_char(&w, 0x0D);
    assert_eq!(art_names(&u, &w)[0], "*local\\soundoptions");
    // Configure Controls is requested once.
    for _ in 0..4 {
        u.root_char(&w, 0xF028);
    }
    u.root_char(&w, 0x0D);
    assert!(u.ui.take_controls_request());
    assert!(!u.ui.take_controls_request());
    // Esc closes the whole menu from a sub-menu.
    u.key(&w, Action::GameMenu);
    assert!(!u.ui.is_open(9));
    // Reopened: first page, Save and Exit asks the host once.
    u.key(&w, Action::GameMenu);
    u.click(&w, row(1));
    assert!(u.ui.take_exit_request());
    assert!(!u.ui.take_exit_request());
    // Return closes.
    u.click(&w, row(2));
    assert!(!u.ui.is_open(9));
}

#[test]
fn configure_controls_opens_over_the_game_and_applies_the_bindings() {
    use crate::controls::{Action as Act, Key};
    let mut u = ui(Some(areas()), true);
    let w = world(AMAZON, 1, true);
    let dir = std::env::temp_dir().join(format!("d2rust-ctl-ingame-{}", std::process::id()));
    let path = dir.join("controls.toml");
    let open = |u: &mut Ui| {
        u.key(&w, Action::GameMenu);
        u.root_char(&w, 0xF028);
        u.root_char(&w, 0x0D);
        for _ in 0..4 {
            u.root_char(&w, 0xF028);
        }
        u.root_char(&w, 0x0D);
        assert!(u.ui.service_controls(false, Some(path.clone())));
        assert!(u.ui.controls_open() && u.ui.is_open(9));
    };
    let rebind_inventory = |u: &mut Ui| {
        // Down to Inventory (command 1), Enter, then C.
        loop {
            let sh = u.ui.shared.borrow();
            let m = sh.esc.controls.as_ref().unwrap().model();
            if m.rows()[m.selected()].cmd == 1 {
                break;
            }
            drop(sh);
            assert!(u.ui.controls_key(0x28));
        }
        u.ui.controls_key(0x0D);
        u.ui.controls_key(0x43);
    };
    let button = |i: i32| Point::new(90 + 206 * i + 103, 70 + 350);
    // Cancel restores: nothing accepted, back on Options with Previous.
    open(&mut u);
    let drawn: Vec<String> = {
        let ctx = UiCtx {
            tick: 0,
            world: &w,
            strings: &NoStrings,
        };
        let mut out: Vec<UiDraw> = Vec::new();
        u.root.draw(&ctx, &mut out);
        out.iter()
            .filter_map(|d| match d {
                UiDraw::Text(t) if t.style.font == 13 => Some(String::from_utf16_lossy(&t.text)),
                _ => None,
            })
            .collect()
    };
    for want in ["Cancel", "Default", "Accept", "Key / Button One"] {
        assert!(drawn.iter().any(|t| t == want), "{want}: {drawn:?}");
    }
    rebind_inventory(&mut u);
    u.click(&w, button(0));
    assert!(!u.ui.controls_open() && u.ui.take_accepted_bindings().is_none());
    {
        let sh = u.ui.shared.borrow();
        let m = &sh.esc.menu;
        assert_eq!(m.menu, crate::ui::original::options_menu::MenuId::Options);
        assert_eq!(m.selected, m.rows().len() - 1);
    }
    assert!(!path.exists());
    // Accept applies the new key and writes it.
    u.key(&w, Action::GameMenu);
    open(&mut u);
    rebind_inventory(&mut u);
    u.click(&w, button(2));
    assert!(!u.ui.controls_open() && u.ui.is_open(9));
    let b = u.ui.take_accepted_bindings().expect("accepted");
    assert!(b.inputs(Act::ToggleInventory).contains(&Key::C));
    assert!(path.is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_mini_panel_game_menu_button_opens_it() {
    let mut u = ui(Some(areas()), true);
    let w = world(AMAZON, 1, true);
    u.ui.set_ui(0x15, 0, false).unwrap();
    u.root.sync_states(&u.ui.shared.borrow().states);
    // The hit test reads the layout of the last draw: the
    // frame drew layout 2.
    u.ui.shared.borrow_mut().hud.mini.last_layout =
        Some(crate::ui::panels::control::minipanel::Layout::Two);
    // Row 7 of the mini panel; the exact rectangle is the spec's, so
    // find it by scanning the panel strip for a click that opens ui 9.
    let mut opened = false;
    'scan: for y in 440..600 {
        for x in (250..560).step_by(2) {
            u.click(&w, Point::new(x, y));
            if u.ui.is_open(9) {
                opened = true;
                break 'scan;
            }
            if !u.ui.is_open(0x15) {
                u.ui.set_ui(0x15, 0, false).unwrap();
                u.root.sync_states(&u.ui.shared.borrow().states);
            }
        }
    }
    assert!(opened);
}

#[test]
fn gold_dialog_and_esc_menu_draw_by_the_screen_at_640_and_800() {
    // The gold button x in [W − sx − 237, W − sx − 217], y in
    // [H + sy − 87, H + sy − 69]: (493, 462) at 800 × 600 (sx 80, sy −60),
    // (413, 400) at 640 × 480 (sx 0, sy 0).
    for (screen, button, half) in [
        (Screen::R800, Point::new(493, 462), 400),
        (Screen::R640, Point::new(413, 400), 320),
    ] {
        let mut u = ui_at(screen, Some(areas()), true);
        let mut w = world(AMAZON, 1, true);
        let key = w.local_player.unwrap();
        w.units.get_mut(&key).unwrap().stats.insert(14, 5000);
        u.key(&w, Action::ToggleInventory);
        let b = PointerButton::Left;
        u.send(
            &w,
            UiEvent::Press {
                button: b,
                at: button,
            },
        );
        u.send(
            &w,
            UiEvent::Release {
                button: b,
                at: button,
            },
        );
        let ctx = UiCtx {
            tick: 0,
            world: &w,
            strings: &NoStrings,
        };
        let mut out: Vec<UiDraw> = Vec::new();
        u.root.draw(&ctx, &mut out);
        let clips: Vec<_> = out
            .iter()
            .filter_map(|d| match d {
                UiDraw::Text(t) if t.style.font == 1 => Some(t.clip),
                _ => None,
            })
            .collect();
        assert!(!clips.is_empty(), "the dialog opened at {screen:?}");
        assert!(clips.iter().all(|c| *c == screen.rect()), "{screen:?}");
        // The Esc menu: Return to Game centred on h at 640 / 800.
        // Esc closes the dialog, then the panel, then opens the menu.
        for _ in 0..4 {
            if !u.ui.is_open(9) {
                u.key(&w, Action::GameMenu);
            }
        }
        assert!(u.ui.is_open(9));
        let mut out: Vec<UiDraw> = Vec::new();
        u.root.draw(&ctx, &mut out);
        let pents: Vec<i32> = out
            .iter()
            .filter_map(|d| match d {
                UiDraw::Image(i) if u.ui.files().name(i.image.file) == Some("cursor\\pentspin") => {
                    assert_eq!(i.clip, screen.rect());
                    Some(i.at.x)
                }
                _ => None,
            })
            .collect();
        assert_eq!(pents, [half - 301, half + 249], "{screen:?}");
    }
}

// D2RUST's own, unverified: the drop-gold dialog; the button and the OK request
// are the dialog's own.
#[test]
fn the_gold_button_opens_the_dialog_and_ok_sends_drop_gold() {
    let mut u = ui(Some(areas()), true);
    let mut w = world(AMAZON, 1, true);
    let key = w.local_player.unwrap();
    w.units.get_mut(&key).unwrap().stats.insert(14, 5000);
    u.key(&w, Action::ToggleInventory);
    let texts = |u: &Ui| -> Vec<String> {
        let ctx = UiCtx {
            tick: 0,
            world: &w,
            strings: &NoStrings,
        };
        let mut out: Vec<UiDraw> = Vec::new();
        u.root.draw(&ctx, &mut out);
        out.iter()
            .filter_map(|d| match d {
                UiDraw::Text(t) => Some(String::from_utf16_lossy(&t.text)),
                _ => None,
            })
            .collect()
    };
    assert!(texts(&u).is_empty());
    // X in [W − sx − 237, W − sx − 217], y in [H + sy − 87, H + sy − 69].
    let button = Point::new(493, 462);
    let b = PointerButton::Left;
    // A press sets the flag (the button's frame 1, one row lower).
    u.send(
        &w,
        UiEvent::Press {
            button: b,
            at: button,
        },
    );
    assert_eq!(
        panel_images(&u.images(&w), "panel\\goldcoinbtn"),
        vec![(1, 484, 470)]
    );
    // The release in the rectangle opens the dialog.
    u.send(
        &w,
        UiEvent::Release {
            button: b,
            at: button,
        },
    );
    assert_eq!(
        panel_images(&u.images(&w), "panel\\goldcoinbtn"),
        vec![(0, 484, 469)]
    );
    assert!(texts(&u).contains(&"do you want to drop?".to_string()));
    // Digits fill the edit box, letters are ignored; past the maximum
    // (stat 14) the box takes the maximum.
    for c in "15x00".chars() {
        let r = u.send(&w, UiEvent::Char(c as u16));
        assert_ne!(r, Routed::Unhandled);
    }
    assert!(texts(&u).contains(&"1500_".to_string()));
    // The belt keys and the menu key do not reach the game while open.
    assert_ne!(u.key(&w, Action::BeltSlot1), Routed::Unhandled);
    // Enter is OK: C→S 0x50 [player GUID][1500]; the dialog closes.
    u.send(&w, UiEvent::Char(0x0D));
    let guid = key.guid.to_le_bytes();
    let mut want = vec![0x50];
    want.extend_from_slice(&guid);
    want.extend_from_slice(&1500u32.to_le_bytes());
    assert_eq!(u.root.intents(), &[ClientIntent(want)]);
    assert!(!texts(&u).contains(&"do you want to drop?".to_string()));
    // Cancel (Esc) sends nothing and the Esc menu stays shut.
    u.send(
        &w,
        UiEvent::Press {
            button: b,
            at: button,
        },
    );
    u.send(
        &w,
        UiEvent::Release {
            button: b,
            at: button,
        },
    );
    for c in "99999".chars() {
        u.send(&w, UiEvent::Char(c as u16));
    }
    assert!(texts(&u).contains(&"5000_".to_string()));
    assert_ne!(u.key(&w, Action::GameMenu), Routed::Unhandled);
    assert!(!u.ui.is_open(9));
    assert_eq!(u.root.intents().len(), 1);
    assert!(!texts(&u).contains(&"do you want to drop?".to_string()));
    // Zero is not sent.
    u.send(
        &w,
        UiEvent::Press {
            button: b,
            at: button,
        },
    );
    u.send(
        &w,
        UiEvent::Release {
            button: b,
            at: button,
        },
    );
    u.send(&w, UiEvent::Char(0x0D));
    assert_eq!(u.root.intents().len(), 1);
}

// D2RUST's own, unverified: the stash gold dialogs.
#[test]
fn stash_gold_withdraw_and_deposit_send_0x4f() {
    let mut u = ui(Some(areas()), true);
    let mut w = world(AMAZON, 1, true);
    let key = w.local_player.unwrap();
    w.units.get_mut(&key).unwrap().stats.insert(14, 5000);
    w.units.get_mut(&key).unwrap().stats.insert(15, 70000);
    u.ui.set_ui(0x19, 0, false).unwrap();
    u.root.sync_states(&u.ui.shared.borrow().states);
    assert!(u.ui.is_open(0x19));
    let texts = |u: &Ui| -> Vec<String> {
        let ctx = UiCtx {
            tick: 0,
            world: &w,
            strings: &NoStrings,
        };
        let mut out: Vec<UiDraw> = Vec::new();
        u.root.draw(&ctx, &mut out);
        out.iter()
            .filter_map(|d| match d {
                UiDraw::Text(t) => Some(String::from_utf16_lossy(&t.text)),
                _ => None,
            })
            .collect()
    };
    // The stash gold button (kind 4, withdraw): typed 1234 -> 0x4F 0x13.
    let s = Screen::R800;
    let btn = Point::new(s.sx() + 80, s.h + s.sy() - 455 + 5);
    u.click(&w, btn);
    assert!(texts(&u).contains(&"_".to_string()));
    for c in "1234".chars() {
        u.send(&w, UiEvent::Char(c as u16));
    }
    u.send(&w, UiEvent::Char(0x0D));
    assert_eq!(
        u.root.intents(),
        &[ClientIntent(vec![0x4F, 0x13, 0, 0, 0, 0xD2, 0x04])]
    );
    // Over the stash maximum nothing grows: the field takes the stat 15.
    u.click(&w, btn);
    for c in "9999999".chars() {
        u.send(&w, UiEvent::Char(c as u16));
    }
    assert!(texts(&u).contains(&"70000_".to_string()));
    u.send(&w, UiEvent::Char(0x1B));
    assert_eq!(u.root.intents().len(), 1);
    // The inventory gold button with the stash open (kind 3, deposit):
    // the field is pre-filled with the carried gold -> 0x4F 0x14.
    let inv_btn = Point::new(493, 462);
    u.click(&w, inv_btn);
    assert!(texts(&u).contains(&"5000_".to_string()));
    u.send(&w, UiEvent::Char(0x0D));
    assert_eq!(
        u.root.intents().last(),
        Some(&ClientIntent(vec![0x4F, 0x14, 0, 0, 0, 0x88, 0x13]))
    );
}

// D2RUST's own, unverified: the stash GoldMax line reads string 4051 through
// `ctx.strings`; cap = the fixed stash limit.
#[test]
fn stash_gold_max_line_resolves_its_string_id() {
    struct Strs(Vec<u16>);
    impl crate::ui::StringLookup for Strs {
        fn get(&self, _: &str) -> Option<&[u16]> {
            None
        }
        fn get_id(&self, id: u16) -> Option<&[u16]> {
            (id == 4051).then_some(self.0.as_slice())
        }
    }
    let mut u = ui(Some(areas()), true);
    let w = world(AMAZON, 1, true);
    u.ui.apply_output(
        &crate::bridge::output::Output::TradeAction {
            code: 0x10,
            dead_or_absent: false,
        },
        &w,
    )
    .unwrap();
    // Mirror the state flags into the root (no action: no hotkey runs).
    let e = UiEvent::Press {
        button: PointerButton::Right,
        at: Point::new(0, 0),
    };
    u.ui.after_event(&mut u.root, e, Routed::Unhandled).unwrap();
    let texts = |s: &dyn crate::ui::StringLookup| -> Vec<String> {
        let ctx = UiCtx {
            tick: 0,
            world: &w,
            strings: s,
        };
        let mut out: Vec<UiDraw> = Vec::new();
        u.root.draw(&ctx, &mut out);
        out.iter()
            .filter_map(|d| match d {
                UiDraw::Text(t) => Some(String::from_utf16_lossy(&t.text)),
                _ => None,
            })
            .collect()
    };
    assert!(texts(&NoStrings).is_empty());
    assert_eq!(
        texts(&Strs("Gold Max: %d".encode_utf16().collect())),
        vec!["Gold Max: 2500000"]
    );
}

fn tree_tables() -> skill_tree_ui::SkillTreeTables {
    use skill_tree_ui::SkillTreeRow;
    let row = |skill, page, r, c, cel, req: u16| SkillTreeRow {
        skill,
        class: 0,
        page,
        row: r,
        column: c,
        icon_cel: cel,
        maxlvl: 20,
        ingame: true,
        reqlevel: req,
        reqskill: [u16::MAX; 3],
        ..Default::default()
    };
    skill_tree_ui::SkillTreeTables {
        rows: vec![
            row(6, 1, 1, 1, 2, 1),
            row(7, 1, 2, 3, 4, 6),
            row(8, 2, 3, 2, 6, 1),
        ],
    }
}

#[test]
fn skill_tree_draws_icons_and_levels_and_spends_a_point() {
    use crate::bridge::skills::{SkillEntry as Entry, SkillList, NATIVE};
    let mut w = world(AMAZON, 1, true);
    let key = w.local_player.unwrap();
    {
        let u = w.units.get_mut(&key).unwrap();
        u.stats.insert(12, 5); // level
        u.stats.insert(5, 2); // free points
        u.skills = Some(SkillList {
            entries: vec![Entry {
                skill: 6,
                base: 3,
                owner: NATIVE,
                ..Default::default()
            }],
            ..Default::default()
        });
    }
    let mut u = ui(Some(areas()), true);
    u.ui.set_skill_tree_tables(tree_tables());
    u.key(&w, Action::ToggleSkillTree);
    let img = u.images(&w);
    // Tab 1: skills 6 (column 1, row 1) and 7 (column 3, row 2).
    let icons = panel_images(&img, "spells\\amskillicon");
    assert_eq!(icons, vec![(2, 415, 122), (4, 553, 190)]);
    // The level number of skill 6 (hard points 3), none for skill 7.
    let ctx = UiCtx {
        tick: 0,
        world: &w,
        strings: &NoStrings,
    };
    let mut out: Vec<UiDraw> = Vec::new();
    u.root.draw(&ctx, &mut out);
    let numbers: Vec<_> = out
        .iter()
        .filter_map(|d| match d {
            UiDraw::Text(t) => Some((String::from_utf16_lossy(&t.text), t.at.x, t.at.y)),
            _ => None,
        })
        .filter(|(s, ..)| s == "3")
        .collect();
    assert_eq!(numbers.len(), 1);
    // A click on skill 6's icon with a free point sends 0x3B.
    u.click(&w, Point::new(430, 100));
    let want = ClientIntent::from_message(&d2_proto::client::AddSkillPoint { skill: 6 });
    assert_eq!(u.root.intents(), &[want]);
    // Skill 7 needs level 6: its click sends nothing.
    u.root.take_intents();
    u.click(&w, Point::new(570, 170));
    assert!(u.root.intents().is_empty());
}

fn ui_with_fonts(w: &ClientWorld) -> Ui {
    let mut u = ui(Some(areas()), true);
    let mut f = FontMeasure::default();
    f.insert(
        1,
        FontTable::parse(&character_bind_tests::tbl(6)).expect("tbl"),
    );
    u.ui.set_fonts(f);
    u.key(w, Action::ToggleCharacter);
    u
}

// The game sends three messages for a Shift click on Vitality with 70
// points.
#[test]
fn a_stat_button_spends_one_point_and_shift_spends_all_in_chunks_of_32() {
    let mut w = world(AMAZON, 1, true);
    let key = w.local_player.unwrap();
    w.units.get_mut(&key).unwrap().stats.insert(4, 70);
    let strength = Point::new(198, 164);
    let mut u = ui_with_fonts(&w);
    u.click(&w, strength);
    assert_eq!(u.root.intents(), &[ClientIntent(vec![0x3A, 0x00, 0x00])]);
    let mut u = ui_with_fonts(&w);
    u.ui.set_shift(true);
    u.click(&w, strength);
    assert_eq!(
        u.root.intents(),
        &[
            ClientIntent(vec![0x3A, 0x00, 0x1F]),
            ClientIntent(vec![0x3A, 0x00, 0x1F]),
            ClientIntent(vec![0x3A, 0x00, 0x05]),
        ]
    );
}

#[test]
fn a_cursor_jump_effect_becomes_a_cursor_warp_to_the_new_x_at_the_same_y() {
    let w = world(AMAZON, 1, true);
    let mut u = ui(Some(areas()), true);
    // Nothing pending: no warp.
    assert_eq!(u.ui.take_cursor_warp(), None);
    // Key I at 800 × 600 with the mouse at x 500: the cursor jumps to 300.
    let e = UiEvent::CursorMoved(Point::new(500, 77));
    u.send(&w, e);
    u.key(&w, Action::ToggleInventory);
    assert_eq!(u.ui.take_cursor_warp(), Some(Point::new(300, 77)));
}

/// The kept cursor cell on the play path: a world
/// with a 2 × 3 (`qui `, graphic 56 × 84) or 2 × 2 (`gem2`, 56 × 56)
/// item on the cursor, the panels' art rows and frame sizes set.
mod grid_hover {
    use super::*;
    use crate::bridge::items::{mode, ItemArtRow};
    use crate::ui::panels::inv_items::tests::world as item_world;

    fn grid_ui(open: &[u8]) -> Ui {
        let mut u = ui(Some(areas()), true);
        let mut art = ItemArtRows::default();
        let row = |w, h, f: &str| ItemArtRow {
            inv_w: w,
            inv_h: h,
            inv_file: f.into(),
            flippy_file: String::new(),
            beltable: false,
        };
        art.0.insert(*b"qui ", row(2, 3, "invqlt"));
        art.0.insert(*b"gem2", row(2, 2, "invgem2"));
        u.ui.set_item_art(art);
        u.ui.set_item_frame_sizes(BTreeMap::from([
            ("invqlt".to_string(), (56, 84)),
            ("invgem2".to_string(), (56, 56)),
        ]));
        for &s in open {
            u.ui.set_ui(u32::from(s), 0, false).unwrap();
        }
        u.ui.sync_root(&mut u.root);
        u
    }

    fn cursor_world(code: &[u8; 4]) -> ClientWorld {
        let mut w = item_world(&[(9, mode::CURSOR, (0, 0, 0, 0), code)], Some(9));
        let p = w.local_player.unwrap();
        w.units.get_mut(&p).unwrap().mode = 1;
        w
    }

    // The spec vector: record 16, 2 × 3 item, graphic 56 × 84. A move to
    // (500, 340) sets cell (2, 0); at (700, 340) the footprint overhangs
    // the last column, the kept cell stays (2, 0), but the drop cell
    // is recomputed from the click without the
    // overflow return, fails the placement test and sends nothing
    // (changed 2026-10-09, q-fix-ui-drop-cell: it used to place at the
    // kept cell with 0x18). A press inside the grid places.
    #[test]
    fn an_inventory_press_over_the_last_column_keeps_the_last_cell() {
        let mut u = grid_ui(&[crate::ui::states::id::INVENTORY]);
        let w = cursor_world(b"qui ");
        u.send(&w, UiEvent::CursorMoved(Point::new(500, 340)));
        u.send(&w, UiEvent::CursorMoved(Point::new(700, 340)));
        u.click(&w, Point::new(700, 340));
        assert_eq!(u.root.take_intents(), Vec::<ClientIntent>::new());
        u.click(&w, Point::new(500, 340));
        let want = ClientIntent::from_message(&crate::bridge::items::insert(9, 2, 0, 0));
        assert_eq!(u.root.take_intents(), vec![want]);
    }

    // The stash misclick: a 2 × 2 item moved over
    // stash cell (4, 2), then pressed over the right half of the last
    // column (c = (14 − 154 + 319) / 29 − 1 = 5, 2 + 5 > 6) sends nothing:
    // the drop cell fails the placement test (changed 2026-10-09,
    // q-fix-ui-drop-cell: it used to place at the kept cell (4, 2)); the
    // kept cell's own press still places on page 4.
    #[test]
    fn a_stash_press_over_the_last_column_places_at_the_kept_cell() {
        use crate::ui::states::id;
        let mut u = grid_ui(&[id::STASH]);
        // The cell arithmetic above is the expansion stash (record 28,
        // top 142); the classic stash (record 24) is at top 333 on the
        // install (q-prov-data), so the game here is an expansion game.
        let mut w = cursor_world(b"gem2");
        w.expansion = 1;
        u.send(&w, UiEvent::CursorMoved(Point::new(290, 239)));
        u.click(&w, Point::new(319, 239));
        assert_eq!(u.root.take_intents(), Vec::<ClientIntent>::new());
        u.click(&w, Point::new(290, 239));
        let want = ClientIntent::from_message(&crate::bridge::items::insert(9, 4, 2, 4));
        assert_eq!(u.root.take_intents(), vec![want]);
    }

    // The backpack beside a second grid: with the stash, the cube or the shop
    // open (the inventory flag itself stays clear), a move over a backpack
    // item makes it the hovered item, which its tip and tint read. It used
    // to track the backpack only while the inventory flag was set, so the
    // backpack showed no tips next to another grid.
    #[test]
    fn the_backpack_tracks_its_hover_beside_every_second_grid() {
        use crate::ui::states::id;
        let over_item = Point::new(430, 325);
        for open in [id::INVENTORY, id::STASH, id::CUBE, id::NPC_SHOP] {
            let mut u = grid_ui(&[open]);
            assert_eq!(u.ui.is_open(id::INVENTORY), open == id::INVENTORY);
            let mut w = item_world(&[(9, mode::STORED, (0, 0, 0, 1), b"gem2")], None);
            w.expansion = 1;
            let p = w.local_player.unwrap();
            w.units.get_mut(&p).unwrap().mode = 1;
            u.send(&w, UiEvent::CursorMoved(over_item));
            let h = u.ui.shared.borrow().items.hover.get();
            assert!(
                h.in_grid && h.item == Some(9),
                "ui {open:#x}: the backpack item is hovered ({h:?})"
            );
        }
    }
}

/// The close hooks of a close that is
/// not the panel's own button: Esc's close-all here.
mod close_hooks {
    use super::*;
    use crate::ui::states::id;

    fn alive() -> ClientWorld {
        let mut w = world(AMAZON, 1, true);
        let p = w.local_player.unwrap();
        w.units.get_mut(&p).unwrap().position = Some((100, 100));
        w
    }

    fn esc(u: &mut Ui, w: &ClientWorld) -> Vec<ClientIntent> {
        u.key(w, Action::GameMenu);
        u.root.take_intents()
    }

    // Stash open in inventory mode 0x0C (S→C 0x77 0x10), Esc: the hook
    // sends one 0x4F 0x12 and resets the mode; opened without the mode
    // (not by the server) it sends nothing.
    #[test]
    fn esc_closing_the_stash_sends_one_0x4f_0x12() {
        let w = alive();
        let mut u = ui(Some(areas()), true);
        u.ui.set_ui(u32::from(id::STASH), 0, false).unwrap();
        u.ui.msg.inventory_mode = msg_ui::MODE_STASH;
        u.ui.sync_root(&mut u.root);
        assert_eq!(
            esc(&mut u, &w),
            vec![ClientIntent(vec![0x4F, 0x12, 0, 0, 0, 0, 0])]
        );
        assert!(!u.ui.is_open(id::STASH));
        assert_eq!(u.ui.msg.inventory_mode, 0);
        // Mode 0: the hook sends nothing.
        u.ui.set_ui(u32::from(id::STASH), 0, false).unwrap();
        u.ui.sync_root(&mut u.root);
        assert!(esc(&mut u, &w).is_empty());
    }

    #[test]
    fn esc_closing_the_cube_sends_one_0x4f_0x17() {
        let w = alive();
        let mut u = ui(Some(areas()), true);
        u.ui.set_ui(u32::from(id::CUBE), 0, false).unwrap();
        u.ui.msg.inventory_mode = msg_ui::MODE_CUBE;
        u.ui.sync_root(&mut u.root);
        assert_eq!(
            esc(&mut u, &w),
            vec![ClientIntent(vec![0x4F, 0x17, 0, 0, 0, 0, 0])]
        );
        assert_eq!(u.ui.msg.inventory_mode, 0);
    }

    // The waypoint menu open (S→C 0x63 stored), Esc: the latched 0x49
    // with level 0; without a room for the player nothing is sent.
    #[test]
    fn esc_closing_the_waypoint_menu_sends_0x49_level_0() {
        let open = WaypointOpen {
            guid: 0x0A,
            record: Default::default(),
            current: 1,
            seq: 1,
            tab: 0,
        };
        let w = alive();
        let mut u = ui(Some(areas()), true);
        u.ui.shared.borrow_mut().waypoint_open = Some(open);
        u.ui.set_ui(u32::from(id::WAYPOINT), 0, false).unwrap();
        u.ui.sync_root(&mut u.root);
        let want =
            ClientIntent::from_message(&d2_proto::client::TakeOrCloseWp { wp: 0x0A, level: 0 });
        assert_eq!(esc(&mut u, &w), vec![want]);
        // No position (no room): the hook sends nothing.
        let mut nowhere = alive();
        let p = nowhere.local_player.unwrap();
        nowhere.units.get_mut(&p).unwrap().position = None;
        u.ui.set_ui(u32::from(id::WAYPOINT), 0, false).unwrap();
        u.ui.sync_root(&mut u.root);
        assert!(esc(&mut u, &nowhere).is_empty());
    }
}

/// Key commands with an `OriginalUi` handler.
mod key_commands {
    use super::*;

    // Space (command 38): the close-all; with nothing to close, the
    // automap part is handed to the host once.
    #[test]
    fn clear_screen_closes_all_then_asks_for_the_automap() {
        let w = world(AMAZON, 1, true);
        let mut u = ui(Some(areas()), true);
        u.key(&w, Action::ToggleInventory);
        assert!(u.ui.is_open(1));
        u.key(&w, Action::ClearScreen);
        assert!(!u.ui.is_open(1));
        assert!(!u.ui.take_clear_automap(), "it closed something");
        u.key(&w, Action::ClearScreen);
        assert!(u.ui.take_clear_automap());
        assert!(!u.ui.take_clear_automap());
        assert!(!u.ui.is_open(9), "Space never opens the game menu");
    }

    // O (command 54) without a hireling: nothing opens.
    #[test]
    fn the_hireling_key_needs_a_hireling() {
        let w = world(AMAZON, 1, true);
        let mut u = ui(Some(areas()), true);
        u.key(&w, Action::ToggleHireling);
        assert!(!u.ui.is_open(0x24));
    }

    // M (command 3): SetUIState(0x18, toggle, 0).
    #[test]
    fn m_toggles_the_message_log_state() {
        let w = world(AMAZON, 1, true);
        let mut u = ui(Some(areas()), true);
        u.key(&w, Action::ToggleMessageLog);
        assert!(u.ui.is_open(0x18));
        u.key(&w, Action::ToggleMessageLog);
        assert!(!u.ui.is_open(0x18));
    }

    // S (command 13): SetUIState(3, toggle, 0), then the cursor reset.
    #[test]
    fn s_toggles_the_right_skill_pick() {
        let w = world(AMAZON, 1, true);
        let mut u = ui(Some(areas()), true);
        u.ui.shared.borrow_mut().hud.select_left = true;
        u.key(&w, Action::ToggleSkillMenuRight);
        assert!(u.ui.is_open(3));
        assert!(!u.ui.shared.borrow().hud.select_left);
        u.key(&w, Action::ToggleSkillMenuRight);
        assert!(!u.ui.is_open(3));
    }
}

/// The centre of the inventory close rectangle.
fn inv_close_point(u: &Ui) -> Point {
    let r = crate::ui::panels::inventory::close_rect(&u.ui.shared.borrow().tables, &Screen::R800)
        .expect("the close row");
    Point::new(r.x + r.w as i32 / 2, r.y + r.h as i32 / 2)
}

/// S→C 0x77 `code` (0x10 stash, 0x15 cube) delivered and mirrored.
fn open_by_0x77(u: &mut Ui, w: &ClientWorld, code: u8) {
    u.ui.apply_output(
        &crate::bridge::output::Output::TradeAction {
            code,
            dead_or_absent: false,
        },
        w,
    )
    .unwrap();
    let e = UiEvent::Press {
        button: PointerButton::Right,
        at: Point::new(0, 0),
    };
    u.ui.after_event(&mut u.root, e, Routed::Unhandled).unwrap();
    u.root.take_intents();
}

// The inventory close button with the stash up is the stash's: press sets the
// inventory close flag (sound 4), the release closes ui 0x19 alone and its hook
// sends one 0x4F 0x12; ui 1 is never touched and the open mode returns to 0.
#[test]
fn the_inventory_close_button_closes_the_stash() {
    let mut u = ui(Some(areas()), true);
    let w = world(AMAZON, 1, true);
    open_by_0x77(&mut u, &w, 0x10);
    assert!(u.ui.is_open(0x19) && !u.ui.is_open(UI_INVENTORY));
    let p = inv_close_point(&u);
    u.ui.take_outcome();
    u.click(&w, p);
    assert!(!u.ui.is_open(0x19), "the stash closed");
    assert!(!u.ui.is_open(UI_INVENTORY));
    assert_eq!(u.ui.open_mode().get(), 0);
    assert_eq!(
        u.root.take_intents(),
        vec![ClientIntent(vec![0x4F, 0x12, 0, 0, 0, 0, 0])]
    );
    assert!(u
        .ui
        .take_outcome()
        .sounds
        .contains(&crate::audio::driver::SoundRequest::Ui(4)));
}

// The same for the cube: one 0x4F 0x17 through the close hook.
#[test]
fn the_inventory_close_button_closes_the_cube() {
    let mut u = ui(Some(areas()), true);
    let w = world(AMAZON, 1, true);
    open_by_0x77(&mut u, &w, 0x15);
    assert!(u.ui.is_open(0x1A));
    let p = inv_close_point(&u);
    u.click(&w, p);
    assert!(!u.ui.is_open(0x1A), "the cube closed");
    assert!(!u.ui.is_open(UI_INVENTORY));
    let sent = u.root.take_intents();
    assert_eq!(
        sent.iter().filter(|i| i.0[..2] == [0x4F, 0x17]).count(),
        1,
        "{sent:?}"
    );
}

/// The waypoint menu on the play path.
mod waypoint_play {
    use super::*;

    /// Client quest flags (S→C 0x29) with bit 0 of each record set.
    fn quest(records: &[usize]) -> Output {
        let mut record = [0u8; 96];
        for &q in records {
            record[2 * q] |= 1;
        }
        Output::QuestFlags { record }
    }

    fn open(u: &mut Ui, tab: u8) {
        u.ui.shared.borrow_mut().waypoint_open = Some(WaypointOpen {
            guid: 0x0A,
            record: Default::default(),
            current: 1,
            seq: 1,
            tab,
        });
        u.ui.set_ui(0x14, 0, false).unwrap();
        u.ui.sync_root(&mut u.root);
    }

    fn tabs(u: &Ui, w: &ClientWorld) -> Vec<u32> {
        panel_images(&u.images(w), "menu\\expwaygatetabs")
            .into_iter()
            .map(|(f, ..)| f)
            .collect()
    }

    // The open's tab is the menu's current tab:
    // frame 2t; the drawn tabs test records 7 / 15 / 23 / 26, so
    // with only record 7 tab 1 draws its frame 3 and tabs 3, 4 none.
    #[test]
    fn the_menu_opens_on_the_open_tab_and_draws_the_gated_tabs() {
        let w = world(AMAZON, 1, true);
        let mut u = ui(Some(areas()), true);
        u.ui.apply_output(&quest(&[7, 15]), &w).unwrap();
        open(&mut u, 2);
        let t = tabs(&u, &w);
        assert!(t.contains(&4), "{t:?}");
        assert!(t.contains(&1) && t.contains(&3), "{t:?}");
        assert!(!t.contains(&7) && !t.contains(&9), "{t:?}");
    }

    // A tab click past the quest gate walks down to the last reachable
    // tab: with record 7 only, tab 3 opens as tab 1.
    #[test]
    fn a_gated_tab_click_walks_down_the_quest_records() {
        let w = world(AMAZON, 1, true);
        let mut u = ui(Some(areas()), true);
        u.ui.apply_output(&quest(&[7]), &w).unwrap();
        open(&mut u, 0);
        u.send(
            &w,
            UiEvent::Press {
                button: PointerButton::Left,
                at: Point::new(300, 80),
            },
        );
        let t = tabs(&u, &w);
        assert!(t.contains(&2), "{t:?}");
        assert!(!t.contains(&6), "{t:?}");
    }

    // A press outside the left half closes the menu with one latched
    // C→S 0x49 level 0; the press reaches the menu, not the
    // world.
    #[test]
    fn a_press_outside_the_left_half_closes_with_one_0x49() {
        let mut w = world(AMAZON, 1, true);
        let p = w.local_player.unwrap();
        w.units.get_mut(&p).unwrap().position = Some((100, 100));
        let mut u = ui(Some(areas()), true);
        open(&mut u, 0);
        let r = u.send(
            &w,
            UiEvent::Press {
                button: PointerButton::Left,
                at: Point::new(600, 200),
            },
        );
        assert_eq!(r, Routed::Panel(PanelId(0x14)));
        let want =
            ClientIntent::from_message(&d2_proto::client::TakeOrCloseWp { wp: 0x0A, level: 0 });
        assert_eq!(u.root.take_intents(), vec![want]);
        assert!(!u.ui.is_open(0x14));
    }

    // The close button's hover draws the filled rectangle (colour 0,
    // mode 2) around the "Cancel" tip: at 800 × 600 the area is
    // (353, 447) 36 × 34.
    #[test]
    fn the_close_hover_draws_the_filled_rectangle() {
        let w = world(AMAZON, 1, true);
        let mut u = ui(Some(areas()), true);
        open(&mut u, 0);
        let rects = |u: &Ui| -> Vec<(i32, i32, u8, u8)> {
            let ctx = UiCtx {
                tick: 0,
                world: &w,
                strings: &NoStrings,
            };
            let mut out: Vec<UiDraw> = Vec::new();
            u.root.draw(&ctx, &mut out);
            out.iter()
                .filter_map(|d| match d {
                    UiDraw::Rect(r) if r.y0 == 430 => Some((r.x0, r.y1, r.color, r.mode)),
                    _ => None,
                })
                .collect()
        };
        assert!(rects(&u).is_empty());
        u.send(&w, UiEvent::CursorMoved(Point::new(370, 460)));
        // No font bound: the tip's half width is 0.
        assert_eq!(rects(&u), vec![(367, 447, 0, 2)]);
    }
}

/// The control panel's small items on the play path.
mod hud_small {
    use super::*;
    use crate::controls::{Action as Act, Bindings, Key};
    use crate::ui::StringLookup;

    /// The mini panel open at 800 × 600, single player: layout 2 buttons
    /// at x 326 + 21 i, y H − 50; function f = i, + 1 from i = 3.
    fn mini_open(u: &mut Ui) {
        u.ui.set_ui(0x15, 0, false).unwrap();
        u.ui.sync_root(&mut u.root);
        // The hit test reads the layout of the last draw.
        u.ui.shared.borrow_mut().hud.mini.last_layout =
            Some(crate::ui::panels::control::minipanel::Layout::Two);
    }

    /// The press point of mini-panel button i (inside x_i < x < x_i + 20,
    /// H − 69 < y < H − 47).
    fn button(i: i32) -> Point {
        Point::new(326 + 21 * i + 10, 540)
    }

    struct Strs(Vec<(u16, Vec<u16>)>);
    impl Strs {
        fn new(v: &[(u16, &str)]) -> Self {
            Strs(
                v.iter()
                    .map(|(k, t)| (*k, t.encode_utf16().collect()))
                    .collect(),
            )
        }
    }
    impl StringLookup for Strs {
        fn get(&self, _: &str) -> Option<&[u16]> {
            None
        }
        fn get_id(&self, id: u16) -> Option<&[u16]> {
            self.0
                .iter()
                .find(|(k, _)| *k == id)
                .map(|(_, t)| t.as_slice())
        }
    }

    fn texts(u: &Ui, w: &ClientWorld, strings: &dyn StringLookup) -> Vec<String> {
        let ctx = UiCtx {
            tick: 0,
            world: w,
            strings,
        };
        let mut out: Vec<UiDraw> = Vec::new();
        u.root.draw(&ctx, &mut out);
        out.iter()
            .filter_map(|d| match d {
                UiDraw::Text(t) => Some(String::from_utf16_lossy(&t.text)),
                _ => None,
            })
            .collect()
    }

    // The Quest Log button (f 6, i 5) runs: the quest log
    // opens and asks for the quest data, as the Q key does.
    #[test]
    fn the_quest_log_button_opens_the_quest_log() {
        let w = world(AMAZON, 1, true);
        let mut u = ui(Some(areas()), true);
        mini_open(&mut u);
        u.click(&w, button(5));
        assert!(u.ui.is_open(0x0F));
        assert_eq!(
            u.root.take_intents(),
            vec![quest_log_ui::request_quest_data()]
        );
    }

    // The Automap button (f 4, i 3) toggles UI state 0x0A, the state the
    // shown automap follows.
    #[test]
    fn the_automap_button_toggles_state_0x0a() {
        let w = world(AMAZON, 1, true);
        let mut u = ui(Some(areas()), true);
        mini_open(&mut u);
        u.click(&w, button(3));
        assert!(u.ui.is_open(0x0A));
    }

    // Hovering a button shows its string and ` (%s)` with its binding's
    // key; the run button's tip adds the Run key.
    #[test]
    fn the_mini_panel_and_run_tips_name_their_keys() {
        let w = world(AMAZON, 1, true);
        let mut u = ui(Some(areas()), true);
        let mut b = Bindings::empty();
        b.set(Act::ToggleQuests, &[Key::Q]);
        b.set(Act::ToggleRun, &[Key::R]);
        u.ui.set_belt_keys(&b);
        mini_open(&mut u);
        let s = Strs::new(&[(4175, "Quest Log"), (4178, " (%s)"), (4179, "Run")]);
        u.send(&w, UiEvent::CursorMoved(button(5)));
        let want = format!("Quest Log ({})", Key::Q.name());
        assert!(texts(&u, &w, &s).contains(&want), "{:?}", texts(&u, &w, &s));
        // The run rectangle x W/2 − 145…W/2 − 128, y H − 28…H − 8.
        u.send(&w, UiEvent::CursorMoved(Point::new(262, 580)));
        let want = format!("Run ({})", Key::R.name());
        assert!(texts(&u, &w, &s).contains(&want), "{:?}", texts(&u, &w, &s));
    }

    // A belt with extra rows and more than one row blocks the right side:
    // layout 1, the art at (W/2 − 205, H − 47); the buttons move to x0 = W/2 −
    // 202 (offset −118).
    #[test]
    fn a_belt_with_extra_rows_moves_the_mini_panel_left() {
        use crate::bridge::items::mode;
        use crate::ui::panels::inv_items::tests::world as item_world;
        // A worn belt of `belts` type 0 (3 rows) whose rows are shown by the
        // belt key (state 0x1F open: the mini
        // panel moves to x 195). With the rows folded again the panel is back at
        // layout 2.
        let mut w = item_world(&[(7, mode::BODY, (8, 0, 0, 0), b"lbl ")], None);
        let me = w.local_player.unwrap();
        w.units.get_mut(&me).unwrap().mode = 1;
        let mut u = ui(Some(areas()), true);
        u.ui.set_belt_parts(hud_belt::BeltParts {
            records: Vec::new(),
            types: BTreeMap::from([(*b"lbl ", 0)]),
            beltable: Default::default(),
        });
        u.ui.shared.borrow_mut().states.force(0x1F, true);
        mini_open(&mut u);
        let art: Vec<(i32, i32)> = panel_images(&u.images(&w), "panel\\minipanel_s")
            .into_iter()
            .map(|(_, x, y)| (x, y))
            .collect();
        assert_eq!(art, vec![(195, 553)]);
        // The next frame draws the rows (the state 0x1F draw sets the pop-up
        // flag after the belt draw), which sets the second pop-up flag.
        // (The test's belt has no `belts.bin` records, so its rows are not
        // drawn: set the flag the draw would.)
        let _ = u.images(&w);
        u.ui.shared.borrow_mut().hud.belt.state.extra_boxes = true;
        // Button 5 (Quest Log) at x0 = 198: x 303 … 323.
        u.click(&w, Point::new(198 + 105 + 10, 540));
        assert!(u.ui.is_open(0x0F));
    }

    // A state with flag bit 24 (`stambarblue`) draws the stamina bar blue (fill
    // frame 2).
    #[test]
    fn a_stambarblue_state_draws_the_stamina_bar_blue() {
        let mut w = world(AMAZON, 1, true);
        let key = w.local_player.unwrap();
        let unit = w.units.get_mut(&key).unwrap();
        unit.stats.insert(10, 40 << 8);
        unit.stats.insert(11, 80 << 8);
        let mut u = ui(Some(areas()), true);
        let frames = |u: &Ui, w: &ClientWorld| -> Vec<u32> {
            panel_images(&u.images(w), hud::FILL_FILE)
                .into_iter()
                .map(|(f, ..)| f)
                .collect()
        };
        assert!(!frames(&u, &w).contains(&2));
        u.ui.set_hud_tables(hud::HudTables {
            stambarblue: vec![0x3A],
            ..Default::default()
        });
        w.units.get_mut(&key).unwrap().states.insert(0x3A);
        assert!(frames(&u, &w).contains(&2), "{:?}", frames(&u, &w));
    }

    // The play draws rows 249–270: the globes' row
    // window is `CelDrawEx`, the skill icons `CelDrawColor`; the stamina bar
    // the rectangle (colour = the palette's nearest gold, mode 2); the skill
    // icons before the new-stats / new-skills buttons; the mini panel open from
    // the start.
    #[test]
    fn the_control_panel_draws_in_the_recorded_order_and_calls() {
        use crate::ui::draw::CelCall;
        let mut w = world(AMAZON, 1, true);
        let key = w.local_player.unwrap();
        let unit = w.units.get_mut(&key).unwrap();
        unit.stats.insert(10, 80 << 8);
        unit.stats.insert(11, 80 << 8);
        for (stat, v) in [(6, 50), (7, 50), (8, 20), (9, 20)] {
            unit.stats.insert(stat, v << 8);
        }
        let mut u = ui(Some(areas()), true);
        assert!(u.ui.is_open(0x15), "the mini panel is open from the start");
        let mut colors = [d2_formats::palette::Rgb::default(); 256];
        colors[109] = d2_formats::palette::Rgb {
            r: 244,
            g: 192,
            b: 76,
        };
        u.ui.set_palette(&d2_formats::palette::Palette { colors });
        let ctx = UiCtx {
            tick: 0,
            world: &w,
            strings: &NoStrings,
        };
        let mut out: Vec<UiDraw> = Vec::new();
        u.root.draw(&ctx, &mut out);
        let files = u.ui.files();
        let named: Vec<(String, CelCall)> = out
            .iter()
            .filter_map(|d| match d {
                UiDraw::Image(i) => Some((files.name(i.image.file)?.to_string(), i.call)),
                _ => None,
            })
            .collect();
        let call_of = |n: &str| named.iter().find(|(f, _)| f == n).map(|(_, c)| *c);
        assert_eq!(call_of("panel\\hlthmana"), Some(CelCall::Ex));
        assert_eq!(call_of("panel\\overlap"), Some(CelCall::Draw));
        let stamina: Vec<(i32, i32, u8, u8)> = out
            .iter()
            .filter_map(|d| match d {
                UiDraw::Rect(r) if r.mode == 2 => Some((r.x0, r.y0, r.color, r.mode)),
                _ => None,
            })
            .collect();
        assert_eq!(stamina, vec![(273, 573, 109, 2)]);
        assert!(
            !named.iter().any(|(f, _)| f == hud::FILL_FILE),
            "no fill cel"
        );
        let pos = |n: &str| named.iter().position(|(f, _)| f == n);
        let (mini, menu) = (pos("panel\\minipanel_s"), pos("panel\\menubutton"));
        assert!(mini.is_some() && menu < mini);
        if let (Some(icon), Some(level)) = (
            named.iter().position(|(f, _)| f.contains("skillicon")),
            pos("panel\\level"),
        ) {
            assert!(icon < level, "the skill icons before the level buttons");
            assert_eq!(named[icon].1, CelCall::Color);
        }
    }

    // The life text toggle is stored at once and read at start.
    #[test]
    fn the_globe_text_toggles_are_stored_and_read_back() {
        let w = world(AMAZON, 1, true);
        let mut u = ui(Some(areas()), true);
        u.click(&w, Point::new(50, 560));
        let s = u.ui.take_settings_change().expect("stored at once");
        assert_eq!((s.show_hp_text, s.show_mp_text), (1, 0));
        let mut v = ui(Some(areas()), true);
        v.ui.set_settings(s);
        assert!(v.ui.shared.borrow().hud.input.show_hp);
        assert!(!v.ui.shared.borrow().hud.input.show_mp);
    }
}

/// The skill tree's band, number and tool tips on the play path.
mod skill_tree_play {
    use super::*;
    use crate::ui::StringLookup;

    struct Strs(Vec<(u16, Vec<u16>)>);
    impl StringLookup for Strs {
        fn get(&self, _: &str) -> Option<&[u16]> {
            None
        }
        fn get_id(&self, id: u16) -> Option<&[u16]> {
            self.0
                .iter()
                .find(|(k, _)| *k == id)
                .map(|(_, t)| t.as_slice())
        }
    }

    fn strs() -> Strs {
        Strs(
            [(4144, "Close"), (4224, "Tree A"), (4225, "Tree B")]
                .into_iter()
                .map(|(k, t)| (k, t.encode_utf16().collect()))
                .collect(),
        )
    }

    fn draws(u: &Ui, w: &ClientWorld) -> Vec<UiDraw> {
        let s = strs();
        let ctx = UiCtx {
            tick: 0,
            world: w,
            strings: &s,
        };
        let mut out: Vec<UiDraw> = Vec::new();
        u.root.draw(&ctx, &mut out);
        out
    }

    fn texts(d: &[UiDraw]) -> Vec<(String, i32, i32)> {
        d.iter()
            .filter_map(|d| match d {
                UiDraw::Text(t) => Some((String::from_utf16_lossy(&t.text), t.at.x, t.at.y)),
                _ => None,
            })
            .collect()
    }

    fn tab_frames(u: &Ui, w: &ClientWorld) -> Vec<u32> {
        panel_images(&u.images(w), "spells\\skltree_a_back")[4..]
            .iter()
            .map(|a| a.0)
            .collect()
    }

    // The tab-1 band reaches y = H − 49: at 800 × 600 a press at
    // (700, 530), below the inventory area's bottom (501), selects tab 1.
    #[test]
    fn the_tab_1_band_reaches_above_the_control_panel() {
        let mut u = ui(Some(areas()), true);
        let w = world(AMAZON, 1, true);
        u.key(&w, Action::ToggleSkillTree);
        let _ = u.images(&w);
        u.click(&w, Point::new(700, 200));
        assert_eq!(tab_frames(&u, &w), vec![12, 13, 14, 15]);
        let r = u.click(&w, Point::new(700, 530));
        assert_eq!(r.0, Routed::Panel(PanelId(4)));
        assert_eq!(tab_frames(&u, &w), vec![4, 5, 6, 7]);
        // Below y = H − 48 the press is the control panel's, not the
        // tree's.
        let r = u.click(&w, Point::new(700, 560));
        assert_ne!(r.0, Routed::Panel(PanelId(4)));
    }

    // With free skill points the number is drawn at (W − sx − 52, H + sy −
    // 400) = (668, 140), colour 0.
    #[test]
    fn the_free_points_number_is_drawn() {
        let mut u = ui(Some(areas()), true);
        let mut w = world(AMAZON, 1, true);
        u.key(&w, Action::ToggleSkillTree);
        assert!(!texts(&draws(&u, &w)).iter().any(|t| t.0 == "3"));
        let key = w.local_player.unwrap();
        w.units.get_mut(&key).unwrap().stats.insert(5, 3);
        assert!(texts(&draws(&u, &w)).contains(&("3".into(), 668, 140)));
    }

    // Tab 1 current, the mouse in tab 2's band: the black mode-6
    // rectangle (W − sx − 89, T + 5)–(W − sx + 1, T + 25) and string 4225
    // at (W − sx − 89, T + 20), T = H + sy − 264 = 276. In the
    // close rectangle (amazon tab 1: X 571, Y 477) `strClose` is queued at
    // (X + 15, H + sy − 98).
    #[test]
    fn the_tab_and_close_tool_tips() {
        let mut u = ui(Some(areas()), true);
        let w = world(AMAZON, 1, true);
        u.key(&w, Action::ToggleSkillTree);
        u.send(&w, UiEvent::CursorMoved(Point::new(700, 300)));
        let d = draws(&u, &w);
        let rects: Vec<_> = d
            .iter()
            .filter_map(|d| match d {
                UiDraw::Rect(r) => Some((r.x0, r.y0, r.x1, r.y1, r.color, r.mode)),
                _ => None,
            })
            .collect();
        assert_eq!(rects, vec![(631, 281, 721, 301, 0, 6)]);
        assert!(texts(&d).contains(&("Tree B".into(), 631, 296)));
        u.send(&w, UiEvent::CursorMoved(Point::new(580, 460)));
        assert!(texts(&draws(&u, &w)).contains(&("Close".into(), 586, 442)));
    }

    // The hovered tree icon gets its description box: the pop-up at the
    // icon's left − 24 and 5 rows under its bottom row, the text built by
    // (no free points: no point can be spent) from the
    // `skills` / `skilldesc` rows and the learned level, in colour 1
    // (playtest B028).
    #[test]
    fn a_hovered_tree_icon_draws_the_skill_description() {
        use crate::bridge::skills::{SkillEntry as Entry, SkillList, NATIVE};
        use d2_data::tables::{Missiles, Record, Skilldesc, Skills};
        fn blank<T: Record>() -> T {
            T::decode(&vec![0u8; T::SIZE])
        }
        let mut w = world(AMAZON, 1, true);
        let key = w.local_player.unwrap();
        {
            let u = w.units.get_mut(&key).unwrap();
            u.stats.insert(12, 5);
            u.skills = Some(SkillList {
                entries: vec![Entry {
                    skill: 6,
                    base: 3,
                    owner: NATIVE,
                    ..Default::default()
                }],
                ..Default::default()
            });
        }
        let mut u = ui(Some(areas()), true);
        u.ui.set_skill_tree_tables(tree_tables());
        // Skill 6: an Amazon skill, `reqlevel` 1, `skilldesc` row 0 with
        // no lines.
        let mut row: Skills = blank();
        row.charclass = 0xFF;
        let mut skills = vec![row.clone(); 7];
        skills[6].charclass = 0;
        skills[6].reqlevel = 1;
        let mut desc: Skilldesc = blank();
        desc.str_name = 100;
        desc.str_long = 101;
        let mut chars = crate::ui::char_feed::CharTables::default();
        chars.desc.skills = Some(std::sync::Arc::new(d2_sim::skills::SkillTables {
            skills,
            skilldesc: vec![desc],
            missiles: vec![blank::<Missiles>()],
            skills_code: Vec::new(),
            miss_code: Vec::new(),
            level_cap: 99,
            stat_count: 359,
        }));
        u.ui.set_char_tables(chars);
        let mut f = FontMeasure::default();
        f.insert(
            1,
            FontTable::parse(&character_bind_tests::tbl(6)).expect("tbl"),
        );
        u.ui.set_fonts(f);
        u.key(&w, Action::ToggleSkillTree);
        let s = Strs(
            [
                (100, "Magic Arrow"),
                (101, "Fires an arrow"),
                (3994, "ÿc"),
                (3998, "\n"),
                (4187, "Required Level : "),
            ]
            .into_iter()
            .map(|(k, t)| (k, t.encode_utf16().collect()))
            .collect(),
        );
        let drawn = |u: &Ui| {
            let ctx = UiCtx {
                tick: 0,
                world: &w,
                strings: &s,
            };
            let mut out: Vec<UiDraw> = Vec::new();
            u.root.draw(&ctx, &mut out);
            out
        };
        let tip = |d: &[UiDraw]| {
            d.iter().find_map(|d| match d {
                UiDraw::Text(t) if String::from_utf16_lossy(&t.text).contains("Magic") => {
                    Some((String::from_utf16_lossy(&t.text), t.style.color))
                }
                _ => None,
            })
        };
        // Off the icon: no box.
        u.send(&w, UiEvent::CursorMoved(Point::new(300, 300)));
        assert_eq!(tip(&drawn(&u)), None);
        // On skill 6's icon (cel point (415, 122)).
        u.send(&w, UiEvent::CursorMoved(Point::new(430, 100)));
        let d = drawn(&u);
        // Bottom line first: the requirement (reqlevel 1 + L 3), the long
        // text, the green name on top.
        assert_eq!(
            tip(&d),
            Some((
                "Required Level : 4\nFires an arrow\nÿc2Magic Arrow".to_string(),
                1
            ))
        );
        // The box: left 24 under the icon's left edge, top 5 under its row.
        let boxes: Vec<_> = d
            .iter()
            .filter_map(|d| match d {
                UiDraw::Rect(r) if r.mode == 2 => Some((r.x0, r.y0)),
                _ => None,
            })
            .collect();
        assert_eq!(boxes, vec![(415 - 24, 122 + 5)]);
        // The pop-up is the last UI step: after every panel draw.
        let last_text = d
            .iter()
            .rposition(|d| matches!(d, UiDraw::Text(_)))
            .unwrap();
        assert!(matches!(&d[last_text], UiDraw::Text(t)
            if String::from_utf16_lossy(&t.text).contains("Magic")));
    }

    // The character panel's close button tip: `strClose` at (sx + 143,
    // H + sy − 95) = (223, 445) while the mouse is in [sx + 128, sx +
    // 160] × [H + sy − 92, H + sy − 60].
    #[test]
    fn the_character_close_tip() {
        let mut u = ui(Some(areas()), true);
        let w = world(AMAZON, 1, true);
        u.key(&w, Action::ToggleCharacter);
        u.send(&w, UiEvent::CursorMoved(Point::new(150, 300)));
        assert!(!texts(&draws(&u, &w)).iter().any(|t| t.0 == "Close"));
        u.send(&w, UiEvent::CursorMoved(Point::new(220, 460)));
        assert!(texts(&draws(&u, &w)).contains(&("Close".into(), 223, 445)));
    }

    // The stash close button tip: in [X, X + 40] × [Y − 35, Y + 5] (X =
    // sx + 272, Y = H + sy − 64 in an expansion game) `strClose` at (X +
    // 12 − w / 2, Y − 35); no font bound: w = 0, (364, 441).
    #[test]
    fn the_stash_close_tip() {
        let mut u = ui(Some(areas()), true);
        let w = world(AMAZON, 1, true);
        u.ui.set_ui(0x19, 0, false).unwrap();
        u.ui.sync_root(&mut u.root);
        u.send(&w, UiEvent::CursorMoved(Point::new(360, 460)));
        assert!(texts(&draws(&u, &w)).contains(&("Close".into(), 364, 441)));
    }
}

/// The inventory's empty equipment-slot pictures on the play path.
mod equip_backgrounds_play {
    use super::*;
    use crate::bridge::items::mode;
    use crate::ui::panels::inv_items::tests::world as item_world;
    use crate::ui::panels::inventory::{BinRect, EquipRects};

    fn rects() -> EquipRects {
        EquipRects {
            torso: BinRect::new(631, 688, 152, 238),
            r_arm: BinRect::new(518, 574, 132, 245),
            l_arm: BinRect::new(744, 800, 132, 245),
            ..EquipRects::default()
        }
    }

    fn pictures(u: &Ui, w: &ClientWorld, file: &str) -> Vec<(i32, i32)> {
        panel_images(&u.images(w), file)
            .into_iter()
            .map(|(_, x, y)| (x, y))
            .collect()
    }

    // With nothing worn the torso picture is drawn at (left + 2, bottom −
    // 2) and both hands' at (left, bottom − 2); a worn armour hides its
    // slot's; a two-handed weapon in the right hand hides both hands'.
    #[test]
    fn empty_slots_get_their_pictures() {
        let mut u = ui(Some(areas()), true);
        let r = inventory_record(AMAZON, &Screen::R800).unwrap();
        let mut all = vec![EquipRects::default(); r + 1];
        all[r] = rects();
        u.ui.set_equip_rects(all);
        u.ui.set_inv_tables(std::sync::Arc::new(
            crate::ui::panels::inv_items::equip::tests::tables(),
        ));
        let mut w = item_world(&[], None);
        let me = w.local_player.unwrap();
        w.units.get_mut(&me).unwrap().mode = 1;
        u.key(&w, Action::ToggleInventory);
        assert_eq!(pictures(&u, &w, "panel\\inv_armor"), vec![(633, 236)]);
        assert_eq!(
            pictures(&u, &w, "panel\\inv_weapons"),
            vec![(744, 243), (518, 243)]
        );
        let mut w = item_world(
            &[
                (5, mode::BODY, (3, 0, 0, 0), b"qui "),
                (6, mode::BODY, (4, 0, 0, 0), b"2hs "),
            ],
            None,
        );
        let me = w.local_player.unwrap();
        w.units.get_mut(&me).unwrap().mode = 1;
        assert!(pictures(&u, &w, "panel\\inv_armor").is_empty());
        assert!(pictures(&u, &w, "panel\\inv_weapons").is_empty());
    }
}

// Key mode: 1 at game start, 0 with the chat open (key-up kept), 2 with the
// stash open, back to 1 on close; ui 5 closing while the stash is open leaves
// the mode alone (key mode 2 with the
// stash open and 1 after Esc closes it).
#[test]
fn the_ui_hooks_move_the_key_mode() {
    let mut u = ui(Some(areas()), true);
    assert_eq!(u.ui.key_mode(), 1);
    if u.ui.set_ui(5, 0, false).expect("chat on") {
        assert_eq!(u.ui.key_mode(), 0);
        u.ui.set_ui(5, 1, false).expect("chat off");
        assert_eq!(u.ui.key_mode(), 1);
    }
    if u.ui.set_ui(25, 0, false).expect("stash on") && u.ui.is_open(25) {
        assert_eq!(u.ui.key_mode(), 2);
        u.ui.set_ui(25, 1, false).expect("stash off");
        assert_eq!(u.ui.key_mode(), 1);
    }
}

// D2RUST's own: the Esc menu's pentagrams in live play step on the wall clock
// while the paused server tick stands still; without it (check runs) they
// keep the server tick's 40 ms clock.
#[test]
fn esc_menu_pentagrams_step_on_the_wall_clock_while_paused() {
    let mut u = ui(Some(areas()), true);
    let w = world(AMAZON, 1, true);
    u.key(&w, Action::GameMenu);
    assert!(u.ui.is_open(9));
    // The right pentagram's frame is the counter f.
    let right_frame = |u: &Ui| -> u32 {
        let ctx = UiCtx {
            tick: 0,
            world: &w,
            strings: &NoStrings,
        };
        let mut out: Vec<UiDraw> = Vec::new();
        u.root.draw(&ctx, &mut out);
        let frames: Vec<u32> = out
            .iter()
            .filter_map(|d| match d {
                UiDraw::Image(i) if u.ui.files().name(i.image.file) == Some("cursor\\pentspin") => {
                    Some(i.image.frame)
                }
                _ => None,
            })
            .collect();
        assert_eq!(frames.len(), 2, "two pentagrams");
        frames[1]
    };
    // The server tick never moves (the pause); only the wall clock does.
    u.ui.set_menu_wall_ms(Some(1_000));
    let f0 = right_frame(&u);
    let mut seen = vec![f0];
    for ms in [1_051, 1_102, 1_153] {
        u.ui.set_menu_wall_ms(Some(ms));
        seen.push(right_frame(&u));
    }
    // One step per draw after more than 50 ms: f, f+1, f+2, f+3.
    let want: Vec<u32> = (0..4).map(|k| (f0 + k) % 8).collect();
    assert_eq!(seen, want);
    // Within 50 ms of the last step: no step.
    u.ui.set_menu_wall_ms(Some(1_190));
    assert_eq!(right_frame(&u), (f0 + 4) % 8);
    assert_eq!(right_frame(&u), (f0 + 4) % 8);
    // Check runs (no wall clock): the server tick's clock, which stands at
    // tick 0 behind the last stamp, so no step.
    u.ui.set_menu_wall_ms(None);
    assert_eq!(right_frame(&u), (f0 + 4) % 8);
    assert_eq!(right_frame(&u), (f0 + 4) % 8);
}

/// The side panels in the 16:9 frame: the 800 × 600 layout centred 134
/// columns in ([`Screen::WIDE`]), with the left panels opened at the
/// frame's first column and the right panels against its last, 214
/// columns out from their 800 × 600 places. Points here are in the UI
/// box's space (frame column − 134).
mod wide_panels {
    use super::*;
    use crate::bridge::items::{mode, ItemArtRow};
    use crate::ui::panels::inv_items::tests::world as item_world;
    use crate::ui::states::id;

    /// How far each side moves out in the 16:9 frame.
    const OUT: i32 = 214;

    type Fixture<'a> = (u32, u8, (u8, u16, u16, u8), &'a [u8; 4]);

    fn panels_ui(screen: Screen, open: &[u8]) -> Ui {
        let mut u = ui_at(screen, Some(areas()), true);
        let mut art = ItemArtRows::default();
        art.0.insert(
            *b"gem2",
            ItemArtRow {
                inv_w: 2,
                inv_h: 2,
                inv_file: "invgem2".into(),
                flippy_file: String::new(),
                beltable: false,
            },
        );
        u.ui.set_item_art(art);
        u.ui.set_item_frame_sizes(BTreeMap::from([("invgem2".to_string(), (56, 56))]));
        for &s in open {
            u.ui.set_ui(u32::from(s), 0, false).unwrap();
        }
        u.ui.sync_root(&mut u.root);
        u
    }

    /// An expansion game whose local player stands (mode 1) with `items`.
    fn items_world(items: &[Fixture], cursor: Option<u32>) -> ClientWorld {
        let mut w = item_world(items, cursor);
        let p = w.local_player.unwrap();
        w.units.get_mut(&p).unwrap().mode = 1;
        w.expansion = 1;
        w
    }

    /// The intents of a move then a click at `at`.
    fn click_intents(u: &mut Ui, w: &ClientWorld, at: Point) -> Vec<ClientIntent> {
        u.send(w, UiEvent::CursorMoved(at));
        u.click(w, at);
        u.root.take_intents()
    }

    /// Both side panels open at once (character on the left, inventory
    /// on the right): their art and each side's stone border are drawn
    /// 214 columns out, so in frame columns the panels cover 0..320 and
    /// 748..1068; the control panel stays centred.
    #[test]
    fn both_panels_draw_at_the_frame_edges() {
        let w = world(AMAZON, 1, true);
        let at = |screen| {
            let mut u = ui_at(screen, Some(areas()), true);
            u.key(&w, Action::ToggleCharacter);
            u.key(&w, Action::ToggleInventory);
            assert_eq!(u.ui.open_mode().get(), 3);
            u.images(&w)
        };
        let (base, wide) = (at(Screen::R800), at(Screen::WIDE));
        let art = |v: &[(String, u32, i32, i32)]| panel_images(v, "panel\\invchar6");
        let (a, b) = (art(&base), art(&wide));
        assert_eq!(a.len(), 8);
        assert_eq!(a.len(), b.len());
        for ((f, x0, y0), (g, x1, y1)) in a.iter().zip(&b) {
            assert_eq!((f, y0), (g, y1));
            // Frames 0–3 are the character panel, 4–7 the inventory.
            let dx = if *f < 4 { -OUT } else { OUT };
            assert_eq!(*x1, x0 + dx, "frame {f}");
        }
        // The character art starts at the frame's column 0 (box −134), the
        // inventory art at column 748 (box 614).
        assert_eq!(b.iter().map(|i| i.1).min(), Some(-134));
        assert!(b.iter().any(|&(f, x, _)| f == 4 && x == 614));
        let border = |v: &[(String, u32, i32, i32)]| panel_images(v, "panel\\800borderframe");
        let (a, b) = (border(&base), border(&wide));
        assert_eq!(a.len(), 10);
        assert_eq!(a.len(), b.len());
        for ((f, x0, _), (_, x1, _)) in a.iter().zip(&b) {
            let dx = if *f < 5 { -OUT } else { OUT };
            assert_eq!(*x1, x0 + dx, "border frame {f}");
        }
        let ctrl = |v: &[(String, u32, i32, i32)]| panel_images(v, "panel\\800ctrlpnl7");
        assert_eq!(ctrl(&base), ctrl(&wide));
    }

    /// Hit tests follow the panels: the character panel takes the frame's
    /// first 320 columns, the inventory its last 320, and the world
    /// between them takes no panel.
    #[test]
    fn hits_follow_the_panels_and_leave_the_middle_to_the_world() {
        let w = world(AMAZON, 1, true);
        let mut u = ui_at(Screen::WIDE, Some(areas()), true);
        u.send(&w, UiEvent::CursorMoved(Point::new(300, 300)));
        u.key(&w, Action::ToggleCharacter);
        u.key(&w, Action::ToggleInventory);
        // Where a left press at frame column `x` (row 300) goes.
        let press = |u: &mut Ui, x: i32| {
            let (button, at) = (PointerButton::Left, Point::new(x - 134, 300));
            let r = u.send(&w, UiEvent::Press { button, at });
            u.send(&w, UiEvent::Release { button, at });
            r
        };
        let character = Routed::Panel(PanelId(u16::from(id::CHARACTER)));
        let inventory = Routed::Panel(PanelId(u16::from(id::INVENTORY)));
        // Frame columns 10 and 300: the character panel.
        assert_eq!(press(&mut u, 10), character);
        assert_eq!(press(&mut u, 300), character);
        // Frame columns 330..740: the world.
        for x in [330, 534, 740] {
            assert_eq!(press(&mut u, x), Routed::Unhandled, "frame column {x}");
        }
        // Frame columns 760 and 1060: the inventory.
        assert_eq!(press(&mut u, 760), inventory);
        assert_eq!(press(&mut u, 1060), inventory);
        assert!(u.ui.is_open(id::CHARACTER) && u.ui.is_open(id::INVENTORY));
    }

    /// Opening a side panel moves the cursor with the view: by W / 4 = 200
    /// at 800 × 600, by half the panel's 320 columns in the 16:9 frame.
    #[test]
    fn the_cursor_jump_follows_the_view_move() {
        let w = world(AMAZON, 1, true);
        for (screen, to) in [(Screen::R800, 300), (Screen::WIDE, 340)] {
            let mut u = ui_at(screen, Some(areas()), true);
            u.send(&w, UiEvent::CursorMoved(Point::new(500, 77)));
            u.key(&w, Action::ToggleInventory);
            assert_eq!(u.ui.take_cursor_warp(), Some(Point::new(to, 77)));
        }
    }

    /// An item moves between the stash (left) and the inventory (right)
    /// the same way in both frames: each click at the 800 × 600 point,
    /// moved with its side, sends the same message. Picked from the stash
    /// and placed in the inventory, then picked from the inventory and
    /// placed in the stash.
    #[test]
    fn items_move_between_the_left_and_right_panels() {
        // Stash cell (0, 0) and inventory cell (2, 0) at 800 × 600.
        let stash = Point::new(154 + 14, 142 + 14);
        let inv = Point::new(419 + 29 * 2 + 14, 315 + 14);
        let same = |p: Point| p;
        let wide = |p: Point| Point::new(p.x + Screen::WIDE.side_dx(p.x), p.y);
        assert_eq!(wide(stash).x, stash.x - OUT);
        assert_eq!(wide(inv).x, inv.x + OUT);
        let in_stash = items_world(&[(9, mode::STORED, (0, 0, 0, 5), b"gem2")], None);
        let in_inv = items_world(&[(9, mode::STORED, (0, 2, 0, 1), b"gem2")], None);
        let on_cursor = items_world(&[(9, mode::CURSOR, (0, 0, 0, 0), b"gem2")], Some(9));
        let run = |screen: Screen, map: &dyn Fn(Point) -> Point| {
            let mut u = panels_ui(screen, &[id::STASH]);
            let pick = click_intents(&mut u, &in_stash, map(stash));
            let put = click_intents(&mut u, &on_cursor, map(inv));
            let pick_back = click_intents(&mut u, &in_inv, map(inv));
            let put_back = click_intents(&mut u, &on_cursor, map(stash));
            (pick, put, pick_back, put_back)
        };
        let base = run(Screen::R800, &same);
        assert!(!base.0.is_empty(), "no pick from the stash");
        assert!(!base.2.is_empty(), "no pick from the inventory");
        assert_eq!(base.1.len(), 1, "one place in the inventory");
        assert_eq!(base.3.len(), 1, "one place in the stash");
        // The place messages (0x18: id, x, y, page) name the page: 0 (inventory), 4 (stash).
        assert_eq!(base.1[0].0.get(13), Some(&0));
        assert_eq!(base.3[0].0.get(13), Some(&4));
        assert_eq!(run(Screen::WIDE, &wide), base);
    }

    /// The `inventory` table's rectangles move with their side: an `inv`
    /// area of the right panel 214 columns right, one of the left side
    /// 214 left; an empty one stays, and nothing moves at 4:3.
    #[test]
    fn table_rectangles_move_with_their_side() {
        let right = InvArea {
            left: 400,
            right: 720,
            top: 60,
            bottom: 501,
        };
        let left = InvArea {
            left: 80,
            right: 400,
            ..right
        };
        assert_eq!(at_edges_area(&Screen::R800, right), right);
        assert_eq!(at_edges_area(&Screen::R800, left), left);
        let r = at_edges_area(&Screen::WIDE, right);
        assert_eq!((r.left, r.right), (614, 934));
        let l = at_edges_area(&Screen::WIDE, left);
        assert_eq!((l.left, l.right), (-134, 186));
        assert_eq!(
            at_edges_area(&Screen::WIDE, InvArea::default()),
            InvArea::default()
        );
    }
}

#[path = "control_ids_tests.rs"]
mod control_ids;

#[path = "item_grid_ids_tests.rs"]
mod item_grid_ids;

#[path = "original_char_skill_ids_tests.rs"]
mod char_skill_ids;

#[test]
fn hovering_a_skill_assigns_custom_keys_and_preserves_the_server_save_path() {
    use crate::bridge::skills::{SkillEntry, SkillList, NATIVE};
    use crate::controls::{Bindings, Context, Key};
    for key in [
        Key::J,
        Key::Digit7,
        Key::Numpad3,
        Key::Mouse4,
        Key::F12,
        Key::RightCtrl,
        Key::NumpadEnter,
        Key::Up,
    ] {
        let mut u = ui(Some(areas()), true);
        let mut w = world(AMAZON, 1, true);
        let player = w.local_player.unwrap();
        w.units.get_mut(&player).unwrap().skills = Some(SkillList {
            entries: vec![SkillEntry {
                skill: 47,
                base: 1,
                owner: NATIVE,
                ..Default::default()
            }],
            ..Default::default()
        });
        let mut tables = hud::HudTables::default();
        tables.icons.insert(47, (1, 1));
        tables.flags.insert(
            47,
            hud::SkillButtonFlags {
                in_game: true,
                ..Default::default()
            },
        );
        u.ui.set_hud_tables(tables);
        u.key(&w, Action::ToggleSkillMenuRight);
        let at = Point::new(690, 490);
        u.send(&w, UiEvent::CursorMoved(at));
        let defaults = crate::controls::Preset::Original.bindings().unwrap();
        assert!(matches!(
            u.ui.bind_skill_key(&w, &defaults, Key::I, at, None),
            SkillKeyResult::Consumed
        ));
        assert!(u.ui.shared.borrow().messages.list().total_lines() > 0);
        assert!(!u.ui.is_open(UI_INVENTORY));
        assert_eq!(u.ui.shared.borrow().hud.hotkeys.slots[0].skill, -1);
        let bindings = Bindings::empty();
        let path = std::env::temp_dir().join(format!(
            "d2rust-skill-key-{}-{}.toml",
            std::process::id(),
            key.name()
        ));
        let result = u.ui.bind_skill_key(&w, &bindings, key, at, Some(&path));
        let SkillKeyResult::Assigned { action, bindings } = result else {
            panic!("{result:?}");
        };
        assert_eq!(bindings.action_for(Context::World, key), Some(action));
        assert_eq!(crate::controls::load(&path).unwrap().1, bindings);
        std::fs::remove_file(path).unwrap();
        u.key(&w, action);
        let intents = u.root.take_intents();
        assert!(
            intents.iter().any(|i| i.0[0] == 0x51),
            "skill binding is saved by the server"
        );
        assert_eq!(u.ui.shared.borrow().hud.hotkeys.slots[0].skill, 47);
        u.key(&w, Action::ToggleSkillMenuRight);
        u.key(&w, action);
        assert!(
            u.root.take_intents().iter().any(|i| i.0[0] == 0x3C),
            "custom key selects the skill"
        );
    }
}

#[test]
fn adaptive_panel_tables_art_and_hits_share_one_translation_after_repeated_resizes() {
    use crate::rules::adaptive_integer::FrameSelection;
    use crate::ui::ScaleMode;
    let base_area = areas()[16];
    let base_layout = crate::ui::panels::inv_items::fallback_layout(&Screen::R800);
    let mut ui = OriginalUi::new(
        UiConfig {
            screen: Screen::R800,
            expansion_installed: true,
        },
        Some(areas()),
    )
    .unwrap();
    ui.set_inv_layouts(vec![base_layout; 32]);
    let rows = crate::ui::layout::panel_layout().unwrap();
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
        let f = FrameSelection::for_window(w, h, FrameSize::WIDE, ScaleMode::Integer).frame;
        let screen = Screen::for_frame(f);
        for target in [screen, Screen::WIDE, screen, Screen::R800, screen] {
            ui.resize_screen(target);
            let sh = ui.shared.borrow();
            let area = sh.inv_areas.as_ref().unwrap()[16];
            assert_eq!(area, at_edges_area(&target, base_area));
            let l = sh.items.layouts.as_ref().unwrap()[16];
            assert_eq!(l, at_edges_layout(&target, base_layout));
            let (x, y, _, _) = l.grid.cell(0, 0);
            assert!(l.grid.contains_mouse(Point::new(x + 1, y + 1)));
            assert_eq!(l.grid.mouse_cell(Point::new(x + 1, y + 1)), (0, 0));
            assert!(area.rect().contains(Point::new(x + 1, y + 1)));
            assert_eq!(
                at_edges_area(&target, InvArea::default()),
                InvArea::default()
            );
            for row in &rows {
                // These panels all position their native-size art and controls from H+sy.
                if !matches!(
                    row.panel,
                    PanelKey::Ui(1 | 2 | 4 | 0x0c | 0x14 | 0x19 | 0x1a)
                ) {
                    continue;
                }
                assert_eq!(
                    row.y.eval(&target) - row.y.eval(&Screen::R800),
                    target.panel_dy(),
                    "{:?} {}",
                    row.panel,
                    row.item
                );
                if let Some(hit) = row.hit_rect(&target) {
                    assert!(hit.contains(Point::new(hit.x, hit.y)));
                    assert!(hit.y >= 0, "{:?} {}", row.panel, row.item);
                }
            }
        }
    }
}

#[test]
fn waypoint_skill_and_stash_art_and_hits_share_the_panel_shift() {
    use crate::rules::adaptive_integer::FrameSelection;
    use crate::rules::camera::FrameSize;
    use crate::ui::ScaleMode;
    let rows = crate::ui::layout::panel_layout().unwrap();
    let find = |panel, kind, item: &str| {
        rows.iter()
            .find(|r| r.panel == panel && r.kind == kind && r.item == item)
            .unwrap()
    };
    let waypoint = PanelKey::Ui(crate::ui::panels::waypoint::UI_WAYPOINT);
    let base_tab = find(waypoint, crate::ui::layout::RowKind::Draw, "tab0");
    let base_env = crate::ui::panels::waypoint::WpEnv::new(&Screen::R800, true);
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
        let f = FrameSelection::for_window(w, h, FrameSize::WIDE, ScaleMode::Integer).frame;
        let screen = Screen::for_frame(f);
        let dy = screen.panel_dy();
        // Waypoint: the drawn tab row and the input strip move by the panel shift.
        let env = crate::ui::panels::waypoint::WpEnv::new(&screen, true);
        let tab = find(waypoint, crate::ui::layout::RowKind::Draw, "tab0");
        assert_eq!(tab.y.eval(&screen), base_tab.y.eval(&Screen::R800) + dy);
        assert_eq!(-env.sy - (-base_env.sy), dy);
        assert_eq!(
            tab.y.eval(&screen) + env.sy,
            base_tab.y.eval(&Screen::R800) + base_env.sy
        );
        // Skills: the icon draw position is its table position and its hit box.
        let icon = crate::ui::panels::skilltree::icon_pos(&screen, 1, 1).unwrap();
        let draw = find(
            PanelKey::Ui(4),
            crate::ui::layout::RowKind::Draw,
            "icon_c1_r1",
        );
        assert_eq!(
            (icon.x, icon.y),
            (draw.x.eval(&screen), draw.y.eval(&screen))
        );
        let hit = find(
            PanelKey::Ui(4),
            crate::ui::layout::RowKind::Hit,
            "icon_c1_r1",
        )
        .hit_rect(&screen)
        .unwrap();
        assert!(hit.contains(Point::new(icon.x + 1, icon.y - 1)));
        // Stash: the fallback grid follows the same art translation.
        for exp in [true, false] {
            let base = crate::ui::panels::stash_items::fallback_stash_grid(exp, &Screen::R800);
            let target = crate::ui::panels::stash_items::fallback_stash_grid(exp, &screen);
            assert_eq!(target.left, base.left + screen.edge_dx());
            assert_eq!(target.top, base.top + dy);
            assert_eq!(target.bottom - target.top, base.bottom - base.top);
        }
    }
}

// Game panels cover the world, its hover included: the world pick has no
// point while the pointer is over an open panel (so no unit under a panel is
// highlighted or labelled), and keeps it over the world beside the panel.
#[test]
fn the_world_is_not_hovered_under_an_open_panel() {
    use crate::ui::states::id;
    use crate::world_view::present::pick_point;
    let (left, right) = ((200, 300), (600, 300));
    let mut u = ui(Some(areas()), true);
    let w = world(AMAZON, 1, true);
    // The panels' areas read the player's class (the facts of an event).
    u.send(&w, UiEvent::CursorMoved(Point::new(left.0, left.1)));
    assert_eq!(pick_point(Some(left), &u.root), Some(left));
    assert_eq!(pick_point(Some(right), &u.root), Some(right));
    assert_eq!(pick_point(Some((300, 590)), &u.root), None, "control panel");
    for (open, covered, free) in [
        (id::INVENTORY, right, left),
        (id::CHARACTER, left, right),
        (id::SKILL_TREE, right, left),
    ] {
        u.ui.set_ui(u32::from(open), 0, false).unwrap();
        u.ui.sync_root(&mut u.root);
        assert_eq!(
            pick_point(Some(covered), &u.root),
            None,
            "ui {open:#x} covers"
        );
        assert_eq!(
            pick_point(Some(free), &u.root),
            Some(free),
            "ui {open:#x} beside"
        );
        u.ui.set_ui(u32::from(open), 1, false).unwrap();
        u.ui.sync_root(&mut u.root);
    }
    // The stash: the stash on the left, the backpack on the right.
    u.ui.set_ui(u32::from(id::STASH), 0, false).unwrap();
    u.ui.sync_root(&mut u.root);
    assert_eq!(pick_point(Some(left), &u.root), None, "stash");
    assert_eq!(
        pick_point(Some(right), &u.root),
        None,
        "backpack beside the stash"
    );
}

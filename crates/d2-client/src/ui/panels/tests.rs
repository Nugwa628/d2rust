//! Tests of the machine tables, the screen layout model and the
//! UI state machine, from the spec's test vectors and rules.

use super::super::geom::{Point, Rect};
use super::super::layout::*;
use super::super::states::*;
use super::*;
use crate::rules::camera::{FrameSize, OpenMode, ViewRect};

fn env(screen: Screen) -> GateEnv {
    GateEnv {
        expansion: true,
        player: Some(PlayerLife {
            alive: true,
            dead: false,
        }),
        chat_blocked: false,
        input_hold: false,
        modal_text: false,
        npc_active: false,
        screen,
        mouse: Point::new(0, 0),
    }
}

fn states() -> UiStates {
    UiStates::new().expect("ui-states.tsv")
}

fn set(s: &mut UiStates, e: &mut GateEnv, ui: u32, mode: u32) -> (bool, Vec<UiEffect>) {
    let mut fx = Vec::new();
    let r = s.set(ui, mode, false, e, &mut fx).expect("no fatal error");
    (r, fx)
}

const ON: u32 = 0;
const OFF: u32 = 1;
const TOGGLE: u32 = 2;

fn mode(m: u8) -> UiEffect {
    UiEffect::OpenMode(OpenMode::new(m).unwrap())
}

// ---------------------------------------------------------------- tables

#[test]
fn ui_states_table_loads_38_rows_with_slots() {
    let rows = ui_states().unwrap();
    assert_eq!(rows.len(), 38);
    assert_eq!(rows[1].name, "UI_INVENTORY");
    assert_eq!(rows[1].flag, 0x007A27C4);
    let slot = |i: usize| rows[i].slot;
    for i in [1, 4] {
        assert_eq!(slot(i), SlotKind::Right, "{i}");
    }
    for i in [2, 0x0F, 0x10, 0x14, 0x16, 0x24, 0x25] {
        assert_eq!(slot(i), SlotKind::Left, "{i}");
    }
    for i in [0x0C, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x20] {
        assert_eq!(slot(i), SlotKind::Full, "{i}");
    }
    assert_eq!(slot(0x0E), SlotKind::Anvil);
    let exp: Vec<u8> = rows.iter().filter(|r| r.exp_only).map(|r| r.id).collect();
    assert_eq!(exp, vec![0x23, 0x24, 0x25]);
}

#[test]
fn row_zero_is_all_ignore() {
    let rows = ui_states().unwrap();
    assert!(rows[0].conflicts.iter().all(|&c| c == Conflict::Ignore));
}

#[test]
fn ui_states_parser_is_strict() {
    let good = UI_STATES_TSV;
    assert!(parse_ui_states(good).is_ok());
    // M08: each perturbation must fail.
    let bad_digit = good.replacen(
        "\t02011000110330101010000111111110101000",
        "\t02011000110330101010000111111110101009",
        1,
    );
    assert!(parse_ui_states(&bad_digit).is_err());
    let bad_slot = good.replacen("\tright\t", "\tup\t", 1);
    assert!(parse_ui_states(&bad_slot).is_err());
    let short = good.replacen(
        "\t02011000110330101010000111111110101000",
        "\t0201100011033010101000011111111010100",
        1,
    );
    assert!(parse_ui_states(&short).is_err());
    let missing: String = good.lines().take(38).map(|l| format!("{l}\n")).collect();
    assert!(parse_ui_states(&missing).is_err());
}

#[test]
fn panel_layout_loads_every_row() {
    let rows = panel_layout().unwrap();
    assert_eq!(rows.len(), PANEL_LAYOUT_TSV.lines().count() - 1);
    assert!(rows.iter().any(|r| r.panel == PanelKey::Border));
    assert!(rows.iter().any(|r| r.panel == PanelKey::CtrlPnl));
    let hits = rows.iter().filter(|r| r.kind == RowKind::Hit).count();
    assert!(hits > 0);
}

#[test]
fn panel_layout_parser_is_strict() {
    let good = PANEL_LAYOUT_TSV;
    assert!(parse_panel_layout(good).is_ok());
    for (from, to) in [
        ("res2,mode_l", "res2,mode_x"),
        ("\tH+sy-224\t", "\tH+sy--224\t"),
        ("\tH+sy-224\t", "\tH + sy\t"),
        ("\tdraw\t", "\tpaint\t"),
        ("0x00498630", "zz"),
        ("\t33\t33\t", "\t-\t33\t"),
    ] {
        let bad = good.replacen(from, to, 1);
        assert_ne!(bad, good, "{from}");
        assert!(parse_panel_layout(&bad).is_err(), "{from} → {to}");
    }
}

#[test]
fn npc_menus_table_loads_48_records() {
    let m = npc_menus().unwrap();
    assert_eq!(m.len(), 48);
    // Akara (148): talk, trade, cancel (count 3).
    let akara = &m[0];
    assert_eq!((akara.npc, akara.count, akara.flag), (148, 3, 1));
    assert_eq!(
        akara.options[..2],
        [
            Some(MenuOption {
                string: 3381,
                kind: OptionKind::Talk
            }),
            Some(MenuOption {
                string: 3396,
                kind: OptionKind::Trade
            })
        ]
    );
    assert_eq!(akara.options[2], None);
    let bad = NPC_MENUS_TSV.replacen("3396:trade", "3396:steal", 1);
    assert!(parse_npc_menus(&bad).is_err());
}

// ---------------------------------------------------------------

#[test]
fn expressions_parse_and_evaluate() {
    let e = Expr::parse("W-sx-320+256").unwrap();
    assert_eq!(e.eval(&Screen::R800), 800 - 80 - 320 + 256);
    assert_eq!(e.eval(&Screen::R640), 640 - 320 + 256);
    assert_eq!(Expr::parse("W2+101").unwrap().eval(&Screen::R640), 421);
    assert_eq!(Expr::parse("34-sy").unwrap().eval(&Screen::R800), 94);
    for bad in ["", "-5", "W+-1", "W+", "w", "H +1", "1.5"] {
        assert_eq!(Expr::parse(bad), None, "{bad}");
    }
}

#[test]
fn panel_shift_by_resolution_mode() {
    assert_eq!((Screen::R800.sx(), Screen::R800.sy()), (80, -60));
    assert_eq!((Screen::R640.sx(), Screen::R640.sy()), (0, 0));
}

#[test]
fn every_panel_position_at_800_is_640_plus_80_60() {
    for r in panel_layout().unwrap() {
        if !matches!(r.panel, PanelKey::Ui(_)) {
            continue;
        }
        let (x6, y6) = (r.x.eval(&Screen::R640), r.y.eval(&Screen::R640));
        let (x8, y8) = (r.x.eval(&Screen::R800), r.y.eval(&Screen::R800));
        assert_eq!((x8, y8), (x6 + 80, y6 + 60), "line {}", r.line);
        if let (Some(a), Some(b)) = (r.hit_rect(&Screen::R640), r.hit_rect(&Screen::R800)) {
            if !r.has(Cond::Tab1Bottom) {
                assert_eq!((b.x, b.y, b.w, b.h), (a.x + 80, a.y + 60, a.w, a.h));
            }
        }
    }
}

#[test]
fn panel_quads_and_slots() {
    let s = Screen::R800;
    assert_eq!(
        s.quads(s.left_x0()),
        [(80, 316), (336, 316), (80, 492), (336, 492)]
    );
    let s = Screen::R640;
    assert_eq!(
        s.quads(s.right_x0()),
        [(320, 256), (576, 256), (320, 432), (576, 432)]
    );
    assert_eq!(Screen::R800.right_x0(), 400);
}

#[test]
fn hit_rects_cover_the_stated_columns_and_rows() {
    let rows = panel_layout().unwrap();
    let t = PanelTables {
        files: UiFiles::new(&rows),
        layout: rows,
    };
    // Skill in column 2, row 3 at 800 × 600: hit 485–531 × 211–257.
    let r = t
        .item(PanelKey::Ui(4), "icon_c2_r3", RowKind::Hit)
        .next()
        .unwrap();
    assert_eq!(r.hit_rect(&Screen::R800), Some(Rect::new(485, 211, 47, 47)));
    // Tab 1: rows H + sy − 155 … H − 50 (the bottom ignores sy).
    let r = t
        .item(PanelKey::Ui(4), "tab1", RowKind::Hit)
        .next()
        .unwrap();
    assert_eq!(
        r.hit_rect(&Screen::R800),
        Some(Rect::new(632, 385, 89, 166))
    );
    assert_eq!(
        r.hit_rect(&Screen::R640),
        Some(Rect::new(552, 325, 89, 106))
    );
}

#[test]
fn file_registry_expands_class_letters() {
    let rows = panel_layout().unwrap();
    let f = UiFiles::new(&rows);
    assert!(f.id("panel\\invchar6").is_some());
    assert!(f.id("PANEL\\InvChar6").is_some());
    for c in CLASS_LETTERS {
        assert!(f.id(&format!("spells\\skltree_{c}_back")).is_some(), "{c}");
    }
    assert_eq!(f.id("spells\\skltree_C_back"), None);
    assert!(!f.names().iter().any(|n| n.contains("ccskillicon")));
    let art = rows
        .iter()
        .find(|r| r.panel == PanelKey::Ui(4) && r.item == "art0")
        .unwrap();
    assert_eq!(
        f.row_file(art, Some(5)).and_then(|i| f.name(i)),
        Some("spells\\skltree_d_back")
    );
    assert_eq!(f.row_file(art, None), None);
}

#[test]
fn centered_in_a_span() {
    // Span [190, 268] is 79 wide.
    assert_eq!(centered_in(190, 268, 25), 190 + 27);
    assert_eq!(centered_in(190, 268, 79), 190);
    assert_eq!(centered_in(190, 268, 100), 190);
}

// ------------------------------------------------------------------

#[test]
fn bad_state_or_mode_is_fatal() {
    let mut s = states();
    let mut e = env(Screen::R800);
    let mut fx = Vec::new();
    assert_eq!(
        s.set(0x26, ON, false, &mut e, &mut fx),
        Err(UiStateError::BadState(0x26))
    );
    assert_eq!(
        s.set(1, 3, false, &mut e, &mut fx),
        Err(UiStateError::BadMode(3))
    );
    assert!(fx.is_empty());
}

#[test]
fn expansion_states_need_an_expansion_game() {
    let mut s = states();
    let mut e = env(Screen::R800);
    e.expansion = false;
    for ui in [0x23, 0x24, 0x25] {
        assert_eq!(set(&mut s, &mut e, ui, ON), (false, vec![]));
    }
    e.expansion = true;
    assert!(set(&mut s, &mut e, 0x24, ON).0);
    assert!(s.is_open(0x24));
}

#[test]
fn flag_update_by_player_life() {
    let mut s = states();
    let mut e = env(Screen::R800);
    e.player = Some(PlayerLife {
        alive: false,
        dead: false,
    });
    assert_eq!(
        set(&mut s, &mut e, 1, ON),
        (true, vec![UiEffect::InventoryHook, mode(0)])
    );
    assert!(!s.is_open(1));
    set(&mut s, &mut e, 1, TOGGLE);
    assert!(!s.is_open(1));
    // Chat toggles anyway.
    set(&mut s, &mut e, 5, TOGGLE);
    assert!(s.is_open(5));
    // Off always clears.
    s.force(4, true);
    set(&mut s, &mut e, 4, OFF);
    assert!(!s.is_open(4));
    // No player: on works.
    e.player = None;
    set(&mut s, &mut e, 1, ON);
    assert!(s.is_open(1));
}

#[test]
fn hooks_and_open_mode_effects_in_order() {
    let mut s = states();
    let mut e = env(Screen::R800);
    assert_eq!(
        set(&mut s, &mut e, 4, ON),
        (true, vec![UiEffect::Opened(4), mode(1)])
    );
    assert_eq!(
        set(&mut s, &mut e, 4, OFF),
        (true, vec![UiEffect::Closed(4), mode(0)])
    );
    // Off on a closed state: no hook, mode still recomputed.
    assert_eq!(set(&mut s, &mut e, 4, OFF), (true, vec![mode(0)]));
    // A none-slot state changes no mode.
    assert_eq!(
        set(&mut s, &mut e, 0x0A, ON),
        (true, vec![UiEffect::Opened(0x0A)])
    );
}

#[test]
fn on_twice_is_refused_by_the_diagonal() {
    let mut s = states();
    let mut e = env(Screen::R800);
    assert!(set(&mut s, &mut e, 1, ON).0);
    assert_eq!(set(&mut s, &mut e, 1, ON), (false, vec![]));
    assert!(s.is_open(1));
}

#[test]
fn set_ui_state_1_on_with_nothing_open() {
    let mut s = states();
    let mut e = env(Screen::R800);
    let (r, fx) = set(&mut s, &mut e, 1, ON);
    assert!(r && s.is_open(1));
    assert_eq!(
        fx,
        vec![UiEffect::Opened(1), UiEffect::InventoryHook, mode(1)]
    );
    assert_eq!(
        ViewRect::new(FrameSize::D2RUST, s.open_mode()).shift_x,
        -200
    );
}

#[test]
fn gate_vectors() {
    let mut e = env(Screen::R800);
    // Inventory open, character on: C[1][2] = 0, both open, mode 3.
    let mut s = states();
    set(&mut s, &mut e, 1, ON);
    assert!(set(&mut s, &mut e, 2, ON).0);
    assert!(s.is_open(1) && s.is_open(2));
    assert_eq!(s.open_mode().get(), 3);
    // Inventory open, skill tree on: C[1][4] = 1.
    let mut s = states();
    set(&mut s, &mut e, 1, ON);
    let (r, fx) = set(&mut s, &mut e, 4, ON);
    assert!(r && !s.is_open(1) && s.is_open(4));
    assert_eq!(
        fx,
        vec![
            UiEffect::Closed(1),
            UiEffect::InventoryHook,
            mode(0),
            UiEffect::Opened(4),
            mode(1)
        ]
    );
    // Stash open, inventory toggle: C[0x19][1] = 2, refused.
    let mut s = states();
    set(&mut s, &mut e, 0x19, ON);
    assert_eq!(set(&mut s, &mut e, 1, TOGGLE), (false, vec![]));
    // Quest log open, character on: C[0x0F][2] = 1.
    let mut s = states();
    set(&mut s, &mut e, 0x0F, ON);
    assert!(set(&mut s, &mut e, 2, ON).0);
    assert!(!s.is_open(0x0F) && s.is_open(2));
    assert_eq!(s.open_mode().get(), 2);
}

#[test]
fn off_and_toggle_close_skip_the_gate() {
    let mut s = states();
    let mut e = env(Screen::R800);
    s.force(0x19, true);
    s.force(1, true);
    e.modal_text = true;
    // Toggle of an open state closes it although C[0x19][1] = 2.
    assert!(set(&mut s, &mut e, 1, TOGGLE).0);
    assert!(!s.is_open(1));
    assert!(set(&mut s, &mut e, 0x19, OFF).0);
}

#[test]
fn closed_by_the_gate_stays_closed_after_a_later_refusal() {
    // Synthetic table: with inventory (1) and stash (0x19) open, a
    // character request is closed by row 1 and refused by row 0x19.
    let mut rows = ui_states().unwrap();
    rows[1].conflicts[2] = Conflict::Close;
    rows[0x19].conflicts[2] = Conflict::Refuse;
    let mut s = UiStates::from_rows(rows);
    let mut e = env(Screen::R800);
    s.force(1, true);
    s.force(0x19, true);
    assert!(!set(&mut s, &mut e, 2, ON).0);
    assert!(!s.is_open(1) && s.is_open(0x19) && !s.is_open(2));
}

#[test]
fn conflict_3_is_fatal_only_for_ui_0() {
    let mut rows = ui_states().unwrap();
    rows[1].conflicts[0] = Conflict::RefuseFatal0;
    let mut s = UiStates::from_rows(rows);
    let mut e = env(Screen::R800);
    s.force(1, true);
    let mut fx = Vec::new();
    assert_eq!(
        s.set(0, ON, false, &mut e, &mut fx),
        Err(UiStateError::Fatal3(1))
    );
    // C[1][0x0B] = 3 in the real table: refused, not fatal.
    let mut s = states();
    s.force(1, true);
    assert_eq!(set(&mut s, &mut e, 0x0B, ON), (false, vec![]));
}

#[test]
fn conflict_4_ends_the_npc_interaction() {
    // C[0x0E][8] = 4 (anvil open, NPC menu requested).
    let mut s = states();
    let mut e = env(Screen::R800);
    s.force(0x0E, true);
    e.npc_active = true;
    let (r, fx) = set(&mut s, &mut e, 8, ON);
    assert!(r);
    assert_eq!(fx, vec![UiEffect::EndNpcInteraction, UiEffect::Opened(8)]);
    assert!(!e.npc_active);
    // Without an interaction nothing ends.
    let mut s = states();
    s.force(0x0E, true);
    assert_eq!(fx_of(&mut s, &mut e, 8), vec![UiEffect::Opened(8)]);
}

fn fx_of(s: &mut UiStates, e: &mut GateEnv, ui: u32) -> Vec<UiEffect> {
    set(s, e, ui, ON).1
}

#[test]
fn chat_and_escape_menu_gate() {
    let mut s = states();
    let mut e = env(Screen::R800);
    e.chat_blocked = true;
    assert!(!set(&mut s, &mut e, 5, ON).0);
    assert!(set(&mut s, &mut e, 9, ON).0);
    set(&mut s, &mut e, 9, OFF);
    e.chat_blocked = false;
    e.input_hold = true;
    assert!(!set(&mut s, &mut e, 5, ON).0);
    assert!(!set(&mut s, &mut e, 9, ON).0);
    e.input_hold = false;
    e.player = None;
    assert_eq!(set(&mut s, &mut e, 9, ON), (false, vec![]));
    e.player = Some(PlayerLife {
        alive: false,
        dead: true,
    });
    assert_eq!(set(&mut s, &mut e, 9, ON), (false, vec![UiEffect::Respawn]));
    assert!(!s.is_open(9));
}

#[test]
fn modal_text_screen_allows_five_states() {
    let mut e = env(Screen::R800);
    e.modal_text = true;
    for ui in 0..0x26u32 {
        let mut s = states();
        let r = set(&mut s, &mut e, ui, ON).0;
        assert_eq!(r, MODAL_ALLOWED.contains(&(ui as u8)), "ui {ui}");
    }
}

#[test]
fn open_mode_table() {
    let mut e = env(Screen::R800);
    let cases: &[(&[u8], u32, u32, u8)] = &[
        (&[], 1, ON, 1),
        (&[2], 1, ON, 3),
        (&[2, 1], 1, OFF, 2),
        (&[1], 1, OFF, 0),
        (&[], 0x14, ON, 2),
        (&[4], 0x16, ON, 3),
        (&[4, 0x16], 0x16, OFF, 1),
        (&[0x24], 0x24, OFF, 0),
        (&[], 0x19, ON, 3),
        (&[0x1A], 0x1A, OFF, 0),
        (&[], 0x0E, ON, 1),
        (&[0x0E], 0x0E, OFF, 0),
    ];
    for &(open, ui, m, want) in cases {
        let mut s = states();
        for &o in open {
            s.force(o, true);
        }
        assert!(set(&mut s, &mut e, ui, m).0, "{open:?} {ui} {m}");
        assert_eq!(s.open_mode().get(), want, "{open:?} {ui} {m}");
    }
}

#[test]
fn cursor_jump() {
    let jump = |open: &[u8], ui: u32, m: u32, screen: Screen, x: i32, j: bool| {
        let mut s = states();
        for &o in open {
            s.force(o, true);
        }
        let mut e = env(screen);
        e.mouse = Point::new(x, 77);
        let mut fx = Vec::new();
        s.set(ui, m, j, &mut e, &mut fx).unwrap();
        fx.iter().find_map(|f| match f {
            UiEffect::CursorX(x) => Some(*x),
            _ => None,
        })
    };
    let r8 = Screen::R800;
    let r6 = Screen::R640;
    // Vectors: right opened at 800, x 500 → 300; left closed at 640,
    // x 600 → 440.
    assert_eq!(jump(&[], 1, ON, r8, 500, true), Some(300));
    assert_eq!(jump(&[0x14], 0x14, OFF, r6, 600, true), Some(440));
    // Bounds.
    assert_eq!(jump(&[], 1, ON, r8, 200, true), None);
    assert_eq!(jump(&[1], 1, OFF, r8, 399, true), Some(599));
    assert_eq!(jump(&[1], 1, OFF, r8, 400, true), None);
    assert_eq!(jump(&[], 0x14, ON, r8, 599, true), Some(799));
    assert_eq!(jump(&[], 0x14, ON, r8, 600, true), None);
    assert_eq!(jump(&[0x14], 0x14, OFF, r8, 400, true), None);
    // No jump without the flag, with the other side open, or for full
    // and anvil kinds.
    assert_eq!(jump(&[], 1, ON, r8, 500, false), None);
    assert_eq!(jump(&[2], 1, ON, r8, 500, true), None);
    assert_eq!(jump(&[], 0x19, ON, r8, 500, true), None);
    assert_eq!(jump(&[], 0x0E, ON, r8, 500, true), None);
}

#[test]
fn a_refused_request_changes_nothing() {
    // Stash open, inventory on / toggle: C[0x19][1] = 2 refuses. The call
    // returns 0 with no effect; every flag and the open mode stay.
    let mut e = env(Screen::R800);
    let mut s = states();
    set(&mut s, &mut e, 0x19, ON);
    let open: Vec<bool> = (0..0x26).map(|ui| s.is_open(ui)).collect();
    let mode_before = s.open_mode().get();
    for m in [ON, TOGGLE] {
        assert_eq!(set(&mut s, &mut e, 1, m), (false, vec![]));
        assert_eq!((0..0x26).map(|ui| s.is_open(ui)).collect::<Vec<_>>(), open);
        assert_eq!(s.open_mode().get(), mode_before);
    }
}

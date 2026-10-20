//! The message log window (ui 0x18) on a synthetic model.

use super::msg_log_ui::*;
use super::*;
use crate::bridge::world::{ClientUnit, UnitKey, PLAYER};
use crate::ui::draw::UiDraw;
use crate::ui::{
    ActionId, NoPanelRules, NoStrings, Point, PointerButton, StringLookup, UiCtx, UiEvent,
};

struct Strings;

impl StringLookup for Strings {
    fn get(&self, _: &str) -> Option<&[u16]> {
        None
    }
    fn get_id(&self, id: u16) -> Option<&'static [u16]> {
        let s: &str = match id {
            4035 => "Message Log",
            4144 => "Close",
            _ => return None,
        };
        Some(Box::leak(
            s.encode_utf16().collect::<Vec<_>>().into_boxed_slice(),
        ))
    }
}

fn world() -> ClientWorld {
    let mut w = ClientWorld::default();
    let key = UnitKey::new(PLAYER, 1);
    let mut u = ClientUnit::new(key);
    u.mode = 1;
    w.units.insert(key, u);
    w.local_player = Some(key);
    w
}

fn setup() -> (OriginalUi, UiRoot) {
    let config = UiConfig {
        screen: Screen::R800,
        expansion_installed: false,
    };
    let ui = OriginalUi::new(config, None).unwrap();
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    ui.install(&mut root).unwrap();
    (ui, root)
}

fn toggle_log(ui: &mut OriginalUi, root: &mut UiRoot, w: &ClientWorld) {
    let e = UiEvent::Action(ActionId(Action::ToggleMessageLog.index() as u16));
    let ctx = UiCtx {
        tick: 0,
        world: w,
        strings: &NoStrings,
    };
    ui.before_event(e, w);
    let r = root.dispatch(e, &ctx);
    ui.after_event(root, e, r).unwrap();
}

fn add_message(ui: &OriginalUi, text: &str, color: u32) {
    let t: Vec<u16> = text.encode_utf16().collect();
    ui.shared
        .borrow_mut()
        .messages
        .add(&t, color, 0, 800, false, None);
}

thread_local! {
    /// The bridge frame of the next [`draws`]: each draw is one 40 ms frame.
    static FRAME: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn draws(root: &UiRoot, w: &ClientWorld) -> Vec<UiDraw> {
    let tick = FRAME.with(|f| f.replace(f.get() + 1));
    let ctx = UiCtx {
        tick,
        world: w,
        strings: &Strings,
    };
    let mut out: Vec<UiDraw> = Vec::new();
    root.draw(&ctx, &mut out);
    out
}

fn texts(d: &[UiDraw]) -> Vec<(String, i32, i32, u16)> {
    d.iter()
        .filter_map(|d| match d {
            UiDraw::Text(t) => Some((
                String::from_utf16_lossy(&t.text),
                t.at.x,
                t.at.y,
                t.style.color,
            )),
            _ => None,
        })
        .collect()
}

// (playtest B017)
#[test]
fn the_open_log_draws_its_box_title_close_and_the_messages_newest_first() {
    let (mut ui, mut root) = setup();
    let w = world();
    add_message(&ui, "first", 0);
    add_message(&ui, "second", 4);
    toggle_log(&mut ui, &mut root, &w);
    assert!(ui.is_open(0x18));
    let d = draws(&root, &w);
    // The box rectangle: (5, 30, 780 × 470), colour 0, mode 1.
    assert!(
        d.contains(&UiDraw::Rect(crate::ui::draw::RectRequest::sized(
            5, 30, 780, 470, 0, 1
        ))),
        "{d:?}"
    );
    let t = texts(&d);
    // The title is centred in the box: x = 5 + (780 − 66 + 1) / 2 + 1 with
    // the 8-px fallback width of 11 units; at the first item's row.
    let title = t.iter().find(|l| l.0 == "Message Log").expect("title");
    assert_eq!((title.2, title.3), (30 + 21, 4));
    let close = t.iter().find(|l| l.0 == "Close").expect("close");
    assert_eq!(close.2, 30 + 21 + (600 - 165));
    // Nothing is selected at the open: colour 0.
    assert_eq!(close.3, 0);
    // The messages, newest first, from x 37, rows 90 and 90 + 14 + 2.
    let first = t.iter().find(|l| l.0 == "second").expect("newest");
    let second = t.iter().find(|l| l.0 == "first").expect("oldest");
    assert_eq!((first.1, first.2, first.3), (5 + 0x20, 65 + 0x19, 4));
    assert_eq!(
        (second.1, second.2, second.3),
        (5 + 0x20, 65 + 0x19 + 14 + 2, 0)
    );
    // The list border and the scroll arrows are cels of the files.
    let files = ui.files();
    let names: Vec<String> = d
        .iter()
        .filter_map(|d| match d {
            UiDraw::Image(i) => files
                .name(i.image.file)
                .map(|n| format!("{n}:{}", i.image.frame)),
            _ => None,
        })
        .collect();
    assert!(names.contains(&"menu\\textslid:9".to_string()), "{names:?}");
    assert!(names.contains(&"menu\\textslid:8".to_string()));
    assert!(names.contains(&"menu\\textslid:13".to_string()));
    assert!(
        names
            .iter()
            .filter(|n| n.starts_with("menu\\boxpieces"))
            .count()
            > 40
    );
}

// The shortcut bar (the mini panel) is not drawn while the log is open:
// both sides are blocked by state 0x18, as
// in the game; the control panel stays.
#[test]
fn the_mini_panel_is_hidden_and_the_control_panel_stays() {
    let (mut ui, mut root) = setup();
    let w = world();
    let names = |root: &UiRoot, ui: &OriginalUi| -> Vec<String> {
        let files = ui.files();
        draws(root, &w)
            .iter()
            .filter_map(|d| match d {
                UiDraw::Image(i) => files.name(i.image.file).map(str::to_string),
                _ => None,
            })
            .collect()
    };
    assert!(names(&root, &ui)
        .iter()
        .any(|n| n.starts_with("panel\\minipanel")));
    toggle_log(&mut ui, &mut root, &w);
    let n = names(&root, &ui);
    assert!(!n.iter().any(|n| n.starts_with("panel\\minipanel")));
    assert!(n.iter().any(|n| n.starts_with("panel\\800ctrlpnl")));
}

// The Close item runs SetUIState(0x18, off) on the release
// over the pressed item.
#[test]
fn the_close_item_closes_the_log() {
    let (mut ui, mut root) = setup();
    let w = world();
    toggle_log(&mut ui, &mut root, &w);
    let at = Point::new(400, 30 + 21 + 435 - 4);
    let button = PointerButton::Left;
    let send = |ui: &mut OriginalUi, root: &mut UiRoot, e: UiEvent| {
        let ctx = UiCtx {
            tick: 0,
            world: &w,
            strings: &Strings,
        };
        ui.before_event(e, &w);
        let r = root.dispatch(e, &ctx);
        ui.after_event(root, e, r).unwrap();
    };
    send(&mut ui, &mut root, UiEvent::CursorMoved(at));
    send(&mut ui, &mut root, UiEvent::Press { button, at });
    assert!(ui.is_open(0x18));
    send(&mut ui, &mut root, UiEvent::Release { button, at });
    assert!(!ui.is_open(0x18));
}

// (list box)
#[test]
fn visible_count_follows_the_list_height() {
    // 400 high: 380 usable; a one-line entry is 16.
    let lines = vec![1; 40];
    assert_eq!(visible_count(&lines, 0, 400), 23); // 23 * 16 = 368, 24 * 16 = 384 > 380
    assert_eq!(visible_count(&lines[..10], 0, 400), 10);
    // Entries of six lines are 86 high: 4 fit (344), the 5th (430) not.
    assert_eq!(visible_count(&[6; 9], 0, 400), 4);
    // From a later first entry the sum restarts; when the rest fits the
    // function returns the total, not the rest (the draw is bounded by the
    // entries).
    assert_eq!(visible_count(&lines, 30, 400), 40);
    assert_eq!(visible_count(&lines, 10, 400), 23);
}

// (list box thumb)
#[test]
fn the_thumb_shows_only_when_entries_do_not_fit() {
    let g = LogGeom::new(800, 600, true);
    assert_eq!(g.list, (5, 65, 780, 400));
    assert_eq!(g.scroll, (767, 81));
    assert_eq!(g.body, 31);
    assert!(thumb(&[1; 10], 0, &g).is_none());
    // 46 entries of one line, 23 visible: ratio 64/128, track 28 -> 14.
    let lines = vec![1; 46];
    let t = thumb(&lines, 0, &g).unwrap();
    assert_eq!(t[0], (0x10, 81 + 13));
    assert_eq!(t.len(), 1 + 14 + 1);
    assert_eq!(*t.last().unwrap(), (0xf, 81 + 13 + (14 * 3 + 3) * 4));
}

#[test]
fn a_line_loses_units_until_it_fits() {
    let w = |t: &[u16]| 6 * t.len() as i32;
    let long: Vec<u16> = "x".repeat(200).encode_utf16().collect();
    // 780 - 0x32 = 730 px: 121 units of 6 px.
    assert_eq!(clip_line(&long, &w, 780).len(), 121);
    let short: Vec<u16> = "short".encode_utf16().collect();
    assert_eq!(clip_line(&short, &w, 780), short);
}

/// Opens the log with `n` one-line messages `m0` … (newest first in the
/// list) and returns a sender.
fn scroll_setup(n: usize) -> (OriginalUi, UiRoot, ClientWorld) {
    let (mut ui, mut root) = setup();
    let w = world();
    for i in 0..n {
        add_message(&ui, &format!("m{i}"), 0);
    }
    toggle_log(&mut ui, &mut root, &w);
    (ui, root, w)
}

fn send(ui: &mut OriginalUi, root: &mut UiRoot, w: &ClientWorld, e: UiEvent) -> bool {
    let ctx = UiCtx {
        tick: 0,
        world: w,
        strings: &Strings,
    };
    ui.before_event(e, w);
    let r = root.dispatch(e, &ctx);
    ui.after_event(root, e, r).unwrap();
    r != crate::ui::Routed::Unhandled
}

/// The text of the first drawn log line (the first visible entry).
fn first_line(root: &UiRoot, w: &ClientWorld) -> String {
    texts(&draws(root, w))
        .into_iter()
        .find(|t| t.0.starts_with('m'))
        .unwrap()
        .0
}

// At 800 × 600 the scroll column is x 767, the first piece y 81, 31 body
// pieces: the up cell is x 768..=782, y 70..=80; the down cell y 442..=452.
const UP: Point = Point::new(775, 80);
const DOWN: Point = Point::new(775, 452);

// The list box's arrow step.
#[test]
fn a_held_down_arrow_steps_on_nine_of_ten_draws() {
    let (mut ui, mut root, w) = scroll_setup(40);
    let button = PointerButton::Left;
    assert_eq!(first_line(&root, &w), "m39");
    send(&mut ui, &mut root, &w, UiEvent::CursorMoved(DOWN));
    send(&mut ui, &mut root, &w, UiEvent::Press { button, at: DOWN });
    // 12 draws: counts 1..=12 step except 10.
    for _ in 0..12 {
        draws(&root, &w);
    }
    // The 13th draw (this one) steps after its lines.
    assert_eq!(first_line(&root, &w), "m28");
    // 40 entries, 23 visible: the last first entry is 17.
    for _ in 0..20 {
        draws(&root, &w);
    }
    assert_eq!(first_line(&root, &w), "m22");
    // The release over the cell steps once more (nothing left here).
    send(
        &mut ui,
        &mut root,
        &w,
        UiEvent::Release { button, at: DOWN },
    );
    assert_eq!(first_line(&root, &w), "m22");
    // Released: no more repeat; the up arrow goes back.
    send(&mut ui, &mut root, &w, UiEvent::CursorMoved(UP));
    send(&mut ui, &mut root, &w, UiEvent::Press { button, at: UP });
    send(&mut ui, &mut root, &w, UiEvent::Release { button, at: UP });
    assert_eq!(first_line(&root, &w), "m23");
}

// The repeat counts 40 ms of wall time, not draws: at 144 draws a second
// a per-draw count scrolled 5.8 times too fast.
#[test]
fn a_held_arrow_steps_at_the_same_pace_at_25_60_and_144_draws_a_second() {
    use crate::ui::ui_clock::{draw_ms, RATES};
    for fps in RATES {
        let (mut ui, mut root, w) = scroll_setup(40);
        let button = PointerButton::Left;
        ui.set_menu_wall_ms(Some(draw_ms(fps, 0)));
        send(&mut ui, &mut root, &w, UiEvent::CursorMoved(DOWN));
        send(&mut ui, &mut root, &w, UiEvent::Press { button, at: DOWN });
        // Held 420 ms: the first draw's count and one per 40 ms after it,
        // 11 counts, 10 of them steps (the 10th skips).
        let mut i = 0;
        while draw_ms(fps, i) - draw_ms(fps, 0) <= 420 {
            ui.set_menu_wall_ms(Some(draw_ms(fps, i)));
            draws(&root, &w);
            i += 1;
        }
        assert_eq!(first_line(&root, &w), "m29", "{fps}/s");
    }
}

// (the counter is not reset when the pointer leaves the cell)
#[test]
fn the_repeat_pauses_outside_the_cell_and_keeps_its_count() {
    let (mut ui, mut root, w) = scroll_setup(40);
    let button = PointerButton::Left;
    send(&mut ui, &mut root, &w, UiEvent::CursorMoved(DOWN));
    send(&mut ui, &mut root, &w, UiEvent::Press { button, at: DOWN });
    for _ in 0..5 {
        draws(&root, &w);
    }
    // The cell's bottom row (y = 453) is outside: no count, no step.
    send(
        &mut ui,
        &mut root,
        &w,
        UiEvent::CursorMoved(Point::new(775, 453)),
    );
    for _ in 0..5 {
        draws(&root, &w);
    }
    assert_eq!(first_line(&root, &w), "m34");
    // Back in: counts 6..=10, the 10th skips.
    send(&mut ui, &mut root, &w, UiEvent::CursorMoved(DOWN));
    for _ in 0..5 {
        draws(&root, &w);
    }
    assert_eq!(first_line(&root, &w), "m30");
}

// The list's arrow keys.
#[test]
fn the_wheel_and_the_up_down_keys_step_the_list() {
    let (mut ui, mut root, w) = scroll_setup(40);
    let wheel = |steps| UiEvent::Wheel {
        steps,
        at: Point::new(400, 200),
    };
    assert!(send(&mut ui, &mut root, &w, wheel(-3)));
    assert_eq!(first_line(&root, &w), "m38");
    send(&mut ui, &mut root, &w, UiEvent::Char(0xF028));
    send(&mut ui, &mut root, &w, UiEvent::Char(0xF028));
    assert_eq!(first_line(&root, &w), "m36");
    send(&mut ui, &mut root, &w, wheel(1));
    send(&mut ui, &mut root, &w, UiEvent::Char(0xF026));
    assert_eq!(first_line(&root, &w), "m38");
    // Left / Right are consumed and do nothing.
    assert!(send(&mut ui, &mut root, &w, UiEvent::Char(0xF025)));
    assert_eq!(first_line(&root, &w), "m38");
    // At the top the up step does nothing.
    for _ in 0..5 {
        send(&mut ui, &mut root, &w, UiEvent::Char(0xF026));
    }
    assert_eq!(first_line(&root, &w), "m39");
}

/// In the 16:9 frame the log spans the whole frame: its box is laid out
/// for 1068 columns and moved 134 left into the UI box's space (frame
/// columns 5..1053), its clips take the whole frame, and its Close item
/// is clicked where it is drawn.
#[test]
fn the_wide_log_spans_the_whole_frame() {
    let config = UiConfig {
        screen: Screen::WIDE,
        expansion_installed: false,
    };
    let mut ui = OriginalUi::new(config, None).unwrap();
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    ui.install(&mut root).unwrap();
    let w = world();
    toggle_log(&mut ui, &mut root, &w);
    let d = draws(&root, &w);
    assert!(
        d.contains(&UiDraw::Rect(crate::ui::draw::RectRequest::sized(
            5 - 134,
            30,
            1048,
            470,
            0,
            1
        ))),
        "{d:?}"
    );
    // The log's texts clip to the whole frame (box columns −134..934).
    let mut seen = 0;
    for x in &d {
        if let UiDraw::Text(t) = x {
            let s = String::from_utf16_lossy(&t.text);
            if s == "Message Log" || s == "Close" {
                assert_eq!(t.clip, crate::ui::Rect::new(-134, 0, 1068, 600), "{s}");
                seen += 1;
            }
        }
    }
    assert_eq!(seen, 2);
    // The Close item, centred in the frame (box column 400), closes it.
    let close = texts(&d)
        .into_iter()
        .find(|l| l.0 == "Close")
        .expect("close");
    let at = Point::new(400, close.2 - 4);
    let button = PointerButton::Left;
    for e in [
        UiEvent::CursorMoved(at),
        UiEvent::Press { button, at },
        UiEvent::Release { button, at },
    ] {
        let ctx = UiCtx {
            tick: 0,
            world: &w,
            strings: &Strings,
        };
        ui.before_event(e, &w);
        let r = root.dispatch(e, &ctx);
        ui.after_event(&mut root, e, r).unwrap();
    }
    assert!(!ui.is_open(0x18));
}

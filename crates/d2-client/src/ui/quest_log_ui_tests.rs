//! The quest log panel on a synthetic model: Q opens it and asks for the
//! quest data; the status list, the flag records and the counters of the
//! quest messages become rows, icons and text.

use super::quest_log_ui::{flags_of, request_quest_data};
use super::*;
use crate::bridge::output::Output;
use crate::bridge::world::{ClientUnit, UnitKey, PLAYER};
use crate::ui::draw::UiDraw;
use crate::ui::{ActionId, NoPanelRules, NoStrings, StringLookup, UiCtx, UiEvent};

struct Strings;

impl StringLookup for Strings {
    fn get(&self, _: &str) -> Option<&[u16]> {
        None
    }
    fn get_id(&self, id: u16) -> Option<&'static [u16]> {
        // Leaked once per call: a test table of three strings.
        let s: &str = match id {
            3714 => "Den of Evil",
            3739 => "one left",
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

fn press_q(ui: &mut OriginalUi, root: &mut UiRoot, w: &ClientWorld) {
    let e = UiEvent::Action(ActionId(Action::ToggleQuests.index() as u16));
    let ctx = UiCtx {
        tick: 0,
        world: w,
        strings: &NoStrings,
    };
    ui.before_event(e, w);
    let r = root.dispatch(e, &ctx);
    ui.after_event(root, e, r).unwrap();
}

fn drawn(ui: &OriginalUi, root: &UiRoot, w: &ClientWorld) -> (Vec<(String, u32)>, Vec<String>) {
    let ctx = UiCtx {
        tick: 0,
        world: w,
        strings: &Strings,
    };
    let mut out: Vec<UiDraw> = Vec::new();
    root.draw(&ctx, &mut out);
    let files = ui.files();
    let mut images = Vec::new();
    let mut texts = Vec::new();
    for d in out {
        match d {
            UiDraw::Image(i) => {
                images.push((files.name(i.image.file).unwrap().to_string(), i.image.frame))
            }
            UiDraw::Text(t) => texts.push(String::from_utf16_lossy(&t.text)),
            UiDraw::Rect(r) => panic!("rectangle {r:?}"),
            UiDraw::Line(_) => panic!("line"),
        }
    }
    (images, texts)
}

#[test]
fn q_opens_the_log_and_asks_for_the_quest_data_once() {
    let (mut ui, mut root) = setup();
    let w = world();
    press_q(&mut ui, &mut root, &w);
    assert!(ui.is_open(0x0F));
    assert_eq!(root.take_intents(), [request_quest_data()]);
    press_q(&mut ui, &mut root, &w);
    assert!(!ui.is_open(0x0F));
    assert!(root.take_intents().is_empty(), "closing asks for nothing");
}

#[test]
fn a_started_den_of_evil_draws_its_icon_title_and_count_text() {
    let (mut ui, mut root) = setup();
    let w = world();
    let mut status = [0u8; 41];
    status[1] = 3;
    ui.apply_output(&Output::QuestLog { status }, &w).unwrap();
    ui.apply_output(
        &Output::QuestSpecial {
            code: 1,
            words: [1, 0, 0, 0, 0, 0],
        },
        &w,
    )
    .unwrap();
    press_q(&mut ui, &mut root, &w);
    let (images, texts) = drawn(&ui, &root, &w);
    assert!(images.contains(&("menu\\questbackground".to_string(), 0)));
    assert!(
        images.contains(&("menu\\a1q1".to_string(), 0)),
        "the in-progress icon: {images:?}"
    );
    assert!(texts.contains(&"Den of Evil".to_string()), "{texts:?}");
    // Status 3 with one monster left: text 3739.
    assert!(texts.contains(&"one left".to_string()), "{texts:?}");
}

#[test]
fn flags_of_reads_48_little_endian_slots() {
    let mut r = [0u8; 96];
    r[2] = 0x04;
    r[3] = 0x80;
    let f = flags_of(&r);
    assert_eq!(f.word(1), 0x8004);
    assert!(f.bit(1, 2) && f.bit(1, 15) && !f.bit(0, 0));
}

/// The long text of the Den of Evil's "one left" row, to wrap at 270 px.
struct LongStrings;

impl StringLookup for LongStrings {
    fn get(&self, _: &str) -> Option<&[u16]> {
        None
    }
    fn get_id(&self, id: u16) -> Option<&'static [u16]> {
        let s: &str = match id {
            3714 => "Den of Evil",
            // 90 characters: two 45-character lines of the 6-px test font.
            3739 => "aaaa bbbb cccc dddd eeee ffff gggg hhhh iiii jjjj kkkk lll mmmm nnnn oooo pppp qqqq rr",
            _ => return None,
        };
        Some(Box::leak(
            s.encode_utf16().collect::<Vec<_>>().into_boxed_slice(),
        ))
    }
}

fn started_den(ui: &mut OriginalUi, w: &ClientWorld) {
    let mut status = [0u8; 41];
    status[1] = 3;
    ui.apply_output(&Output::QuestLog { status }, w).unwrap();
    ui.apply_output(
        &Output::QuestSpecial {
            code: 1,
            words: [1, 0, 0, 0, 0, 0],
        },
        w,
    )
    .unwrap();
}

fn texts_at(
    root: &UiRoot,
    w: &ClientWorld,
    strings: &dyn StringLookup,
) -> Vec<(String, i32, i32, u16)> {
    let ctx = UiCtx {
        tick: 0,
        world: w,
        strings,
    };
    let mut out: Vec<UiDraw> = Vec::new();
    root.draw(&ctx, &mut out);
    out.into_iter()
        .filter_map(|d| match d {
            UiDraw::Text(t) => Some((
                String::from_utf16_lossy(&t.text),
                t.at.x,
                t.at.y,
                t.style.font,
            )),
            _ => None,
        })
        .collect()
}

// The description pane: font 8, the title centred on x 160
// of the art, its text wrapped at 270 px from x 16, rows 248 / 270 + 20 i
// (playtest B023).
#[test]
fn the_description_pane_has_the_original_font_wrap_and_rows() {
    use crate::ui::original::{FontMeasure, CHARACTER_FONTS};
    let (mut ui, mut root) = setup();
    let mut src = crate::assets::path::MemorySource::default();
    for id in CHARACTER_FONTS {
        src.insert(
            crate::ui::font_info(id).unwrap().tbl_path,
            super::character_bind_tests::tbl(6),
        );
    }
    ui.set_fonts(FontMeasure::load(&src, &CHARACTER_FONTS).unwrap());
    let w = world();
    started_den(&mut ui, &w);
    press_q(&mut ui, &mut root, &w);
    let s = Screen::R800;
    let (sx, base) = (s.sx(), s.h + s.sy() - 480);
    let t = texts_at(&root, &w, &LongStrings);
    // The title: width 11 · 6 = 66, x = sx + 160 − 33.
    assert!(
        t.contains(&("Den of Evil".to_string(), sx + 160 - 33, base + 248, 8)),
        "{t:?}"
    );
    // The text: the wrapped lines from x = sx + 16, 20 rows apart.
    let lines: Vec<&(String, i32, i32, u16)> = t.iter().filter(|l| l.1 == sx + 16).collect();
    assert!(lines.len() >= 2, "wrapped into lines: {t:?}");
    for (i, l) in lines.iter().enumerate() {
        assert_eq!((l.2, l.3), (base + 270 + 20 * i as i32, 8), "{l:?}");
        assert!(l.0.chars().count() * 6 <= 270 + 6, "{l:?}");
    }
}

// The close button acts on the release inside its rectangle; a release
// elsewhere only lets go (playtest B023).
#[test]
fn the_close_button_closes_the_log_on_release() {
    let (mut ui, mut root) = setup();
    let w = world();
    press_q(&mut ui, &mut root, &w);
    assert!(ui.is_open(0x0F));
    let s = Screen::R800;
    let (sx, base) = (s.sx(), s.h + s.sy() - 480);
    let inside = Point::new(sx + 0x116 + 5, base + 0x188 + 5);
    let button = PointerButton::Left;
    let send = |ui: &mut OriginalUi, root: &mut UiRoot, e: UiEvent| {
        let ctx = UiCtx {
            tick: 0,
            world: &w,
            strings: &NoStrings,
        };
        ui.before_event(e, &w);
        let r = root.dispatch(e, &ctx);
        ui.after_event(root, e, r).unwrap();
    };
    // Press inside, release outside: still open.
    send(&mut ui, &mut root, UiEvent::Press { button, at: inside });
    send(
        &mut ui,
        &mut root,
        UiEvent::Release {
            button,
            at: Point::new(sx + 5, base + 5),
        },
    );
    assert!(ui.is_open(0x0F), "released outside the button");
    // Press and release inside: closed.
    send(&mut ui, &mut root, UiEvent::Press { button, at: inside });
    assert!(ui.is_open(0x0F), "the press alone does not close it");
    send(&mut ui, &mut root, UiEvent::Release { button, at: inside });
    assert!(!ui.is_open(0x0F), "released inside the button");
}

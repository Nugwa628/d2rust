//! The main menu's Options screen: the flow in and out, the Video page's
//! rows, changes reaching the shared settings, and the panel drawing.

use super::*;
use crate::app::config::{Settings, WindowMode};
use crate::ui::front_end::{FrontEnd, FrontInput, CONTROLS, MAIN_MENU};
use crate::ui::original::options_menu::Row;

fn front(settings: Settings) -> FrontEnd {
    let mut f = FrontEnd::with_screens(true, Box::new(true));
    f.set_settings(settings);
    f.goto(MAIN_MENU);
    f
}

fn key(f: &mut FrontEnd, k: u16) {
    f.input(FrontInput::Key(k));
    f.tick();
}

fn click(f: &mut FrontEnd, x: i32, y: i32) {
    f.input(FrontInput::Down(Point::new(x, y)));
    f.input(FrontInput::Up(Point::new(x, y)));
    f.tick();
}

/// A fresh screen on the Video page, as a visit opens it.
fn video_screen() -> OptionsScreen {
    let mut s = OptionsScreen::default();
    s.menu.open_page(ENTRY_PAGE);
    s
}

fn adv(_: u16, t: &[u16]) -> i32 {
    t.len() as i32 * 8
}

fn texts(items: &[DrawItem]) -> Vec<&str> {
    items
        .iter()
        .filter_map(|d| match d {
            DrawItem::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn cels(items: &[DrawItem], name: &str) -> usize {
    items
        .iter()
        .filter(|d| matches!(d, DrawItem::UiCel { file, .. } if file == name))
        .count()
}

#[test]
fn the_main_menu_button_opens_the_video_page_and_esc_returns() {
    let mut f = front(Settings::default());
    click(&mut f, 400, super::super::main_menu::OPTIONS_Y - 10);
    assert_eq!(f.current(), OPTIONS);
    key(&mut f, vk::ESC);
    assert_eq!(f.current(), MAIN_MENU);
    // A second visit opens on the Video page again.
    click(&mut f, 400, super::super::main_menu::OPTIONS_Y - 10);
    assert_eq!(f.current(), OPTIONS);
    // Esc from any page goes straight back.
    key(&mut f, vk::ESC);
    assert_eq!(f.current(), MAIN_MENU);
}

#[test]
fn a_visit_opens_on_the_video_page_with_its_first_row_selected() {
    let s = video_screen();
    assert_eq!(s.menu.menu, MenuId::Video);
    assert_eq!(s.menu.rows()[s.menu.selected].row, Row::Resolution);
}

#[test]
fn the_video_page_has_every_display_row() {
    let s = video_screen();
    let rows: Vec<Row> = s.menu.rows().iter().map(|d| d.row).collect();
    for r in [
        Row::WindowMode,
        Row::WindowSize,
        Row::Scale,
        Row::VSync,
        Row::FrameLimit,
        Row::AnimationFrames,
        Row::Gamma,
        Row::Contrast,
        Row::MenuSides,
        Row::Previous,
    ] {
        assert!(rows.contains(&r), "{r:?} missing from {rows:?}");
    }
}

#[test]
fn the_display_rows_are_drawn_as_text_and_the_others_as_art() {
    let mut s = video_screen();
    let items = s.items(0, &adv);
    let t = texts(&items);
    for label in [
        "Window",
        "Window Size",
        "Scale",
        "VSync",
        "Frame Limit",
        "Animation frames",
        "Menu Sides",
    ] {
        assert!(t.contains(&label), "{label} not drawn: {t:?}");
    }
    assert_eq!(cels(&items, "*local\\gamma"), 1);
    assert_eq!(cels(&items, "*local\\contrast"), 1);
    // The title art is 295 px wide: two 256-px cels.
    assert_eq!(cels(&items, "*local\\videooptions"), 2);
}

#[test]
fn the_page_sits_on_a_dark_framed_panel_over_a_darkened_menu() {
    let mut s = video_screen();
    let items = s.items(0, &adv);
    // About three quarters dark: two half-black passes, first.
    assert_eq!(&items[..2], &BACKDROP);
    // Then the solid backing around the rows and pentagrams.
    let (l, t, r, b) = s.panel();
    assert_eq!(
        items[2],
        DrawItem::Shade {
            at: Point::new(l, t),
            w: r - l + 1,
            h: b - t + 1,
            color: PANEL_COLOR,
            mode: 5,
        }
    );
    assert!(l <= 400 - 301 && r >= 400 + 301, "{l} {r}");
    assert!((0..600).contains(&t) && (0..600).contains(&b) && t < b);
    // Framed by the box border.
    assert!(cels(&items, BOXPIECES_FILE) >= 8);
    // Every row's label and the pentagrams are inside the backing.
    for i in 0..s.menu.rows().len() {
        let y = s.menu.baseline(i);
        assert!(t <= y - 16 && y <= b, "row {i} at {y}");
    }
    assert_eq!(cels(&items, "cursor\\pentspin"), 2);
}

#[test]
fn the_panel_fits_every_page() {
    let mut s = OptionsScreen::default();
    for page in [
        MenuId::Options,
        MenuId::Sound,
        MenuId::Video,
        MenuId::Automap,
    ] {
        s.menu.open_page(page);
        let (_, t, _, b) = s.panel();
        assert!(t >= 0 && b < 600, "{page:?}: {t}..{b}");
        assert!(t <= s.menu.y_top(0), "{page:?}");
        let last = s.menu.rows().len() - 1;
        assert!(b >= s.menu.baseline(last), "{page:?}");
    }
}

#[test]
fn the_sound_page_draws_its_sliders() {
    let mut s = OptionsScreen::default();
    s.menu.open_page(MenuId::Sound);
    let items = s.items(0, &adv);
    assert_eq!(cels(&items, "*local\\sound"), 1);
    assert!(cels(&items, "widgets\\optskull") >= 2);
}

#[test]
fn the_pentagrams_spin_as_the_screen_is_drawn() {
    let mut s = video_screen();
    let frames = |items: &[DrawItem]| -> Vec<u32> {
        items
            .iter()
            .filter_map(|d| match d {
                DrawItem::UiCel { file, frame, .. } if file == "cursor\\pentspin" => Some(*frame),
                _ => None,
            })
            .collect()
    };
    let a = frames(&s.items(200, &adv));
    let b = frames(&s.items(300, &adv));
    assert_ne!(a, b);
}

#[test]
fn a_change_on_the_video_page_reaches_the_shared_settings_once() {
    let s = Settings::default();
    assert!(s.vsync);
    let mut f = front(s);
    f.goto(OPTIONS);
    assert_eq!(f.take_settings_change(), None);
    // From Resolution: Window, Window Size, Scale, VSync; then flip it.
    for _ in 0..4 {
        key(&mut f, VK_DOWN);
    }
    key(&mut f, VK_RIGHT);
    let new = f.take_settings_change().expect("a change");
    assert!(!new.vsync);
    assert_eq!(Settings { vsync: true, ..new }, s);
    assert_eq!(f.take_settings_change(), None);
}

#[test]
fn a_click_on_a_row_changes_it() {
    let mut f = front(Settings::default());
    f.goto(OPTIONS);
    let s = video_screen();
    let i = s
        .menu
        .rows()
        .iter()
        .position(|d| d.row == Row::Scale)
        .unwrap();
    let y = s.menu.baseline(i) - 8;
    f.input(FrontInput::Move(Point::new(400, y)));
    click(&mut f, 400, y);
    let new = f.take_settings_change().expect("a change");
    assert_ne!(new.scale_mode, Settings::default().scale_mode);
}

#[test]
fn the_window_size_row_follows_the_monitor_and_mode() {
    let mut f = front(Settings {
        window_mode: WindowMode::Fullscreen,
        ..Settings::default()
    });
    f.set_display(Some((1920, 1080)), Some(144));
    f.goto(OPTIONS);
    // Fullscreen: Window Size is skipped (Window, then Scale).
    key(&mut f, VK_DOWN);
    key(&mut f, VK_DOWN);
    key(&mut f, VK_RIGHT);
    let new = f.take_settings_change().expect("a change");
    assert_ne!(new.scale_mode, Settings::default().scale_mode);
    assert_eq!(new.window_size, Settings::default().window_size);
}

#[test]
fn previous_menu_climbs_to_the_options_list_then_the_main_menu() {
    let mut f = front(Settings::default());
    f.goto(OPTIONS);
    key(&mut f, VK_UP); // wraps to Previous Menu
    key(&mut f, vk::ENTER);
    assert_eq!(f.current(), OPTIONS, "on the Options list");
    // The Options list opens with Previous Menu selected.
    key(&mut f, vk::ENTER);
    assert_eq!(f.current(), MAIN_MENU);
}

#[test]
fn configure_controls_comes_back_to_the_options_list() {
    let mut f = front(Settings::default());
    f.goto(OPTIONS);
    key(&mut f, VK_UP);
    key(&mut f, vk::ENTER); // up to the Options list, Previous selected
    key(&mut f, VK_UP); // Configure Controls
    key(&mut f, vk::ENTER);
    assert_eq!(f.current(), CONTROLS);
    key(&mut f, vk::ESC);
    assert_eq!(f.current(), OPTIONS);
}

#[test]
fn without_a_sound_device_the_sound_rows_are_off() {
    let mut f = front(Settings::default());
    f.set_sound_device(false);
    f.goto(OPTIONS);
    key(&mut f, VK_UP);
    key(&mut f, vk::ENTER); // the Options list
                            // Down from Previous wraps past Sound Options to Video Options.
    key(&mut f, VK_DOWN);
    key(&mut f, vk::ENTER);
    key(&mut f, VK_DOWN);
    key(&mut f, VK_RIGHT);
    // Resolution changed, so the Video page opened, not the Sound page.
    let new = f.take_settings_change().expect("a change");
    assert_ne!(new.resolution, Settings::default().resolution);
}

#[test]
fn the_video_page_shows_the_display_and_picture_lines_on_its_panel() {
    use crate::ui::original::options_menu::WindowFacts;
    let mut f = front(Settings::default());
    f.set_display(Some((3840, 2160)), Some(144));
    f.set_window(WindowFacts {
        mode: Some(WindowMode::Windowed),
        size: Some((2560, 1377)),
        frame: Some((1068, 600)),
    });
    f.goto(OPTIONS);
    let items = f.overlay(&adv);
    let t = texts(&items);
    assert!(
        t.contains(&"Display: Window 2560x1377 on 3840x2160 at 144 Hz"),
        "{t:?}"
    );
    assert!(
        t.contains(&"Picture: 1068x600 shown at 2.29x (Sharp)"),
        "{t:?}"
    );
    assert!(t.contains(&"View (next start)"), "{t:?}");
    assert!(t.contains(&"Maximized (2560x1377)"), "{t:?}");
    // Alt+Enter to Windowed Borderless: the next tick shows it.
    f.set_window(WindowFacts {
        mode: Some(WindowMode::WindowedBorderless),
        size: Some((3840, 2160)),
        frame: Some((1068, 600)),
    });
    f.tick();
    let items = f.overlay(&adv);
    let t = texts(&items);
    assert!(
        t.contains(&"Display: 3840x2160 at 144 Hz, Windowed Borderless"),
        "{t:?}"
    );
    assert!(
        t.contains(&"Picture: 1068x600 shown at 3.6x (Sharp)"),
        "{t:?}"
    );
}

#[test]
fn the_window_size_row_shows_native_greyed_outside_a_plain_window() {
    let mut f = front(Settings {
        window_mode: WindowMode::Fullscreen,
        ..Settings::default()
    });
    f.set_display(Some((1920, 1080)), Some(60));
    f.goto(OPTIONS);
    let items = f.overlay(&adv);
    let native = items.iter().find_map(|d| match d {
        DrawItem::Text { text, color, .. } if text == "Native (1920x1080)" => Some(*color),
        _ => None,
    });
    assert_eq!(native, Some(COLOR_DISABLED));
}

/// The Video page, info lines included, leaves room for the whole panel
/// border: its top pieces from the panel's top, its bottom pieces 10 px
/// below the panel's bottom.
#[test]
fn the_video_panel_and_its_border_show_whole() {
    let mut s = video_screen();
    let (_, t, _, b) = s.panel();
    assert!(t >= 0 && b + 10 < 600, "{t}..{b}");
    let items = s.items(0, &adv);
    let pieces: Vec<i32> = items
        .iter()
        .filter_map(|d| match d {
            DrawItem::UiCel { file, at, .. } if file == BOXPIECES_FILE => Some(at.y),
            _ => None,
        })
        .collect();
    assert!(pieces.iter().all(|&y| (0..600).contains(&y)), "{pieces:?}");
    // The info lines sit inside the panel, above the first row.
    for k in 0..2 {
        let y = s.menu.info_baseline(k);
        assert!(t <= y - 13 && y < s.menu.y_top(0), "info {k} at {y}");
    }
}

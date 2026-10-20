use super::*;

fn menu_at(m: MenuId) -> OptionsMenu {
    let mut o = OptionsMenu::default();
    o.set_settings(Settings::default());
    o.open();
    o.go(m);
    o.take_events();
    o.take_changed();
    o
}

fn row_of(o: &OptionsMenu, r: Row) -> usize {
    o.rows().iter().position(|d| d.row == r).unwrap()
}

fn pt(x: i32, y: i32) -> Point {
    Point::new(x, y)
}

#[test]
fn opens_on_return_to_game_and_the_tree_navigates() {
    let mut o = OptionsMenu::default();
    o.open();
    assert_eq!((o.menu, o.selected), (MenuId::Game, 2));
    assert_eq!(o.pentagram_y(), 285 + 51);
    // Down wraps to Options.
    o.key_down();
    assert_eq!(o.selected, 0);
    assert_eq!(o.take_events(), vec![MenuEvent::CursorPass]);
    o.key_enter();
    assert_eq!((o.menu, o.selected), (MenuId::Options, 4));
    assert_eq!(o.take_events(), vec![MenuEvent::CursorSelect]);
    for (i, m) in [(0, MenuId::Sound), (1, MenuId::Video), (2, MenuId::Automap)] {
        o.selected = i;
        o.key_enter();
        assert_eq!((o.menu, o.selected), (m, m.rows().len() - 1));
        o.key_enter(); // SPrevious
        assert_eq!(o.menu, MenuId::Options);
    }
    o.selected = 3;
    o.take_events();
    o.key_enter();
    assert_eq!(
        o.take_events(),
        vec![MenuEvent::CursorSelect, MenuEvent::ConfigureControls]
    );
    o.selected = 4;
    o.key_enter();
    assert_eq!((o.menu, o.selected), (MenuId::Game, 2));
}

#[test]
fn save_and_exit_and_return_are_events() {
    let mut o = menu_at(MenuId::Game);
    o.selected = 1;
    o.key_enter();
    assert!(o.take_events().contains(&MenuEvent::SaveAndExit));
    o.selected = 2;
    o.key_enter();
    assert!(o.take_events().contains(&MenuEvent::Close));
}

#[test]
fn row_tops_follow_the_position_table() {
    let tops = |m| {
        let o = menu_at(m);
        (0..o.rows().len()).map(|i| o.y_top(i)).collect::<Vec<_>>()
    };
    assert_eq!(tops(MenuId::Game), [185, 235, 285]);
    assert_eq!(tops(MenuId::Options), [135, 185, 235, 285, 335]);
    assert_eq!(tops(MenuId::Sound), [80, 125, 170, 215, 260, 305, 350, 395]);
    assert_eq!(tops(MenuId::Automap), [103, 148, 193, 238, 283, 328, 373]);
}

#[test]
fn row_tops_at_640_follow_the_position_table() {
    let tops = |m| {
        let mut o = menu_at(m);
        o.screen = Screen::R640;
        (0..o.rows().len()).map(|i| o.y_top(i)).collect::<Vec<_>>()
    };
    // The 640 × 480 column of the table (h = 320).
    assert_eq!(tops(MenuId::Game), [125, 175, 225]);
    assert_eq!(tops(MenuId::Options), [75, 125, 175, 225, 275]);
    assert_eq!(tops(MenuId::Sound), [20, 65, 110, 155, 200, 245, 290, 335]);
    assert_eq!(tops(MenuId::Automap), [43, 88, 133, 178, 223, 268, 313]);
    let mut o = menu_at(MenuId::Game);
    assert_eq!(o.half(), 400);
    o.screen = Screen::R640;
    assert_eq!(o.half(), 320);
    // Row hit-test follows the same y: the second row's top at 640.
    assert_eq!(o.row_at(o.y_top(1) + 3 + 3), Some(1));
}

#[test]
fn up_skips_the_disabled_rows_of_the_sound_menu() {
    let mut o = menu_at(MenuId::Sound);
    assert_eq!(o.selected, 7);
    o.key_up();
    assert_eq!(o.selected, 6); // NPC Speech
    o.key_up();
    assert_eq!(o.selected, 2); // Music: Bias, EAX, 3D Sound disabled
    o.key_up();
    o.key_up(); // Sound, then wrap past the title to SPrevious
    assert_eq!(o.selected, 7);
}

#[test]
fn slider_keys_step_and_clamp() {
    let mut o = menu_at(MenuId::Sound);
    o.selected = row_of(&o, Row::Music);
    assert_eq!(o.value(o.selected), 10);
    o.key_left();
    assert_eq!((o.value(o.selected), o.settings.music_volume), (9, 45));
    assert_eq!(o.take_events(), vec![MenuEvent::CursorPass]);
    assert!(o.take_changed().is_some());
    // Sound at 20: Right changes nothing, no sound.
    o.selected = row_of(&o, Row::Sound);
    assert_eq!(o.value(o.selected), 20);
    o.key_right();
    assert_eq!(o.settings.master_volume, 100);
    assert!(o.take_events().is_empty() && o.take_changed().is_none());
}

#[test]
fn choice_keys_wrap_and_enter_cycles() {
    let mut o = menu_at(MenuId::Video);
    o.selected = row_of(&o, Row::LightQuality);
    assert_eq!(o.settings.light_quality, 2);
    o.key_right();
    assert_eq!(o.settings.light_quality, 0);
    o.key_left();
    assert_eq!(o.settings.light_quality, 2);
    let mut a = menu_at(MenuId::Automap);
    a.settings.automap_fade = 3;
    a.selected = row_of(&a, Row::MapFade);
    a.key_enter();
    assert_eq!(a.settings.automap_fade, 0);
    assert_eq!(a.take_events(), vec![MenuEvent::CursorPass]);
}

#[test]
fn gamma_and_contrast_mapping() {
    let mut o = menu_at(MenuId::Video);
    o.selected = row_of(&o, Row::Gamma);
    assert_eq!(o.value(o.selected), 10);
    o.key_right();
    assert_eq!((o.value(o.selected), o.settings.gamma), (11, 165));
    // A stored 160 snaps to 155 on game join and is marked for writing.
    let mut j = OptionsMenu::default();
    j.set_settings(Settings {
        gamma: 160,
        ..Settings::default()
    });
    assert_eq!(j.settings.gamma, 155);
    assert!(j.take_changed().is_some());
    // Contrast 100 is position 99; Left gives 98.
    o.selected = row_of(&o, Row::Contrast);
    assert_eq!(o.value(o.selected), 99);
    o.key_left();
    assert_eq!((o.value(o.selected), o.settings.contrast), (98, 98));
}

#[test]
fn drag_vectors_on_music() {
    let mut o = menu_at(MenuId::Sound);
    let y = 180; // Music row (top 170)
    for (x, p, vol) in [(352, 0, 0), (400, 4, 20), (617, 20, 100), (618, 20, 100)] {
        let mut m = o.clone();
        m.press(pt(x, y));
        assert_eq!(m.selected, 2);
        assert_eq!((m.value(2), m.settings.music_volume), (p, vol), "x {x}");
    }
    // Press outside the strict bounds: no drag.
    o.press(pt(341, y));
    assert_eq!(o.settings.music_volume, 50);
    // A drag keeps going after the pointer leaves the row (latch).
    o.release(pt(341, y));
    o.press(pt(400, y));
    o.moved(pt(450, 500));
    // x 450: trunc(trunc(98 / 6.625 + 1) / 2) = 7.
    assert_eq!(o.value(2), 7);
}

#[test]
fn pointer_rows_follow_the_sound_menu_vectors() {
    let o = menu_at(MenuId::Sound);
    assert_eq!(o.row_at(175), Some(1));
    assert_eq!(o.row_at(180), Some(2));
    assert_eq!(o.row_at(300), None); // EAX disabled
    assert_eq!(o.row_at(80), None);
    assert_eq!(o.row_at(440), None);
}

#[test]
fn click_activates_on_release_and_hover_selects() {
    let mut o = menu_at(MenuId::Game);
    o.moved(pt(400, 200));
    assert_eq!(o.selected, 0);
    o.press(pt(400, 200));
    assert_eq!(o.menu, MenuId::Game); // not yet
    o.release(pt(400, 200));
    assert_eq!(o.menu, MenuId::Options);
    // Released elsewhere: nothing.
    o.press(pt(400, 150));
    o.release(pt(400, 340));
    assert_eq!(o.menu, MenuId::Options);
}

// Party Names is enabled only while Show Party is on.
#[test]
fn party_names_is_enabled_only_while_the_party_is_shown() {
    let mut o = menu_at(MenuId::Automap);
    let names = row_of(&o, Row::MapNames);
    let mut s = Settings {
        automap_party: 1,
        ..Settings::default()
    };
    o.set_settings(s);
    assert!(o.enabled(names));
    s.automap_party = 0;
    o.set_settings(s);
    assert!(!o.enabled(names));
}

// With the sound device down (`-ns`, `--no-sound`) Sound Options, Sound,
// Music and NPC Speech are disabled; up, they are enabled.
#[test]
fn the_sound_rows_follow_the_sound_device() {
    let mut o = menu_at(MenuId::Options);
    let so = row_of(&o, Row::SoundOptions);
    assert!(o.enabled(so));
    o.sound_device = false;
    assert!(!o.enabled(so));
    o.go(MenuId::Sound);
    for r in [Row::Sound, Row::Music, Row::NpcSpeech] {
        let i = row_of(&o, r);
        assert!(!o.enabled(i), "{r:?} disabled with the device down");
        o.sound_device = true;
        assert!(o.enabled(i), "{r:?} enabled with the device up");
        o.sound_device = false;
    }
}

#[test]
fn the_window_row_cycles_the_modes_and_marks_the_change() {
    use crate::app::config::WindowMode;
    let mut o = menu_at(MenuId::Video);
    o.selected = row_of(&o, Row::WindowMode);
    assert_eq!(o.value(o.selected), 0);
    o.key_right();
    assert_eq!(o.settings.window_mode, WindowMode::WindowedBorderless);
    o.key_enter();
    assert_eq!(o.settings.window_mode, WindowMode::Fullscreen);
    o.key_right();
    assert_eq!(o.settings.window_mode, WindowMode::Windowed);
    o.key_left();
    assert_eq!(
        (o.value(o.selected), o.settings.window_mode),
        (2, WindowMode::Fullscreen)
    );
    assert_eq!(
        o.take_changed()
            .map(|s| (s.window_mode, s.alt_enter_return)),
        Some((WindowMode::Fullscreen, WindowMode::Fullscreen))
    );
    let i = o.selected;
    assert_eq!(o.rows()[i].values[1], "Windowed Borderless");
}

#[test]
fn the_window_size_row_offers_only_what_fits_the_monitor() {
    use crate::app::config::WindowSize;
    let mut o = menu_at(MenuId::Video);
    o.monitor = Some((1920, 1080));
    let i = row_of(&o, Row::WindowSize);
    o.selected = i;
    o.settings.window_size = WindowSize::Fixed(800, 600);
    assert_eq!(o.settings.window_size, WindowSize::Fixed(800, 600));
    // 1600 x 1200 and 2400 x 1800 are too tall for this monitor: right
    // goes to 640 x 480 and 1280 x 960, then wraps to Maximized and 1920 x 1080.
    o.key_right();
    assert_eq!(o.settings.window_size, WindowSize::Fixed(640, 480));
    o.key_right();
    assert_eq!(o.settings.window_size, WindowSize::Fixed(1280, 960));
    o.key_right();
    assert_eq!(o.settings.window_size, WindowSize::Maximized);
    o.key_right();
    assert_eq!(o.settings.window_size, WindowSize::Fixed(1920, 1080));
    o.key_right();
    assert_eq!(o.settings.window_size, WindowSize::Fixed(800, 600));
    o.key_left();
    assert_eq!(o.settings.window_size, WindowSize::Fixed(1920, 1080));
    assert_eq!(o.rows()[i].values[o.value(i) as usize], "1920x1080");
    assert!(o.take_changed().is_some());
    // A 2160p monitor offers every size.
    o.monitor = Some((3840, 2160));
    let mut seen = vec![o.settings.window_size];
    for _ in 0..20 {
        o.key_right();
        if !seen.contains(&o.settings.window_size) {
            seen.push(o.settings.window_size);
        }
    }
    assert_eq!(seen.len(), 10);
}

#[test]
fn the_window_size_row_is_for_a_plain_window_only() {
    use crate::app::config::WindowMode;
    let mut o = menu_at(MenuId::Video);
    let i = row_of(&o, Row::WindowSize);
    // The default: Windowed, Maximized.
    assert!(o.enabled(i));
    assert_eq!(o.rows()[i].values[o.value(i) as usize], "Maximized");
    for mode in [WindowMode::WindowedBorderless, WindowMode::Fullscreen] {
        o.settings.set_window_mode(mode);
        assert!(!o.enabled(i), "{mode:?}");
        // Greyed: moving down from the Window row skips it.
        o.selected = row_of(&o, Row::WindowMode);
        o.key_down();
        assert_eq!(o.rows()[o.selected].row, Row::Scale, "{mode:?}");
    }
    o.settings.set_window_mode(WindowMode::Windowed);
    assert!(o.enabled(i));
}

/// Every Video row, VSync and Frame Limit included, fits the 640 x 480
/// frame as well as the 800 x 600 one.
#[test]
fn the_video_page_fits_both_frames_and_every_row_is_hit() {
    for screen in [Screen::R800, Screen::R640] {
        let mut o = menu_at(MenuId::Video);
        o.screen = screen;
        let last = o.rows().len() - 1;
        row_of(&o, Row::VSync);
        row_of(&o, Row::FrameLimit);
        row_of(&o, Row::AnimationFrames);
        assert!(o.y0() >= 0);
        assert!(o.y_top(last) + o.last_row_height() <= screen.h);
        assert!(o.baseline(last) < screen.h);
        for i in 1..=last {
            let want = o.enabled(i).then_some(i);
            assert_eq!(o.row_at(o.y_top(i) + 20), want, "{screen:?} row {i}");
        }
    }
}

#[test]
fn vsync_and_frame_limit_follow_scale_and_default_to_on_and_off() {
    let o = menu_at(MenuId::Video);
    let scale = row_of(&o, Row::Scale);
    assert_eq!(row_of(&o, Row::VSync), scale + 1);
    assert_eq!(row_of(&o, Row::FrameLimit), scale + 2);
    let (v, l) = (row_of(&o, Row::VSync), row_of(&o, Row::FrameLimit));
    assert_eq!(o.rows()[v].values[o.value(v) as usize], "On");
    assert_eq!(o.rows()[l].values[o.value(l) as usize], "Off");
}

#[test]
fn the_vsync_row_toggles_and_marks_the_change() {
    let mut o = menu_at(MenuId::Video);
    o.selected = row_of(&o, Row::VSync);
    o.key_right();
    assert!(!o.settings.vsync);
    assert_eq!(o.take_changed().map(|s| s.vsync), Some(false));
    o.key_enter();
    assert!(o.settings.vsync);
}

#[test]
fn the_animation_frames_row_defaults_to_smooth_and_offers_original() {
    use crate::app::config::AnimationFrameMode;
    let mut o = menu_at(MenuId::Video);
    let i = row_of(&o, Row::AnimationFrames);
    assert_eq!(i, row_of(&o, Row::FrameLimit) + 1);
    assert_eq!(o.label_text(i), "Animation frames");
    assert_eq!(o.value_text(i), "Smooth");
    assert_eq!(o.settings.animation_frames, AnimationFrameMode::Smooth);
    o.selected = i;
    o.key_left();
    assert_eq!(o.settings.animation_frames, AnimationFrameMode::Original);
    assert_eq!(o.value_text(i), "Original");
    assert_eq!(
        o.take_changed().map(|s| s.animation_frames),
        Some(AnimationFrameMode::Original)
    );
    o.key_enter();
    assert_eq!(o.settings.animation_frames, AnimationFrameMode::Smooth);
}

#[test]
fn the_frame_limit_row_offers_every_limit_with_vsync_off() {
    let mut o = menu_at(MenuId::Video);
    o.refresh_hz = Some(60);
    o.settings.vsync = false;
    o.selected = row_of(&o, Row::FrameLimit);
    let mut seen = Vec::new();
    for _ in 0..FRAME_LIMITS.len() {
        o.key_right();
        seen.push(o.settings.frame_limit);
        assert!(!o.greyed(o.selected, o.value(o.selected)));
    }
    assert_eq!(seen, [30, 60, 120, 144, 165, 240, 360, 0]);
}

#[test]
fn with_vsync_on_limits_above_the_refresh_rate_are_greyed_and_skipped() {
    let mut o = menu_at(MenuId::Video);
    o.refresh_hz = Some(144);
    let i = row_of(&o, Row::FrameLimit);
    o.selected = i;
    let mut seen = Vec::new();
    for _ in 0..FRAME_LIMITS.len() {
        o.key_right();
        seen.push(o.settings.frame_limit);
    }
    // 165, 240 and 360 are skipped; 144 itself stays (VSync holds it).
    assert_eq!(seen, [30, 60, 120, 144, 0, 30, 60, 120]);
    for (v, &limit) in FRAME_LIMITS.iter().enumerate() {
        assert_eq!(o.greyed(i, v as i64), limit > 144, "{limit}");
    }
    // A limit picked with VSync off stays when VSync goes on, shown greyed.
    o.settings.vsync = false;
    o.settings.frame_limit = 240;
    assert!(!o.greyed(i, o.value(i)));
    o.settings.vsync = true;
    assert!(o.greyed(i, o.value(i)));
    assert_eq!(o.settings.frame_limit, 240);
    // An unknown refresh rate greys nothing.
    o.refresh_hz = None;
    assert!(!o.greyed(i, o.value(i)));
}

#[test]
fn the_menu_sides_row_follows_contrast_and_cycles_black_and_blur() {
    use crate::app::config::MenuSides;
    let mut o = menu_at(MenuId::Video);
    let i = row_of(&o, Row::MenuSides);
    assert_eq!(i, row_of(&o, Row::Contrast) + 1);
    assert!(o.enabled(i));
    o.selected = i;
    assert_eq!(o.value(i), 0);
    assert_eq!(o.rows()[i].values[o.value(i) as usize], "Black");
    o.key_right();
    assert_eq!(o.settings.menu_sides, MenuSides::Blur);
    assert_eq!(o.rows()[i].values[o.value(i) as usize], "Blur");
    o.key_enter();
    assert_eq!(o.settings.menu_sides, MenuSides::Black);
    o.key_left();
    assert_eq!(
        o.take_changed().map(|s| s.menu_sides),
        Some(MenuSides::Blur)
    );
}

#[test]
fn the_view_row_names_the_picture_shape_and_keeps_its_stored_values() {
    let mut o = menu_at(MenuId::Video);
    let i = row_of(&o, Row::Resolution);
    assert_eq!(o.rows()[i].label, "View (next start)");
    assert_eq!(
        o.rows()[i].values,
        ["4:3 Classic (640)", "4:3 Original", "16:9 Widescreen"]
    );
    // 16:9 by default; the stored value is the frame index as before.
    assert_eq!(o.value_text(i), "16:9 Widescreen");
    for (v, frame) in [(0u8, (640, 480)), (1, (800, 600)), (2, (1068, 600))] {
        o.settings.resolution = v;
        let f = o.settings.frame_size();
        assert_eq!((f.width, f.height), frame);
        assert_eq!(o.value_text(i), VIEW_NAMES[usize::from(v)]);
    }
}

#[test]
fn the_display_line_shows_the_monitor_and_the_window_on_it() {
    use crate::app::config::WindowMode::*;
    let uhd = Some((3840, 2160));
    assert_eq!(
        display_line(uhd, Some(144), Fullscreen, Some((3840, 2160))),
        "Display: 3840x2160 at 144 Hz, Fullscreen"
    );
    assert_eq!(
        display_line(uhd, Some(144), WindowedBorderless, None),
        "Display: 3840x2160 at 144 Hz, Windowed Borderless"
    );
    assert_eq!(
        display_line(uhd, Some(144), Windowed, Some((1920, 1080))),
        "Display: Window 1920x1080 on 3840x2160 at 144 Hz"
    );
    // What the platform hasn't reported is left out.
    assert_eq!(
        display_line(uhd, None, Windowed, Some((1920, 1080))),
        "Display: Window 1920x1080 on 3840x2160"
    );
    assert_eq!(
        display_line(None, None, Windowed, Some((1920, 1080))),
        "Display: Window 1920x1080"
    );
    assert_eq!(
        display_line(None, None, Fullscreen, Some((2560, 1440))),
        "Display: 2560x1440, Fullscreen"
    );
    assert_eq!(
        display_line(None, None, Windowed, None),
        "Display: ?, Windowed"
    );
}

#[test]
fn the_picture_line_shows_the_frame_its_scale_and_the_scaler() {
    use crate::app::config::ScaleMode::*;
    let wide = (1068, 600);
    assert_eq!(
        picture_line(wide, Some((3840, 2160)), Fit, None),
        "Picture: 1068x600 shown at 3.6x (Sharp)"
    );
    assert_eq!(
        picture_line(wide, Some((1920, 1080)), Fit, None),
        "Picture: 1068x600 shown at 1.8x (Sharp)"
    );
    assert_eq!(
        picture_line(wide, Some((2560, 1440)), Fit, None),
        "Picture: 1068x600 shown at 2.4x (Sharp)"
    );
    assert_eq!(
        picture_line((800, 600), Some((3840, 2160)), Integer, None),
        "Picture: 800x600 shown at 3x (Integer)"
    );
    assert_eq!(
        picture_line((800, 600), Some((1366, 768)), Fit, None),
        "Picture: 800x600 shown at 1.28x (Sharp)"
    );
    // Too small for Integer: shown as Fit is, and said so.
    assert_eq!(
        picture_line((800, 600), Some((640, 480)), Integer, None),
        "Picture: 800x600 shown at 0.8x (Sharp)"
    );
    // No window yet: the frame alone.
    assert_eq!(picture_line(wide, None, Fit, None), "Picture: 1068x600");
    // The View row asks for another frame: it shows at the next start.
    assert_eq!(
        picture_line((800, 600), Some((1600, 1200)), Fit, Some("16:9 Widescreen")),
        "Picture: 800x600 shown at 2x (Sharp), next start: 16:9 Widescreen"
    );
    assert_eq!(scale_text(2.0), "2x");
    assert_eq!(scale_text(3.5955), "3.6x");
    assert_eq!(scale_text(1.28), "1.28x");
    assert_eq!(scale_text(1.05), "1.05x");
}

fn facts(mode: crate::app::config::WindowMode, size: (u32, u32)) -> WindowFacts {
    WindowFacts {
        mode: Some(mode),
        size: Some(size),
        frame: Some((1068, 600)),
    }
}

#[test]
fn the_video_page_info_lines_follow_the_window_and_the_settings() {
    use crate::app::config::{ScaleMode, WindowMode};
    let mut o = menu_at(MenuId::Video);
    o.monitor = Some((3840, 2160));
    o.refresh_hz = Some(144);
    o.window = facts(WindowMode::Fullscreen, (3840, 2160));
    assert_eq!(
        o.info_lines(),
        [
            "Display: 3840x2160 at 144 Hz, Fullscreen",
            "Picture: 1068x600 shown at 3.6x (Sharp)"
        ]
    );
    // Alt+Enter to a window, and Integer picked: both lines follow.
    o.window = facts(WindowMode::Windowed, (2560, 1377));
    o.settings.scale_mode = ScaleMode::Integer;
    assert_eq!(
        o.info_lines(),
        [
            "Display: Window 2560x1377 on 3840x2160 at 144 Hz",
            "Picture: 1068x600 shown at 2x (Integer)"
        ]
    );
    // View changed to 4:3 Original: the picture in force stays until the
    // next start.
    o.settings.resolution = 1;
    assert_eq!(
        o.info_lines()[1],
        "Picture: 1068x600 shown at 2x (Integer), next start: 4:3 Original"
    );
    // Only the Video page has them.
    o.go(MenuId::Sound);
    assert!(o.info_lines().is_empty());
}

#[test]
fn the_window_size_row_shows_the_native_and_the_maximized_size() {
    use crate::app::config::{WindowMode, WindowSize};
    let mut o = menu_at(MenuId::Video);
    let i = row_of(&o, Row::WindowSize);
    o.monitor = Some((3840, 2160));
    // Windowed, Maximized: the window's real size.
    o.window = facts(WindowMode::Windowed, (2560, 1377));
    assert!(o.enabled(i));
    assert_eq!(o.value_text(i), "Maximized (2560x1377)");
    // A fixed size shows as listed.
    o.settings.window_size = WindowSize::Fixed(1920, 1080);
    assert_eq!(o.value_text(i), "1920x1080");
    // Borderless and Fullscreen: greyed, the monitor's own size.
    for mode in [WindowMode::WindowedBorderless, WindowMode::Fullscreen] {
        o.settings.set_window_mode(mode);
        o.window = facts(mode, (3840, 2160));
        assert!(!o.enabled(i), "{mode:?}");
        assert_eq!(o.value_text(i), "Native (3840x2160)", "{mode:?}");
    }
    o.monitor = None;
    o.window = WindowFacts::default();
    assert_eq!(o.value_text(i), "Native");
}

#[test]
fn a_wide_value_ends_at_the_value_column_edge() {
    // Up to 130 px: centred in the column (half + 100.. half + 230).
    assert_eq!(value_x(400, 30), 400 + 100 + 50);
    assert_eq!(value_x(400, 130), 400 + 100);
    // Wider: right-aligned on half + 230.
    assert_eq!(value_x(400, 170), 400 + 230 - 170);
}

/// The Video page with its info band fits the 640 x 480 and 800 x 600
/// frames: the band sits above the first row, nothing leaves the frame,
/// and the rows keep at least the pitch the slider boxes need (33 px).
#[test]
fn the_video_page_and_its_info_band_fit_640x480() {
    for (screen, margin) in [(Screen::R640, 0), (Screen::R800, 0), (Screen::R800, 25)] {
        let mut o = menu_at(MenuId::Video);
        o.screen = screen;
        o.margin = margin;
        let last = o.rows().len() - 1;
        assert_eq!(o.info_top(), margin, "{screen:?}");
        assert_eq!(o.y0(), o.info_top() + INFO_H);
        assert!(o.info_baseline(1) < o.y0(), "{screen:?}");
        assert!(
            o.y_top(last) + o.last_row_height() <= screen.h - margin,
            "{screen:?}"
        );
        assert!(o.baseline(last) < screen.h - margin, "{screen:?}");
        // Below a slider row there is room for its box; the other rows
        // keep a label's height.
        for i in 0..last {
            let gap = o.y_top(i + 1) - o.y_top(i);
            let least = match o.rows()[i].kind {
                Kind::Slider { .. } => SLIDER_PITCH,
                _ => 30,
            };
            assert!(gap >= least, "{screen:?} margin {margin} row {i}: {gap}");
        }
    }
}

/// A page that fits keeps one pitch for every row, sliders included.
#[test]
fn a_page_that_fits_keeps_an_even_pitch() {
    let o = menu_at(MenuId::Sound);
    let pitch = o.y_top(1) - o.y_top(0);
    for i in 1..o.rows().len() {
        assert_eq!(o.y_top(i) - o.y_top(i - 1), pitch, "row {i}");
    }
}

/// Text is drawn by the display upscaler like the rest of the picture, so
/// the Video page has no text choice of its own.
#[test]
fn the_video_page_has_no_text_row() {
    let o = menu_at(MenuId::Video);
    assert!(o.rows().iter().all(|r| r.label != "Text"));
    let last = o.rows().len() - 1;
    assert_eq!(o.rows()[last].row, Row::Previous);
    assert_eq!(o.rows()[last - 1].row, Row::MenuSides);
}

#[test]
fn every_editable_row_roundtrips_all_positions_through_the_settings_file() {
    use crate::app::config::{parse_settings, write_settings};
    for page in [MenuId::Sound, MenuId::Video, MenuId::Automap] {
        for i in 0..page.rows().len() {
            let mut menu = menu_at(page);
            menu.refresh_hz = Some(360);
            if !menu.enabled(i) {
                continue;
            }
            let n = match menu.rows()[i].kind {
                Kind::Choice(n) | Kind::Slider { n, .. } => n,
                _ => continue,
            };
            for v in 0..n {
                if !menu.allowed(i, i64::from(v)) {
                    continue;
                }
                menu.apply(i, i64::from(v));
                let changed = menu.take_changed().expect("change reaches host");
                assert_eq!(menu.value(i), i64::from(v), "{:?}", menu.rows()[i].row);
                let saved = parse_settings(&write_settings(&changed)).unwrap();
                assert_eq!(saved, changed);
                menu.set_settings(saved);
                assert_eq!(
                    menu.value(i),
                    i64::from(v),
                    "read back {:?}",
                    menu.rows()[i].row
                );
            }
        }
    }
}

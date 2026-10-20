//! The front-end host headless: a minimal app reaches the main menu from
//! the trademark, then the game start from the flow; no game files.

use std::time::Duration;

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use d2_client::app::front_host::{
    add_front_end, compose, frame_point, vk, FrontHost, HEIGHT, WIDTH,
};
use d2_client::ui::front_end::{FrontInput, Outcome, Trigger, MAIN_MENU, TRADEMARK};

fn app(first: bool) -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::asset::AssetPlugin::default(),
        bevy::input::InputPlugin,
    ))
    .init_asset::<Image>()
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        40,
    )));
    // The real systems: input feed, 40 ms driver, draw, finish.
    add_front_end(&mut app, FrontHost::new(true, Box::new(true), None, first));
    app
}

#[test]
fn reaches_main_menu_then_game_start() {
    let mut a = app(true);
    assert_eq!(a.world().non_send::<FrontHost>().front.current(), TRADEMARK);
    // Click on the trademark image (Continue on button-up).
    {
        let mut h = a.world_mut().non_send_mut::<FrontHost>();
        let p = d2_client::ui::geom::Point::new(100, 100);
        h.front.input(FrontInput::Down(p));
        h.front.input(FrontInput::Up(p));
    }
    a.update();
    a.update();
    assert_eq!(a.world().non_send::<FrontHost>().front.current(), MAIN_MENU);
    a.world_mut()
        .non_send_mut::<FrontHost>()
        .front
        .trigger(Trigger::SinglePlayer);
    a.world_mut()
        .non_send_mut::<FrontHost>()
        .front
        .trigger(Trigger::Ok);
    a.update();
    a.update();
    let out = a.world().non_send::<FrontHost>().outcome;
    assert!(
        matches!(out, Some(Outcome::GameLoad(g)) if !g.new_character),
        "{out:?}"
    );
}

#[test]
fn later_entry_is_the_main_menu_and_esc_exits() {
    let mut a = app(false);
    assert_eq!(a.world().non_send::<FrontHost>().front.current(), MAIN_MENU);
    a.world_mut()
        .non_send_mut::<FrontHost>()
        .front
        .input(FrontInput::Key(vk(KeyCode::Escape).unwrap()));
    a.update();
    a.update();
    a.update();
    assert_eq!(
        a.world().non_send::<FrontHost>().outcome,
        Some(Outcome::Exit)
    );
}

#[test]
fn input_mapping_and_compose() {
    assert_eq!(vk(KeyCode::KeyA), Some(b'A' as u16));
    assert_eq!(vk(KeyCode::Digit5), Some(b'5' as u16));
    assert_eq!(vk(KeyCode::Enter), Some(13));
    let p = frame_point(Vec2::new(640.0, 480.0), Vec2::new(1280.0, 960.0));
    assert_eq!((p.x, p.y), (400, 300));
    let px = compose(&[], None);
    assert_eq!(px.len(), (WIDTH * HEIGHT * 4) as usize);
    assert_eq!(&px[..4], &[0, 0, 0, 255]);
}

/// The main menu at 4:3 and 16:9 (Fit, the default scale): the picture's
/// pixels map exactly, the side fill is off the picture.
#[test]
fn the_pointer_maps_exactly_at_4_3_and_16_9() {
    use d2_client::app::front_host::{front_pointer, front_presentation};
    for (w, h, left, top) in [
        (800, 600, 0, 0),
        (1600, 1200, 0, 0),
        (1920, 1080, 240, 0),
        (2560, 1440, 320, 0),
        (3840, 2160, 480, 0),
    ] {
        let p = front_presentation(w, h).unwrap();
        assert_eq!((p.left, p.top), (left, top), "{w}x{h}");
        assert_eq!(p.image_size(), (w - 2 * left, h), "{w}x{h}");
        // Every picture pixel's first window pixel maps back to it.
        for (fx, fy) in [
            (0, 0),
            (1, 1),
            (399, 299),
            (400, 300),
            (799, 599),
            (123, 456),
        ] {
            let (x, y) = p.from_frame(fx, fy);
            let at = front_pointer(&p, Vec2::new(x as f32 + 0.5, y as f32 + 0.5));
            assert!(at.on_picture, "{w}x{h} {fx},{fy}");
            assert_eq!((at.at.x, at.at.y), (fx, fy), "{w}x{h}");
        }
        // The window pixels beside the picture are off it, and count as
        // its nearest edge pixel.
        if left > 0 {
            for x in [0.5, left as f32 - 0.5] {
                let at = front_pointer(&p, Vec2::new(x, 300.5));
                assert!(!at.on_picture, "{w}x{h} {x}");
                assert_eq!(at.at.x, 0);
            }
            let at = front_pointer(&p, Vec2::new((w - 1) as f32 + 0.5, 0.5));
            assert!(!at.on_picture);
            assert_eq!((at.at.x, at.at.y), (799, 0));
        }
    }
}

fn windowed(a: &mut App, w: u32, h: u32) -> Entity {
    let mut win = Window {
        resolution: bevy::window::WindowResolution::new(w, h).with_scale_factor_override(1.0),
        ..Default::default()
    };
    win.set_physical_cursor_position(None);
    a.world_mut().spawn((win, bevy::window::PrimaryWindow)).id()
}

fn click(a: &mut App, win: Entity, at: Vec2) {
    use bevy::input::mouse::MouseButtonInput;
    use bevy::input::ButtonState;
    a.world_mut()
        .get_mut::<Window>(win)
        .unwrap()
        .set_physical_cursor_position(Some(at.as_dvec2()));
    for state in [ButtonState::Pressed, ButtonState::Released] {
        a.world_mut().write_message(MouseButtonInput {
            button: MouseButton::Left,
            state,
            window: win,
        });
        a.update();
    }
    a.update();
}

/// A click on the side fill of a 16:9 window does nothing; on the picture
/// it does (the trademark screen continues on any click).
#[test]
fn a_click_beside_the_picture_is_ignored() {
    for (w, h) in [(1920, 1080), (2560, 1440), (3840, 2160)] {
        let mut a = app(true);
        let win = windowed(&mut a, w, h);
        a.update();
        click(&mut a, win, Vec2::new(10.5, h as f32 / 2.0));
        click(&mut a, win, Vec2::new(w as f32 - 10.5, h as f32 / 2.0));
        assert_eq!(
            a.world().non_send::<FrontHost>().front.current(),
            TRADEMARK,
            "{w}x{h}"
        );
        click(&mut a, win, Vec2::new(w as f32 / 2.0, h as f32 / 2.0));
        assert_eq!(
            a.world().non_send::<FrontHost>().front.current(),
            MAIN_MENU,
            "{w}x{h}"
        );
    }
}

/// The side fill shows only for "blur" on a window wider than 4:3; the
/// picture is placed the same either way.
#[test]
fn the_side_fill_shows_for_blur_on_a_wide_window() {
    use d2_client::app::config::{MenuSides, Settings, WindowChoice};
    use d2_client::app::front_fill::{FILL_H, FILL_W};
    fn fill_visible(a: &mut App) -> Option<Visibility> {
        let images = a.world().resource::<Assets<Image>>();
        let fills: Vec<AssetId<Image>> = images
            .iter()
            .filter(|(_, i)| i.width() == FILL_W && i.height() == FILL_H)
            .map(|(id, _)| id)
            .collect();
        let mut q = a.world_mut().query::<(&Sprite, &Visibility)>();
        q.iter(a.world())
            .find(|(s, _)| fills.contains(&s.image.id()))
            .map(|(_, v)| *v)
    }
    fn picture(a: &mut App) -> Transform {
        let mut q = a.world_mut().query::<(&Sprite, &Transform)>();
        let t: Vec<Transform> = q
            .iter(a.world())
            .filter(|(s, _)| s.custom_size.is_none())
            .map(|(_, t)| *t)
            .collect();
        t.into_iter().find(|t| t.translation.z == 0.0).unwrap()
    }
    for (w, h, wide) in [
        (1920, 1080, true),
        (2560, 1440, true),
        (3840, 2160, true),
        (800, 600, false),
        (1600, 1200, false),
    ] {
        let mut placed = Vec::new();
        for sides in MenuSides::ALL {
            let mut a = app(false);
            windowed(&mut a, w, h);
            a.insert_resource(WindowChoice::of(&Settings {
                menu_sides: sides,
                ..Settings::default()
            }));
            a.update();
            a.update();
            let want = if wide && sides == MenuSides::Blur {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            assert_eq!(fill_visible(&mut a), Some(want), "{w}x{h} {sides:?}");
            placed.push(picture(&mut a));
        }
        assert_eq!(placed[0], placed[1], "{w}x{h}");
    }
}

/// The picture pixel `(fx, fy)` as a physical cursor position in a
/// `w x h` window (the centre of its first window pixel).
fn on_window(w: u32, h: u32, fx: i32, fy: i32) -> Vec2 {
    let p = d2_client::app::front_host::front_presentation(w, h).unwrap();
    let (x, y) = p.from_frame(fx, fy);
    Vec2::new(x as f32 + 0.5, y as f32 + 0.5)
}

/// The middle of the Display Options button, in picture pixels.
fn display_options_button() -> (i32, i32) {
    use d2_client::ui::front_end::screens::main_menu::{OPTIONS_H, OPTIONS_Y};
    (400, OPTIONS_Y - i32::from(OPTIONS_H) / 2)
}

/// The main menu's Display Options button is hit where it is drawn at 4:3
/// and 16:9 (Fit, the default scale, V10's centred picture), and Esc
/// brings the main menu back.
#[test]
fn display_options_is_clicked_where_it_shows_at_every_size() {
    use d2_client::ui::front_end::OPTIONS;
    let (bx, by) = display_options_button();
    for (w, h) in [(800, 600), (1920, 1080), (2560, 1440), (3840, 2160)] {
        let mut a = app(false);
        let win = windowed(&mut a, w, h);
        a.update();
        // Just above and below the button: nothing.
        for fy in [by - 18 - 2, by + 18 + 2] {
            click(&mut a, win, on_window(w, h, bx, fy));
            assert_eq!(
                a.world().non_send::<FrontHost>().front.current(),
                MAIN_MENU,
                "{w}x{h} y {fy}"
            );
        }
        // Its left and right ends and its middle open the Options screen.
        for fx in [264 + 1, bx, 264 + 272 - 2] {
            let mut a = app(false);
            let win = windowed(&mut a, w, h);
            a.update();
            click(&mut a, win, on_window(w, h, fx, by));
            assert_eq!(
                a.world().non_send::<FrontHost>().front.current(),
                OPTIONS,
                "{w}x{h} x {fx}"
            );
        }
        click(&mut a, win, on_window(w, h, bx, by));
        assert_eq!(a.world().non_send::<FrontHost>().front.current(), OPTIONS);
        a.world_mut()
            .non_send_mut::<FrontHost>()
            .front
            .input(FrontInput::Key(vk(KeyCode::Escape).unwrap()));
        a.update();
        a.update();
        assert_eq!(
            a.world().non_send::<FrontHost>().front.current(),
            MAIN_MENU,
            "{w}x{h}"
        );
    }
}

/// A change on the main menu's Options screen is applied at once (VSync
/// and the frame limit, the window mode, the menu sides) and saved, so a
/// game started afterwards reads the same settings.
#[test]
fn display_options_apply_live_and_are_saved() {
    use d2_client::app::config::{
        load_settings, ConfigRes, MenuSides, Settings, WindowChoice, WindowMode,
    };
    use d2_client::app::vsync::SyncChoice;
    use d2_client::controls::Bindings;
    use d2_client::ui::front_end::OPTIONS;
    let dir = std::env::temp_dir().join(format!("d2rust-frontopt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let s = Settings::default();
    let mut a = app(false);
    let win = windowed(&mut a, 1920, 1080);
    a.insert_resource(ConfigRes {
        dir: dir.clone(),
        settings: s,
        bindings: Bindings::empty(),
        sound_device: true,
    })
    .insert_resource(WindowChoice::of(&s))
    .insert_resource(SyncChoice::of(&s));
    a.update();
    let (bx, by) = display_options_button();
    click(&mut a, win, on_window(1920, 1080, bx, by));
    assert_eq!(a.world().non_send::<FrontHost>().front.current(), OPTIONS);
    let keys = |a: &mut App, ks: &[KeyCode]| {
        for k in ks {
            a.world_mut()
                .non_send_mut::<FrontHost>()
                .front
                .input(FrontInput::Key(vk(*k).unwrap()));
            a.update();
            a.update();
        }
    };
    use KeyCode::{ArrowDown as D, ArrowRight as R};
    // The Video page opens on Resolution. Window: Windowed to Windowed
    // Borderless.
    keys(&mut a, &[D, R]);
    let choice = *a.world().resource::<WindowChoice>();
    assert_eq!(choice.mode, WindowMode::WindowedBorderless);
    assert!(choice.pending, "the window follows on the next frame");
    // Window Size is off in Borderless: down twice is VSync, then Off.
    keys(&mut a, &[D, D, R]);
    assert!(!a.world().resource::<SyncChoice>().vsync);
    // Frame Limit: Off to 30.
    keys(&mut a, &[D, R]);
    assert_eq!(a.world().resource::<SyncChoice>().frame_limit, 30);
    // Menu Sides (past Animation frames, Light Quality, Blended Shadows,
    // Gamma, Contrast; Perspective is off): Black to Blur.
    keys(&mut a, &[D, D, D, D, D, D, R]);
    assert_eq!(a.world().resource::<WindowChoice>().sides, MenuSides::Blur);
    // Saved: what the next game start reads is what the screen shows.
    let cfg = a.world().resource::<ConfigRes>().settings;
    assert_eq!(
        (cfg.window_mode, cfg.vsync, cfg.frame_limit, cfg.menu_sides),
        (WindowMode::WindowedBorderless, false, 30, MenuSides::Blur)
    );
    assert_eq!(load_settings(&dir).unwrap(), cfg);
    let _ = std::fs::remove_dir_all(&dir);
}

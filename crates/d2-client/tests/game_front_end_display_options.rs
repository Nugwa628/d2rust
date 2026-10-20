//! The main menu's Display Options button and screen on the user's
//! install, rendered without a window (`front_host::compose`), for judging
//! how they look: the main menu with the button (at rest and pressed), and
//! the Options screen's Video and Sound pages, each as the 800 x 600
//! picture and as a 16:9 window shows it (1920 x 1080, the picture scaled
//! and centred). `before_*` shots draw the same rows without the darkened
//! backdrop and the panel, as the screen first looked.
//! Ignored: needs `D2_GAME_DIR`; `D2_SHOTS` names the output folder
//! (default: the temp dir).

use std::path::{Path, PathBuf};
use std::time::Duration;

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use d2_client::app::front_host::{add_front_end, compose, FrontArt, FrontHost};
use d2_client::app::front_start::{front_host, Entry};
use d2_client::app::single_player::GameData;
use d2_client::ui::box_border::BOXPIECES_FILE;
use d2_client::ui::front_end::screens::main_menu::{OPTIONS_H, OPTIONS_Y};
use d2_client::ui::front_end::*;
use d2_client::ui::geom::Point;

fn app(host: FrontHost) -> App {
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
    add_front_end(&mut app, host);
    app
}

fn host(a: &mut App) -> Mut<'_, FrontHost> {
    a.world_mut().non_send_mut::<FrontHost>()
}

fn ticks(a: &mut App, n: u32) {
    for _ in 0..n {
        a.update();
    }
}

fn png(path: &Path, w: u32, h: u32, rgba: &[u8]) {
    let file = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(rgba).unwrap();
    println!("wrote {}", path.display());
}

/// The 800 x 600 picture in a `w x h` window: scaled to the height
/// (nearest pixel) and centred, black beside it.
fn in_window(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let iw = 800 * h / 600;
    let left = (w - iw) / 2;
    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..iw {
            let (sx, sy) = (x * 800 / iw, y * 600 / h);
            let s = ((sy * 800 + sx) * 4) as usize;
            let d = ((y * w + left + x) * 4) as usize;
            out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
        }
        for x in (0..left).chain(left + iw..w) {
            out[((y * w + x) * 4 + 3) as usize] = 255;
        }
    }
    out
}

/// Draws the current screen; `plain` leaves out the darkened backdrop and
/// the panel (the rows straight over the menu art).
fn shot(a: &mut App, out: &Path, name: &str, plain: bool) {
    let mut h = host(a);
    let h = &mut *h;
    let mut items = h.front.draw();
    let over = match h.art.as_mut() {
        Some(art) => {
            let art = std::cell::RefCell::new(art);
            h.front
                .overlay(&|font, text| art.borrow_mut().text_width(font, text))
        }
        None => h.front.overlay(&|_, _| 0),
    };
    items.extend(over);
    if plain {
        let mut panel = true;
        items.retain(|d| match d {
            DrawItem::Dim { .. } => false,
            DrawItem::Shade { mode: 5, .. } if panel => {
                panel = false;
                false
            }
            DrawItem::UiCel { file, .. } => file != BOXPIECES_FILE,
            _ => true,
        });
    }
    if let Some(c) = h.front.cursor_item() {
        items.push(c);
    }
    let palette = h.front.palette();
    if let Some(art) = h.art.as_mut() {
        art.use_palette(palette);
    }
    let rgba = compose(&items, h.art.as_mut());
    png(&out.join(format!("{name}_800x600.png")), 800, 600, &rgba);
    png(
        &out.join(format!("{name}_1920x1080.png")),
        1920,
        1080,
        &in_window(&rgba, 1920, 1080),
    );
}

#[test]
#[ignore = "needs D2_GAME_DIR (the user's install)"]
fn display_options_shots_on_the_install() {
    let game = PathBuf::from(std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR"));
    let out = std::env::var_os("D2_SHOTS")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&out).unwrap();
    let saves = std::env::temp_dir().join(format!("d2rust-frontopt-saves-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&saves);
    std::fs::create_dir_all(&saves).unwrap();
    let data = GameData::select(Some(&game)).expect("live data");
    let GameData::Live(d) = &data;
    let mut art = FrontArt::new(d.archives.source());
    if let Ok(t) = d2_client::app::strings::TableStrings::load(
        d.archives.as_ref(),
        d2_client::app::strings::LANG,
    ) {
        art = art.with_strings(move |id| u16::try_from(id).map(|i| t.by_id(i)).unwrap_or_default());
    }
    use d2_data::bin::TableFiles;
    let (h, _handles) = front_host(&saves, Some(art), d.archives.lod(), Entry::MainMenu);
    let mut a = app(h);
    ticks(&mut a, 30);
    assert_eq!(host(&mut a).front.current(), MAIN_MENU);
    let button = Point::new(400, OPTIONS_Y - i32::from(OPTIONS_H) / 2);
    host(&mut a)
        .front
        .input(FrontInput::Move(Point::new(700, 300)));
    ticks(&mut a, 1);
    shot(&mut a, &out, "main_menu", false);
    host(&mut a).front.input(FrontInput::Move(button));
    host(&mut a).front.input(FrontInput::Down(button));
    ticks(&mut a, 1);
    shot(&mut a, &out, "main_menu_pressed", false);
    host(&mut a).front.input(FrontInput::Up(button));
    ticks(&mut a, 3);
    assert_eq!(host(&mut a).front.current(), OPTIONS);
    // The Video page with VSync selected (the pointer on the row).
    host(&mut a).front.input(FrontInput::Key(40));
    host(&mut a).front.input(FrontInput::Key(40));
    host(&mut a).front.input(FrontInput::Key(40));
    host(&mut a).front.input(FrontInput::Key(40));
    ticks(&mut a, 3);
    shot(&mut a, &out, "options_video", false);
    shot(&mut a, &out, "before_options_video", true);
    // The Video page's info lines on a 1080p 144 Hz monitor (made up, no
    // window here): Windowed Borderless (Window Size greyed, "Native"),
    // and a maximized window below the taskbar.
    use d2_client::app::config::WindowMode;
    use d2_client::ui::original::options_menu::WindowFacts;
    for (name, mode, size) in [
        (
            "info_borderless",
            WindowMode::WindowedBorderless,
            (1920, 1080),
        ),
        ("info_windowed", WindowMode::Windowed, (1920, 1017)),
    ] {
        {
            let mut h = host(&mut a);
            h.front.set_settings(d2_client::app::config::Settings {
                window_mode: mode,
                ..Default::default()
            });
            h.front.set_display(Some((1920, 1080)), Some(144));
            h.front.set_window(WindowFacts {
                mode: Some(mode),
                size: Some(size),
                frame: Some((1068, 600)),
            });
            // Re-entered, so the page takes the settings.
            h.front.goto(OPTIONS);
        }
        ticks(&mut a, 2);
        shot(&mut a, &out, &format!("options_video_{name}"), false);
    }
    // Up to the Options list, then the Sound page.
    host(&mut a).front.input(FrontInput::Key(0x1B));
    ticks(&mut a, 3);
    host(&mut a).front.input(FrontInput::Move(button));
    host(&mut a).front.input(FrontInput::Down(button));
    host(&mut a).front.input(FrontInput::Up(button));
    ticks(&mut a, 3);
    host(&mut a).front.input(FrontInput::Key(38));
    host(&mut a).front.input(FrontInput::Key(13));
    host(&mut a).front.input(FrontInput::Key(40));
    host(&mut a).front.input(FrontInput::Key(13));
    host(&mut a).front.input(FrontInput::Key(40));
    ticks(&mut a, 3);
    shot(&mut a, &out, "options_sound", false);
    shot(&mut a, &out, "before_options_sound", true);
}

//! Front-end screens on the user's install, rendered without a window
//! (`front_host::compose`), for comparing with screenshots of the game.
//! Ignored: needs `D2_GAME_DIR`; `D2_SAVE_DIR` (optional) is copied into a
//! temp save folder so character select lists real characters;
//! `D2_SHOTS` names the output folder (default: the temp dir).

use std::path::PathBuf;
use std::time::Duration;

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use d2_client::app::front_host::{add_front_end, compose, FrontArt, FrontHost};
use d2_client::app::front_start::{front_host, Entry};
use d2_client::app::single_player::GameData;
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

fn click(a: &mut App, p: Point) {
    host(a).front.input(FrontInput::Down(p));
    ticks(a, 1);
    host(a).front.input(FrontInput::Up(p));
    ticks(a, 25);
}

fn shot(a: &mut App, out: &std::path::Path, name: &str) {
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
    let palette = h.front.palette();
    if let Some(art) = h.art.as_mut() {
        art.use_palette(palette);
    }
    let rgba = compose(&items, h.art.as_mut());
    let path = out.join(format!("{name}.png"));
    let file = std::fs::File::create(&path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), 800, 600);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&rgba).unwrap();
    println!("wrote {} (screen {:?})", path.display(), h.front.current());
}

/// The last drawn frame as the native UI pass shows it in a 2560 x 1440
/// window (the picture 1920 x 1440), run on the CPU.
fn native_shot(a: &mut App, out: &std::path::Path, name: &str) {
    let place = d2_client::gpu_compositor::ui_pass::UiPlacement {
        frame_w: 800,
        frame_h: 600,
        out_w: 1920,
        out_h: 1440,
    };
    let rgba = host(a).native_rgba(place).expect("native frame");
    let path = out.join(format!("{name}.png"));
    let file = std::fs::File::create(&path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), 1920, 1440);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&rgba).unwrap();
    println!("wrote {}", path.display());
}

#[test]
#[ignore = "needs D2_GAME_DIR (the user's install)"]
fn front_end_screens_on_the_install() {
    let game = PathBuf::from(std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR"));
    let out = std::env::var_os("D2_SHOTS")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&out).unwrap();
    let saves = std::env::temp_dir().join(format!("d2rust-shots-saves-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&saves);
    std::fs::create_dir_all(&saves).unwrap();
    if let Some(src) = std::env::var_os("D2_SAVE_DIR") {
        for e in std::fs::read_dir(src).unwrap().flatten() {
            if e.path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("d2s"))
            {
                std::fs::copy(e.path(), saves.join(e.file_name())).unwrap();
            }
        }
    }
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
    let (h, _handles) = front_host(&saves, Some(art), d.archives.lod(), Entry::First);
    let mut a = app(h);
    // The trademark screen at its first frame (the start-up chain's videos
    // are not played: C5 stub).
    ticks(&mut a, 1);
    shot(&mut a, &out, "ours_trademark");
    // Past the trademark screen to the main menu.
    for _ in 0..400 {
        ticks(&mut a, 1);
        if host(&mut a).front.current() == MAIN_MENU {
            break;
        }
        if host(&mut a).frames % 50 == 49 {
            host(&mut a).front.input(FrontInput::Key(0x1B));
        }
    }
    ticks(&mut a, 25);
    shot(&mut a, &out, "ours_main");
    native_shot(&mut a, &out, "native_main_1440");
    click(&mut a, Point::new(400, 308));
    shot(&mut a, &out, "ours_after_single_player");
    // Character select's "Create New Character" (33..200, 470..528).
    click(&mut a, Point::new(117, 500));
    shot(&mut a, &out, "ours_create");
    // Select the Barbarian (400, 330), type a name, wait for the walk.
    host(&mut a)
        .front
        .input(FrontInput::Move(Point::new(400, 250)));
    click(&mut a, Point::new(400, 250));
    for c in "Werwer".encode_utf16() {
        host(&mut a).front.input(FrontInput::Char(c));
    }
    ticks(&mut a, 80);
    shot(&mut a, &out, "ours_create_selected");
    native_shot(&mut a, &out, "native_create_selected_1440");
    let _ = std::fs::remove_dir_all(&saves);
}

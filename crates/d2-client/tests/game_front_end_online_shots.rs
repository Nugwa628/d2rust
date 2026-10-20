//! The online screens on the user's install, rendered without a window
//! (`front_host::compose`), against the in-memory fake server: one PNG per
//! screen and state for review. Ignored: needs `D2_GAME_DIR`; `D2_SHOTS`
//! names the output folder (default: the temp dir). Run with
//! `--features online` for the main menu's ONLINE button.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use d2_client::app::front_host::{add_front_end, compose, FrontArt, FrontHost};
use d2_client::app::single_player::GameData;
use d2_client::ui::front_end::screens::online::fake::{character, FakeServer, SharedFake};
use d2_client::ui::front_end::screens::online::{self, GameInfo, Online, ServerEntry};
use d2_client::ui::front_end::*;
use d2_client::ui::geom::Point;
use d2_formats::d2s::status;

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

fn input(a: &mut App, ev: FrontInput) {
    host(a).front.input(ev);
    ticks(a, 2);
}

fn click_id(a: &mut App, id: &str) {
    let e = host(a)
        .front
        .ui_elements()
        .into_iter()
        .find(|e| e.id == id)
        .unwrap_or_else(|| panic!("no {id}"));
    host(a).front.input(FrontInput::Move(e.at));
    host(a).front.input(FrontInput::Down(e.at));
    ticks(a, 1);
    host(a).front.input(FrontInput::Up(e.at));
    ticks(a, 6);
}

fn typed(a: &mut App, s: &str) {
    for u in s.encode_utf16() {
        host(a).front.input(FrontInput::Char(u));
    }
    ticks(a, 2);
}

fn shot(a: &mut App, out: &Path, name: &str) {
    // The pointer out of the way.
    host(a).front.input(FrontInput::Move(Point::new(799, 599)));
    ticks(a, 7);
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

fn game(name: &str, difficulty: u8, players: u8, locked: bool) -> GameInfo {
    GameInfo {
        id: 0,
        name: name.into(),
        difficulty,
        players,
        max_players: 8,
        locked,
        hardcore: false,
        expansion: true,
        joinable: 0,
    }
}

#[test]
#[ignore = "needs D2_GAME_DIR (the user's install)"]
fn online_screens_on_the_install() {
    let game_dir = PathBuf::from(std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR"));
    let out = std::env::var_os("D2_SHOTS")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&out).unwrap();
    let data = GameData::select(Some(&game_dir)).expect("live data");
    let GameData::Live(d) = &data;
    let mut art = FrontArt::new(d.archives.source());
    if let Ok(t) = d2_client::app::strings::TableStrings::load(
        d.archives.as_ref(),
        d2_client::app::strings::LANG,
    ) {
        art = art.with_strings(move |id| u16::try_from(id).map(|i| t.by_id(i)).unwrap_or_default());
    }

    let mut fake = FakeServer::default();
    fake.server_name = "D2RUST Europe".into();
    fake.add_account(
        "Mogger",
        "password1",
        vec![
            character("Werwer", 4, 34, 5, status::EXPANSION),
            character("Sparks", 1, 12, 0, status::EXPANSION),
            character("Ironwolf", 6, 71, 10, status::EXPANSION),
        ],
    );
    fake.add_account("Other", "password2", vec![character("Taken", 0, 1, 0, 0)]);
    fake.add_game(game("cows", 0, 3, false), "");
    fake.add_game(game("baal run", 2, 7, false), "");
    fake.add_game(game("nm chaos", 1, 2, true), "moo");
    fake.add_game(game("full house", 0, 8, false), "");
    let server = SharedFake(std::rc::Rc::new(std::cell::RefCell::new(fake)));
    let handle = Online::new(
        Box::new(server.clone()),
        vec![
            ServerEntry {
                id: "eu1".into(),
                name: "D2RUST Europe".into(),
                address: "37.27.64.176:6112".into(),
            },
            ServerEntry {
                id: "us1".into(),
                name: "D2RUST America".into(),
                address: "127.0.0.1:6112".into(),
            },
        ],
    );
    use d2_data::bin::TableFiles;
    let mut reg = Registry::default();
    screens::register_all(&mut reg);
    d2_client::app::front_host::register_credits(&mut reg, &art);
    online::register_with(&mut reg, handle.clone());
    let front = FrontEnd::new(d.archives.lod(), Box::new(false), reg);
    let mut a = app(FrontHost::with_front(front, Some(art), false));
    ticks(&mut a, 30);
    shot(&mut a, &out, "online_00_main_menu");

    // Login.
    host(&mut a).front.trigger(Trigger::Online);
    ticks(&mut a, 10);
    shot(&mut a, &out, "online_01_login");
    typed(&mut a, "mogger");
    input(&mut a, FrontInput::Key(9));
    typed(&mut a, "wrong-pass");
    input(&mut a, FrontInput::Key(13));
    ticks(&mut a, 4);
    shot(&mut a, &out, "online_02_login_wrong_password");
    click_id(&mut a, "online_login.message_ok");
    click_id(&mut a, "online_login.new_account");
    click_id(&mut a, "online_login.field(1)");
    typed(&mut a, "secret12");
    shot(&mut a, &out, "online_03_new_account");
    click_id(&mut a, "online_login.new_account");
    server.0.borrow_mut().hold = true;
    click_id(&mut a, "online_login.field(1)");
    for _ in 0..40 {
        host(&mut a).front.input(FrontInput::Key(8));
    }
    ticks(&mut a, 2);
    typed(&mut a, "password1");
    input(&mut a, FrontInput::Key(13));
    shot(&mut a, &out, "online_04_logging_in");
    server.0.borrow_mut().release();
    ticks(&mut a, 4);
    server.0.borrow_mut().release();
    ticks(&mut a, 6);
    server.0.borrow_mut().release();
    ticks(&mut a, 6);

    // Characters.
    shot(&mut a, &out, "online_05_characters");
    click_id(&mut a, "online_chars.char(Sparks)");
    click_id(&mut a, "online_chars.delete");
    click_id(&mut a, "online_chars.yes");
    typed(&mut a, "pass");
    shot(&mut a, &out, "online_06_delete_password");
    click_id(&mut a, "online_chars.delete_cancel");

    // Create in online mode, a taken name.
    click_id(&mut a, "online_chars.create_new");
    click_id(&mut a, "online_create.class.amazon");
    ticks(&mut a, 40);
    typed(&mut a, "Taken");
    input(&mut a, FrontInput::Key(13));
    ticks(&mut a, 4);
    shot(&mut a, &out, "online_07_create_name_taken");
    click_id(&mut a, "online_create.message_ok");
    input(&mut a, FrontInput::Key(27));
    ticks(&mut a, 6);

    // Lobby with Werwer (Nightmare unlocked, Hell not).
    click_id(&mut a, "online_chars.char(Werwer)");
    click_id(&mut a, "online_chars.ok");
    ticks(&mut a, 6);
    server
        .0
        .borrow_mut()
        .chat_from("Ironwolf", "anyone for cows?");
    server
        .0
        .borrow_mut()
        .chat_from("Sparks", "need a key for the nightmare game, it is moo");
    ticks(&mut a, 4);
    shot(&mut a, &out, "online_08_lobby");
    click_id(&mut a, "online_lobby.game(baal run)");
    shot(&mut a, &out, "online_09_lobby_locked_difficulty");
    click_id(&mut a, "online_lobby.game(nm chaos)");
    click_id(&mut a, "online_lobby.primary");
    typed(&mut a, "mo");
    shot(&mut a, &out, "online_10_password_prompt");
    click_id(&mut a, "online_lobby.prompt_cancel");
    click_id(&mut a, "online_lobby.tab_create");
    click_id(&mut a, "online_lobby.field(0)");
    typed(&mut a, "werwer games");
    click_id(&mut a, "online_lobby.field(1)");
    typed(&mut a, "pw");
    shot(&mut a, &out, "online_11_create_game");
    click_id(&mut a, "online_lobby.field(0)");
    for _ in 0..12 {
        host(&mut a).front.input(FrontInput::Key(8));
    }
    typed(&mut a, "cows");
    click_id(&mut a, "online_lobby.primary");
    ticks(&mut a, 4);
    shot(&mut a, &out, "online_12_create_name_taken");
    click_id(&mut a, "online_lobby.message_ok");
    click_id(&mut a, "online_lobby.tab_join");
    typed(&mut a, "hello all");
    shot(&mut a, &out, "online_13_lobby_chat_typing");

    // A lost connection: back to login with the reason.
    server
        .0
        .borrow_mut()
        .drop_connection("The connection to the server was lost.");
    ticks(&mut a, 6);
    shot(&mut a, &out, "online_14_connection_lost");
}

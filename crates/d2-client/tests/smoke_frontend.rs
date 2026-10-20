//! Front-end smoke test (q-smoke-frontend): the whole menu flow and the
//! game start without a window, the way `main`'s `play` runs it
//! (`front_start::front_host`, the front-end host's own systems,
//! `play_start::resolve`, the app's game wiring), on synthetic art, a
//! temp save folder and the synthetic game. No game files.
//!
//! startup → trademark → main menu → Single Player → create (each class) →
//! OK → game start with that name / class; Esc → Options (each sub-menu, a
//! slider, Configure Controls with a rebind) → Save and Exit → character
//! select lists the character → select → load → same character; a
//! character with Nightmare open → difficulty popup → Nightmare; Credits,
//! and back; delete; the `--new` / `--save` CLI paths.

use crate::app_support;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use d2_client::app::front_host::{add_front_end, write_stub, FrontArt, FrontHost};
use d2_client::app::front_start::{front_host, Entry, StartChoice, StartHandles};
use d2_client::app::play_start::{self, CliStart};
use d2_client::app::single_player::{self, DEFAULT_SEED};
use d2_client::assets::path::FileSource;
use d2_client::ui::front_end::screens::create::{Class, NewCharacter, EXPANSION};
use d2_client::ui::front_end::*;
use d2_client::ui::geom::Point;
use d2_server::seams::Clock;

use app_support::SharedLink;

// ---------------------------------------------------------------- art

/// Synthetic art: every DC6 the screens name is a 32-frame 8 × 8 cel (a
/// font's 256 frames), every font table 256 glyphs of width 6, the sky
/// palette a grey ramp. Fixtures, not the user's files.
struct SyntheticArt {
    dc6: Vec<u8>,
    font: Vec<u8>,
    palette: Vec<u8>,
}

/// A `.tbl`: 256 records of width 6.
fn tbl() -> Vec<u8> {
    let mut d = b"Woo!".to_vec();
    d.extend_from_slice(&1u16.to_le_bytes());
    d.extend_from_slice(&0u16.to_le_bytes());
    d.extend_from_slice(&256u16.to_le_bytes());
    d.extend_from_slice(&[10, 0]);
    for i in 0..256u16 {
        d.extend_from_slice(&i.to_le_bytes());
        d.extend_from_slice(&[0, 6, 10, 0, 0, 0]);
        d.extend_from_slice(&i.to_le_bytes());
        d.extend_from_slice(&[0; 4]);
    }
    d
}

impl SyntheticArt {
    fn new() -> Self {
        use test_fixtures::sprites::{dc6_file, dc6_frames, Dc6Shape};
        let cel = |n: u32| {
            let shape = Dc6Shape {
                directions: 1,
                frames: n,
                width: 8,
                height: 8,
                panel: true,
            };
            dc6_file(&dc6_frames(shape, 7), 1, n)
        };
        Self {
            dc6: cel(32),
            font: cel(256),
            palette: (0..768).map(|i| (i / 3) as u8).collect(),
        }
    }
}

impl FileSource for SyntheticArt {
    fn read_file(&self, name: &str) -> Option<Result<Vec<u8>, String>> {
        if name.eq_ignore_ascii_case(SKY_PALETTE[0]) {
            return Some(Ok(self.palette.clone()));
        }
        let n = name.to_ascii_lowercase().replace('/', "\\");
        if n.contains("\\font\\") {
            return match () {
                _ if n.ends_with(".tbl") => Some(Ok(tbl())),
                _ if n.ends_with(".dc6") => Some(Ok(self.font.clone())),
                _ => None,
            };
        }
        n.ends_with(".dc6").then(|| Ok(self.dc6.clone()))
    }
}

fn art() -> FrontArt {
    FrontArt::new(Arc::new(SyntheticArt::new()))
}

// ---------------------------------------------------------- front end

fn temp_dir(tag: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("smoke-front-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The front-end window's app, headless: the host's real systems.
struct Front {
    app: App,
    handles: StartHandles,
    dir: PathBuf,
}

impl Front {
    fn open(dir: &Path, entry: Entry) -> Self {
        let (host, handles) = front_host(dir, Some(art()), true, entry);
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
        let mut f = Self {
            app,
            handles,
            dir: dir.to_path_buf(),
        };
        f.frames(1);
        f
    }

    fn host(&self) -> &FrontHost {
        self.app.world().non_send::<FrontHost>()
    }

    fn input(&mut self, ev: FrontInput) {
        self.app
            .world_mut()
            .non_send_mut::<FrontHost>()
            .front
            .input(ev);
    }

    fn frames(&mut self, n: u32) {
        for _ in 0..n {
            self.app.update();
        }
    }

    fn current(&self) -> ScreenId {
        self.host().front.current()
    }

    fn outcome(&self) -> Option<Outcome> {
        self.host().outcome
    }

    /// A point where the front end's hit test (topmost clickable control)
    /// finds the first control matching `want`.
    fn point_of(&self, want: impl Fn(&Control) -> bool) -> Point {
        let controls = self.host().front.controls();
        let clickable = |c: &Control| {
            c.enabled
                && c.visible
                && matches!(
                    c.kind,
                    ControlKind::Image | ControlKind::Button | ControlKind::AnimImage
                )
                && c.action != Action::None
        };
        let i = controls
            .iter()
            .position(|c| want(c) && clickable(c))
            .unwrap_or_else(|| panic!("no such control on {:?}", self.current()));
        let r = controls[i].hit_box();
        for y in r.y..r.y + i32::from(controls[i].h) {
            for x in r.x..r.x + i32::from(controls[i].w) {
                let p = Point::new(x, y);
                let top = controls.iter().rposition(|c| clickable(c) && c.contains(p));
                if top == Some(i) {
                    return p;
                }
            }
        }
        panic!("control {i} on {:?} is covered", self.current());
    }

    /// A click (move, down, up) on the control matching `want`.
    fn click(&mut self, want: impl Fn(&Control) -> bool) {
        let p = self.point_of(want);
        self.input(FrontInput::Move(p));
        self.input(FrontInput::Down(p));
        self.input(FrontInput::Up(p));
        self.frames(2);
    }

    fn click_trigger(&mut self, t: Trigger) {
        self.click(|c| c.action == Action::Trigger(t));
    }

    fn click_custom(&mut self, id: u32) {
        self.click(|c| c.action == Action::Custom(id));
    }

    fn key(&mut self, vk: u16) {
        self.input(FrontInput::Key(vk));
        self.input(FrontInput::KeyUp(vk));
        self.frames(2);
    }

    fn type_text(&mut self, s: &str) {
        for u in s.encode_utf16() {
            self.input(FrontInput::Char(u));
        }
        self.frames(2);
    }

    /// The texts the select screen shows (character names).
    fn texts(&self) -> Vec<String> {
        self.host()
            .front
            .controls()
            .iter()
            .filter_map(|c| c.text.clone())
            .collect()
    }

    /// The flow ended in a game load: the choice `main` resolves.
    fn choice(&self) -> (GameLoad, StartChoice) {
        let Some(Outcome::GameLoad(g)) = self.outcome() else {
            panic!("no game load: {:?} on {:?}", self.outcome(), self.current());
        };
        let c = StartChoice::resolve(g, &self.handles, &self.dir).expect("a choice");
        (g, c)
    }
}

/// Trademark (first entry) → main menu by a click on the picture.
fn to_main_menu(f: &mut Front) {
    assert_eq!(f.current(), TRADEMARK);
    // The trademark art is drawn (synthetic cel, sky palette).
    assert!(f
        .host()
        .drawn
        .iter()
        .any(|d| matches!(d, DrawItem::Art { .. })));
    f.click_trigger(Trigger::Continue);
    assert_eq!(f.current(), MAIN_MENU);
}

/// Create screen: pick `class`, type `name`, OK.
fn create(f: &mut Front, class: Class, name: &str) {
    assert_eq!(f.current(), CHAR_CREATE);
    let id = d2_client::ui::front_end::screens::create::act::CLASS + u32::from(class.id());
    f.click_custom(id);
    f.type_text(name);
    f.click_custom(d2_client::ui::front_end::screens::create::act::OK);
}

// --------------------------------------------------------------- game

struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// `main`'s `play_once` up to the window: the game start resolved from
/// the front end's choice, on the synthetic game.
fn resolve(dir: &Path, g: GameLoad, c: &StartChoice) -> anyhow::Result<play_start::GameStart> {
    play_start::resolve(
        &CliStart {
            save_dir: Some(dir.to_path_buf()),
            ..CliStart::default()
        },
        &app_support::game_data(),
        None,
        g.difficulty,
        Some(c),
    )
}

/// What the joined game holds for the local player.
#[derive(Debug, PartialEq, Eq)]
struct Joined {
    class: u32,
    name: Vec<u8>,
    difficulty: u8,
}

/// The game window's app, headless: the app's own game wiring
/// (`add_game`, the 0x67 with the front end's flags) and the classic UI
/// on synthetic panel files.
struct Game {
    app: App,
    server: app_support::Server<StepClock>,
    ms: Arc<AtomicU32>,
}

impl Game {
    fn start(start: play_start::GameStart) -> Self {
        let ms = Arc::new(AtomicU32::new(1000));
        let request = start.character.clone();
        let (link, _) = single_player::start_with(
            app_support::game_data(),
            DEFAULT_SEED,
            start.character,
            StepClock(ms.clone()),
        )
        .unwrap();
        let server = Arc::new(Mutex::new(link));
        let mut app = App::new();
        app.insert_resource(d2_client::bridge::mirror::ScriptedClock(ms.clone()));
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Image>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>();
        // The input system reads the primary window (no OS window here).
        app.world_mut()
            .spawn((Window::default(), bevy::window::PrimaryWindow));
        d2_client::app::play::add_game(&mut app, Box::new(SharedLink(server.clone())), true)
            .unwrap();
        d2_client::app::play::send_create_game_flags(&mut app, &request, start.start_flags)
            .unwrap();
        app_support::live_tables(&mut app);
        // The client's own level data, as `play::run` adds it.
        d2_client::app::play::add_client_data(
            &mut app,
            single_player::client_drlg_source(&app_support::game_data()),
            single_player::client_level_rows(&app_support::game_data()),
        );
        // The act palettes with their text colours.
        d2_client::app::palette::add_act_palettes(
            &mut app,
            d2_client::app::palette::ActPalettes {
                pl2: std::array::from_fn(|_| vec![0; 1024 + 1714 * 256 + 13 * (3 + 256)]),
                shown: None,
            },
        );
        d2_client::app::ui::add_original_ui(
            &mut app,
            d2_client::app::ui::UiParts {
                source: Arc::new(SyntheticArt::new()),
                inv_areas: None,
                expansion_installed: true,
                fonts: None,
                resist_penalties: None,
            },
        )
        .unwrap();
        // The loading screen covers the window until the world is placed
        // (the loading steps).
        d2_client::app::loading_overlay::add_loading(&mut app, Some(Arc::new(SyntheticArt::new())));
        let mut g = Self { app, server, ms };
        g.app.update();
        assert!(g.loading(), "no loading screen at game start");
        for _ in 0..200 {
            g.frame();
            if g.joined().is_some() && g.in_game() && !g.loading() {
                return g;
            }
        }
        panic!("the game never left the loading screen into the world");
    }

    fn frame(&mut self) {
        self.ms.fetch_add(40, Ordering::SeqCst);
        self.app.update();
    }

    fn loading(&self) -> bool {
        self.app
            .world()
            .resource::<d2_client::app::front_start::LoadingState>()
            .covering()
    }

    fn in_game(&self) -> bool {
        let w = self
            .app
            .world()
            .resource::<d2_client::bridge::BridgeResource>()
            .0
            .world();
        w.in_game && w.local_player.is_some()
    }

    fn joined(&self) -> Option<Joined> {
        app_support::local_player(&self.server)?;
        Some(app_support::with(&self.server, |l| {
            let sim = &l.host().game;
            let (p, _) = single_player::local_player(sim).expect("joined");
            Joined {
                class: sim.events.action.sys.units.get(p).map_or(99, |u| u.class),
                name: sim.world.rest.names.get(&p).cloned().unwrap_or_default(),
                difficulty: l.host_mut().game.events.action.hooks().ai_info.difficulty,
            }
        }))
    }

    /// One key press, then its release on the next frame.
    fn press(&mut self, key: KeyCode) {
        self.app
            .world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
        self.frame();
        let mut k = self.app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        k.release(key);
        k.clear();
        self.frame();
    }

    fn ui(&mut self) -> Mut<'_, d2_client::world_view::WorldViewUi> {
        self.app
            .world_mut()
            .non_send_mut::<d2_client::world_view::WorldViewUi>()
    }

    fn menu_open(&mut self) -> bool {
        self.ui().original.as_ref().unwrap().is_open(9)
    }

    fn exited(&mut self) -> bool {
        self.app.should_exit().is_some()
    }
}

/// A save header with progression `status` (expansion bit, difficulties
/// open) the select screen reads.
fn save_with_status(dir: &Path, name: &str, class: u8, status: u16) {
    let c = NewCharacter {
        name: name.into(),
        class: Class::Amazon,
        hardcore: false,
        expansion: true,
    };
    let path = write_stub(dir, &c, 1).unwrap();
    let mut b = std::fs::read(&path).unwrap();
    // The stub keeps its "new character" flag, or
    // the load would look for the body sections a stub does not have.
    let new = d2_formats::d2s::status::NEW;
    let status = status | (u16::from(b[0x24]) & new);
    b[0x24..0x26].copy_from_slice(&status.to_le_bytes());
    b[0x28] = class;
    // The select scan ignores the checksum but the game's load checks it
    // (the sum over the file with u32 at 0xC zeroed).
    b[0xC..0x10].fill(0);
    let sum = d2_formats::d2s::checksum(&b);
    b[0xC..0x10].copy_from_slice(&sum.to_le_bytes());
    std::fs::write(&path, b).unwrap();
}

const NAMES: [&str; 7] = ["Barb", "Sorc", "Pala", "Necro", "Assa", "Amaz", "Drui"];

// ---------------------------------------------------------------- tests

/// Each class: trademark → main menu → Single Player (no saves: create) →
/// hero, name, OK → stub written → game started with that name and class
/// at Normal.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn each_class_is_created_and_starts_the_game() {
    for (i, &(class, _, _)) in EXPANSION.iter().enumerate() {
        let name = NAMES[i];
        let dir = temp_dir(&format!("class{i}"));
        let mut f = Front::open(&dir, Entry::First);
        to_main_menu(&mut f);
        f.click_trigger(Trigger::SinglePlayer);
        create(&mut f, class, name);
        let (g, c) = f.choice();
        assert!(g.new_character, "{class:?}");
        assert!(dir.join(format!("{name}.d2s")).is_file(), "{class:?} stub");
        assert_eq!((c.name.as_str(), c.class), (name, class.id()));
        let start = resolve(&dir, g, &c).unwrap_or_else(|e| panic!("{class:?}: {e}"));
        assert_eq!(start.difficulty, 0);
        assert_eq!(start.save_path, Some(dir.join(format!("{name}.d2s"))));
        let game = Game::start(start);
        assert_eq!(
            game.joined(),
            Some(Joined {
                class: u32::from(class.id()),
                name: name.as_bytes().to_vec(),
                difficulty: 0,
            }),
            "{class:?}"
        );
    }
}

/// In game: Esc → Options → every sub-menu (a value changed in each) →
/// Configure Controls (Inventory rebound to C, Accept) → Save and Exit;
/// the front end comes back at character select with the character
/// listed; select, OK → the same character in game.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn esc_options_save_and_exit_then_reload_the_character() {
    let dir = temp_dir("esc");
    let cfg = dir.join("cfg");
    std::env::set_var("D2RUST_CONFIG_DIR", &cfg);
    let mut f = Front::open(&dir, Entry::First);
    to_main_menu(&mut f);
    f.click_trigger(Trigger::SinglePlayer);
    create(&mut f, Class::Sorceress, "Tester");
    let (g, c) = f.choice();
    let mut game = Game::start(resolve(&dir, g, &c).unwrap());

    // Esc opens the game menu (Return to Game selected).
    game.press(KeyCode::Escape);
    assert!(game.menu_open());
    // Down wraps to Options; Enter opens it (Previous selected).
    game.press(KeyCode::ArrowDown);
    game.press(KeyCode::Enter);
    // Sound, Video (1), Automap (2): open, Down to the first row,
    // Left changes it (a slider or a choice), Up wraps to Previous, Enter.
    for row in 0..3 {
        for _ in 0..=row {
            game.press(KeyCode::ArrowDown);
        }
        game.press(KeyCode::Enter);
        game.press(KeyCode::ArrowDown);
        game.press(KeyCode::ArrowLeft);
        let changed = game.ui().original.as_mut().unwrap().take_settings_change();
        assert!(changed.is_some(), "sub-menu {row}: no value changed");
        game.press(KeyCode::ArrowUp);
        game.press(KeyCode::Enter);
        assert!(game.menu_open());
    }
    // Configure Controls.
    for _ in 0..4 {
        game.press(KeyCode::ArrowDown);
    }
    game.press(KeyCode::Enter);
    let screen = game
        .ui()
        .original
        .as_ref()
        .unwrap()
        .controls_screen()
        .expect("Configure Controls open");
    // An expansion install shows the expansion table.
    assert_eq!(screen.rows().len(), 62);
    // Down to Inventory (command 1), Enter, C.
    for _ in 0..80 {
        let s = game
            .ui()
            .original
            .as_ref()
            .unwrap()
            .controls_screen()
            .unwrap();
        if s.rows()[s.selected()].cmd == 1 {
            break;
        }
        game.press(KeyCode::ArrowDown);
    }
    game.press(KeyCode::Enter);
    game.press(KeyCode::KeyC);
    // Accept (button 2).
    let at = Point::new(90 + 206 * 2 + 103, 70 + 350);
    for e in [
        d2_client::ui::UiEvent::Press {
            button: d2_client::ui::PointerButton::Left,
            at,
        },
        d2_client::ui::UiEvent::Release {
            button: d2_client::ui::PointerButton::Left,
            at,
        },
    ] {
        game.ui().queue.0.push(e);
    }
    game.frame();
    assert!(!game.ui().original.as_ref().unwrap().controls_open());
    assert!(game.menu_open());
    {
        use d2_client::controls::{Action as Act, Key};
        let ui = game.ui();
        let b = ui.bindings.as_ref().expect("bindings");
        assert!(b.inputs(Act::ToggleInventory).contains(&Key::C));
        // Esc (command 56, not reassignable) still opens the game menu.
        assert_eq!(b.inputs(Act::GameMenu), &[Key::Escape]);
    }
    assert!(cfg.join("controls.toml").is_file());

    // Esc closes the whole menu; Esc opens it; Up to Save and Exit; Enter.
    game.press(KeyCode::Escape);
    assert!(!game.menu_open());
    game.press(KeyCode::Escape);
    assert!(game.menu_open());
    game.press(KeyCode::ArrowUp);
    game.press(KeyCode::Enter);
    assert!(game.exited(), "Save and Exit did not end the game");

    // Back in the front end: the main menu (as in the game), then
    // Single Player: character select, the character listed.
    let mut f = Front::open(&dir, Entry::MainMenu);
    assert_eq!(f.current(), MAIN_MENU);
    f.click_trigger(Trigger::SinglePlayer);
    assert_eq!(f.current(), CHAR_SELECT);
    assert!(f.texts().iter().any(|t| t == "Tester"), "{:?}", f.texts());
    f.click_custom(d2_client::ui::front_end::screens::char_select::ids::SLOT_TEXT);
    f.click_custom(d2_client::ui::front_end::screens::char_select::ids::OK);
    let (g, c) = f.choice();
    assert!(!g.new_character);
    assert_eq!(c.save, Some(dir.join("Tester.d2s")));
    let game = Game::start(resolve(&dir, g, &c).unwrap());
    assert_eq!(
        game.joined(),
        Some(Joined {
            class: 1,
            name: b"Tester".to_vec(),
            difficulty: 0,
        })
    );
}

/// A character with Nightmare open: OK → the difficulty popup →
/// Nightmare → the game runs on Nightmare; Esc on the popup goes back.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn difficulty_popup_starts_the_game_on_nightmare() {
    let dir = temp_dir("diff");
    // Expansion, progression 5: Normal and Nightmare open.
    save_with_status(&dir, "Zed", 4, 0x0520);
    let mut f = Front::open(&dir, Entry::MainMenu);
    assert_eq!(f.current(), MAIN_MENU);
    f.click_trigger(Trigger::SinglePlayer);
    assert_eq!(f.current(), CHAR_SELECT);
    f.click_custom(d2_client::ui::front_end::screens::char_select::ids::SLOT_TEXT);
    f.key(13);
    assert_eq!(f.current(), DIFFICULTY);
    f.key(27);
    assert_eq!(f.current(), CHAR_SELECT);
    f.key(13);
    assert_eq!(f.current(), DIFFICULTY);
    f.click_trigger(Trigger::Difficulty(1));
    let (g, c) = f.choice();
    assert_eq!(g.difficulty, Some(1));
    let start = resolve(&dir, g, &c).unwrap();
    assert_eq!(start.difficulty, 1);
    let game = Game::start(start);
    assert_eq!(
        game.joined(),
        Some(Joined {
            class: 4,
            name: b"Zed".to_vec(),
            difficulty: 1,
        })
    );
}

/// Credits open from the main menu and come back; Esc on the main menu
/// exits.
#[test]
fn credits_and_back() {
    let dir = temp_dir("credits");
    let mut f = Front::open(&dir, Entry::MainMenu);
    f.click_trigger(Trigger::Credits);
    assert_eq!(f.current(), CREDITS);
    f.frames(30);
    f.key(27);
    assert_eq!(f.current(), MAIN_MENU);
    f.click_trigger(Trigger::Credits);
    f.click_trigger(Trigger::Exit);
    assert_eq!(f.current(), MAIN_MENU);
    f.key(27);
    assert_eq!(f.outcome(), Some(Outcome::Exit));
}

/// Character select: Delete → Yes removes the file and the slot; the
/// other character stays; Create New and Exit lead where the front end's flow says.
#[test]
fn delete_a_character() {
    use d2_client::ui::front_end::screens::char_select::ids;
    let dir = temp_dir("delete");
    for (name, class) in [("Keep", Class::Paladin), ("Gone", Class::Druid)] {
        let c = NewCharacter {
            name: name.into(),
            class,
            hardcore: false,
            expansion: true,
        };
        write_stub(&dir, &c, 1).unwrap();
    }
    let mut f = Front::open(&dir, Entry::MainMenu);
    f.click_trigger(Trigger::SinglePlayer);
    assert_eq!(f.current(), CHAR_SELECT);
    let names = f.texts();
    let slot = names
        .iter()
        .filter(|t| *t == "Keep" || *t == "Gone")
        .position(|t| t == "Gone")
        .expect("Gone listed");
    f.click_custom(ids::SLOT_TEXT + slot as u32);
    f.click_custom(ids::DELETE);
    // No keeps it.
    f.click_custom(ids::POPUP_NO);
    assert!(dir.join("Gone.d2s").is_file());
    f.click_custom(ids::DELETE);
    f.click_custom(ids::POPUP_YES);
    assert!(!dir.join("Gone.d2s").exists());
    assert!(dir.join("Keep.d2s").is_file());
    let texts = f.texts();
    assert!(!texts.iter().any(|t| t == "Gone"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Keep"), "{texts:?}");
    f.click_trigger(Trigger::CreateNew);
    assert_eq!(f.current(), CHAR_CREATE);
    f.key(27);
    assert_eq!(f.current(), CHAR_SELECT);
    f.key(27);
    assert_eq!(f.current(), MAIN_MENU);
}

/// `play --new CLASS NAME [--save-dir]` and `play --save FILE` still
/// resolve as before the front end: the new character joins with its
/// class and name; a name already saved, a save inside the install and a
/// save on synthetic data stop before the window with an error.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn play_cli_new_and_save_paths() {
    let dir = temp_dir("cli");
    let cli = CliStart {
        new: Some(("necromancer".into(), "Cli".into())),
        save_dir: Some(dir.clone()),
        difficulty: 2,
        ..CliStart::default()
    };
    let start = play_start::resolve(&cli, &app_support::game_data(), None, None, None).unwrap();
    assert_eq!(start.save_path, Some(dir.join("Cli.d2s")));
    assert_eq!((start.difficulty, start.start_flags), (2, None));
    let game = Game::start(start);
    assert_eq!(
        game.joined(),
        Some(Joined {
            class: 2,
            name: b"Cli".to_vec(),
            difficulty: 2,
        })
    );
    // The name is taken once the file exists (a character the front end
    // would have written: the stub of a new Necromancer).
    let taken = NewCharacter {
        name: "Cli".into(),
        class: Class::Necromancer,
        hardcore: false,
        expansion: true,
    };
    write_stub(&dir, &taken, 1).unwrap();
    let err = play_start::resolve(&cli, &app_support::game_data(), None, None, None).unwrap_err();
    assert!(err.to_string().contains("already exists"), "{err}");
    // --save: the install's tables read the file; a save inside the
    // install is refused.
    let save = CliStart {
        save: Some(dir.join("Cli.d2s")),
        ..CliStart::default()
    };
    let start = play_start::resolve(&save, &app_support::game_data(), None, None, None).unwrap();
    assert_eq!(start.save_path, Some(dir.join("Cli.d2s")));
    assert!(
        start.origin.starts_with("character from "),
        "{}",
        start.origin
    );
    let err =
        play_start::resolve(&save, &app_support::game_data(), Some(&dir), None, None).unwrap_err();
    assert!(err.to_string().contains("inside the game install"), "{err}");
    // Neither flag: the default character, not saved.
    let start = play_start::resolve(
        &CliStart::default(),
        &app_support::game_data(),
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(start.save_path, None);
}

/// `play` after a game (Save and Exit, or the window closed): the front
/// end opens at the main menu (as in the game),
/// not at character select; Single Player lists the saved
/// character.
#[test]
fn after_a_game_the_front_end_opens_at_the_main_menu() {
    let dir = temp_dir("after");
    let c = NewCharacter {
        name: "Back".into(),
        class: Class::Assassin,
        hardcore: false,
        expansion: true,
    };
    write_stub(&dir, &c, 1).unwrap();
    let mut f = Front::open(&dir, Entry::MainMenu);
    assert_eq!(f.current(), MAIN_MENU);
    f.click_trigger(Trigger::SinglePlayer);
    assert_eq!(f.current(), CHAR_SELECT);
    assert!(f.texts().iter().any(|t| t == "Back"), "{:?}", f.texts());
}

/// The Configure Controls table as play bindings keeps command 56 (Esc →
/// Game Menu, not reassignable): Accept or a saved
/// `controls.toml` must not leave the game without its Esc key.
#[test]
fn controls_table_bindings_keep_esc_on_the_game_menu() {
    use d2_client::controls::original::BindingTable;
    use d2_client::controls::{Action as Act, Key};
    use d2_client::ui::front_end::screens::controls::table_to_bindings;
    let b = table_to_bindings(&BindingTable::defaults());
    assert_eq!(b.inputs(Act::GameMenu), &[Key::Escape]);
}

/// The paused pass on the play path: with
/// the Esc menu (UI state 9) open the
/// client runs no server frame, so the game's frame counter stands
/// still; closed again, it ticks on.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_esc_menu_pauses_the_single_player_game() {
    let dir = temp_dir("pause");
    let cfg = dir.join("cfg");
    std::env::set_var("D2RUST_CONFIG_DIR", &cfg);
    let mut f = Front::open(&dir, Entry::First);
    to_main_menu(&mut f);
    f.click_trigger(Trigger::SinglePlayer);
    create(&mut f, Class::Sorceress, "Pauser");
    let (g, c) = f.choice();
    let mut game = Game::start(resolve(&dir, g, &c).unwrap());
    let frame = |game: &Game| app_support::with(&game.server, |l| l.host().game.game.frame);
    let start = frame(&game);
    for _ in 0..5 {
        game.frame();
    }
    assert!(frame(&game) > start, "the game ticks");
    game.press(KeyCode::Escape);
    assert!(game.menu_open());
    let paused = frame(&game);
    for _ in 0..10 {
        game.frame();
    }
    assert_eq!(frame(&game), paused, "no server frame while paused");
    game.press(KeyCode::Escape);
    assert!(!game.menu_open());
    for _ in 0..5 {
        game.frame();
    }
    assert!(frame(&game) > paused, "the game ticks again");
}

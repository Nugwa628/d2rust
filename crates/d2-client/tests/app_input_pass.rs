//! The world clicks of the play app, headless: the app's own game wiring
//! (`add_game`, the classic UI) over the synthetic single-player game
//! with a primary window, mouse buttons pressed and released through
//! Bevy's `ButtonInput` and the window cursor, as `smoke_frontend.rs`
//! starts its game.
//!
//! - the world click runs on the loop pass that sees the button, also on
//!   a pass without a server tick (per pass, not per tick);
//! - a left release outside the frame ends the held button (the held
//!   button state := 0);
//! - a lost window focus runs the left release;
//! - the play bindings are the `original` preset: M toggles the message
//!   log, Alt shows items.
//!
//! Art, tables and the game are synthetic fixtures, not the user's files.

use crate::app_support;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowFocused};
use d2_client::app::play_start::{self, CliStart};
use d2_client::app::single_player::{self, DEFAULT_SEED};
use d2_client::assets::path::FileSource;
use d2_client::ui::edge;
use d2_client::ui::geom::Point;
use d2_client::world_view::WorldViewState;
use d2_server::seams::Clock;

use app_support::SharedLink;

/// Every DC6 an 8 × 8 cel of 32 frames (256 for fonts), every font table
/// 256 glyphs of width 6.
struct SyntheticArt {
    dc6: Vec<u8>,
    font: Vec<u8>,
}

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
        }
    }
}

impl FileSource for SyntheticArt {
    fn read_file(&self, name: &str) -> Option<Result<Vec<u8>, String>> {
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

struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// The `inventory.bin` `inv` rectangles of the 33 records (800 × 600:
/// records 16–32), as the live table has them.
fn inv_areas() -> Vec<d2_client::ui::original::InvArea> {
    use d2_client::ui::original::InvArea;
    (0..33)
        .map(|i| {
            if i >= 16 {
                InvArea {
                    left: 400,
                    right: 720,
                    top: 60,
                    bottom: 501,
                }
            } else {
                InvArea {
                    left: 320,
                    right: 640,
                    top: 0,
                    bottom: 441,
                }
            }
        })
        .collect()
}

/// The play app in the world, the local player joined.
struct Game {
    app: App,
    server: app_support::Server<StepClock>,
    ms: Arc<AtomicU32>,
}

impl Game {
    fn start() -> Self {
        // No saved `controls.toml`: the play default.
        let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("input-pass-{}", std::process::id()));
        std::env::set_var("D2RUST_CONFIG_DIR", &dir);
        let start = play_start::resolve(
            &CliStart::default(),
            &app_support::game_data(),
            None,
            None,
            None,
        )
        .unwrap();
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
            .init_resource::<ButtonInput<KeyCode>>()
            .add_message::<WindowFocused>();
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        d2_client::app::play::add_game(&mut app, Box::new(SharedLink(server.clone())), true)
            .unwrap();
        d2_client::app::play::send_create_game_flags(&mut app, &request, start.start_flags)
            .unwrap();
        app_support::live_tables(&mut app);
        d2_client::app::play::add_client_data(
            &mut app,
            single_player::client_drlg_source(&app_support::game_data()),
            single_player::client_level_rows(&app_support::game_data()),
        );
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
                inv_areas: Some(inv_areas()),
                expansion_installed: true,
                fonts: None,
                resist_penalties: None,
            },
        )
        .unwrap();
        let mut g = Self { app, server, ms };
        for _ in 0..200 {
            g.tick();
            if g.in_game() && g.app.world().resource::<WorldViewState>().last.is_some() {
                // Settle: a few more ticks so the view has drawn.
                for _ in 0..5 {
                    g.tick();
                }
                return g;
            }
        }
        panic!("the game never reached the world");
    }

    /// One loop pass with a server tick (the host clock advances 40 ms).
    fn tick(&mut self) {
        self.ms.fetch_add(40, Ordering::SeqCst);
        self.app.update();
    }

    /// One loop pass without a server tick.
    fn pass(&mut self) {
        self.app.update();
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

    fn server_ticks(&self) -> u64 {
        self.app
            .world()
            .resource::<d2_client::bridge::BridgeResource>()
            .0
            .world()
            .server_ticks
    }

    /// The server player's precise (16.16) position: the walk's own
    /// client receives nothing while walking.
    fn player_pos(&self) -> (u32, u32) {
        app_support::with(&self.server, |l| {
            let g = &mut l.host_mut().game;
            let (p, _) = single_player::local_player(g).unwrap();
            let d = g
                .events
                .action
                .hooks()
                .paths
                .as_ref()
                .and_then(|s| s.dynamic(p))
                .expect("the player's path");
            (d.precise_x, d.precise_y)
        })
    }

    fn left_held(&self) -> bool {
        self.app
            .world()
            .resource::<WorldViewState>()
            .click
            .left_held
    }

    /// The window cursor at frame point `at`, or outside the window.
    fn cursor(&mut self, at: Option<Point>) {
        let mut q = self
            .app
            .world_mut()
            .query_filtered::<&mut Window, With<PrimaryWindow>>();
        let mut w = q.single_mut(self.app.world_mut()).unwrap();
        let pos = at.map(|p| edge::frame_to_window(&w, p).unwrap().as_dvec2());
        w.set_physical_cursor_position(pos);
    }

    fn button(&mut self, down: bool) {
        let mut b = self
            .app
            .world_mut()
            .resource_mut::<ButtonInput<MouseButton>>();
        b.clear();
        if down {
            b.press(MouseButton::Left);
        } else {
            b.release(MouseButton::Left);
        }
    }

    fn clear_buttons(&mut self) {
        self.app
            .world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
    }
}

/// The ground left of the player, in the play area, no panel open.
const GROUND: Point = Point::new(300, 250);

// A press on a pass without a server tick is dispatched on that pass
// (before this change the world clicks waited for the next drawn tick).
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_press_is_dispatched_on_its_loop_pass() {
    let mut g = Game::start();
    g.cursor(Some(GROUND));
    g.pass();
    let ticks = g.server_ticks();
    g.button(true);
    g.pass();
    assert_eq!(g.server_ticks(), ticks, "no server tick on this pass");
    assert!(g.left_held(), "left down (kind 0) ran on the pass");
    g.clear_buttons();
    g.pass();
    assert!(g.left_held(), "still held");
    g.button(false);
    g.pass();
    assert!(!g.left_held(), "left up (kind 2) on its pass");
}

// The left button released with the cursor outside the window: the walk
// ends (before this change the release was dropped and the held repeat
// kept walking).
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_release_outside_the_frame_ends_the_held_button() {
    let mut g = Game::start();
    g.cursor(Some(GROUND));
    g.button(true);
    g.tick();
    assert!(g.left_held());
    g.clear_buttons();
    g.tick();
    g.cursor(None);
    g.tick();
    assert!(g.left_held(), "leaving the frame alone releases nothing");
    g.button(false);
    g.tick();
    assert!(!g.left_held(), "the release outside the frame ends it");
}

// The window loses the focus with the left button still down (alt-tab):
// the left release runs.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_lost_focus_releases_the_held_button() {
    let mut g = Game::start();
    g.cursor(Some(GROUND));
    g.button(true);
    g.tick();
    assert!(g.left_held());
    g.clear_buttons();
    let window = g
        .app
        .world_mut()
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(g.app.world())
        .unwrap();
    g.app.world_mut().write_message(WindowFocused {
        window,
        focused: false,
    });
    g.tick();
    assert!(!g.left_held(), "focus loss ran the left release");
}

impl Game {
    fn key(&mut self, k: KeyCode, down: bool) {
        let mut b = self.app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        b.clear();
        if down {
            b.press(k);
        } else {
            b.release(k);
        }
    }

    fn is_open(&mut self, ui: u8) -> bool {
        self.app
            .world_mut()
            .non_send_mut::<d2_client::world_view::WorldViewUi>()
            .original
            .as_ref()
            .unwrap()
            .is_open(ui)
    }
}

// The play app's default bindings are the `original` preset's: M (command 3)
// toggles ui 0x18, Alt (command 37) holds ui 0x0D open (Ctrl as Run
// held: `edge` tests). With the D2RUST
// dev preset none of these did anything.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_play_keys_are_the_original_preset() {
    let mut g = Game::start();
    assert!(!g.is_open(0x18));
    g.key(KeyCode::KeyM, true);
    g.pass();
    assert!(g.is_open(0x18), "M opened the message log state");
    g.key(KeyCode::KeyM, false);
    g.pass();
    g.key(KeyCode::AltLeft, true);
    g.pass();
    assert!(g.is_open(0x0D), "Alt shows items");
    g.key(KeyCode::AltLeft, false);
    g.pass();
    assert!(!g.is_open(0x0D));
}

// B029: with the inventory open (right panel, open mode 1) a press on the
// uncovered left half is a world click (only a click
// inside the panel's area is consumed).
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_press_beside_the_open_inventory_is_a_world_click() {
    let mut g = Game::start();
    g.key(KeyCode::KeyI, true);
    g.pass();
    g.key(KeyCode::KeyI, false);
    g.pass();
    assert!(g.is_open(1), "I opened the inventory");
    g.cursor(Some(GROUND));
    g.pass();
    g.button(true);
    g.pass();
    assert!(g.left_held(), "the press left of the panel walked");
    let start = g.player_pos();
    g.clear_buttons();
    for _ in 0..25 {
        g.tick();
    }
    assert_ne!(
        g.player_pos(),
        start,
        "the player moved with the panel open"
    );
}

// Control for B029: the same press with no panel open moves the player.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_press_with_no_panel_moves_the_player() {
    let mut g = Game::start();
    g.cursor(Some(GROUND));
    g.pass();
    g.button(true);
    g.pass();
    let start = g.player_pos();
    g.clear_buttons();
    for _ in 0..25 {
        g.tick();
    }
    assert_ne!(g.player_pos(), start, "the player moved");
}

/// A left press at `at` with the inventory open: did the world click run.
fn press_with_inventory(at: Point) -> bool {
    let mut g = Game::start();
    g.key(KeyCode::KeyI, true);
    g.pass();
    g.key(KeyCode::KeyI, false);
    g.pass();
    assert!(g.is_open(1));
    g.cursor(Some(at));
    g.pass();
    g.button(true);
    g.pass();
    g.left_held()
}

// B029 suspects: the panel's area edge. A press inside the inventory's
// area (x 400..719, y 60..501) is the panel's (no world click); the
// columns just outside it are the world's.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_inventory_area_edge_splits_panel_and_world() {
    assert!(press_with_inventory(Point::new(399, 250)), "x 399: world");
    assert!(!press_with_inventory(Point::new(400, 250)), "x 400: panel");
    assert!(!press_with_inventory(Point::new(719, 250)), "x 719: panel");
    assert!(press_with_inventory(Point::new(720, 250)), "x 720: world");
}

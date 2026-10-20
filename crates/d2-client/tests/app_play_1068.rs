//! The play path at the 16:9 frame (`d2-client play --res 1068x600`),
//! headless, wired as `app_play_640.rs` wires the 640 × 480 frame
//! (synthetic UI and tile files in a memory source, the game's tables from
//! `D2_GAME_DIR`): the world view composes a 1068 × 600 frame, the camera
//! is the 1068-wide camera of the UI's open mode, and the HUD and panels
//! keep the 800 × 600 layout, centred 134 px in.
//!
//! One test in its own binary: the play frame is a once-per-process
//! setting (`FrameSize::set_play`).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use d2_client::app::palette::{add_act_palettes, ActPalettes};
use d2_client::app::play::{
    add_client_data, add_game, add_preview, add_walk, predict_link, send_create_game_for,
};
use d2_client::app::single_player;
use d2_client::app::ui::{add_original_ui_with, UiParts};
use d2_client::assets::path::MemorySource;
use d2_client::bridge::link::{LinkError, Sent, ServerLink};
use d2_client::bridge::predict::Speeds;
use d2_client::bridge::{Pumped, SendQueue};
use d2_client::rules::camera::{FrameSize, OpenMode, ViewRect};
use d2_client::rules::unit_composite::code;
use d2_client::ui::layout::Screen;
use d2_client::ui::original::{FontMeasure, OriginalUi, UiConfig, CHARACTER_FONTS};
use d2_client::ui::{font_info, UiDraw};
use d2_client::world_view::tile_assets::TileAssets;
use d2_client::world_view::unit_assets::UnitLooks;
use d2_client::world_view::{WorldViewState, WorldViewUi};
use d2_server::seams::Clock;

use crate::app_support;

/// Records every C→S message sent through it.
struct RecLink<L> {
    inner: L,
    sent: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl<L: ServerLink> ServerLink for RecLink<L> {
    fn protocol_version(&self) -> u32 {
        self.inner.protocol_version()
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        self.inner.tick_rate()
    }
    fn send(&mut self, queue: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        self.sent.lock().unwrap().push(msg.to_vec());
        self.inner.send(queue, msg)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        self.inner.pump()
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        self.inner.receive()
    }
}

struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// A DT1 of one tile with one 32 × 32 RLE block, the
/// fixture of `app_play_preview.rs`.
fn dt1_bytes() -> Vec<u8> {
    let encoded = [0x00u8, 0x02, 0x0A, 0x0B];
    let mut d = Vec::new();
    d.extend_from_slice(&7u32.to_le_bytes());
    d.extend_from_slice(&6u32.to_le_bytes());
    d.extend_from_slice(&[0; 260]);
    d.extend_from_slice(&1u32.to_le_bytes());
    d.extend_from_slice(&276u32.to_le_bytes());
    let mut tile = vec![0u8; 96];
    tile[0x48..0x4C].copy_from_slice(&372u32.to_le_bytes());
    tile[0x50..0x54].copy_from_slice(&1u32.to_le_bytes());
    d.extend_from_slice(&tile);
    for v in [0u16, 0, 0] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.extend_from_slice(&[0, 0]);
    d.extend_from_slice(&0x1001u16.to_le_bytes());
    d.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
    d.extend_from_slice(&0u16.to_le_bytes());
    d.extend_from_slice(&20u32.to_le_bytes());
    d.extend_from_slice(&encoded);
    d
}

/// A DC6 of one direction with `frames` frames of 2 × 2 literal pixels.
fn dc6(frames: u32) -> Vec<u8> {
    let rows = [2u8, 1, 2, 0x80, 2, 3, 4, 0x80];
    let mut d = Vec::new();
    for v in [6i32, 1, 0] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.extend_from_slice(&[0xEE; 4]);
    d.extend_from_slice(&1u32.to_le_bytes());
    d.extend_from_slice(&frames.to_le_bytes());
    let mut at = d.len() + 4 * frames as usize;
    let mut body = Vec::new();
    for _ in 0..frames {
        d.extend_from_slice(&(at as u32).to_le_bytes());
        for v in [0u32, 2, 2, 0, 0, 0, 0, rows.len() as u32] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        body.extend_from_slice(&rows);
        body.extend_from_slice(&[0xEE; 3]);
        at += 32 + rows.len() + 3;
    }
    d.extend(body);
    d
}

/// COF bytes: one direction, one frame, one layer
/// (component 1, weapon class `hth`), animation rate 256.
fn cof_bytes() -> Vec<u8> {
    let mut v = vec![1, 1, 1, 20, 0, 0, 0, 0];
    for x in [-10i32, 10, -20, 0] {
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.extend_from_slice(&256u32.to_le_bytes());
    v.extend_from_slice(&[1, 0, 1, 0, 0]);
    v.extend_from_slice(b"hth\0");
    v.push(0);
    v.push(1);
    v
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

/// A `pal.pl2` of zeros with its 13 text colours.
fn pl2() -> Vec<u8> {
    vec![0; 1024 + 1714 * 256 + 13 * (3 + 256)]
}

/// Invented unit tokens: every player class is `OY`, mode 5 `TN`,
/// component 1 `TR`. `OYTRlitTNhth` is the one player component file
/// name read as a DC6, so a DC6 fixture
/// draws the player.
fn looks() -> UnitLooks {
    UnitLooks {
        player_tokens: vec![code(b"OY"); 7],
        player_modes: [b"DT", b"NU", b"WL", b"RN", b"GH", b"TN", b"TW"]
            .iter()
            .map(|m| code(*m))
            .collect(),
        components: vec![code(b"HD"), code(b"TR")],
        ..Default::default()
    }
}

/// Every file the play preview reads here.
fn files() -> MemorySource {
    let mut s = MemorySource::default();
    s.insert(r"DATA\GLOBAL\TILES\floor.dt1", dt1_bytes());
    s.insert(r"data\global\chars\OY\cof\OYTNhth.cof", cof_bytes());
    s.insert(r"data\global\chars\OY\TR\OYTRlitTNhth.dc6", dc6(1));
    let config = UiConfig {
        screen: Screen::R800,
        expansion_installed: false,
    };
    let ui = OriginalUi::new(config, None).unwrap();
    for name in ui.files().names() {
        s.insert(&format!("data\\global\\ui\\{name}.dc6"), dc6(64));
    }
    for id in 0..14 {
        let Some(f) = font_info(id) else { continue };
        s.insert(f.tbl_path, tbl());
        s.insert(f.dc6_path, dc6(256));
    }
    s
}

fn step(app: &mut App, ms: &AtomicU32, n: usize) {
    for _ in 0..n {
        app.update();
        ms.fetch_add(40, Ordering::SeqCst);
    }
}

/// The UI image draws of the last frame: (file name, frame, x, y, clip h).
fn images(app: &App) -> Vec<(String, u32, i32, i32, u16)> {
    let ui = app.world().non_send::<WorldViewUi>();
    let files = ui.original.as_ref().unwrap().files();
    let state = app.world().resource::<WorldViewState>();
    state
        .last_ui
        .iter()
        .filter_map(|d| match d {
            UiDraw::Image(i) => Some((
                files.name(i.image.file).unwrap().to_string(),
                i.image.frame,
                i.at.x,
                i.at.y,
                i.clip.h,
            )),
            UiDraw::Text(_) | UiDraw::Rect(_) | UiDraw::Line(_) => None,
        })
        .collect()
}

/// The clip spans (x0, x1) of the side bar cels, in draw order.
fn side_bars(app: &App) -> Vec<(i32, i32)> {
    let ui = app.world().non_send::<WorldViewUi>();
    let files = ui.original.as_ref().unwrap().files();
    let state = app.world().resource::<WorldViewState>();
    state
        .last_ui
        .iter()
        .filter_map(|d| match d {
            UiDraw::Image(i) if files.name(i.image.file) == Some("d2rust\\sidebar") => {
                Some((i.clip.x, i.clip.x + i32::from(i.clip.w)))
            }
            _ => None,
        })
        .collect()
}

/// The camera of the last drawn frame.
fn camera(app: &App) -> d2_client::rules::camera::Camera {
    let state = app.world().resource::<WorldViewState>();
    let cam = *state.camera.read().unwrap();
    cam.expect("a frame was drawn")
}

fn open_mode(app: &App) -> u8 {
    let ui = app.world().non_send::<WorldViewUi>();
    ui.original.as_ref().unwrap().open_mode().get()
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_play_path_runs_the_16_9_frame() {
    FrameSize::set_play(FrameSize::WIDE).unwrap();
    // The UI keeps the 800 × 600 layout; the frame places it 134 px in.
    assert_eq!(Screen::play(), Screen::WIDE);
    assert_eq!(FrameSize::WIDE.ui_left(), 134);
    let data = app_support::game_data();
    let character = single_player::new_character("sorceress", "Test").unwrap();
    let ms = Arc::new(AtomicU32::new(1000));
    let (link, _) = single_player::start_with(
        data.clone(),
        single_player::DEFAULT_SEED,
        character.clone(),
        StepClock(ms.clone()),
    )
    .unwrap();
    let link = RecLink {
        inner: link,
        sent: Arc::new(Mutex::new(Vec::new())),
    };
    let source = Arc::new(files());
    let mut app = App::new();
    app.insert_resource(d2_client::bridge::mirror::ScriptedClock(ms.clone()));
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .init_resource::<ButtonInput<MouseButton>>();
    let (link, tap) = predict_link(Box::new(link));
    add_game(&mut app, link, false).unwrap();
    send_create_game_for(&mut app, &character).unwrap();
    let levels = single_player::client_level_rows(&data);
    add_client_data(
        &mut app,
        single_player::client_drlg_source(&data),
        levels.clone(),
    );
    add_preview(
        &mut app,
        levels,
        TileAssets::new(Some(source.clone()), None),
    );
    add_act_palettes(
        &mut app,
        ActPalettes {
            pl2: std::array::from_fn(|_| pl2()),
            shown: None,
        },
    );
    let fonts = FontMeasure::load(source.as_ref(), &CHARACTER_FONTS).unwrap();
    add_original_ui_with(
        &mut app,
        UiParts {
            source: source.clone(),
            inv_areas: None,
            expansion_installed: false,
            fonts: Some(fonts),
            resist_penalties: Some(vec![0, 20, 50]),
        },
        looks(),
    )
    .unwrap();
    add_walk(&mut app, tap, Some(Speeds { walk: 6, run: 9 }));
    step(&mut app, &ms, 10);

    // The composed frame and the camera are 1068 × 600 (play area
    // H − 40; ±W / 4 per open mode).
    {
        let state = app.world().resource::<WorldViewState>();
        assert_eq!((state.cycle.width(), state.cycle.height()), (1068, 600));
        assert!(state.last.is_some(), "a frame was presented");
    }
    let cam = camera(&app);
    assert_eq!(cam.size, FrameSize::WIDE);
    assert_eq!(cam.view, ViewRect::new(FrameSize::WIDE, OpenMode::NONE));
    assert_eq!(cam.view.right - cam.view.left, 1068);

    // The control panel's 800-wide art sits centred: its first piece at
    // x 0 of the UI box, 134 in the frame.
    let ctrl: Vec<(u32, i32)> = images(&app)
        .into_iter()
        .filter(|(name, ..)| name == "panel\\800ctrlpnl7")
        .map(|(_, frame, x, ..)| (frame, x))
        .collect();
    assert!(ctrl.contains(&(0, 134)), "{ctrl:?}");

    // The strips beside it get the side bar (one cel of the bar's own
    // frame, made from the panel's art) over x 0..134 and 934..1068.
    assert_eq!(side_bars(&app), vec![(0, 134), (934, 1068)]);

    // The help screen dims the whole frame above the panel, not only the
    // centred 800 columns.
    let help = u32::from(d2_client::ui::original::help_screen::STATE);
    let set = |app: &mut App, mode: u32| {
        app.world_mut()
            .non_send_mut::<WorldViewUi>()
            .original
            .as_mut()
            .unwrap()
            .set_ui(help, mode, false)
            .unwrap();
    };
    set(&mut app, 0);
    step(&mut app, &ms, 2);
    let dims: Vec<(i32, i32, i32, i32)> = {
        let state = app.world().resource::<WorldViewState>();
        state
            .last_ui
            .iter()
            .filter_map(|d| match d {
                UiDraw::Rect(r) if (r.y0, r.y1) == (0, 600 - 0x31) => {
                    Some((r.x0, r.x1, r.color.into(), r.mode.into()))
                }
                _ => None,
            })
            .collect()
    };
    assert_eq!(dims, vec![(0, 1068, 0, 1)]);
    set(&mut app, 1);
    step(&mut app, &ms, 2);

    // The character panel (ui 2, left) opens at the frame's left edge:
    // the 800 × 600 quads (X0 = sx = 80, top H + sy − 224 = 316, bottom
    // H + sy − 48 = 492) start at column 0.
    app.world_mut()
        .non_send_mut::<WorldViewUi>()
        .original
        .as_mut()
        .unwrap()
        .set_ui(2, 0, false)
        .unwrap();
    step(&mut app, &ms, 2);
    let ui_open = |app: &App| {
        let ui = app.world().non_send::<WorldViewUi>();
        ui.original.as_ref().unwrap().is_open(2)
    };
    assert!(ui_open(&app));
    let quads: Vec<(u32, i32, i32)> = images(&app)
        .into_iter()
        .filter(|(name, frame, ..)| name == "panel\\invchar" && *frame < 4)
        .map(|(_, frame, x, y, _)| (frame, x, y))
        .collect();
    assert_eq!(
        quads,
        vec![(0, 0, 316), (1, 256, 316), (2, 0, 492), (3, 256, 492)]
    );
    // The open left panel moves the camera by half its 320 columns.
    let mode = open_mode(&app);
    assert_ne!(mode, 0);
    let cam = camera(&app);
    assert_eq!(
        cam.view,
        ViewRect::new(FrameSize::WIDE, OpenMode::new(mode).unwrap())
    );
    assert_eq!(cam.view.shift_x.abs(), 160);

    // The inventory (ui 1, right) opens with it, against the frame's right
    // edge: its quads end at column 1068. Both sides open: no camera move.
    app.world_mut()
        .non_send_mut::<WorldViewUi>()
        .original
        .as_mut()
        .unwrap()
        .set_ui(1, 0, false)
        .unwrap();
    step(&mut app, &ms, 2);
    let quads: Vec<(u32, i32, i32)> = images(&app)
        .into_iter()
        .filter(|(name, frame, ..)| name == "panel\\invchar" && *frame >= 4)
        .map(|(_, frame, x, y, _)| (frame, x, y))
        .collect();
    assert_eq!(
        quads,
        vec![(4, 748, 316), (5, 1004, 316), (6, 748, 492), (7, 1004, 492)]
    );
    assert_eq!(open_mode(&app), 3);
    assert_eq!(camera(&app).view.shift_x, 0);
}

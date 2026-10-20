//! The level-up buttons of the control panel, headless as `play` wires
//! it (the fixtures of `app_hud_e2e.rs`): unspent stat and skill points in
//! the model draw the glowing `Panel\Level` buttons, a click on one opens
//! the character panel / skill tree, and spending the points hides them.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use d2_client::app::hud::set_hud_tables;
use d2_client::app::palette::{add_act_palettes, ActPalettes};
use d2_client::app::play::{
    add_client_data, add_game, add_preview, add_walk, predict_link, send_create_game_for,
};
use d2_client::app::single_player::{self};
use d2_client::app::ui::{add_original_ui_with, UiParts};
use d2_client::assets::path::MemorySource;
use d2_client::bridge::link::{LinkError, Sent, ServerLink};
use d2_client::bridge::predict::Speeds;
use d2_client::bridge::{BridgeResource, Pumped, SendQueue};
use d2_client::rules::unit_composite::code;
use d2_client::ui::layout::Screen;
use d2_client::ui::original::hud::HudTables;
use d2_client::ui::original::{FontMeasure, OriginalUi, UiConfig, CHARACTER_FONTS};
use d2_client::ui::{font_info, Point, PointerButton, UiDraw, UiEvent};
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

fn click(app: &mut App, ms: &AtomicU32, x: i32, y: i32) {
    let at = Point::new(x, y);
    for e in [
        UiEvent::Press {
            button: PointerButton::Left,
            at,
        },
        UiEvent::Release {
            button: PointerButton::Left,
            at,
        },
    ] {
        app.world_mut()
            .non_send_mut::<WorldViewUi>()
            .queue
            .0
            .push(e);
    }
    step(app, ms, 2);
}

fn receive(app: &mut App, m: &[u8]) {
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .receive_chunk(m)
        .unwrap();
}

fn open(app: &App, ui: u8) -> bool {
    app.world()
        .non_send::<WorldViewUi>()
        .original
        .as_ref()
        .unwrap()
        .is_open(ui)
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn unspent_points_light_the_level_buttons_and_open_the_panels() {
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
    let sent = Arc::new(Mutex::new(Vec::new()));
    let link = RecLink {
        inner: link,
        sent: sent.clone(),
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
    set_hud_tables(&mut app, HudTables::default());
    add_walk(&mut app, tap, Some(Speeds { walk: 6, run: 9 }));
    step(&mut app, &ms, 10);

    // No points: both buttons are the closed frame 2 (800 × 600: x 206
    // and 563, y 592).
    let img = images(&app);
    let lvl = |img: &[(String, u32, i32, i32, u16)], x: i32| {
        img.iter()
            .find(|d| d.0 == "panel\\level" && d.2 == x && d.3 == 592)
            .map(|d| d.1)
    };
    assert_eq!(lvl(&img, 206), Some(2), "{img:?}");
    assert_eq!(lvl(&img, 563), Some(2), "{img:?}");

    // The server's level-up stat messages: 5 stat points, 1 skill point.
    receive(&mut app, &[0x1D, 4, 5]);
    receive(&mut app, &[0x1D, 5, 1]);
    step(&mut app, &ms, 2);
    let img = images(&app);
    assert_eq!(lvl(&img, 206), Some(0), "new stats lit: {img:?}");
    assert_eq!(lvl(&img, 563), Some(0), "new skills lit: {img:?}");

    // A click on the new-stats button opens the character panel (state 2),
    // on the new-skills button the skill tree (state 4).
    assert!(!open(&app, 2) && !open(&app, 4));
    click(&mut app, &ms, 220, 575);
    assert!(open(&app, 2), "character panel open");
    click(&mut app, &ms, 580, 575);
    assert!(open(&app, 4), "skill tree open");

    // Spent: the buttons go back to the closed frame.
    receive(&mut app, &[0x1D, 4, 0]);
    receive(&mut app, &[0x1D, 5, 0]);
    step(&mut app, &ms, 2);
    let img = images(&app);
    assert_eq!(lvl(&img, 206), Some(2), "{img:?}");
    assert_eq!(lvl(&img, 563), Some(2), "{img:?}");
}

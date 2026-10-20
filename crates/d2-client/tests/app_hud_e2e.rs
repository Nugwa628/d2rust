//! The play HUD headless, wired as `d2-client play --new` wires it (the
//! fixtures of `app_play_e2e.rs`: synthetic files in a memory source,
//! never a game file):
//!
//! - the control panel draws the life and mana globes filled from the
//!   model's stats, the stamina bar, the run button and the left skill's
//!   icon every frame;
//! - a click on the run button toggles the walk's run lock and the button
//!   then shows running;
//! - a click on the left skill button opens state 3 (skill select); a
//!   click on a skill there sends C→S 0x3C and closes it; S→C 0x23 then
//!   changes the button's icon.

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
use d2_client::world_view::walk::PreviewWalk;
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

/// The UI rectangle draws of the last frame.
fn rect_draws(app: &App) -> Vec<d2_client::ui::draw::RectRequest> {
    let state = app.world().resource::<WorldViewState>();
    state
        .last_ui
        .iter()
        .filter_map(|d| match d {
            UiDraw::Rect(r) => Some(*r),
            _ => None,
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

fn stat(id: u8, v: u32) -> Vec<u8> {
    let mut m = vec![0x1F, id];
    m.extend_from_slice(&v.to_le_bytes());
    m
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_play_hud_draws_model_values_and_takes_clicks() {
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
    // Two synthetic `skills` rows (skills 0 and 1).
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .set_skill_rows(vec![d2_client::bridge::world::SkillRow::default(); 2]);
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
    // Synthetic tables: skill 0 a general skill (icon cel 7), skill 1 a
    // sorceress skill (cel 9); MaxLvl 99, level L needs 500 L.
    let mut tables = HudTables::default();
    tables.icons.insert(0, (0xFF, 7));
    tables.icons.insert(1, (1, 9));
    tables.experience = std::iter::once([99; 7])
        .chain((0..100u32).map(|l| [500 * l; 7]))
        .collect();
    set_hud_tables(&mut app, tables);
    add_walk(&mut app, tap, Some(Speeds { walk: 6, run: 9 }));

    step(&mut app, &ms, 10);
    let guid = app
        .world()
        .resource::<BridgeResource>()
        .0
        .world()
        .local()
        .expect("local player")
        .key
        .guid;
    // Synthetic S→C: life 50 / 100, mana 100 / 100, stamina 40 / 80
    // (×256); skills 0 and 1 at level 1 (0x94), skill 0 left (0x23).
    let mut msgs = Vec::new();
    for (id, v) in [(7, 100), (6, 50), (9, 100), (8, 100), (11, 80), (10, 40)] {
        msgs.extend(stat(id, v << 8));
    }
    msgs.extend_from_slice(&[0x94, 2]);
    msgs.extend_from_slice(&guid.to_le_bytes());
    msgs.extend_from_slice(&[0, 0, 1, 1, 0, 1]);
    msgs.extend_from_slice(&[0x23, 0]);
    msgs.extend_from_slice(&guid.to_le_bytes());
    msgs.extend_from_slice(&[1, 0, 0]);
    msgs.extend_from_slice(&u32::MAX.to_le_bytes());
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .receive_chunk(&msgs)
        .unwrap();
    // Past the globe smoothing (at most 15 updates).
    step(&mut app, &ms, 20);

    // 1. The overlays, bound to the model.
    let img = images(&app);
    let has = |name: &str, frame: u32, x: i32, y: i32, h: u16| {
        img.iter().any(|d| *d == (name.to_string(), frame, x, y, h))
    };
    assert!(
        has("panel\\hlthmana", 0, 29, 587, 40),
        "life half full: {img:?}"
    );
    assert!(
        has("panel\\hlthmana", 1, 689, 587, 80),
        "mana full: {img:?}"
    );
    assert!(has("panel\\overlap", 0, 28, 595, 600), "{img:?}");
    // Stamina 40 / 80: 51 pixels wide, gold. The bar is the rectangle
    // fill (x, y, w, 18, colour, mode 2; as in a
    // known Pandemonium Fortress scene), not a
    // `hudfill` tile: the old expectation predates that change.
    let rects = rect_draws(&app);
    assert!(
        rects
            .iter()
            .any(|r| (r.x0, r.y0, r.x1, r.y1, r.mode) == (273, 573, 273 + 51, 573 + 18, 2)),
        "stamina bar: {rects:?}"
    );
    assert!(
        has("panel\\runbutton", 0, 255, 590, 600),
        "walking: {img:?}"
    );
    assert!(
        has("spells\\skillicon", 7, 117, 600, 600),
        "left skill icon: {img:?}"
    );

    // 2. The run button toggles the walk's run lock.
    assert!(!app.world().resource::<PreviewWalk>().run.run_lock);
    click(&mut app, &ms, 260, 585);
    step(&mut app, &ms, 1);
    assert!(app.world().resource::<PreviewWalk>().run.run_lock, "run on");
    assert!(has_now(&app, "panel\\runbutton", 2), "{:?}", images(&app));

    // 3. The left skill button opens the skill select (state 3).
    click(&mut app, &ms, 130, 580);
    let open = |app: &App| {
        app.world()
            .non_send::<WorldViewUi>()
            .original
            .as_ref()
            .unwrap()
            .is_open(3)
    };
    assert!(open(&app), "state 3 open");
    let img = images(&app);
    assert!(
        img.iter().any(|d| d.0 == "spells\\soskillicon" && d.1 == 9),
        "skill 1 listed: {img:?}"
    );
    // Skill 1 is the second icon of the row (x 165, bottom y 552).
    sent.lock().unwrap().clear();
    click(&mut app, &ms, 170, 530);
    assert!(!open(&app), "state 3 closed");
    let msgs = sent.lock().unwrap().clone();
    let mut want = vec![0x3C];
    want.extend_from_slice(&(1u32 | 0x8000_0000).to_le_bytes());
    want.extend_from_slice(&u32::MAX.to_le_bytes());
    assert!(msgs.contains(&want), "C→S 0x3C sent: {msgs:?}");

    // 4. The server's 0x23 (skill 1 left) changes the button.
    let mut m = vec![0x23, 0];
    m.extend_from_slice(&guid.to_le_bytes());
    m.extend_from_slice(&[1, 1, 0]);
    m.extend_from_slice(&u32::MAX.to_le_bytes());
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .receive_chunk(&m)
        .unwrap();
    step(&mut app, &ms, 2);
    assert!(
        has_now(&app, "spells\\soskillicon", 9),
        "the left button shows skill 1: {:?}",
        images(&app)
    );
}

fn has_now(app: &App, name: &str, frame: u32) -> bool {
    images(app).iter().any(|d| d.0 == name && d.1 == frame)
}

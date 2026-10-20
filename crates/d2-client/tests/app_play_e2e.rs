//! The whole play preview headless, wired as `d2-client play --new`
//! wires it: a new character (D3) joins the synthetic single-player game
//! through the walk recorder ([`predict_link`]); the map, the units, the
//! original UI with its fonts and text colours, and the walk prediction
//! (D2) are on. Every file is a synthetic fixture in a memory source (a
//! one-tile DT1, an invented player token whose torso file is a DC6,
//! panel and glyph DC6s, font tables, zero act palettes), never a game
//! file.
//!
//! - the frame draws map tiles and the player's unit;
//! - a world click sends a walk and the predicted position moves;
//! - opening the character panel draws the character's name.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use bevy::prelude::*;
use d2_client::app::palette::{add_act_palettes, ActPalettes};
use d2_client::app::play::{
    add_client_data, add_game, add_preview, add_walk, predict_link, send_create_game_for,
};
use d2_client::app::single_player::{self};
use d2_client::app::ui::{add_original_ui_with, UiParts};
use d2_client::assets::path::MemorySource;
use d2_client::bridge::predict::Speeds;
use d2_client::bridge::BridgeResource;
use d2_client::controls::Action;
use d2_client::rules::unit_composite::code;
use d2_client::scene::ItemTag;
use d2_client::ui::layout::Screen;
use d2_client::ui::original::{FontMeasure, OriginalUi, UiConfig, CHARACTER_FONTS};
use d2_client::ui::{font_info, ActionId, Point, PointerButton, UiDraw, UiEvent};
use d2_client::world_view::tile_assets::TileAssets;
use d2_client::world_view::unit_assets::UnitLooks;
use d2_client::world_view::walk::PreviewWalk;
use d2_client::world_view::{WorldViewState, WorldViewUi};
use d2_server::seams::Clock;

use crate::app_support;

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

fn queue(app: &mut App, e: UiEvent) {
    app.world_mut()
        .non_send_mut::<WorldViewUi>()
        .queue
        .0
        .push(e);
}

/// The frame of the current tick was drawn (no build error).
fn drawn_now(app: &App) -> bool {
    let tick = app
        .world()
        .resource::<BridgeResource>()
        .0
        .world()
        .server_ticks;
    let state = app.world().resource::<WorldViewState>();
    state.last.is_some_and(|l| l.server_tick == tick)
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_play_preview_draws_walks_and_opens_the_character_panel() {
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
    app_support::live_tables(&mut app);
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
            // Synthetic `difficultylevels` `ResistPenalty` (the game is an
            // expansion game, which needs them).
            resist_penalties: Some(vec![0, 20, 50]),
        },
        looks(),
    )
    .unwrap();
    // Synthetic fixture: charstats-shaped speeds (walk 6, run 9).
    add_walk(&mut app, tap, Some(Speeds { walk: 6, run: 9 }));

    step(&mut app, &ms, 10);

    // 1. Tiles and the player's unit.
    let guid = {
        let w = app.world().resource::<BridgeResource>().0.world();
        assert!(w.local_room().is_some(), "joined into the town room");
        w.local().expect("local player").key.guid
    };
    assert!(drawn_now(&app));
    let state = app.world().resource::<WorldViewState>();
    assert!(
        state
            .last_tags
            .iter()
            .any(|t| matches!(t, ItemTag::Tile { .. })),
        "map tiles drawn: {:?}",
        state.last
    );
    assert!(
        state.last_tags.contains(&ItemTag::Unit(guid)),
        "the player's unit drawn: {:?}",
        state.last
    );

    // 2. A click on the ground: a walk, and the prediction moves.
    // The synthetic join sends no skill list (no `skills` rows), so the
    // click would have no left skill: a synthetic S→C 0x94
    // (skill 0 at level 1) and 0x23 (skill 0 as the left skill, native).
    let mut msgs = vec![0x94, 1];
    msgs.extend_from_slice(&guid.to_le_bytes());
    msgs.extend_from_slice(&[0, 0, 1]);
    msgs.extend_from_slice(&[0x23, 0]);
    msgs.extend_from_slice(&guid.to_le_bytes());
    msgs.extend_from_slice(&[1, 0, 0]);
    msgs.extend_from_slice(&u32::MAX.to_le_bytes());
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .receive_chunk(&msgs)
        .unwrap();
    let before = app.world().resource::<PreviewWalk>().predict.position();
    assert!(before.is_some(), "the prediction starts at the player");
    let at = Point::new(560, 200);
    queue(
        &mut app,
        UiEvent::Press {
            button: PointerButton::Left,
            at,
        },
    );
    queue(
        &mut app,
        UiEvent::Release {
            button: PointerButton::Left,
            at,
        },
    );
    step(&mut app, &ms, 2);
    let walk = app.world().resource::<PreviewWalk>();
    assert!(
        walk.predict.walking().is_some(),
        "the click sent a walk (C→S 0x01)"
    );
    step(&mut app, &ms, 3);
    let after = app.world().resource::<PreviewWalk>().predict.position();
    assert_ne!(after, before, "the predicted position moved");
    assert!(drawn_now(&app), "frames still draw while walking");

    // 3. The character panel (C): the name line in its font.
    let toggle = ActionId(Action::ToggleCharacter.index() as u16);
    queue(&mut app, UiEvent::Action(toggle));
    step(&mut app, &ms, 2);
    assert!(app
        .world()
        .non_send::<WorldViewUi>()
        .original
        .as_ref()
        .unwrap()
        .is_open(2));
    assert!(drawn_now(&app), "the panel frame draws");
    let state = app.world().resource::<WorldViewState>();
    let name: Vec<u16> = "Test".encode_utf16().collect();
    assert!(
        state
            .last_ui
            .iter()
            .any(|d| matches!(d, UiDraw::Text(t) if t.text == name)),
        "the name text is drawn"
    );
}

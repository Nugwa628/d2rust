//! The position check's visibility predicate in the play wiring:
//! the play app gives the bridge the world
//! view's predicate (`app::visibility::add_visibility`), so a walk whose
//! S→C 0x96 point differs from the client's cell on both axes is checked
//! through rules 1–5 instead of being refused. The play preview runs
//! headless as `app_play_e2e.rs` wires it; every file is a synthetic
//! fixture (that test's), never a game file.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use d2_client::app::palette::{add_act_palettes, ActPalettes};
use d2_client::app::play::{
    add_client_data, add_game, add_preview, add_walk, predict_link, send_create_game_for,
};
use d2_client::app::single_player::{self};
use d2_client::app::ui::{add_original_ui_with, UiParts};
use d2_client::app::visibility::add_visibility;
use d2_client::assets::path::MemorySource;
use d2_client::bridge::predict::Speeds;
use d2_client::bridge::world::VisibleFn;
use d2_client::bridge::BridgeResource;
use d2_client::rules::unit_composite::code;
use d2_client::scene::ItemTag;
use d2_client::ui::layout::Screen;
use d2_client::ui::original::{FontMeasure, OriginalUi, UiConfig, CHARACTER_FONTS};
use d2_client::ui::{font_info, Point, PointerButton, UiEvent};
use d2_client::world_view::tile_assets::TileAssets;
use d2_client::world_view::unit_assets::UnitLooks;
use d2_client::world_view::walk::PreviewWalk;
use d2_client::world_view::{WorldViewState, WorldViewUi};
use d2_data::bin::BinTable;
use d2_data::fixup::records::stat_ops;
use d2_data::tables::{Itemstatcost, Record};
use d2_server::seams::Clock;
use d2_sim::stats::{ClassStats, StatData, StatLists, StatTable};

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

/// A synthetic `itemstatcost` (`app_stamina.rs`'s): 359 stats, the
/// first 16 saved.
fn stat_data() -> Arc<StatData> {
    let (n, size) = (359, Itemstatcost::SIZE);
    let mut records = vec![0u8; n * size];
    for (s, r) in records.chunks_mut(size).enumerate() {
        for o in [0x32, 0x48, 0x4A, 0x56, 0x58, 0x5A, 0x5C] {
            r[o..o + 2].copy_from_slice(&0xFFFFu16.to_le_bytes());
        }
        r[0..2].copy_from_slice(&(s as u16).to_le_bytes());
        if s < 16 {
            r[5] |= 0x10;
        }
    }
    let mut t = BinTable {
        name: "itemstatcost".into(),
        source: "synthetic".into(),
        count: n,
        record_size: size,
        records,
    };
    stat_ops(&mut t);
    Arc::new(StatData {
        stats: StatTable::from_fixed(&t).expect("itemstatcost"),
        classes: vec![ClassStats::default(); 7],
        ..StatData::default()
    })
}

/// The synthetic new character has no life (no `itemstatcost` /
/// `charstats`), so the server's vitals sync (its
/// step 1: max life ≤ 0) sends nothing. Synthetic fill: 100 life and
/// stamina, so its 0x96 WalkVerify reaches the client.
fn give_life(server: &Server) {
    with(server, |l| {
        let sim = &mut l.host_mut().game;
        let (p, g) = single_player::local_player(sim).expect("joined");
        let sys = &mut sim.events.action.sys;
        sys.stats = StatLists::new(stat_data());
        let ty = sys.units.get(p).unwrap().ty;
        sys.stats
            .alloc_extended(&mut sys.hooks, p, ty, g, 1, 0, None);
        for (s, v) in [
            (12u16, 1),
            (7, 100 << 8),
            (6, 100 << 8),
            (11, 100 << 8),
            (10, 100 << 8),
        ] {
            sys.stats.unit_set(&mut sys.hooks, p, s, v, 0);
        }
    });
}

type Server = app_support::Server<StepClock>;

fn with<R: Send + 'static>(
    server: &Server,
    f: impl FnOnce(&mut single_player::Link<StepClock>) -> R + Send + 'static,
) -> R {
    app_support::with(server, f)
}

/// Invented unit tokens: every player class is `OY`, component 1 `TR`,
/// and every player mode but death reads the mode token `TN`, so the one
/// DC6 torso file `OYTRlitTNhth` draws the
/// player standing, walking and running: the predicate finds a COF and a
/// torso cel in each mode.
fn looks() -> UnitLooks {
    UnitLooks {
        player_tokens: vec![code(b"OY"); 7],
        player_modes: [b"DT", b"TN", b"TN", b"TN", b"TN", b"TN", b"TN"]
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

fn click(app: &mut App, at: Point) {
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
        queue(app, e);
    }
}

/// The play preview as `play --new` wires it on synthetic data, with the
/// unit art of [`files`] and the play app's visibility predicate
/// (`add_visibility`, as `app::play::run` installs it).
fn play_app(ms: &Arc<AtomicU32>) -> (App, Server) {
    let data = app_support::game_data();
    let character = single_player::new_character("sorceress", "Test").unwrap();
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
    let server: Server = Arc::new(Mutex::new(link));
    let (link, tap) = predict_link(Box::new(app_support::SharedLink(server.clone())));
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
            resist_penalties: Some(vec![0, 20, 50]),
        },
        looks(),
    )
    .unwrap();
    // Synthetic fixture: charstats-shaped speeds (walk 6, run 9).
    add_walk(&mut app, tap, Some(Speeds { walk: 6, run: 9 }));
    add_visibility(&mut app);
    (app, server)
}

/// The synthetic S→C 0x94 (skill 0 at level 1) and 0x23 (skill 0 as the
/// left skill) of `app_play_e2e.rs`: the synthetic join sends no skill
/// list, and a ground click needs a left skill.
fn left_skill(app: &mut App, guid: u32) {
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
}

/// Walks the local player diagonally (both sub-tile axes)
/// and returns the refused S→C 0x96
/// and how often the position check asked the predicate.
fn diagonal_walk() -> (Vec<String>, usize) {
    let ms = Arc::new(AtomicU32::new(1000));
    let (mut app, server) = play_app(&ms);
    step(&mut app, &ms, 10);
    give_life(&server);
    let guid = {
        let w = app.world().resource::<BridgeResource>().0.world();
        assert!(w.local_room().is_some(), "joined into the town room");
        w.local().expect("local player").key.guid
    };
    assert!(
        app.world()
            .resource::<WorldViewState>()
            .last_tags
            .contains(&ItemTag::Unit(guid)),
        "the player is drawn"
    );
    // Count the predicate's calls around the installed one (it still
    // answers).
    let calls = Arc::new(AtomicUsize::new(0));
    {
        let mut bridge = app.world_mut().resource_mut::<BridgeResource>();
        if let Some(real) = bridge.0.inputs().visible.clone() {
            let n = calls.clone();
            bridge.0.set_visibility(Some(VisibleFn::new(move |u, a, b| {
                n.fetch_add(1, Ordering::SeqCst);
                real.visible(u, a, b)
            })));
        }
    }
    left_skill(&mut app, guid);
    let cell = |app: &App| {
        let (x, y) = app
            .world()
            .resource::<PreviewWalk>()
            .predict
            .position()
            .expect("the prediction follows the player");
        (x >> 16, y >> 16)
    };
    let start = cell(&app);
    // The player draws at screen (400, 292): clicks below it walk +x and
    // +y together, inside the synthetic town room.
    for at in [
        Point::new(400, 360),
        Point::new(400, 360),
        Point::new(300, 340),
    ] {
        click(&mut app, at);
        step(&mut app, &ms, 40);
    }
    let end = cell(&app);
    assert!(
        end.0 != start.0 && end.1 != start.1,
        "the walk moved both axes: {start:?} → {end:?}"
    );
    // The position check compares with the client's own path cell,
    // which the server's 0x96
    // of a diagonal walk mostly shares on one axis: a server point one
    // sub-tile off it on both axes (inside the tolerance) reaches rule 6
    // and its predicate.
    let off = d2_sim::path::walk::messages::walk_verify(
        100,
        (end.0 + 1) as u16,
        (end.1 + 1) as u16,
        0,
        0,
    );
    {
        let mut bridge = app.world_mut().resource_mut::<BridgeResource>();
        bridge.0.receive_chunk(&off).unwrap();
        bridge.0.update_pass();
    }
    let refused = app
        .world()
        .resource::<BridgeResource>()
        .0
        .log()
        .rejected
        .iter()
        .filter(|r| r.id == 0x96)
        .map(|r| format!("{:?}", r.error))
        .collect();
    (refused, calls.load(Ordering::SeqCst))
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_diagonal_walk_checks_0x96_through_the_view_predicate() {
    let (refused, calls) = diagonal_walk();
    assert!(refused.is_empty(), "S→C 0x96 refused: {refused:?}");
    assert!(
        calls > 0,
        "a 0x96 point off the client cell on both axes asked the predicate"
    );
}

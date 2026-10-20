//! The app's own single-player game run headless for 3000 server ticks
//! (a known gap): on live data the server thread
//! panicked "live DRLG room" ~104 ticks in, when tick step 10 freed the
//! first never-visited level. The synthetic game
//! has the same shape: Cold Plains' first room is streamed at game
//! creation and never visited, so tick step 9 deactivates the room and
//! step 10 frees the level while the player stands in the town. The
//! server must keep ticking, the client must stay in game, and the freed
//! level must hold no rooms.
//!
//! The synthetic data has no two adjacent levels, so the walk across a
//! level border on foot is covered on the DRLG side by
//! `d2-sim` `drlg::tests::walk` (3000 ticks across two levels and back)
//! and on the user's files by a local run.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use d2_client::app::play::{add_client_data, add_game, send_create_game};
use d2_client::app::single_player::{self};
use d2_client::bridge::BridgeResource;
use d2_client::rules::OpenMode;
use d2_client::world_view::WorldViewState;
use d2_server::seams::Clock;

use crate::app_support;
use app_support::SharedLink;

/// Server ticks the game runs.
const TICKS: i32 = 3000;

/// The host clock, advanced by the test.
struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_app_game_runs_3000_ticks_while_an_unvisited_level_is_freed() {
    let data = app_support::game_data();
    let ms = Arc::new(AtomicU32::new(1000));
    let (link, _) = single_player::start(
        data.clone(),
        single_player::DEFAULT_SEED,
        StepClock(ms.clone()),
    )
    .unwrap();
    let server = Arc::new(Mutex::new(link));
    let mut app = App::new();
    app.insert_resource(d2_client::bridge::mirror::ScriptedClock(ms.clone()));
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .init_resource::<ButtonInput<MouseButton>>();
    add_game(&mut app, Box::new(SharedLink(server.clone())), false).unwrap();
    send_create_game(&mut app).unwrap();
    add_client_data(
        &mut app,
        single_player::client_drlg_source(&data),
        single_player::client_level_rows(&data),
    );
    app_support::live_tables(&mut app);
    app.world_mut()
        .resource_mut::<WorldViewState>()
        .feed
        .set_ui_open_mode(OpenMode::new(0).unwrap());
    app.update();

    let frame = |server: &app_support::Server<StepClock>| {
        app_support::with(server, |l| l.host_mut().game.game.frame)
    };
    let cold_plains_rooms = |server: &app_support::Server<StepClock>| {
        app_support::with(server, |l| {
            let hooks = l.host_mut().game.events.action.hooks();
            let d = hooks.drlg.dungeon.acts[0].as_ref().expect("act 0 DRLG");
            let l = d
                .find_level(single_player::COLD_PLAINS)
                .expect("Cold Plains allocated");
            d.level_rooms(l).len()
        })
    };
    // Game creation streamed Cold Plains' first room.
    let mut steps = 0;
    while app_support::local_player(&server).is_none() {
        ms.fetch_add(40, Ordering::SeqCst);
        app.update();
        steps += 1;
        assert!(steps < 20, "joined within 20 frames");
    }
    assert!(cold_plains_rooms(&server) > 0);
    let start = frame(&server);
    let mut last = start;
    while last - start < TICKS {
        ms.fetch_add(40, Ordering::SeqCst);
        app.update();
        // A dead server thread fails `with` (the link's call errors).
        let now = frame(&server);
        assert!(now >= last, "server frames run forward");
        last = now;
        steps += 1;
        assert!(steps < 4 * TICKS as usize, "server ticks advance");
    }
    let b = &app.world().resource::<BridgeResource>().0;
    assert!(b.log().rejected.is_empty(), "{:?}", b.log().rejected);
    let w = b.world();
    assert!(w.in_game);
    assert_eq!(w.player_level(), Some(single_player::ACT1_TOWN as u16));
    // Step 10 freed the never-visited level.
    assert_eq!(cold_plains_rooms(&server), 0);
}

/// The server player's sub-tile.
fn server_player(server: &app_support::Server<StepClock>) -> Option<(i32, i32)> {
    app_support::with(server, |l| {
        let (p, _) = single_player::local_player(&l.host().game)?;
        let game = &mut l.host_mut().game;
        game.game.lists.unit(p)?.room()?;
        Some(game.events.action.hooks().path_position(p))
    })
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn walking_east_out_of_the_town_the_map_follows_into_the_blood_moor() {
    use d2_client::app::play::{add_walk, predict_link};
    use d2_client::bridge::predict::Speeds;
    let data = app_support::game_data();
    let ms = Arc::new(AtomicU32::new(1000));
    let (link, _) = single_player::start(
        data.clone(),
        single_player::DEFAULT_SEED,
        StepClock(ms.clone()),
    )
    .unwrap();
    let server = Arc::new(Mutex::new(link));
    let mut app = App::new();
    app.insert_resource(d2_client::bridge::mirror::ScriptedClock(ms.clone()));
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .init_resource::<ButtonInput<MouseButton>>();
    let (link, tap) = predict_link(Box::new(SharedLink(server.clone())));
    add_game(&mut app, link, false).unwrap();
    add_walk(&mut app, tap, Some(Speeds { walk: 6, run: 9 }));
    send_create_game(&mut app).unwrap();
    add_client_data(
        &mut app,
        single_player::client_drlg_source(&data),
        single_player::client_level_rows(&data),
    );
    app_support::live_tables(&mut app);
    app.update();
    let mut steps = 0;
    let mut step = |app: &mut App| {
        ms.fetch_add(40, Ordering::SeqCst);
        app.update();
        steps += 1;
        assert!(steps < 2000, "the walk ends");
    };
    while app_support::local_player(&server).is_none() {
        step(&mut app);
    }
    for _ in 0..10 {
        step(&mut app);
    }
    let level = |app: &App| {
        app.world()
            .resource::<BridgeResource>()
            .0
            .world()
            .player_level()
    };
    assert_eq!(level(&app), Some(single_player::ACT1_TOWN as u16));
    let start = server_player(&server).expect("player placed");
    // The install's town is the level rect the player stands in; the
    // route to the Blood Moor runs over the server's collision
    // (`test_fixtures::host::route`), each leg a walk intent (C→S 0x03).
    let rect = app_support::with(&server, |l| {
        let h = l.host_mut().game.events.action.hooks();
        let d = h.drlg.dungeon.acts[0].as_ref().expect("Act I");
        let town = d.find_level(single_player::ACT1_TOWN).expect("the town");
        d.level(town).rect
    });
    assert!(
        (rect.x * 5..(rect.x + rect.w) * 5).contains(&start.0),
        "{start:?} is in the town {rect:?}"
    );
    app_support::walk_into(
        &mut app,
        &server,
        &ms,
        single_player::ACT1_TOWN,
        single_player::BLOOD_MOOR,
    );
    for _ in 0..10 {
        step(&mut app);
    }
    // The server player walked into the Blood Moor; the client's player
    // is there too (the predicted walk recached its room).
    assert_eq!(level(&app), Some(single_player::BLOOD_MOOR as u16));
    let b = &app.world().resource::<BridgeResource>().0;
    assert!(b.log().rejected.is_empty(), "{:?}", b.log().rejected);
}

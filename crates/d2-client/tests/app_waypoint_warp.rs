//! (q-waypoint-travel) A waypoint warp; see also: a level warp in the app's own synthetic game, headless (task
//! `q-warps`): the Blood Moor's cave entrance
//! is a tile unit (S→C 0x09); the client's interact sender puts C→S 0x13
//! (unit type 5) on the wire; the server runs the walk into the warp,
//! places the player in the Den of Evil and sends the new
//! level's 0x07 / 0x15 / 0x0D; the client's model follows to the new
//! level (that test, `app_level_warp.rs`; this one drives
//! the waypoint path, C→S 0x13 on the object then 0x49, no tile).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use d2_client::app::play::{add_client_data, add_game, add_walk, predict_link, send_create_game};
use d2_client::app::single_player::{self};
use d2_client::bridge::predict::Speeds;
use d2_client::bridge::world::UnitKey;
use d2_client::bridge::BridgeResource;
use d2_server::seams::Clock;

use crate::app_support;
use app_support::SharedLink;

/// The host clock, advanced by the test.
struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// The server player's level id.
fn server_level(server: &app_support::Server<StepClock>) -> Option<u32> {
    app_support::with(server, |l| {
        let (p, _) = single_player::local_player(&l.host().game)?;
        let g = &mut l.host_mut().game;
        let room = g.game.lists.unit(p)?.room()?;
        g.events.action.hooks().drlg.level_id(&g.game, room)
    })
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn taking_a_waypoint_to_cold_plains_moves_the_player() {
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
        assert!(steps < 3000, "the warp happens");
    };
    while app_support::local_player(&server).is_none() {
        step(&mut app);
    }
    for _ in 0..30 {
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
    // A new character knows only the town waypoint (B034): the character
    // learns Cold Plains' (stands in for activating it there).
    let map = app_support::waypoints().map.clone();
    app_support::with(&server, move |l| {
        let (p, _) = single_player::local_player(&l.host().game).unwrap();
        let i = map.index_of_level(single_player::COLD_PLAINS).unwrap();
        let wp = l
            .host_mut()
            .game
            .events
            .action
            .hooks()
            .waypoints
            .entry(p)
            .or_default();
        wp.get_mut(0).set(u32::from(i)).unwrap();
    });
    // Walk to the town waypoint and operate it (C→S 0x13), then take it
    // (C→S 0x49 [GUID][level]).
    let wp: UnitKey = app_support::operate_town_waypoint(&mut app, &server, &ms);
    let mut m = vec![0x49];
    m.extend_from_slice(&wp.guid.to_le_bytes());
    m.extend_from_slice(&single_player::COLD_PLAINS.to_le_bytes());
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .send_bytes(&m)
        .unwrap();
    while server_level(&server) != Some(single_player::COLD_PLAINS) {
        step(&mut app);
    }
    for _ in 0..30 {
        step(&mut app);
    }
    assert_eq!(level(&app), Some(single_player::COLD_PLAINS as u16));
    let b = &app.world().resource::<BridgeResource>().0;
    assert!(b.log().rejected.is_empty(), "{:?}", b.log().rejected);
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn taking_a_waypoint_to_an_unbuilt_level_builds_it_on_arrival() {
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
        assert!(steps < 3000, "the warp happens");
    };
    while app_support::local_player(&server).is_none() {
        step(&mut app);
    }
    for _ in 0..30 {
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
    // The destination: the first Act I waypoint level (`levels`
    // `Waypoint` order) the game has not built yet, read from the server
    // (which levels the act's creation builds is the DRLG's, not this
    // test's); the character learns its waypoint (stands in for
    // activating it).
    let map = app_support::waypoints().map.clone();
    let dest = app_support::with(&server, move |l| {
        let (p, _) = single_player::local_player(&l.host().game).unwrap();
        let g = &mut l.host_mut().game;
        let game = &mut g.game;
        let unbuilt = g
            .events
            .action
            .hooks()
            .drlg
            .with_act(0, &mut game.lists, |d, _| {
                (0..map.level_count())
                    .filter(|&lv| map.act(lv) == Some(0) && map.index_of_level(lv).is_some())
                    .find(|&lv| d.find_level(lv).is_none())
            })
            .flatten()
            .expect("an Act I waypoint level not built at the join");
        let i = map.index_of_level(unbuilt).unwrap();
        let wp = g.events.action.hooks().waypoints.entry(p).or_default();
        wp.get_mut(0).set(u32::from(i)).unwrap();
        unbuilt
    });
    // Walk to the town waypoint and operate it (C→S 0x13), then take it
    // (C→S 0x49 [GUID][level]).
    let wp: UnitKey = app_support::operate_town_waypoint(&mut app, &server, &ms);
    let mut m = vec![0x49];
    m.extend_from_slice(&wp.guid.to_le_bytes());
    m.extend_from_slice(&dest.to_le_bytes());
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .send_bytes(&m)
        .unwrap();
    while server_level(&server) != Some(dest) {
        step(&mut app);
    }
    for _ in 0..30 {
        step(&mut app);
    }
    assert_eq!(level(&app), Some(dest as u16));
    let b = &app.world().resource::<BridgeResource>().0;
    assert!(b.log().rejected.is_empty(), "{:?}", b.log().rejected);
}

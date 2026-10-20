//! (q-act3-act5-gaps) The Act III and Act V waypoints of the app's synthetic
//! game, headless: the player is moved to the act's town by the act-change
//! queue, operates the town's waypoint (C→S 0x13) and takes it (C→S 0x49
//! [GUID][level]) to a field of the act; the server places the player there
//! and the client's model follows. The Kurast Docks waypoint is the
//! `DockTown3.ds1` preset; the field indexes are
//! the game's `levels` `Waypoint` column, as `waypoints.tsv` holds.

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
fn the_act_iii_waypoint_takes_the_player_to_spider_forest() {
    travel(75, 76, 2);
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_act_v_waypoint_takes_the_player_to_rigid_highlands() {
    travel(single_player::ACT5_TOWN, 111, 4);
}

/// From the waypoint of `town` (act `act`, 0-based) to the field level
/// `field`, whose waypoint index is its `levels` `Waypoint`.
fn travel(town: u32, field: u32, act: u8) {
    let wp_index = u32::from(
        app_support::waypoints()
            .map
            .index_of_level(field)
            .expect("the field has a waypoint"),
    );
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
        assert!(steps < 4000, "the warp happens");
    };
    while app_support::local_player(&server).is_none() {
        step(&mut app);
    }
    for _ in 0..30 {
        step(&mut app);
    }
    // The character learns the field's waypoint (stands in for
    // activating it) and is moved to the act's town.
    app_support::with(&server, move |l| {
        let (p, _) = single_player::local_player(&l.host().game).unwrap();
        let g = &mut l.host_mut().game;
        let wp = g.events.action.hooks().waypoints.entry(p).or_default();
        wp.get_mut(0).set(wp_index).unwrap();
        g.events.action.hooks().act_changes.push((p, town, 0));
    });
    while server_level(&server) != Some(town) {
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
    assert_eq!(level(&app), Some(town as u16));
    let b = &app.world().resource::<BridgeResource>().0;
    assert_eq!(b.world().act.as_ref().map(|a| a.act), Some(act));
    let wp: UnitKey = app_support::operate_town_waypoint(&mut app, &server, &ms);
    let mut m = vec![0x49];
    m.extend_from_slice(&wp.guid.to_le_bytes());
    m.extend_from_slice(&field.to_le_bytes());
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .send_bytes(&m)
        .unwrap();
    while server_level(&server) != Some(field) {
        step(&mut app);
    }
    for _ in 0..30 {
        step(&mut app);
    }
    assert_eq!(level(&app), Some(field as u16));
    let b = &app.world().resource::<BridgeResource>().0;
    assert!(b.log().rejected.is_empty(), "{:?}", b.log().rejected);
}

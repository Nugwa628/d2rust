//! B002: the local walk prediction against the server player on the real
//! tables. A walk that carries the player over a level border ends on the
//! server's cell centre; the prediction must end there too (it used to
//! drop the walk at the border and rest up to a step short of it, so the
//! next walk started from another fraction than the server's and the two
//! drifted apart: the rubber band).
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use d2_client::app::play::{add_client_data, add_game, add_walk, predict_link, send_create_game};
use d2_client::app::single_player;
use d2_client::world_view::walk::PreviewWalk;
use d2_server::seams::Clock;

use crate::app_support;
use app_support::SharedLink;

struct StepClock(Arc<AtomicU32>);
impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// The server player's precise (16.16) position.
fn server_precise(server: &app_support::Server<StepClock>) -> (u32, u32) {
    app_support::with(server, |l| {
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

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_walk_over_the_level_border_rests_where_the_server_does() {
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
    add_walk(
        &mut app,
        tap,
        single_player::walk_speeds(&data, &single_player::Character::New).unwrap(),
    );
    send_create_game(&mut app).unwrap();
    add_client_data(
        &mut app,
        single_player::client_drlg_source(&data),
        single_player::client_level_rows(&data),
    );
    app_support::live_tables(&mut app);
    app.update();
    let step = |app: &mut App| {
        ms.fetch_add(40, Ordering::SeqCst);
        app.update();
    };
    while app_support::local_player(&server).is_none() {
        step(&mut app);
    }
    for _ in 0..30 {
        step(&mut app);
    }
    // Out of the town into the Blood Moor: the last leg crosses the border.
    app_support::walk_into(
        &mut app,
        &server,
        &ms,
        single_player::ACT1_TOWN,
        single_player::BLOOD_MOOR,
    );
    for _ in 0..6 {
        step(&mut app);
    }
    let client = app
        .world()
        .resource::<PreviewWalk>()
        .predict
        .position()
        .expect("the prediction");
    assert_eq!(
        client,
        server_precise(&server),
        "the walk ends on the server's cell centre"
    );
}

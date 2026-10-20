//! Town Portal in the app's own synthetic game, headless (task
//! `q-town-portal`). The player stands in
//! the Blood Moor of Evil (reached through its warp tile, as `app_level_warp.rs`);
//! the Town Portal cast (`ActionSim::open_town_portal`,
//! what the scroll's C→S 0x20 ends in) makes the portal pair: the field
//! portal reaches the client (S→C 0x51), a click on it (C→S 0x13) moves
//! the player to the town, where the town portal's click brings the
//! player back and removes the pair.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use d2_client::app::play::{add_client_data, add_game, add_walk, predict_link, send_create_game};
use d2_client::app::single_player;
use d2_client::bridge::world::{UnitKey, OBJECT};
use d2_client::bridge::BridgeResource;
use d2_server::seams::Clock;
use d2_sim::units::UnitType;

use crate::app_support;
use app_support::SharedLink;

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

/// The class-59 objects of the server's game.
fn server_portals(server: &app_support::Server<StepClock>) -> usize {
    app_support::with(server, |l| {
        let g = &l.host().game.game;
        g.lists
            .units_of_type(UnitType::Object)
            .into_iter()
            .filter(|&u| {
                l.host()
                    .game
                    .events
                    .action
                    .sys
                    .units
                    .get(u)
                    .is_some_and(|r| r.class == 59)
            })
            .count()
    })
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_town_portal_takes_the_player_to_town_and_back_and_goes() {
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
    let mut steps = 0;
    let mut step = |app: &mut App| {
        ms.fetch_add(40, Ordering::SeqCst);
        app.update();
        steps += 1;
        assert!(steps < 6000, "the test finishes");
    };
    while app_support::local_player(&server).is_none() {
        step(&mut app);
    }
    for _ in 0..30 {
        step(&mut app);
    }
    let find = |app: &App, ty: u8, class: u32| -> Option<UnitKey> {
        app.world()
            .resource::<BridgeResource>()
            .0
            .world()
            .units
            .iter()
            .find(|(k, u)| k.unit_type == ty && u.class == class)
            .map(|(k, _)| *k)
    };

    // Out of town into the Blood Moor on foot.
    app_support::walk_into(
        &mut app,
        &server,
        &ms,
        single_player::ACT1_TOWN,
        single_player::BLOOD_MOOR,
    );
    for _ in 0..30 {
        step(&mut app);
    }
    assert_eq!(server_portals(&server), 0);

    // The Town Portal use: the pair exists, the field portal reaches the
    // client.
    let made = app_support::with(&server, |l| {
        let (p, _) = single_player::local_player(&l.host().game).unwrap();
        let g = &mut l.host_mut().game;
        g.events.action.open_town_portal(&mut g.game, p).is_some()
    });
    assert!(made, "the pair was created");
    assert_eq!(server_portals(&server), 2);
    for _ in 0..10 {
        step(&mut app);
    }
    let field = find(&app, OBJECT, 59).expect("the field portal reached the client");

    // Click it (the portal's hostile delay of 5000 ms from the host
    // clock's zero has passed): the player is
    // in the town.
    ms.fetch_add(10_000, Ordering::SeqCst);
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .interact(field)
        .unwrap();
    while server_level(&server) != Some(single_player::ACT1_TOWN) {
        step(&mut app);
    }
    for _ in 0..30 {
        step(&mut app);
    }
    assert_eq!(server_portals(&server), 2, "the pair stays on the way in");
    let town_portal = find(&app, OBJECT, 59).expect("the town portal reached the client");

    // Click the town portal: back in the Blood Moor, the pair is gone.
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .interact(town_portal)
        .unwrap();
    while server_level(&server) != Some(single_player::BLOOD_MOOR) {
        step(&mut app);
    }
    for _ in 0..30 {
        step(&mut app);
    }
    assert_eq!(server_portals(&server), 0, "used from town, both go");
    assert!(
        find(&app, OBJECT, 59).is_none(),
        "the client's portals went too"
    );
    let b = &app.world().resource::<BridgeResource>().0;
    assert!(b.log().rejected.is_empty(), "{:?}", b.log().rejected);
}

/// The synthetic game booted up to the Blood Moor, as the test above.
fn boot() -> (App, app_support::Server<StepClock>, Arc<AtomicU32>) {
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
    (app, server, ms)
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn casting_in_town_makes_nothing_and_the_owner_name_arrives() {
    let (mut app, server, ms) = boot();
    let mut steps = 0;
    let mut step = |app: &mut App| {
        ms.fetch_add(40, Ordering::SeqCst);
        app.update();
        steps += 1;
        assert!(steps < 6000, "the test finishes");
    };
    while app_support::local_player(&server).is_none() {
        step(&mut app);
    }
    for _ in 0..30 {
        step(&mut app);
    }
    let find_all = |app: &App, ty: u8, class: u32| -> Vec<UnitKey> {
        app.world()
            .resource::<BridgeResource>()
            .0
            .world()
            .units
            .iter()
            .filter(|(k, u)| k.unit_type == ty && u.class == class)
            .map(|(k, _)| *k)
            .collect()
    };
    let cast = |server: &app_support::Server<StepClock>| {
        app_support::with(server, |l| {
            let (p, _) = single_player::local_player(&l.host().game).unwrap();
            let g = &mut l.host_mut().game;
            g.events.action.open_town_portal(&mut g.game, p).is_some()
        })
    };

    // A cast in town makes nothing (as in the game).
    assert!(!cast(&server), "refused in town");
    assert_eq!(server_portals(&server), 0);

    // Into the Blood Moor, cast there: the owner name rides with the
    // portal.
    app_support::walk_into(
        &mut app,
        &server,
        &ms,
        single_player::ACT1_TOWN,
        single_player::BLOOD_MOOR,
    );
    for _ in 0..30 {
        step(&mut app);
    }
    assert!(cast(&server));
    for _ in 0..10 {
        step(&mut app);
    }
    let field = find_all(&app, OBJECT, 59)[0];
    let name = app.world().resource::<BridgeResource>().0.world().units[&field]
        .kind
        .clone();
    let d2_client::bridge::world::KindData::Object(d) = name else {
        panic!("an object");
    };
    let owner = d.owner_name.expect("S→C 0x82 named the owner");
    assert_ne!(owner[0], 0, "a name, not an empty field");

    // To town through it (state 102 needs a states table: the unit test
    // `portal_use_sets_state_102`).
    ms.fetch_add(10_000, Ordering::SeqCst);
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .interact(field)
        .unwrap();
    while server_level(&server) != Some(single_player::ACT1_TOWN) {
        step(&mut app);
    }
    for _ in 0..30 {
        step(&mut app);
    }

    // A cast in town after a field cast still makes nothing: the field
    // pair stays as it was (q-fix-real-tp-town-cast).
    assert!(!cast(&server), "refused in town with a pair open");
    assert_eq!(server_portals(&server), 2, "the pair is untouched");
    for _ in 0..10 {
        step(&mut app);
    }
    let near = find_all(&app, OBJECT, 59);
    assert!(!near.is_empty(), "the town portal reached the client");
    ms.fetch_add(10_000, Ordering::SeqCst);
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .interact(near[0])
        .unwrap();
    while server_level(&server) != Some(single_player::BLOOD_MOOR) {
        step(&mut app);
    }
    let b = &app.world().resource::<BridgeResource>().0;
    assert!(b.log().rejected.is_empty(), "{:?}", b.log().rejected);
}

/// B012: arriving in town by the portal, the server walks the player out
/// of the arrival point (S→C 0x0D code 1). The
/// model's mode stays the town walk (6) for good; the player must be drawn
/// standing (town neutral 5) once the prediction's walk-out ended, not in
/// the walk animation until the next click.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_portal_arrival_ends_standing_not_walking() {
    let (mut app, server, ms) = boot();
    let mut steps = 0;
    let mut step = |app: &mut App| {
        ms.fetch_add(40, Ordering::SeqCst);
        app.update();
        steps += 1;
        assert!(steps < 6000, "the test finishes");
    };
    while app_support::local_player(&server).is_none() {
        step(&mut app);
    }
    for _ in 0..30 {
        step(&mut app);
    }
    app_support::walk_into(
        &mut app,
        &server,
        &ms,
        single_player::ACT1_TOWN,
        single_player::BLOOD_MOOR,
    );
    for _ in 0..30 {
        step(&mut app);
    }
    let made = app_support::with(&server, |l| {
        let (p, _) = single_player::local_player(&l.host().game).unwrap();
        let g = &mut l.host_mut().game;
        g.events.action.open_town_portal(&mut g.game, p).is_some()
    });
    assert!(made);
    for _ in 0..10 {
        step(&mut app);
    }
    let field = app
        .world()
        .resource::<BridgeResource>()
        .0
        .world()
        .units
        .iter()
        .find(|(k, u)| k.unit_type == OBJECT && u.class == 59)
        .map(|(k, _)| *k)
        .expect("the field portal");
    ms.fetch_add(10_000, Ordering::SeqCst);
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .interact(field)
        .unwrap();
    while server_level(&server) != Some(single_player::ACT1_TOWN) {
        step(&mut app);
    }
    // The walk-out is under way, then over.
    for _ in 0..60 {
        step(&mut app);
    }
    let b = &app.world().resource::<BridgeResource>().0;
    let walk = app
        .world()
        .resource::<d2_client::world_view::walk::PreviewWalk>();
    let (neutral, town_walk) = (5, 6);
    assert_eq!(b.world().local().map(|p| p.mode), Some(town_walk));
    assert!(walk.predict.walking().is_none(), "the walk-out ended");
    assert_eq!(
        walk.predict.stood_mode(b.world()),
        Some(neutral),
        "drawn standing"
    );
}

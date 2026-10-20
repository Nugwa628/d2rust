//! Town and panel gaps of the play (q-town-gaps), on the install: the
//! vendors a player needs are placed by the towns' DS1 presets when the
//! player brings their rooms into play (`app_support::approach`), never
//! by the build.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use d2_client::app::play::{add_client_data, add_game, add_walk, predict_link, send_create_game};
use d2_client::app::single_player;
use d2_client::bridge::predict::Speeds;
use d2_server::seams::Clock;
use d2_sim::units::UnitType;
use d2_sim::world::npc::class;

use crate::app_support;
use app_support::SharedLink;

struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Joins the install's game; with `town`, the player is then moved there
/// by the NPC act change (`NpcWorld::act_change`). Walks the town until
/// each class of `classes` stands next to the player and the client model
/// holds it (a preset places a unit when its room comes into play).
fn npcs_are_placed(town: Option<u32>, classes: &[u16]) {
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
        assert!(steps < 3000, "the town is reached");
    };
    while app_support::local_player(&server).is_none() {
        step(&mut app);
    }
    for _ in 0..30 {
        step(&mut app);
    }
    if let Some(town) = town {
        app_support::with(&server, move |l| {
            let (p, _) = single_player::local_player(&l.host().game).unwrap();
            l.host_mut()
                .game
                .events
                .action
                .hooks()
                .act_changes
                .push((p, town, 0));
        });
        for _ in 0..60 {
            step(&mut app);
        }
        assert_eq!(app_support::server_level(&server), Some(town));
    }
    let classes: Vec<u32> = classes.iter().map(|&c| u32::from(c)).collect();
    for &c in &classes {
        app_support::approach(&mut app, &server, &ms, UnitType::Monster as u8, &[c]);
    }
}

/// Gheed and Charsi are Rogue Encampment presets.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_gambler_and_repairers_of_act_one_are_placed() {
    npcs_are_placed(None, &[class::GHEED, class::CHARSI]);
}

/// Elzix is a Lut Gholein preset.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_gambler_of_act_two_is_placed() {
    npcs_are_placed(Some(single_player::ACT2_TOWN), &[class::ELZIX]);
}

/// Jamella is a Pandemonium Fortress preset.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_healer_of_act_four_is_placed() {
    npcs_are_placed(Some(single_player::PANDEMONIUM_FORTRESS), &[class::JAMELLA]);
}

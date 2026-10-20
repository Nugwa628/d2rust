//! The one-app session, headless: three games in one app (front end ->
//! game -> front end), each started on the running app's world and ended
//! by Save and Exit. Every game is the same as the first; nothing a game
//! installed is left after it; the process memory does not grow per game.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use d2_client::app::play::{install_live_client, register_live_client, LiveClient};
use d2_client::app::session::{self, GameEnder, GameEntity, GameStarter, Screen};
use d2_client::app::single_player::{self, GameData};
use d2_client::bridge::BridgeResource;
use d2_server::seams::Clock;

use crate::app_support;

struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// The resident set (`VmRSS`, KiB) of this process.
fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("VmRSS:"))
                .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
        })
        .unwrap_or(0)
}

fn own_resources(world: &World) -> std::collections::BTreeSet<bevy::ecs::component::ComponentId> {
    world.iter_resources().map(|(i, _)| i.id()).collect()
}

fn live_entities(app: &mut App) -> usize {
    let w = app.world_mut();
    w.query::<Entity>().iter(w).count()
}

fn game_entities(app: &mut App) -> usize {
    let w = app.world_mut();
    w.query_filtered::<(), With<GameEntity>>().iter(w).count()
}

fn in_game(app: &App) -> bool {
    app.world()
        .get_resource::<BridgeResource>()
        .is_some_and(|b| b.0.world().in_game && b.0.world().local().is_some())
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn three_games_in_one_app_leave_nothing_behind() {
    let data = app_support::game_data();
    let GameData::Live(live) = data.clone();
    let ms = Arc::new(AtomicU32::new(1000));
    let mut app = App::new();
    app.insert_resource(d2_client::bridge::mirror::ScriptedClock(ms.clone()));
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), StatesPlugin))
        .init_asset::<Image>()
        .init_resource::<ButtonInput<MouseButton>>();
    register_live_client(&mut app, false);
    session::add_session(&mut app);
    let starts = Arc::new(AtomicU32::new(0));
    let game = {
        let (ms, starts) = (ms.clone(), starts.clone());
        GameStarter(Box::new(move |world: &mut World| {
            let character = single_player::Character::New;
            let seed = single_player::game_seed(&character, None);
            let (link, started) = single_player::start_with(
                data.clone(),
                seed,
                character.clone(),
                StepClock(ms.clone()),
            )?;
            let speeds = single_player::walk_speeds(&data, &character)?;
            install_live_client(
                world,
                Box::new(link),
                LiveClient {
                    data: &live,
                    request: &character,
                    start_flags: None,
                    prices: started.prices,
                    speeds,
                    hardcore: false,
                    automap_files: None,
                    gpu: false,
                },
            )?;
            starts.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }))
    };
    app.insert_non_send(game)
        .insert_non_send(GameEnder(Box::new(|_| Ok(false))));
    let step = |app: &mut App| {
        app.update();
        ms.fetch_add(40, Ordering::SeqCst);
    };
    // The front end's frames before the first game (the engine adds its
    // own lazy resources in the first passes).
    for _ in 0..5 {
        step(&mut app);
    }
    let mut front = own_resources(app.world());
    let mut entities = live_entities(&mut app);
    let mut rss = Vec::new();
    for round in 1..=3u32 {
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Game);
        let mut frames = 0;
        while !in_game(&app) {
            step(&mut app);
            frames += 1;
            assert!(frames < 2000, "game {round}: the join finishes");
        }
        for _ in 0..50 {
            step(&mut app);
        }
        assert_eq!(starts.load(Ordering::SeqCst), round);
        assert!(game_entities(&mut app) > 0);
        // Save and Exit: the server's leave takes the client out; the
        // session goes back to the front end.
        app.world_mut()
            .resource_mut::<BridgeResource>()
            .0
            .save_and_exit()
            .unwrap();
        let mut frames = 0;
        while *app.world().resource::<State<Screen>>().get() != Screen::Front {
            step(&mut app);
            frames += 1;
            assert!(frames < 500, "game {round}: the leave finishes");
        }
        step(&mut app);
        // After the first game: the engine's record of the first state
        // change and the session's game counter; nothing else.
        let now = own_resources(app.world());
        if round == 1 {
            use std::any::TypeId;
            let allowed = [
                TypeId::of::<bevy::state::state::PreviousState<Screen>>(),
                TypeId::of::<session::GameGeneration>(),
            ];
            for id in now.difference(&front) {
                let t = app
                    .world()
                    .components()
                    .get_info(*id)
                    .and_then(|i| i.type_id());
                assert!(
                    t.is_some_and(|t| allowed.contains(&t)),
                    "game 1 left resource {id:?}"
                );
            }
            front = now;
        } else {
            assert_eq!(now, front, "game {round}: resources left");
        }
        assert_eq!(game_entities(&mut app), 0);
        // The entities: a resource is an entity, kept empty when the
        // resource goes and reused when it comes back, so the first game
        // sets the count every later game ends with.
        let count = live_entities(&mut app);
        if round == 1 {
            entities = count;
        } else {
            assert_eq!(count, entities, "game {round}: entities left");
        }
        rss.push(rss_kib());
        println!("after game {round}: rss {} KiB", rss_kib());
    }
    // Measured on the first run: the second and third games end within a
    // few MiB of the first (allocator slack), not a game's size apart.
    let growth = rss[2].saturating_sub(rss[0]);
    assert!(
        growth < 64 * 1024,
        "rss grew {growth} KiB over two games: {rss:?}"
    );
}

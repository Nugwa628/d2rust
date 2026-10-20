//! The play preview's player death headless, over the synthetic
//! single-player game: a player whose life reaches 0 dies on the server
//! (DT, then DD with a corpse), the client model follows (modes 0, 0x11),
//! the death screen comes up, and Esc sends C→S 0x41, which respawns the
//! player in town.
//!
//! The save is the DD start's; the DT start saves
//! nothing.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use d2_client::app::death::{add_death, DeathScreen};
use d2_client::app::hardcore::add_hardcore;
use d2_client::app::play::{add_client_data, add_game, add_preview, send_create_game_for};
use d2_client::app::save;
use d2_client::app::single_player::{self};
use d2_client::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use d2_client::bridge::mirror::DynLink;
use d2_client::bridge::modes::player_mode;
use d2_client::bridge::BridgeResource;
use d2_client::world_view::tile_assets::TileAssets;
use d2_server::seams::Clock;
use d2_sim::units::hooks::Sim;
use d2_sim::units::modes::player_event1;

use crate::app_support;

struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// The link the app holds, shared with the test so it can reach the
/// server thread.
struct Shared<L>(Arc<Mutex<L>>);

impl<L: ServerLink> ServerLink for Shared<L> {
    fn protocol_version(&self) -> u32 {
        self.0.lock().unwrap().protocol_version()
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        self.0.lock().unwrap().tick_rate()
    }
    fn send(&mut self, q: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        self.0.lock().unwrap().send(q, msg)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        self.0.lock().unwrap().pump()
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        self.0.lock().unwrap().receive()
    }
}

fn step(app: &mut App, ms: &AtomicU32, n: usize) {
    for _ in 0..n {
        app.update();
        ms.fetch_add(40, Ordering::SeqCst);
    }
}

fn local_mode(app: &App) -> u32 {
    app.world()
        .resource::<BridgeResource>()
        .0
        .world()
        .local()
        .expect("local")
        .mode
}

struct Outcome {
    dropped: Vec<(d2_sim::units::UnitId, u32)>,
    left: bool,
    dead: bool,
    status: u16,
}

fn run(hardcore: bool) -> Outcome {
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
    let link = Arc::new(Mutex::new(link));
    link.lock()
        .unwrap()
        .with(move |l| l.host_mut().game.events.action.hooks().x.hardcore = hardcore)
        .unwrap();
    let dyn_link: DynLink = Box::new(Shared(link.clone()));
    let mut app = App::new();
    app.insert_resource(d2_client::bridge::mirror::ScriptedClock(ms.clone()));
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ButtonInput<KeyCode>>();
    add_game(&mut app, dyn_link, false).unwrap();
    send_create_game_for(&mut app, &character).unwrap();
    let levels = single_player::client_level_rows(&data);
    add_client_data(
        &mut app,
        single_player::client_drlg_source(&data),
        levels.clone(),
    );
    add_preview(&mut app, levels, TileAssets::default());
    add_death(&mut app);
    add_hardcore(&mut app, hardcore);
    step(&mut app, &ms, 10);

    link.lock()
        .unwrap()
        .with(|l| {
            let sim = &mut l.host_mut().game;
            let (p, _) = single_player::local_player(sim).expect("joined");
            sim.events.action.start_death(&mut sim.game, p);
        })
        .unwrap();
    step(&mut app, &ms, 4);
    assert!(app.world().resource::<DeathScreen>().active);
    link.lock()
        .unwrap()
        .with(|l| {
            let sim = &mut l.host_mut().game;
            let (p, _) = single_player::local_player(sim).expect("joined");
            let s = &mut sim.events.action.sys;
            let mut u = Sim {
                game: &mut sim.game,
                units: &mut s.units,
                stats: &mut s.stats,
                data: &s.data,
            };
            player_event1(&mut u, &mut s.hooks, p).unwrap();
        })
        .unwrap();
    step(&mut app, &ms, 3);
    assert_eq!(local_mode(&app), player_mode::DEAD);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Escape);
    step(&mut app, &ms, 6);
    let left = app.should_exit().is_some();
    let (dropped, live) = link
        .lock()
        .unwrap()
        .with(|l| {
            let sim = &mut l.host_mut().game;
            let dropped = sim.events.action.hooks().x.dropped.clone();
            let live = save::read_live(sim).unwrap();
            (dropped, live)
        })
        .unwrap();
    let base = save::base_save(&character);
    let written = save::apply_live(&base, &live, 1).header.status;
    Outcome {
        dropped,
        left,
        dead: live.hardcore_dead,
        status: written,
    }
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_hardcore_death_drops_the_client_and_marks_the_save_dead() {
    let Outcome {
        dropped,
        left,
        dead,
        status,
    } = run(true);
    assert_eq!(dropped.len(), 1, "the client was dropped");
    assert_eq!(dropped[0].1, 3, "reason 3, the hardcore resurrect");
    assert!(left, "the game closes");
    assert!(dead);
    assert_eq!(
        status & (d2_formats::d2s::status::HARDCORE | d2_formats::d2s::status::DEAD),
        d2_formats::d2s::status::HARDCORE | d2_formats::d2s::status::DEAD
    );
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_softcore_death_respawns_and_the_save_stays_alive() {
    let Outcome {
        dropped,
        left,
        dead,
        status,
    } = run(false);
    assert!(dropped.is_empty());
    assert!(!left);
    assert!(!dead);
    // Both death starts set 0x08 for every player, softcore
    // too (measured 0x0028 after a town respawn); the loader refuses a
    // dead character only when it is hardcore, so it stays
    // playable.
    assert_eq!(
        status & d2_formats::d2s::status::DEAD,
        d2_formats::d2s::status::DEAD
    );
    assert_eq!(status & d2_formats::d2s::status::HARDCORE, 0);
}

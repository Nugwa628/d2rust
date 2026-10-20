//! The play preview headless on the user's install, wired as
//! `d2-client play` wires it (`add_live_client`): the map is built from
//! the client DRLG, the near rooms' DT1 tiles are read from the install's
//! archives, and every frame is built: resident tiles are drawn, the
//! others skipped and logged once.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use d2_client::app::play::{add_live_client, LiveClient};
use d2_client::app::single_player::{self, GameData};
use d2_client::bridge::BridgeResource;
use d2_client::world_view::WorldViewState;
use d2_server::seams::Clock;

use crate::app_support;
use app_support::SharedLink;

struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_preview_draws_the_map_from_the_client_drlg() {
    let data = app_support::game_data();
    let GameData::Live(live) = data.clone();
    let character = single_player::new_character("sorceress", "Test").unwrap();
    let speeds = single_player::walk_speeds(&data, &character).unwrap();
    let ms = Arc::new(AtomicU32::new(1000));
    let (link, started) = single_player::start_with(
        data,
        single_player::DEFAULT_SEED,
        character.clone(),
        StepClock(ms.clone()),
    )
    .unwrap();
    let server = Arc::new(Mutex::new(link));
    let mut app = App::new();
    app.insert_resource(d2_client::bridge::mirror::ScriptedClock(ms.clone()));
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .init_resource::<ButtonInput<MouseButton>>();
    add_live_client(
        &mut app,
        Box::new(SharedLink(server.clone())),
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
    )
    .unwrap();
    for _ in 0..12 {
        app.update();
        ms.fetch_add(40, Ordering::SeqCst);
    }
    let b = &app.world().resource::<BridgeResource>().0;
    let w = b.world();
    assert!(w.local_room().is_some(), "joined into the town room");
    let state = app.world().resource::<WorldViewState>();
    assert!(state.preview);
    let last = state.last.expect("frames are built, none failed");
    assert_eq!(last.server_tick, w.server_ticks);
    assert!(last.items > 0, "the map's tiles are drawn: {last:?}");
    // The town's DT1 tiles of the install are resident in the frame store.
    assert!(!state.assets.frames.is_empty(), "no tile frame is resident");
}

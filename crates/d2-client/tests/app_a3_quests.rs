//! The Guardian quest (A3Q6, chain 20) in the app's synthetic game,
//! headless (task `q-a3-quests`): a monster of Mephisto's class is linked
//! to chain 20 by its creation-time boss mods
//! and its death reaches the quest control (state 6).
//! Mephisto is spawned by hand: the synthetic game has no Durance of Hate
//! level to hold him; the class-to-chain table is the monster init's,
//! checked on the live `monstats.txt` by `real_boss_chains_follow_the_live_base_ids`.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use d2_client::app::play::{add_client_data, add_game, add_walk, predict_link, send_create_game};
use d2_client::app::single_player::{self};
use d2_client::bridge::predict::Speeds;
use d2_client::bridge::BridgeResource;
use d2_server::seams::Clock;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::action::{KillStep, Pending};
use d2_sim::world::quests::act3::npc::MEPHISTO;

use crate::app_support;
use app_support::SharedLink;

struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

type Server = app_support::Server<StepClock>;

/// (state, status) of chain 20 on the server.
fn chain20(server: &Server) -> Option<(u8, u8)> {
    app_support::with(server, |l| {
        let r = l.host().game.world.quests.record(20)?;
        Some((r.state, r.status))
    })
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn mephistos_death_reaches_the_guardian_quest() {
    let data = app_support::game_data();
    let ms = Arc::new(AtomicU32::new(1000));
    let (link, _) = single_player::start(
        data.clone(),
        single_player::DEFAULT_SEED,
        StepClock(ms.clone()),
    )
    .unwrap();
    let server: Server = Arc::new(Mutex::new(link));
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
    let mut steps = 0u32;
    let mut step = |app: &mut App, n: u32| {
        for _ in 0..n {
            ms.fetch_add(40, Ordering::SeqCst);
            app.update();
            steps += 1;
            assert!(steps < 20000);
        }
    };
    while app_support::local_player(&server).is_none() {
        step(&mut app, 1);
    }
    step(&mut app, 30);
    assert!(chain20(&server).is_some_and(|c| c.0 < 6), "chain 20 exists");
    app_support::with(&server, |l| {
        let g = &mut l.host_mut().game;
        let (p, _) = single_player::local_player(g).unwrap();
        let (room, pos) = {
            let e = g.game.lists.unit(p).unwrap();
            (e.room().unwrap(), g.events.action.hooks().path_position(p))
        };
        let req = d2_sim::units::lifecycle::AllocRequest {
            ty: UnitType::Monster,
            class: u32::from(MEPHISTO),
            room: Some(room),
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: false,
        };
        let m: UnitId = g
            .events
            .action
            .with(&mut g.game, |gm, v| {
                let m = v.allocate(gm, &req, pos.0 + 4, pos.1)?;
                // The chain his creation-time boss mods link.
                v.link_boss_chains(m, req.class);
                Some(m)
            })
            .expect("Mephisto");
        g.events
            .action
            .hooks()
            .x
            .kill_step(&mut g.game, KillStep::QuestKill, m, p);
    });
    step(&mut app, 30);
    assert_eq!(chain20(&server).map(|c| c.0), Some(6), "Mephisto's death");
    let b = &app.world().resource::<BridgeResource>().0;
    assert!(b.log().rejected.is_empty(), "{:?}", b.log().rejected);
}

// D2RUST's own: dev tools.
//! The dev tools on the app's own game, headless: commands go down the
//! channel the overlay uses, the server thread's before-pump hook runs
//! them between frames, and the client model follows. God mode keeps a
//! player alive through a lethal death start; `town` / `warp` move the
//! player; the stat commands change what the game's own messages carry.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use d2_client::app::dev::server::DevChannel;
use d2_client::app::play::{add_client_data, add_game, add_walk, predict_link, send_create_game};
use d2_client::app::single_player::{self, Link};
use d2_client::app::strings::{TableStrings, LANG};
use d2_client::bridge::modes::player_mode;
use d2_client::bridge::predict::Speeds;
use d2_client::bridge::BridgeResource;
use d2_proto::dev::{parse, DevCommand, DevReply};
use d2_server::seams::Clock;
use d2_sim::stats::stat;

use crate::app_support;
use app_support::SharedLink;

struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

struct Game {
    app: App,
    server: app_support::Server<StepClock>,
    chan: DevChannel,
    ms: Arc<AtomicU32>,
}

impl Game {
    fn start() -> Self {
        Self::start_with(None)
    }

    fn start_with(start: Option<d2_client::app::dev::slots::DevStart>) -> Self {
        let data = app_support::game_data();
        let live = app_support::live();
        let ms = Arc::new(AtomicU32::new(1000));
        let (mut link, _) = single_player::start(
            data.clone(),
            single_player::DEFAULT_SEED,
            StepClock(ms.clone()),
        )
        .unwrap();
        let strings = TableStrings::load(live.archives.as_ref(), LANG).unwrap();
        let catalog = d2_client::app::dev::catalog(&live, &strings);
        let chan = d2_client::app::dev::install_server(&mut link, catalog, start).unwrap();
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
        let mut g = Game {
            app,
            server,
            chan,
            ms,
        };
        let mut n = 0;
        while app_support::local_player(&g.server).is_none() {
            g.step(1);
            n += 1;
            assert!(n < 500, "the join runs");
        }
        g.step(30);
        g
    }

    fn step(&mut self, n: usize) {
        for _ in 0..n {
            self.ms.fetch_add(40, Ordering::SeqCst);
            self.app.update();
        }
    }

    /// Sends one console line and steps until its reply arrives.
    fn run(&mut self, line: &str) -> DevReply {
        let cmd: DevCommand = parse(line).unwrap().unwrap();
        self.chan.lock().unwrap().commands.push(cmd);
        for _ in 0..10 {
            self.step(1);
            if let Some(r) = self.chan.lock().unwrap().replies.pop() {
                return r;
            }
        }
        panic!("no reply to {line:?}");
    }

    fn ok(&mut self, line: &str) -> String {
        let r = self.run(line);
        assert!(r.ok, "{line:?}: {}", r.text);
        r.text
    }

    fn on_server<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Link<StepClock>) -> R + Send + 'static,
    ) -> R {
        app_support::with(&self.server, f)
    }

    fn stat(&self, s: u16) -> i32 {
        self.on_server(move |l| {
            let g = &l.host().game;
            let (p, _) = single_player::local_player(g).unwrap();
            g.events.action.sys.stats.unit_base(p, s, 0)
        })
    }

    fn server_level(&self) -> Option<u32> {
        self.on_server(|l| {
            let g = &mut l.host_mut().game;
            let (p, _) = single_player::local_player(g)?;
            let room = g.game.lists.unit(p)?.room()?;
            g.events.action.hooks().drlg.level_id(&g.game, room)
        })
    }

    fn client_level(&self) -> Option<u16> {
        self.app
            .world()
            .resource::<BridgeResource>()
            .0
            .world()
            .player_level()
    }

    fn local_mode(&self) -> u32 {
        self.app
            .world()
            .resource::<BridgeResource>()
            .0
            .world()
            .local()
            .expect("local")
            .mode
    }
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn god_mode_refuses_death_and_teleport_moves_the_player() {
    let mut g = Game::start();
    assert_eq!(g.client_level(), Some(single_player::ACT1_TOWN as u16));

    // Without god mode a lethal death start is a death (the control).
    // With it: the player stays alive with full life.
    assert_eq!(g.ok("god"), "god on");
    assert!(!g.run("god").switches.god);
    let r = g.run("god on");
    assert!(r.ok && r.switches.god);
    let (life, max) = g.on_server(|l| {
        let sim = &mut l.host_mut().game;
        let (p, _) = single_player::local_player(sim).unwrap();
        let sys = &mut sim.events.action.sys;
        sys.stats.unit_set(&mut sys.hooks, p, stat::HITPOINTS, 0, 0);
        sim.events.action.start_death(&mut sim.game, p);
        let st = &sim.events.action.sys.stats;
        (st.unit_base(p, stat::HITPOINTS, 0), st.max_life(p))
    });
    assert!(max > 0);
    assert_eq!(life, max, "god mode refills the life at the death start");
    g.step(5);
    let mode = g.local_mode();
    assert_ne!(mode, player_mode::DEATH, "no death animation");
    assert_ne!(mode, player_mode::DEAD);

    // Teleport: act 2's town, then a level by name, then back by id.
    g.ok("town 2");
    let mut n = 0;
    while g.server_level() != Some(40) {
        g.step(1);
        n += 1;
        assert!(n < 300, "the act change ends in Lut Gholein");
    }
    g.step(60);
    assert_eq!(g.client_level(), Some(40), "the client follows to act 2");
    let t = g.ok("warp rocky waste");
    assert!(t.contains("level 41"), "{t}");
    g.step(30);
    assert_eq!(g.server_level(), Some(41));
    g.ok("town 1");
    let mut n = 0;
    while g.server_level() != Some(1) {
        g.step(1);
        n += 1;
        assert!(n < 300, "back to the Rogue Encampment");
    }
    g.step(60);
    assert_eq!(g.client_level(), Some(1));

    // Turning god mode off: the next death start is a death.
    g.ok("god off");
    g.on_server(|l| {
        let sim = &mut l.host_mut().game;
        let (p, _) = single_player::local_player(sim).unwrap();
        sim.events.action.start_death(&mut sim.game, p);
    });
    g.step(5);
    assert_eq!(g.local_mode(), player_mode::DEATH, "the control: a death");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn stat_commands_level_gold_points_and_heal() {
    let mut g = Game::start();
    assert_eq!(g.stat(stat::LEVEL), 1);
    let t = g.ok("level 10");
    assert!(t.contains("1 -> 10"), "{t}");
    assert_eq!(g.stat(stat::LEVEL), 10);
    // 5 stat points and 1 skill point a level (`charstats`).
    assert_eq!(g.stat(4), 45);
    assert_eq!(g.stat(5), 9);
    assert!(!g.run("level 5").ok, "only raising");
    g.ok("statpts 3");
    assert_eq!(g.stat(4), 48);
    g.ok("skillpts 2");
    assert_eq!(g.stat(5), 11);
    g.ok("gold 1000000");
    assert_eq!(g.stat(14), 100_000, "capped at level × 10000");
    // Heal after losing life.
    g.on_server(|l| {
        let sim = &mut l.host_mut().game;
        let (p, _) = single_player::local_player(sim).unwrap();
        let sys = &mut sim.events.action.sys;
        sys.stats
            .unit_set(&mut sys.hooks, p, stat::HITPOINTS, 256, 0);
        sys.stats.unit_set(&mut sys.hooks, p, stat::MANA, 0, 0);
    });
    g.ok("heal");
    let full = g.on_server(|l| {
        let sim = &l.host().game;
        let (p, _) = single_player::local_player(sim).unwrap();
        let st = &sim.events.action.sys.stats;
        st.unit_base(p, stat::HITPOINTS, 0) == st.max_life(p)
            && st.unit_base(p, stat::MANA, 0) == st.max_mana(p)
    });
    assert!(full);
    // The client hears of the new level through the game's messages.
    g.step(10);
    // waypoints / acts answer ok.
    assert!(g.ok("waypoints").contains("waypoints known"));
    g.ok("acts");
    // One quest flag bit, set and cleared in every difficulty.
    let quest_bit = |g: &Game, difficulty: usize| {
        g.on_server(move |l| {
            let sim = &l.host().game;
            let (p, _) = single_player::local_player(sim).unwrap();
            sim.world.rest.quests[&p].flags[difficulty].get(37, 8)
        })
    };
    assert!(!quest_bit(&g, 0));
    assert!(g.ok("quest 37 8").contains("set"));
    assert!(quest_bit(&g, 0) && quest_bit(&g, 2));
    assert!(g.ok("quest 37 8 off").contains("cleared"));
    assert!(!quest_bit(&g, 0) && !quest_bit(&g, 2));
    assert!(!g.run("quest 41 0").ok);
    // The help and an unknown name.
    assert!(g.ok("help").contains("god"));
    assert!(!g.run("warp no such place").ok);
    // An item at the player's feet; monsters spawned and killed.
    let held = |g: &Game| {
        g.on_server(|l| {
            let sim = &l.host().game;
            let (p, _) = single_player::local_player(sim).unwrap();
            sim.world
                .inventory
                .as_ref()
                .map_or(0, |i| i.state.items_of(p).len())
        })
    };
    let before = held(&g);
    let t = g.ok("item hax magic");
    assert!(t.contains("magic") && t.contains("in the inventory"), "{t}");
    assert_eq!(held(&g), before + 1);
    let t = g.ok("item Ring rare ilvl 50 ground");
    assert!(t.contains("at your feet") && t.contains("[rin]"), "{t}");
    assert_eq!(held(&g), before + 1);
    let t = g.ok("identify");
    assert!(t.starts_with("identified"), "{t}");
    g.step(5);
    let t = g.ok("warp blood moor");
    assert!(t.contains("level 2"), "{t}");
    g.step(30);
    let t = g.ok("spawn zombie 3");
    assert!(t.starts_with("spawned"), "{t}");
    g.step(5);
    let t = g.ok("kill");
    assert!(!t.contains("killed 0 "), "{t}");
    // speed, nocd, freemana.
    let v0 = g.stat(67);
    assert!(g.ok("speed 100").contains("+100"));
    assert_eq!(g.stat(67), v0 + 100);
    assert!(g.run("speed 0").switches.speed == 0);
    assert_eq!(g.stat(67), v0);
    assert!(g.run("nocd").switches.nocd);
    assert!(g.run("freemana on").switches.freemana);
    g.on_server(|l| {
        let sim = &mut l.host_mut().game;
        let (p, _) = single_player::local_player(sim).unwrap();
        let sys = &mut sim.events.action.sys;
        sys.stats.unit_set(&mut sys.hooks, p, stat::MANA, 0, 0);
    });
    g.step(2);
    assert!(g.on_server(|l| {
        let sim = &l.host().game;
        let (p, _) = single_player::local_player(sim).unwrap();
        let st = &sim.events.action.sys.stats;
        st.unit_base(p, stat::MANA, 0) == st.max_mana(p)
    }));
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_loaded_slot_takes_the_player_back_with_its_switches() {
    use d2_client::app::dev::slots::DevStart;
    use d2_proto::dev::DevSwitchState;
    let start = DevStart {
        level: 40,
        x: 5153,
        y: 5203,
        switches: DevSwitchState {
            god: true,
            nocd: false,
            freemana: true,
            speed: 0,
        },
    };
    let mut g = Game::start_with(Some(start));
    let mut n = 0;
    let r = loop {
        g.step(1);
        if let Some(r) = g.chan.lock().unwrap().replies.pop() {
            break r;
        }
        n += 1;
        assert!(n < 800, "the slot's way back ends");
    };
    assert!(r.ok, "{}", r.text);
    assert!(r.switches.god && r.switches.freemana && !r.switches.nocd);
    assert_eq!(g.server_level(), Some(40));
    let (x, y) = g.on_server(|l| {
        let sim = &l.host().game;
        let (p, _) = single_player::local_player(sim).unwrap();
        sim.events.action.sys.hooks.path_position(p)
    });
    assert!((x - 5153).abs() <= 8 && (y - 5203).abs() <= 8, "({x}, {y})");
}

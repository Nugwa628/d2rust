//! Playtest B032 probe: doors on the app's own game, headless, reached
//! with the dev warp.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use d2_client::app::dev::server::DevChannel;
use d2_client::app::play::{add_client_data, add_game, add_walk, predict_link, send_create_game};
use d2_client::app::single_player::{self, Link};
use d2_client::app::strings::{TableStrings, LANG};
use d2_client::bridge::predict::Speeds;
use d2_client::bridge::world::{KindData, UnitKey, OBJECT};
use d2_client::bridge::BridgeResource;
use d2_proto::dev::{parse, DevCommand, DevReply};
use d2_server::seams::Clock;

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
}

/// A door of the client model: key, class, mode, cell, footprint.
type Door = (UnitKey, u32, u32, (u16, u16), bool);

/// The door objects (objects.txt `IsDoor`) of the client model.
fn doors(g: &Game) -> Vec<Door> {
    let b = &g.app.world().resource::<BridgeResource>().0;
    let rows = &b.inputs().objclient.rows;
    b.world()
        .units
        .iter()
        .filter(|(k, u)| {
            k.unit_type == OBJECT && rows.get(u.class as usize).is_some_and(|r| r.is_door != 0)
        })
        .filter_map(|(k, u)| {
            let fp = match &u.kind {
                KindData::Object(d) => d.footprint,
                _ => false,
            };
            Some((*k, u.class, u.mode, u.position?, fp))
        })
        .collect()
}

// Playtest B032 (not reproduced): the closed doors of the Monastery Gate
// and the Barracks reach the client as objects with a footprint, the
// cursor on one hovers it, and a run through a closed door stops in front
// of it on the server (the door's cells carry the object bit 0x400).
// A live monster drawn over the cursor outranks a door (type priority:
// monster 0, object -6): in the Barracks a monster pack idles just past
// the door at (15030, 4430), so the cursor on that door hovers a monster.
// The door check then runs on the doors alone.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn closed_doors_are_hovered_and_block_a_run() {
    use d2_client::rules::camera::{moving_to_client, Camera, FrameSize, OpenMode};
    let mut g = Game::start();
    for lv in ["monastery gate", "barracks"] {
        assert!(g.ok(&format!("warp {lv}")).contains("warped"));
        g.step(60);
        let d = doors(&g);
        assert!(!d.is_empty(), "{lv}: doors near the arrival");
        let cursor = |(x, y): (u16, u16)| {
            let at = moving_to_client(u32::from(x) << 16, u32::from(y) << 16);
            let cam = Camera::new(FrameSize::D2RUST, OpenMode::NONE, at, (0, 0));
            let (fx, fy) = d2_client::bridge::hover::unit_feet(&cam, OBJECT, (x, y));
            (cam, (fx, fy - 20))
        };
        // With the monsters alive: a door under a live monster's hover box
        // hovers a monster, never another object.
        let w = g.app.world().resource::<BridgeResource>().0.world();
        for &(k, _, _, at, _) in &d {
            let (cam, mouse) = cursor(at);
            let covered = w.units.iter().any(|(mk, mu)| {
                mk.unit_type == 1
                    && d2_client::bridge::hover::monster_hoverable(w, mu)
                    && mu.position.is_some_and(|c| {
                        let (mx, my) = d2_client::bridge::hover::unit_feet(&cam, 1, c);
                        let (dx, dy) = (mouse.0 - mx, mouse.1 - my);
                        dx.abs() <= d2_client::bridge::hover::HIT_HALF_WIDTH
                            && (-d2_client::bridge::hover::HIT_ABOVE
                                ..=d2_client::bridge::hover::HIT_BELOW)
                                .contains(&dy)
                    })
            });
            let got = d2_client::bridge::hover::pick(w, &cam, mouse);
            if covered {
                assert_eq!(
                    got.map(|g| g.unit_type),
                    Some(1),
                    "{lv} {k:?}: a monster outranks the door"
                );
            } else {
                assert_eq!(got, Some(k), "{lv} {k:?}: hovered");
            }
        }
        // The doors alone (the monsters and floor items left out of a copy
        // of the client model): each one is hovered by the cursor on it.
        let mut w = w.clone();
        w.units.retain(|k, _| !matches!(k.unit_type, 1 | 4));
        let w = &w;
        for (k, _, mode, at, fp) in d {
            assert_eq!(fp, mode == 0, "{lv} {k:?}: a closed door has a footprint");
            let (cam, mouse) = cursor(at);
            assert_eq!(
                d2_client::bridge::hover::pick(w, &cam, mouse),
                Some(k),
                "{lv} {k:?}: hovered"
            );
        }
    }
    // The Barracks door at (14980, 4485) (class 14, closed, 3 × 1): its
    // cells hold 0x400; a run from (14983, 4473) to (14981, 4492) ends in
    // front of it.
    let mask = g.on_server(|l| {
        let g = &mut l.host_mut().game;
        let d = g.events.action.hooks().drlg.dungeon.acts[0]
            .as_ref()
            .unwrap();
        d.collision_at(14980, 4485)
    });
    assert_eq!(mask.map(|m| m & 0x400), Some(0x400));
    let pos = |g: &Game| {
        g.on_server(|l| {
            let g = &mut l.host_mut().game;
            let (p, _) = single_player::local_player(g).unwrap();
            g.events.action.hooks().path_position(p)
        })
    };
    assert_eq!(pos(&g), (14983, 4473), "the Barracks arrival");
    let mut m = vec![0x03];
    m.extend_from_slice(&14981u16.to_le_bytes());
    m.extend_from_slice(&4492u16.to_le_bytes());
    g.app
        .world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .send_bytes(&m)
        .unwrap();
    g.step(150);
    assert!(
        pos(&g).1 < 4485,
        "stopped before the door, at {:?}",
        pos(&g)
    );
}

/// The keys in the local player's inventory page (the sum of the key
/// stacks' quantities), read from the server's inventory model.
fn keys(g: &Game) -> i32 {
    g.on_server(|l| {
        let s = &mut l.host_mut().game;
        let (p, _) = single_player::local_player(s).unwrap();
        let inv = s.world.inventory.as_ref().expect("the inventory model");
        let Some(items) = inv.state.inventories.get(&p).map(|i| i.items().to_vec()) else {
            return 0;
        };
        items
            .into_iter()
            .filter(|i| {
                inv.state.items.get(i).is_some_and(|d| {
                    d.page == d2_sim::items::moves::page::INVENTORY
                        && inv
                            .tables
                            .item(d.record)
                            .is_some_and(|r| r.code == d2_sim::items::moves::handlers::KEY_CODE)
                })
            })
            .map(|i| s.events.action.sys.stats.unit_base(i, 70, 0))
            .sum()
    })
}

/// Creates object `class` two sub-tiles east of the local player and
/// locks it (`door`: mode 6; else the chest's locked flag 0x80); returns
/// its unit and GUID.
fn locked_object(g: &Game, class: u32, door: bool) -> (d2_sim::units::UnitId, u32) {
    g.on_server(move |l| {
        let s = &mut l.host_mut().game;
        let (p, _) = single_player::local_player(s).unwrap();
        let line = format!("object {class} @x+2 @y");
        let r = d2_sim::poke::apply_line(
            &mut s.game,
            &mut s.events,
            &d2_sim::poke::Env::new(p),
            &line,
        )
        .unwrap();
        let d2_sim::poke::PokeResult::Ok(Some(guid)) = r else {
            panic!("{line}: {r:?}");
        };
        let obj = s
            .game
            .lists
            .find_unit(d2_sim::units::UnitType::Object, guid)
            .unwrap();
        if door {
            s.events.action.sys.units.get_mut(obj).unwrap().mode = 6;
        } else {
            let st = s.events.action.hooks().objects.as_mut().unwrap();
            st.control.data.get_mut(&obj).unwrap().interact |= 0x80;
        }
        (obj, guid)
    })
}

/// The local player operates the object (the server's operate entry, as
/// a C→S 0x13 reaches it); returns the object's mode after the tick that
/// handles the key use.
fn operate(g: &mut Game, obj: d2_sim::units::UnitId, guid: u32) -> u32 {
    g.on_server(move |l| {
        let s = &mut l.host_mut().game;
        let (p, _) = single_player::local_player(s).unwrap();
        s.events
            .action
            .with(&mut s.game, |gm, v| v.operate_object(gm, Some(p), guid))
            .expect("the operate entry runs");
    });
    g.step(3);
    g.on_server(move |l| {
        let s = &mut l.host_mut().game;
        s.events.action.sys.units.get(obj).map_or(0, |u| u.mode)
    })
}

// Owner report E5 (locked doors and chests): the key test reads the
// player's inventory. A locked door (mode 6) or a locked chest stays shut
// for a player without a key; with a key it opens and takes one key from
// the stack (the last key of a stack is used up).
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_locked_door_and_a_locked_chest_take_a_key() {
    let mut g = Game::start();
    let class = g.on_server(|l| {
        let s = &mut l.host_mut().game;
        let (p, _) = single_player::local_player(s).unwrap();
        s.events.action.sys.units.get(p).map(|u| u.class)
    });
    assert_ne!(class, Some(6), "an assassin opens a chest without a key");
    assert!(g.ok("warp blood moor").contains("warped"));
    g.step(30);
    assert_eq!(keys(&g), 0);
    // No key: the door and the chest stay shut.
    let (door, door_guid) = locked_object(&g, 14, true);
    assert_eq!(
        operate(&mut g, door, door_guid),
        6,
        "a locked door without a key"
    );
    let (chest, chest_guid) = locked_object(&g, 5, false);
    assert_eq!(
        operate(&mut g, chest, chest_guid),
        0,
        "a locked chest without a key"
    );
    // A key stack: each lock takes one.
    g.ok("item key");
    g.ok("item key");
    g.step(5);
    let before = keys(&g);
    assert!(before >= 2, "the keys reached the inventory: {before}");
    assert_eq!(
        operate(&mut g, door, door_guid),
        2,
        "the key opens the door"
    );
    assert_eq!(keys(&g), before - 1, "the door took one key");
    assert_ne!(
        operate(&mut g, chest, chest_guid),
        0,
        "the key opens the chest"
    );
    assert_eq!(keys(&g), before - 2, "the chest took one key");
}

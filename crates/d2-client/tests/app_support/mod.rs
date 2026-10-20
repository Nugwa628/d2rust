//! Shared set-up of the app tests over the app's own single-player game:
//! a link the app owns while the test keeps a handle to the server thread
//! (the player exists only after the join, so
//! the test looks it up after the app's frames ran the session sequence).
#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use d2_client::app::server_thread::ThreadLink;
use d2_client::app::single_player::{self, Link};
use d2_client::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use d2_server::seams::Clock;
use d2_sim::units::UnitId;

/// A link shared by the app (the bridge's link) and the test.
pub struct SharedLink<T>(pub Arc<Mutex<T>>);

impl<T: ServerLink> ServerLink for SharedLink<T> {
    fn protocol_version(&self) -> u32 {
        self.0.lock().unwrap().protocol_version()
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        self.0.lock().unwrap().tick_rate()
    }
    fn send(&mut self, queue: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        self.0.lock().unwrap().send(queue, msg)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        self.0.lock().unwrap().pump()
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        self.0.lock().unwrap().receive()
    }
}

/// The app game's server thread, shared.
pub type Server<C> = Arc<Mutex<ThreadLink<Link<C>>>>;

/// The local player and its GUID, once the join ran.
pub fn local_player<C: Clock + Send + 'static>(server: &Server<C>) -> Option<(UnitId, u32)> {
    server
        .lock()
        .unwrap()
        .with(|l| single_player::local_player(&l.host().game))
        .unwrap()
}

/// Runs `f` on the server's link between frames.
pub fn with<C, R, F>(server: &Server<C>, f: F) -> R
where
    C: Clock + Send + 'static,
    R: Send + 'static,
    F: FnOnce(&mut Link<C>) -> R + Send + 'static,
{
    server.lock().unwrap().with(f).unwrap()
}

/// The reason every test on the user's files carries: the real-data
/// gate (`tools/realdata-gate.sh`, `--run-ignored only`) runs it; CI has
/// no game files and skips it.
pub const REAL_DATA: &str = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)";

/// The user's game install (`$D2_GAME_DIR`), loaded as `d2-client play`
/// loads it (`GameData::select`), once per test binary. No directory is a
/// failure, never a fallback: the tests that call this are
/// `#[ignore = REAL_DATA]`.
pub fn game_data() -> single_player::GameData {
    static DATA: std::sync::OnceLock<single_player::GameData> = std::sync::OnceLock::new();
    DATA.get_or_init(|| {
        let dir = std::env::var_os("D2_GAME_DIR")
            .map(std::path::PathBuf::from)
            .expect("D2_GAME_DIR: this test runs on the user's game install");
        single_player::GameData::select(Some(&dir)).expect("the install loads")
    })
    .clone()
}

/// The user's live data (`GameData::Live`).
pub fn live() -> Arc<single_player::LiveData> {
    let single_player::GameData::Live(d) = game_data();
    d
}

/// The client tables `play::add_live_client` gives the bridge on the
/// user's files: skills, class skills, skill tables, unit rows, item
/// tables, object rows (for tests that wire the app by hand).
pub fn live_tables(app: &mut bevy::prelude::App) {
    let mut b = app
        .world_mut()
        .resource_mut::<d2_client::bridge::BridgeResource>();
    live_bridge_tables(&mut b.0);
}

/// [`live_tables`] on a bridge without an app.
pub fn live_bridge_tables<L: ServerLink>(b: &mut d2_client::bridge::Bridge<L>) {
    let d = live();
    let data = single_player::GameData::Live(d.clone());
    let a = d.archives.as_ref();
    b.set_skill_rows(single_player::client_skill_rows(a).unwrap());
    b.set_class_skills(single_player::client_class_skills(a).unwrap());
    b.set_skill_tables(Arc::new(single_player::client_skill_tables(a).unwrap()));
    b.set_unit_rows(single_player::client_unit_rows(a).unwrap());
    b.set_item_tables(Arc::new(d2_client::app::items::TableDecoder(Arc::new(
        d.tables.item_tables().unwrap(),
    ))));
    b.set_object_rows(single_player::client_object_rows(&data));
}

/// The `lvlwarp` ids (the warp tile units' classes)
/// of the warps from level `from` to level `to`: the install's `levels`
/// row of `from`, the `Warp` of every `Vis` slot that names `to`. A level
/// can have several (the Blood Moor's four ways into the Den of Evil,
/// the stamp placed decides which tile exists).
pub fn warp_ids(from: u32, to: u32) -> Vec<u32> {
    let d = live();
    let l = &d.levels.drlg.levels[from as usize];
    let ids: Vec<u32> = l
        .vis
        .iter()
        .zip(&l.warp)
        .filter(|&(&v, &w)| v == to && w >= 0)
        .map(|(_, &w)| w as u32)
        .collect();
    assert!(!ids.is_empty(), "level {from} has no way to level {to}");
    ids
}

/// The world sub-tile position of the warp tile preset of act 0's level
/// `level` whose class is in `classes` (the
/// room's tile origin x 5 + the preset's position), once the level's room
/// holds it.
pub fn warp_preset_spot<C: Clock + Send + 'static>(
    server: &Server<C>,
    level: u32,
    classes: &[u32],
) -> Option<(i32, i32)> {
    let classes = classes.to_vec();
    with(server, move |l| {
        let g = &mut l.host_mut().game;
        let h = g.events.action.hooks();
        let mut found = None;
        h.drlg.with_act(0, &mut g.game.lists, |d, svc| {
            let lv = d.find_level(level)?;
            for r in d.level_rooms(lv) {
                let rect = d.room(r).rect;
                for p in svc.types.preset_units(d, r) {
                    if p.unit_type == 5 && classes.contains(&p.class) {
                        found = Some((rect.x * 5 + p.x, rect.y * 5 + p.y));
                    }
                }
            }
            Some(())
        });
        found
    })
}

/// Run legs (C→S 0x03) inside level `level` of act 0 toward `goal`
/// (sub-tiles) until the server player is within `reach` sub-tiles of it,
/// re-planning over the rooms that activate on the way.
pub fn walk_toward<C: Clock + Send + 'static>(
    app: &mut bevy::prelude::App,
    server: &Server<C>,
    ms: &std::sync::atomic::AtomicU32,
    level: u32,
    goal: (i32, i32),
    reach: i32,
) {
    use std::sync::atomic::Ordering;
    let step = |app: &mut bevy::prelude::App| {
        app.update();
        ms.fetch_add(40, Ordering::SeqCst);
    };
    let at = |server: &Server<C>| {
        with(server, |l| {
            let g = &mut l.host_mut().game;
            let (p, _) = single_player::local_player(g).expect("joined");
            g.events.action.hooks().path_position(p)
        })
    };
    let near = |p: (i32, i32)| (p.0 - goal.0).abs() <= reach && (p.1 - goal.1).abs() <= reach;
    for _ in 0..40 {
        if near(at(server)) {
            return;
        }
        let legs = with(server, move |l| {
            let g = &mut l.host_mut().game;
            let (p, _) = single_player::local_player(g).expect("joined");
            let h = g.events.action.hooks();
            let start = h.path_position(p);
            let d = h.drlg.dungeon.acts[0].as_ref().expect("Act I");
            let area = d.level(d.find_level(level).expect("level allocated")).rect;
            test_fixtures::host::route_near(d, start, area, goal, reach, 12)
        });
        if legs.is_empty() {
            step(app);
            continue;
        }
        for (x, y) in legs {
            let mut m = vec![0x03];
            m.extend_from_slice(&(x as u16).to_le_bytes());
            m.extend_from_slice(&(y as u16).to_le_bytes());
            app.world_mut()
                .resource_mut::<d2_client::bridge::BridgeResource>()
                .0
                .send_bytes(&m)
                .unwrap();
            step(app);
            step(app);
            for _ in 0..400 {
                let moving = with(server, |l| {
                    let g = &l.host().game;
                    let (p, _) = single_player::local_player(g)?;
                    g.events.action.sys.units.get(p).map(|u| u.mode)
                })
                .is_some_and(|m| test_fixtures::host::MOVING.contains(&m));
                if !moving {
                    break;
                }
                step(app);
            }
            if near(at(server)) {
                break;
            }
        }
    }
    assert!(near(at(server)), "walked to {goal:?}, at {:?}", at(server));
}

/// The centre of the NPC menu box's selectable row `i` (the spec box sits
/// above the NPC; `OriginalUi::npc_menu_row_point`).
pub fn npc_menu_row(app: &bevy::prelude::App, i: usize) -> d2_client::ui::Point {
    app.world()
        .non_send::<d2_client::world_view::WorldViewUi>()
        .original
        .as_ref()
        .expect("the game UI")
        .npc_menu_row_point(i)
        .unwrap_or_else(|| panic!("the NPC menu box has a row {i}"))
}

/// The NPC menu's selectable row count, 0 when no box is up.
pub fn npc_menu_len(app: &bevy::prelude::App) -> usize {
    app.world()
        .non_send::<d2_client::world_view::WorldViewUi>()
        .original
        .as_ref()
        .expect("the game UI")
        .npc_menu()
        .filter(|m| !m.talking)
        .map_or(0, |m| m.rows.len())
}

/// The centre of the talk topic box's cancel.
pub fn npc_topic_cancel(app: &bevy::prelude::App) -> d2_client::ui::Point {
    app.world()
        .non_send::<d2_client::world_view::WorldViewUi>()
        .original
        .as_ref()
        .expect("the game UI")
        .npc_topic_cancel_point()
        .expect("the talk topic box is up")
}

/// The server player's level (`None` before the join).
pub fn server_level<C: Clock + Send + 'static>(server: &Server<C>) -> Option<u32> {
    with(server, |l| {
        let (p, _) = single_player::local_player(&l.host().game)?;
        let g = &mut l.host_mut().game;
        let room = g.game.lists.unit(p)?.room()?;
        g.events.action.hooks().drlg.level_id(&g.game, room)
    })
}

/// Run legs (C→S 0x03 through the app's bridge) from level `from` into
/// level `to` of act 0 along the collision route of the server's active
/// rooms (`test_fixtures::host::route`), stepping `app` one frame (40 ms
/// on `ms`) at a time, until the server player is in `to`.
pub fn walk_into<C: Clock + Send + 'static>(
    app: &mut bevy::prelude::App,
    server: &Server<C>,
    ms: &std::sync::atomic::AtomicU32,
    from: u32,
    to: u32,
) {
    use std::sync::atomic::Ordering;
    let step = |app: &mut bevy::prelude::App| {
        app.update();
        ms.fetch_add(40, Ordering::SeqCst);
    };
    let moving = |server: &Server<C>| {
        with(server, |l| {
            let g = &l.host().game;
            let (p, _) = single_player::local_player(g)?;
            g.events.action.sys.units.get(p).map(|u| u.mode)
        })
        .is_some_and(|m| test_fixtures::host::MOVING.contains(&m))
    };
    for _ in 0..12 {
        if server_level(server) == Some(to) {
            return;
        }
        let legs = with(server, move |l| {
            let g = &mut l.host_mut().game;
            let (p, _) = single_player::local_player(g).expect("joined");
            let start = g.events.action.hooks().path_position(p);
            let h = g.events.action.hooks();
            let d = h.drlg.dungeon.acts[0].as_ref().expect("Act I");
            let rect = |id| {
                d.find_level(id)
                    .map(|l| d.level(l).rect)
                    .expect("level allocated")
            };
            test_fixtures::host::route(d, start, rect(from), rect(to), 12)
        });
        for (x, y) in legs {
            let mut m = vec![0x03];
            m.extend_from_slice(&(x as u16).to_le_bytes());
            m.extend_from_slice(&(y as u16).to_le_bytes());
            app.world_mut()
                .resource_mut::<d2_client::bridge::BridgeResource>()
                .0
                .send_bytes(&m)
                .unwrap();
            step(app);
            step(app);
            for _ in 0..400 {
                if !moving(server) {
                    break;
                }
                step(app);
            }
            if server_level(server) == Some(to) {
                break;
            }
        }
    }
    assert_eq!(server_level(server), Some(to), "walked into level {to}");
}

/// The server unit of type `ty` (1 monster, else object) and a class of
/// `classes` nearest to the local player: its GUID and position.
pub fn server_unit<C: Clock + Send + 'static>(
    server: &Server<C>,
    ty: u8,
    classes: &[u32],
) -> Option<(u32, (i32, i32))> {
    let classes = classes.to_vec();
    with(server, move |l| {
        let g = &mut l.host_mut().game;
        let (p, _) = single_player::local_player(g)?;
        let at = g.events.action.hooks().path_position(p);
        let st = match ty {
            1 => d2_sim::units::UnitType::Monster,
            _ => d2_sim::units::UnitType::Object,
        };
        let units: Vec<UnitId> = g
            .game
            .lists
            .units_of_type(st)
            .into_iter()
            .filter(|&u| {
                g.events
                    .action
                    .sys
                    .units
                    .get(u)
                    .is_some_and(|r| classes.contains(&r.class))
            })
            .collect();
        units
            .into_iter()
            .map(|u| {
                let guid = g.game.lists.unit(u).map_or(0, |e| e.guid);
                (guid, g.events.action.hooks().path_position(u))
            })
            .min_by_key(|&(_, q)| test_fixtures::host::cheb(at, q))
    })
}

/// The install's waypoint tables (`levels` `Waypoint` / `Act`, the
/// waypoint object classes).
pub fn waypoints() -> d2_sim::world::waypoints::WaypointData {
    let w = &live().waypoints;
    d2_sim::world::waypoints::WaypointData::new(&w.levels, &w.objects)
}

/// The server player's level and its act (`levels` `Act`).
pub fn level_act<C: Clock + Send + 'static>(server: &Server<C>) -> (u32, usize) {
    let level = server_level(server).expect("the player in a level");
    let act = waypoints().map.act(level).expect("the level's act");
    (level, usize::from(act))
}

/// The server player's position.
pub fn server_pos<C: Clock + Send + 'static>(server: &Server<C>) -> (i32, i32) {
    with(server, |l| {
        let g = &mut l.host_mut().game;
        let (p, _) = single_player::local_player(g).expect("joined");
        g.events.action.hooks().path_position(p)
    })
}

/// Run legs (C→S 0x03 through the app's bridge) inside the server
/// player's level toward `goal` (`test_fixtures::host::route_near`,
/// within `reach` sub-tiles), each until the player stops.
pub fn walk_town_to<C: Clock + Send + 'static>(
    app: &mut bevy::prelude::App,
    server: &Server<C>,
    ms: &std::sync::atomic::AtomicU32,
    goal: (i32, i32),
    reach: i32,
) {
    use std::sync::atomic::Ordering;
    let step = |app: &mut bevy::prelude::App| {
        app.update();
        ms.fetch_add(40, Ordering::SeqCst);
    };
    let moving = |server: &Server<C>| {
        with(server, |l| {
            let g = &l.host().game;
            let (p, _) = single_player::local_player(g)?;
            g.events.action.sys.units.get(p).map(|u| u.mode)
        })
        .is_some_and(|m| test_fixtures::host::MOVING.contains(&m))
    };
    let start = server_pos(server);
    let (level, act) = level_act(server);
    let legs = with(server, move |l| {
        let g = &mut l.host_mut().game;
        let d = g.events.action.hooks().drlg.dungeon.acts[act]
            .as_ref()
            .expect("the act");
        let area = d
            .find_level(level)
            .map(|l| d.level(l).rect)
            .expect("the player's level");
        test_fixtures::host::route_near(d, start, area, goal, reach, 12)
    });
    for (x, y) in legs {
        let mut m = vec![0x03];
        m.extend_from_slice(&(x as u16).to_le_bytes());
        m.extend_from_slice(&(y as u16).to_le_bytes());
        app.world_mut()
            .resource_mut::<d2_client::bridge::BridgeResource>()
            .0
            .send_bytes(&m)
            .unwrap();
        step(app);
        step(app);
        for _ in 0..400 {
            if !moving(server) {
                break;
            }
            step(app);
        }
    }
    for _ in 0..4 {
        step(app);
    }
}

/// Walks the server player's level (a town) until the server unit of type `ty` and a
/// class of `classes` stands within 10 sub-tiles of the player and the client
/// model holds it: toward it once the server has it, else through the
/// town's rooms nearest first (a preset is placed when the player brings
/// its room into play). Its GUID.
pub fn approach<C: Clock + Send + 'static>(
    app: &mut bevy::prelude::App,
    server: &Server<C>,
    ms: &std::sync::atomic::AtomicU32,
    ty: u8,
    classes: &[u32],
) -> u32 {
    let in_model = |app: &bevy::prelude::App, guid: u32| {
        app.world()
            .resource::<d2_client::bridge::BridgeResource>()
            .0
            .world()
            .units
            .keys()
            .any(|k| k.unit_type == ty && k.guid == guid)
    };
    let p = server_pos(server);
    let (level, act) = level_act(server);
    let mut rooms: Vec<(i32, i32)> = with(server, move |l| {
        let g = &mut l.host_mut().game;
        let d = g.events.action.hooks().drlg.dungeon.acts[act]
            .as_ref()
            .expect("the act");
        let town = d.find_level(level).expect("the player's level");
        d.level_rooms(town)
            .into_iter()
            .map(|r| {
                let t = d.room(r).rect;
                ((t.x * 2 + t.w) * 5 / 2, (t.y * 2 + t.h) * 5 / 2)
            })
            .collect()
    });
    rooms.sort_by_key(|&c| test_fixtures::host::cheb(c, p));
    let mut rooms = rooms.into_iter();
    for _ in 0..40 {
        match server_unit(server, ty, classes) {
            Some((guid, at)) => {
                if test_fixtures::host::cheb(server_pos(server), at) <= 10 && in_model(app, guid) {
                    return guid;
                }
                walk_town_to(app, server, ms, at, 4);
            }
            None => {
                let Some(c) = rooms.next() else { break };
                walk_town_to(app, server, ms, c, 4);
            }
        }
    }
    panic!("unit {ty}/{classes:?} not reached");
}

/// Walks the server player's level room by room (nearest first) until the
/// client model holds a tile unit (S→C 0x09) of `class`: a warp tile is
/// created when its room comes into play. Its key.
pub fn approach_tile<C: Clock + Send + 'static>(
    app: &mut bevy::prelude::App,
    server: &Server<C>,
    ms: &std::sync::atomic::AtomicU32,
    class: u32,
) -> d2_client::bridge::world::UnitKey {
    let find = |app: &bevy::prelude::App| {
        app.world()
            .resource::<d2_client::bridge::BridgeResource>()
            .0
            .world()
            .units
            .iter()
            .find(|(k, u)| k.unit_type == d2_client::bridge::world::TILE && u.class == class)
            .map(|(k, _)| *k)
    };
    // The level's rooms are made as the player moves: recount them every
    // round and walk to the nearest one not yet visited.
    let (level, act) = level_act(server);
    let mut visited: Vec<(i32, i32)> = Vec::new();
    for _ in 0..60 {
        if let Some(k) = find(app) {
            return k;
        }
        let p = server_pos(server);
        let mut rooms: Vec<(i32, i32)> = with(server, move |l| {
            let g = &mut l.host_mut().game;
            let d = g.events.action.hooks().drlg.dungeon.acts[act]
                .as_ref()
                .expect("the act");
            let lv = d.find_level(level).expect("the player's level");
            d.level_rooms(lv)
                .into_iter()
                .map(|r| {
                    let t = d.room(r).rect;
                    ((t.x * 2 + t.w) * 5 / 2, (t.y * 2 + t.h) * 5 / 2)
                })
                .collect()
        });
        rooms.retain(|c| !visited.contains(c));
        rooms.sort_by_key(|&c| test_fixtures::host::cheb(c, p));
        let Some(c) = rooms.first().copied() else {
            break;
        };
        visited.push(c);
        walk_town_to(app, server, ms, c, 4);
    }
    find(app).unwrap_or_else(|| {
        let tiles: Vec<_> = app
            .world()
            .resource::<d2_client::bridge::BridgeResource>()
            .0
            .world()
            .units
            .iter()
            .filter(|(k, _)| k.unit_type == d2_client::bridge::world::TILE)
            .map(|(_, u)| (u.class, u.position))
            .collect();
        panic!(
            "no tile of class {class} reached the client; tiles {tiles:?}; player at {:?} in level {level}",
            server_pos(server)
        )
    })
}

/// Runs to the unit `key` as the client does before an interact (C→S
/// 0x04), until the server player stops.
pub fn run_to_unit<C: Clock + Send + 'static>(
    app: &mut bevy::prelude::App,
    server: &Server<C>,
    ms: &std::sync::atomic::AtomicU32,
    key: d2_client::bridge::world::UnitKey,
) {
    use std::sync::atomic::Ordering;
    let mut m = vec![0x04];
    m.extend_from_slice(&u32::from(key.unit_type).to_le_bytes());
    m.extend_from_slice(&key.guid.to_le_bytes());
    app.world_mut()
        .resource_mut::<d2_client::bridge::BridgeResource>()
        .0
        .send_bytes(&m)
        .unwrap();
    for i in 0..300 {
        app.update();
        ms.fetch_add(40, Ordering::SeqCst);
        let mode = with(server, |l| {
            let g = &l.host().game;
            let (p, _) = single_player::local_player(g)?;
            g.events.action.sys.units.get(p).map(|u| u.mode)
        });
        if i > 2 && !mode.is_some_and(|m| test_fixtures::host::MOVING.contains(&m)) {
            break;
        }
    }
    for _ in 0..4 {
        app.update();
        ms.fetch_add(40, Ordering::SeqCst);
    }
}

/// The waypoint of the server player's town (an `objects` row with
/// operate function 23): walked to (the town's
/// preset places it when its room comes into play) and operated (C→S 0x13 through the
/// bridge's interact sender). Its key.
pub fn operate_town_waypoint<C: Clock + Send + 'static>(
    app: &mut bevy::prelude::App,
    server: &Server<C>,
    ms: &std::sync::atomic::AtomicU32,
) -> d2_client::bridge::world::UnitKey {
    use std::sync::atomic::Ordering;
    let classes: Vec<u32> = waypoints()
        .waypoint_classes()
        .into_iter()
        .map(u32::from)
        .collect();
    let guid = approach(app, server, ms, 2, &classes);
    let key = d2_client::bridge::world::UnitKey::new(2, guid);
    run_to_unit(app, server, ms, key);
    app.world_mut()
        .resource_mut::<d2_client::bridge::BridgeResource>()
        .0
        .interact(key)
        .unwrap();
    for _ in 0..10 {
        app.update();
        ms.fetch_add(40, Ordering::SeqCst);
    }
    key
}

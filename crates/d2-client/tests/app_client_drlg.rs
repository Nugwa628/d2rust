//! The client DRLG in the play mode's wiring, headless: `add_game` +
//! `add_client_data` over the app's own single-player game, whose
//! session flow (`d2_server::adapters::session_flow`) answers the
//! client's C→S 0x67 with 0x01, 0x00, 0x02 and its 0x6B (the bridge's
//! answer to 0x02) with the join: 0x59, 0x0B, 0x03, 0x07 and 0x15,
//! followed by the room switch's 0x07s. The client builds its own act DRLG from 0x03 and the
//! rooms 0x07 brings in sight; the local player's level is the level of
//! its room. A real join (ignored test) is delivered by a scripted link
//! in the game's order (0x01, 0x03, 0x59 at (0, 0), 0x0B, 0x07, 0x15,
//! 0x04).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use bevy::prelude::*;
use d2_client::app::single_player::{self, GameData};
use d2_client::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use d2_client::bridge::world::{ClientTables, ClientWorld};
use d2_client::bridge::Bridge;
use d2_client::rules::OpenMode;
use d2_proto::PROTOCOL_VERSION;
use d2_server::seams::Clock;

use crate::app_support;

/// Delivers one chunk list per pump (each pump a tick).
struct Script(VecDeque<Vec<Vec<u8>>>, Vec<Vec<u8>>);

impl ServerLink for Script {
    fn protocol_version(&self) -> u32 {
        PROTOCOL_VERSION
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        d2_sim::tick::TickRate::BASE
    }
    fn send(&mut self, _: SendQueue, _: &[u8]) -> Result<Sent, LinkError> {
        Ok(Sent::Queued)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        self.1 = self.0.pop_front().unwrap_or_default();
        Ok(Pumped { ticked: true })
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.1)
    }
}

fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap())
        .collect()
}

/// 0x59 for player 1 ("werwer", class 1) at (0, 0).
fn assign_player() -> Vec<u8> {
    let mut b = hex("59 01 00 00 00 01 77 65 72 77 65 72");
    b.resize(0x1A, 0);
    b
}

/// The client join with 0x03 `load_act`, the 0x07
/// `reveal` and the 0x15 `place`; frame 1 receives everything up to
/// 0x15, frame 2 the 0x04.
fn join(load_act: &str, reveal: &str, place: &str) -> Script {
    let frame1 = vec![
        hex("01 00 04 00 10 00 01 00"),
        hex(load_act),
        assign_player(),
        hex("0b 00 01 00 00 00"),
        hex(reveal),
        hex(place),
    ];
    Script(VecDeque::from([frame1, vec![hex("04")]]), Vec::new())
}

fn check_join(w: &ClientWorld, rejected: usize) -> u16 {
    assert_eq!(rejected, 0);
    assert!(w.in_game);
    let act = w.act.expect("0x03 received");
    let d = w.drlg.as_ref().expect("client DRLG built");
    assert_eq!((d.drlg.act, d.drlg.init_seed), (act.act, act.init_seed));
    assert!(d.drlg.on_client);
    let own = w
        .local_room()
        .expect("the local player is in an active room");
    assert_eq!(w.player_level(), Some(own.level));
    own.level
}

/// The host clock, advanced by the test.
struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// A real join (a known game session): 0x03 seq 142 (act 0, init seed 0x103888C4), 0x07
/// seq 144 (level 1, tile (0x3A0, 0x388)), 0x15 seq 154 (player 1 to
/// (4673, 4548)). Expect the client DRLG of the user's tables to have a
/// level-1 room of origin tile (928, 904) holding the player: level 1.
///
/// `D2_GAME_DIR=<install> cargo test -p d2-client --test app_client_drlg -- --ignored`
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn the_recorded_join_on_the_install() {
    use d2_client::app::palette::{act_palette_path, ActPalettes};

    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
    let data = GameData::select(Some(std::path::Path::new(&dir))).unwrap();
    let GameData::Live(live) = &data;
    let link = join(
        "03 00 c4 88 38 10 01 00 61 d1 e0 9f",
        "07 a0 03 88 03 01",
        "15 00 01 00 00 00 41 12 c4 11 01",
    );
    let mut bridge = Bridge::new(link).unwrap();
    bridge.set_drlg_source(Some(single_player::client_drlg_source(&data)));
    bridge.set_tables(ClientTables {
        levels: single_player::client_level_rows(&data),
        ..ClientTables::default()
    });
    // The install's client tables: the join selects the real skills.
    app_support::live_bridge_tables(&mut bridge);
    bridge.frame().unwrap();
    bridge.frame().unwrap();
    let w = bridge.world();
    let level = check_join(w, bridge.log().rejected.len());
    assert_eq!(level, 1);
    let own = w.local_room().unwrap();
    assert_eq!((own.x0, own.y0), (928 * 5, 904 * 5));
    // The client's generation of the Rogue Encampment passes its 35 rooms
    // to the automap callback +0x454, and no town art.
    let d = &w.drlg.as_ref().unwrap().drlg;
    assert_eq!(d.automap_rooms.len(), 35);
    assert!(d.automap_towns.is_empty());
    // Every act palette is in the archives and is a valid `pal.pl2`.
    let palettes = ActPalettes::live(live.archives.as_ref()).unwrap();
    for a in 0..5 {
        d2_client::scene::present_palette(palettes.of(a))
            .unwrap_or_else(|e| panic!("{}: {e}", act_palette_path(a)));
    }
}

/// The session join of the app's game on the user's files: game entry in
/// the Rogue Encampment (the town's spawn
/// search), the client DRLG built from the 0x03 and the 0x07s, the local
/// player in a level-1 room at the server's point. Prints the received
/// 0x07 count (1 from game entry + the spawn room's adjacency array).
///
/// `D2_GAME_DIR=<install> cargo test -p d2-client --test app_client_drlg -- --ignored`
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn the_session_join_on_the_install() {
    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
    let data = GameData::select(Some(std::path::Path::new(&dir))).unwrap();
    let ms = Arc::new(AtomicU32::new(1000));
    let (link, _) = single_player::start(
        data.clone(),
        single_player::DEFAULT_SEED,
        StepClock(ms.clone()),
    )
    .unwrap();
    let mut bridge = Bridge::new(link).unwrap();
    bridge.set_drlg_source(Some(single_player::client_drlg_source(&data)));
    bridge.set_tables(ClientTables {
        levels: single_player::client_level_rows(&data),
        ..ClientTables::default()
    });
    // The install's client tables: the join selects the real skills.
    app_support::live_bridge_tables(&mut bridge);
    // The session sequence: 0x67, then 0x6B after the flush with 0x02.
    bridge.send(&single_player::create_request()).unwrap();
    bridge.frame().unwrap();
    for _ in 0..2 {
        ms.fetch_add(40, Ordering::SeqCst);
        bridge.frame().unwrap();
    }
    let server_pos = bridge
        .link_mut()
        .with(|l| {
            let sim = &mut l.host_mut().game;
            let (player, _) = single_player::local_player(sim).expect("joined");
            sim.events.action.hooks().path_position(player)
        })
        .unwrap();
    assert!(
        bridge.log().rejected.is_empty(),
        "{:?}",
        bridge.log().rejected
    );
    let w = bridge.world();
    assert_eq!(w.act.map(|a| (a.act, a.town_level)), Some((0, 1)));
    let me = w.local_player.expect("0x0B named the local player");
    let pos = w.units[&me].position.expect("placed by 0x15");
    assert_eq!((i32::from(pos.0), i32::from(pos.1)), server_pos);
    assert_eq!(w.player_level(), Some(1));
    println!(
        "player at {pos:?}, {} rooms in sight: {:?}",
        w.rooms_in_sight.len(),
        w.rooms_in_sight
    );
    assert!(w.rooms_in_sight.len() >= 2);
}

/// The monster light columns the model creates monster lights from
/// (`MonsterClass::light`, `light_rgb`; the monster row), on the user's
/// install: each `monstats` row carries its `MonStatsEx` row's `Light`
/// and `light-r/g/b`, and the known counts hold (100 `monstats2`
/// rows with `Light` > 0; the fallen shamans' 5 with 230, 168, 255).
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn monster_light_rows_come_from_the_users_monstats2() {
    use d2_data::tables::{decode_all, Monstats, Monstats2};
    let d = app_support::live();
    let a = d.archives.as_ref();
    let rows = single_player::client_unit_rows(a).unwrap();
    let set = d2_data::bin::load_from(a, "eng").unwrap();
    let m1: Vec<Monstats> = decode_all(set.table("monstats").unwrap()).unwrap();
    let m2: Vec<Monstats2> = decode_all(set.table("monstats2").unwrap()).unwrap();
    assert_eq!(m2.iter().filter(|r| r.light > 0).count(), 100);
    let mut checked = 0;
    for (m, class) in m1.iter().zip(&rows.monsters) {
        let Some(class) = class else { continue };
        let x = &m2[usize::from(m.monstatsex)];
        assert_eq!(
            (class.light, class.light_rgb),
            (x.light, (x.light_r, x.light_g, x.light_b))
        );
        checked += 1;
    }
    assert!(
        checked > 600,
        "{checked} monstats rows with a monstats2 row"
    );
    assert!(rows
        .monsters
        .iter()
        .flatten()
        .any(|c| (c.light, c.light_rgb) == (5, (230, 168, 255))));
}

/// The Arreat Summit background on the user's archives: `summit01` and
/// `cloud01` load and give the 12 mountain cels (resolution mode 2) and
/// the 10 clouds' 20 cels, in pass 1, the clouds in draw mode 3 through
/// the act V tables.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_summit_background_draws_from_the_users_archives() {
    use d2_client::bridge::drlg::DrlgRoomId;
    use d2_client::bridge::world::{ActiveRoom, ClientUnit, UnitKey};
    use d2_client::rules::camera::{Camera, ClientPos, FrameSize};
    use d2_client::rules::shading::ShadeTables;
    use d2_client::scene::{order::pass, BlendOp};
    use d2_client::world_view::background_view::BackgroundView;
    use d2_client::world_view::{ViewAssets, WorldFrame};
    use d2_sim::rng::Seed;
    let d = app_support::live();
    let pl2 = d
        .archives
        .source()
        .read_file(r"data\global\palette\act5\pal.pl2")
        .expect("act V pal.pl2")
        .unwrap();
    let mut assets = ViewAssets::from_pl2(&pl2).unwrap();
    let pl2 = d2_formats::palette::Pl2::parse(&pl2).unwrap();
    assets.shades = Some(ShadeTables::push(&mut assets.maps, &pl2));
    let mut w = ClientWorld::default();
    let p = UnitKey::new(0, 1);
    let mut u = ClientUnit::new(p);
    u.position = Some((5000, 5000));
    w.units.insert(p, u);
    w.local_player = Some(p);
    w.active_rooms = Some(vec![ActiveRoom {
        x0: 4900,
        y0: 4900,
        w: 200,
        h: 200,
        level: 120,
        room: DrlgRoomId(1),
    }]);
    w.room_units.place(p, Some(DrlgRoomId(1)));
    let mut frame = WorldFrame {
        camera: Some(Camera::new(
            FrameSize::D2RUST,
            OpenMode::NONE,
            ClientPos { x: 0, y: 0 },
            (0, 0),
        )),
        ..WorldFrame::default()
    };
    let mut v = BackgroundView::new(d.archives.source(), Some(Seed::new(7, 666)));
    let log = v.add_to_frame(&w, 0, 10_063 + 2_056, &mut assets, &mut frame);
    assert!(log.is_empty(), "{log:?}");
    assert_eq!(frame.items.len(), 12 + 20);
    assert!(frame
        .items
        .iter()
        .all(|i| i.key.pass() == pass::LEVEL_BACKGROUND));
    let opaque = frame
        .items
        .iter()
        .filter(|i| i.blend == BlendOp::Opaque)
        .count();
    assert_eq!(opaque, 12, "the mountains opaque, the clouds blended");
}

/// The `states` rows the model's state messages and colour call read, on
/// the user's install (the live rows): seven states have
/// a `colorpri`, all with a `colorshift`; 90 `blue` 100 / 108 / 150, 215,
/// 255 and 2 `poison` 95 / 104 / 128, 255, 128.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn state_colour_rows_come_from_the_users_states() {
    let d = app_support::live();
    let rows = single_player::client_unit_rows(d.archives.as_ref()).unwrap();
    assert_eq!(rows.states.len(), 185);
    let coloured: Vec<usize> = (0..rows.states.len())
        .filter(|&s| rows.states[s].colorpri > 0)
        .collect();
    assert_eq!(coloured.len(), 7, "{coloured:?}");
    assert!(coloured.iter().all(|&s| rows.states[s].colorshift != 0));
    let row = |s: usize| {
        let r = rows.states[s];
        (r.colorpri, r.colorshift, r.light_rgb)
    };
    assert_eq!(row(90), (100, 108, (150, 215, 255)));
    assert_eq!(row(2), (95, 104, (128, 255, 128)));
}

/// The client update function the effect layer's flat rule reads
/// (`missiles` `pCltDoFunc`; functions 2 and 11 set flag 0x10000 at the
/// animation end), on the user's install: the effect rows carry each
/// row's value, and both functions are in use.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn effect_rows_carry_the_users_client_missile_functions() {
    use d2_data::tables::{decode_all, Missiles};
    let d = app_support::live();
    let a = d.archives.as_ref();
    let rows = d2_client::app::missile_art::effect_rows(a).unwrap();
    let set = d2_data::bin::load_from(a, "eng").unwrap();
    let table: Vec<Missiles> = decode_all(set.table("missiles").unwrap()).unwrap();
    assert_eq!(rows.missiles.len(), table.len());
    for (r, m) in rows.missiles.iter().zip(&table) {
        assert_eq!(r.clt_do_func, m.pcltdofunc);
    }
    for f in [2, 11] {
        let n = rows.missiles.iter().filter(|r| r.clt_do_func == f).count();
        assert!(n > 0, "function {f} unused");
        println!("pCltDoFunc {f}: {n} rows");
    }
}

/// The client create on the user's `missiles` rows: missile 287
/// `denofevillight` (`Light` 10; client function 23) created at a point
/// gets its kind-1 light of radius 10.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_den_light_missile_creates_from_the_users_rows() {
    use d2_client::bridge::client_missiles::{create, flag, CreateRecord};
    let d = app_support::live();
    let rows = single_player::client_unit_rows(d.archives.as_ref()).unwrap();
    assert_eq!(rows.missiles[287].clt_do_func, 23);
    assert_eq!(rows.missiles[287].light, 10);
    let mut w = ClientWorld::default();
    let rec = CreateRecord {
        flags: flag::POSITION,
        class: 287,
        x: 100,
        y: 100,
        ..CreateRecord::default()
    };
    let k = create(&mut w, &rows.missiles, &rec, true).unwrap().unwrap();
    let (_, light) = w.lights.iter().next().unwrap();
    assert_eq!((light.owner_guid, light.radius), (k.guid, 80));
}

/// The client missile update on the user's rows: the Den light (287,
/// function 23) never expires; an `arrow` (row 0, function 1) ends
/// after its frames (`Range`) and its light, if any, dies.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn client_missiles_update_on_the_users_rows() {
    use d2_client::bridge::client_missiles::{create, flag, update, CreateRecord};
    let d = app_support::live();
    let rows = single_player::client_unit_rows(d.archives.as_ref()).unwrap();
    let missiles = &rows.missiles;
    assert_eq!(missiles[0].clt_do_func, 1);
    let mut w = ClientWorld::default();
    let rec = |class| CreateRecord {
        flags: flag::POSITION,
        class,
        x: 100,
        y: 100,
        ..CreateRecord::default()
    };
    let den = create(&mut w, missiles, &rec(287), true).unwrap().unwrap();
    let arrow = create(&mut w, missiles, &rec(0), true).unwrap().unwrap();
    let frames = w.objclient.missiles[&arrow].current;
    assert_eq!(frames, i32::from(missiles[0].range));
    for n in 1..=frames {
        update(&mut w, missiles, arrow, true).unwrap();
        update(&mut w, missiles, den, true).unwrap();
        assert_eq!(
            w.objclient.set_c.contains_key(&arrow),
            n < frames,
            "update {n}"
        );
    }
    for _ in 0..1000 {
        update(&mut w, missiles, den, true).unwrap();
    }
    assert!(w.objclient.set_c.contains_key(&den));
}

/// The `pCltDoFunc` ids of the user's `missiles` rows by row count (a
/// listing for the queue, not a check): `--nocapture` prints them.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn client_missile_function_counts_on_the_users_rows() {
    let d = app_support::live();
    let rows = single_player::client_unit_rows(d.archives.as_ref()).unwrap();
    let mut n = std::collections::BTreeMap::new();
    for r in &rows.missiles {
        *n.entry(r.clt_do_func).or_insert(0usize) += 1;
    }
    let mut v: Vec<_> = n.into_iter().collect();
    v.sort_by_key(|&(f, c)| (std::cmp::Reverse(c), f));
    for (f, c) in &v {
        println!("pCltDoFunc {f}: {c}");
    }
    assert_eq!(v.iter().map(|(_, c)| c).sum::<usize>(), rows.missiles.len());
}

/// Every user `missiles` row whose client function the model runs
/// ([`RUN`]): created at a point with a player owner and a target 6
/// sub-tiles east, then 60 client updates of every client missile with
/// the user's skills tables and a drawn frame's unit origin around the
/// start; no handler error, and every function has a user row made.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_users_client_missiles_run_their_functions() {
    use d2_client::bridge::client_missiles::{create, flag, update_with, CreateRecord, Env};
    use d2_client::bridge::world::{ClientUnit, UnitKey, UnitOrigin};
    let d = app_support::live();
    let rows = single_player::client_unit_rows(d.archives.as_ref()).unwrap();
    let skills = single_player::client_skill_tables(d.archives.as_ref()).unwrap();
    let missiles = &rows.missiles;
    let mut w = ClientWorld::default();
    let p = UnitKey::new(0, 1);
    let mut u = ClientUnit::new(p);
    u.position = Some((100, 100));
    w.units.insert(p, u);
    // The camera on (100, 100): client pixel (0, 1600),
    // 800 × 600, mode 0.
    w.unit_origin = Some(UnitOrigin {
        x: -400,
        y: 1600 - 300 + 16 - 8,
        width: 800,
        play_height: 560,
    });
    let mut made = std::collections::BTreeMap::new();
    for (class, row) in missiles.iter().enumerate() {
        if !RUN.contains(&row.clt_do_func) {
            continue;
        }
        let rec = CreateRecord {
            flags: flag::POSITION | flag::TARGET_RELATIVE,
            owner: Some(p),
            class: class as u32,
            x: 100,
            y: 100,
            tx: 6,
            level: 1,
            ..CreateRecord::default()
        };
        if create(&mut w, missiles, &rec, true).unwrap().is_some() {
            *made.entry(row.clt_do_func).or_insert(0) += 1;
        }
    }
    assert_eq!(made.keys().copied().collect::<Vec<_>>(), RUN.to_vec());
    assert!(made.values().sum::<i32>() > 550, "{made:?}");
    let env = Env {
        rows: missiles,
        lights: true,
        skills: Some(&skills),
        monsters: &rows.monsters,
        model: None,
    };
    for _ in 0..60 {
        let keys: Vec<_> = w.objclient.missiles.keys().copied().collect();
        for k in keys {
            update_with(&mut w, &env, k).unwrap();
        }
    }
}

/// The client functions the model runs, in
/// order.
const RUN: [u16; 37] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 17, 18, 19, 20, 23, 25, 27, 37, 39, 43, 44, 45, 46, 47,
    48, 49, 51, 52, 53, 58, 59, 60, 63, 65, 68,
];

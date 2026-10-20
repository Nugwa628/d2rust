//! A game from the synthetic install, end to end and in CI: the
//! archives (tables, strings, AnimData and the DRLG's DS1 / DT1 files,
//! [`test_fixtures::drlg`]) → the loaded and fixed-up set →
//! [`GameData`] (table views, `world_data` providers) → a `WorldSim`
//! with act 0 created → the one-room town generated and streamed, its
//! waypoint and a player allocated → `SimGame::join` → `Host::frame`s.
//!
//! Expected values are the fixture's own content (the town's size, the
//! waypoint record) and the creation life and mana; everything else is
//! an invariant (no wiring error, no frame fault, the player stays in its
//! room, two runs are identical).

use std::path::PathBuf;
use std::sync::OnceLock;

use d2_data::tables::Objects;
use d2_server::adapters::handlers::world::ActionWorld;
use d2_server::adapters::session::{
    create_game, enter_game, Entry, GameSetup, HotKey, JoinError, PlayerRecord, SkillHand,
};
use d2_server::adapters::session_flow::{
    CharacterLoader, CreateGame, CreateRefusal, Loaded, SessionFault, SessionFlow,
};
use d2_server::adapters::storage::CharacterStore;
use d2_server::adapters::{PlayerData, PlayerFields, ProtoSizes, SimGame, UnitFacts};
use d2_server::host::Host;
use d2_server::seams::{ClientId, Clock, MessageSink, PlayerGate, Pos, SessionHandler};
use d2_sim::combat::vitals::init_player_stats;
use d2_sim::game::Game;
use d2_sim::stats::lists::NoHost;
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::interaction::{VitalsRest, VitalsView};
use d2_sim::wiring::worldgen::WorldSim;
use test_fixtures::content::{FLOOR_DT1, PRESET_DS1, PRESET_SIZE, WALL_DT1};
use test_fixtures::drlg::{archive_name, FIXED_LIBRARY, TOWN_WAYPOINT};
use test_fixtures::game::{ActCreation, GameData, Seams, Sim};
use test_fixtures::{install, synth};

/// The synthetic town (levels row 1, lvlprest Def 0).
const TOWN: u32 = 1;
/// Fixture choices (`run` stages the player at a fixed point; the
/// session join places it by the spawn search, `join_run`).
const INIT: u32 = 0x1234_5678;
const GAME_SEED: u32 = 1234;
const CLASS: u32 = 3;
const CLIENT: ClientId = 0;
const FRAMES: usize = 50;
/// Sub-tiles per tile.
const SUB: i32 = 5;

fn data() -> &'static GameData {
    static D: OnceLock<GameData> = OnceLock::new();
    D.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("synthetic-game-{}", std::process::id()));
        let i = install::build(&dir, &synth::synthetic()).unwrap_or_else(|e| panic!("{e}"));
        GameData::from_install(&i).unwrap_or_else(|e| panic!("{e}"))
    })
}

struct NoRest;
impl VitalsRest for NoRest {
    fn refresh(&mut self, _: UnitId) {}
    fn level_up_notify(&mut self, _: UnitId) {}
    fn level_up_event(&mut self, _: UnitId) {}
}

struct NoSession;
impl SessionHandler for NoSession {
    fn system_message(&mut self, _: ClientId, _: &[u8], _: usize, _: &mut dyn MessageSink) {}
}

struct Ms(u32);
impl Clock for Ms {
    fn now_ms(&mut self) -> u32 {
        self.0
    }
}

type TestHost = Host<Sim, ProtoSizes, NoSession, Ms>;

/// What one run leaves behind, compared across runs.
#[derive(Debug, PartialEq, Eq)]
struct Run {
    player: UnitId,
    pos: (i32, i32),
    room_rect: (i32, i32, i32, i32),
    waypoint: (i32, i32),
    life: i32,
    received: Vec<Vec<u8>>,
}

#[test]
fn providers_hold_the_named_files() {
    let d = data();
    // Every lvlprest DS1 (4), lvlsub DS1 (1), lvltypes DT1 (2 floors + the
    // wall) and the fixed library (3).
    assert_eq!(d.files.ds1.0.len(), 4);
    assert_eq!(d.files.subs.0.len(), 1);
    assert_eq!(d.files.dt1.0.len(), 6);
    let keys: Vec<String> = d
        .files
        .ds1
        .0
        .keys()
        .map(|k| String::from_utf8_lossy(k).into_owned())
        .collect();
    let town = d
        .files
        .ds1
        .0
        .get(archive_name(PRESET_DS1[0]).as_bytes())
        .unwrap_or_else(|| panic!("town DS1 not among {keys:?}"));
    // Stored size = lvlprest SizeX / SizeY.
    assert_eq!((town.width, town.height), (PRESET_SIZE, PRESET_SIZE));
    let (kind, id, x, y) = TOWN_WAYPOINT;
    assert_eq!(town.objects.len(), 1);
    let o = &town.objects[0];
    assert_eq!((o.kind, o.id, o.x, o.y), (kind, id, x, y));
    for lib in FIXED_LIBRARY {
        assert!(d.files.dt1.0.contains_key(lib.as_bytes()), "{lib}");
    }
    // The lvltypes and lvlprest strings are fixed up to archive names,
    // which the provider keys on.
    for f in FLOOR_DT1.iter().chain([&WALL_DT1]) {
        assert!(
            d.files.dt1.0.contains_key(archive_name(f).as_bytes()),
            "{f}"
        );
    }
    // The archive name the provider asks with is the fixture's.
    assert_eq!(
        d2_server::world_data::archive_name(PRESET_DS1[0].as_bytes()),
        archive_name(PRESET_DS1[0]).into_bytes()
    );
}

fn run() -> Run {
    let d = data();
    let (mut sim, types) = d
        .world_sim(
            ActCreation::TownOnly,
            INIT,
            TOWN,
            GAME_SEED,
            Seams::default(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    let vitals = d.vitals().unwrap();
    let objects: Vec<Objects> = d.rows().unwrap();

    // The town generated and its one room streamed.
    let mut game = Game::new();
    game.lists.ensure_act(0).unwrap();
    let (room, rect, wp) = sim
        .action
        .hooks()
        .drlg
        .with_act(0, &mut game.lists, |dr, svc| {
            let lv = dr.get_or_alloc_level(svc.data, svc.types, TOWN)?;
            if dr.level_rooms(lv).is_empty() {
                dr.generate_level(svc.data, svc.types, lv)?;
            }
            let rooms = dr.level_rooms(lv);
            assert_eq!(rooms.len(), 1, "a one-room town");
            let r = rooms[0];
            // The room build moves the map's preset units onto the
            // room's list, room-relative.
            let active = dr.stream_room(svc, r)?;
            let t = types.borrow();
            let presets = t.act_presets(0).expect("act 0 presets");
            let units = presets.room_units(r);
            assert_eq!(units.len(), 1, "the waypoint preset");
            let u = &units[0];
            assert_eq!(u.unit_type, 2);
            let class = u.class;
            assert_eq!(
                objects[class as usize].operatefn, 23,
                "the waypoint row (objects row 0)"
            );
            let wp = (u.x, u.y, class as u32);
            drop(t);
            let rect = dr.room(r).rect;
            // Every cell of the (w + 1) × (h + 1) grid has the floor bit:
            // one floor record each, chosen from the written floor tile
            // (key (0, 0, 0), not the (10, 0, 0) fallback).
            let tiles = dr.room(r).tiles().expect("the room's tiles");
            assert_eq!(
                tiles.floors.len(),
                ((rect.w + 1) * (rect.h + 1)) as usize,
                "floor records"
            );
            for f in &tiles.floors {
                let t = dr.tile_info(f.tile);
                assert_eq!((t.orientation, t.main, t.sub), (0, 0, 0));
            }
            assert!(tiles.walls.is_empty() && tiles.shadows.is_empty());
            Ok::<_, d2_sim::drlg::DrlgError>((active, rect, wp))
        })
        .expect("act 0 has a DRLG")
        .unwrap_or_else(|e| panic!("town: {e:?}"));
    let room = room.expect("the town room is active");
    assert_eq!(sim.errors(), Vec::<String>::new(), "game creation");
    // The preset is the whole room: PRESET_SIZE tiles square.
    assert_eq!((rect.w, rect.h), (PRESET_SIZE as i32, PRESET_SIZE as i32));

    // Room-relative preset sub-tiles + the room's origin.
    let wp_at = (rect.x * SUB + wp.0, rect.y * SUB + wp.1);
    let start = (wp_at.0 + 5, wp_at.1 + 5);
    let mut alloc = |ty, class, (x, y): (i32, i32)| {
        let req = AllocRequest {
            ty,
            class,
            room: Some(room),
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: ty == UnitType::Player,
        };
        sim.action
            .with(&mut game, |g, v| v.allocate(g, &req, x, y))
            .expect("allocated")
    };
    let object = alloc(UnitType::Object, wp.2, wp_at);
    let player = alloc(UnitType::Player, CLASS, start);
    sim.action.sys.units.get_mut(player).unwrap().mode = 1;
    {
        let s = &mut sim.action.sys;
        let mut v = VitalsView {
            units: &s.units,
            stats: &mut s.stats,
            hooks: &mut NoHost,
            rest: &mut NoRest,
        };
        init_player_stats(&mut v, &vitals, player, 0);
    }
    // Creation life ((vit + hpadd) << 8) from the
    // synthetic charstats row.
    let cs = vitals.charstats(CLASS as i32).expect("charstats row");
    let life = (i32::from(cs.vit) + i32::from(cs.hpadd)) << 8;
    assert_eq!(sim.action.sys.stats.unit_total(player, 6, 0), life);

    let world = ActionWorld {
        waypoints: Some(d.waypoints().unwrap()),
        ..ActionWorld::default()
    };
    let mut s: Sim = SimGame::with_world(game, sim, world);
    s.join(CLIENT, Some(player), None, client_state::IN_GAME)
        .expect("join");
    s.set_player(
        player,
        PlayerFields {
            gate: PlayerGate {
                mode: 1,
                uninterruptable: false,
            },
            data: Some(PlayerData { last_accept: 0 }),
        },
    );
    s.set_unit(
        object,
        UnitFacts {
            act: 0,
            pos: Pos {
                x: wp_at.0,
                y: wp_at.1,
            },
            owner: None,
        },
    );
    let mut host: TestHost = Host::new(s, ProtoSizes, NoSession, Ms(1000));
    host.connect(CLIENT);
    // The first frame starts the tick timer (no tick).
    host.frame().expect("first frame");
    let mut received = host.receive(CLIENT);
    for f in 0..FRAMES {
        host.clock.0 += 40;
        let r = host.frame().unwrap_or_else(|e| panic!("frame {f}: {e:?}"));
        assert!(r.ticked, "one tick per frame");
        received.extend(host.receive(CLIENT));
        let s = &host.game;
        assert!(s.tick_faults.is_empty(), "frame {f}: {:?}", s.tick_faults);
        assert!(s.world.faults.is_empty(), "frame {f}: {:?}", s.world.faults);
        assert_eq!(s.events.errors(), Vec::<String>::new(), "frame {f}");
    }
    let s = &mut host.game;
    assert!(
        s.game.lists.unit(player).is_some(),
        "the player is still there"
    );
    // The player's position (its path).
    let pos = s.events.action.hooks().path_position(player);
    assert!(
        pos.0 >= rect.x * SUB
            && pos.0 < (rect.x + rect.w) * SUB
            && pos.1 >= rect.y * SUB
            && pos.1 < (rect.y + rect.h) * SUB,
        "player at {pos:?} outside the town room {rect:?}"
    );
    Run {
        player,
        pos,
        room_rect: (rect.x, rect.y, rect.w, rect.h),
        waypoint: wp_at,
        life,
        received,
    }
}

#[test]
fn town_join_and_frames() {
    let a = run();
    // This run stages the player in the town room without the session
    // join (`town_entry_sends_the_join_sequence` runs that): the count of
    // what the idle client receives is printed, and compared across runs.
    println!(
        "player {:?} at {:?} in {:?}, waypoint {:?}, {} messages",
        a.player,
        a.pos,
        a.room_rect,
        a.waypoint,
        a.received.len()
    );
    // Determinism on the synthetic data: a second game is identical.
    assert_eq!(run(), a);
}

/// The character name of the join test (zero-padded, 0x59 bytes 6..22).
fn name() -> [u8; 16] {
    let mut n = [0u8; 16];
    n[..6].copy_from_slice(b"werwer");
    n
}

/// The join test's character: a reference record
/// (hand 1 skill 0, hand 0 skill 36, no item), portal flags 5, hot keys in
/// slot 3 (skill 0, flag set) and slot 7 (a skill id past the `skills`
/// rows: not sent).
fn entry(skills: usize) -> Entry {
    let mut e = Entry::new(0, name());
    e.hotkeys[3] = HotKey {
        skill: 0,
        flag: true,
        item: u32::MAX,
    };
    e.hotkeys[7] = HotKey {
        skill: skills as i16,
        flag: false,
        item: 9,
    };
    e.record = Some(PlayerRecord {
        portal_flags: 5,
        hands: [
            SkillHand {
                skill: 36,
                item: u32::MAX,
            },
            SkillHand {
                skill: 0,
                item: u32::MAX,
            },
        ],
    });
    e
}

/// What the client receives from the session join (`enter_game`) and the
/// next ticks, with the facts the expected bytes are built from.
#[derive(Debug, PartialEq, Eq)]
struct Joined {
    guid: u32,
    obj_seed: u32,
    room_rect: (i32, i32, i32, i32),
    pos: (i32, i32),
    /// The waypoint's GUID, class, position, mode and interact byte.
    waypoint: (u32, u16, (i32, i32), u8, u8),
    /// The client's state after the frames.
    state: u32,
    /// Messages of the first flush (the join and tick 1).
    first: usize,
    /// `skills` rows of the game.
    skills: usize,
    received: Vec<Vec<u8>>,
}

/// The game of the join test (Normal,
/// expansion, not ladder, the reference arena flags).
const SETUP: GameSetup = GameSetup {
    difficulty: 0,
    arena_flags: 0x0010_0004,
    expansion: true,
    ladder: false,
};

/// The town of [`run`] with its waypoint, an unplaced player, its client
/// record, then `create_game`, `enter_game` and `FRAMES` frames.
fn join_run() -> Joined {
    join_with(|s, player| {
        let skills = s.events.action.hooks().tables.skills.skills.len();
        let entry = entry(skills);
        assert_eq!(enter_game(s, CLIENT, &entry), Ok(player));
        // A second entry of the placed player is refused.
        assert_eq!(
            enter_game(s, CLIENT, &entry),
            Err(JoinError::Placed(player))
        );
    })
}

/// The town of the join tests before any client: the game with its
/// waypoint, and the facts the expected bytes are built from.
struct Town {
    s: Sim,
    rect: (i32, i32, i32, i32),
    wp: UnitId,
    wp_class: u32,
    wp_at: (i32, i32),
    obj_seed: u32,
}

/// The town of [`run`] with its waypoint, as `SimGame` (no client).
fn town() -> Town {
    let d = data();
    let (mut sim, _) = d
        .world_sim(
            ActCreation::TownOnly,
            INIT,
            TOWN,
            GAME_SEED,
            Seams::default(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    let mut game = Game::new();
    game.lists.ensure_act(0).unwrap();
    let (room, rect) = sim
        .action
        .hooks()
        .drlg
        .with_act(0, &mut game.lists, |dr, svc| {
            let lv = dr.get_or_alloc_level(svc.data, svc.types, TOWN)?;
            let r = dr.level_rooms(lv)[0];
            Ok::<_, d2_sim::drlg::DrlgError>((dr.stream_room(svc, r)?, dr.room(r).rect))
        })
        .expect("act 0 has a DRLG")
        .unwrap_or_else(|e| panic!("town: {e:?}"));
    let room = room.expect("the town room is active");
    let (_, _, wx, wy) = TOWN_WAYPOINT;
    let wp_at = (rect.x * SUB + wx as i32, rect.y * SUB + wy as i32);
    let objects: Vec<Objects> = d.rows().unwrap();
    let wp_class = objects
        .iter()
        .position(|o| o.operatefn == 23)
        .expect("the waypoint row") as u32;
    let req = AllocRequest {
        ty: UnitType::Object,
        class: wp_class,
        room: Some(room),
        add: true,
        fixed_guid: None,
        mode: 1,
        allied: false,
    };
    let wp = sim
        .action
        .with(&mut game, |g, v| v.allocate(g, &req, wp_at.0, wp_at.1))
        .expect("allocated");
    let obj_seed = sim.action.hooks().objects.as_ref().unwrap().obj_seed;
    let world = ActionWorld {
        waypoints: Some(d.waypoints().unwrap()),
        ..ActionWorld::default()
    };
    Town {
        s: SimGame::with_world(game, sim, world),
        rect: (rect.x, rect.y, rect.w, rect.h),
        wp,
        wp_class,
        wp_at,
        obj_seed,
    }
}

/// The player as the save loader leaves it: no room, at (0, 0), mode 1.
fn alloc_player(s: &mut Sim, class: u32) -> UnitId {
    let req = AllocRequest {
        ty: UnitType::Player,
        class,
        room: None,
        add: true,
        fixed_guid: None,
        mode: 1,
        allied: true,
    };
    let p = s
        .events
        .action
        .with(&mut s.game, |g, v| v.allocate(g, &req, 0, 0))
        .expect("allocated");
    s.events.action.sys.units.get_mut(p).unwrap().mode = 1;
    p
}

/// [`join_run`] with the entry step `enter` (the client is joined, its
/// player unplaced).
fn join_with(enter: impl FnOnce(&mut Sim, UnitId)) -> Joined {
    let mut t = town();
    let s = &mut t.s;
    let player = alloc_player(s, CLASS);
    // The client record as its allocation leaves it (state 0); game
    // creation sets state 1, the join 2 then 3.
    s.join(CLIENT, Some(player), None, 0).expect("join");
    create_game(s, CLIENT, &SETUP).expect("game creation");
    enter(s, player);
    let id = s.sim_client(CLIENT).unwrap();
    assert_eq!(
        s.game.lists.client(id).unwrap().state,
        client_state::JOINING
    );
    let mut host: TestHost = Host::new(t.s, ProtoSizes, NoSession, Ms(1000));
    host.connect(CLIENT);
    host.frame().expect("first frame");
    let received = host.receive(CLIENT);
    run_frames(
        host, received, &t.rect, t.wp, t.wp_class, t.wp_at, t.obj_seed,
    )
}

/// `FRAMES` frames of a joined host, then what the client received and
/// the facts of the join (the waypoint's 0x51 fields read at the end:
/// it does not change).
fn run_frames(
    mut host: TestHost,
    mut received: Vec<Vec<u8>>,
    rect: &(i32, i32, i32, i32),
    wp: UnitId,
    wp_class: u32,
    wp_at: (i32, i32),
    obj_seed: u32,
) -> Joined {
    // The first flush (the host's first frame starts the tick driver's
    // clock and may not tick).
    let mut first = None;
    for f in 0..FRAMES {
        host.clock.0 += 40;
        host.frame().unwrap_or_else(|e| panic!("frame {f}: {e:?}"));
        received.extend(host.receive(CLIENT));
        if first.is_none() && !received.is_empty() {
            first = Some(received.len());
        }
        let s = &host.game;
        assert!(s.tick_faults.is_empty(), "frame {f}: {:?}", s.tick_faults);
        assert_eq!(s.events.errors(), Vec::<String>::new(), "frame {f}");
    }
    let s = &mut host.game;
    let id = s.sim_client(CLIENT).unwrap();
    let player = s.player_of(CLIENT).expect("the client's player");
    let guid = s.game.lists.unit(player).unwrap().guid;
    let wp_guid = s.game.lists.unit(wp).unwrap().guid;
    let skills = s.events.action.hooks().tables.skills.skills.len();
    let a = &mut s.events.action;
    let wp_mode = a.sys.units.get(wp).unwrap().mode as u8;
    let wp_interact = a
        .hooks()
        .objects
        .as_ref()
        .and_then(|o| o.control.data.get(&wp))
        .map_or(0, |d| d.interact);
    let pos = a.hooks().path_position(player);
    let state = s.game.lists.client(id).unwrap().state;
    Joined {
        guid,
        obj_seed,
        room_rect: *rect,
        pos,
        waypoint: (wp_guid, wp_class as u16, wp_at, wp_mode, wp_interact),
        state,
        first: first.unwrap_or(0),
        skills,
        received,
    }
}

#[test]
fn town_entry_sends_the_join_sequence() {
    let j = join_run();
    let (rx, ry, rw, rh) = j.room_rect;
    // The spawn search put the player in the one town room.
    let (x, y) = j.pos;
    assert!(
        x >= rx * SUB && x < (rx + rw) * SUB && y >= ry * SUB && y < (ry + rh) * SUB,
        "player at {:?} outside the town room {:?}",
        j.pos,
        j.room_rect
    );
    assert!(j.skills > 0, "the synthetic skills table has rows");
    let want = join_messages(&j);
    assert_eq!(
        j.received[..want.len()].to_vec(),
        want,
        "join prefix of {:02x?}",
        j.received
    );
    // The first tick populates the town room, so the client's room is
    // ready: 0x04 once, the client in game.
    let rest = &j.received[want.len()..];
    assert_eq!(rest.iter().filter(|m| m[..] == [0x04]).count(), 1);
    let at = j.received.iter().position(|m| m[..] == [0x04]).unwrap();
    assert!(
        at < j.first,
        "0x04 at {at}, after the first flush's {}",
        j.first
    );
    assert_eq!(j.state, client_state::IN_GAME);
    // No further room comes into sight while the player stands still.
    assert!(!rest.iter().any(|m| m[0] == 0x07 || m[0] == 0x08));
    // Determinism on the synthetic data.
    assert_eq!(join_run(), j);
}

/// The 0x67 of the session tests: the fixture's name, Normal, `flags`,
/// locale 0 (`client-messages.tsv` row 0x67).
fn create_request(class: u8, flags: u32) -> CreateGame {
    let mut game_name = [0u8; 16];
    game_name[..4].copy_from_slice(b"game");
    CreateGame {
        game_name,
        class,
        char_name: name(),
        flags,
        ..CreateGame::default()
    }
}

/// Expansion (bit 20) and bit 2.
const EXPANSION_FLAGS: u32 = (1 << 20) | 0x4;

/// The town with a session flow whose loader allocates the player of the
/// request's class with [`entry`]'s values, behind a host; the loader's
/// calls are counted in `loads`.
fn session_host(loads: std::rc::Rc<std::cell::Cell<u32>>) -> (TestHost, TownFacts) {
    let mut t = town();
    let loader: CharacterLoader<WorldSim<Seams>, ActionWorld> =
        Box::new(move |s: &mut Sim, _: ClientId, r: &CreateGame| {
            loads.set(loads.get() + 1);
            let player = alloc_player(s, u32::from(r.class));
            let skills = s.events.action.hooks().tables.skills.skills.len();
            let mut entry = entry(skills);
            entry.name = r.char_name;
            Ok(Loaded { player, entry })
        });
    t.s.set_session(SessionFlow::new(loader));
    let Town {
        s,
        rect,
        wp,
        wp_class,
        wp_at,
        obj_seed,
    } = t;
    let mut host: TestHost = Host::new(s, ProtoSizes, NoSession, Ms(1000));
    host.connect(CLIENT);
    (host, (rect, wp, wp_class, wp_at, obj_seed))
}

/// [`Town`]'s facts without the game.
type TownFacts = ((i32, i32, i32, i32), UnitId, u32, (i32, i32), u32);

fn faults(host: &TestHost) -> &[(ClientId, SessionFault)] {
    &host.game.session().expect("a session flow").faults
}

#[test]
fn session_messages_run_creation_then_the_join() {
    let loads = std::rc::Rc::new(std::cell::Cell::new(0));
    let (mut host, t) = session_host(loads.clone());
    // C→S 0x67 in the first drain: 0x01, 0x00,
    // 0x02, with the first tick's flush.
    let req = create_request(CLASS as u8, EXPANSION_FLAGS);
    host.send_system(CLIENT, &req.encode()).expect("queued");
    host.frame().expect("frame 1");
    let mut received = host.receive(CLIENT);
    host.clock.0 += 40;
    let f = host.frame().expect("frame 2");
    assert!(f.ticked);
    received.extend(host.receive(CLIENT));
    assert_eq!(received, join_messages_prefix());
    let id = host.game.sim_client(CLIENT).expect("the client record");
    assert_eq!(
        host.game.game.lists.client(id).unwrap().state,
        client_state::LOADING
    );
    assert_eq!(loads.get(), 0, "the character loads at the join");
    // C→S 0x6B after tick 1: the join, then the next tick's 0x04.
    host.send_system(CLIENT, &[0x6B]).expect("queued");
    let (rect, wp, wp_class, wp_at, obj_seed) = t;
    let j = run_frames(host, received, &rect, wp, wp_class, wp_at, obj_seed);
    assert_eq!(loads.get(), 1);
    let want = join_messages(&j);
    assert_eq!(
        j.received[..want.len()].to_vec(),
        want,
        "session prefix of {:02x?}",
        j.received
    );
    let rest = &j.received[want.len()..];
    assert_eq!(rest.iter().filter(|m| m[..] == [0x04]).count(), 1);
    assert_eq!(j.state, client_state::IN_GAME);
}

/// The three game-creation messages of [`SETUP`].
fn join_messages_prefix() -> Vec<Vec<u8>> {
    vec![
        vec![0x01, 0x00, 0x04, 0x00, 0x10, 0x00, 0x01, 0x00],
        vec![0x00],
        vec![0x02],
    ]
}

#[test]
fn session_refusals_stop_the_sequence() {
    let loads = std::rc::Rc::new(std::cell::Cell::new(0));
    let (mut host, _t) = session_host(loads.clone());
    // 0x6B with no client record: log, stop.
    host.send_system(CLIENT, &[0x6B]).expect("queued");
    // 0x67's checks: locale > 14, neither flag bit 1 nor 2,
    // class ≥ 7: nothing happens.
    let mut bad = create_request(CLASS as u8, EXPANSION_FLAGS);
    bad.locale = 15;
    host.send_system(CLIENT, &bad.encode()).expect("queued");
    host.send_system(CLIENT, &create_request(CLASS as u8, 1 << 20).encode())
        .expect("queued");
    host.send_system(CLIENT, &create_request(7, EXPANSION_FLAGS).encode())
        .expect("queued");
    // A classic game (no bit 20) with an expansion class: load result
    // 0x18, the client removed, the loader not called.
    host.send_system(CLIENT, &create_request(5, 0x4).encode())
        .expect("queued");
    host.send_system(CLIENT, &[0x6B]).expect("queued");
    host.frame().expect("frame 1");
    host.clock.0 += 40;
    host.frame().expect("frame 2");
    assert_eq!(
        faults(&host),
        [
            (CLIENT, SessionFault::NoClientRecord),
            (
                CLIENT,
                SessionFault::CreateRefused(CreateRefusal::Locale(15))
            ),
            (
                CLIENT,
                SessionFault::CreateRefused(CreateRefusal::Flags(1 << 20))
            ),
            (CLIENT, SessionFault::CreateRefused(CreateRefusal::Class(7))),
            (CLIENT, SessionFault::LoadRefused(0x18)),
        ]
    );
    assert_eq!(loads.get(), 0);
    assert_eq!(host.game.sim_client(CLIENT), None);
    // The accepted 0x67's creation messages were buffered, but the client
    // left the game's list before the flush, which walks that list:
    // only the refusal's direct S→C 0xB4 with the load result arrives.
    assert_eq!(host.receive(CLIENT), vec![vec![0xB4, 0x18, 0, 0, 0]]);
}

#[test]
fn create_checks_run_in_their_order() {
    let loads = std::rc::Rc::new(std::cell::Cell::new(0));
    let (mut host, _t) = session_host(loads.clone());
    let other: ClientId = 1;
    host.connect(other);
    // No NUL in the character name's 16 bytes; in the game name's.
    let mut bad = create_request(CLASS as u8, EXPANSION_FLAGS);
    bad.char_name = [b'a'; 16];
    host.send_system(CLIENT, &bad.encode()).expect("queued");
    let mut bad = create_request(CLASS as u8, EXPANSION_FLAGS);
    bad.game_name = [b'g'; 16];
    host.send_system(CLIENT, &bad.encode()).expect("queued");
    // The character-name test runs before the locale test.
    let mut bad = create_request(CLASS as u8, EXPANSION_FLAGS);
    bad.char_name = [b'a'; 16];
    bad.locale = 15;
    host.send_system(CLIENT, &bad.encode()).expect("queued");
    // Accepted; then the same client again, before the class test.
    host.send_system(
        CLIENT,
        &create_request(CLASS as u8, EXPANSION_FLAGS).encode(),
    )
    .expect("queued");
    host.send_system(CLIENT, &create_request(7, EXPANSION_FLAGS).encode())
        .expect("queued");
    // Another client with the same name in another case (names compare
    // without case).
    let mut same = create_request(CLASS as u8, EXPANSION_FLAGS);
    same.char_name
        .iter_mut()
        .for_each(|c| *c = c.to_ascii_uppercase());
    assert_ne!(same.char_name, name());
    host.send_system(other, &same.encode()).expect("queued");
    host.frame().expect("frame 1");
    assert_eq!(
        faults(&host),
        [
            (CLIENT, SessionFault::CreateRefused(CreateRefusal::CharName)),
            (CLIENT, SessionFault::CreateRefused(CreateRefusal::GameName)),
            (CLIENT, SessionFault::CreateRefused(CreateRefusal::CharName)),
            (
                CLIENT,
                SessionFault::CreateRefused(CreateRefusal::HasGame(name()))
            ),
            (
                other,
                SessionFault::CreateRefused(CreateRefusal::NameTaken(name()))
            ),
        ]
    );
    assert!(host.game.sim_client(CLIENT).is_some());
    assert_eq!(host.game.sim_client(other), None);
}

/// A host whose client is in game (state 4) through 0x67 and 0x6B.
fn in_game_host() -> TestHost {
    let loads = std::rc::Rc::new(std::cell::Cell::new(0));
    let (mut host, _t) = session_host(loads);
    host.send_system(
        CLIENT,
        &create_request(CLASS as u8, EXPANSION_FLAGS).encode(),
    )
    .expect("queued");
    host.frame().expect("frame 1");
    host.clock.0 += 40;
    host.frame().expect("frame 2");
    host.send_system(CLIENT, &[0x6B]).expect("queued");
    for _ in 0..4 {
        host.clock.0 += 40;
        host.frame().expect("frame");
    }
    let id = host.game.sim_client(CLIENT).expect("the client record");
    assert_eq!(
        host.game.game.lists.client(id).unwrap().state,
        client_state::IN_GAME
    );
    host.receive(CLIENT);
    host
}

#[test]
fn leave_sends_its_messages_and_removes_the_client() {
    let mut host = in_game_host();
    host.send_system(CLIENT, &[0x69]).expect("queued");
    host.frame().expect("frame");
    // The direct 0xB0 is on the system list (received first), then the
    // flushed 0x05 and 0x06; single player has nobody left for the 0x5A.
    assert_eq!(host.receive(CLIENT), [vec![0xB0], vec![0x05], vec![0x06]]);
    assert_eq!(host.game.sim_client(CLIENT), None);
    // No character storage is installed: the save is named as not written.
    assert_eq!(faults(&host), [(CLIENT, SessionFault::NotSaved)]);
    // M08: a second leave without a record does nothing.
    host.send_system(CLIENT, &[0x69]).expect("queued");
    host.clock.0 += 40;
    host.frame().expect("frame");
    assert_eq!(host.receive(CLIENT), Vec::<Vec<u8>>::new());
}

/// The join sequence of the first tick in state 3: after 0x04,
/// S→C 0x5B (the player's GUID, class, name, level, no party), 0x65
/// (kill count 0) and the join 0x5A code 2 with the name, so the
/// client's roster holds the player. When
/// it leaves, another client in state 4 gets 0x5C (the leaver's GUID)
/// then the leave 0x5A code 3; the leaver gets neither.
#[test]
fn join_and_leave_send_the_roster_messages() {
    use d2_sim::units::messages as msg;
    let loads = std::rc::Rc::new(std::cell::Cell::new(0));
    let (mut host, _t) = session_host(loads);
    host.send_system(
        CLIENT,
        &create_request(CLASS as u8, EXPANSION_FLAGS).encode(),
    )
    .expect("queued");
    host.frame().expect("frame 1");
    host.clock.0 += 40;
    host.frame().expect("frame 2");
    host.send_system(CLIENT, &[0x6B]).expect("queued");
    let mut got = Vec::new();
    for _ in 0..4 {
        host.clock.0 += 40;
        host.frame().expect("frame");
        got.extend(host.receive(CLIENT));
    }
    let p = host.game.player_of(CLIENT).expect("player");
    let guid = host.game.game.lists.unit(p).unwrap().guid;
    let level = host.game.events.action.sys.stats.unit_total(p, 12, 0) as u16;
    let at = got.iter().position(|m| m == &[0x04]).expect("0x04");
    // q-fix-flow-server: the state-3 inventory refresh's 0x48 after 0x04, and the
    // join's 0x8D between 0x65 and 0x5A.
    let g = guid.to_le_bytes();
    assert_eq!(
        got[at + 1..at + 6],
        [
            vec![0x48, 0, 0, g[0], g[1], g[2], g[3], 0, 0, 0, 0],
            msg::player_joined(guid, CLASS as u8, &name(), level, 0xFFFF),
            msg::player_kill_count(guid, 0).to_vec(),
            msg::assign_player_to_party(guid, msg::NO_PARTY).to_vec(),
            msg::player_event(2, &name()).to_vec(),
        ]
    );
    assert_eq!(&got[at + 2][8..14], b"werwer");

    // A second client in game (state 4) watches the first leave.
    let other: ClientId = 1;
    host.connect(other);
    let p2 = alloc_player(&mut host.game, CLASS);
    host.game
        .join(other, Some(p2), None, client_state::IN_GAME)
        .unwrap();
    host.send_system(CLIENT, &[0x69]).expect("queued");
    host.clock.0 += 40;
    host.frame().expect("frame");
    assert_eq!(host.receive(CLIENT), [vec![0xB0], vec![0x05], vec![0x06]]);
    let left: Vec<Vec<u8>> = host
        .receive(other)
        .into_iter()
        .filter(|m| matches!(m[0], 0x5C | 0x5A))
        .collect();
    assert_eq!(
        left,
        [
            msg::player_left(guid).to_vec(),
            msg::player_event(3, &name()).to_vec()
        ]
    );
}

/// A second client's 0x67 joins the running game: it is told the game's
/// own settings (not the Hell it asked for), loads its own character, and
/// the game counts two players, then one after the first leaves.
#[test]
fn a_second_client_joins_with_the_games_settings() {
    let loads = std::rc::Rc::new(std::cell::Cell::new(0));
    let (mut host, _t) = session_host(loads.clone());
    let step = |host: &mut TestHost, n: usize, c: ClientId| {
        let mut got = Vec::new();
        for _ in 0..n {
            host.clock.0 += 40;
            host.frame().expect("frame");
            got.extend(host.receive(c));
        }
        got
    };
    host.send_system(
        CLIENT,
        &create_request(CLASS as u8, EXPANSION_FLAGS).encode(),
    )
    .expect("queued");
    step(&mut host, 2, CLIENT);
    host.send_system(CLIENT, &[0x6B]).expect("queued");
    step(&mut host, 4, CLIENT);
    assert_eq!(host.game.events.world.init_info.players, 1);

    let other: ClientId = 1;
    host.connect(other);
    let mut req = create_request(CLASS as u8, EXPANSION_FLAGS);
    req.difficulty = 2;
    req.char_name = [0; 16];
    req.char_name[..6].copy_from_slice(b"second");
    host.send_system(other, &req.encode()).expect("queued");
    let got = step(&mut host, 2, other);
    assert_eq!(got[..3], join_messages_prefix()[..], "the game's own 0x01");
    host.send_system(other, &[0x6B]).expect("queued");
    step(&mut host, 4, other);
    assert_eq!(loads.get(), 2);
    let (a, b) = (host.game.player_of(CLIENT), host.game.player_of(other));
    assert!(a.is_some() && b.is_some() && a != b, "{a:?} {b:?}");
    assert_eq!(host.game.events.world.init_info.players, 2);

    let gone = a.unwrap();
    host.send_system(CLIENT, &[0x69]).expect("queued");
    step(&mut host, 1, other);
    assert_eq!(host.game.player_of(CLIENT), None);
    assert!(host.game.game.lists.unit(gone).is_none(), "the player left");
    assert!(host.game.game.lists.unit(b.unwrap()).is_some());
    assert_eq!(host.game.events.world.init_info.players, 1);
}

/// A character store that records each save: the client, the frame,
/// and whether the client was still in game (state 4) when it ran.
struct Recorder(std::sync::Arc<std::sync::Mutex<Vec<(ClientId, i32, bool)>>>);

impl CharacterStore<WorldSim<Seams>, ActionWorld> for Recorder {
    fn save(&mut self, sim: &mut Sim, client: ClientId) -> Result<(), String> {
        let in_game = sim
            .sim_client(client)
            .and_then(|i| sim.game.lists.client(i))
            .is_some_and(|e| e.state == client_state::IN_GAME);
        self.0
            .lock()
            .unwrap()
            .push((client, sim.game.frame, in_game));
        Ok(())
    }
}

type Saves = std::sync::Arc<std::sync::Mutex<Vec<(ClientId, i32, bool)>>>;

fn with_recorder(host: &mut TestHost) -> Saves {
    let saves = Saves::default();
    host.game.set_storage(Box::new(Recorder(saves.clone())));
    saves
}

#[test]
fn leave_saves_the_character_before_its_messages() {
    let mut host = in_game_host();
    let saves = with_recorder(&mut host);
    let frame = host.game.game.frame;
    host.send_system(CLIENT, &[0x69]).expect("queued");
    host.frame().expect("frame");
    // Saved once, in the drain (no tick ran), while the client was still
    // in game: before the 0x05, 0x06, 0xB0 and the removal.
    assert_eq!(*saves.lock().unwrap(), [(CLIENT, frame, true)]);
    assert_eq!(host.receive(CLIENT), [vec![0xB0], vec![0x05], vec![0x06]]);
    assert_eq!(host.game.sim_client(CLIENT), None);
    assert_eq!(faults(&host), []);
}

#[test]
fn three_periods_of_ticks_save_three_times() {
    let mut host = in_game_host();
    let saves = with_recorder(&mut host);
    let start = host.game.game.frame;
    while host.game.game.frame < start + 3 * 8192 {
        host.clock.0 += 40;
        host.frame().expect("frame");
        host.receive(CLIENT);
    }
    let frames: Vec<i32> = saves.lock().unwrap().iter().map(|s| s.1).collect();
    assert_eq!(frames, [8192, 2 * 8192, 3 * 8192]);
}

#[test]
fn a_refused_save_is_a_session_fault_and_the_leave_goes_on() {
    struct Refuse;
    impl CharacterStore<WorldSim<Seams>, ActionWorld> for Refuse {
        fn save(&mut self, _: &mut Sim, _: ClientId) -> Result<(), String> {
            Err("disk full".into())
        }
    }
    let mut host = in_game_host();
    host.game.set_storage(Box::new(Refuse));
    host.send_system(CLIENT, &[0x69]).expect("queued");
    host.frame().expect("frame");
    assert_eq!(host.receive(CLIENT), [vec![0xB0], vec![0x05], vec![0x06]]);
    assert_eq!(
        faults(&host),
        [(CLIENT, SessionFault::SaveFailed("disk full".into()))]
    );
}

#[test]
fn the_tick_saves_every_8192_frames() {
    let mut host = in_game_host();
    let saves = with_recorder(&mut host);
    // Two frames short of the period: the save is on frame 8192 only.
    host.game.game.frame = 8190;
    for _ in 0..3 {
        host.clock.0 += 40;
        host.frame().expect("frame");
    }
    assert_eq!(host.game.game.frame, 8193);
    assert_eq!(*saves.lock().unwrap(), [(CLIENT, 8192, true)]);
    assert!(!host.game.game.character_save_due);
    // Without a storage the save is recorded as not written (M08).
    let mut host = in_game_host();
    host.game.game.frame = 8191;
    host.clock.0 += 40;
    host.frame().expect("frame");
    assert_eq!(
        host.game.save_faults,
        [(CLIENT, d2_server::adapters::storage::SaveFault::NoStorage)]
    );
}

#[test]
fn the_tick_save_runs_before_the_client_loop() {
    let mut host = in_game_host();
    let saves = with_recorder(&mut host);
    host.game.game.frame = 8191;
    // A client back in the join state: step 5's per-client loop puts it
    // in game again, after the save (the save runs first).
    let id = host.game.sim_client(CLIENT).expect("joined");
    host.game.game.lists.client_mut(id).expect("client").state = client_state::JOINING;
    host.clock.0 += 40;
    host.frame().expect("frame");
    assert_eq!(*saves.lock().unwrap(), [(CLIENT, 8192, false)]);
    let state = host.game.game.lists.client(id).map(|e| e.state);
    assert_eq!(state, Some(client_state::IN_GAME));
}

#[test]
fn leave_needs_state_4() {
    let loads = std::rc::Rc::new(std::cell::Cell::new(0));
    let (mut host, _t) = session_host(loads);
    host.send_system(
        CLIENT,
        &create_request(CLASS as u8, EXPANSION_FLAGS).encode(),
    )
    .expect("queued");
    // State 1 (created, not joined): 0x69 does nothing.
    host.send_system(CLIENT, &[0x69]).expect("queued");
    host.frame().expect("frame 1");
    assert!(host.game.sim_client(CLIENT).is_some());
    assert_eq!(faults(&host), []);
}

#[test]
fn loader_item_skill_messages_follow_the_add_messages() {
    use d2_sim::units::messages as m;
    // The loader's item calls queued three messages (0x22 twice, 0x21):
    // they go after 0x76 and before the 0x0B and the 0x23;
    // the other item messages stay where they were queued.
    let mut g = 0;
    let j = join_with(|s, player| {
        g = s.game.lists.unit(player).unwrap().guid;
        let skills = s.events.action.hooks().tables.skills.skills.len();
        let a = &mut s.events.action;
        a.sys.hooks.session.join_items.insert(
            player,
            vec![
                m::update_item_skill(0, g, 219, 2, false).to_vec(),
                m::update_item_skill(0, g, 217, 1, false).to_vec(),
                m::update_oskill(0, false, g, 36, 0, 1).to_vec(),
            ],
        );
        assert_eq!(enter_game(s, CLIENT, &entry(skills)), Ok(player));
    });
    let ids: Vec<u8> = j.received.iter().map(|b| b[0]).collect();
    let at = |pred: &dyn Fn(&Vec<u8>) -> bool| {
        j.received
            .iter()
            .position(pred)
            .unwrap_or_else(|| panic!("message ids {ids:02x?}"))
    };
    let id = |b: &Vec<u8>, i: u8| b[0] == i;
    let (p76, p0b) = (at(&|b| id(b, 0x76)), at(&|b| id(b, 0x0B)));
    let p22: Vec<usize> = (0..j.received.len())
        .filter(|&i| j.received[i][0] == 0x22)
        .collect();
    let p21 = at(&|b| id(b, 0x21));
    let p23 = at(&|b| id(b, 0x23));
    assert_eq!(p22.len(), 2);
    assert!(p76 < p22[0] && p22[0] < p22[1] && p22[1] < p21 && p21 < p0b);
    assert!(p21 < p23);
    assert_eq!(j.received[p21][7..11], [36, 0, 0, 1]);
}

#[test]
fn game_list_and_the_no_effect_ids() {
    let mut host = in_game_host();
    host.send_system(CLIENT, &[0x6A]).expect("queued");
    host.send_system(CLIENT, &[0x6E]).expect("queued");
    host.send_system(CLIENT, &[0x70]).expect("queued");
    host.frame().expect("frame");
    let mut game = [0u8; 53];
    game[0] = 0xB2;
    game[1..5].copy_from_slice(b"game");
    game[0x31] = 1;
    // The first game of the process has id 1.
    game[0x33] = 1;
    let mut end = [0u8; 53];
    end[0] = 0xB2;
    end[0x33..0x35].copy_from_slice(&[0xFF, 0xFF]);
    let got = host.receive(CLIENT);
    assert_eq!(got[..2], [game.to_vec(), end.to_vec()]);
    let flow = host.game.session().unwrap();
    assert!(flow.heartbeat_flag.contains(&CLIENT));
    assert_eq!(flow.faults, []);
}

#[test]
fn save_upload_collects_chunks_then_checksums() {
    let mut host = in_game_host();
    let save: Vec<u8> = (0..300u32).map(|i| (i * 7) as u8).collect();
    let chunk = |part: &[u8]| {
        let mut m = vec![0x6C, part.len() as u8];
        m.extend((save.len() as u32).to_le_bytes());
        m.extend(part);
        // The size rule is len + 7 (`client-messages.tsv`): one byte
        // after the data.
        m.push(0);
        m
    };
    host.send_system(CLIENT, &chunk(&save[..200]))
        .expect("queued");
    host.frame().expect("frame");
    let u = &host.game.session().unwrap().uploads[&CLIENT];
    assert_eq!((u.count, u.complete), (200, false));
    host.send_system(CLIENT, &chunk(&save[200..]))
        .expect("queued");
    host.clock.0 += 40;
    host.frame().expect("frame");
    let u = &host.game.session().unwrap().uploads[&CLIENT];
    assert_eq!((u.count, u.complete), (300, true));
    assert_eq!(u.buffer, save);
    assert_eq!(u.checksum, d2_formats::d2s::checksum(&save));
    // M08: one more chunk overflows (fatal 0xB2F); a total ≥ 0x2000 is the
    // fatal assert.
    host.send_system(CLIENT, &chunk(&save[..1]))
        .expect("queued");
    let mut big = vec![0x6C, 1];
    big.extend(0x2000u32.to_le_bytes());
    big.extend([0, 0]);
    host.send_system(CLIENT, &big).expect("queued");
    host.clock.0 += 40;
    host.frame().expect("frame");
    assert_eq!(
        host.game.session().unwrap().faults,
        [
            (
                CLIENT,
                SessionFault::UploadOverflow {
                    count: 300,
                    len: 1,
                    total: 300
                }
            ),
            (CLIENT, SessionFault::UploadTotal(0x2000)),
        ]
    );
}

/// The messages of game creation and the join up to game entry's 0x7E,
/// with `j`'s facts.
fn join_messages(j: &Joined) -> Vec<Vec<u8>> {
    let (rx, ry, _, _) = j.room_rect;
    let (x, y) = j.pos;
    let g = j.guid.to_le_bytes();
    let mut assign = vec![0x59, g[0], g[1], g[2], g[3], CLASS as u8];
    assign.extend(name());
    assign.extend([0, 0, 0, 0]);
    // The player has no state: the stream is the end marker alone.
    let states = vec![0xAA, 0, g[0], g[1], g[2], g[3], 8, 0xFF];
    let proximity = vec![0x76, 0, g[0], g[1], g[2], g[3]];
    let handshake = vec![0x0B, 0, g[0], g[1], g[2], g[3]];
    let portal = vec![0x5F, 5, 0, 0, 0];
    let hotkey = vec![0x7B, 3, 0, 0x80, 0xFF, 0xFF, 0xFF, 0xFF];
    let hand = |h: u8, skill: u8| {
        vec![
            0x23, 0, g[0], g[1], g[2], g[3], h, skill, 0, 0xFF, 0xFF, 0xFF, 0xFF,
        ]
    };
    let mut load = vec![0x03, 0];
    load.extend(INIT.to_le_bytes());
    load.extend((TOWN as u16).to_le_bytes());
    load.extend(j.obj_seed.to_le_bytes());
    let reveal = {
        let mut b = vec![0x07];
        b.extend((rx as u16).to_le_bytes());
        b.extend((ry as u16).to_le_bytes());
        b.push(TOWN as u8);
        b
    };
    let (wg, wc, (wx, wy), wm, wi) = j.waypoint;
    let object = {
        let mut b = vec![0x51, 2];
        b.extend(wg.to_le_bytes());
        b.extend(wc.to_le_bytes());
        b.extend((wx as u16).to_le_bytes());
        b.extend((wy as u16).to_le_bytes());
        b.extend([wm, wi]);
        b
    };
    let mut place = vec![0x15, 0, g[0], g[1], g[2], g[3]];
    place.extend((x as u16).to_le_bytes());
    place.extend((y as u16).to_le_bytes());
    place.push(1);
    // Game creation, the join (the player's own add
    // messages, 0x0B, 0x5F, slot 3's hot key, the two hands; no 0x95: the
    // player has no life), then game entry: 0x07 for the spawn room, the
    // room switch (the town has one room: its 0x07 again, then the add
    // messages of its units, the waypoint), 0x15, 0x7E; all before the
    // first tick.
    vec![
        vec![0x01, 0x00, 0x04, 0x00, 0x10, 0x00, 0x01, 0x00],
        vec![0x00],
        vec![0x02],
        assign,
        states,
        proximity,
        handshake,
        portal,
        hotkey,
        hand(1, 0),
        hand(0, 36),
        load,
        DARKNESS.to_vec(),
        reveal.clone(),
        reveal,
        object,
        place,
        vec![0x7E, 0, 0, 0, 0],
    ]
}

/// S→C 0x53 of a new act's environment record
/// (`53 02000000 00000000 00`).
const DARKNESS: [u8; 10] = [0x53, 2, 0, 0, 0, 0, 0, 0, 0, 0];

/// A save whose name is the fixture's player name, act 0 of Normal.
fn save_named() -> d2_formats::d2s::D2s {
    let mut h = d2_formats::d2s::Header::default();
    let n = name();
    let len = n.iter().position(|&c| c == 0).unwrap_or(15);
    h.set_name(&n[..len]).unwrap();
    h.class = CLASS as u8;
    h.towns = [0x80, 0, 0];
    d2_formats::d2s::D2s {
        header: h,
        body: Some(d2_formats::d2s::Body::default()),
    }
}

#[test]
fn a_full_save_loads_before_the_join_sequence() {
    use d2_formats::d2s::{StatEntry, Stats};
    use d2_server::adapters::character::LoadContext;
    use d2_server::adapters::session::enter_game_from_save;
    let mut save = save_named();
    let st = |id, value| StatEntry {
        id,
        layer: 0,
        value,
    };
    // Level 2, gold over the carry limit (2 × 10,000), life and mana.
    save.body.as_mut().unwrap().stats = Stats::Bits(vec![
        st(6, 50 << 8),
        st(7, 60 << 8),
        st(8, 9 << 8),
        st(10, 1),
        st(11, 70 << 8),
        st(12, 2),
        st(14, 20_001),
    ]);
    let mut report = None;
    let mut life = None;
    let mut native = None;
    let vitals = data().vitals().unwrap();
    let j = join_with(|s, player| {
        if s.events.action.hooks().vitals.is_none() {
            s.events.action.hooks().vitals = Some(std::sync::Arc::new(vitals.clone()));
        }
        let (p, r) = enter_game_from_save(s, CLIENT, &save, &LoadContext::default())
            .expect("loaded and placed");
        native = s
            .events
            .action
            .hooks()
            .skill_lists
            .get(&player)
            .map(|l| l.base_levels());
        assert_eq!(p, player);
        let v = &s.events.action.sys.stats;
        assert_eq!(v.unit_base(player, 14, 0), 0, "gold over the limit → 0");
        assert_eq!(v.unit_base(player, 12, 0), 2);
        // Stamina := maxstamina. (Stats 30 and 67–69 are past
        // the synthetic itemstatcost's 16 rows: not observable here.)
        assert_eq!(v.unit_base(player, 10, 0), v.unit_total(player, 11, 0));
        assert_eq!(v.unit_base(player, 10, 0), 70 << 8);
        life = Some((v.unit_base(player, 6, 0), v.unit_base(player, 8, 0)));
        report = Some(r);
    });
    assert_eq!(life, Some((50 << 8, 9 << 8)));
    let r = report.unwrap();
    assert!(!r.new_character);
    assert_eq!(r.act, 0);
    // The steps without a provider are named, in order. The waypoints step
    // has one now (`ActionCharacter::set_waypoints`), so it is applied and no longer
    // listed.
    let steps: Vec<_> = r.unapplied.iter().map(|u| u.step).collect();
    assert_eq!(
        steps,
        [
            "header",
            "quests",
            "npc fields",
            "item indices",
            "quest entry"
        ]
    );
    // The session sequence follows the load unchanged: game creation,
    // then the join (no player record given: no 0x5F, no 0x23).
    // The skills section's S→C 0x94 follows the add messages, with the
    // list the player init made (skill 0, then the fixture class's only
    // `Skill 1`…`Skill 10` id, Attack again, so the player init raises it to base
    // 2): `94 01 <guid> 0000 02`.
    assert_eq!(native, Some(vec![(0, 2)]));
    let ids: Vec<u8> = j.received.iter().take(9).map(|m| m[0]).collect();
    assert_eq!(ids, [0x01, 0x00, 0x02, 0x59, 0xAA, 0x76, 0x94, 0x0B, 0x95]);
    let g = j.guid.to_le_bytes();
    assert_eq!(j.received[6], [0x94, 1, g[0], g[1], g[2], g[3], 0, 0, 2]);
    assert_eq!(j.received[9][0], 0x03);
}

#[test]
fn a_stub_starts_a_new_character_before_the_join_sequence() {
    use d2_server::adapters::character::LoadContext;
    use d2_server::adapters::session::{enter_game_from_save, initial_portal_flags};
    let n = name();
    let len = n.iter().position(|&c| c == 0).unwrap_or(15);
    let stub = d2_formats::d2s::D2s::new_stub(&n[..len], CLASS as u8, 0, 1).unwrap();
    let vitals = data().vitals().unwrap();
    let cs = vitals
        .charstats(CLASS as i32)
        .expect("charstats row")
        .clone();
    let mut report = None;
    let mut portals = Vec::new();
    let mut list = None;
    let j = join_with(|s, player| {
        if s.events.action.hooks().vitals.is_none() {
            s.events.action.hooks().vitals = Some(std::sync::Arc::new(vitals.clone()));
        }
        portals = s.events.action.hooks().drlg.data.portal_levels();
        let (_, r) = enter_game_from_save(s, CLIENT, &stub, &LoadContext::default())
            .expect("started and placed");
        list = s.events.action.hooks().skill_lists.get(&player).cloned();
        let v = &s.events.action.sys.stats;
        // The creation stats.
        assert_eq!(v.unit_base(player, 0, 0), i32::from(cs.str));
        report = Some(r);
    });
    let r = report.unwrap();
    assert!(r.new_character);
    // The load's own 0x23 (hand 0, `StartSkill`, item −1) after the add
    // messages, then 0x0B, 0x5F with the portal flags and the two hands with item 0.
    assert_ne!(cs.startskill, 0, "the fixture class has a start skill");
    assert_eq!(r.right_skill, Some(cs.startskill));
    let g = j.guid.to_le_bytes();
    let k = cs.startskill.to_le_bytes();
    let hand = |h: u8, skill: [u8; 2], item: [u8; 4]| {
        let mut m = vec![0x23, 0, g[0], g[1], g[2], g[3], h, skill[0], skill[1]];
        m.extend(item);
        m
    };
    let flags = initial_portal_flags(&portals).to_le_bytes();
    let ids: Vec<u8> = j.received.iter().take(9).map(|m| m[0]).collect();
    assert_eq!(ids, [0x01, 0x00, 0x02, 0x59, 0xAA, 0x76, 0x23, 0x0B, 0x5F]);
    assert_eq!(j.received[6], hand(0, k, [0xFF; 4]));
    assert_eq!(
        j.received[8],
        [0x5F, flags[0], flags[1], flags[2], flags[3]]
    );
    // Rules 3.4 and 3.8: the stat messages of the mod array (the start
    // stats' `Saved` base values), before and after
    // the two hands, the same both times; strength among them.
    let stats: Vec<&Vec<u8>> = j.received[9..]
        .iter()
        .take_while(|m| (0x1D..=0x1F).contains(&m[0]))
        .collect();
    let n = stats.len();
    assert!(stats.contains(&&vec![0x1D, 0, cs.str]), "{stats:02X?}");
    // Item 0 (REC-02, `facts/join/a1-new-ama.tsv`).
    assert_eq!(j.received[9 + n], hand(1, [0, 0], [0; 4]));
    assert_eq!(j.received[10 + n], hand(0, k, [0; 4]));
    let again: Vec<&Vec<u8>> = j.received[11 + n..11 + 2 * n].iter().collect();
    assert_eq!(again, stats);
    // The server player init's native skills:
    // skill 0, then the fixture's only `Skill 1`…`Skill 10` id, Attack
    // again (base 2), owner −1, in both hands. The stub load reads no
    // skills section, so the join has no S→C 0x94.
    let list = list.expect("the player has a server skill list");
    assert_eq!(list.base_levels(), [(0, 2)]);
    assert_eq!((list.left, list.right), (Some(0), Some(0)));
    assert!(j.received.iter().all(|m| m[0] != 0x94));
    // `StartSkill` is not a native skill (it comes with the start items'
    // stat 107, which has no provider): "has skill" stays unapplied.
    let steps: Vec<_> = r.unapplied.iter().map(|u| u.step).collect();
    assert_eq!(
        steps,
        [
            "new character set-up",
            "start items",
            "has skill",
            "mouse skills",
            "quest entry"
        ]
    );
}

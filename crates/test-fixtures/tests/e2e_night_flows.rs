//! The night's world features end to end through the real host
//! (`d2_server::host::Host` over `SimGame` on `WorldSim`, the `WiredWorld`
//! host) on the synthetic install (`test_fixtures::install`, no game
//! files): the game-creation sequence (`WorldSim::create_game`: regions →
//! object control → NPC control → quest control), the session join
//! (`enter_game`: 0x59, 0x0B, 0x03, 0x07, 0x15), then intents in and S→C
//! bytes and sim state out for:
//!
//! - a chest (operate 4) and a shrine (operate 2) through C→S 0x13;
//! - the Act I quest objects: a Cairn stone's init (6) and Cain's gibbet
//!   (operate 10, then its object event 7);
//! - an Act II quest step: Horazon's journal (operate 42).
//!
//! The synthetic install's `objects.txt` has three rows (waypoint,
//! chest, door) and one `shrines.txt` row; the object rows this run
//! needs beyond those (the shrine, Cairn stone 17, gibbet 26, journal
//! 357) are added to the object tables in [`object_tables`], by
//! `objects.txt` row index and the columns the game reads.
//!
//! The seams D2RUST does not implement yet are answered by [`Night`] (the
//! action wiring's `Pending`: the interact range answers
//! "in range", the routes handed back are logged, sends are collected)
//! and by the shared e2e rest (`e2e_support::Rest`: NPC, vendor and
//! quest seams, staged answers and call logs). Every place a flow
//! reaches such a seam is named in the test.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use d2_data::tables::{Levels, Monstats, Objects, Shrines};
use d2_server::adapters::handlers::world::{ActionWorld, Outbox, WiredWorld};
use d2_server::adapters::session::{enter_game, Entry};
use d2_server::adapters::{PlayerData, PlayerFields, ProtoSizes, SimGame, UnitFacts};
use d2_server::dispatch::Outcome;
use d2_server::host::{Handled, Host};
use d2_server::seams::{ClientId, Clock, MessageSink, PlayerGate, Pos, ResultCode, SessionHandler};
use d2_sim::game::Game;
use d2_sim::monsters::init::GameInfo;
use d2_sim::rng::Seed;
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::messages as msg;
use d2_sim::units::{RoomId, UnitId, UnitType};
use d2_sim::wiring::action::objects::ObjectRoute;
use d2_sim::wiring::action::{ActionHooks, Pending};
use d2_sim::wiring::worldgen::{CreationTables, WorldPending, WorldSim, WorldState};
use d2_sim::world::npc::HireRow;
use d2_sim::world::objects::ObjectTables;
use d2_sim::world::quests::{PlayerQuests, QuestError, QuestTables};
use d2_sim::world::waypoints::WaypointData;
use test_fixtures::game::{ActCreation, GameData};
use test_fixtures::{install, synth};

#[path = "../../d2-client/tests/e2e_support/mod.rs"]
mod e2e_support;
use e2e_support::{blank, item_tables, vendor_tables, Rest};

// ---- constants ------------------------------------------------------------------------

/// The synthetic town (levels row 1, lvlprest Def 0) and the run's seeds
/// (fixture choices, as `synthetic_game.rs`).
const TOWN: u32 = 1;
const INIT: u32 = 0x1234_5678;
const GAME_SEED: u32 = 1234;
const CLASS: u32 = 3;
const CLIENT: ClientId = 0;
/// Sub-tiles per tile.
const SUB: i32 = 5;

/// `objects.txt` rows: the synthetic install's chest (row 1, operate
/// 4), and the rows this run adds: a shrine (init 1, operate 2,
/// `SubClass` bit 0), Cairn stone Alpha (row 17: init 6, operate 9), the
/// gibbet (row 26: operate 10) and Horazon's journal (row 357: operate
/// 42).
const CHEST: u32 = 1;
const SHRINE: u32 = 3;
const STONE: u32 = 17;
const GIBBET: u32 = 26;
const JOURNAL: u32 = 357;
/// `FrameCnt1` of the added rows: 15 frames (fixed up × 256).
const FRAMES1: u32 = 15 << 8;
/// The shrine record the shrine's init picks
/// (`Parm0` 0, two `shrines` rows → `roll(1) + 1` = 1) and its reset
/// time in minutes.
const SHRINE_ID: usize = 1;
const SHRINE_CODE: u8 = 2;
const SHRINE_RESET_MIN: u8 = 1;
/// Quest chains and slots.
const A1Q4: u8 = 4;
const A2Q4: u8 = 11;
const A2Q4_SLOT: u8 = 12;

fn data() -> &'static GameData {
    static D: OnceLock<GameData> = OnceLock::new();
    D.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("e2e-night-flows-{}", std::process::id()));
        let i = install::build(&dir, &synth::synthetic()).unwrap_or_else(|e| panic!("{e}"));
        GameData::from_install(&i).unwrap_or_else(|e| panic!("{e}"))
    })
}

/// The object tables of the run: the install's `objects`, `shrines` and
/// `levels` with the rows named above added (blank rows up to 357).
fn object_tables() -> ObjectTables {
    let d = data();
    let mut objects: Vec<Objects> = d.rows().unwrap();
    assert_eq!(objects[CHEST as usize].operatefn, 4, "the install's chest");
    objects.resize(JOURNAL as usize + 1, blank());
    let mut row = |i: u32, initfn: u8, operatefn: u8| {
        let o = &mut objects[i as usize];
        o.initfn = initfn;
        o.operatefn = operatefn;
        o.selectable0 = 1;
        o.framecnt1 = FRAMES1;
        o.sizex = 1;
        o.sizey = 1;
    };
    row(SHRINE, 1, 2);
    row(STONE, 6, 9);
    row(GIBBET, 0, 10);
    row(JOURNAL, 0, 42);
    objects[SHRINE as usize].subclass = 1;
    let mut shrines: Vec<Shrines> = d.rows().unwrap();
    shrines.truncate(1);
    let mut s: Shrines = blank();
    s.code = SHRINE_CODE;
    s.reset_time_in_minutes = SHRINE_RESET_MIN;
    shrines.push(s);
    ObjectTables {
        objects,
        shrines,
        levels: d.rows::<Levels>().unwrap(),
        objgroup: Vec::new(),
        leveldefs: Vec::new(),
    }
}

// ---- the action wiring's seams ----------------------------------------------------------

/// The action wiring's `Pending` (not implemented yet): the
/// interact range
/// answered "in range" (the path finder does not
/// write it), the routes the object module hands back logged
/// (`Pending::object_route`), and every send collected for the host.
#[derive(Default)]
struct Night {
    sent: Vec<(UnitId, Vec<u8>)>,
    routes: Vec<ObjectRoute>,
}

impl Pending for Night {
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.sent.push((player, msg.to_vec()));
    }
    fn object_in_range(&self, _: &Game, _: UnitId, _: UnitId) -> bool {
        true
    }
    fn object_route(&mut self, _: &mut Game, route: ObjectRoute) {
        self.routes.push(route);
    }
}

impl WorldPending for Night {}

impl Outbox for Night {
    fn take_sent(&mut self) -> Vec<(UnitId, Vec<u8>)> {
        std::mem::take(&mut self.sent)
    }
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

type World = WiredWorld<Rest>;
type Sim = SimGame<WorldSim<Night>, World>;
type TestHost = Host<Sim, ProtoSizes, NoSession, Ms>;

// ---- the game ------------------------------------------------------------------------

/// The character name of the join (zero-padded, 0x59 bytes 6..22).
fn name() -> [u8; 16] {
    let mut n = [0u8; 16];
    n[..5].copy_from_slice(b"night");
    n
}

/// What game creation and the join leave behind, for the assertions.
struct Fx {
    host: TestHost,
    player: UnitId,
    /// The town room and its sub-tile rectangle (x0, y0, x1, y1).
    room: RoomId,
    rect: (i32, i32, i32, i32),
    /// The game seed before creation and after it; the object
    /// control's `dwObjSeed`.
    seed_before: Seed,
    seed_created: Seed,
    obj_seed: u32,
    /// The client's S→C transcript of the join frames.
    joined: Vec<Vec<u8>>,
}

impl Fx {
    /// Game creation on the synthetic town, the town
    /// generated and streamed, an unplaced player joined in state 4 and
    /// entered (`enter_game`), the host's first frame (no tick) and one
    /// tick.
    fn new() -> Self {
        let d = data();
        let (drlg, types) = d.level_types();
        let world = d
            .drlg_world(drlg, &types, ActCreation::TownOnly, INIT, TOWN)
            .unwrap();
        let mut hooks = ActionHooks::new(
            Arc::new(d.action_tables().unwrap()),
            world,
            Seed::init_low(GAME_SEED),
            Night::default(),
        );
        hooks.anim_data = Some(Arc::new(d.anim.clone()));
        hooks.vitals = Some(Arc::new(d.vitals().unwrap()));
        hooks.enable_paths().unwrap();
        let info = GameInfo {
            expansion: true,
            ..GameInfo::default()
        };
        let state = WorldState::new(types, Arc::new(d.world_tables().unwrap()), info);
        let mut sim = WorldSim::new(
            Arc::new(d.stat_data().unwrap()),
            d.unit_data().unwrap(),
            hooks,
            state,
        );
        // 1. Game creation: the four seeded controls in their
        // order, before any unit.
        let seed_before = sim.action.hooks().game_seed;
        let monstats: Vec<Monstats> = d.rows().unwrap();
        let quest_tables = QuestTables::load().unwrap();
        let objects = object_tables();
        let created = sim
            .create_game(CreationTables {
                objects: Arc::new(objects.clone()),
                monstats: &monstats,
                hirelings: HireRow::from_table(d.table("hireling").unwrap()).unwrap(),
                quests: &quest_tables,
            })
            .unwrap();
        let seed_created = sim.action.hooks().game_seed;
        let obj_seed = sim.action.hooks().objects.as_ref().unwrap().obj_seed;

        // 2. The town and its one room streamed.
        let mut game = Game::new();
        game.lists.ensure_act(0).unwrap();
        let (room, r) = sim
            .action
            .hooks()
            .drlg
            .with_act(0, &mut game.lists, |dr, svc| {
                let lv = dr.get_or_alloc_level(svc.data, svc.types, TOWN)?;
                let r = dr.level_rooms(lv)[0];
                Ok::<_, d2_sim::drlg::DrlgError>((dr.stream_room(svc, r)?, dr.room(r).rect))
            })
            .expect("act 0 has a DRLG")
            .unwrap();
        let room = room.expect("the town room is active");
        let rect = (r.x * SUB, r.y * SUB, (r.x + r.w) * SUB, (r.y + r.h) * SUB);

        // 3. The player as the save loader leaves it (no room, (0, 0)).
        let req = AllocRequest {
            ty: UnitType::Player,
            class: CLASS,
            room: None,
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: true,
        };
        let player = sim
            .action
            .with(&mut game, |g, v| v.allocate(g, &req, 0, 0))
            .unwrap();
        sim.action.sys.units.get_mut(player).unwrap().mode = 1;

        // The wired host: the waypoints (`ActionWorld`), the NPC /
        // vendor / quest systems on the created controls.
        let mut rest = Rest::default();
        rest.quests.insert(player, PlayerQuests::default());
        let action = ActionWorld {
            waypoints: Some(WaypointData::new(&objects.levels, &objects.objects)),
            ..ActionWorld::default()
        };
        let world = WiredWorld::new(
            action,
            item_tables(),
            created.quests,
            created.npc,
            vendor_tables(),
            rest,
            1000,
        );
        let mut s: Sim = SimGame::with_world(game, sim, world);
        s.join(CLIENT, Some(player), None, client_state::IN_GAME)
            .unwrap();
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
        let entry = Entry::new(0, name());
        assert_eq!(enter_game(&mut s, CLIENT, &entry), Ok(player));
        let mut host: TestHost = Host::new(s, ProtoSizes, NoSession, Ms(1000));
        host.connect(CLIENT);
        let r = host.frame().unwrap();
        assert!(!r.ticked, "the first frame starts the tick clock");
        let mut joined = host.receive(CLIENT);
        host.clock.0 += 40;
        assert!(host.frame().unwrap().ticked);
        joined.extend(host.receive(CLIENT));
        let mut fx = Self {
            host,
            player,
            room,
            rect,
            seed_before,
            seed_created,
            obj_seed,
            joined,
        };
        fx.assert_clean();
        fx
    }

    fn sim(&mut self) -> &mut Sim {
        &mut self.host.game
    }
    fn world(&mut self) -> &mut World {
        &mut self.host.game.world
    }
    fn guid(&self, u: UnitId) -> u32 {
        self.host.game.game.lists.unit(u).unwrap().guid
    }
    fn mode(&mut self, u: UnitId) -> u32 {
        self.sim().events.action.sys.units.get(u).unwrap().mode
    }
    fn pos(&mut self, u: UnitId) -> (i32, i32) {
        self.sim().events.action.hooks().path_position(u)
    }
    fn frame(&self) -> i32 {
        self.host.game.game.frame
    }
    fn control_seed(&mut self) -> Seed {
        self.sim()
            .events
            .action
            .hooks()
            .objects
            .as_ref()
            .unwrap()
            .control
            .seed
    }
    fn routes(&mut self) -> Vec<ObjectRoute> {
        std::mem::take(&mut self.sim().events.action.hooks().x.routes)
    }
    fn timers(&self, u: UnitId) -> Vec<(u8, i32)> {
        let t = &self.host.game.game.timers;
        let mut v: Vec<_> = t
            .unit_timers(u)
            .into_iter()
            .filter_map(|i| Some((t.event(i)?.0, t.expire(i)?)))
            .collect();
        v.sort();
        v
    }
    fn quests(&mut self) -> &mut PlayerQuests {
        let p = self.player;
        self.world().rest.quests.get_mut(&p).unwrap()
    }

    /// No handler fault, no tick fault, no wiring error, nothing the
    /// dispatcher left unhandled.
    fn assert_clean(&mut self) {
        let s = &mut self.host.game;
        assert!(s.tick_faults.is_empty(), "{:?}", s.tick_faults);
        assert!(
            s.world.action.faults.is_empty(),
            "{:?}",
            s.world.action.faults
        );
        assert!(s.unhandled.is_empty(), "{:?}", s.unhandled);
        assert_eq!(s.events.errors(), Vec::<String>::new());
    }

    /// An object of `class` allocated in the town room at the sub-tile
    /// (x, y) (the allocation runs its init dispatch),
    /// its position staged for the dispatcher's range check
    /// from its path.
    fn object(&mut self, class: u32, at: (i32, i32)) -> UnitId {
        let req = AllocRequest {
            ty: UnitType::Object,
            class,
            room: Some(self.room),
            add: true,
            fixed_guid: None,
            mode: 0,
            allied: false,
        };
        let s = &mut self.host.game;
        let u = s
            .events
            .action
            .with(&mut s.game, |g, v| v.allocate(g, &req, at.0, at.1))
            .expect("allocated");
        // Staged as already announced to the client (the room clean-up
        // cleared unit flag 0x10).
        s.events.action.sys.units.get_mut(u).unwrap().flags &=
            !d2_sim::units::record::flags::SEED_SET;
        // The 0x13 object case walks the player when the object is out of
        // its interact range; these flows test
        // the operate, so the player stands beside the object.
        let (player, room) = (self.player, self.room);
        s.events.action.with(&mut s.game, |g, v| {
            d2_sim::wiring::path::PathCtx::of(v, g).teleport(player, Some(room), at.0 - 2, at.1)
        });
        self.stage(u);
        u
    }

    fn stage(&mut self, u: UnitId) {
        let (x, y) = self.pos(u);
        self.sim().set_unit(
            u,
            UnitFacts {
                act: 0,
                pos: Pos { x, y },
                owner: None,
            },
        );
    }

    /// A free sub-tile of the town room `k` steps of 4 sub-tiles from the
    /// room's corner along the diagonal, clear of the player.
    fn spot(&mut self, k: i32) -> (i32, i32) {
        let (x0, y0, x1, y1) = self.rect;
        let p = self.pos(self.player);
        let mut at = (x0 + 4 + 4 * k, y0 + 4);
        if (at.0 - p.0).abs() < 3 && (at.1 - p.1).abs() < 3 {
            at.1 += 6;
        }
        assert!(at.0 < x1 && at.1 < y1, "{at:?} outside the room");
        at
    }

    /// One host frame of 40 ms with `msgs` from the client: the codes the
    /// dispatcher returned, the S→C the client received.
    fn step(&mut self, msgs: &[Vec<u8>]) -> (Vec<ResultCode>, Vec<Vec<u8>>) {
        self.stage(self.player);
        for m in msgs {
            let sent = self.host.send_game(CLIENT, m).unwrap();
            assert!(sent.is_some(), "dropped by the duplicate filter");
        }
        self.host.clock.0 += 40;
        let r = self.host.frame().unwrap();
        assert!(r.ticked);
        let codes = r
            .messages
            .iter()
            .map(|m| match m.handled {
                Handled::Game(Outcome::Dispatched(c)) => c,
                ref h => panic!("not dispatched: {h:?}"),
            })
            .collect();
        (codes, self.host.receive(CLIENT))
    }

    fn frames(&mut self, n: usize) -> Vec<Vec<u8>> {
        let mut got = Vec::new();
        for _ in 0..n {
            got.extend(self.step(&[]).1);
        }
        got
    }
}

/// C→S 0x13 InteractWithEntity for an object (unit
/// type u32@1, GUID u32@5).
fn interact(guid: u32) -> Vec<u8> {
    let mut m = vec![0x13, 2, 0, 0, 0];
    m.extend(guid.to_le_bytes());
    m
}

/// S→C 0x0E ObjectState (12 bytes): type 2 @1,
/// GUID @2, 3 @6, selectable @7, mode u32 @8.
fn object_state(guid: u32, selectable: bool, mode: u32) -> Vec<u8> {
    let mut m = vec![0x0E, 2];
    m.extend(guid.to_le_bytes());
    m.extend([3, u8::from(selectable)]);
    m.extend(mode.to_le_bytes());
    m
}

/// S→C 0x28 QuestInfo (103
/// bytes): 0x28, unit type, GUID u32, 0, the 96-byte current record.
fn quest_info(rec: &PlayerQuests) -> Vec<u8> {
    let mut m = vec![0x28, 6, 0, 0, 0, 0, 0];
    m.extend(rec.flags[0].0);
    m
}

// ---- 1. game creation and the join -----------------------------------------------------

#[test]
fn game_creation_then_the_real_join() {
    let mut fx = Fx::new();
    // Creation derives, each from one game-seed step, the
    // regions, the object control (`dwObjSeed` = that step's lo'), the NPC control and the quest
    // control;
    // nothing else steps it before the first unit.
    let mut want = fx.seed_before;
    want.step();
    let obj_seed = want.step();
    want.step();
    want.step();
    assert_eq!(fx.seed_created, want, "four game-seed steps");
    assert_eq!(fx.obj_seed, obj_seed);
    // The controls exist on the host: every quest chain of the quest
    // table, the NPC record of the install's one `interact` monster.
    assert!(fx.world().quests.record(A1Q4).is_some());
    assert!(fx.world().quests.record(A2Q4).is_some());
    assert!(
        fx.world().npc.record(2).is_some(),
        "keeper (monstats row 2)"
    );

    // The join:
    // 0x59 with the player's own part B (0xAA without states, 0x76), 0x0B,
    // 0x03 (act 0, the act DRLG's init seed, the town level, the object
    // seed), 0x53, game entry: 0x07 of the spawn room, the room switch's
    // 0x07 per room of its adjacency array (the town has one room, no
    // unit in it), 0x15 at the spawn search's point (flag 1), 0x7E; then
    // the first tick: the room is ready, 0x04, and
    // the join sequence.
    let p = fx.player;
    let g = fx.guid(p).to_le_bytes();
    let (x, y) = fx.pos(p);
    let (x0, y0, x1, y1) = fx.rect;
    assert!(
        x >= x0 && x < x1 && y >= y0 && y < y1,
        "spawned in the town"
    );
    let mut assign = vec![0x59, g[0], g[1], g[2], g[3], CLASS as u8];
    assign.extend(name());
    assign.extend([0, 0, 0, 0]);
    let handshake = vec![0x0B, 0, g[0], g[1], g[2], g[3]];
    let mut load = vec![0x03, 0];
    load.extend(INIT.to_le_bytes());
    load.extend((TOWN as u16).to_le_bytes());
    load.extend(obj_seed.to_le_bytes());
    let mut reveal = vec![0x07];
    reveal.extend(((x0 / SUB) as u16).to_le_bytes());
    reveal.extend(((y0 / SUB) as u16).to_le_bytes());
    reveal.push(TOWN as u8);
    let mut place = vec![0x15, 0, g[0], g[1], g[2], g[3]];
    place.extend((x as u16).to_le_bytes());
    place.extend((y as u16).to_le_bytes());
    place.push(1);
    let states = vec![0xAA, 0, g[0], g[1], g[2], g[3], 8, 0xFF];
    let level = fx.sim().events.action.sys.stats.unit_total(p, 12, 0) as u16;
    let proximity = vec![0x76, 0, g[0], g[1], g[2], g[3]];
    let relator2 = vec![0x48, 0, 0, g[0], g[1], g[2], g[3], 0, 0, 0, 0];
    assert_eq!(
        fx.joined,
        vec![
            assign,
            states,
            proximity,
            handshake,
            load,
            // 0x53: the new act's environment record.
            vec![0x53, 2, 0, 0, 0, 0, 0, 0, 0, 0],
            reveal.clone(),
            reveal,
            place,
            vec![0x7E, 0, 0, 0, 0],
            // The town's preset waypoint (`drlg::TOWN_WAYPOINT`, objects
            // row 0, the town room at the origin), created by the object
            // pass of the room population: its
            // S→C 0x51. The game's join sends the town's 17 objects this
            // way (the waypoint, class 119, among them); the real build's
            // order and modes are measured by the packets channel of
            // `packets-town-arrival-ama` (MATCH 40/40; q-fix-real-preset-objects).
            {
                let (_, _, wx, wy) = test_fixtures::drlg::TOWN_WAYPOINT;
                let mut m = vec![0x51, 2, 1, 0, 0, 0, 0, 0];
                m.extend((wx as u16).to_le_bytes());
                m.extend((wy as u16).to_le_bytes());
                m.extend([0, 0]);
                m
            },
            vec![0x04],
            // q-fix-flow-server: the
            // inventory refresh's 0x48, then the join sequence:
            // 0x5B, 0x65, 0x8D, the join 0x5A.
            relator2,
            msg::player_joined(fx.guid(p), CLASS as u8, &name(), level, 0xFFFF),
            msg::player_kill_count(fx.guid(p), 0).to_vec(),
            vec![0x8D, g[0], g[1], g[2], g[3], 0xFF, 0xFF],
            msg::player_event(2, &name()).to_vec(),
        ],
        "{:02x?}",
        fx.joined
    );
    // Standing still: nothing more.
    assert_eq!(fx.frames(5), Vec::<Vec<u8>>::new());
    fx.assert_clean();
}

// ---- 2. a chest and a shrine ------------------------------------------------------------

#[test]
fn opening_a_chest() {
    let mut fx = Fx::new();
    let at = fx.spot(0);
    let chest = fx.object(CHEST, at);
    let g = fx.guid(chest);
    // Creation: `Selectable0` 1 → flag 0x2; mode 0.
    assert_eq!(fx.mode(chest), 0);
    let c0 = fx.control_seed();
    let (codes, got) = fx.step(&[interact(g)]);
    assert_eq!(codes, [ResultCode::Done]);
    // Operating the install's chest (no init function: `InteractType` 0,
    // not locked, not sparkling): picks 1, Q 0, one `roll(100)` on the
    // control seed; `Mode1` 0 → mode 2, flag 0x2 cleared; the update
    // pass sends 0x0E with selectable 0, mode 2.
    let mut c = c0;
    let r = c.roll(100);
    assert_eq!(fx.control_seed(), c, "one control-seed draw");
    assert_eq!(fx.mode(chest), 2);
    assert_eq!(got, [object_state(g, false, 2)]);
    // The drop `D(Q)` when r
    // ≥ 25: the chest-drop seam has no provider on the wired host
    // (`ChestWorld::chest_drop` default), so no item exists.
    let items = fx.host.game.game.lists.units_of_type(UnitType::Item);
    assert!(
        items.is_empty(),
        "r = {r}: chest drop is Pending, {items:?}"
    );
    // Rule 1: an open chest does nothing (past the client's 200 ms
    // duplicate window).
    assert_eq!(fx.frames(5), Vec::<Vec<u8>>::new());
    let (codes, got) = fx.step(&[interact(g)]);
    assert_eq!((codes, got), (vec![ResultCode::Done], vec![]));
    assert_eq!(fx.control_seed(), c);
    assert!(fx.routes().is_empty());
    fx.assert_clean();
}

#[test]
fn a_poked_operate_is_the_clients_0x13() {
    // The same chest operated twice from the same state: once by the
    // client's 0x13 through the queue and the frame's drain, once by
    // `operate` (`d2_sim::poke::interact_calls`, the dispatcher run now
    // by `Host::dispatch_now`) before the frame. Same result code, same
    // object state, same control seed, same S→C messages.
    let run = |poked: bool| {
        let mut fx = Fx::new();
        let at = fx.spot(0);
        let chest = fx.object(CHEST, at);
        let g = fx.guid(chest);
        let (codes, got) = if poked {
            let d = d2_sim::poke::parse_directive_text(&format!("operate 2/{g}")).unwrap();
            let calls = d2_sim::poke::interact_calls_with(&d, 2, g, fx.guid(fx.player));
            assert_eq!(calls.len(), 1);
            let mut m = vec![calls[0].id];
            for v in &calls[0].values {
                m.extend(v.to_le_bytes());
            }
            assert_eq!(m, interact(g));
            fx.stage(fx.player);
            let code = fx.host.dispatch_now(CLIENT, &m).expect("a player");
            let (none, got) = fx.step(&[]);
            assert!(none.is_empty());
            (vec![code], got)
        } else {
            fx.step(&[interact(g)])
        };
        (codes, got, fx.mode(chest), fx.control_seed())
    };
    let (sent, poked) = (run(false), run(true));
    assert_eq!(sent.0, [ResultCode::Done]);
    assert_eq!(sent.2, 2, "the chest opened");
    assert_eq!(poked, sent);
}

#[test]
fn using_a_shrine() {
    let mut fx = Fx::new();
    let at = fx.spot(1);
    let c0 = fx.control_seed();
    let shrine = fx.object(SHRINE, at);
    // The shrine init (`Parm0` 0): n = 2 − 1 rows; id := `roll(1)` + 1 = 1
    // on the control seed (`LevelMin` 0 ≤ level 1: one try).
    let mut c = c0;
    assert_eq!(c.roll(1) + 1, SHRINE_ID as u32);
    assert_eq!(fx.control_seed(), c);
    let data = |fx: &mut Fx| {
        fx.sim()
            .events
            .action
            .hooks()
            .objects
            .as_ref()
            .unwrap()
            .control
            .data[&shrine]
    };
    let d = data(&mut fx);
    assert_eq!(
        (d.interact, d.shrine),
        (SHRINE_ID as u8, Some(SHRINE_ID as u16))
    );
    let (g, pg) = (fx.guid(shrine), fx.guid(fx.player));
    let (codes, got) = fx.step(&[interact(g)]);
    assert_eq!(codes, [ResultCode::Done]);
    let f = fx.frame() - 1;
    // Operating it: the object's operate field := operator GUID + 1, flag 0x1, mode 1; the
    // hover is created (the host now provides it; q-doors) and event 6 is
    // queued at frame + 300; the code-2 effect runs on the stat seams
    // (none on the wired host); reset time 1 minute → event
    // 5 at frame + 1201.
    assert_eq!(fx.mode(shrine), 1);
    assert_eq!(data(&mut fx).operator, pg + 1);
    assert_eq!(fx.timers(shrine), [(5, f + 1201), (6, f + 300)]);
    // The update pass: 0x0E (mode 1), then, mode 1 with `SubClass` bit 0 and
    // an operator, 0x4D: 2 @1, object GUID @2, operator GUID @6, the
    // shrine's `Code` @10, zero u8 @11 and u16s @13, @15 (17 bytes).
    let mut shrine_msg = vec![0x4D, 2];
    shrine_msg.extend(g.to_le_bytes());
    shrine_msg.extend(pg.to_le_bytes());
    shrine_msg.push(SHRINE_CODE);
    shrine_msg.extend([0; 6]);
    // Then the hover (flag 0x100, 0x26
    // form 5): u8@2 0, unit type 2 @3, GUID @4, bytes 8–9 unwritten (0),
    // empty name @10, the text "%d"(3683 + shrine id) after it.
    let mut hover = vec![0x26, 5, 0, 2];
    hover.extend(g.to_le_bytes());
    hover.extend([0; 2]);
    hover.push(0);
    hover.extend(format!("{}", 3683 + SHRINE_ID).bytes());
    hover.push(0);
    assert_eq!(got, [object_state(g, true, 1), shrine_msg, hover]);
    // A used shrine refuses (nothing sent).
    assert_eq!(fx.frames(5), Vec::<Vec<u8>>::new());
    let (codes, got) = fx.step(&[interact(g)]);
    assert_eq!((codes, got), (vec![ResultCode::Done], vec![]));
    // Event 6 at frame + 300: the hover has expired → freed, queued
    // with flag 0x100: 0x76 (type u8@1, GUID u32@2),
    // and nothing else until event 5.
    let n = (f + 300 - fx.frame()) as usize;
    assert_eq!(fx.frames(n - 1), Vec::<Vec<u8>>::new());
    let mut gone = vec![0x76, 2];
    gone.extend(g.to_le_bytes());
    assert_eq!(fx.frames(1), [gone]);
    assert_eq!(fx.frame(), f + 300);
    // Event 5 (`SubClass` bit 0): mode 0, the operate field := 0, queued: 0x0E.
    let n = (f + 1201 - fx.frame()) as usize;
    let quiet = fx.frames(n - 1);
    assert_eq!(quiet, Vec::<Vec<u8>>::new());
    let got = fx.frames(1);
    assert_eq!(fx.frame(), f + 1201);
    assert_eq!(fx.mode(shrine), 0);
    assert_eq!(data(&mut fx).operator, 0);
    assert_eq!(got, [object_state(g, true, 0)]);
    fx.assert_clean();
}

// ---- 3. Act I: Cairn stone and gibbet --------------------------------------------------

#[test]
fn cairn_stone_inits_on_the_created_quest_control() {
    let mut fx = Fx::new();
    // A new game's chain-4 record has not-intro 1, so the
    // quest is live: X `b4d`, X `cain_gone` and the
    // stone's reset byte are 0, so the init changes nothing.
    assert!(fx.world().quests.record(A1Q4).unwrap().not_intro);
    let at = fx.spot(2);
    let live = fx.object(STONE, at);
    // The quest init is queued by the allocation and run on the host's
    // quest control after the tick.
    assert_eq!(fx.frames(1), Vec::<Vec<u8>>::new());
    assert_eq!(fx.mode(live), 0);
    let x = fx.world().quests.record(A1Q4).unwrap().extra.q4.clone();
    assert_eq!((x.portal_stone, x.portal_timer), (0, false));
    // A game where the quest is done or skipped (not-intro 0, staged):
    // step 3 on the next class-17 stone: X `portal_stone` := its GUID, the
    // Tristram-portal timer starts, object mode := 2 (sent as 0x0E by
    // the update pass).
    fx.world().quests.record_mut(A1Q4).unwrap().not_intro = false;
    let at = fx.spot(5);
    let stone = fx.object(STONE, at);
    let got = fx.frames(1);
    let g = fx.guid(stone);
    let x = fx.world().quests.record(A1Q4).unwrap().extra.q4.clone();
    assert_eq!(x.portal_stone, g, "X +0x40 := the class-17 stone");
    assert!(x.portal_timer, "X +0x44 := 1");
    assert_eq!(fx.mode(stone), 2, "object mode := 2");
    // The mode's 0x0E. Run inside the allocation, the init's 0x0E
    // would go out on the next client pass; D2RUST runs the queued init
    // after this tick, so it comes one tick
    // later (finding N-3). Both ticks
    // are collected; the tick itself is not asserted.
    let mut got = got;
    got.extend(fx.frames(1));
    assert_eq!(got, [object_state(g, true, 2)]);
    // The timer creates the Tristram portal through the rest's
    // `create_portal` (no provider: refused), so step 3's
    // failure path keeps the timer pending and creates no portal.
    fx.frames(4);
    let x = fx.world().quests.record(A1Q4).unwrap().extra.q4.clone();
    assert!(x.portal_timer && !x.portal_made);
    assert!(fx.routes().is_empty());
    fx.assert_clean();
}

#[test]
fn opening_cains_gibbet_and_its_event_7() {
    let mut fx = Fx::new();
    // The gibbet opens the quest only when not-intro ≠ 0: the state the intro
    // (Akara's first talk) leaves, staged here.
    fx.world().quests.record_mut(A1Q4).unwrap().not_intro = true;
    let at = fx.spot(3);
    let gibbet = fx.object(GIBBET, at);
    fx.frames(1);
    let (g, pg) = (fx.guid(gibbet), fx.guid(fx.player));
    let (codes, got) = fx.step(&[interact(g)]);
    assert_eq!(codes, [ResultCode::Done]);
    let f = fx.frame() - 1;
    // Step 4: mode 1, object event 1 at frame + (FrameCnt1 >> 8); step
    // 5: X `gibbet_open` := 3, X `gibbet_player` := the player's GUID; step 6: event 7 at
    // frame + 17.
    assert_eq!(fx.mode(gibbet), 1);
    let x = fx.world().quests.record(A1Q4).unwrap().extra.q4.clone();
    assert_eq!((x.gibbet_open, x.gibbet_player), (3, pg));
    assert_eq!(fx.timers(gibbet), [(1, f + 15), (7, f + 17)]);
    // Step 8: 4.13, then 4.1; 0x28 to the player;
    // the update pass's 0x0E for mode 1.
    assert!(fx.quests().flags[0].get(4, 13) && fx.quests().flags[0].get(4, 1));
    let info = quest_info(fx.quests());
    assert!(got.contains(&info), "{got:02x?}");
    assert!(got.contains(&object_state(g, true, 1)), "{got:02x?}");
    // Event 7 at frame + 17: X `gibbet_open` := 3, object mode := 3 (sent as 0x0E).
    // Event 7 runs in tick step 4 on the host's lent quest control, so
    // the client pass (step 5) of the same tick sends the 0x0E
    // (finding N-3 fixed): no 0x0E in the ticks before, exactly one in that tick.
    let mut last = Vec::new();
    while fx.frame() < f + 17 {
        assert_eq!(fx.mode(gibbet), 1);
        assert!(!last.iter().any(|m: &Vec<u8>| m[0] == 0x0E), "{last:02x?}");
        last = fx.frames(1);
    }
    assert_eq!(fx.mode(gibbet), 3);
    let states: Vec<_> = last.iter().filter(|m| m[0] == 0x0E).cloned().collect();
    assert_eq!(states, [object_state(g, true, 3)]);
    // Step 3's position and room come from the path provider and the
    // object's list room (`HostQuests::unit_position`, finding N-2
    // fixed): no fault. The cain1 spawn and the free-spot search are the
    // rest's (no provider: none), so step 4 runs: nobody in Tristram, no
    // portal; X `town_cain_due` := 1 (Cain still to spawn in town), X `cain_failed` := 1.
    assert_eq!(fx.world().quests.faults, Vec::<QuestError>::new());
    let x = fx.world().quests.record(A1Q4).unwrap().extra.q4.clone();
    assert!(x.town_cain_due && x.cain_failed);
    assert!(!x.out_portal && x.found_player.is_none());
    assert!(fx.routes().is_empty());
    fx.assert_clean();
}

// ---- 4. Act II: Horazon's journal -------------------------------------------------------

#[test]
fn reading_horazons_journal() {
    let mut fx = Fx::new();
    // The journal's state change runs only with not-intro ≠ 0 (the
    // state Jerhyn's talk leaves), staged.
    fx.world().quests.record_mut(A2Q4).unwrap().not_intro = true;
    let at = fx.spot(4);
    let tome = fx.object(JOURNAL, at);
    fx.frames(1);
    let g = fx.guid(tome);
    let (codes, got) = fx.step(&[interact(g)]);
    assert_eq!(codes, [ResultCode::Done]);
    let f = fx.frame() - 1;
    // Mode 0 → 1 and the end-animation event at frame + (FrameCnt1 >> 8).
    assert_eq!(fx.mode(tome), 1);
    assert_eq!(fx.timers(tome), [(1, f + 15)]);
    // Chain 11: state := 5; the player is not in level 74 (the grants
    // stay off); the completion flag: the player lacks 12.0 and 12.1 →
    // 12.14 and `5D 0B 00 0C 0000` (the quest-done
    // message, act argument 0).
    assert_eq!(fx.world().quests.record(A2Q4).unwrap().state, 5);
    let q = fx.quests().flags[0];
    assert!(q.get(A2Q4_SLOT, 14));
    assert!(!q.get(A2Q4_SLOT, 0) && !q.get(A2Q4_SLOT, 1) && !q.get(A2Q4_SLOT, 13));
    let done = vec![0x5D, A2Q4, 0x00, 0x0C, 0x00, 0x00];
    assert!(got.contains(&done), "{got:02x?}");
    assert!(got.contains(&object_state(g, true, 1)), "{got:02x?}");
    // The scroll text 396: S→C
    // 0x27 type 2, the journal's GUID, count 1, entry 0 kind 0 with the
    // string; bytes 7, 9, 12–39 are 0 here (unwritten by the game, masked).
    let mut scroll = vec![0x27, 2];
    scroll.extend_from_slice(&g.to_le_bytes());
    scroll.extend_from_slice(&[1, 0, 0, 0]);
    scroll.extend_from_slice(&396u16.to_le_bytes());
    scroll.resize(40, 0);
    assert!(got.contains(&scroll), "{got:02x?}");
    // The scroll's room field := the tome's room (`unit_position` on the wired host,
    // finding N-2 fixed).
    let room = fx
        .world()
        .quests
        .record(A2Q4)
        .unwrap()
        .extra
        .a2
        .q4
        .tome_room;
    assert!(room.is_some());
    // A second read: the mode stays 1 (the mode-0 test); state is 5
    // now, and the grants and the completion flag run only inside "not
    // intro and state ≠ 5" (QB-7): nothing is
    // sent again.
    fx.frames(5);
    let (_, again) = fx.step(&[interact(g)]);
    assert_eq!(fx.mode(tome), 1);
    assert!(!again.contains(&done), "{again:02x?}");
    assert!(!again.iter().any(|m| m[0] == 0x5D), "{again:02x?}");
    assert!(fx.routes().is_empty());
    fx.assert_clean();
}

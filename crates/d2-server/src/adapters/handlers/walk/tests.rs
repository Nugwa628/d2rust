//! C→S 0x01–0x04 through `SimGame::handle` and the game tick on the
//! wired sim (`ActionSim` with the path provider on, a synthetic act 0:
//! Cold Plains as rooms A and B side by side, the town (level 1) as room
//! C). Only `Pending` (interaction, the transport) is a test provider.
//! Positions are the path records' own. The message bytes are built with
//! `d2-proto`'s typed builders. These are the end-to-end tests of
//! [`super`]: request, movement ticks, the update-pass messages, the
//! client vitals sync, and the id table check.

use std::collections::BTreeMap;
use std::sync::Arc;

use d2_data::bin::BinTable;
use d2_data::fixup::records::stat_ops;
use d2_data::tables::{Charstats, Itemstatcost, Levels, Objects, Record};
use d2_proto::server::{
    MapHide, MapReveal, PlayerMove, PlayerStop, PlayerToTarget, ReassignPlayer, RemoveUnit,
};
use d2_sim::combat::CombatTables;
use d2_sim::drlg::room::LinkAt;
use d2_sim::drlg::{
    CellGrid, Drlg, DrlgData, DrlgError, DrlgRoomId, Dungeon, GridPass, LevelDef, LevelIdx,
    LevelTypes, RoomGrids, RoomKind, TileInfo, TileRect, TileSource,
};
use d2_sim::game::Game;
use d2_sim::path::{CollisionRooms, DynamicPath};
use d2_sim::rng::Seed;
use d2_sim::skills::SkillTables;
use d2_sim::stats::{StatData, StatTable};
use d2_sim::units::hooks::UnitData;
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::{RoomId, UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionSim, ActionTables, DrlgWorld, Pending};
use d2_sim::world::waypoints::{WaypointData, NO_WAYPOINT};

use super::{enable_paths, form, Form, WALK_IDS};
use crate::adapters::handlers::world::{ActionWorld, Outbox};
use crate::adapters::{PlayerData, PlayerFields, SimGame};
use crate::buffers::QueueError;
use crate::seams::{ClientId, Intents, MessageSink, PlayerGate, ResultCode, Tick};

// ---- fixture -----------------------------------------------------------------------------

/// The seams with no provider: the transport.
#[derive(Default)]
struct TestPending {
    sent: Vec<(UnitId, Vec<u8>)>,
}

impl Pending for TestPending {
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.sent.push((player, msg.to_vec()));
    }
}

impl Outbox for TestPending {
    fn take_sent(&mut self) -> Vec<(UnitId, Vec<u8>)> {
        std::mem::take(&mut self.sent)
    }
}

/// Every message queued, in order.
#[derive(Default)]
struct Sink(Vec<(ClientId, Vec<u8>)>);

impl MessageSink for Sink {
    fn queue(&mut self, client: ClientId, msg: &[u8]) -> Result<(), QueueError> {
        self.0.push((client, msg.to_vec()));
        Ok(())
    }
}

/// The listed rooms per level, every cell a floor.
struct Types(BTreeMap<u32, Vec<TileRect>>);

impl LevelTypes for Types {
    fn generate(
        &mut self,
        drlg: &mut Drlg,
        _: &DrlgData,
        level: LevelIdx,
    ) -> Result<(), DrlgError> {
        let id = drlg.level(level).id;
        for rect in self.0.get(&id).cloned().unwrap_or_default() {
            let r = drlg.alloc_room(level, RoomKind::Preset, rect);
            drlg.room_mut(r).dt1_mask = 1;
            drlg.link_room(r, LinkAt::Tail);
        }
        Ok(())
    }
    fn room_grids(
        &mut self,
        drlg: &mut Drlg,
        _: &DrlgData,
        room: DrlgRoomId,
    ) -> Result<RoomGrids, DrlgError> {
        use d2_sim::drlg::tiles::cell;
        let r = drlg.room(room).rect;
        let (w, h) = (r.w as usize + 1, r.h as usize + 1);
        let mut g = CellGrid::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let edge = x == 0 || y == 0 || x == w - 1 || y == h - 1;
                g.set(x, y, cell::FLOOR | if edge { cell::LINKED } else { 0 });
            }
        }
        Ok(RoomGrids {
            passes: vec![GridPass {
                cells: g,
                orientation: None,
                fill_blanks: false,
            }],
            ..RoomGrids::default()
        })
    }
}

struct Tiles(BTreeMap<Vec<u8>, Vec<TileInfo>>);

impl TileSource for Tiles {
    fn dt1(&self, path: &[u8]) -> Option<&[TileInfo]> {
        self.0.get(path).map(Vec::as_slice)
    }
}

fn tile(o: u32, main: u32, sub: u32, rarity: u32) -> TileInfo {
    TileInfo {
        orientation: o,
        main,
        sub,
        rarity,
        material: 0,
        subtile_flags: [0; 25],
        roof_height: 0,
        height: 0,
        light_direction: 0,
    }
}

fn tiles() -> Tiles {
    use d2_sim::drlg::tiles::FIXED_LIBRARY;
    let mut t = BTreeMap::new();
    t.insert(b"floor.dt1".to_vec(), vec![tile(0, 0, 0, 1)]);
    let blank_tile = |sub| {
        let mut x = tile(0, 30, sub, 0);
        x.subtile_flags = [0x20; 25];
        x
    };
    t.insert(
        FIXED_LIBRARY[0].to_vec(),
        vec![blank_tile(0), blank_tile(1)],
    );
    t.insert(FIXED_LIBRARY[1].to_vec(), vec![]);
    t.insert(FIXED_LIBRARY[2].to_vec(), vec![tile(10, 0, 0, 0)]);
    Tiles(t)
}

fn blank<T: Record>() -> T {
    T::decode(&vec![0u8; T::SIZE])
}

/// Cold Plains (rooms A, B; waypoint index 1) and the town, level 1
/// (room C; index 0).
const COLD_PLAINS: u32 = 3;
const TOWN: u32 = 1;
/// The players' class: sorceress.
const CLASS: u32 = 1;
/// `stamina` (8.8 fixed), `velocitypercent` (its creation value is 100).
const STAT_STAMINA: u16 = 10;
const STAT_VELOCITY: u16 = 67;
/// Charstats of [`CLASS`]: `WalkVelocity` 6 (0x600 per tick),
/// `RunVelocity` 9, `RunDrain` 20.
const WALK_VELOCITY: u8 = 6;
const RUN_DRAIN: u8 = 20;

fn stat_data() -> Arc<StatData> {
    let (n, size) = (359, Itemstatcost::SIZE);
    let mut records = vec![0u8; n * size];
    for (s, r) in records.chunks_mut(size).enumerate() {
        for o in [0x32, 0x48, 0x4A, 0x56, 0x58, 0x5A, 0x5C] {
            r[o..o + 2].copy_from_slice(&0xFFFFu16.to_le_bytes());
        }
        r[0..2].copy_from_slice(&(s as u16).to_le_bytes());
    }
    let mut t = BinTable {
        name: "itemstatcost".into(),
        source: "synthetic".into(),
        count: n,
        record_size: size,
        records,
    };
    stat_ops(&mut t);
    Arc::new(StatData {
        stats: StatTable::from_fixed(&t).expect("itemstatcost"),
        ..StatData::default()
    })
}

fn drlg() -> DrlgWorld {
    let mut data = DrlgData {
        levels: vec![LevelDef::default(); 150],
        ..DrlgData::default()
    };
    for l in &mut data.levels {
        l.warp = [-1; 8];
    }
    let mut files = vec![Vec::new(); 32];
    files[0] = b"floor.dt1".to_vec();
    data.lvltypes = vec![vec![Vec::new(); 32], files];
    for id in [TOWN, COLD_PLAINS] {
        data.levels[id as usize].drlg_type = 2;
        data.levels[id as usize].level_type = 1;
    }
    let mut types = Types(BTreeMap::from([
        (
            COLD_PLAINS,
            vec![TileRect::new(0, 0, 8, 8), TileRect::new(8, 0, 8, 8)],
        ),
        (TOWN, vec![TileRect::new(0, 16, 8, 8)]),
    ]));
    let mut dungeon = Dungeon::default();
    dungeon.acts[0] = Some(Drlg::create(0, 1, 0, 0, false, &data, &mut types).unwrap());
    DrlgWorld {
        dungeon,
        data: Arc::new(data),
        tiles: Box::new(tiles()),
        types: Box::new(types),
    }
}

fn tables() -> ActionTables {
    let mut charstats = vec![blank::<Charstats>(); 7];
    charstats[CLASS as usize].walkvelocity = WALK_VELOCITY;
    charstats[CLASS as usize].runvelocity = 9;
    charstats[CLASS as usize].rundrain = RUN_DRAIN;
    ActionTables {
        missiles: Vec::new(),
        skills: SkillTables {
            skills: Vec::new(),
            skilldesc: Vec::new(),
            missiles: Vec::new(),
            skills_code: Vec::new(),
            miss_code: Vec::new(),
            level_cap: 0,
            stat_count: 0,
        },
        combat: CombatTables {
            charstats,
            difficultylevels: Vec::new(),
            monstats: Vec::new(),
            monstats2: Vec::new(),
            hitclass: Vec::new(),
        },
        levels: Vec::new(),
        skill_modes: Vec::new(),
        overlay_count: 0,
        monequip: Vec::new(),
        arena: Vec::new(),
    }
}

/// `levels` rows (count 150) and one waypoint object class 0.
fn waypoint_data() -> WaypointData {
    let mut levels = vec![blank::<Levels>(); 150];
    for l in &mut levels {
        l.waypoint = NO_WAYPOINT;
    }
    levels[TOWN as usize].waypoint = 0;
    levels[COLD_PLAINS as usize].waypoint = 1;
    let mut o: Objects = blank();
    o.operatefn = 23;
    o.initfn = 17;
    o.framecnt1 = 15 << 8;
    WaypointData::new(&levels, &[o])
}

/// Messages queued, in order.
type Queued = Vec<(ClientId, Vec<u8>)>;
/// (precise x, precise y, mode, queued) after one tick.
type Frame = (u32, u32, u32, Queued);

type Sim = SimGame<ActionSim<TestPending>, ActionWorld>;

struct Fx {
    sim: Sim,
    /// Active rooms A, B (Cold Plains) and C (town).
    a: RoomId,
    b: RoomId,
    c: RoomId,
    out: Sink,
}

const ALIVE: PlayerGate = PlayerGate {
    mode: 1,
    uninterruptable: false,
};

impl Fx {
    /// The game with the path provider on (before any allocation) and
    /// every room streamed.
    fn new() -> Self {
        let hooks = ActionHooks::new(
            Arc::new(tables()),
            drlg(),
            Seed::init_low(1234),
            TestPending::default(),
        );
        let mut events = ActionSim::new(stat_data(), UnitData::default(), hooks);
        enable_paths(&mut events).expect("embedded path tables");
        let mut game = Game::new();
        game.lists.ensure_act(0).unwrap();
        let mut active = Vec::new();
        for id in [COLD_PLAINS, TOWN] {
            let r = events
                .hooks()
                .drlg
                .with_act(0, &mut game.lists, |d, svc| {
                    let l = d.get_or_alloc_level(svc.data, svc.types, id)?;
                    d.generate_level(svc.data, svc.types, l)?;
                    let mut out = Vec::new();
                    for r in d.level_rooms(l) {
                        out.push(d.stream_room(svc, r)?.expect("active"));
                    }
                    Ok::<_, DrlgError>(out)
                })
                .unwrap()
                .unwrap();
            active.extend(r);
        }
        let mut sim: Sim = SimGame::with_events(game, events);
        sim.world.waypoints = Some(waypoint_data());
        Self {
            sim,
            a: active[0],
            b: active[1],
            c: active[2],
            out: Sink::default(),
        }
    }

    fn alloc(&mut self, ty: UnitType, room: RoomId, x: i32, y: i32) -> UnitId {
        let req = AllocRequest {
            ty,
            class: if ty == UnitType::Player { CLASS } else { 0 },
            room: Some(room),
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: ty == UnitType::Player,
        };
        let u = self
            .sim
            .events
            .with(&mut self.sim.game, |g, v| v.allocate(g, &req, x, y))
            .expect("allocated");
        // Staged as already known to the clients: the room clean-up has
        // cleared unit flag 0x10, so the client pass announces nothing.
        self.sim.events.sys.units.get_mut(u).unwrap().flags &=
            !d2_sim::units::record::flags::SEED_SET;
        u
    }

    /// A player (neutral, velocity percent 100, stamina 0x6400) for
    /// `client` in `room` at (x, y).
    fn player(&mut self, client: ClientId, room: RoomId, x: i32, y: i32) -> UnitId {
        let p = self.alloc(UnitType::Player, room, x, y);
        // Players are allocated in mode 0; set neutral (1).
        self.sim.events.sys.units.get_mut(p).unwrap().mode = 1;
        self.sim.events.with(&mut self.sim.game, |_, v| {
            v.set_base(p, STAT_VELOCITY, 100);
            v.set_base(p, STAT_STAMINA, 0x6400);
        });
        self.sim
            .join(client, Some(p), Some(room), client_state::IN_GAME)
            .unwrap();
        self.sim.set_player(
            p,
            PlayerFields {
                gate: ALIVE,
                data: Some(PlayerData { last_accept: 0 }),
            },
        );
        p
    }

    /// Stages `p`'s client in `room`'s client array, as the client's room
    /// switch does in a game.
    fn stage_room_client(&mut self, p: UnitId, room: RoomId) {
        let lists = &self.sim.game.lists;
        let c = lists
            .clients()
            .into_iter()
            .find(|&c| lists.client(c).and_then(|e| e.player) == Some(p))
            .expect("client");
        let act = lists.room(room).expect("room").act;
        let d = self.sim.events.hooks().drlg.dungeon.acts[usize::from(act)]
            .as_mut()
            .expect("act");
        let id = d.drlg_room_of(room).expect("active room");
        d.add_room_client(id, c);
    }

    fn guid(&self, u: UnitId) -> u32 {
        self.sim.game.lists.unit(u).unwrap().guid
    }

    fn path(&mut self, u: UnitId) -> DynamicPath {
        self.sim
            .events
            .hooks()
            .paths
            .as_ref()
            .unwrap()
            .dynamic(u)
            .unwrap()
            .clone()
    }

    fn mode(&self, u: UnitId) -> u32 {
        self.sim.events.sys.units.get(u).unwrap().mode
    }

    fn stamina(&mut self, u: UnitId) -> i32 {
        self.sim
            .events
            .with(&mut self.sim.game, |_, v| v.stat(u, STAT_STAMINA))
    }

    /// One message from `client` through `SimGame::handle` (the
    /// dispatcher's checks are not part of this module): the result and
    /// what was queued.
    fn handle(&mut self, client: ClientId, msg: &[u8]) -> (ResultCode, Vec<(ClientId, Vec<u8>)>) {
        let r = self.sim.handle(client, msg, msg.len(), &mut self.out);
        (r, std::mem::take(&mut self.out.0))
    }

    /// One game tick; what was queued during it.
    /// One tick and the host's flush after it (the vitals sync runs in
    /// the flush, `Tick::flush_sync`).
    fn tick(&mut self) -> Vec<(ClientId, Vec<u8>)> {
        self.sim.tick(&mut self.out);
        self.sim.flush_sync(&mut self.out);
        std::mem::take(&mut self.out.0)
    }

    /// Ticks while `u` is in `mode`, at most `max`: (precise x, precise
    /// y, mode, messages) after each tick.
    fn run(&mut self, u: UnitId, mode: u32, max: usize) -> Vec<Frame> {
        let mut out = Vec::new();
        for _ in 0..max {
            let sent = self.tick();
            let d = self.path(u);
            let m = self.mode(u);
            out.push((d.precise_x, d.precise_y, m, sent));
            if m != mode {
                break;
            }
        }
        out
    }

    fn assert_clean(&mut self) {
        assert!(
            self.sim.world.faults.is_empty(),
            "{:?}",
            self.sim.world.faults
        );
        assert!(self.sim.unhandled.is_empty(), "{:?}", self.sim.unhandled);
        assert!(self.sim.tick_faults.is_empty());
        let h = self.sim.events.hooks();
        assert!(h.errors.is_empty(), "{:?}", h.errors);
        assert!(self.sim.events.sys.errors.is_empty());
    }
}

fn point(id: u8, x: u16, y: u16) -> Vec<u8> {
    let mut m = vec![id];
    m.extend_from_slice(&x.to_le_bytes());
    m.extend_from_slice(&y.to_le_bytes());
    m
}

fn unit_msg(id: u8, ty: u32, guid: u32) -> Vec<u8> {
    let mut m = vec![id];
    m.extend_from_slice(&ty.to_le_bytes());
    m.extend_from_slice(&guid.to_le_bytes());
    m
}

/// Precise x of sub-tile `x`'s centre.
fn centre(x: u32) -> u32 {
    (x << 16) | 0x8000
}

/// The movement of every walk / run in this file: (26, 10) → (31, 10)
/// at velocity 0x600: +0x6000 per tick for 13 ticks, the 14th lands on the target centre and stops
/// (mode 1).
fn assert_m1(ticks: &[Frame], mode: u32) {
    assert_eq!(ticks.len(), 14);
    for (k, t) in ticks.iter().take(13).enumerate() {
        assert_eq!(t.0, 0x1AE000 + k as u32 * 0x6000, "tick {}", k + 1);
        assert_eq!((t.1, t.2), (centre(10), mode), "tick {}", k + 1);
    }
    assert_eq!(
        (ticks[13].0, ticks[13].1, ticks[13].2),
        (centre(31), centre(10), 1)
    );
}

/// The same leg run (the run stat list's stat 67 = 100·9/6 − 100 = 50,
/// velocity 0x600 · 150 / 100 = 0x900): +0x9000 per tick for 8 ticks, the 9th lands on the
/// target centre and stops (mode 1).
fn assert_m2(ticks: &[Frame], mode: u32) {
    assert_eq!(ticks.len(), 9);
    for (k, t) in ticks.iter().take(8).enumerate() {
        assert_eq!(t.0, 0x1B1000 + k as u32 * 0x9000, "tick {}", k + 1);
        assert_eq!((t.1, t.2), (centre(10), mode), "tick {}", k + 1);
    }
    assert_eq!(
        (ticks[8].0, ticks[8].1, ticks[8].2),
        (centre(31), centre(10), 1)
    );
}

/// Two players in room A: `p` at (26, 10) for client 0, `q` at (26, 14)
/// for client 1, and an object at (31, 10).
fn two_players() -> (Fx, UnitId, UnitId, UnitId) {
    let mut fx = Fx::new();
    let a = fx.a;
    let o = fx.alloc(UnitType::Object, a, 31, 10);
    let p = fx.player(0, a, 26, 10);
    let q = fx.player(1, a, 26, 14);
    (fx, p, q, o)
}

#[test]
fn walk_to_point_moves_the_player_and_tells_the_other_client() {
    let (mut fx, p, _, _) = two_players();
    let guid = fx.guid(p);
    // The request replies nothing, result 0.
    assert_eq!(
        fx.handle(0, &point(0x01, 31, 10)),
        (ResultCode::Done, vec![])
    );
    assert_eq!(fx.mode(p), 2);
    let d = fx.path(p);
    assert_eq!((d.target_x, d.target_y, d.target_unit), (31, 10, None));
    assert_eq!(d.velocity, 0x600);
    let ticks = fx.run(p, 2, 20);
    assert_m1(&ticks, 2);
    // In the update pass of tick 1 (the mode set queued the player):
    // client 1 gets 0x0F (walk code 1, target, the cell); the walking
    // player's own client gets nothing.
    let want = PlayerMove {
        type_: 0,
        guid,
        code: 1,
        target_x: 31,
        target_y: 10,
        zero: 0,
        x: 26,
        y: 10,
    };
    assert_eq!(ticks[0].3, vec![(1, want.encode().to_vec())]);
    // The stop's mode set (neutral, message code 7) tells client 1 only:
    // 0x0D code 7 at the stop cell; the own client nothing.
    let stop = d2_sim::path::walk::messages::player_stop(0, guid, 7, 31, 10, 0, 0);
    let rest: Vec<_> = ticks[1..].iter().flat_map(|t| t.3.clone()).collect();
    assert_eq!(rest, vec![(1, stop.to_vec())]);
    assert!(fx.tick().is_empty());
    assert_eq!(fx.path(p).cell(), d2_sim::path::Point::new(31, 10));
    fx.assert_clean();
}

#[test]
fn run_to_point_drains_stamina_and_sends_the_run_code() {
    let (mut fx, p, _, _) = two_players();
    let guid = fx.guid(p);
    assert_eq!(
        fx.handle(0, &point(0x03, 31, 10)),
        (ResultCode::Done, vec![])
    );
    assert_eq!(fx.mode(p), 3);
    // The run stat list: velocity 0x900.
    assert_eq!(fx.path(p).velocity, 0x900);
    let ticks = fx.run(p, 3, 20);
    assert_m2(&ticks, 3);
    let want = PlayerMove {
        type_: 0,
        guid,
        code: 0x17,
        target_x: 31,
        target_y: 10,
        zero: 0,
        // The cell after tick 1 (0x1B1000: 27), when the update pass runs.
        x: 27,
        y: 10,
    };
    assert_eq!(ticks[0].3, vec![(1, want.encode().to_vec())]);
    // Running drains 2 · RunDrain = 40 per running tick, for 9 ticks.
    assert_eq!(fx.stamina(p), 0x6400 - 9 * 2 * i32::from(RUN_DRAIN));
    // The stop's mode set freed the run list.
    let v = fx
        .sim
        .events
        .with(&mut fx.sim.game, |_, v| v.stat(p, STAT_VELOCITY));
    assert_eq!(v, 100);
    fx.assert_clean();
}

#[test]
fn walk_and_run_to_a_unit_send_0x10() {
    for (id, mode, code) in [(0x02u8, 2u32, 0u8), (0x04, 3, 0x18)] {
        let (mut fx, p, q, o) = two_players();
        let (guid, og) = (fx.guid(p), fx.guid(o));
        // The 0x0F with the target goes out only to a client in the
        // target's room's client array.
        let a = fx.a;
        fx.stage_room_client(q, a);
        assert_eq!(
            fx.handle(0, &unit_msg(id, 2, og)),
            (ResultCode::Done, vec![])
        );
        assert_eq!(fx.mode(p), mode);
        let t = fx.path(p).target_unit.expect("target unit");
        assert_eq!((t.unit, t.ty, t.guid), (o, UnitType::Object, og));
        let ticks = fx.run(p, mode, 20);
        // The run to a unit ends where the unit distance reaches 0 (the
        // object at (31, 10) has size 0: Δ = (2, 0) gives 0 at once), at
        // the centre of cell 29.
        let (first, step, moving) = if mode == 3 {
            (0x1B1000, 0x9000, 5)
        } else {
            (0x1AE000, 0x6000, 7)
        };
        assert_eq!(ticks.len(), moving + 1);
        for (k, t) in ticks.iter().take(moving).enumerate() {
            assert_eq!(t.0, first + k as u32 * step, "tick {}", k + 1);
            assert_eq!((t.1, t.2), (centre(10), mode), "tick {}", k + 1);
        }
        let last = &ticks[moving];
        assert_eq!((last.0, last.1, last.2), (centre(29), centre(10), 1));
        let want = PlayerToTarget {
            type_: 0,
            guid,
            code,
            target_type: 2,
            target_guid: og,
            // The cell after tick 1 (the walk message is sent in its update pass).
            x: if mode == 3 { 27 } else { 26 },
            y: 10,
        };
        assert_eq!(ticks[0].3, vec![(1, want.encode().to_vec())], "{id:#04x}");
        // The stop's neutral mode (row NU) tells client 1 with 0x0D code 7.
        let rest: Vec<_> = ticks[1..].iter().flat_map(|t| t.3.clone()).collect();
        assert!(
            rest.iter()
                .all(|(to, m)| *to == 1 && m[0] == 0x0D && m[6] == 7),
            "{rest:?}"
        );
        assert!(rest.len() <= 1, "{rest:?}");
        fx.assert_clean();
    }
    // Without the client in the target's room: the point message 0x0F
    // (code 1, the target's position).
    let (mut fx, p, _, og) = two_players();
    let (guid, og) = (fx.guid(p), fx.guid(og));
    fx.handle(0, &unit_msg(0x02, 2, og));
    let ticks = fx.run(p, 2, 1);
    assert_eq!(ticks[0].3.len(), 1);
    let (to, m) = &ticks[0].3[0];
    assert_eq!((*to, m[0], m[6]), (1, 0x0F, 1));
    assert_eq!(&m[2..6], &guid.to_le_bytes());
}

#[test]
fn walking_into_room_b_changes_the_players_room() {
    // Room A is sub-tiles x 0..40, B 40..80: 6 sub-tiles at 0x6000 are
    // 16 ticks; the step into x = 40 moves the player to B's unit list.
    let mut fx = Fx::new();
    let (a, b) = (fx.a, fx.b);
    let p = fx.player(0, a, 37, 10);
    assert_eq!(
        fx.handle(0, &point(0x01, 43, 10)),
        (ResultCode::Done, vec![])
    );
    let ticks = fx.run(p, 2, 30);
    assert_eq!(ticks.len(), 16);
    assert_eq!(
        (ticks[15].0, ticks[15].1, ticks[15].2),
        (centre(43), centre(10), 1)
    );
    // One client: nothing is sent while walking.
    assert!(ticks.iter().all(|t| t.3.is_empty()));
    let d = fx.path(p);
    assert_eq!((d.room, d.prev_room), (Some(b), Some(a)));
    assert_eq!(fx.sim.game.lists.unit(p).unwrap().room(), Some(b));
    fx.assert_clean();
}

#[test]
fn refused_requests_still_return_0() {
    let (mut fx, p, _, _) = two_players();
    // No unit of that GUID: a log line, nothing else.
    assert_eq!(
        fx.handle(0, &unit_msg(0x02, 2, 999)),
        (ResultCode::Done, vec![])
    );
    assert_eq!(fx.mode(p), 1);
    // Mode DT (0, dead) refuses every movement.
    fx.sim.events.sys.units.get_mut(p).unwrap().mode = 0;
    assert_eq!(
        fx.handle(0, &point(0x01, 31, 10)),
        (ResultCode::Done, vec![])
    );
    assert_eq!(fx.mode(p), 0);
    assert_eq!(fx.path(p).cell(), d2_sim::path::Point::new(26, 10));
    let ticks: Vec<_> = (0..3).map(|_| fx.tick()).collect();
    assert!(ticks.iter().all(Vec::is_empty));
    fx.assert_clean();
}

#[test]
fn without_the_path_provider_the_ids_stay_stubs() {
    // Routing check: the same game with the provider off leaves
    // 0x01–0x04 to the stub (recorded, result 0, nothing moves).
    let mut fx = Fx::new();
    fx.sim.events.hooks().paths = None;
    let a = fx.a;
    let p = fx.player(0, a, 26, 10);
    for m in [
        point(0x01, 31, 10),
        unit_msg(0x02, 0, 0),
        point(0x03, 1, 1),
        unit_msg(0x04, 0, 0),
    ] {
        assert_eq!(fx.handle(0, &m), (ResultCode::Done, vec![]));
    }
    assert_eq!(fx.mode(p), 1);
    let ids: Vec<u8> = fx.sim.unhandled.iter().map(|u| u.1).collect();
    assert_eq!(ids, [0x01, 0x02, 0x03, 0x04]);
}

/// The waypoint `o` at (20, 20) in room A, the player at (22, 20) knowing
/// the town and Cold Plains, the menu open; C→S 0x49 to `level`.
fn travel(fx: &mut Fx, level: u32) -> (UnitId, (ResultCode, Queued)) {
    let a = fx.a;
    travel_from(fx, a, level)
}

/// [`travel`] with the waypoint object in `wp_room` (the player in A).
fn travel_from(fx: &mut Fx, wp_room: RoomId, level: u32) -> (UnitId, (ResultCode, Queued)) {
    let a = fx.a;
    let o = fx.alloc(UnitType::Object, wp_room, 20, 20);
    let p = fx.player(0, a, 22, 20);
    let wp = fx.guid(o);
    let h = fx.sim.events.hooks();
    let rec = h.waypoints.entry(p).or_default().get_mut(0);
    rec.set(0).unwrap();
    rec.set(1).unwrap();
    let r = fx.sim.events.sys.units.get_mut(p).unwrap();
    r.interact.set(2, wp);
    let mut m = vec![0x49];
    m.extend_from_slice(&wp.to_le_bytes());
    m.extend_from_slice(&(level as u16).to_le_bytes());
    m.extend_from_slice(&[0, 0]);
    let r = fx.handle(0, &m);
    (p, r)
}

#[test]
fn waypoint_to_the_town_places_the_player_and_sends_0x0d() {
    // A same-act warp places the player in the destination's spawn room
    // (C, the town's one room), so the arrival runs a mode request 2 at
    // its own position (no path → neutral, town: 5) and S→C 0x0D at
    // x + 3, y + 3. The placement's 0x07 comes first.
    let mut fx = Fx::new();
    let c = fx.c;
    let a = fx.a;
    let (p, r) = travel(&mut fx, TOWN);
    let guid = fx.guid(p);
    let wp = fx
        .sim
        .game
        .lists
        .room_units(a)
        .into_iter()
        .find(|&u| u != p)
        .expect("the waypoint stays in A");
    let d = fx.path(p);
    assert_eq!(d.room, Some(c));
    assert_eq!(fx.sim.game.lists.unit(p).unwrap().room(), Some(c));
    let rect = fx.sim.events.hooks().drlg.subtile_rect(c).unwrap();
    assert!(rect.contains(d.x(), d.y()));
    assert_eq!(fx.mode(p), 5);
    let (x, y) = (d.x() as u16, d.y() as u16);
    let reveal = MapReveal {
        x: 0,
        y: 16,
        level: TOWN as u8,
    };
    let stop = PlayerStop {
        type_: 0,
        guid,
        a: 1,
        x: x + 3,
        y: y + 3,
        b: 0,
        life_pct: 0,
    };
    assert_eq!(
        r,
        (
            ResultCode::Done,
            vec![(0, reveal.encode().to_vec()), (0, stop.encode().to_vec())]
        )
    );
    assert_ne!(fx.sim.events.sys.units.get(p).unwrap().flags2 & 0x10000, 0);
    // Tick 1's per-client update walks the client's room (still A)
    // adjacency before the room switch, so C's queued player sends
    // nothing there. The room switch then sends 0x07 for each room of C's
    // array the client joins (the one-room town: C), and leaves A's array
    // {A, B} in its order: 0x0A for each unit of A (the waypoint), 0x08 A,
    // then, A being the client's old room, the player update (0x15 with
    // flag 1 for flags 2 bit 0x10000), then 0x08 B.
    let hide = |x: u16| MapHide {
        x,
        y: 0,
        level: COLD_PLAINS as u8,
    };
    let remove = RemoveUnit {
        type_: 2,
        guid: fx.guid(wp),
    };
    let reassign = ReassignPlayer {
        type_: 0,
        guid,
        x: d.x() as u16,
        y: d.y() as u16,
        flag: 1,
    };
    assert_eq!(
        fx.tick(),
        vec![
            (0, reveal.encode().to_vec()),
            (0, remove.encode().to_vec()),
            (0, hide(0).encode().to_vec()),
            (0, reassign.encode().to_vec()),
            (0, hide(8).encode().to_vec()),
        ]
    );
    for _ in 0..2 {
        assert!(fx.tick().is_empty());
    }
    fx.assert_clean();
}

#[test]
fn a_warp_within_the_level_sends_0x15_in_the_next_update_pass() {
    // Travel to Cold Plains, the player's level, from the town's waypoint
    // (travel to the waypoint's own level only closes the menu): placed
    // in room A (the client's room), so tick 1's update pass sends 0x15
    // (flags 2 bit 0x10000 → flag 1) at the player's cell. The arrival's
    // room test fails (the spawn search's room is not A), so no 0x0D.
    let mut fx = Fx::new();
    let a = fx.a;
    let c = fx.c;
    let (p, r) = travel_from(&mut fx, c, COLD_PLAINS);
    let guid = fx.guid(p);
    let reveal = MapReveal {
        x: 0,
        y: 0,
        level: COLD_PLAINS as u8,
    };
    assert_eq!(r, (ResultCode::Done, vec![(0, reveal.encode().to_vec())]));
    let d = fx.path(p);
    assert_eq!(d.room, Some(a));
    let reassign = ReassignPlayer {
        type_: 0,
        guid,
        x: d.x() as u16,
        y: d.y() as u16,
        flag: 1,
    }
    .encode()
    .to_vec();
    assert_eq!(fx.tick(), vec![(0, reassign)]);
    assert!(fx.tick().is_empty());
    // The room clean-up clears flags 2 bit 0x10000 after that pass, so
    // the next queueing (the walk's mode set) sends no 0x15; the own
    // client gets no walk message either.
    assert_eq!(
        fx.handle(0, &point(0x01, 26, 22)),
        (ResultCode::Done, vec![])
    );
    assert_ne!(fx.sim.events.sys.units.get(p).unwrap().flags & 0x1, 0);
    assert_eq!(fx.tick(), vec![]);
    assert_eq!(fx.sim.events.sys.units.get(p).unwrap().flags2 & 0x10000, 0);
    fx.assert_clean();
}

// ---- the update pass ---------------------------------------------------------------------

/// S→C 0x15 for `u` at its path cell with `flag`.
fn reassign(fx: &mut Fx, u: UnitId, flag: u8) -> Vec<u8> {
    let d = fx.path(u);
    ReassignPlayer {
        type_: 0,
        guid: fx.guid(u),
        x: d.x() as u16,
        y: d.y() as u16,
        flag,
    }
    .encode()
    .to_vec()
}

/// S→C 0x0F for `u` walking from (26, 10) to (31, 10) with `code`.
fn m1_move(fx: &Fx, u: UnitId, code: u8) -> Vec<u8> {
    PlayerMove {
        type_: 0,
        guid: fx.guid(u),
        code,
        target_x: 31,
        target_y: 10,
        zero: 0,
        x: 26,
        y: 10,
    }
    .encode()
    .to_vec()
}

fn set_flags2(fx: &mut Fx, u: UnitId, bits: u32) {
    fx.sim.events.sys.units.get_mut(u).unwrap().flags2 |= bits;
}

#[test]
fn bit_0x800_tells_only_the_other_clients_before_the_mode_update() {
    // Flags 2 bit 0x800 → 0x15 with flag 0 to every client whose player
    // is not the unit; the unit's own client gets nothing. The 0x15 step
    // runs before the walk-message step in the same pass, so client 1
    // gets 0x15, then the walk's 0x0F.
    let (mut fx, p, _, _) = two_players();
    set_flags2(&mut fx, p, 0x800);
    let x15 = reassign(&mut fx, p, 0);
    assert_eq!(
        fx.handle(0, &point(0x01, 31, 10)),
        (ResultCode::Done, vec![])
    );
    let ticks = fx.run(p, 2, 20);
    assert_m1(&ticks, 2);
    assert_eq!(ticks[0].3, vec![(1, x15), (1, m1_move(&fx, p, 1))]);
    fx.assert_clean();
}

#[test]
fn bit_0x10000_tells_every_client_before_the_mode_update() {
    // Bit 0x10000 → 0x15 with flag 1, the own client included; bit
    // 0x10000 wins over 0x800 (flag 1, and the own client still gets it).
    // Then the walk message for the other client only.
    let (mut fx, p, _, _) = two_players();
    set_flags2(&mut fx, p, 0x10000 | 0x800);
    let x15 = reassign(&mut fx, p, 1);
    assert_eq!(
        fx.handle(0, &point(0x01, 31, 10)),
        (ResultCode::Done, vec![])
    );
    let ticks = fx.run(p, 2, 20);
    assert_m1(&ticks, 2);
    let mut got = ticks[0].3.clone();
    // Clients run in client-list order; per client the 0x15 comes before
    // the walk message.
    got.sort_by_key(|m| m.0);
    assert_eq!(
        got,
        vec![(0, x15.clone()), (1, x15), (1, m1_move(&fx, p, 1))]
    );
    fx.assert_clean();
}

#[test]
fn town_walk_sends_the_walk_code() {
    // In the town the walk request sets mode 6 (town walk); its row
    // shares the walk's code 1.
    let mut fx = Fx::new();
    let c = fx.c;
    let rect = fx.sim.events.hooks().drlg.subtile_rect(c).unwrap();
    let (x, y) = (rect.x + 10, rect.y + 10);
    let p = fx.player(0, c, x, y);
    fx.player(1, c, x, y + 4);
    assert_eq!(
        fx.handle(0, &point(0x01, (x + 5) as u16, y as u16)),
        (ResultCode::Done, vec![])
    );
    assert_eq!(fx.mode(p), 6);
    let want = PlayerMove {
        type_: 0,
        guid: fx.guid(p),
        code: 1,
        target_x: (x + 5) as u16,
        target_y: y as u16,
        zero: 0,
        x: x as u16,
        y: y as u16,
    };
    let ticks = fx.run(p, 6, 20);
    // The stop sets town neutral (5).
    assert_eq!(ticks.last().unwrap().2, 5);
    assert_eq!(ticks[0].3, vec![(1, want.encode().to_vec())]);
    // The town neutral (TN 5) tells client 1 with 0x0D code 7.
    let rest: Vec<_> = ticks[1..].iter().flat_map(|t| t.3.clone()).collect();
    assert!(
        rest.iter()
            .all(|(to, m)| *to == 1 && m[0] == 0x0D && m[6] == 7),
        "{rest:?}"
    );
    assert_eq!(rest.len(), 1, "{rest:?}");
    fx.assert_clean();
}

#[test]
fn only_players_get_the_movement_messages() {
    // An object with a player's flags in a walk mode, queued in the
    // clients' room: the walk-message and 0x15 steps are the player's
    // (monsters have their own update, not written here; other unit types
    // have none), so nothing is sent.
    let (mut fx, _, _, o) = two_players();
    let r = fx.sim.events.sys.units.get_mut(o).unwrap();
    r.mode = 2;
    r.flags |= d2_sim::units::record::flags::CHANGED;
    r.flags2 |= 0x10000 | 0x800;
    fx.sim.game.lists.queue_update(o).unwrap();
    assert!(fx.tick().is_empty());
    fx.assert_clean();
}

#[test]
fn the_update_pass_sends_nothing_without_the_path_provider() {
    // Update-pass check: the same queued player with bit 0x10000 sends
    // 0x15 with the provider on, nothing with it off.
    for on in [true, false] {
        let (mut fx, p, _, _) = two_players();
        set_flags2(&mut fx, p, 0x10000);
        let x15 = reassign(&mut fx, p, 1);
        if !on {
            fx.sim.events.hooks().paths = None;
        }
        fx.sim.game.lists.queue_update(p).unwrap();
        let mut got = fx.tick();
        got.sort_by_key(|m| m.0);
        if on {
            assert_eq!(got, vec![(0, x15.clone()), (1, x15)]);
        } else {
            assert!(got.is_empty(), "{got:?}");
        }
    }
}

// ---- id table ----------------------------------------------------------------------------

const CLIENT_TSV: &str = include_str!("../../../../../../tables/sim/client-messages.tsv");

/// The table's problems against the TSV rows: each id must be a `sim`
/// `handler` row of the same name, with the layout of its form, and
/// the dispatcher's point / unit parse must agree with the form.
fn problems(ids: &[(u8, &str, &str, Form, u32)], tsv: &str) -> Vec<String> {
    let mut lines = tsv.lines();
    let head: Vec<&str> = lines.next().unwrap().split('\t').collect();
    let col = |n: &str| head.iter().position(|h| *h == n).unwrap();
    let rows: Vec<Vec<&str>> = lines.map(|l| l.split('\t').collect()).collect();
    let mut out = Vec::new();
    for w in ids.windows(2) {
        if w[0].0 >= w[1].0 {
            out.push(format!("order {:#04x}", w[1].0));
        }
    }
    for &(id, name, _, f, _) in ids {
        let key = format!("0x{id:02X}");
        let Some(r) = rows.iter().find(|r| r[col("id")] == key) else {
            out.push(format!("{id:#04x} missing"));
            continue;
        };
        let layout = match f {
            Form::Point => "x:u16@1 y:u16@3",
            Form::Unit => "type:u32@1 id:u32@5",
        };
        if r[col("name")] != name {
            out.push(format!("{id:#04x} name"));
        }
        if r[col("kind")] != "handler" || r[col("scope")] != "sim" {
            out.push(format!("{id:#04x} not a sim handler"));
        }
        if r[col("layout")] != layout {
            out.push(format!("{id:#04x} layout"));
        }
        let parsed = match f {
            Form::Point => crate::dispatch::is_point(id),
            Form::Unit => crate::dispatch::is_unit(id),
        };
        if !parsed {
            out.push(format!("{id:#04x} parse"));
        }
    }
    out
}

#[test]
fn walk_ids_match_client_messages_tsv() {
    assert_eq!(problems(WALK_IDS, CLIENT_TSV), Vec::<String>::new());
    assert_eq!(form(0x03), Some(Form::Point));
    assert_eq!(form(0x04), Some(Form::Unit));
    assert_eq!(form(0x05), None);
    // Perturbations: a stubbed row, a changed layout, a swapped form are
    // each reported.
    let stub = CLIENT_TSV.replace(
        "\thandler\talive\twalk to a unit",
        "\tstub0\talive\twalk to a unit",
    );
    assert_eq!(problems(WALK_IDS, &stub), ["0x02 not a sim handler"]);
    let moved = CLIENT_TSV.replace(
        "0x03\tRun\t5\t==5\tx:u16@1 y:u16@3",
        "0x03\tRun\t5\t==5\tx:u16@3 y:u16@1",
    );
    assert_eq!(problems(WALK_IDS, &moved), ["0x03 layout"]);
    let mut swapped = WALK_IDS.to_vec();
    swapped[3].3 = Form::Point;
    assert_eq!(
        problems(&swapped, CLIENT_TSV),
        ["0x04 layout", "0x04 parse"]
    );
}

// ---- client vitals sync ------------------------------------------------------------------

const STAT_HITPOINTS: u16 = 6;
const STAT_MAXHP: u16 = 7;

/// The sync's messages to `client` among a tick's (ids 0x18, 0x95, 0x96).
fn sync_msgs(sent: &[(ClientId, Vec<u8>)], client: ClientId) -> Vec<Vec<u8>> {
    sent.iter()
        .filter(|(c, m)| *c == client && matches!(m[0], 0x18 | 0x95 | 0x96))
        .map(|(_, m)| m.clone())
        .collect()
}

/// With the sync on, the first tick ends the player's client batch with
/// 0x95 (cache all zero, Δlife ≥ 10 %) at the path record's position;
/// the second player (no max life: M ≤ 0) gets nothing. While running,
/// the unforced runs wait (|Δlife| < 10 %); the forced run at the 20th
/// counted update sends 0x96 with the drained stamina, the path's cell
/// and its offset to the target, and the counter restarts. Bytes stated
/// with `d2-proto`'s typed builders (`server-messages.tsv` `bits:`).
#[test]
fn vitals_sync_on_the_host_tick() {
    use d2_proto::server::{LifeManaUpdate2, WalkVerify};
    use d2_proto::FixedMessage;
    let (mut fx, p, _, _) = two_players();
    fx.sim.events.with(&mut fx.sim.game, |_, v| {
        v.set_base(p, STAT_HITPOINTS, 40 << 8);
        v.set_base(p, STAT_MAXHP, 50 << 8);
    });
    assert!(sync_msgs(&fx.tick(), 0).is_empty(), "off by default");
    fx.sim.events.hooks().enable_vitals_sync();
    let sent = fx.tick();
    let d = fx.path(p);
    let want = LifeManaUpdate2 {
        life: 40,
        mana: 0,
        stamina: 100,
        x: d.x() as u16,
        y: d.y() as u16,
        dx: (d.x() as u16).wrapping_sub(d.target_x) as u8,
        dy: (d.y() as u16).wrapping_sub(d.target_y) as u8,
    };
    // The sync's own message, then the stat messages of what is nonzero
    // (the maxima: `vitals_sync::stat_changes`).
    assert_eq!(sync_msgs(&sent, 0), [want.encode().to_vec()]);
    assert!(sent
        .iter()
        .skip_while(|m| m.1[0] != 0x95)
        .skip(1)
        .all(|m| matches!(m.1[0], 0x1D..=0x1F)));
    assert!(sync_msgs(&sent, 1).is_empty());
    assert_eq!(
        fx.sim
            .game
            .lists
            .client(fx.sim.sim_client(0).unwrap())
            .unwrap()
            .update_count,
        0
    );

    assert_eq!(fx.handle(0, &point(0x03, 31, 10)).0, ResultCode::Done);
    let mut found = None;
    for k in 1..=25 {
        let sent = fx.tick();
        let msgs = sync_msgs(&sent, 0);
        if !msgs.is_empty() {
            found = Some((k, msgs));
            break;
        }
    }
    let (k, msgs) = found.expect("a forced sync within 20 updates");
    assert_eq!(k, 20, "forced at +0x1B0 = 20");
    let got = WalkVerify::decode(&msgs[0]).unwrap();
    let d = fx.path(p);
    assert_eq!(u32::from(got.stamina), (fx.stamina(p) >> 8) as u32);
    assert!(got.stamina < 100);
    assert_eq!((got.x, got.y), (d.x() as u16, d.y() as u16));
    assert_eq!(got.dx, (d.x() as u16).wrapping_sub(d.target_x) as u8);
    assert_eq!(
        fx.sim
            .game
            .lists
            .client(fx.sim.sim_client(0).unwrap())
            .unwrap()
            .update_count,
        0
    );
    fx.assert_clean();
}

impl Fx {
    /// Everything a walk request could touch, as text: the game, units,
    /// stats (the interactions on the unit records), the path state, the
    /// player fields, the stubs and the errors.
    fn digest(&self, p: UnitId) -> String {
        let s = &self.sim.events.sys;
        format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}{:?}|{:?}{:?}",
            self.sim.game,
            s.units,
            s.stats,
            s.hooks.paths,
            self.sim.player_fields(p),
            self.sim.unhandled,
            s.hooks.errors,
            s.errors,
        )
    }
}

/// The refusals that come before the mode write (the target lookup and
/// the mode check) change nothing and queue nothing, though they return
/// 0.
#[test]
fn early_refusals_change_nothing() {
    let (mut fx, p, _, _) = two_players();
    for m in [unit_msg(0x02, 2, 999), unit_msg(0x04, 1, 999)] {
        let before = fx.digest(p);
        assert_eq!(fx.handle(0, &m), (ResultCode::Done, vec![]), "{m:02X?}");
        assert_eq!(fx.digest(p), before, "§1.2 rule 1: {m:02X?}");
    }
    fx.sim.events.sys.units.get_mut(p).unwrap().mode = 0;
    for m in [point(0x01, 31, 10), point(0x03, 31, 10)] {
        let before = fx.digest(p);
        assert_eq!(fx.handle(0, &m), (ResultCode::Done, vec![]), "{m:02X?}");
        assert_eq!(fx.digest(p), before, "§1.3 mode DT: {m:02X?}");
    }
    assert!(fx.sim.events.sys.hooks.x.sent.is_empty());
}

/// C→S 0x5F runs the position resync on the path provider (`RESYNC_ID`):
/// a point less than 5 away is ignored (0, nothing moves); without the
/// provider the id stays a stub (recorded, 0).
#[test]
fn update_player_pos_routes_to_the_resync() {
    let mut fx = Fx::new();
    let a = fx.a;
    let p = fx.player(0, a, 26, 10);
    let before = fx.path(p);
    assert_eq!(
        fx.handle(0, &point(super::RESYNC_ID, 27, 11)),
        (ResultCode::Done, vec![])
    );
    assert_eq!(fx.mode(p), 1);
    assert_eq!(fx.path(p).precise_x, before.precise_x);
    fx.assert_clean();
    fx.sim.events.hooks().paths = None;
    assert_eq!(
        fx.handle(0, &point(super::RESYNC_ID, 27, 11)),
        (ResultCode::Done, vec![])
    );
    let ids: Vec<u8> = fx.sim.unhandled.iter().map(|u| u.1).collect();
    assert_eq!(ids, [super::RESYNC_ID]);
}

/// The play client sends a new C→S 0x03 every frame while the run is
/// held, each a point a little ahead. In the town (no drain) every
/// request finds the player already in mode 3: the mode set frees
/// nothing, so the run list must not be attached a second time
/// (`wiring::path::walk` `attach_run_stats`): the velocity stays 0x900 and the player
/// advances +0x9000 a tick (a walk-speed 0x600 would be a third slower
/// than the client's run prediction).
#[test]
fn a_run_requested_every_tick_in_the_town_keeps_the_run_velocity() {
    let mut fx = Fx::new();
    let c = fx.c;
    let rect = fx.sim.events.hooks().drlg.subtile_rect(c).unwrap();
    let (x, y) = (rect.x + 2, rect.y + 4);
    let p = fx.player(0, c, x, y);
    let start = fx.path(p).precise_x;
    for k in 1..=4u32 {
        let cell = fx.path(p).cell();
        assert_eq!(
            fx.handle(0, &point(0x03, (cell.x + 3) as u16, y as u16)),
            (ResultCode::Done, vec![])
        );
        assert_eq!(fx.mode(p), 3, "tick {k}");
        assert_eq!(fx.path(p).velocity, 0x900, "tick {k}");
        fx.tick();
        assert_eq!(fx.path(p).precise_x, start + k * 0x9000, "tick {k}");
    }
    let v = fx
        .sim
        .events
        .with(&mut fx.sim.game, |_, v| v.stat(p, STAT_VELOCITY));
    assert_eq!(v, 150, "one run list (+50)");
    // Stamina untouched in the town.
    assert_eq!(fx.stamina(p), 0x6400);
    fx.assert_clean();
}

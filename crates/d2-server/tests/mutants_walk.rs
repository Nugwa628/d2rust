//! Mutation tests of the walk / run routing on the wired single-player host: C→S
//! 0x01 through `SimGame::handle` on `SimGame<ActionSim<_>, WiredWorld<_>>` reaches
//! the action wiring's path provider (`WiredWorld`'s `WorldHost::walk` delegates to
//! its `ActionWorld`), so the player walks exactly as on the action host (the same
//! 13-tick walk as `adapters/handlers/walk/tests.rs`) and the other client gets 0x0F
//! in the update pass. Synthetic tables and DRLG (two 8×8-tile rooms of Cold Plains,
//! every cell a floor); the NPC, vendor and quest seams of the wired host are no-op
//! rests no walk reaches.
//!
//! Integration test of `d2-server`; it shares the `mutants_handlers_fx` fixture and
//! guards the routing from the dispatcher to the path provider.

mod mutants_handlers_fx;

use std::collections::BTreeMap;
use std::sync::Arc;

use d2_data::bin::BinTable;
use d2_data::fixup::records::stat_ops;
use d2_data::tables::{Charstats, Itemstatcost, Record};
use d2_proto::server::PlayerMove;
use d2_server::adapters::handlers::walk::enable_paths;
use d2_server::adapters::handlers::world::{ActionWorld, Outbox, WiredWorld};
use d2_server::adapters::{PlayerData, PlayerFields, SimGame};
use d2_server::buffers::QueueError;
use d2_server::seams::{ClientId, Intents, MessageSink, PlayerGate, ResultCode, Tick};
use d2_sim::combat::CombatTables;
use d2_sim::drlg::room::LinkAt;
use d2_sim::drlg::{
    CellGrid, Drlg, DrlgData, DrlgError, DrlgRoomId, Dungeon, GridPass, LevelDef, LevelIdx,
    LevelTypes, RoomGrids, RoomKind, TileInfo, TileRect, TileSource,
};
use d2_sim::game::Game;
use d2_sim::items::ItemTables;
use d2_sim::rng::Seed;
use d2_sim::skills::SkillTables;
use d2_sim::stats::{StatData, StatTable};
use d2_sim::units::hooks::UnitData;
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::{RoomId, UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionSim, ActionTables, DrlgWorld, Pending};
use d2_sim::world::npc::NpcControl;
use d2_sim::world::quests::{QuestControl, QuestTables};
use d2_sim::world::vendors::VendorTables;
use mutants_handlers_fx::Rest;

// ---- fixture -----------------------------------------------------------------------------

/// The action wiring's seams: `Pending`'s defaults; sends kept.
#[derive(Default)]
struct ActionRest {
    sent: Vec<(UnitId, Vec<u8>)>,
}

impl Pending for ActionRest {
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.sent.push((player, msg.to_vec()));
    }
}

impl Outbox for ActionRest {
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

const COLD_PLAINS: u32 = 3;
/// The players' class: sorceress.
const CLASS: u32 = 1;
/// `velocitypercent`, a player's walk speed in percent (100 at creation).
const STAT_VELOCITY: u16 = 67;

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
    data.levels[COLD_PLAINS as usize].drlg_type = 2;
    data.levels[COLD_PLAINS as usize].level_type = 1;
    let mut types = Types(BTreeMap::from([(
        COLD_PLAINS,
        vec![TileRect::new(0, 0, 8, 8), TileRect::new(8, 0, 8, 8)],
    )]));
    let mut dungeon = Dungeon::default();
    dungeon.acts[0] = Some(Drlg::create(0, 1, 0, 0, false, &data, &mut types).unwrap());
    DrlgWorld {
        dungeon,
        data: Arc::new(data),
        tiles: Box::new(tiles()),
        types: Box::new(types),
    }
}

/// Charstats `WalkVelocity` 6 for [`CLASS`] (vector V1: velocity 0x600).
fn tables() -> ActionTables {
    let mut charstats = vec![blank::<Charstats>(); 7];
    charstats[CLASS as usize].walkvelocity = 6;
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

type Sim = SimGame<ActionSim<ActionRest>, WiredWorld<Rest>>;

const ALIVE: PlayerGate = PlayerGate {
    mode: 1,
    uninterruptable: false,
};

struct Fx {
    sim: Sim,
    /// Cold Plains' first active room.
    a: RoomId,
    out: Sink,
}

impl Fx {
    /// The wired host; the path provider on (before any allocation) when
    /// `paths`; every room of Cold Plains streamed.
    fn new(paths: bool) -> Self {
        let hooks = ActionHooks::new(
            Arc::new(tables()),
            drlg(),
            Seed::init_low(1234),
            ActionRest::default(),
        );
        let mut events = ActionSim::new(stat_data(), UnitData::default(), hooks);
        if paths {
            enable_paths(&mut events).expect("embedded path tables");
        }
        let mut game = Game::new();
        game.lists.ensure_act(0).unwrap();
        let active = events
            .hooks()
            .drlg
            .with_act(0, &mut game.lists, |d, svc| {
                let l = d.get_or_alloc_level(svc.data, svc.types, COLD_PLAINS)?;
                d.generate_level(svc.data, svc.types, l)?;
                let mut out = Vec::new();
                for r in d.level_rooms(l) {
                    out.push(d.stream_room(svc, r)?.expect("active"));
                }
                Ok::<_, DrlgError>(out)
            })
            .unwrap()
            .unwrap();
        let mut seed = Seed::init_low(0x5EED);
        let npc = NpcControl::new(&[], Vec::new(), true, 0, &mut seed).unwrap();
        let quests = QuestControl::new(&QuestTables::load().unwrap(), &mut seed).unwrap();
        let world = WiredWorld::new(
            ActionWorld::default(),
            ItemTables::default(),
            quests,
            npc,
            VendorTables::default(),
            Rest::default(),
            1000,
        );
        Self {
            sim: SimGame::with_world(game, events, world),
            a: active[0],
            out: Sink::default(),
        }
    }

    /// A neutral player (velocity percent 100) for `client` in room A at
    /// (x, y).
    fn player(&mut self, client: ClientId, x: i32, y: i32) -> UnitId {
        let req = AllocRequest {
            ty: UnitType::Player,
            class: CLASS,
            room: Some(self.a),
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: true,
        };
        let p = self
            .sim
            .events
            .with(&mut self.sim.game, |g, v| v.allocate(g, &req, x, y))
            .expect("allocated");
        self.sim.events.sys.units.get_mut(p).unwrap().mode = 1;
        // Staged as already announced: the seed-set flag is cleared.
        self.sim.events.sys.units.get_mut(p).unwrap().flags &=
            !d2_sim::units::record::flags::SEED_SET;
        self.sim.events.with(&mut self.sim.game, |_, v| {
            v.set_base(p, STAT_VELOCITY, 100);
        });
        let a = self.a;
        self.sim
            .join(client, Some(p), Some(a), client_state::IN_GAME)
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

    fn mode(&self, u: UnitId) -> u32 {
        self.sim.events.sys.units.get(u).unwrap().mode
    }

    fn precise(&mut self, u: UnitId) -> Option<(u32, u32)> {
        let d = self.sim.events.hooks().paths.as_ref()?.dynamic(u)?;
        Some((d.precise_x, d.precise_y))
    }

    fn handle(&mut self, client: ClientId, msg: &[u8]) -> (ResultCode, Vec<(ClientId, Vec<u8>)>) {
        let r = self.sim.handle(client, msg, msg.len(), &mut self.out);
        (r, std::mem::take(&mut self.out.0))
    }

    fn tick(&mut self) -> Vec<(ClientId, Vec<u8>)> {
        self.sim.tick(&mut self.out);
        std::mem::take(&mut self.out.0)
    }
}

fn point(id: u8, x: u16, y: u16) -> Vec<u8> {
    let mut m = vec![id];
    m.extend_from_slice(&x.to_le_bytes());
    m.extend_from_slice(&y.to_le_bytes());
    m
}

/// Precise coordinate (16.16 fixed point) of sub-tile `x`'s centre.
fn centre(x: u32) -> u32 {
    (x << 16) | 0x8000
}

// ---- tests -------------------------------------------------------------------------------

// Checks the walk request is routed to the path provider, moves the player and
// reaches the other client.
#[test]
fn the_wired_host_routes_walk_to_the_path_provider() {
    // Kills `WiredWorld::walk → None` (the id falls to the stub: recorded,
    // nothing moves) and `→ Some(Default)` (result 0 without the request:
    // nothing moves, nothing sent).
    let mut fx = Fx::new(true);
    let p = fx.player(0, 26, 10);
    let _q = fx.player(1, 26, 14);
    let guid = fx.sim.game.lists.unit(p).unwrap().guid;
    // A walk request gets no reply and result 0.
    assert_eq!(
        fx.handle(0, &point(0x01, 31, 10)),
        (ResultCode::Done, vec![])
    );
    assert!(fx.sim.unhandled.is_empty(), "{:?}", fx.sim.unhandled);
    assert_eq!(fx.mode(p), 2);
    // +0x6000 per tick for 13 ticks, the 14th on the target centre, neutral.
    let mut sent = Vec::new();
    for k in 0..13u32 {
        sent.push(fx.tick());
        assert_eq!(
            fx.precise(p),
            Some((0x1AE000 + k * 0x6000, centre(10))),
            "tick {}",
            k + 1
        );
        assert_eq!(fx.mode(p), 2);
    }
    sent.push(fx.tick());
    assert_eq!(fx.precise(p), Some((centre(31), centre(10))));
    assert_eq!(fx.mode(p), 1);
    // In tick 1's update pass client 1 gets 0x0F.
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
    assert_eq!(sent[0], vec![(1, want.encode().to_vec())]);
    // The 14th tick's neutral mode (NU): 0x0D code 7 at the stop cell to
    // client 1 only.
    let stop = d2_sim::path::walk::messages::player_stop(0, guid, 7, 31, 10, 0, 0);
    assert!(sent[1..13].iter().all(Vec::is_empty), "{sent:?}");
    assert_eq!(sent[13], vec![(1, stop.to_vec())]);
    assert!(fx.sim.world.action.faults.is_empty());
    assert!(fx.sim.events.hooks().errors.is_empty());
}

#[test]
fn without_the_path_provider_the_wired_host_keeps_the_stub() {
    // Mutation guard for the test above: the same host with the provider off leaves
    // 0x01 to the stub (recorded, result 0), so the test above fails when
    // the routing does.
    let mut fx = Fx::new(false);
    let p = fx.player(0, 26, 10);
    assert_eq!(
        fx.handle(0, &point(0x01, 31, 10)),
        (ResultCode::Done, vec![])
    );
    assert_eq!(fx.mode(p), 1);
    let ids: Vec<u8> = fx.sim.unhandled.iter().map(|u| u.1).collect();
    assert_eq!(ids, [0x01]);
}

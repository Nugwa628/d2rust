//! Mutation-testing gaps of the world hosts on the merged host: C→S 0x49 on the
//! wired host (`WiredWorld`'s `HostWaypoints`: the interaction at the host's one
//! owner, the difficulty from the game's home) and on `ActionWorld`, the faults
//! both record, and a sale to the right NPC's vendor record reaching the player's
//! own client. Every message goes through `SimGame::handle`; the assertions are
//! the game's outcomes (result code, interaction, warp and arrival, the S→C
//! bytes, gold).
//!
//! Integration test of `d2-server`; it shares the `mutants_handlers_fx` fixture
//! and guards the waypoint and vendor wiring of the server's world hosts.

mod mutants_handlers_fx;

use std::collections::BTreeMap;
use std::sync::Arc;

use d2_data::tables::{Levels, Objects};
use d2_server::adapters::handlers::world::{ActionWorld, WiredWorld, WorldError};
use d2_server::adapters::{PlayerData, PlayerFields, SimGame};
use d2_server::buffers::QueueError;
use d2_server::seams::{Intents, MessageSink, ResultCode};
use d2_sim::drlg::room::LinkAt;
use d2_sim::drlg::{
    CellGrid, Drlg, DrlgData, DrlgError, DrlgRoomId, Dungeon, GridPass, LevelDef, LevelIdx,
    LevelTypes, RoomGrids, RoomKind as DrlgRoomKind, TileInfo, TileRect, TileSource,
};
use d2_sim::game::Game;
use d2_sim::rng::Seed;
use d2_sim::units::hooks::UnitData;
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::{RoomId, UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionSim, DrlgWorld};
use d2_sim::world::npc::{transaction, NpcControl};
use d2_sim::world::quests::{QuestControl, QuestTables};
use d2_sim::world::vendors::VendorTables;
use d2_sim::world::waypoints::{WaypointData, NO_WAYPOINT};
use mutants_handlers_fx::{
    blank, cube_item_tables, empty_action_tables, handle, stat_data, ActionRest, Rest, TradeFx,
    ALIVE,
};

use ResultCode::*;

// ---- the waypoint world ------------------------------------------------------------------

struct Types(BTreeMap<u32, TileRect>);

impl LevelTypes for Types {
    fn generate(
        &mut self,
        drlg: &mut Drlg,
        _: &DrlgData,
        level: LevelIdx,
    ) -> Result<(), DrlgError> {
        let id = drlg.level(level).id;
        if let Some(&rect) = self.0.get(&id) {
            let r = drlg.alloc_room(level, DrlgRoomKind::Preset, rect);
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
        let r = drlg.room(room).rect;
        let (w, h) = (r.w as usize + 1, r.h as usize + 1);
        let mut g = CellGrid::new(w, h);
        for y in 0..h {
            for x in 0..w {
                g.set(x, y, d2_sim::drlg::tiles::cell::FLOOR);
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

fn tile(orientation: u32, main: u32, sub: u32, rarity: u32) -> TileInfo {
    TileInfo {
        orientation,
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

/// Cold Plains (act 0, waypoint index 1), level 4 (act 0, index 2) and
/// Lut Gholein (act 1, index 9).
const COLD_PLAINS: u32 = 3;
const LEVEL4: u32 = 4;
const ACT2_TOWN: u32 = 40;

fn drlg() -> DrlgWorld {
    use d2_sim::drlg::tiles::FIXED_LIBRARY;
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
    for id in [COLD_PLAINS, LEVEL4, ACT2_TOWN] {
        data.levels[id as usize].drlg_type = 2;
        data.levels[id as usize].level_type = 1;
    }
    let mut types = Types(
        [
            (COLD_PLAINS, TileRect::new(0, 0, 8, 8)),
            (LEVEL4, TileRect::new(8, 0, 8, 8)),
            (ACT2_TOWN, TileRect::new(0, 0, 8, 8)),
        ]
        .into(),
    );
    let mut dungeon = Dungeon::default();
    dungeon.acts[0] = Some(Drlg::create(0, 1, 0, 0, false, &data, &mut types).unwrap());
    dungeon.acts[1] = Some(Drlg::create(1, 2, 0, 0, false, &data, &mut types).unwrap());
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
    DrlgWorld {
        dungeon,
        data: Arc::new(data),
        tiles: Box::new(Tiles(t)),
        types: Box::new(types),
    }
}

/// `levels` rows (count 150) and one waypoint object class 0.
fn waypoint_data() -> WaypointData {
    let mut levels = vec![blank::<Levels>(); 150];
    for (i, l) in levels.iter_mut().enumerate() {
        l.waypoint = NO_WAYPOINT;
        l.act = if i >= 40 { 1 } else { 0 };
    }
    levels[1].waypoint = 0;
    levels[COLD_PLAINS as usize].waypoint = 1;
    levels[LEVEL4 as usize].waypoint = 2;
    levels[ACT2_TOWN as usize].waypoint = 9;
    let mut o: Objects = blank();
    o.operatefn = 23;
    o.initfn = 17;
    o.framecnt1 = 15 << 8;
    WaypointData::new(&levels, &[o])
}

/// The action sim and game: a waypoint at (20, 20) in Cold Plains,
/// another in Lut Gholein, a third at (20, 20) in level 4, a player of
/// `class` at (20 + dx, 20) in Cold Plains who knows index 1 on Normal.
/// Returns (events, game, player, its room, waypoint GUIDs).
fn world(class: u32, dx: i32) -> (ActionSim<ActionRest>, Game, UnitId, RoomId, Wps) {
    let hooks = ActionHooks::new(
        Arc::new(empty_action_tables()),
        drlg(),
        Seed::init_low(1234),
        ActionRest::default(),
    );
    let mut sim = ActionSim::new(stat_data(), UnitData::default(), hooks);
    let mut game = Game::new();
    let mut rooms = Vec::new();
    for (act, level) in [(0u8, COLD_PLAINS), (1, ACT2_TOWN), (0, LEVEL4)] {
        game.lists.ensure_act(act).unwrap();
        let r = sim
            .hooks()
            .drlg
            .with_act(act, &mut game.lists, |d, svc| {
                let l = d.get_or_alloc_level(svc.data, svc.types, level)?;
                d.generate_level(svc.data, svc.types, l)?;
                let r = d.level_rooms(l)[0];
                d.stream_room(svc, r)
            })
            .unwrap()
            .unwrap()
            .unwrap();
        rooms.push(r);
    }
    let mut spawn = |ty, class, room, x| {
        let req = AllocRequest {
            ty,
            class,
            room: Some(room),
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: ty == UnitType::Player,
        };
        sim.with(&mut game, |g, v| v.allocate(g, &req, x, 20))
            .unwrap()
    };
    let o = spawn(UnitType::Object, 0, rooms[0], 20);
    let far = spawn(UnitType::Object, 0, rooms[1], 20);
    let player = spawn(UnitType::Player, class, rooms[0], 20 + dx);
    // After the player: the other units' GUIDs stay as before.
    let side = spawn(UnitType::Object, 0, rooms[2], 20);
    sim.sys.units.get_mut(player).unwrap().mode = 1;
    sim.hooks()
        .waypoints
        .entry(player)
        .or_default()
        .get_mut(0)
        .set(1)
        .unwrap();
    let wp = game.lists.unit(o).unwrap().guid;
    let far_wp = game.lists.unit(far).unwrap().guid;
    let side_wp = game.lists.unit(side).unwrap().guid;
    let wps = Wps {
        wp,
        far_wp,
        side_wp,
    };
    (sim, game, player, rooms[0], wps)
}

/// The waypoint GUIDs of [`world`]: Cold Plains, Lut Gholein, level 4.
struct Wps {
    wp: u32,
    far_wp: u32,
    side_wp: u32,
}

type Wired = SimGame<ActionSim<ActionRest>, WiredWorld<Rest>>;

/// The wired host on [`world`], the player on `client`.
struct Fx {
    sim: Wired,
    player: UnitId,
    wp: u32,
    far_wp: u32,
    /// Level 4's waypoint: travel to Cold Plains from it is not to its
    /// own level.
    side_wp: u32,
}

impl Fx {
    fn new(class: u32, dx: i32, client: u32) -> Self {
        let (mut events, game, player, room, wps) = world(class, dx);
        let Wps {
            wp,
            far_wp,
            side_wp,
        } = wps;
        let mut seed = events.hooks().game_seed;
        let npc = NpcControl::new(&[], Vec::new(), true, 0, &mut seed).unwrap();
        let quests = QuestControl::new(&QuestTables::load().unwrap(), &mut seed).unwrap();
        events.hooks().game_seed = seed;
        let action = ActionWorld {
            waypoints: Some(waypoint_data()),
            ..ActionWorld::default()
        };
        let world = WiredWorld::new(
            action,
            cube_item_tables(),
            quests,
            npc,
            VendorTables::default(),
            Rest::default(),
            1000,
        );
        let mut sim: Wired = SimGame::with_world(game, events, world);
        sim.join(client, Some(player), Some(room), client_state::IN_GAME)
            .unwrap();
        sim.set_player(
            player,
            PlayerFields {
                gate: ALIVE,
                data: Some(PlayerData { last_accept: 0 }),
            },
        );
        Fx {
            sim,
            player,
            wp,
            far_wp,
            side_wp,
        }
    }

    /// The player's interaction, at its one owner (the unit record).
    fn interact(&mut self) -> Option<(u8, u32)> {
        let units = &self.sim.events.sys.units;
        units.get(self.player).unwrap().interact.get()
    }

    fn set_interact(&mut self, i: (u8, u32)) {
        let p = self.player;
        let r = self.sim.events.sys.units.get_mut(p).unwrap();
        r.interact.reset();
        r.interact.set(i.0, i.1);
    }

    fn log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.sim.events.hooks().x.log)
    }
}

fn msg(wp: u32, level: u32) -> Vec<u8> {
    let mut m = vec![0x49];
    m.extend_from_slice(&wp.to_le_bytes());
    m.extend_from_slice(&(level as u16).to_le_bytes());
    m.extend_from_slice(&[0, 0]);
    m
}

/// On the wired host a sorceress 22 subtiles away travels to Cold Plains: the
/// interaction (at the host's owner) is reset, warp then arrival mode, S→C 0x0D
/// at x + 3, y + 3, one arrival node.
#[test]
fn wired_travel() {
    let mut fx = Fx::new(1, 22, 0);
    // From level 4's waypoint (travel to its own level only closes).
    fx.set_interact((2, fx.side_wp));
    let p = fx.player;
    let guid = fx.sim.game.lists.unit(p).unwrap().guid;
    let (code, got) = handle(&mut fx.sim, 0, &msg(fx.side_wp, COLD_PLAINS));
    assert_eq!(code, Done);
    assert_eq!(
        fx.log(),
        [format!("warp {} 3 0", p.0), format!("arrival mode {}", p.0)]
    );
    let mut want = vec![0x0D, 0x00];
    want.extend_from_slice(&guid.to_le_bytes());
    want.push(1);
    want.extend_from_slice(&(42u16 + 3).to_le_bytes());
    want.extend_from_slice(&(20u16 + 3).to_le_bytes());
    want.extend_from_slice(&[0, 0]);
    assert_eq!(got, vec![want]);
    assert_eq!(fx.interact(), None);
    assert_eq!(fx.sim.events.sys.hooks.arrivals.0.len(), 1);
    assert!(fx.sim.world.action.faults.is_empty());
}

/// Level 0 closes the menu (the interaction is reset), nothing else.
#[test]
fn wired_close() {
    let mut fx = Fx::new(0, 10, 0);
    fx.set_interact((2, fx.wp));
    let (code, got) = handle(&mut fx.sim, 0, &msg(fx.wp, 0));
    assert_eq!((code, got), (Done, vec![]));
    assert_eq!(fx.interact(), None);
    assert!(fx.log().is_empty());
}

/// A refusal resets the interaction only when its unit is the waypoint.
#[test]
fn wired_refusal_closes_only_its_own_menu() {
    let mut fx = Fx::new(0, 11, 0);
    fx.set_interact((1, 77));
    assert_eq!(handle(&mut fx.sim, 0, &msg(fx.wp, 0)).0, Refused);
    assert_eq!(fx.interact(), Some((1, 77)));
    let mut fx = Fx::new(0, 11, 0);
    fx.set_interact((2, fx.wp));
    assert_eq!(handle(&mut fx.sim, 0, &msg(fx.wp, 0)).0, Refused);
    assert_eq!(fx.interact(), None);
}

/// The waypoint record of the game's difficulty decides (the player knows index 1
/// on Normal only).
#[test]
fn wired_record_of_the_game_difficulty() {
    let mut fx = Fx::new(1, 0, 0);
    fx.sim.events.hooks().ai_info.difficulty = 1;
    let (code, got) = handle(&mut fx.sim, 0, &msg(fx.wp, COLD_PLAINS));
    assert_eq!((code, got), (Invalid, vec![]));
    assert!(fx.log().is_empty());
}

/// Within the hostile delay: sound 0x13, the interaction reset, result 1.
#[test]
fn wired_hostile_delay() {
    let mut fx = Fx::new(1, 0, 0);
    fx.set_interact((2, fx.wp));
    fx.sim.events.hooks().x.hostile = true;
    let (code, got) = handle(&mut fx.sim, 0, &msg(fx.wp, COLD_PLAINS));
    assert_eq!((code, got), (Refused, vec![]));
    assert_eq!(fx.log(), [format!("sound {} 19", fx.player.0)]);
    assert_eq!(fx.interact(), None);
}

/// A missing object → 1; an object in another act → 2.
#[test]
fn wired_missing_and_other_act() {
    let mut fx = Fx::new(0, 0, 0);
    assert_eq!(handle(&mut fx.sim, 0, &msg(0xDEAD, 0)).0, Refused);
    let far = fx.far_wp;
    assert_eq!(handle(&mut fx.sim, 0, &msg(far, 0)).0, Invalid);
}

/// A sink that refuses every message (a queueing failure).
struct Refusing;

impl MessageSink for Refusing {
    fn queue(&mut self, _: u32, msg: &[u8]) -> Result<(), QueueError> {
        Err(QueueError::TooLarge(msg.len()))
    }
}

/// The handlers' fault path (`WorldError`): the arrival message fails to
/// queue; the host records the fault and the handler returns 3 (both
/// hosts).
#[test]
fn unqueued_message_is_a_recorded_fault() {
    let mut fx = Fx::new(1, 22, 0);
    // From level 4's waypoint, so the travel sends.
    let m = msg(fx.side_wp, COLD_PLAINS);
    let code = Intents::handle(&mut fx.sim, 0, &m, m.len(), &mut Refusing);
    assert_eq!(code, Malformed);
    let faults = &fx.sim.world.action.faults;
    assert_eq!(faults.len(), 1);
    assert!(matches!(faults[0].error, WorldError::Sink(_)));

    let (events, game, player, room, wps) = world(1, 22);
    let action: ActionWorld = ActionWorld {
        waypoints: Some(waypoint_data()),
        ..ActionWorld::default()
    };
    let mut sim = SimGame::with_world(game, events, action);
    sim.join(0, Some(player), Some(room), client_state::IN_GAME)
        .unwrap();
    sim.set_player(
        player,
        PlayerFields {
            gate: ALIVE,
            data: Some(PlayerData { last_accept: 0 }),
        },
    );
    let m = msg(wps.side_wp, COLD_PLAINS);
    let code = Intents::handle(&mut sim, 0, &m, m.len(), &mut Refusing);
    assert_eq!(code, Malformed);
    assert_eq!(sim.world.faults.len(), 1);
    assert!(matches!(sim.world.faults[0].error, WorldError::Sink(_)));
}

// ---- vendors ------------------------------------------------------------------------------

fn npc_msg(id: u8, npc: u32, tail: &[u8]) -> Vec<u8> {
    let mut m = vec![id, 1, 0, 0, 0];
    m.extend_from_slice(&npc.to_le_bytes());
    m.extend_from_slice(tail);
    m
}

/// A sale on the wired host, the player on client 2: Akara's trade opened (0x13,
/// 0x2F, 0x38), then 0x33 of the player's cap. The cap is one of Akara's permanent
/// codes (her record, not Gheed's): no copy is kept, the cap leaves with S→C 0x9D
/// action 5, the price is paid and S→C 0x2A code 1 with the cap's GUID and the
/// new gold reaches client 2.
#[test]
fn sale_uses_the_npcs_own_record_and_client() {
    let mut fx = TradeFx::new(2);
    let ng = fx.guid(fx.akara);
    for m in [
        npc_msg(0x13, ng, &[]),
        npc_msg(0x2F, ng, &[]),
        npc_msg(0x38, ng, &[0; 4]),
    ] {
        assert_eq!(handle(&mut fx.sim, 2, &m).0, Done, "{m:02X?}");
    }
    let gold = fx.gold();
    let cap = fx.cap;
    let eg = fx.guid(cap);
    let mut sell = vec![0x33];
    sell.extend_from_slice(&ng.to_le_bytes());
    sell.extend_from_slice(&eg.to_le_bytes());
    sell.extend_from_slice(&[0; 8]);
    let (code, got) = handle(&mut fx.sim, 2, &sell);
    assert_eq!(code, Done);
    let now = fx.gold();
    assert!(now > gold, "{now} {gold}");
    // A stored item (mode 0): S→C 0x9D action 5 (command flags 0x20, the
    // stored page shown) before the 0x2A. Both reach the client in one send.
    assert_eq!(got.len(), 1, "{got:02X?}");
    let m = &got[0];
    let size = usize::from(m[2]);
    assert_eq!((m[0], m[1]), (0x9D, 0x05));
    assert_eq!(m[4..8], eg.to_le_bytes());
    assert_eq!(m[size..], transaction(3, 1, eg, now as u32));
    let inv = fx.sim.world.inventory.as_ref().unwrap();
    assert!(!inv.state.items_of(fx.player).contains(&cap));
    let _ = fx.gheed;
}

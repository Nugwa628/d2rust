//! C→S 0x49 on the wired sim: `ActionSim` (unit records, DRLG levels and
//! rooms, the `WaypointView` adapter) with a two-act synthetic DRLG.
//! Only `Pending` (path positions, interaction, warp, sends) is faked.
//!
//! Part of the world handler's test suite; `objects` reuses its fixtures
//! (`TestPending`, `field_drlg`, `field_room`, `blank`).

use std::collections::BTreeMap;
use std::sync::Arc;

use d2_data::tables::{Levels, Objects, Record};
use d2_sim::combat::CombatTables;
use d2_sim::drlg::room::LinkAt;
use d2_sim::drlg::{
    CellGrid, Drlg, DrlgData, DrlgError, DrlgRoomId, Dungeon, GridPass, LevelDef, LevelIdx,
    LevelTypes, RoomGrids, RoomKind, TileInfo, TileRect, TileSource,
};
use d2_sim::game::Game;
use d2_sim::rng::Seed;
use d2_sim::skills::SkillTables;
use d2_sim::stats::StatData;
use d2_sim::units::hooks::UnitData;
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::{RoomId, UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionSim, ActionTables, DrlgWorld, Pending};
use d2_sim::world::waypoints::{ArrivalNode, WaypointData, NO_WAYPOINT};

use super::*;
use crate::adapters::handlers::world::{ActionWorld, Outbox};
use crate::adapters::{PlayerData, PlayerFields, SimGame};
use crate::seams::PlayerGate;

/// The seams with no provider: positions, interact info, warp and
/// arrival mode (logged), and the transport.
#[derive(Default)]
pub struct TestPending {
    /// Staged unit positions.
    pub pos: BTreeMap<UnitId, (i32, i32)>,
    /// Messages sent, by recipient.
    pub sent: Vec<(UnitId, Vec<u8>)>,
    /// Log of the calls the sim made.
    pub log: Vec<String>,
    /// Operators are in interact range of objects ([`super::objects`]).
    pub in_range: bool,
}

impl Pending for TestPending {
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.pos.get(&unit).copied().unwrap_or_default()
    }
    fn place(&mut self, unit: UnitId, x: i32, y: i32) {
        self.pos.insert(unit, (x, y));
    }
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.sent.push((player, msg.to_vec()));
    }
    fn warp(&mut self, _: &mut Game, player: UnitId, level: u32, tile_code: u8) {
        self.log
            .push(format!("warp {} {level} {tile_code}", player.0));
    }
    fn set_player_mode_arrival(&mut self, _: &mut Game, player: UnitId) {
        self.log.push(format!("arrival mode {}", player.0));
    }
    fn object_in_range(&self, _: &Game, _: UnitId, _: UnitId) -> bool {
        self.in_range
    }
    fn object_route(&mut self, _: &mut Game, route: d2_sim::wiring::action::ObjectRoute) {
        self.log.push(format!("object route {route:?}"));
    }
}

impl Outbox for TestPending {
    fn take_sent(&mut self) -> Vec<(UnitId, Vec<u8>)> {
        std::mem::take(&mut self.sent)
    }
}

/// One 8×8-tile room per level.
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
    let blank = |sub| {
        let mut x = tile(0, 30, sub, 0);
        x.subtile_flags = [0x20; 25];
        x
    };
    t.insert(FIXED_LIBRARY[0].to_vec(), vec![blank(0), blank(1)]);
    t.insert(FIXED_LIBRARY[1].to_vec(), vec![]);
    t.insert(FIXED_LIBRARY[2].to_vec(), vec![tile(10, 0, 0, 0)]);
    Tiles(t)
}

pub(crate) fn blank<T: Record>() -> T {
    T::decode(&vec![0u8; T::SIZE])
}

/// A DRLG world for act 0 whose Cold Plains (a field level) has one
/// 8×8-tile room (for the handler tests of other modules that need a
/// unit in a real room).
pub(crate) fn field_drlg() -> DrlgWorld {
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
    let mut types = Types(BTreeMap::from([(COLD_PLAINS, TileRect::new(0, 0, 8, 8))]));
    let mut dungeon = Dungeon::default();
    dungeon.acts[0] = Some(Drlg::create(0, 1, 0, 0, false, &data, &mut types).unwrap());
    DrlgWorld {
        dungeon,
        data: Arc::new(data),
        tiles: Box::new(tiles()),
        types: Box::new(types),
    }
}

/// [`field_drlg`]'s Cold Plains room, generated and streamed on `sim`
/// (act 0 is made in `game`'s lists).
pub(crate) fn field_room<X: Pending>(sim: &mut ActionSim<X>, game: &mut Game) -> RoomId {
    game.lists.ensure_act(0).unwrap();
    sim.hooks()
        .drlg
        .with_act(0, &mut game.lists, |d, svc| {
            let l = d.get_or_alloc_level(svc.data, svc.types, COLD_PLAINS)?;
            d.generate_level(svc.data, svc.types, l)?;
            let r = d.level_rooms(l)[0];
            d.stream_room(svc, r)
        })
        .unwrap()
        .unwrap()
        .unwrap()
}

/// Cold Plains (act 0, waypoint index 1), Burial Grounds-like level 4
/// (index 2), Blood Moor 2 (no waypoint), Lut Gholein 40 (act 1, index 9).
pub(crate) const COLD_PLAINS: u32 = 3;
const LEVEL4: u32 = 4;
const BLOOD_MOOR: u32 = 2;
const ACT2_TOWN: u32 = 40;

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

type Sim = SimGame<ActionSim<TestPending>, ActionWorld>;

struct Fx {
    host: TestHost<Sim>,
    player: UnitId,
    wp: u32,
    far_wp: u32,
    /// A waypoint in level 4 (act 0) at the same spot as `wp`: travel to
    /// Cold Plains from it is not to its own level.
    side_wp: u32,
}

/// Cold Plains and level 4 (one room each, act 0) and level 40 (one
/// room, act 1). A waypoint object at (20, 20) in Cold Plains, another in
/// level 40, a third at (20, 20) in level 4, and a player of `class` at
/// (20 + dx, 20) in Cold Plains for client 0. The player knows index 1
/// (Cold Plains).
fn fixture(class: u32, dx: i32) -> Fx {
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
    let mut types = Types(BTreeMap::from([
        (COLD_PLAINS, TileRect::new(0, 0, 8, 8)),
        (LEVEL4, TileRect::new(8, 0, 8, 8)),
        (ACT2_TOWN, TileRect::new(0, 0, 8, 8)),
    ]));
    let mut dungeon = Dungeon::default();
    dungeon.acts[0] = Some(Drlg::create(0, 1, 0, 0, false, &data, &mut types).unwrap());
    dungeon.acts[1] = Some(Drlg::create(1, 2, 0, 0, false, &data, &mut types).unwrap());
    let world = DrlgWorld {
        dungeon,
        data: Arc::new(data),
        tiles: Box::new(tiles()),
        types: Box::new(types),
    };
    let tables = ActionTables {
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
            charstats: Vec::new(),
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
    };
    let hooks = ActionHooks::new(
        Arc::new(tables),
        world,
        Seed::init_low(1234),
        TestPending::default(),
    );
    let mut sim = ActionSim::new(Arc::new(StatData::default()), UnitData::default(), hooks);
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
    let spawn = |sim: &mut ActionSim<TestPending>,
                 game: &mut Game,
                 ty: UnitType,
                 class: u32,
                 room: RoomId,
                 x: i32| {
        let req = AllocRequest {
            ty,
            class,
            room: Some(room),
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: ty == UnitType::Player,
        };
        let u = sim.with(game, |g, v| v.allocate(g, &req, x, 20)).unwrap();
        // Staged as already announced: the seed-set flag is cleared.
        sim.sys.units.get_mut(u).unwrap().flags &= !d2_sim::units::record::flags::SEED_SET;
        u
    };
    let o = spawn(&mut sim, &mut game, UnitType::Object, 0, rooms[0], 20);
    let far = spawn(&mut sim, &mut game, UnitType::Object, 0, rooms[1], 20);
    let player = spawn(
        &mut sim,
        &mut game,
        UnitType::Player,
        class,
        rooms[0],
        20 + dx,
    );
    // After the player: the other units' GUIDs stay as before.
    let side = spawn(&mut sim, &mut game, UnitType::Object, 0, rooms[2], 20);
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
    let mut s: Sim = SimGame::with_events(game, sim);
    s.world.waypoints = Some(waypoint_data());
    s.join(0, Some(player), Some(rooms[0]), client_state::IN_GAME)
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
    Fx {
        host: host(s),
        player,
        wp,
        far_wp,
        side_wp,
    }
}

impl Fx {
    fn pending(&mut self) -> &mut TestPending {
        &mut self.host.game.events.hooks().x
    }

    /// The player's interact info (the unit record's), while active.
    fn interact(&self) -> Option<(u8, u32)> {
        let units = &self.host.game.events.sys.units;
        units.get(self.player).unwrap().interact.get()
    }

    /// Stages the player's interact info as (type, GUID), active.
    fn stage(&mut self, ty: u8, guid: u32) {
        let units = &mut self.host.game.events.sys.units;
        let r = units.get_mut(self.player).unwrap();
        r.interact.reset();
        r.interact.set(ty, guid);
    }

    /// The player's interact info is the waypoint (the menu is open).
    fn open_menu(&mut self) {
        let wp = self.wp;
        self.stage(2, wp);
    }

    fn msg(wp: u32, level: u16) -> Vec<u8> {
        let mut m = vec![0x49];
        m.extend_from_slice(&wp.to_le_bytes());
        m.extend_from_slice(&level.to_le_bytes());
        m.extend_from_slice(&[0, 0]);
        m
    }

    fn assert_clean(&mut self) {
        assert!(self.host.game.world.faults.is_empty());
        assert!(self.host.game.unhandled.is_empty());
        assert!(self.host.game.events.hooks().errors.is_empty());
    }
}

// Closing the menu (index 0) resets the interact info and sends nothing.
#[test]
fn close_resets_the_interact_info_without_a_message() {
    let mut fx = fixture(0, 10);
    fx.open_menu();
    let (code, got) = send(&mut fx.host, &Fx::msg(fx.wp, 0));
    assert_eq!(code, ResultCode::Done);
    assert_eq!(got, Vec::<Vec<u8>>::new());
    assert_eq!(fx.interact(), None);
    // Level 0 has no waypoint index: travel stops after the reset.
    assert!(fx.pending().log.is_empty());
    fx.assert_clean();
}

// An out-of-reach waypoint is refused and the menu is closed.
#[test]
fn amazon_out_of_reach_is_refused_and_the_menu_closed() {
    let mut fx = fixture(0, 11);
    fx.open_menu();
    let (code, got) = send(&mut fx.host, &Fx::msg(fx.wp, 0));
    assert_eq!(code, ResultCode::Refused);
    assert!(got.is_empty());
    assert_eq!(fx.interact(), None);
    fx.assert_clean();
}

// A sorceress at level 22 travels and gets the arrival message.
#[test]
fn sorceress_at_22_travels_with_the_arrival_message() {
    let mut fx = fixture(1, 22);
    // From level 4's waypoint: travel to the object's own level would
    // only close the menu.
    let side = fx.side_wp;
    fx.stage(2, side);
    let p = fx.player;
    let guid = fx.host.game.game.lists.unit(p).unwrap().guid;
    let (code, got) = send(&mut fx.host, &Fx::msg(side, COLD_PLAINS as u16));
    assert_eq!(code, ResultCode::Done);
    // The warp is a seam (logged); the player stays in the only room of
    // the level, which is the spawn room: mode 2, then 0x0D at x+3, y+3.
    assert_eq!(
        fx.pending().log,
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
    let room = fx.host.game.game.lists.unit(p).unwrap().room();
    assert_eq!(
        fx.host.game.events.sys.hooks.arrivals.0,
        vec![ArrivalNode { room, x: 42, y: 20 }]
    );
    fx.assert_clean();
}

// Levels with no waypoint, or past the table, are malformed.
#[test]
fn levels_without_a_waypoint_or_past_the_table_are_malformed() {
    let mut fx = fixture(0, 0);
    for level in [BLOOD_MOOR as u16, 0x200] {
        let (code, got) = send(&mut fx.host, &Fx::msg(fx.wp, level));
        assert_eq!(code, ResultCode::Malformed, "level {level}");
        assert!(got.is_empty());
    }
    fx.assert_clean();
}

// An unknown index is refused and closes only the menu of its own waypoint.
#[test]
fn unknown_index_is_refused_and_closes_only_its_own_menu() {
    let mut fx = fixture(0, 0);
    // Interact info on another unit: kept.
    fx.stage(1, 77);
    let (code, _) = send(&mut fx.host, &Fx::msg(fx.wp, LEVEL4 as u16));
    assert_eq!(code, ResultCode::Invalid);
    assert_eq!(fx.interact(), Some((1, 77)));
    fx.assert_clean();
    // Interact info on the waypoint: reset (a fresh client: the
    // transport filters a repeated message).
    let mut fx = fixture(0, 0);
    fx.open_menu();
    let (code, got) = send(&mut fx.host, &Fx::msg(fx.wp, LEVEL4 as u16));
    assert_eq!(code, ResultCode::Invalid);
    assert!(got.is_empty());
    assert_eq!(fx.interact(), None);
    fx.assert_clean();
}

// A missing object is refused; a waypoint of another act is invalid.
#[test]
fn missing_object_and_other_act() {
    let mut fx = fixture(0, 0);
    let (code, _) = send(&mut fx.host, &Fx::msg(0xDEAD, 0));
    assert_eq!(code, ResultCode::Refused);
    let far = fx.far_wp;
    let (code, got) = send(&mut fx.host, &Fx::msg(far, 0));
    assert_eq!(code, ResultCode::Invalid);
    assert!(got.is_empty());
    fx.assert_clean();
}

// A staged player's act follows the world each tick.
#[test]
fn a_staged_players_act_follows_the_world_each_tick() {
    let mut fx = fixture(0, 0);
    // The fixture's spawn point (20, 20).
    let pos = crate::seams::Pos { x: 20, y: 20 };
    // Staged with an act the world no longer has it in (as after a
    // cross-act warp): the waypoint of its own act reads as another act.
    fx.host.game.set_unit(
        fx.player,
        crate::adapters::UnitFacts {
            act: 1,
            pos,
            owner: None,
        },
    );
    fx.host.clock.0 += 40;
    assert!(fx.host.frame().unwrap().ticked);
    fx.open_menu();
    let (code, _) = send(&mut fx.host, &Fx::msg(fx.wp, 0));
    assert_ne!(code, ResultCode::Invalid, "refused as another act");
    fx.assert_clean();
}

/// A host without waypoint tables keeps the stub (reported unhandled, result 0).
#[test]
fn without_waypoint_tables_0x49_stays_a_stub() {
    let mut fx = fixture(0, 0);
    fx.host.game.world.waypoints = None;
    let (code, got) = send(&mut fx.host, &Fx::msg(fx.wp, 0));
    assert_eq!(code, ResultCode::Done);
    assert!(got.is_empty());
    assert_eq!(fx.host.game.unhandled, vec![(0, 0x49, 9)]);
}

/// The NPC walk-command counter is process-wide. `ProcessState` hands it
/// to a new game and reads back what the game left; a fresh process
/// starts at 0.
#[test]
fn process_state_carries_the_npc_walk_counter_across_games() {
    use super::super::ProcessState;
    let mut process = ProcessState::new();
    let mut first = fixture(0, 0);
    let d = &mut first.host.game.events;
    process.start_game(d);
    assert_eq!(d.sys.hooks.ai.as_ref().unwrap().npc_walk_counter, 0);
    d.sys.hooks.ai.as_mut().unwrap().npc_walk_counter = 7;
    process.end_game(d);
    assert_eq!(process.npc_walk_counter, 7);
    // The next game of the process continues from 7, also without a store.
    let mut second = fixture(0, 0);
    let d = &mut second.host.game.events;
    d.sys.hooks.ai = None;
    process.start_game(d);
    assert_eq!(d.sys.hooks.ai.as_ref().unwrap().npc_walk_counter, 7);
    d.sys.hooks.ai.as_mut().unwrap().npc_walk_counter = u32::MAX;
    process.end_game(d);
    assert_eq!(process.npc_walk_counter, u32::MAX);
}

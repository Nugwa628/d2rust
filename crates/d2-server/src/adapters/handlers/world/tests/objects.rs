//! C→S 0x13 with unit type 2 (the object case) on the wired sim:
//! `ActionSim` with its object state (objects created through the real
//! unit allocation and init dispatch), routed by the world handler to the
//! operate entry; operate 23 to the waypoints, the object update pass's
//! S→C 0x0E through the tick. Only `Pending` (positions, interact range,
//! routes, sends) is faked ([`super::waypoints::TestPending`]).
//!
//! Part of the world handler's test suite: checks that `world::handler`
//! routes object interactions to the sim's object module and that the
//! results reach the client.

use std::sync::Arc;

use d2_data::tables::{Levels, Objects};
use d2_sim::combat::CombatTables;
use d2_sim::game::Game;
use d2_sim::rng::Seed;
use d2_sim::skills::SkillTables;
use d2_sim::stats::StatData;
use d2_sim::units::hooks::UnitData;
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionSim, ActionTables, ObjectRoute};
use d2_sim::world::objects::{state_message, Dispatch, ObjectTables, Operate};
use d2_sim::world::waypoints::{WaypointData, NO_WAYPOINT};

use super::waypoints::{blank, field_drlg, field_room, TestPending, COLD_PLAINS};
use super::*;
use crate::adapters::handlers::world::{route, ActionWorld, System};
use crate::adapters::{PlayerData, PlayerFields, SimGame};
use crate::seams::PlayerGate;

const WAYPOINT: u32 = 0;
const TORCH: u32 = 1;
const QUEST: u32 = 2;

/// objects.txt rows: a waypoint (init 17, operate 23), a torch (operate
/// 11), a quest object (operate 9).
fn rows() -> Vec<Objects> {
    let row = |initfn: u8, operatefn: u8| {
        let mut o: Objects = blank();
        o.initfn = initfn;
        o.operatefn = operatefn;
        o.framecnt1 = 15 << 8;
        o
    };
    vec![row(17, 23), row(0, 11), row(0, 9)]
}

fn levels() -> Vec<Levels> {
    let mut levels = vec![blank::<Levels>(); 150];
    for l in &mut levels {
        l.waypoint = NO_WAYPOINT;
    }
    levels[COLD_PLAINS as usize].waypoint = 1;
    levels
}

type Sim = SimGame<ActionSim<TestPending>, ActionWorld>;

struct Fx {
    host: TestHost<Sim>,
    player: UnitId,
    /// Waypoint, torch, quest object.
    objects: [UnitId; 3],
}

/// Cold Plains with a waypoint at (20, 20), a torch at (24, 20), a quest
/// object at (26, 20) and a player at (22, 20) for client 0. `objects`:
/// the game's object state is created (`ActionSim::create_objects`)
/// before the objects; without it they are plain units.
fn fixture(objects: bool) -> Fx {
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
    let pending = TestPending {
        in_range: true,
        ..TestPending::default()
    };
    let hooks = ActionHooks::new(
        Arc::new(tables),
        field_drlg(),
        Seed::init_low(1234),
        pending,
    );
    let mut sim = ActionSim::new(Arc::new(StatData::default()), UnitData::default(), hooks);
    let mut game = Game::new();
    let room = field_room(&mut sim, &mut game);
    if objects {
        sim.create_objects(Arc::new(ObjectTables {
            objects: rows(),
            shrines: Vec::new(),
            levels: levels(),
            objgroup: Vec::new(),
            leveldefs: Vec::new(),
        }));
    }
    let mut alloc = |game: &mut Game, ty: UnitType, class: u32, x: i32| {
        let req = AllocRequest {
            ty,
            class,
            room: Some(room),
            add: true,
            fixed_guid: None,
            mode: if ty == UnitType::Player { 1 } else { 0 },
            allied: ty == UnitType::Player,
        };
        let u = sim.with(game, |g, v| v.allocate(g, &req, x, 20)).unwrap();
        // Staged as already announced: the seed-set flag is cleared.
        sim.sys.units.get_mut(u).unwrap().flags &= !d2_sim::units::record::flags::SEED_SET;
        u
    };
    let objs = [
        alloc(&mut game, UnitType::Object, WAYPOINT, 20),
        alloc(&mut game, UnitType::Object, TORCH, 24),
        alloc(&mut game, UnitType::Object, QUEST, 26),
    ];
    let player = alloc(&mut game, UnitType::Player, 0, 22);
    sim.sys.units.get_mut(player).unwrap().mode = 1;
    let mut s: Sim = SimGame::with_events(game, sim);
    s.world.waypoints = Some(WaypointData::new(&levels(), &rows()));
    s.join(0, Some(player), Some(room), client_state::IN_GAME)
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
        objects: objs,
    }
}

impl Fx {
    fn guid(&self, u: UnitId) -> u32 {
        self.host.game.game.lists.unit(u).unwrap().guid
    }
    fn mode(&mut self, u: UnitId) -> u32 {
        self.host.game.events.sys.units.get(u).unwrap().mode
    }
    fn pending(&mut self) -> &mut TestPending {
        &mut self.host.game.events.hooks().x
    }
    fn assert_clean(&mut self) {
        assert!(self.host.game.world.faults.is_empty());
        assert!(self.host.game.unhandled.is_empty());
        assert!(self.host.game.events.hooks().errors.is_empty());
    }
}

/// C→S 0x13 for unit type 2 and `guid`.
fn msg(guid: u32) -> Vec<u8> {
    let mut m = vec![0x13, 2, 0, 0, 0];
    m.extend_from_slice(&guid.to_le_bytes());
    m
}

// Operating a waypoint sets the level's bit and sends the object's mode 1 to the client.
#[test]
fn waypoint_operate_reaches_the_waypoints_and_the_client() {
    let mut fx = fixture(true);
    let wp = fx.objects[0];
    let g = fx.guid(wp);
    assert_eq!(route(&msg(g)), Some(System::Objects));
    let (code, got) = send(&mut fx.host, &msg(g));
    assert_eq!(code, ResultCode::Done);
    // First the level bit is set; then the object goes to mode 1 (on the
    // object module), sent by the tick's update pass as S→C 0x0E.
    let p = fx.player;
    assert!(fx.host.game.events.hooks().waypoints[&p].0[0]
        .test(1)
        .unwrap());
    assert_eq!(fx.mode(wp), 1);
    assert_eq!(got, [state_message(g, false, 1).to_vec()]);
    fx.assert_clean();
}

// Missing object, torch (operate 11), quest object (operate 9) and out-of-range cases.
#[test]
fn torch_quest_and_refusals() {
    let mut fx = fixture(true);
    let [wp, torch, quest] = fx.objects;
    // Missing object → 1.
    let (code, got) = send(&mut fx.host, &msg(0xDEAD));
    assert_eq!((code, got), (ResultCode::Refused, vec![]));
    // The torch runs operate 11; whatever mode it leaves is sent
    // as 0x0E by the update pass.
    let g = fx.guid(torch);
    let (code, got) = send(&mut fx.host, &msg(g));
    assert_eq!(code, ResultCode::Done);
    let m = fx.mode(torch);
    let want: Vec<Vec<u8>> = if m == 0 {
        vec![]
    } else {
        vec![state_message(g, false, m).to_vec()]
    };
    assert_eq!(got, want);
    // Operate 9 is a quest function: handed back, nothing else.
    let g = fx.guid(quest);
    let (code, got) = send(&mut fx.host, &msg(g));
    assert_eq!((code, got), (ResultCode::Done, vec![]));
    let p = fx.player;
    let r = ObjectRoute::Operate(Dispatch::Quest(Operate {
        object: quest,
        operator: Some(p),
        class: QUEST as u16,
        operate_fn: 9,
    }));
    assert_eq!(
        fx.pending().log.last(),
        Some(&format!("object route {r:?}"))
    );
    // Out of interact range: no dispatch.
    fx.pending().in_range = false;
    let g = fx.guid(wp);
    let (code, got) = send(&mut fx.host, &msg(g));
    assert_eq!((code, got), (ResultCode::Done, vec![]));
    assert_eq!(fx.mode(wp), 0);
    fx.assert_clean();
}

// Without the game's object state, unit type 2 is reported as unhandled.
#[test]
fn without_object_state_type_2_stays_a_stub() {
    let mut fx = fixture(false);
    let g = fx.guid(fx.objects[0]);
    let (code, got) = send(&mut fx.host, &msg(g));
    assert_eq!((code, got), (ResultCode::Done, vec![]));
    assert_eq!(fx.host.game.unhandled, vec![(0, 0x13, 9)]);
}

// The object module's host tick follows the frame's host clock.
#[test]
fn the_object_host_tick_is_the_frames_host_clock() {
    let mut fx = fixture(true);
    // `host` ran one frame at 1000 ms; the game reads the clock plus the
    // uptime origin (`seams::HOST_CLOCK_ORIGIN_MS`).
    let o = crate::seams::HOST_CLOCK_ORIGIN_MS;
    let tick = |fx: &mut Fx| {
        fx.host
            .game
            .events
            .hooks()
            .objects
            .as_ref()
            .unwrap()
            .host_tick
    };
    assert_eq!(tick(&mut fx), o + 1000);
    fx.host.clock.0 = 7_777;
    fx.host.frame().unwrap();
    assert_eq!(tick(&mut fx), o + 7_777);
    // An operate in that frame reads it (door debounce): the next message's
    // frame moves it again.
    let g = fx.guid(fx.objects[1]);
    send(&mut fx.host, &msg(g));
    assert_eq!(tick(&mut fx), o + 7_817);
}

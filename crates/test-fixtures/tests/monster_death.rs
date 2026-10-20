//! A monster's death end to end on the server host: the synthetic
//! install ([`test_fixtures::synth`]) → `GameData` → a `WorldSim` with
//! the path provider on → the one-room town streamed, a player and a
//! `beast1` monster allocated in it → `SimGame` joined → `Host::frame`s
//! (drain → one tick → flush to the client).
//!
//! The monster is killed (the death mode
//! DT toward the killer) in the message drain before a frame's tick
//! (mode sets run in tick step 4 or in
//! the drain before the tick). Expected:
//!
//! - that frame's tick sends S→C 0x69 code 8: GUID, the
//!   path target (a, b), d = the path direction, e = the unit's hit class;
//! - the DT animation (24 frames at speed 256) ends with
//!   event 1 at f + 24, f = the frame of the kill; its function
//!   sets mode 12 and the same tick sends
//!   0x69 code 9 at the unit's cell with e = 0;
//! - nothing else from the monster in between or after (the room
//!   clean-up clears unit flag 0x1 each tick).
//!
//! Fixture answers for seams D2RUST does not implement yet (each says so where it is
//! answered): the death start's body (it sets mode DT, as a start
//! function sets its mode), the COF name and the animation rate (the
//! death record "BEDTHTH", 24 frames, speed 256, added to this test's
//! AnimData), and the hit class (6; written on the unit record, as the kill is direct, without a hit).

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use d2_data::tables::Monstats;
use d2_formats::animdata::{self, AnimData, AnimRecord};
use d2_server::adapters::handlers::world::{ActionWorld, Outbox};
use d2_server::adapters::{PlayerData, PlayerFields, ProtoSizes, SimGame};
use d2_server::host::Host;
use d2_server::seams::{ClientId, Clock, MessageSink, PlayerGate, SessionHandler};
use d2_sim::game::Game;
use d2_sim::units::hooks::Sim as USim;
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::{modes, UnitId, UnitType};
use d2_sim::wiring::action::reaction::kill;
use d2_sim::wiring::action::{ActionHooks, Pending};
use d2_sim::wiring::worldgen::{WorldPending, WorldSim};
use test_fixtures::game::{ActCreation, GameData};
use test_fixtures::{install, synth};

/// The synthetic town (levels row 1).
const TOWN: u32 = 1;
const INIT: u32 = 0x1234_5678;
const GAME_SEED: u32 = 1234;
const CLASS: u32 = 3;
const CLIENT: ClientId = 0;
/// Sub-tiles per tile.
const SUB: i32 = 5;
/// The death record of this test.
const DEATH: &[u8; 8] = b"BEDTHTH\0";
const DEATH_FRAMES: u32 = 24;
/// The fixture's hit class.
const B0: u8 = 6;
/// Frames before the kill (the monster's AI runs meanwhile).
const BEFORE: usize = 10;

fn data() -> &'static GameData {
    static D: OnceLock<GameData> = OnceLock::new();
    D.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("monster-death-{}", std::process::id()));
        let i = install::build(&dir, &synth::synthetic()).unwrap_or_else(|e| panic!("{e}"));
        GameData::from_install(&i).unwrap_or_else(|e| panic!("{e}"))
    })
}

// ---- the seams ------------------------------------------------------------------------

/// The transport (as `test_fixtures::game::Seams`), plus the fixture
/// answers listed in the module doc.
#[derive(Default)]
struct Fx {
    sent: Vec<(UnitId, Vec<u8>)>,
}

impl Pending for Fx {
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.sent.push((player, msg.to_vec()));
    }
    /// The COF composer is not implemented: the monster's DT name only.
    fn anim_name(&self, _: UnitId, ty: UnitType, _: u32, mode: u32) -> Option<[u8; 8]> {
        (ty == UnitType::Monster && mode == 0).then_some(*DEATH)
    }
    /// The animation rate: the AnimData speed.
    fn anim_rate(&self, _: UnitId, speed: Option<u32>) -> i16 {
        speed.map_or(0, |s| s as i16)
    }
    /// The death start's body is not written: mode DT (a start function
    /// sets its mode).
    fn monster_death_start(
        h: &mut ActionHooks<Self>,
        sim: &mut USim<'_>,
        unit: UnitId,
        _: Option<UnitId>,
    ) -> bool {
        modes::set_mode(sim, h, unit, 0).expect("mode DT");
        true
    }
}

impl WorldPending for Fx {}

impl Outbox for Fx {
    fn take_sent(&mut self) -> Vec<(UnitId, Vec<u8>)> {
        std::mem::take(&mut self.sent)
    }
}

type Game2 = SimGame<WorldSim<Fx>, ActionWorld>;

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

type TestHost = Host<Game2, ProtoSizes, NoSession, Ms>;

/// The run's record: per frame (the tick's frame number) the monster
/// messages (S→C 0x67–0x6D) the client received.
#[derive(Debug, PartialEq, Eq)]
struct Run {
    guid: u32,
    kill_frame: i32,
    frames: Vec<(i32, Vec<Vec<u8>>)>,
    code8: Vec<u8>,
    code9: Vec<u8>,
}

fn is_monster_message(m: &[u8]) -> bool {
    (0x67..=0x6D).contains(&m[0])
}

fn run() -> Run {
    let d = data();
    let (mut sim, _types) = d
        .world_sim(ActCreation::TownOnly, INIT, TOWN, GAME_SEED, Fx::default())
        .unwrap_or_else(|e| panic!("{e}"));
    // This test's AnimData: the synthetic set plus the death record.
    {
        let h = sim.action.hooks();
        let mut a: AnimData = (**h.anim_data.as_ref().unwrap()).clone();
        let len = DEATH.iter().position(|&b| b == 0).unwrap();
        a.buckets[animdata::hash(&DEATH[..len])].push(AnimRecord {
            name: *DEATH,
            frames: DEATH_FRAMES,
            speed: 256,
            events: [0; animdata::EVENTS],
        });
        h.anim_data = Some(Arc::new(a));
    }
    let monstats: Vec<Monstats> = d.rows().unwrap();
    assert!(monstats[0].killable, "monstats row 0 (beast1) is killable");

    let mut game = Game::new();
    game.lists.ensure_act(0).unwrap();
    let (room, rect) = sim
        .action
        .hooks()
        .drlg
        .with_act(0, &mut game.lists, |dr, svc| {
            let lv = dr.get_or_alloc_level(svc.data, svc.types, TOWN)?;
            if dr.level_rooms(lv).is_empty() {
                dr.generate_level(svc.data, svc.types, lv)?;
            }
            let r = dr.level_rooms(lv)[0];
            let active = dr.stream_room(svc, r)?;
            Ok::<_, d2_sim::drlg::DrlgError>((active, dr.room(r).rect))
        })
        .expect("act 0 has a DRLG")
        .unwrap_or_else(|e| panic!("town: {e:?}"));
    let room = room.expect("the town room is active");
    let centre = (
        rect.x * SUB + rect.w * SUB / 2,
        rect.y * SUB + rect.h * SUB / 2,
    );
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
    let player = alloc(UnitType::Player, CLASS, centre);
    let monster = alloc(UnitType::Monster, 0, (centre.0 + 4, centre.1));
    sim.action.sys.units.get_mut(player).unwrap().mode = 1;
    let guid = game.lists.unit(monster).unwrap().guid;

    let mut s: Game2 = SimGame::with_world(game, sim, ActionWorld::default());
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
    let mut host: TestHost = Host::new(s, ProtoSizes, NoSession, Ms(1000));
    host.connect(CLIENT);
    host.frame().expect("first frame");
    host.receive(CLIENT);

    let mut frames = Vec::new();
    let step = |host: &mut TestHost, frames: &mut Vec<(i32, Vec<Vec<u8>>)>| {
        host.clock.0 += 40;
        let r = host.frame().expect("frame");
        assert!(r.ticked, "one tick per frame");
        let got: Vec<Vec<u8>> = host
            .receive(CLIENT)
            .into_iter()
            .filter(|m| is_monster_message(m))
            .collect();
        let s = &host.game;
        assert!(s.tick_faults.is_empty(), "{:?}", s.tick_faults);
        assert_eq!(s.events.errors(), Vec::<String>::new());
        frames.push((s.game.frame, got));
    };
    for _ in 0..BEFORE {
        step(&mut host, &mut frames);
    }
    // Every monster message before the kill is the monster's.
    for (_, ms) in &frames {
        for m in ms {
            assert_eq!(&m[1..5], &guid.to_le_bytes(), "{m:02x?}");
        }
    }
    let before = frames.len();

    // The kill, in the drain before the next frame's tick.
    let kill_frame = host.game.game.frame;
    {
        let s = &mut host.game;
        // The killing hit's class store: the
        // direct kill has no hit, so the fixture writes the hit class.
        s.events
            .action
            .sys
            .units
            .get_mut(monster)
            .unwrap()
            .hit_class = u32::from(B0);
        s.events
            .action
            .combat(&mut s.game, |cv, _| kill(cv, monster, player));
        assert_eq!(s.events.action.sys.units.get(monster).unwrap().mode, 0);
    }
    let path = |host: &mut TestHost| {
        host.game
            .events
            .action
            .hooks()
            .paths
            .as_ref()
            .unwrap()
            .dynamic(monster)
            .unwrap()
            .clone()
    };
    let p8 = path(&mut host);
    let mut code8 = vec![0x69];
    code8.extend(guid.to_le_bytes());
    code8.push(8);
    // The path end: no computed point, so (0, 0) (REC-594).
    assert_eq!(p8.point_count, 0);
    code8.extend([0, 0, 0, 0]);
    code8.extend([p8.direction, B0]);

    let end = kill_frame + DEATH_FRAMES as i32;
    while host.game.game.frame < end + 20 {
        step(&mut host, &mut frames);
        if host.game.game.frame == end {
            assert_eq!(
                host.game.events.action.sys.units.get(monster).unwrap().mode,
                12
            );
        }
    }
    let p9 = path(&mut host);
    let mut code9 = vec![0x69];
    code9.extend(guid.to_le_bytes());
    code9.push(9);
    code9.extend((p9.x() as u16).to_le_bytes());
    code9.extend((p9.y() as u16).to_le_bytes());
    code9.extend([p9.direction, 0]);

    // After the kill: code 8 in the kill's tick, code 9 in the tick of
    // f + 24, nothing else.
    for (f, ms) in &frames[before..] {
        let want: Vec<Vec<u8>> = if *f == kill_frame + 1 {
            vec![code8.clone()]
        } else if *f == end {
            vec![code9.clone()]
        } else {
            Vec::new()
        };
        assert_eq!(ms, &want, "frame {f}");
    }
    Run {
        guid,
        kill_frame,
        frames,
        code8,
        code9,
    }
}

#[test]
fn a_kill_sends_0x69_code_8_then_code_9_at_the_death_end() {
    let a = run();
    println!(
        "monster {:#x}: killed after frame {}, code 8 {:02x?}, code 9 {:02x?}; before the kill: {:02x?}",
        a.guid,
        a.kill_frame,
        a.code8,
        a.code9,
        &a.frames[..BEFORE]
    );
    // The values this fixture gives, as literal bytes: the first unit
    // (GUID 1), a path whose only target is the one the creation's mode
    // request wrote, the spawn cell (24, 20) (state
    // snapshots read an idle spawned monster's path target as its
    // spawn point), d = 0x17: the death clean-up
    // snapped the path direction toward the killer, due −x (the game turns
    // the dead monster to its killer), e = the fixture's hit class; code 9 at the cell (24, 20) where it was
    // allocated (the room centre + (4, 0)).
    // Code 8 (a, b) is the path end, (0, 0) with no computed point,
    // not that target.
    assert_eq!(a.code8, [0x69, 1, 0, 0, 0, 8, 0, 0, 0, 0, 0x17, B0]);
    assert_eq!(a.code9, [0x69, 1, 0, 0, 0, 9, 24, 0, 20, 0, 0x17, 0]);
    assert_eq!(a.kill_frame, BEFORE as i32);
    // Determinism: a second game is identical.
    assert_eq!(run(), a);
}

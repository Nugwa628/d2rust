//! Integration test of the whole wired single-player host on the game's live
//! tables and MPQs (`#[ignore]`, `D2_GAME_DIR`; the game-file tier of
//! `d2-server`), once per class of `charstats`:
//!
//! 1. game creation: act 0 through `WorldTypes` (the `d2_server::world_data`
//!    providers) on a fixed init seed, `WorldSim` with the population
//!    and init tables (population on: the room pass runs; room population
//!    has no coordinate-list provider here, so only presets
//!    place units), the path provider on, `ActionWorld` with the live
//!    waypoint tables; the town generated, its waypoint room streamed;
//! 2. the town waypoint object (the preset unit of the town whose
//!    `objects` row has operate function 23) and the player of the class
//!    (with its creation stats) allocated there; the client joins;
//! 3. 500 host frames (one tick each) with no fault;
//! 4. the walk to the town's Blood Moor exit: the target is computed from
//!    the generated level rects (the shared edge of the Rogue Encampment
//!    and the Blood Moor), each leg is a C→S 0x03 through the host, which
//!    the server's walk handler runs (`adapters::handlers::walk`: the
//!    sim's `walk_message`); ticks until the player
//!    stops; repeated until the player stands in a Blood Moor room;
//! 5. the kill: if a monster stands within the target range, the class's
//!    `StartSkill` is selected (0x3C) and cast on it (0x0D) through the
//!    host; the live host has no skill-use provider (`SkillRest` exists
//!    only as the synthetic e2e fixture), so both stop at the asserted
//!    stub; without a monster the step is skipped and says so;
//! 6. the pick-up (0x16) of a ground item in reach: the one-item-store
//!    change (unify-items) has not landed and the live host has no
//!    inventory parts, so it stops at the asserted stub (skipped without
//!    an item);
//! 7. back to the town waypoint, then C→S 0x49 to the Cold Plains
//!    the player in a Cold Plains room and S→C
//!    0x0D with the bytes (type 0, GUID, 1, x + 3, y + 3,
//!    0, 0), after a 0x07;
//! 8. a digest of the state (units, positions, stats, seeds, DRLG room
//!    counts, the client's transcript) printed per class; the whole run
//!    is done twice in the test and the digests must be equal
//!    (determinism on real data); two local runs compare the printed
//!    digests.
//!
//! Expected values: only fixed ones (creation stats, Blood
//! Moor / Cold Plains room allocations, the 0x0D bytes and order, the
//! 0x49 range); everything else is an invariant (no fault, no wiring
//! error, positions inside their rooms, active rooms within their level,
//! determinism). The walk's success (reaching Blood Moor, back to the
//! waypoint) is the test's goal, not a fixed value: a failure prints the
//! position and is a wiring finding. Unconfirmed until the first local run.
//!
//! The game is built by `test_fixtures::game::GameData` from the live set
//! and driven by `test_fixtures::host::Session`, the same constructor and
//! session the synthetic Act I e2e (`test-fixtures/tests/act1_game.rs`)
//! runs in CI.

use std::sync::OnceLock;

use d2_data::bin;
use d2_formats::mpq::ArchiveSet;
use d2_server::adapters::UnitFacts;
use d2_server::seams::{Pos, ResultCode};
use d2_sim::combat::vitals::VitalsTables;
use d2_sim::path::CollisionRooms;
use d2_sim::units::UnitType;
use test_fixtures::game::{ActCreation, GameData};
use test_fixtures::host::{border_goals, cheb, fnv, Session, Setup};

// ---- live data ------------------------------------------------------------------------

/// The live set as a [`GameData`]: the loaded `.bin` set, AnimData, the
/// fix-ups, the level-type views and every DRLG file.
fn live() -> &'static GameData {
    static L: OnceLock<GameData> = OnceLock::new();
    L.get_or_init(|| {
        let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
        let set = ArchiveSet::open_dir(dir).expect("archives open");
        let bins = bin::load(&set, bin::DEFAULT_LANGUAGE).expect("live .bin set loads");
        GameData::load(bins, &set).unwrap_or_else(|e| panic!("game data: {e}"))
    })
}

// ---- constants ----------------------------------------------

/// The Act I creation DRLG init seed.
const INIT: u32 = 644_409_375;
/// Rogue Encampment, the server's town level.
const TOWN: u32 = 1;
/// Blood Moor and Cold Plains (levels 2 and 3).
const BLOOD_MOOR: u32 = 2;
const COLD_PLAINS: u32 = 3;
/// Room allocations of one Blood Moor / Cold Plains build.
const BLOOD_MOOR_ROOMS: usize = 81;
const COLD_PLAINS_ROOMS: usize = 98;
/// 0x49 range in sub-tiles: 22 for the sorceress (class 1), else 10.
fn waypoint_range(class: u32) -> i32 {
    if class == 1 {
        22
    } else {
        10
    }
}
/// The point / unit target range (`dispatch::in_range`, 50 sub-tiles).
const TARGET_RANGE: i32 = 50;

/// Fixture choices (no rule places a joining player; it is not wired at
/// join): the game seed and the idle frame count; the
/// start offset, leg length and budgets are `test_fixtures::host`'s.
const GAME_SEED: u32 = 1234;
const IDLE_FRAMES: usize = 500;

const CLIENT: d2_server::seams::ClientId = test_fixtures::host::CLIENT;

/// Steps 1–2: game creation (act 0 through the placer, `ActCreation::Full`),
/// the town waypoint and the player of `class`, the Cold Plains waypoint
/// known (staged: a new record knows only the town;
/// nothing activates another here), the client joined.
fn session(class: u32) -> Session {
    Session::new(
        live(),
        &Setup {
            creation: ActCreation::Full,
            init_seed: INIT,
            town: TOWN,
            game_seed: GAME_SEED,
            class,
            known_waypoints: vec![COLD_PLAINS],
        },
    )
}

// ---- the run -----------------------------------------------------------------------

struct Run {
    digest: u64,
    state: String,
    notes: Vec<String>,
}

/// Steps 3–8 for one class.
fn run(class: u32) -> Run {
    let mut fx = session(class);
    let player = fx.player;

    // 3. 500 ticks with no fault; the first tick's room change gives the
    // client a room.
    let frame0 = fx.sim().game.frame;
    for _ in 0..IDLE_FRAMES {
        fx.frame();
    }
    assert_eq!(fx.sim().game.frame, frame0 + 500, "500 ticks");
    fx.assert_clean("500 ticks");
    assert!(fx.sim().game.lists.unit(player).is_some(), "player in game");
    let sc = fx.sim().sim_client(CLIENT).expect("client joined");
    assert!(
        fx.sim()
            .game
            .lists
            .client(sc)
            .and_then(|c| c.room)
            .is_some(),
        "the client has a room (rooms.md §4.1)"
    );
    assert_eq!(fx.pos(), fx.start, "standing still for 500 ticks");

    // 4. The town's Blood Moor exit.
    let town = fx.level_rect(TOWN);
    let moor = fx.level_rect(BLOOD_MOOR);
    let p = fx.pos();
    let goals = border_goals(town, moor, p);
    fx.walk(
        &goals,
        |f| f.unit_level(f.player) == Some(BLOOD_MOOR),
        "Blood Moor",
    );
    println!(
        "class {class}: in Blood Moor at {:?}, frame {}",
        fx.pos(),
        fx.sim().game.frame
    );

    // 5. The kill: a monster within the target range, if any.
    let me = fx.pos();
    let mut target = None;
    for m in fx.sim().game.lists.units_of_type(UnitType::Monster) {
        let at = fx.sim().events.action.hooks().path_position(m);
        let alive = !fx.sim().events.action.sys.units.is_dead(m);
        if alive && cheb(at, me) <= TARGET_RANGE && fx.unit_level(m) != Some(TOWN) {
            target = Some((m, at));
            break;
        }
    }
    match target {
        Some((m, at)) => {
            let guid = fx.sim().game.lists.unit(m).unwrap().guid;
            fx.sim().set_unit(
                m,
                UnitFacts {
                    act: 0,
                    pos: Pos { x: at.0, y: at.1 },
                    owner: None,
                },
            );
            let skill = u32::from(
                live_vitals()
                    .charstats(class as i32)
                    .expect("charstats")
                    .startskill,
            );
            let mut select = vec![0x3C];
            select.extend_from_slice(&(skill & 0x7FFF_FFFF).to_le_bytes());
            select.extend_from_slice(&u32::MAX.to_le_bytes());
            fx.assert_stub(&select);
            let mut cast = vec![0x0D];
            cast.extend_from_slice(&1u32.to_le_bytes());
            cast.extend_from_slice(&guid.to_le_bytes());
            fx.assert_stub(&cast);
            let mclass = fx.sim().events.action.sys.units.get(m).unwrap().class;
            fx.notes.push(format!(
                "kill: monster class {mclass} at {at:?}; 0x3C / 0x0D stubs (no SkillRest on the live host)"
            ));
        }
        None => fx
            .notes
            .push("kill: no monster within range (room population §3 not provided)".into()),
    }

    // 6. The pick-up of a ground item in reach, if any.
    let me = fx.pos();
    let item = fx
        .sim()
        .game
        .lists
        .units_of_type(UnitType::Item)
        .into_iter()
        .find(|&i| {
            let at = fx.host.game.events.action.hooks().path_position(i);
            cheb(at, me) <= TARGET_RANGE
        });
    match item {
        Some(i) => {
            let guid = fx.sim().game.lists.unit(i).unwrap().guid;
            let mut pick = vec![0x16];
            pick.extend_from_slice(&4u32.to_le_bytes());
            pick.extend_from_slice(&guid.to_le_bytes());
            pick.extend_from_slice(&0u32.to_le_bytes());
            fx.assert_stub(&pick);
            fx.notes
                .push("pick-up: 0x16 stub (unify-items not landed, no inventory parts)".into());
        }
        None => fx.notes.push("pick-up: no ground item in reach".into()),
    }

    // 7. Back to the town waypoint, then 0x49 to the Cold Plains. The
    // walk retraces the way out (the positions after each leg since the
    // start), then heads for the start: players path greedily
    // (a ray, a 73-step greedy walk, A* only within
    // 18 sub-tiles), so a straight leg back stops behind the palisade
    // the way out went round.
    let mut back: Vec<(i32, i32)> = fx.trail.iter().rev().copied().collect();
    back.dedup();
    let (wp_at, range, start) = (fx.wp_at, waypoint_range(class), fx.start);
    back.push(start);
    fx.walk(
        &back,
        move |f| {
            let p = f.pos();
            (p.0 - wp_at.0).abs() <= range && (p.1 - wp_at.1).abs() <= range
        },
        "town waypoint",
    );
    let mut m = vec![0x49];
    m.extend_from_slice(&fx.wp_guid.to_le_bytes());
    m.extend_from_slice(&(COLD_PLAINS as u16).to_le_bytes());
    m.extend_from_slice(&[0, 0]);
    let (code, got) = fx.send(&m);
    assert_eq!(code, ResultCode::Done, "0x49 accepted (§6.2 step 9)");
    fx.assert_clean("waypoint");
    assert_eq!(
        fx.unit_level(player),
        Some(COLD_PLAINS),
        "travel ends in a Cold Plains room (§7 rule 6)"
    );
    // The 0x0D carries the position after placement + 3. The
    // frame's tick runs after the drain; the player stands in mode 2 at
    // its own position, so the position is unchanged.
    let (x, y) = fx.pos();
    let mut want = vec![0x0D, 0x00];
    want.extend_from_slice(&fx.player_guid.to_le_bytes());
    want.push(1);
    want.extend_from_slice(&((x + 3) as u16).to_le_bytes());
    want.extend_from_slice(&((y + 3) as u16).to_le_bytes());
    want.extend_from_slice(&[0, 0]);
    let at_0d = got
        .iter()
        .position(|g| g.first() == Some(&0x0D))
        .unwrap_or_else(|| panic!("no 0x0D among {got:02x?}"));
    assert_eq!(got[at_0d], want, "0x0D bytes (§7 rule 7)");
    assert!(
        got[..at_0d].iter().any(|g| g.first() == Some(&0x07)),
        "0x07 before 0x0D (§8 rule 3): {got:02x?}"
    );

    // Invariants: room counts, positions inside rooms.
    let mut levels = Vec::new();
    {
        let d = fx.host.game.events.action.hooks().drlg.dungeon.acts[0]
            .as_ref()
            .unwrap();
        for l in d.level_list() {
            let rooms = d.level_rooms(l);
            if rooms.is_empty() {
                continue;
            }
            let active = rooms
                .iter()
                .filter(|&&r| d.active_room(r).is_some())
                .count();
            levels.push((d.level(l).id, rooms.len(), active));
        }
    }
    for &(id, n, active) in &levels {
        assert!(active <= n, "level {id}: {active} active of {n}");
        match id {
            BLOOD_MOOR => assert_eq!(n, BLOOD_MOOR_ROOMS, "outdoor.md room allocations"),
            COLD_PLAINS => assert_eq!(n, COLD_PLAINS_ROOMS, "outdoor.md room allocations"),
            _ => {}
        }
    }
    let mut units = Vec::new();
    for ty in UnitType::ALL {
        for u in fx.sim().game.lists.units_of_type(ty) {
            let room = fx.sim().game.lists.unit(u).and_then(|e| e.room());
            let h = fx.host.game.events.action.hooks();
            let at = h.path_position(u);
            if let Some(r) = room.filter(|_| h.path_has(u)) {
                if let Some(rect) = h.drlg.subtile_rect(r) {
                    assert!(
                        at.0 >= rect.x
                            && at.0 < rect.x + rect.w
                            && at.1 >= rect.y
                            && at.1 < rect.y + rect.h,
                        "{ty:?} {u:?} at {at:?} outside its room {rect:?}"
                    );
                }
            }
            let r = fx.host.game.events.action.sys.units.get(u).unwrap();
            units.push((ty, r.class, r.guid, r.mode, r.seed, at, room));
        }
    }
    let stats: Vec<i32> = (0..=15)
        .map(|s| {
            fx.host
                .game
                .events
                .action
                .sys
                .stats
                .unit_total(player, s, 0)
        })
        .collect();
    let seed = fx.host.game.events.action.hooks().game_seed;
    let state = format!(
        "frame {} seed {seed:?}\nlevels {levels:?}\nunits {units:?}\nstats {stats:?}\nunhandled {:?}\ntranscript {:?}",
        fx.host.game.game.frame, fx.host.game.unhandled, fx.transcript
    );
    Run {
        digest: fnv(state.as_bytes()),
        state,
        notes: fx.notes,
    }
}

fn live_vitals() -> &'static VitalsTables {
    static V: OnceLock<VitalsTables> = OnceLock::new();
    V.get_or_init(|| live().vitals().expect("vitals tables"))
}

/// The whole run twice: equal digests (same seed, same data → same state
/// and transcript); prints the digest for the run-to-run comparison.
fn twice(class: u32) {
    let a = run(class);
    let b = run(class);
    for n in &a.notes {
        println!("class {class}: {n}");
    }
    if a.digest != b.digest {
        let line = a
            .state
            .lines()
            .zip(b.state.lines())
            .position(|(x, y)| x != y);
        panic!("class {class}: two runs differ (first differing line {line:?})");
    }
    println!("class {class}: digest {:016x}", a.digest);
}

// ---- one test per class (charstats rows 0–6) ------------------------------------------

// Intended claim (unconfirmed until the first local run): none (an integration run; the spec-stated values it asserts are claimed by their own tests).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn wired_host_amazon() {
    twice(0);
}

// Intended claim (unconfirmed until the first local run): none (integration run).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn wired_host_sorceress() {
    twice(1);
}

// Intended claim (unconfirmed until the first local run): none (integration run).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn wired_host_necromancer() {
    twice(2);
}

// Intended claim (unconfirmed until the first local run): none (integration run).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn wired_host_paladin() {
    twice(3);
}

// Intended claim (unconfirmed until the first local run): none (integration run).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn wired_host_barbarian() {
    twice(4);
}

// Intended claim (unconfirmed until the first local run): none (integration run).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn wired_host_druid() {
    twice(5);
}

// Intended claim (unconfirmed until the first local run): none (integration run).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn wired_host_assassin() {
    twice(6);
}

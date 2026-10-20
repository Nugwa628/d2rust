//! Act I on synthetic data, end to end and in CI: the Act I-shaped set
//! ([`test_fixtures::act1`]) → `GameData` → act 0 created by the real
//! placer (`ActCreation::Full`) → the preset town generated → a player
//! joined in the town → frames → walk legs over the town's Blood Moor
//! edge until the player stands in a Blood Moor room, which the Act I
//! outdoor generator built.
//!
//! Expected values: the placement rects, allocation order and DRLG
//! seeds are the reference placement vector:
//! the synthetic levels carry the vector's sizes and offsets, so the
//! placer must reproduce it; the town's room count is the
//! reference one (56 × 40 → 35 rooms).
//! Everything else is an invariant (no fault, no wiring error, positions
//! inside rooms, two runs identical). No `Covers:` claim: the data is
//! made up, not the game's.

use std::path::PathBuf;
use std::sync::OnceLock;

use d2_sim::drlg::{RoomKind, TileRect};
use d2_sim::game::Game;
use d2_sim::path::CollisionRooms;
use d2_sim::rng::Seed;
use d2_sim::units::UnitType;
use test_fixtures::act1::{self, BLOOD_MOOR, COLD_PLAINS, TOWN};
use test_fixtures::game::{ActCreation, GameData};
use test_fixtures::host::{border_goals, fnv, Session, Setup};
use test_fixtures::install;

/// The reference Act I creation's DRLG init seed and start seed.
const INIT: u32 = 644_409_375;
const START: u32 = 4_014_346_869;
/// Fixture choices.
const GAME_SEED: u32 = 1234;
const CLASS: u32 = 3;
const IDLE_FRAMES: usize = 50;

fn data() -> &'static GameData {
    static D: OnceLock<GameData> = OnceLock::new();
    D.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("act1-game-{}", std::process::id()));
        let i = install::build(&dir, &act1::act1()).unwrap_or_else(|e| panic!("{e}"));
        GameData::from_install(&i).unwrap_or_else(|e| panic!("{e}"))
    })
}

fn setup() -> Setup {
    Setup {
        creation: ActCreation::Full,
        init_seed: INIT,
        town: TOWN,
        game_seed: GAME_SEED,
        class: CLASS,
        known_waypoints: Vec::new(),
    }
}

#[test]
fn placement_matches_the_spec_vector() {
    let d = data();
    let (data, types) = d.level_types();
    let w = d
        .drlg_world(data, &types, ActCreation::Full, INIT, TOWN)
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        types.borrow().errors.is_empty(),
        "{:?}",
        types.borrow().errors
    );
    let dr = w.dungeon.acts[0].as_ref().expect("act 0");
    assert_eq!(dr.start_seed, START);
    // Only the Black Marsh draw advanced the DRLG seed.
    assert_eq!(dr.seed, Seed::new(1_406_222_081, 1_674_353_446));
    let mut order: Vec<u32> = dr
        .level_list()
        .into_iter()
        .map(|l| dr.level(l).id)
        .collect();
    order.reverse();
    // The placer rows, then the neighbour-entry walk over 1..17
    // allocates 8..16 (the reference allocation order).
    assert_eq!(
        order,
        [4, 3, 2, 1, 17, 39, 26, 7, 6, 27, 5, 8, 9, 10, 11, 12, 13, 14, 15, 16]
    );
    let rect = |id| dr.level(dr.find_level(id).unwrap()).rect;
    assert_eq!(rect(4), TileRect::new(1000, 1000, 80, 80));
    assert_eq!(rect(3), TileRect::new(920, 984, 80, 80));
    assert_eq!(rect(2), TileRect::new(904, 1064, 56, 96));
    assert_eq!(rect(1), TileRect::new(960, 1112, 56, 40));
    assert_eq!(rect(17), TileRect::new(880, 968, 40, 48));
    for (id, x, y) in [
        (39, 5000, 1148),
        (26, 3000, 1000),
        (7, 3000, 1018),
        (6, 2920, 1002),
        (5, 2904, 1082),
    ] {
        let r = rect(id);
        assert_eq!((r.x, r.y), (x, y), "level {id}");
    }
    // The town generated at creation: 7 × 5
    // preset rooms.
    let town = dr.find_level(TOWN).unwrap();
    assert_eq!(dr.level_rooms(town).len(), 35);
}

/// Every outdoor and preset level of the chain generates and streams
/// on the synthetic files (the Act I generator).
#[test]
fn every_chain_level_generates() {
    let d = data();
    let (data, types) = d.level_types();
    let mut w = d
        .drlg_world(data, &types, ActCreation::Full, INIT, TOWN)
        .unwrap_or_else(|e| panic!("{e}"));
    let mut game = Game::new();
    game.lists.ensure_act(0).unwrap();
    for &(id, drlg_type, ..) in &act1::PLACER_LEVELS {
        let (rooms, outdoor) = w
            .with_act(0, &mut game.lists, |dr, svc| {
                let l = dr.get_or_alloc_level(svc.data, svc.types, id)?;
                if dr.level_rooms(l).is_empty() {
                    dr.generate_level(svc.data, svc.types, l)?;
                }
                let rooms = dr.level_rooms(l);
                for &r in &rooms {
                    dr.stream_room(svc, r)?;
                }
                let outdoor = rooms
                    .iter()
                    .filter(|&&r| dr.room(r).kind != RoomKind::Preset)
                    .count();
                Ok::<_, d2_sim::drlg::DrlgError>((rooms.len(), outdoor))
            })
            .expect("act 0")
            .unwrap_or_else(|e| panic!("level {id}: {e:?}"));
        assert!(rooms > 0, "level {id}: no rooms");
        // Preset levels build only preset rooms; outdoor levels at least
        // one outdoor-grid room besides their preset cells.
        if drlg_type == 2 {
            assert_eq!(outdoor, 0, "level {id}");
        } else {
            assert!(outdoor > 0, "level {id}: no outdoor room");
        }
        assert!(
            types.borrow().errors.is_empty(),
            "level {id}: {:?}",
            types.borrow().errors
        );
    }
}

/// What one run leaves behind, compared across runs.
struct Run {
    digest: u64,
    state: String,
}

fn run() -> Run {
    let mut fx = Session::new(data(), &setup());
    let player = fx.player;
    assert_eq!(fx.unit_level(player), Some(TOWN), "joined in the town");
    let frame0 = fx.sim().game.frame;
    for _ in 0..IDLE_FRAMES {
        fx.frame();
    }
    assert_eq!(fx.sim().game.frame, frame0 + IDLE_FRAMES as i32);
    fx.assert_clean("idle");
    assert_eq!(fx.pos(), fx.start, "standing still");

    // Out of the town over its Blood Moor edge.
    let town = fx.level_rect(TOWN);
    let moor = fx.level_rect(BLOOD_MOOR);
    let p = fx.pos();
    let goals = border_goals(town, moor, p);
    fx.walk(
        &goals,
        |f| f.unit_level(f.player) == Some(BLOOD_MOOR),
        "Blood Moor",
    );
    fx.assert_clean("Blood Moor");

    // Invariants: Blood Moor generated by the outdoor generator, rooms
    // active within their level, units inside their rooms.
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
            let outdoor = rooms
                .iter()
                .filter(|&&r| d.room(r).kind != RoomKind::Preset)
                .count();
            levels.push((d.level(l).id, rooms.len(), outdoor, active));
        }
    }
    let moor_rooms = levels
        .iter()
        .find(|l| l.0 == BLOOD_MOOR)
        .expect("Blood Moor generated");
    assert!(
        moor_rooms.2 > 0,
        "Blood Moor has outdoor rooms: {moor_rooms:?}"
    );
    for &(id, n, _, active) in &levels {
        assert!(active <= n, "level {id}: {active} active of {n}");
        assert!(
            [TOWN, BLOOD_MOOR, COLD_PLAINS].contains(&id) || n > 0,
            "level {id}"
        );
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
    let seed = fx.host.game.events.action.hooks().game_seed;
    let state = format!(
        "frame {} seed {seed:?}\nlevels {levels:?}\nunits {units:?}\nunhandled {:?}\ntranscript {:?}",
        fx.host.game.game.frame, fx.host.game.unhandled, fx.transcript
    );
    println!(
        "in Blood Moor at {:?}, frame {}; levels {levels:?}; {} messages",
        fx.pos(),
        fx.host.game.game.frame,
        fx.transcript.len()
    );
    Run {
        digest: fnv(state.as_bytes()),
        state,
    }
}

#[test]
fn join_town_and_walk_into_blood_moor() {
    let a = run();
    let b = run();
    if a.digest != b.digest {
        let line = a
            .state
            .lines()
            .zip(b.state.lines())
            .position(|(x, y)| x != y);
        panic!("two runs differ (first differing line {line:?})");
    }
    println!("digest {:016x}", a.digest);
}

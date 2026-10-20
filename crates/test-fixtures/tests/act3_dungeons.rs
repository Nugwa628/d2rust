//! The Act III dungeons on synthetic data (task `q-a3-dungeons`): the
//! Act III-shaped set ([`test_fixtures::act3`]) → act 2 created with the
//! Kurast Docks town → every maze level (Spider Cave / Cavern, Swampy
//! Pit, Flayer Dungeon, the sewers, Durance of Hate 1 / 2) and preset
//! level (temples, Durance of Hate 3) generated and streamed by the real
//! generators. No `Covers:` claim: the data is made up, not the game's.

use std::path::PathBuf;
use std::sync::OnceLock;

use d2_sim::drlg::{Drlg, DrlgError, NoLevelTypes, RoomKind};
use d2_sim::game::Game;
use test_fixtures::act3::{self, MAZE_LEVELS, PRESET_LEVELS, TOWN};
use test_fixtures::game::{ActCreation, GameData};
use test_fixtures::host::{Session, Setup};
use test_fixtures::install;

const INIT: u32 = 644_409_375;
const ACT: u8 = 2;

fn data() -> &'static GameData {
    static D: OnceLock<GameData> = OnceLock::new();
    D.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("act3-game-{}", std::process::id()));
        let i = install::build(&dir, &act3::act3()).unwrap_or_else(|e| panic!("{e}"));
        GameData::from_install(&i).unwrap_or_else(|e| panic!("{e}"))
    })
}

/// Sub-tiles' logical rooms per 24-tile maze cell (3 × 3 rooms of 8).
const ROOMS_PER_CELL: usize = 9;

/// Act 2 with the town, a world over it, and the level `id` generated and
/// streamed: (rooms, rooms that are not preset rooms).
fn built(id: u32) -> (usize, usize) {
    let d = data();
    let (data, types) = d.level_types();
    let mut handle = types.clone();
    let mut dr = Drlg::create(ACT, INIT, 0, 0, false, &data, &mut NoLevelTypes)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let town = dr
        .get_or_alloc_level(&data, &mut handle, TOWN)
        .unwrap_or_else(|e| panic!("{e:?}"));
    dr.generate_level(&data, &mut handle, town)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let mut world = d2_sim::wiring::action::DrlgWorld {
        dungeon: Default::default(),
        data: data.clone(),
        tiles: Box::new(d.files.dt1.clone()),
        types: Box::new(types.clone()),
    };
    let mut game = Game::new();
    game.lists.ensure_act(ACT).unwrap();
    world.dungeon.acts[usize::from(ACT)] = Some(dr);
    let out = world
        .with_act(ACT, &mut game.lists, |dr, svc| {
            let l = dr.get_or_alloc_level(svc.data, svc.types, id)?;
            if dr.level_rooms(l).is_empty() {
                dr.generate_level(svc.data, svc.types, l)?;
            }
            let rooms = dr.level_rooms(l);
            for &r in &rooms {
                dr.stream_room(svc, r)?;
            }
            let other = rooms
                .iter()
                .filter(|&&r| dr.room(r).kind != RoomKind::Preset)
                .count();
            Ok::<_, DrlgError>((rooms.len(), other))
        })
        .expect("act 2")
        .unwrap_or_else(|e| panic!("level {id}: {e:?} {:?}", types.borrow().errors));
    assert!(
        types.borrow().errors.is_empty(),
        "level {id}: {:?}",
        types.borrow().errors
    );
    out
}

#[test]
fn every_act3_maze_level_generates_and_streams_as_preset_rooms() {
    for &(id, _) in &MAZE_LEVELS {
        let (rooms, other) = built(id);
        assert!(rooms >= 4 * ROOMS_PER_CELL, "level {id}: {rooms} rooms");
        assert_eq!(other, 0, "level {id}: a maze level has preset rooms only");
        assert_eq!(rooms % ROOMS_PER_CELL, 0, "level {id}: whole cells");
    }
}

/// The maze generator: the Spider levels are ring(2) only (four cells,
/// no grow, no stamps); Sewers 1 is ring(5) (16 cells) plus the two
/// sewer stamps; the Dungeon levels grow past the ring to their
/// `Rooms` target.
#[test]
fn the_act3_maze_builders_place_the_spec_cell_counts() {
    for id in [act3::SPIDER_CAVE, act3::SPIDER_CAVERN] {
        assert_eq!(built(id).0, 4 * ROOMS_PER_CELL, "level {id}");
    }
    assert_eq!(built(act3::SEWERS_1).0, 18 * ROOMS_PER_CELL, "Sewers 1");
    for id in [86, 87, 88, 89, 90, 91] {
        assert!(built(id).0 > 4 * ROOMS_PER_CELL, "Dungeon level {id} grows");
    }
}

#[test]
fn a_maze_level_builds_the_same_each_time() {
    for id in [act3::SPIDER_CAVERN, act3::DURANCE_2] {
        assert_eq!(built(id), built(id), "level {id}");
    }
}

#[test]
fn the_temples_and_durance_3_are_one_preset_room_levels() {
    for id in PRESET_LEVELS {
        let (rooms, _) = built(id);
        assert!(rooms > 0, "level {id}: no rooms");
    }
}

fn setup() -> Setup {
    Setup {
        creation: ActCreation::Full,
        init_seed: INIT,
        town: TOWN,
        game_seed: 1234,
        class: 3,
        known_waypoints: Vec::new(),
    }
}

#[test]
fn the_live_game_builds_every_dungeon_level_of_the_act() {
    let mut fx = Session::new_in_act(data(), &setup(), ACT);
    for &(id, _) in &act3::MAZE_LEVELS {
        let sim = fx.sim();
        let rooms = sim
            .events
            .action
            .hooks()
            .drlg
            .with_act(ACT, &mut sim.game.lists, |dr, svc| {
                let l = dr.get_or_alloc_level(svc.data, svc.types, id)?;
                // A neighbouring level's stamps may have built it already.
                if dr.level_rooms(l).is_empty() {
                    dr.generate_level(svc.data, svc.types, l)?;
                }
                let rooms = dr.level_rooms(l);
                for &r in &rooms {
                    dr.stream_room(svc, r)?;
                }
                Ok::<_, d2_sim::drlg::DrlgError>(rooms.len())
            })
            .expect("act 2 has a DRLG")
            .unwrap_or_else(|e| panic!("level {id}: {e:?}"));
        assert!(rooms >= 36, "level {id}: {rooms} rooms");
    }
    // The built levels sit in the same act as the town.
    for _ in 0..5 {
        fx.frame();
    }
    fx.assert_clean("dungeon levels built");
    for &(id, _) in &act3::MAZE_LEVELS {
        let r = fx.level_rect(id);
        assert!(r.w > 0 && r.h > 0, "level {id} has a rect");
    }
}

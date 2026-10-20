//! Integration tests of warp tiles on the game's live tables and MPQs
//! (`#[ignore]`, `D2_GAME_DIR`; the game-file tier of the level generator
//! and the wired host):
//!
//! 1. every Act I level generated on a fixed seed and every room
//!    streamed: the type-5 preset units the tile fill adds are
//!    the tile classes `expected_classes` lists per level, at the
//!    room-relative sub-tiles of `expected_places`, with
//!    no DRLG or level-type error;
//! 2. the wired single-player host: a player walks from the Rogue
//!    Encampment into the Blood Moor, on to the Den of Evil's cave mouth
//!    (its tile unit created by the room's first population), uses it
//!    (C→S 0x13, unit type 5) and stands in the Den of Evil; then uses
//!    the Den's way up and stands in the Blood Moor again.
//!
//! Expected values are the per-level tables below; the walk's success is the
//! test's goal (a failure prints the position). Unconfirmed until the first
//! local run.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use d2_data::bin;
use d2_formats::mpq::ArchiveSet;
use d2_server::seams::ResultCode;
use d2_sim::drlg::{DrlgRoomId, PresetUnit};
use d2_sim::units::UnitType;
use test_fixtures::game::{ActCreation, GameData};
use test_fixtures::host::{border_goals, cheb, Session, Setup};

fn live() -> &'static GameData {
    static L: OnceLock<GameData> = OnceLock::new();
    L.get_or_init(|| {
        let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
        let set = ArchiveSet::open_dir(dir).expect("archives open");
        let bins = bin::load(&set, bin::DEFAULT_LANGUAGE).expect("live .bin set loads");
        GameData::load(bins, &set).unwrap_or_else(|e| panic!("game data: {e}"))
    })
}

/// The Act I creation DRLG init seed.
const INIT: u32 = 644_409_375;
const TOWN: u32 = 1;
const BLOOD_MOOR: u32 = 2;
const DEN_OF_EVIL: u32 = 8;
const GAME_SEED: u32 = 1234;
const SORCERESS: u32 = 1;

fn session() -> Session {
    Session::new(
        live(),
        &Setup {
            creation: ActCreation::Full,
            init_seed: INIT,
            town: TOWN,
            game_seed: GAME_SEED,
            class: SORCERESS,
            known_waypoints: Vec::new(),
        },
    )
}

/// A tile preset: level id, class, sub-tile from its room's origin, the
/// room's tile origin from the level's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Place {
    level: u32,
    class: u32,
    at: (i32, i32),
    room: (i32, i32),
}

/// Every Act I level generated and every room streamed; the type-5
/// presets of each level, and the errors met.
fn act1_tile_presets(fx: &mut Session) -> (Vec<Place>, Vec<String>) {
    let ids: Vec<u32> = (1..=39).collect();
    let s = fx.sim();
    let (game, events) = (&mut s.game, &mut s.events);
    let mut errors = Vec::new();
    let places = events
        .action
        .hooks()
        .drlg
        .with_act(0, &mut game.lists, |d, svc| {
            let mut out = Vec::new();
            for &id in &ids {
                let l = match d.get_or_alloc_level(svc.data, svc.types, id) {
                    Ok(l) => l,
                    Err(e) => {
                        errors.push(format!("level {id}: allocate {e:?}"));
                        continue;
                    }
                };
                if d.level_rooms(l).is_empty() {
                    if let Err(e) = d.generate_level(svc.data, svc.types, l) {
                        errors.push(format!("level {id}: generate {e:?}"));
                        continue;
                    }
                }
                let origin = d.level(l).rect;
                let rooms: Vec<DrlgRoomId> = d.level_rooms(l);
                for r in rooms {
                    if let Err(e) = d.stream_room(svc, r) {
                        errors.push(format!("level {id}: stream {e:?}"));
                        continue;
                    }
                    let rect = d.room(r).rect;
                    for u in svc.types.preset_units(d, r) {
                        let PresetUnit {
                            unit_type,
                            class,
                            x,
                            y,
                        } = u;
                        if unit_type == 5 {
                            out.push(Place {
                                level: id,
                                class,
                                at: (x, y),
                                room: (rect.x - origin.x, rect.y - origin.y),
                            });
                        }
                    }
                }
            }
            out
        })
        .expect("act 0 has a DRLG");
    errors.extend(fx.sim().events.errors());
    (places, errors)
}

/// The tile classes of each Act I level (one
/// tile per exit cell; a field's cave mouth is one of `Id` 0..3).
fn expected_classes(level: u32) -> Option<Vec<Vec<u32>>> {
    let one = |c: &[u32]| Some(vec![c.to_vec()]);
    let mouths = |extra: &[u32]| {
        Some(
            (0..4)
                .map(|m| {
                    let mut v = vec![m];
                    v.extend_from_slice(extra);
                    v.sort();
                    v
                })
                .collect(),
        )
    };
    match level {
        1 | 26 | 27 | 38 | 39 => one(&[]),
        2 | 3 => Some(vec![vec![2], vec![3]]),
        4 | 5 | 7 => mouths(&[]),
        6 => mouths(&[10]),
        17 => one(&[6, 7]),
        8 | 13..=16 => one(&[4]),
        9 | 11 | 12 => one(&[4, 5]),
        10 => one(&[4, 4, 5]),
        18 | 19 | 25 => one(&[8]),
        20 => one(&[11, 12]),
        21..=24 => one(&[8, 9]),
        28 | 32 => one(&[14]),
        29 | 30 => one(&[13, 14]),
        31 => one(&[13, 13]),
        33 => one(&[15]),
        34 => one(&[16, 18]),
        35 | 36 => one(&[17, 18]),
        37 => one(&[17]),
        _ => None,
    }
}

/// An allowed place: sub-tile from the room, the room's origin in the
/// level when fixed.
type Allowed = ((i32, i32), Option<(i32, i32)>);

/// The room-relative sub-tile of a tile, where the table below fixes
/// it: the allowed (sub-tile, room
/// origin in the level) pairs; `None` for any room.
fn expected_places(level: u32, class: u32) -> Option<Vec<Allowed>> {
    let any = |v: &[(i32, i32)]| Some(v.iter().map(|&p| (p, None)).collect());
    match (level, class) {
        // DenEnt / DenEnt2.
        (2, 2) => any(&[(14, 21)]),
        (2, 3) => any(&[(26, 19)]),
        // CaveDr1 / CaveDr2.
        (3..=7, 2) => any(&[(14, 21)]),
        (3..=7, 3) => any(&[(16, 19)]),
        // clfcave / clfcave2.
        // clfcave: the exit cell (5, 2) would give (26, 9), but that cell
        // has sub 1 (a wall warp tile with no unit); the file's second
        // exit cell (5, 3), sub 0, places it at (5·5 + 1, 5·3 − 1)
        // (`act1_mouth_exit_cells` prints both).
        (4..=7, 0) => any(&[(26, 14)]),
        (4..=7, 1) => any(&[(33, 26)]),
        // Tower1.
        (6, 10) => any(&[(11, 15)]),
        // gravey.ds1: rooms (8, 24) / (8, 0) of the map, which the
        // outdoor placer stamps anywhere in the level.
        (17, 6) => any(&[(25, 15)]),
        (17, 7) => any(&[(20, 32)]),
        // Cave Prev W/E/S/N; CaveRoom2..5.
        (8..=12, 4) => any(&[(17, 30), (37, 20), (17, 20), (22, 30)]),
        (13 | 16, 4) => Some(vec![((22, 30), Some((0, 0)))]),
        (14, 4) => Some(vec![((12, 10), Some((8, 8)))]),
        (15, 4) => Some(vec![((12, 10), Some((16, 8)))]),
        // Cave Down.
        (9..=12, 5) => any(&[(31, 13), (26, 23), (11, 8), (11, 33), (6, 33), (16, 13)]),
        // Tower2.
        (20, 11) => Some(vec![((4, 3), Some((0, 0)))]),
        (20, 12) => Some(vec![((2, 13), Some((0, 0)))]),
        // CryptCountess1 / 2.
        (25, 8) => Some(vec![((15, 21), Some((24, 8))), ((30, 6), Some((8, 24)))]),
        // Cat_Court, Cathy3, Andy3.
        (32, 14) => Some(vec![((9, 18), Some((0, 0)))]),
        (33, 15) => Some(vec![((28, 11), Some((16, 8)))]),
        (37, 17) => Some(vec![((15, 31), Some((16, 16)))]),
        _ => None,
    }
}

/// The exit cells (wall layers, orientation 10 / 11) of the outdoor cave
/// mouths' DS1 files: (x, y,
/// orientation, style, sub, hidden) per file of lvlprest Defs 24, 25,
/// 51, 52, 163.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn act1_mouth_exit_cells() {
    let gd = live();
    let mut got = Vec::new();
    for def in [24u32, 25, 51, 52, 163] {
        let row = gd.level.preset.def(def).expect("lvlprest row");
        let n = usize::try_from(row.files).unwrap_or(0).max(1);
        for f in row.file.iter().take(n).filter(|f| !f.is_empty()) {
            let input = gd.files.ds1.0.get(f).expect("DS1 loaded");
            let stride = input.width as usize + 1;
            let mut cells = Vec::new();
            for (walls, orients) in input.walls.iter().zip(&input.orientations) {
                for (i, (&v, &o)) in walls.iter().zip(orients).enumerate() {
                    if o == 10 || o == 11 {
                        cells.push((
                            i % stride,
                            i / stride,
                            o,
                            (v >> 20) & 0x3F,
                            (v >> 8) & 0xFF,
                            v >> 31,
                        ));
                    }
                }
            }
            println!("def {def} {}: {cells:?}", String::from_utf8_lossy(f));
            got.push((def, cells));
        }
    }
    let first = |def: u32| {
        got.iter()
            .find(|g| g.0 == def)
            .and_then(|g| g.1.first().copied())
    };
    // Expected exit cells: clfcave (5, 2) style 3; clfcave2 (6, 5) style 4; CaveDr1
    // (3, 4) style 5; DenEnt (3, 4) style 5; Tower1 (3, 3) style 2.
    let pos = |c: Option<(usize, usize, u32, u32, u32, u32)>| c.map(|c| (c.0, c.1, c.3));
    assert_eq!(pos(first(25)), Some((5, 2, 3)), "clfcave");
    assert_eq!(pos(first(24)), Some((6, 5, 4)), "clfcave2");
    assert_eq!(pos(first(51)), Some((3, 4, 5)), "CaveDr1");
    assert_eq!(pos(first(52)), Some((3, 4, 5)), "DenEnt");
    assert_eq!(pos(first(163)), Some((3, 3, 2)), "Tower1");
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn act1_warp_tile_places() {
    let mut fx = session();
    let (places, errors) = act1_tile_presets(&mut fx);
    assert_eq!(errors, Vec::<String>::new(), "DRLG / level-type errors");
    let mut by_level: BTreeMap<u32, Vec<Place>> = BTreeMap::new();
    for p in &places {
        by_level.entry(p.level).or_default().push(*p);
    }
    let mut failures = Vec::new();
    for level in 1..=39 {
        let got = by_level.get(&level).cloned().unwrap_or_default();
        let mut classes: Vec<u32> = got.iter().map(|p| p.class).collect();
        classes.sort();
        println!("level {level}: {got:?}");
        if let Some(want) = expected_classes(level) {
            if !want.contains(&classes) {
                failures.push(format!(
                    "level {level}: classes {classes:?}, want one of {want:?}"
                ));
            }
        }
        for p in &got {
            if let Some(allowed) = expected_places(level, p.class) {
                let ok = allowed
                    .iter()
                    .any(|&(at, room)| at == p.at && room.is_none_or(|r| r == p.room));
                if !ok {
                    failures.push(format!("level {level}: {p:?} not in {allowed:?}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The first tile unit of one of `classes` in the game and its sub-tile.
fn tile_unit(fx: &mut Session, classes: &[u32]) -> Option<(u32, u32, (i32, i32))> {
    let tiles = fx.sim().game.lists.units_of_type(UnitType::Tile);
    for u in tiles {
        let class = fx.sim().events.action.sys.units.get(u)?.class;
        if classes.contains(&class) {
            let guid = fx.sim().game.lists.unit(u)?.guid;
            let at = fx.sim().events.action.hooks().path_position(u);
            return Some((guid, class, at));
        }
    }
    None
}

/// The absolute sub-tile of the level's type-5 preset of one of
/// `classes` (its room streamed on the way).
fn tile_preset(fx: &mut Session, level: u32, classes: &[u32]) -> Option<(i32, i32)> {
    let s = fx.sim();
    let (game, events) = (&mut s.game, &mut s.events);
    events
        .action
        .hooks()
        .drlg
        .with_act(0, &mut game.lists, |d, svc| {
            let l = d.find_level(level)?;
            for r in d.level_rooms(l) {
                let rect = d.room(r).rect;
                for u in svc.types.preset_units(d, r) {
                    if u.unit_type == 5 && classes.contains(&u.class) {
                        return Some((rect.x * 5 + u.x, rect.y * 5 + u.y));
                    }
                }
            }
            None
        })
        .flatten()
}

/// C→S 0x13 on the tile unit with `guid`.
fn use_tile(fx: &mut Session, guid: u32) -> ResultCode {
    let mut m = vec![0x13];
    m.extend_from_slice(&5u32.to_le_bytes());
    m.extend_from_slice(&guid.to_le_bytes());
    fx.send(&m).0
}

/// Legs toward `goal` until a tile unit of `classes` exists within 10
/// sub-tiles of the player.
fn walk_to_tile(fx: &mut Session, goal: (i32, i32), classes: &'static [u32], what: &str) {
    // Straight legs get stuck on the mouth's cliff: detour points on two
    // rings around the tile (nearest first), each followed by the tile.
    let mut ring: Vec<(i32, i32)> = Vec::new();
    for r in [20, 45] {
        for (dx, dy) in [
            (0, 1),
            (1, 1),
            (1, 0),
            (1, -1),
            (0, -1),
            (-1, -1),
            (-1, 0),
            (-1, 1),
        ] {
            ring.push((goal.0 + dx * r, goal.1 + dy * r));
        }
    }
    let me = fx.pos();
    ring.sort_by_key(|&g| cheb(g, me));
    let mut goals = vec![goal];
    for g in ring {
        goals.push(g);
        goals.push(goal);
    }
    let near = tile_unit(fx, classes);
    println!("before the walk: tile unit {near:?}");
    fx.walk(
        &goals,
        move |f| {
            let me = f.pos();
            tile_unit(f, classes).is_some_and(|(_, _, at)| cheb(at, me) <= 10)
        },
        what,
    );
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn walk_from_the_rogue_encampment_into_the_den_of_evil() {
    let mut fx = session();
    for _ in 0..20 {
        fx.frame();
    }
    // Into the Blood Moor.
    let town = fx.level_rect(TOWN);
    let moor = fx.level_rect(BLOOD_MOOR);
    let p = fx.pos();
    let goals = border_goals(town, moor, p);
    fx.walk(
        &goals,
        |f| f.unit_level(f.player) == Some(BLOOD_MOOR),
        "Blood Moor",
    );
    // The cave mouth: the Blood Moor's tile preset (`Id` 2 or 3).
    const MOUTH: &[u32] = &[2, 3];
    let mouth = tile_preset(&mut fx, BLOOD_MOOR, MOUTH);
    let mouth = mouth.unwrap_or_else(|| {
        // Its room may not be generated into tiles yet: the level is, so
        // stream the rooms (the preset list fills at the tile build).
        let (places, errors) = act1_tile_presets(&mut fx);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(places.iter().any(|p| p.level == BLOOD_MOOR));
        tile_preset(&mut fx, BLOOD_MOOR, MOUTH).expect("the Den's cave mouth")
    });
    println!("Blood Moor at {:?}; cave mouth at {mouth:?}", fx.pos());
    walk_to_tile(&mut fx, mouth, MOUTH, "the Den of Evil's cave mouth");
    let (guid, class, at) = tile_unit(&mut fx, MOUTH).expect("the mouth's tile unit");
    println!("tile class {class} at {at:?}, player at {:?}", fx.pos());
    assert_eq!(use_tile(&mut fx, guid), ResultCode::Done);
    fx.frame();
    fx.assert_clean("warp into the Den");
    assert_eq!(
        fx.unit_level(fx.player),
        Some(DEN_OF_EVIL),
        "the player stands in the Den of Evil (path-placement.md §12.2)"
    );
    // The way up: the Den's tile of `Id` 4, next to the arrival.
    for _ in 0..5 {
        fx.frame();
    }
    let (up, class, at) = tile_unit(&mut fx, &[4]).expect("the Den's way up");
    println!(
        "in the Den at {:?}; way up class {class} at {at:?}",
        fx.pos()
    );
    assert!(cheb(at, fx.pos()) <= 15, "arrival next to the way up");
    assert_eq!(use_tile(&mut fx, up), ResultCode::Done);
    fx.frame();
    fx.assert_clean("warp back");
    assert_eq!(fx.unit_level(fx.player), Some(BLOOD_MOOR));
}

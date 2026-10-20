//! Act III on synthetic data, in CI (task `q-a3-fields`): the Act III-shaped
//! set ([`test_fixtures::act3`]) → `GameData` → act 2 created by the real
//! jungle placer and Kurast chain → every outdoor level (Spider Forest,
//! Great Marsh, Flayer Jungle, Lower Kurast, Kurast Bazaar, Upper Kurast,
//! Kurast Causeway, Travincal) generated and streamed. No `Covers:` claim:
//! the data is made up, not the game's.

use std::path::PathBuf;
use std::sync::OnceLock;

use d2_sim::drlg::{room_flags, Drlg, DrlgError, RoomKind};
use d2_sim::game::Game;
use test_fixtures::act3::{self, OUTDOOR, TOWN};
use test_fixtures::game::GameData;
use test_fixtures::install;

const INIT: u32 = 644_409_375;

fn data() -> &'static GameData {
    static D: OnceLock<GameData> = OnceLock::new();
    D.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("act3-game-{}", std::process::id()));
        let i = install::build(&dir, &act3::act3()).unwrap_or_else(|e| panic!("{e}"));
        GameData::from_install(&i).unwrap_or_else(|e| panic!("{e}"))
    })
}

#[test]
fn the_jungle_placer_and_kurast_chain_link_the_levels() {
    let d = data();
    let (data, types) = d.level_types();
    let dr = Drlg::create(2, INIT, 0, TOWN, false, &data, &mut types.clone())
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(
        types.borrow().errors.is_empty(),
        "{:?}",
        types.borrow().errors
    );
    let rect = |id| dr.level(dr.find_level(id).unwrap()).rect;
    for id in 75..=83 {
        assert!(dr.find_level(id).is_some(), "level {id} allocated");
    }
    let r = rect(75);
    assert_eq!((r.x, r.y, r.w, r.h), (1000, 1000, 64, 48));
    // The three jungles are 64 × 192; the highest (largest y) is 76 and
    // sits against the docks' north edge.
    for id in 76..=78 {
        assert_eq!((rect(id).w, rect(id).h), (64, 192), "level {id}");
    }
    assert!(rect(76).y >= rect(77).y && rect(77).y >= rect(78).y);
    assert_eq!(rect(76).y + rect(76).h, 1000, "76 touches the docks");
    // The Kurast chain climbs north from level 78, centred on it.
    let j = rect(78);
    let mut y = 0;
    for (id, (w, h)) in [
        (79, (80, 64)),
        (80, (80, 64)),
        (81, (80, 64)),
        (82, (48, 16)),
        (83, (64, 64)),
    ] {
        y -= h;
        let r = rect(id);
        assert_eq!((r.w, r.h), (w, h), "level {id}");
        assert_eq!((r.x, r.y), (j.x + j.w / 2 - w / 2, j.y + y), "level {id}");
    }
}

#[test]
fn every_act3_level_generates_and_streams() {
    let d = data();
    let (data, types) = d.level_types();
    let dr = Drlg::create(2, INIT, 0, TOWN, false, &data, &mut types.clone())
        .unwrap_or_else(|e| panic!("{e:?}"));
    let mut world = d2_sim::wiring::action::DrlgWorld {
        dungeon: Default::default(),
        data: data.clone(),
        tiles: Box::new(d.files.dt1.clone()),
        types: Box::new(types.clone()),
    };
    let mut game = Game::new();
    game.lists.ensure_act(2).unwrap();
    world.dungeon.acts[2] = Some(dr);
    for id in std::iter::once(TOWN).chain(OUTDOOR) {
        let (rooms, outdoor, wp) = world
            .with_act(2, &mut game.lists, |dr, svc| {
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
                let wp = rooms
                    .iter()
                    .filter(|&&r| dr.room(r).flags & room_flags::ANY_WAYPOINT != 0)
                    .count();
                Ok::<_, DrlgError>((rooms.len(), outdoor, wp))
            })
            .expect("act 2")
            .unwrap_or_else(|e| panic!("level {id}: {e:?}"));
        assert!(rooms > 0, "level {id}: no rooms");
        // Jungles: 3 × 3 × 32... the reference 192
        // rooms: every 8 × 8 cell of a 64 × 192 level is a room.
        if (76..=78).contains(&id) {
            assert_eq!(rooms, 192, "level {id} room count");
        }
        if id == 83 {
            assert_eq!((rooms, outdoor), (64, 0), "Travincal: 64 preset rooms");
        }
        // No waypoint placer runs in Act III.
        assert_eq!(wp, 0, "level {id} waypoint rooms");
        assert!(
            types.borrow().errors.is_empty(),
            "level {id}: {:?}",
            types.borrow().errors
        );
    }
}

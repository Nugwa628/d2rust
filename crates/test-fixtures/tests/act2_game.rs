//! Act II on synthetic data, in CI (task `q-a2-fields`): the Act II-shaped
//! set ([`test_fixtures::act2`]) → `GameData` → act 1 created by the real
//! placer → every outdoor level of the desert chain (Rocky Waste, Dry
//! Hills, Far Oasis, Lost City, Valley of Snakes, Canyon of the Magi)
//! generated and streamed by the Act II generator, the waypoint rooms of
//! Dry Hills / Far Oasis / Lost City flagged. No `Covers:` claim: the data
//! is made up, not the game's.

use std::path::PathBuf;
use std::sync::OnceLock;

use d2_sim::drlg::{room_flags, Drlg, DrlgError, RoomKind};
use d2_sim::game::Game;
use test_fixtures::act2::{self, TOWN, WAYPOINT_LEVELS};
use test_fixtures::game::GameData;
use test_fixtures::install;

const INIT: u32 = 644_409_375;

fn data() -> &'static GameData {
    static D: OnceLock<GameData> = OnceLock::new();
    D.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("act2-game-{}", std::process::id()));
        let i = install::build(&dir, &act2::act2()).unwrap_or_else(|e| panic!("{e}"));
        GameData::from_install(&i).unwrap_or_else(|e| panic!("{e}"))
    })
}

#[test]
fn the_act_placer_links_the_desert_chain() {
    let d = data();
    let (data, types) = d.level_types();
    let dr = Drlg::create(1, INIT, 0, TOWN, false, &data, &mut types.clone())
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(
        types.borrow().errors.is_empty(),
        "{:?}",
        types.borrow().errors
    );
    for &(id, ..) in &act2::PLACER_LEVELS {
        assert!(dr.find_level(id).is_some(), "level {id} allocated");
    }
    // Absolute rows (Def): Lut Gholein and the Canyon of the Magi.
    let rect = |id| dr.level(dr.find_level(id).unwrap()).rect;
    assert_eq!(
        (rect(40).x, rect(40).y, rect(40).w, rect(40).h),
        (1000, 1000, 56, 56)
    );
    assert_eq!((rect(46).x, rect(46).y), (2500, 1000));
    // Rocky Waste sits against the town, Valley of Snakes is 32 × 32.
    assert_eq!((rect(45).w, rect(45).h), (32, 32));
}

#[test]
fn every_desert_level_generates_and_streams() {
    let d = data();
    let (data, types) = d.level_types();
    let dr = Drlg::create(1, INIT, 0, TOWN, false, &data, &mut types.clone())
        .unwrap_or_else(|e| panic!("{e:?}"));
    let mut world = d2_sim::wiring::action::DrlgWorld {
        dungeon: Default::default(),
        data: data.clone(),
        tiles: Box::new(d.files.dt1.clone()),
        types: Box::new(types.clone()),
    };
    let mut game = Game::new();
    game.lists.ensure_act(1).unwrap();
    world.dungeon.acts[1] = Some(dr);
    for &(id, drlg_type, ..) in &act2::PLACER_LEVELS {
        let (rooms, outdoor, wp) = world
            .with_act(1, &mut game.lists, |dr, svc| {
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
            .expect("act 1")
            .unwrap_or_else(|e| panic!("level {id}: {e:?}"));
        assert!(rooms > 0, "level {id}: no rooms");
        if drlg_type == 2 {
            assert_eq!(outdoor, 0, "level {id}");
        } else {
            assert!(outdoor > 0, "level {id}: no outdoor room");
        }
        assert_eq!(
            wp,
            usize::from(WAYPOINT_LEVELS.contains(&id)),
            "level {id} waypoint rooms"
        );
        assert!(
            types.borrow().errors.is_empty(),
            "level {id}: {:?}",
            types.borrow().errors
        );
    }
}

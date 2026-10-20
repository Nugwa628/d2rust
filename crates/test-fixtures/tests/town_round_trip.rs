//! The Rogue Encampment after a waypoint round trip (q-fix-pc1-proto-items
//! playability blocker): the town's preset objects (its waypoint, the
//! stash, …) are created by the preset pass, which
//! sets unit flags 0x3000000 on every unit it creates; flag 0x2000000
//! makes them `S` of the compress
//! when the town's rooms are freed, so they are stored
//! and the rooms' next population re-creates them (same
//! class, place and mode, new GUIDs).
//!
//! The live run (`#[ignore]`, `D2_GAME_DIR`): join in the town, take the
//! waypoint to the Cold Plains, idle until tick step 9 has freed every
//! town room, take the Cold Plains waypoint back, compare the town's
//! objects with the ones at the start.

use std::sync::OnceLock;

use d2_data::bin;
use d2_data::tables::Objects;
use d2_formats::mpq::ArchiveSet;
use d2_server::seams::ResultCode;
use d2_sim::units::{UnitId, UnitType};
use test_fixtures::game::{ActCreation, GameData};
use test_fixtures::host::{Session, Setup, WAYPOINT_OPERATE};

fn live() -> &'static GameData {
    static L: OnceLock<GameData> = OnceLock::new();
    L.get_or_init(|| {
        let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
        let set = ArchiveSet::open_dir(dir).expect("archives open");
        let bins = bin::load(&set, bin::DEFAULT_LANGUAGE).expect("live .bin set loads");
        GameData::load(bins, &set).unwrap_or_else(|e| panic!("game data: {e}"))
    })
}

/// The reference Act I creation's DRLG init seed; the rest are fixture choices.
const INIT: u32 = 644_409_375;
const TOWN: u32 = 1;
const COLD_PLAINS: u32 = 3;
const GAME_SEED: u32 = 1234;
const CLASS: u32 = 0;
/// Frames before the snapshot (the town's first populations), and the
/// idle budget away: a room is freed after 11 passes of 12 frames with
/// no client.
const SETTLE_FRAMES: usize = 50;
const AWAY_FRAMES: usize = 400;

/// (class, position, mode, GUID, unit flags) of an object.
type Obj = (u32, (i32, i32), u32, u32, u32);

/// Every object in `level`, sorted by class and position.
fn objects_in(fx: &mut Session, level: u32) -> Vec<Obj> {
    let units: Vec<UnitId> = fx.sim().game.lists.units_of_type(UnitType::Object);
    let mut out = Vec::new();
    for u in units {
        if fx.unit_level(u) != Some(level) {
            continue;
        }
        let at = fx.sim().events.action.hooks().path_position(u);
        let guid = fx.sim().game.lists.unit(u).unwrap().guid;
        let r = fx.sim().events.action.sys.units.get(u).unwrap();
        out.push((r.class, at, r.mode, guid, r.flags));
    }
    out.sort();
    out
}

fn waypoint(d: &GameData, fx: &mut Session, level: u32) -> u32 {
    let rows: Vec<Objects> = d.rows().expect("objects");
    objects_in(fx, level)
        .into_iter()
        .find(|o| {
            rows.get(o.0 as usize)
                .is_some_and(|r| r.operatefn == WAYPOINT_OPERATE)
        })
        .unwrap_or_else(|| panic!("a waypoint in level {level}"))
        .3
}

fn take_waypoint(fx: &mut Session, guid: u32, level: u32) {
    let mut m = vec![0x49];
    m.extend_from_slice(&guid.to_le_bytes());
    m.extend_from_slice(&(level as u16).to_le_bytes());
    m.extend_from_slice(&[0, 0]);
    let (code, _) = fx.send(&m);
    assert_eq!(code, ResultCode::Done, "0x49 to level {level}");
    assert_eq!(fx.unit_level(fx.player), Some(level), "arrived in {level}");
}

fn round_trip(d: &GameData) {
    let mut fx = Session::new(
        d,
        &Setup {
            creation: ActCreation::Full,
            init_seed: INIT,
            town: TOWN,
            game_seed: GAME_SEED,
            class: CLASS,
            known_waypoints: vec![COLD_PLAINS],
        },
    );
    fx.sim().events.action.hooks().enable_inactive_store();
    for _ in 0..SETTLE_FRAMES {
        fx.frame();
    }
    let before = objects_in(&mut fx, TOWN);
    assert!(!before.is_empty(), "the town has objects");
    let wp = waypoint(d, &mut fx, TOWN);
    take_waypoint(&mut fx, wp, COLD_PLAINS);
    for _ in 0..AWAY_FRAMES {
        fx.frame();
    }
    assert_eq!(
        objects_in(&mut fx, TOWN),
        Vec::new(),
        "every town room freed while away"
    );
    let back = waypoint(d, &mut fx, COLD_PLAINS);
    take_waypoint(&mut fx, back, TOWN);
    for _ in 0..SETTLE_FRAMES {
        fx.frame();
    }
    fx.assert_clean("round trip");
    let after = objects_in(&mut fx, TOWN);
    // Same class and place, the stored mode (a mode-1
    // object without `CycleAnim1` and with `Mode2` is stored in mode 2:
    // the waypoint), new GUIDs, unit flags 0x3000000
    // (set by the preset pass).
    let key = |v: &[Obj]| v.iter().map(|o| (o.0, o.1)).collect::<Vec<_>>();
    assert_eq!(key(&after), key(&before), "the town's objects are back");
    for (a, b) in after.iter().zip(&before) {
        assert!(a.2 == b.2 || (b.2, a.2) == (1, 2), "mode {b:?} → {a:?}");
        assert_ne!(a.3, b.3, "a new GUID {a:?}");
        assert_eq!(a.4 & 0x300_0000, 0x300_0000, "{a:?}");
    }
    // The waypoint can be taken again.
    let wp = waypoint(d, &mut fx, TOWN);
    take_waypoint(&mut fx, wp, COLD_PLAINS);
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn the_town_keeps_its_objects_after_a_waypoint_round_trip() {
    round_trip(live());
}

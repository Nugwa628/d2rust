//! Tests for waypoints ↔ DRLG levels: the object and player facts come from the
//! unit lists and the level of their DRLG room, records live per player,
//! ENDANIM goes through the real timer queue, the spawn search runs on
//! the DRLG.

use d2_data::tables::Objects;

use super::*;
use crate::tick::events::event;
use crate::world::waypoints::{ArrivalList, ArrivalNode, WaypointData, WaypointWorld, NO_WAYPOINT};

/// Waypoint index of the fixture's level.
const WP: u8 = 1;

fn data() -> WaypointData {
    let mut levels = vec![blank::<Levels>(); 150];
    for l in &mut levels {
        l.waypoint = NO_WAYPOINT;
    }
    levels[LEVEL as usize].waypoint = WP;
    let mut o: Objects = blank();
    o.operatefn = 23;
    o.initfn = 17;
    o.framecnt1 = 10 << 8;
    WaypointData::new(&levels, &[o])
}

/// A waypoint object (class 0, mode 0) and a player in room A.
fn setup(fx: &mut Fx) -> (UnitId, UnitId) {
    let a = fx.a;
    let o = fx.spawn(UnitType::Object, 0, a, 20, 20);
    fx.sim.sys.units.get_mut(o).unwrap().mode = 0;
    let p = fx.spawn(UnitType::Player, 0, a, 22, 20);
    (o, p)
}

#[test]
fn operate_sets_the_level_bit_and_schedules_endanim() {
    // The operated waypoint's index is set in the player's record
    // of the game difficulty; mode 0 → mode 1 and ENDANIM at
    // f + FrameCnt1 / 256 + 1.
    let mut fx = Fx::new();
    let (o, p) = setup(&mut fx);
    let wd = data();
    fx.game.frame = 7;
    let guid = fx.game.lists.unit(o).unwrap().guid;
    let r = fx.sim.waypoints(&mut fx.game, |w| {
        let (u, facts) = w.object(guid).expect("object");
        assert_eq!(u, o);
        assert_eq!(facts.level, Some(LEVEL));
        assert_eq!((facts.x, facts.y), (20, 20));
        wd.operate(w, o, &facts, p)
    });
    assert_eq!(r, Ok(1));
    assert!(fx.sim.hooks().waypoints[&p].0[0]
        .test(u32::from(WP))
        .unwrap());
    assert!(fx.timers(o).contains(&(event::END_ANIM, 7 + 10 + 1)));
    assert_eq!(fx.sim.hooks().x.log, [format!("object mode {} 1", o.0)]);
    fx.assert_clean();
}

#[test]
fn init_consumes_an_arrival_in_the_object_room() {
    // An arrival node in the object's room (by room rectangle) → mode
    // 1, ENDANIM at f + FrameCnt1 / 256; the list is emptied.
    let mut fx = Fx::new();
    let (o, _) = setup(&mut fx);
    let wd = data();
    let guid = fx.game.lists.unit(o).unwrap().guid;
    let mut arrivals = ArrivalList(vec![ArrivalNode {
        room: None,
        x: 5,
        y: 5,
    }]);
    fx.sim.waypoints(&mut fx.game, |w| {
        let (_, facts) = w.object(guid).unwrap();
        assert_eq!(w.room_rect(facts.room.unwrap()).width, 40);
        wd.init_object(w, &mut arrivals, o, &facts);
    });
    assert!(arrivals.0.is_empty());
    assert!(fx.timers(o).contains(&(event::END_ANIM, 10)));
    fx.assert_clean();
}

#[test]
fn spawn_search_returns_an_active_room_of_the_level() {
    // With `Position` = 0, no waypoint or warp room and a
    // level rect that holds no room at its centre: `roll(room count)` on
    // the level seed picks one of the level's rooms, streamed.
    let mut fx = Fx::new();
    let (a, b) = (fx.a, fx.b);
    let r = fx.sim.waypoints(&mut fx.game, |w| w.spawn_room(LEVEL, 0));
    assert!(r == Some(a) || r == Some(b), "{r:?}");
    // Another act's level has no DRLG: none.
    let r = fx.sim.waypoints(&mut fx.game, |w| w.spawn_room(40, 0));
    assert_eq!(r, None);
    fx.assert_clean();
}

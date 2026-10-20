//! The path provider on the wired sim: the action fixture (act 0's DRLG,
//! level 2 as rooms A and B, streamed) with `enable_paths`. A player's
//! walk request moves it through the real timer queue, sub-tile by
//! sub-tile (a straight walk on the fixture's rooms),
//! across a room edge with the room recache; the waypoint warp places
//! the player in the destination's spawn room; allocation stamps
//! footprints and the coarse free-box search avoids them.

use std::sync::Arc;

use d2_data::tables::{Levels, Objects};

use crate::drlg::collision::bits;
use crate::drlg::TileRect;
use crate::path::coords::Point;
use crate::path::walk::Outcome;
use crate::path::{CollisionRooms, UnitPath};
use crate::skills::fake::blank;
use crate::tick::events::event;
use crate::units::{UnitId, UnitType};
use crate::wiring::action::tests::{Fx, LEVEL};
use crate::world::waypoints::{ArrivalList, WaypointData, NO_WAYPOINT};

/// `velocitypercent`, at its creation value 100
/// (the velocity percent stat).
const STAT_VELOCITY: u16 = 67;

/// The fixture with the path provider on and charstats `WalkVelocity` 6
/// for class 0 (vector V1: velocity 0x600).
fn fx_with(rooms: &[(u32, TileRect)]) -> Fx {
    let mut fx = Fx::with_rooms(rooms);
    let h = fx.sim.hooks();
    h.enable_paths().expect("embedded tables");
    Arc::make_mut(&mut h.tables).combat.charstats[0].walkvelocity = 6;
    fx
}

fn fx() -> Fx {
    fx_with(&[
        (LEVEL, TileRect::new(0, 0, 8, 8)),
        (LEVEL, TileRect::new(8, 0, 8, 8)),
    ])
}

fn player(fx: &mut Fx, x: i32, y: i32) -> UnitId {
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, x, y);
    fx.stats(p, &[(STAT_VELOCITY, 100)]);
    p
}

fn path(fx: &mut Fx, u: UnitId) -> crate::path::DynamicPath {
    fx.sim
        .hooks()
        .paths
        .as_ref()
        .unwrap()
        .dynamic(u)
        .unwrap()
        .clone()
}

fn cell(fx: &mut Fx, x: i32, y: i32) -> u16 {
    let r = fx.a;
    crate::path::collision::point_value(&fx.sim.hooks().drlg, Some(r), x, y, 0xFFFF)
}

fn walk(fx: &mut Fx, p: UnitId, x: i32, y: i32) -> Option<Outcome> {
    fx.sim
        .with(&mut fx.game, |g, v| {
            super::walk::walk_message(v, g, p, 0x01, x as u32, y as u32)
        })
        .1
}

/// Ticks until the player's event 0 stops, recording (precise x, precise
/// y, mode) after each tick.
fn run(fx: &mut Fx, p: UnitId, max: usize) -> Vec<(u32, u32, u32)> {
    let mut out = Vec::new();
    for _ in 0..max {
        fx.tick();
        let d = path(fx, p);
        let mode = fx.sim.sys.units.get(p).unwrap().mode;
        out.push((d.precise_x, d.precise_y, mode));
        if mode != 2 {
            break;
        }
    }
    out
}

#[test]
fn allocation_gives_the_unit_its_path_and_footprint() {
    let mut fx = fx();
    let p = player(&mut fx, 26, 10);
    let d = path(&mut fx, p);
    assert_eq!((d.x(), d.y(), d.room), (26, 10, Some(fx.a)));
    assert_eq!((d.precise_x, d.precise_y), (0x1A8000, 0xA8000));
    assert_eq!((d.foot_mask, d.move_mask, d.path_type), (0x80, 0x1C09, 7));
    assert_eq!((d.unit_size, d.pattern, d.velocity), (2, 1, 0x800));
    // Pattern 1 (plus) with the NO_PATH marker on the centre.
    assert_eq!(cell(&mut fx, 26, 10), 0x80 | bits::NO_PATH);
    for (x, y) in [(25, 10), (27, 10), (26, 9), (26, 11)] {
        assert_eq!(cell(&mut fx, x, y), 0x80, "({x}, {y})");
    }
    assert_eq!(fx.sim.hooks().path_position(p), (26, 10));
    // Removal frees the record and the footprint.
    fx.sim.with(&mut fx.game, |g, v| v.remove(g, p));
    assert_eq!(cell(&mut fx, 26, 10), 0);
    assert!(fx.sim.hooks().paths.as_ref().unwrap().record(p).is_none());
    fx.assert_clean();
}

#[test]
fn walk_request_moves_the_player_sub_tile_by_sub_tile() {
    // A walk from (26, 10) to (31, 10): +0x6000 per tick for 13
    // ticks, the 14th lands on the target centre and stops; the step runs
    // from the tick's timer pass (event 0 every tick)
    // and the stop starts neutral (mode 1).
    let mut fx = fx();
    let p = player(&mut fx, 26, 10);
    assert_eq!(walk(&mut fx, p, 31, 10), Some(Outcome::Moving(1)));
    assert_eq!(fx.sim.sys.units.get(p).unwrap().mode, 2);
    assert_eq!(path(&mut fx, p).velocity, 0x600);
    assert!(fx.timers(p).iter().any(|&(e, _)| e == event::MODE_CHANGE));
    let ticks = run(&mut fx, p, 20);
    assert_eq!(ticks.len(), 14);
    for (k, &(x, y, m)) in ticks.iter().take(13).enumerate() {
        assert_eq!(x, 0x1AE000 + k as u32 * 0x6000, "tick {}", k + 1);
        assert_eq!((y, m), (0xA8000, 2), "tick {}", k + 1);
    }
    assert_eq!(ticks[13], (0x1F8000, 0xA8000, 1));
    let d = path(&mut fx, p);
    assert_eq!((d.cur_point, d.point_count, d.flags & 0x20), (0, 0, 0));
    // The footprint moved with the unit; no event 0 is left.
    assert_eq!(cell(&mut fx, 26, 10), 0);
    assert_eq!(cell(&mut fx, 31, 10), 0x80 | bits::NO_PATH);
    assert!(!fx.timers(p).iter().any(|&(e, _)| e == event::MODE_CHANGE));
    fx.assert_clean();
}

#[test]
fn walking_across_a_room_edge_changes_the_room() {
    // Room A is sub-tiles x 0..40, B 40..80: the step into x = 40 moves
    // the player from A's unit list to B's.
    let mut fx = fx();
    let (a, b) = (fx.a, fx.b);
    let p = player(&mut fx, 37, 10);
    assert_eq!(walk(&mut fx, p, 43, 10), Some(Outcome::Moving(1)));
    let ticks = run(&mut fx, p, 30);
    // 6 sub-tiles at 0x6000: 16 ticks, the last one reaching the point.
    assert_eq!(ticks.len(), 16);
    assert_eq!(ticks[15], (0x2B8000, 0xA8000, 1));
    let d = path(&mut fx, p);
    assert_eq!((d.room, d.prev_room), (Some(b), Some(a)));
    assert_eq!(fx.game.lists.unit(p).unwrap().room(), Some(b));
    assert!(fx.game.lists.room_units(b).contains(&p));
    assert!(!fx.game.lists.room_units(a).contains(&p));
    // The room-change messages ran (flag 0x2 cleared).
    assert_eq!(d.flags & 0x2, 0);
    fx.assert_clean();
}

#[test]
fn a_wall_on_the_way_changes_the_walk() {
    // The same request with a wall in the target cell's row ends
    // elsewhere (the walk check reports the grid it runs on).
    let mut fx = fx();
    let p = player(&mut fx, 26, 10);
    let a = fx.a;
    *fx.sim
        .hooks()
        .drlg
        .grid_mut(a)
        .unwrap()
        .get_mut(29, 10)
        .unwrap() |= bits::WALL;
    let o = walk(&mut fx, p, 31, 10);
    let ticks = run(&mut fx, p, 40);
    assert_ne!(
        (o, ticks.last().copied()),
        (Some(Outcome::Moving(1)), Some((0x1F8000, 0xA8000, 1)))
    );
    // Wherever it went, the player never stands on the wall.
    let d = path(&mut fx, p);
    assert_ne!((d.x(), d.y()), (29, 10));
    fx.assert_clean();
}

/// Object tables for the footprint tests: row 0 a 1 × 1 object with
/// `HasCollision1` set (mask 0x400), row 1 the
/// same without it.
fn collision_objects() -> Arc<crate::world::objects::ObjectTables> {
    let mut solid: Objects = blank();
    (solid.sizex, solid.sizey, solid.hascollision1) = (1, 1, 1);
    let mut open: Objects = blank();
    (open.sizex, open.sizey) = (1, 1);
    Arc::new(crate::world::objects::ObjectTables {
        objects: vec![solid, open],
        shrines: Vec::new(),
        levels: vec![blank::<Levels>(); 150],
        objgroup: Vec::new(),
        leveldefs: Vec::new(),
    })
}

#[test]
fn object_footprint_is_stamped_only_with_has_collision() {
    // The Rogue Encampment torch (class 37, 1 × 1, `HasCollision2` = 1)
    // is in the grid from its creation. Mode 1 here (the
    // fixture's allocation mode): row 0 stamps 0x400 on its cell, row 1
    // (`HasCollision1` = 0) stamps nothing.
    let mut fx = fx();
    fx.sim.create_objects(collision_objects());
    let a = fx.a;
    let cells = [(20, 20), (19, 20), (21, 20), (20, 19), (20, 21), (24, 20)];
    let before: Vec<u16> = cells.iter().map(|&(x, y)| cell(&mut fx, x, y)).collect();
    fx.spawn(UnitType::Object, 0, a, 20, 20);
    fx.spawn(UnitType::Object, 1, a, 24, 20);
    for (&(x, y), b) in cells.iter().zip(before) {
        let stamped = if (x, y) == (20, 20) { 0x400 } else { 0 };
        assert_eq!(cell(&mut fx, x, y), b | stamped, "({x}, {y})");
    }
    fx.assert_clean();
}

/// A run (C→S 0x03) of a player with stamina from (10, 20) to (30, 20),
/// optionally past a solid 1 × 1 object at (20, 20); charstats
/// `WalkVelocity` 6, `RunVelocity` 9 (run list stat 67 = +50).
fn run_past_object(object: bool) -> (crate::path::DynamicPath, i16, u32) {
    let mut fx = fx();
    fx.sim.create_objects(collision_objects());
    Arc::make_mut(&mut fx.sim.hooks().tables).combat.charstats[0].runvelocity = 9;
    let a = fx.a;
    if object {
        fx.spawn(UnitType::Object, 0, a, 20, 20);
    }
    let p = player(&mut fx, 10, 20);
    fx.stats(p, &[(crate::path::walk::step::STAT_STAMINA, 21504)]);
    fx.sim.with(&mut fx.game, |g, v| {
        super::walk::walk_message(v, g, p, 0x03, 30, 20)
    });
    let r = fx.sim.sys.units.get(p).unwrap();
    let (speed, mode) = (r.anim.speed, r.mode);
    (path(&mut fx, p), speed, mode)
}

#[test]
fn a_run_stops_its_ray_before_an_object_footprint() {
    // The ray
    // test from (10, 20) toward (30, 20) tests the player's plus at each
    // cell; at (19, 20) the plus covers the object's cell, so P = (18,
    // 20) is the first point. Without the object the ray
    // is clear and the path is the target alone.
    let (d, _, mode) = run_past_object(true);
    assert_eq!(mode, 3);
    assert_eq!((d.points[0].x, d.points[0].y), (18, 20));
    let (d, _, _) = run_past_object(false);
    assert_eq!(d.point_count, 1);
    assert_eq!((d.points[0].x, d.points[0].y), (30, 20));
}

#[test]
fn a_run_start_speed_reads_the_run_list() {
    // Run speed: w = 101 (mode 3), p = 100 +
    // 50 = 150 → speed 101 · 150 / 100 = 151 from the run start (a
    // level-1 Amazon's run); velocity 0x600 · 150 / 100
    // = 0x900 (vector V2).
    let (d, speed, mode) = run_past_object(false);
    assert_eq!(mode, 3);
    assert_eq!(speed, 151);
    assert_eq!(d.velocity, 0x900);
}

/// Waypoint tables: level 1 (town) has waypoint index 0, the fixture's
/// level 2 index 1; one object class with operate function 23.
fn waypoint_data() -> WaypointData {
    let mut levels = vec![blank::<Levels>(); 150];
    for l in &mut levels {
        l.waypoint = NO_WAYPOINT;
    }
    levels[1].waypoint = 0;
    levels[LEVEL as usize].waypoint = 1;
    let mut o: Objects = blank();
    o.operatefn = 23;
    o.initfn = 17;
    o.framecnt1 = 10 << 8;
    WaypointData::new(&levels, &[o])
}

/// Travel from level 2 (room A) to level 1 (one room, C).
fn travel(fx: &mut Fx) -> (UnitId, Option<crate::units::RoomId>) {
    let a = fx.a;
    let o = fx.spawn(UnitType::Object, 0, a, 20, 20);
    fx.sim.sys.units.get_mut(o).unwrap().mode = 0;
    let p = player(fx, 22, 20);
    let wd = waypoint_data();
    let guid = fx.game.lists.unit(o).unwrap().guid;
    let mut arrivals = ArrivalList(Vec::new());
    let spawn = fx.sim.waypoints(&mut fx.game, |w| {
        use crate::world::waypoints::WaypointWorld;
        let d = w.difficulty();
        w.records(p).unwrap().get_mut(d).set(0).unwrap();
        wd.travel(w, &mut arrivals, p, guid, 1).unwrap();
        // The room the arrival test compares with (one room: the same
        // whatever the search draws).
        w.spawn_room(1, crate::world::waypoints::tile_code(1))
    });
    (p, spawn)
}

#[test]
fn waypoint_warp_places_the_player_in_the_spawn_room() {
    // The e2e step-6 condition in d2-sim terms: after the same-act warp
    // the player's room is the destination's spawn room, so step 7's
    // arrival runs (mode request 2 at its own position → no path →
    // neutral; in town: 5).
    let mut fx = fx_with(&[
        (LEVEL, TileRect::new(0, 0, 8, 8)),
        (LEVEL, TileRect::new(8, 0, 8, 8)),
        (1, TileRect::new(0, 16, 8, 8)),
    ]);
    let (p, spawn) = travel(&mut fx);
    let room = fx.game.lists.unit(p).unwrap().room();
    assert!(room.is_some());
    assert_eq!(room, spawn);
    assert_ne!(room, Some(fx.a));
    let d = path(&mut fx, p);
    assert_eq!(d.room, room);
    let (x, y) = (d.x(), d.y());
    let rect = fx.sim.hooks().drlg.subtile_rect(room.unwrap()).unwrap();
    assert!(rect.contains(x, y), "({x}, {y}) in {rect:?}");
    // The old footprint is cleared (0x4 is the fixture's barrier
    // column). The stamp at the destination is looked up from the
    // destination room (the forced move's second room), so a warp
    // to a room not adjacent to the old one still stamps there.
    assert_eq!(cell(&mut fx, 22, 20), bits::MISSILE_BARRIER);
    let r = room.unwrap();
    let v = crate::path::collision::point_value(&fx.sim.hooks().drlg, Some(r), x, y, 0xFFFF);
    assert_eq!(v & 0x80, 0x80);
    // Room change: flags 2 bit 0x10000, event 14 at f + 50.
    let f = fx.game.frame;
    assert_ne!(fx.sim.sys.units.get(p).unwrap().flags2 & 0x10000, 0);
    assert!(fx.timers(p).contains(&(14, f + 50)));
    // Step 7: neutral in town.
    assert_eq!(fx.sim.sys.units.get(p).unwrap().mode, 5);
    fx.assert_clean();
}

#[test]
fn without_the_provider_the_warp_stays_pending() {
    // For the warp: the same travel with the provider off leaves the
    // player where it was (the e2e step-6 stop).
    let mut fx = Fx::with_rooms(&[
        (LEVEL, TileRect::new(0, 0, 8, 8)),
        (LEVEL, TileRect::new(8, 0, 8, 8)),
        (1, TileRect::new(0, 16, 8, 8)),
    ]);
    let (p, spawn) = travel(&mut fx);
    assert_eq!(fx.game.lists.unit(p).unwrap().room(), Some(fx.a));
    assert_ne!(Some(fx.a), spawn);
}

#[test]
fn coarse_free_box_avoids_a_monster_footprint() {
    // Pass 1 tries (X0 − 1, Y0 − 1) with a 3 × 3 box (n = 1); a size-2
    // monster (pattern 1, NO_PATH 0x1000 on its centre, in mask 0x3C01)
    // stamped at (X0, Y0) blocks it, pass 2's first offset (−2, −2) is
    // free. Without the monster pass 1 wins.
    let mut fx = fx();
    let a = fx.a;
    {
        let t = Arc::make_mut(&mut fx.sim.hooks().tables);
        let ex = usize::from(t.combat.monstats[0].monstatsex);
        t.combat.monstats2[ex].sizex = 2;
    }
    let search = |fx: &mut Fx| {
        let mut pt = Point::new(30, 10);
        let r = super::place::coarse_free_box(&fx.sim.hooks().drlg, a, &mut pt, 1, 0x3C01);
        (r, pt)
    };
    assert_eq!(search(&mut fx), (Some(a), Point::new(29, 9)));
    let m = fx.spawn(UnitType::Monster, 0, a, 30, 10);
    match fx.sim.hooks().paths.as_ref().unwrap().record(m) {
        Some(UnitPath::Dynamic(d)) => {
            assert_eq!((d.foot_mask, d.pattern, d.x(), d.y()), (0x100, 1, 30, 10))
        }
        other => panic!("monster path {other:?}"),
    }
    assert_ne!(cell(&mut fx, 30, 10) & 0x100, 0);
    assert_eq!(search(&mut fx), (Some(a), Point::new(28, 8)));
    fx.assert_clean();
}

/// A monster of class 0 allocated in `mode` at (x, y) of room A.
fn spawn_monster_in_mode(fx: &mut Fx, mode: u32, x: i32, y: i32) -> UnitId {
    let req = crate::units::lifecycle::AllocRequest {
        ty: UnitType::Monster,
        class: 0,
        room: Some(fx.a),
        add: true,
        fixed_guid: None,
        mode,
        allied: false,
    };
    fx.sim
        .with(&mut fx.game, |g, v| v.allocate(g, &req, x, y))
        .expect("allocated")
}

#[test]
fn a_monster_allocated_dead_carries_the_corpse_mask() {
    let mut fx = fx();
    {
        let t = Arc::make_mut(&mut fx.sim.hooks().tables);
        let ex = usize::from(t.combat.monstats[0].monstatsex);
        t.combat.monstats2[ex].sizex = 2;
    }
    // Mode 12 (dead), `deadCol` clear: pattern 5, mask 0x8000, the live
    // mask 0x100 and its NO_PATH marker are gone.
    let m = spawn_monster_in_mode(&mut fx, 12, 30, 10);
    let d = path(&mut fx, m);
    assert_eq!((d.pattern, d.foot_mask), (5, 0x8000));
    assert_eq!(cell(&mut fx, 30, 10) & 0x8000, 0x8000);
    assert_eq!(cell(&mut fx, 30, 10) & (0x100 | bits::NO_PATH), 0);
    assert_eq!(cell(&mut fx, 29, 10) & 0x100, 0);
    // Mode 0 likewise; a live mode keeps the live footprint.
    let m0 = spawn_monster_in_mode(&mut fx, 0, 20, 10);
    assert_eq!(path(&mut fx, m0).foot_mask, 0x8000);
    let live = spawn_monster_in_mode(&mut fx, 1, 10, 10);
    assert_eq!(path(&mut fx, live).foot_mask, 0x100);
    assert_ne!(cell(&mut fx, 10, 10) & 0x100, 0);
    // `deadCol` set: the settings are skipped.
    {
        let t = Arc::make_mut(&mut fx.sim.hooks().tables);
        let ex = usize::from(t.combat.monstats[0].monstatsex);
        t.combat.monstats2[ex].deadcol = true;
    }
    let keep = spawn_monster_in_mode(&mut fx, 12, 24, 10);
    assert_eq!(path(&mut fx, keep).foot_mask, 0x100);
    assert_ne!(cell(&mut fx, 24, 10) & 0x100, 0);
}

/// The player mode-update table's skill rows: a player in a
/// skill mode sends the skill message to other clients, and to its own
/// client only with E flags bit 0x4 (the client starts its own cast at
/// the click).
#[test]
fn a_cast_skips_the_casters_own_client_unless_e_flag_4() {
    use crate::units::lists::client_state;
    let mut fx = fx();
    let p = player(&mut fx, 10, 10);
    let q = player(&mut fx, 12, 10);
    let own = fx
        .game
        .lists
        .add_client(Some(p), None, client_state::IN_GAME);
    let other = fx
        .game
        .lists
        .add_client(Some(q), None, client_state::IN_GAME);
    let entry = crate::skills::SkillEntry {
        skill: 59,
        base: 20,
        owner_guid: -1,
        ..crate::skills::SkillEntry::default()
    };
    fx.sim.sys.hooks.x.used.insert(p, entry);
    {
        let r = fx.sim.sys.units.get_mut(p).unwrap();
        r.mode = 10;
        r.flags |= crate::units::record::flags::CHANGED;
    }
    let pass = |fx: &mut Fx, c| {
        fx.sim.with(&mut fx.game, |g, v| {
            super::walk::update_messages(v, g, c, p);
        });
        std::mem::take(&mut fx.sim.sys.hooks.x.sent)
    };
    assert_eq!(pass(&mut fx, own), vec![]);
    let to_other = pass(&mut fx, other);
    assert_eq!(to_other.len(), 1);
    assert_eq!(
        (to_other[0].0, to_other[0].1[0], to_other[0].1[10]),
        (q, 0x4D, 20)
    );
    fx.sim.sys.hooks.x.entry_flags.insert(p, 0x4);
    let to_own = pass(&mut fx, own);
    assert_eq!(to_own.len(), 1);
    assert_eq!((to_own[0].0, to_own[0].1[0]), (p, 0x4D));
}

#[test]
fn a_face_op_snaps_the_path_direction_toward_the_point() {
    use crate::skills::use_::bodies::PathOp;
    // Bone Prison's segment faces (tx, ty): the face direction from the unit's
    // position, snapped to the path directions (direction and new direction).
    let mut fx = fx();
    let p = player(&mut fx, 10, 10);
    let (r, want) = fx.sim.with(&mut fx.game, |g, v| {
        let want = v.path_dir64(p, (10, 20)).unwrap();
        let r = crate::wiring::interaction::body_path::path_op(v, g, p, PathOp::Face(10, 20));
        (r, want)
    });
    assert_eq!(r, Some(0), "carried here, not by the seam");
    let d = path(&mut fx, p);
    assert_ne!(want, 0, "south of the unit is not direction 0");
    assert_eq!(
        (i32::from(d.direction), i32::from(d.new_direction)),
        (want, want)
    );
}

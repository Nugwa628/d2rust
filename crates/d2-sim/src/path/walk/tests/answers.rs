//! Tests of the answered walk questions (PQ2–PQ9, GR1,
//! OQ4, OQ5, OQ8, the resync, missile paths, edge case 12).

use super::fake::{Ctx, FakeUnit, FakeUnits, FakeWorld, ROOM};
use super::{run_ticks, setup, tables, P};
use crate::drlg::TileRect;
use crate::path::coords::to_fp16_center as centre;
use crate::path::history::PositionHistory;
use crate::path::record::{alloc_dynamic_path, flags, DynamicKind, DynamicPath, PathPoint};
use crate::path::tables::PathTables;
use crate::path::walk::find::compute;
use crate::path::walk::geom::{octant, step};
use crate::path::walk::missile::BOLT_OFFSETS;
use crate::path::walk::request::{handle_message, interrupt_check, set_mode_and_velocity, Outcome};
use crate::path::walk::resync::{handle_resync, resync_distance, Resync, ResyncRing, RING_WINDOW};
use crate::path::walk::seams::{Point, TargetUnit, UsedSkill, WalkError};
use crate::path::walk::sine::SINE_BITS;
use crate::path::walk::step::{Walk, STEP_BASE};
use crate::path::walk::velocity::set_velocity;
use crate::units::{UnitId, UnitType};

// ---- (PQ6) ----------------------------------------------

#[test]
fn failed_concentration_roll_falls_through_to_state_15() {
    let (t, mut c) = setup(40, 40, 10, 10);
    c.u.unit(P).mode = 2;
    c.u.unit(P).used_skill = Some(UsedSkill {
        id: 9,
        interrupt: true,
        ..UsedSkill::default()
    });
    // v = 0: the roll always fails; state 15 then sends it to rule 6
    // (mode 2: refuse). One draw.
    c.u.unit(P).states = vec![42, 15];
    c.u.unit(P).state_stats.insert((42, 164), 0);
    let mut s = c.u.units[&P].seed;
    assert!(!interrupt_check(&t, &mut c, P, 2, None).unwrap());
    s.roll(100);
    assert_eq!(c.u.units[&P].seed, s);
    // Without state 15 the failed roll allows.
    c.u.unit(P).states = vec![42];
    assert!(interrupt_check(&t, &mut c, P, 2, None).unwrap());
}

// ---- (PQ2, PQ3) --------------------------------------

#[test]
fn mode_without_modifier_keeps_the_velocity() {
    let (t, mut c) = setup(40, 40, 10, 10);
    let mut p = c.w.paths[&P].clone();
    p.velocity = 0x1234;
    p.max_velocity = 0x1234;
    p.field_38 = 0;
    // Mode 1 (neutral) has no velocity modifier: velocity unchanged.
    set_mode_and_velocity(&t, &mut c, P, &mut p, 1);
    assert_eq!(
        (p.velocity, p.max_velocity, p.field_38),
        (0x1234, 0x1234, 0)
    );
    // Knockback: 0x1000.
    set_mode_and_velocity(&t, &mut c, P, &mut p, 19);
    assert_eq!(
        (p.velocity, p.max_velocity, p.field_38),
        (0x1000, 0x1000, 15)
    );
    // The same value: +0x38 untouched, the max still written.
    p.field_38 = 0;
    p.max_velocity = 7;
    set_velocity(&mut p, 0x1000);
    assert_eq!(
        (p.velocity, p.max_velocity, p.field_38),
        (0x1000, 0x1000, 0)
    );
}

// ---- steps 2, 3, 5 (PQ5, position history) ---------------------

#[test]
fn state_13_step_goes_on_with_the_movement() {
    let (t, mut c) = setup(200, 200, 100, 100);
    handle_message(&t, &mut c, P, 0x01, 105, 100).unwrap();
    c.u.unit(P).states = vec![13];
    let ticks = run_ticks(&t, &mut c, 1);
    assert!(c.u.log.contains(&"state13".to_string()));
    // M1's first tick: the step ran.
    assert_eq!(ticks[0].0, 0x64E000);
}

#[test]
fn exhausted_run_restarts_as_walk() {
    let (t, mut c) = setup(200, 200, 100, 100);
    handle_message(&t, &mut c, P, 0x03, 120, 100).unwrap();
    assert_eq!(c.u.units[&P].mode, 3);
    c.u.unit(P).stats.insert(10, 1);
    c.u.log.clear();
    run_ticks(&t, &mut c, 1);
    assert_eq!(c.u.units[&P].mode, 2);
    assert!(c.u.log.contains(&"mode 2".to_string()));
    assert!(!c.u.log.iter().any(|l| l.starts_with("run list")));
}

#[test]
fn walk_step_writes_the_position_history() {
    let (t, mut c) = setup(200, 200, 100, 100);
    c.u.history.insert(P, PositionHistory::default());
    handle_message(&t, &mut c, P, 0x01, 120, 100).unwrap();
    let ticks = run_ticks(&t, &mut c, 100);
    assert_eq!(ticks.last().unwrap().0, centre(120));
    let h = &c.u.history[&P];
    // Written on the first tick (far from the empty ring's (0, 0)), then
    // each time d² > 45 (7 sub-tiles along x).
    assert_eq!(h.next, 3);
    assert_eq!(&h.entries[..3], &[(100, 100), (107, 100), (114, 100)]);
}

// ---- (PQ4, PQ7, OQ8) --------------

/// A path at (10, 10) with points (11, 10), (11, 14), half a cell per
/// tick along +x.
fn two_points(c: &Ctx) -> DynamicPath {
    let mut p = c.w.paths[&P].clone();
    p.points[0] = PathPoint { x: 11, y: 10 };
    p.points[1] = PathPoint { x: 11, y: 14 };
    p.put_final_target(Point::new(11, 14));
    p.point_count = 2;
    p.cur_point = 0;
    p.velocity = 0x800;
    p.dir_vec_x = 4096;
    p.dir_vec_y = 0;
    p.flags |= flags::ACTIVE;
    p
}

#[test]
fn path_type_4_does_not_aim_at_the_next_point() {
    for (ty, aims) in [(7u32, true), (4, false)] {
        let (t, mut c) = setup(40, 40, 10, 10);
        let mut p = two_points(&c);
        p.path_type = ty;
        for _ in 0..2 {
            assert!(Walk { t: &t, c: &mut c }
                .movement(P, &mut p, STEP_BASE)
                .unwrap());
        }
        assert_eq!(p.cur_point, 1, "type {ty}");
        let v = (p.dir_vec_x, p.dir_vec_y);
        assert_eq!(v == (0, 4096), aims, "type {ty}: {v:?}");
    }
}

#[test]
fn arrival_result_is_the_repath_result_and_types_are_written_directly() {
    let run = |budget: u8| {
        let (t, mut c) = setup(40, 40, 10, 10);
        c.u.unit(P).ty = UnitType::Monster;
        let mut p = two_points(&c);
        p.set_path_type(&t, false, 13).unwrap();
        p.repath_budget = budget;
        // index ≥ count, position ≠ final target: re-path (finish 0).
        p.cur_point = 2;
        let r = Walk { t: &t, c: &mut c }
            .movement(P, &mut p, STEP_BASE)
            .unwrap();
        (r, p)
    };
    // Budget 0: the re-path returns 0, the arrival fails, reset.
    let (r, p) = run(0);
    assert!(!r);
    assert_eq!((p.point_count, p.flags & flags::ACTIVE), (0, 0));
    // Budget 5: the re-path computes; its points drive the movement.
    let (r, p) = run(5);
    assert!(r);
    assert!(p.point_count > 0);
    // Type 2 written to +0x3C directly: type 13's flag 0x100 is kept.
    assert_eq!(p.path_type, 2);
    assert_ne!(p.flags & 0x100, 0);
    // Budget −= index (2), clamped.
    assert_eq!(p.repath_budget, 3);
}

#[test]
fn repath_budget_setter_and_clamp() {
    let mut p = DynamicPath::default();
    p.set_repath_budget(20).unwrap();
    assert_eq!(p.repath_budget, 20);
    assert!(p.set_repath_budget(256).is_err());
    p.add_repath_budget(-25);
    assert_eq!(p.repath_budget, 0);
    p.add_repath_budget(300);
    assert_eq!(p.repath_budget, 255);
}

// ---- target lead (OQ4) -------------------------------------------

#[test]
fn target_lead_adds_nothing() {
    let (t, mut c) = setup(40, 40, 10, 10);
    let mut m = FakeUnit::player();
    m.ty = UnitType::Monster;
    m.guid = 5;
    m.pos = Point::new(20, 10);
    c.u.units.insert(UnitId(5), m);
    let mut p = c.w.paths[&P].clone();
    p.target_lead = 3;
    p.target_unit = Some(TargetUnit {
        unit: UnitId(5),
        ty: UnitType::Monster,
        guid: 5,
    });
    compute(&t, &mut c, &mut p, P, false).unwrap();
    assert_eq!(p.target(), Point::new(20, 10));
}

// ---- edge case 12, vector W10 (PQ9) ---------------------------------

#[test]
fn w10_cell_walk_overshoot() {
    let (t, mut c) = setup(200, 200, 100, 101);
    let mut p = c.w.paths[&P].clone();
    assert_ne!(p.flags & flags::SAVE_STEPS, 0);
    p.precise_x = 0x6468D3;
    p.precise_y = 0x6558D3;
    let r = Walk { t: &t, c: &mut c }
        .cell_walk(P, &mut p, (0x1972D, 0x1972D))
        .unwrap();
    assert_eq!(r, Ok((0x660000, 0x66F000)));
    assert_eq!(p.saved_count, 10);
    for i in 0..10u16 {
        assert_eq!(
            p.saved_steps[i as usize],
            PathPoint {
                x: 101 + i,
                y: 102 + i
            }
        );
    }
    // The footprint stays where the walk ended, past the target cell
    // (102, 102).
    assert_eq!(c.w.value(Point::new(110, 111)) & 0x1080, 0x1080);
    assert_eq!(c.w.value(Point::new(100, 101)) & 0x1000, 0);
}

// ---- missile paths ----------------------------------------------

/// A missile unit P at (100, 100) in a 200 × 200 world.
fn missile_at(t: &PathTables) -> (Ctx, DynamicPath) {
    let mut w = FakeWorld::new(200, 200);
    let p = alloc_dynamic_path(
        t,
        &mut w,
        DynamicKind::Missile { size: 1 },
        P,
        Some(ROOM),
        100,
        100,
        false,
    )
    .unwrap();
    let mut u = FakeUnits::with_player(P);
    u.unit(P).ty = UnitType::Missile;
    u.unit(P).seed.set(0x1234_5678, 0x9ABC);
    (Ctx::new(w, u), p)
}

#[test]
fn straight_missile_path() {
    let t = tables();
    let (mut c, mut p) = missile_at(&t);
    assert_ne!(p.flags & flags::MISSILE, 0);
    p.put_target(Point::new(110, 105));
    assert_eq!(compute(&t, &mut c, &mut p, P, false).unwrap(), 1);
    assert_eq!((p.point_count, p.cur_point), (1, 0));
    assert_eq!(p.point(0), Point::new(110, 105));
    assert_ne!(p.flags & flags::ACTIVE, 0);
    assert_ne!((p.vel_vec_x, p.vel_vec_y), (0, 0));
    assert_eq!(p.flags & flags::OUTSIDE_ROOM, 0);
    // Out of range (|Δx| ≥ 100) or a zero coordinate: 0, flag 0x20 off.
    for tg in [Point::new(200, 105), Point::new(0, 105), Point::new(110, 0)] {
        p.put_target(tg);
        assert_eq!(compute(&t, &mut c, &mut p, P, false).unwrap(), 0);
        assert_eq!(p.flags & flags::ACTIVE, 0);
    }
    // A target outside the path's room: flag 0x1.
    c.w.room0 = TileRect::new(0, 0, 105, 200);
    p.put_target(Point::new(110, 105));
    compute(&t, &mut c, &mut p, P, false).unwrap();
    assert_ne!(p.flags & flags::OUTSIDE_ROOM, 0);
    // Rule 4 (read 2026-10-08): a target inside leaves the flag set; a
    // path without a room sets it.
    c.w.room0 = TileRect::new(0, 0, 200, 200);
    compute(&t, &mut c, &mut p, P, false).unwrap();
    assert_ne!(p.flags & flags::OUTSIDE_ROOM, 0);
    p.flags &= !flags::OUTSIDE_ROOM;
    let room = p.room.take();
    assert!(room.is_some());
    compute(&t, &mut c, &mut p, P, false).unwrap();
    assert_ne!(p.flags & flags::OUTSIDE_ROOM, 0);
    p.room = room;
    p.flags &= !flags::OUTSIDE_ROOM;
    // A target unit: its position.
    let mut m = FakeUnit::player();
    m.ty = UnitType::Monster;
    m.guid = 5;
    m.pos = Point::new(90, 95);
    c.u.units.insert(UnitId(5), m);
    p.target_unit = Some(TargetUnit {
        unit: UnitId(5),
        ty: UnitType::Monster,
        guid: 5,
    });
    compute(&t, &mut c, &mut p, P, false).unwrap();
    assert_eq!(p.point(0), Point::new(90, 95));
    // Another type with the missile flag: fatal.
    p.path_type = 17;
    assert!(matches!(
        compute(&t, &mut c, &mut p, P, false),
        Err(WalkError::Fatal(_))
    ));
}

#[test]
fn charged_bolt_path_draws_on_the_unit_seed() {
    let t = tables();
    // Offsets: (−1, 0, +1) ten times, then −1, +1.
    assert_eq!(&BOLT_OFFSETS[..6], &[-1, 0, 1, -1, 0, 1]);
    assert_eq!(&BOLT_OFFSETS[27..], &[-1, 0, 1, -1, 1]);
    let (mut c, mut p) = missile_at(&t);
    p.path_type = 10;
    p.max_distance = 9; // n = 4
    p.put_target(Point::new(110, 104));
    let mut s = c.u.units[&P].seed;
    assert_eq!(compute(&t, &mut c, &mut p, P, false).unwrap(), 5);
    assert_eq!(p.point_count, 5);
    let start = Point::new(100, 100);
    let d0 = t.testdir[octant(start, Point::new(110, 104))][0];
    let mut cur = start;
    for k in 0..4 {
        assert_eq!(p.point(k), cur, "point {k}");
        let lo = s.step();
        let d = (BOLT_OFFSETS[(lo & 31) as usize] + d0) & 7;
        let st = step(&t, d as u8);
        cur = Point::new(cur.x + 2 * st.x, cur.y + 2 * st.y);
    }
    assert_eq!(p.point(4), cur);
    // n draws.
    assert_eq!(c.u.units[&P].seed, s);
    // n beyond the 78 point slots: fatal (it would overrun).
    p.max_distance = 200;
    assert!(compute(&t, &mut c, &mut p, P, false).is_err());
}

#[test]
fn blessed_hammer_spiral() {
    let t = tables();
    let (mut c, mut p) = missile_at(&t);
    p.path_type = 14;
    p.precise_x = 0x648000;
    p.precise_y = 0x64C000;
    let seed = c.u.units[&P].seed;
    assert_eq!(compute(&t, &mut c, &mut p, P, false).unwrap(), 77);
    assert_eq!(p.point_count, 77);
    assert_eq!(c.u.units[&P].seed, seed);
    // Reference in double precision (the product of two float32 values is
    // exact there).
    let (sx, sy) = (0x648000u32, 0x64C000u32);
    let mut prev = (sx, sy);
    let mut want = Vec::new();
    let mut k = 0u32;
    while want.len() < 77 {
        k += 1;
        let r = f64::from(9600 * k);
        let a = 16 * k;
        let cos = f64::from(f32::from_bits(SINE_BITS[((a + 128) & 0x1FF) as usize]));
        let sin = f64::from(f32::from_bits(SINE_BITS[(a & 0x1FF) as usize]));
        let x = sx.wrapping_add((cos * r).trunc() as i64 as u32);
        let y = sy.wrapping_add((sin * r).trunc() as i64 as u32);
        if (x >> 16, y >> 16) != (prev.0 >> 16, prev.1 >> 16) {
            want.push(Point::new((x >> 16) as i32, (y >> 16) as i32));
            prev = (x, y);
        }
    }
    assert_eq!(p.live_points(), want);
    // k = 1: r = 9600, a = 16: x moves by cos(16) · 9600 (same cell
    // here), so the first point needs more steps.
    assert_ne!(p.point(0), Point::new(100, 100));
}

// ---- C→S 0x5F --------------------------------------------------

fn msg(x: u16, y: u16) -> [u8; 5] {
    let [x0, x1] = x.to_le_bytes();
    let [y0, y1] = y.to_le_bytes();
    [0x5F, x0, x1, y0, y1]
}

#[test]
fn resync_ignore_and_walk_branches() {
    let (t, mut c) = setup(200, 200, 100, 100);
    assert_eq!(resync_distance(Point::new(0, 0), Point::new(20, 7)), 23);
    // Length.
    assert_eq!(
        handle_resync(&t, &mut c, P, &[0x5F, 0, 0, 0]).unwrap(),
        (3, Resync::BadLength)
    );
    // d < 5, a used skill, dead: ignored.
    assert_eq!(
        handle_resync(&t, &mut c, P, &msg(103, 101)).unwrap(),
        (0, Resync::Ignored)
    );
    c.u.unit(P).used_skill = Some(UsedSkill::default());
    assert_eq!(
        handle_resync(&t, &mut c, P, &msg(150, 100)).unwrap().1,
        Resync::Ignored
    );
    c.u.unit(P).used_skill = None;
    c.u.dead = true;
    assert_eq!(
        handle_resync(&t, &mut c, P, &msg(150, 100)).unwrap().1,
        Resync::Ignored
    );
    c.u.dead = false;
    // d > 45: walk (mode 2 from neutral).
    assert_eq!(
        handle_resync(&t, &mut c, P, &msg(150, 100)).unwrap(),
        (0, Resync::Walk(Outcome::Moving(1)))
    );
    assert_eq!(c.u.units[&P].mode, 2);
    // From run (mode 3): run.
    let (t, mut c) = setup(200, 200, 100, 100);
    c.u.unit(P).mode = 3;
    handle_resync(&t, &mut c, P, &msg(100, 110)).unwrap();
    assert_eq!(c.u.units[&P].mode, 3);
    assert!(c.u.log.iter().any(|l| l.starts_with("run list")));
    // d < 15 walks too; state 108 walks inside the snap band.
    let (t, mut c) = setup(200, 200, 100, 100);
    assert_eq!(
        handle_resync(&t, &mut c, P, &msg(110, 100)).unwrap().1,
        Resync::Walk(Outcome::Moving(1))
    );
    let (t, mut c) = setup(200, 200, 100, 100);
    c.u.unit(P).states = vec![108];
    assert_eq!(
        handle_resync(&t, &mut c, P, &msg(120, 100)).unwrap().1,
        Resync::Walk(Outcome::Moving(1))
    );
    // A target unit other than the player: nothing.
    let (t, mut c) = setup(200, 200, 100, 100);
    let mut m = FakeUnit::player();
    m.ty = UnitType::Monster;
    m.guid = 5;
    c.u.units.insert(UnitId(5), m);
    c.w.paths.get_mut(&P).unwrap().target_unit = Some(TargetUnit {
        unit: UnitId(5),
        ty: UnitType::Monster,
        guid: 5,
    });
    assert_eq!(
        handle_resync(&t, &mut c, P, &msg(150, 100)).unwrap(),
        (0, Resync::KeepsTarget)
    );
    // No client: fatal.
    c.u.has_client = false;
    assert!(handle_resync(&t, &mut c, P, &msg(150, 100)).is_err());
}

#[test]
fn resync_snap_branch() {
    // Not reachable (a wall on the target: the type-15 wall follow
    // ends elsewhere): S→C 0x15 with
    // the player's position, flag 0; the path's mask, type and distances
    // restored, the computed points not.
    let (t, mut c) = setup(200, 200, 100, 100);
    c.w.wall(120, 100);
    assert_eq!(
        handle_resync(&t, &mut c, P, &msg(120, 100)).unwrap(),
        (0, Resync::Reassigned)
    );
    let want = format!(
        "send {:02x?}",
        crate::path::walk::messages::reassign_player(0, 1, 100, 100, 0)
    );
    assert!(c.u.log.contains(&want), "{:?}", c.u.log);
    let p = &c.w.paths[&P];
    assert_eq!(
        (p.move_mask, p.path_type, p.max_distance, p.dist_budget),
        (0x1C09, 7, 73, 73)
    );
    assert_eq!(p.target(), Point::default());
    // Reachable and placed: lock 125 frames, the frame recorded.
    let (t, mut c) = setup(200, 200, 100, 100);
    c.g.frame = 1000;
    c.u.place_ok = true;
    assert_eq!(
        handle_resync(&t, &mut c, P, &msg(120, 100)).unwrap(),
        (0, Resync::Snapped { delay: 125 })
    );
    assert!(c.u.log.contains(&"place 1 (120,100)".to_string()));
    assert!(c.u.log.contains(&"lock 1125".to_string()));
    assert_eq!(c.u.ring.slots, [1000, 0, 0, 0, 0]);
    // Placement fails: 0x15.
    c.u.place_ok = false;
    assert_eq!(
        handle_resync(&t, &mut c, P, &msg(120, 100)).unwrap().1,
        Resync::Reassigned
    );
    // State 54 makes it unreachable.
    c.u.place_ok = true;
    c.u.unit(P).states = vec![54];
    assert_eq!(
        handle_resync(&t, &mut c, P, &msg(120, 100)).unwrap().1,
        Resync::Reassigned
    );
    c.u.unit(P).states.clear();
    // Fifth snap within 2250 frames: the ring is full, r = lo' % 100
    // picks 1500 / 3000 / 4500.
    c.u.ring = ResyncRing {
        slots: [990, 991, 0, 992, 993],
    };
    let mut s = c.u.units[&P].seed;
    let r = s.step() % 100;
    let delay = if r < 50 {
        1500
    } else if r < 75 {
        3000
    } else {
        4500
    };
    assert_eq!(
        handle_resync(&t, &mut c, P, &msg(120, 100)).unwrap().1,
        Resync::Snapped { delay }
    );
    assert_eq!(c.u.ring.slots, [990, 991, 1000, 992, 993]);
    assert_eq!(c.u.units[&P].seed, s);
    // Game type ≠ 0: 125, no draw.
    c.u.game_type = 1;
    c.u.ring = ResyncRing {
        slots: [990, 991, 0, 992, 993],
    };
    assert_eq!(
        handle_resync(&t, &mut c, P, &msg(120, 100)).unwrap().1,
        Resync::Snapped { delay: 125 }
    );
    assert_eq!(c.u.units[&P].seed, s);
    // A stale slot (> 2250 frames old) is reused and does not count.
    let mut ring = ResyncRing {
        slots: [1, 2000, 2001, 2002, 2003],
    };
    assert!(!ring.is_full(3000, RING_WINDOW));
    assert!(ring.record(3000, RING_WINDOW));
    assert_eq!(ring.slots[0], 3000);
    assert!(ring.is_full(3000, RING_WINDOW));
    assert!(!ring.record(3001, RING_WINDOW));
}

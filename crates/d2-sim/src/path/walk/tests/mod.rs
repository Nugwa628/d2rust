//! Unit tests from the walk rules' test vectors and edge cases, on the
//! fakes of [`fake`].

mod answers;
pub(super) mod fake;
mod fsim;
mod gaps;
mod messages;
mod tick_rate;

use fake::{Ctx, FakeUnits, FakeWorld, ROOM};

use super::find::compute;
use super::geom::{direction_vector, octant, path_distance, ray_test, Ray};
use super::request::{handle_message, interrupt_check, mode_check, request, Outcome, WalkTarget};
use super::seams::{Point, UsedSkill};
use super::step::{Step, Walk};
use super::velocity::{mode_velocity, run_velocity_bonus};
use crate::drlg::TileRect;
use crate::path::coords::to_fp16_center as centre;
use crate::path::record::{flags, DynamicPath, PathPoint};
use crate::path::tables::PathTables;
use crate::units::{ClientId, RoomId, UnitId, UnitType};

const P: UnitId = UnitId(1);

fn tables() -> PathTables {
    PathTables::spec().unwrap()
}

/// A world of `w`×`h` with the player at (x, y).
fn setup(w: i32, h: i32, x: i32, y: i32) -> (PathTables, Ctx) {
    let t = tables();
    let mut world = FakeWorld::new(w, h);
    world.add_player(&t, P, x, y);
    let mut units = FakeUnits::with_player(P);
    units.unit(P).pos = Point::new(x, y);
    (t, Ctx::new(world, units))
}

/// A context with the player unit and an empty world.
fn units_only() -> Ctx {
    Ctx::new(FakeWorld::new(1, 1), FakeUnits::with_player(P))
}

/// Computes a type-7 path to `target` (no velocity change, as resync step 3
/// runs before the mode set).
fn compute_to(t: &PathTables, c: &mut Ctx, target: Point) -> (i32, DynamicPath) {
    let mut path = c.w.paths[&P].clone();
    path.put_target(target);
    path.target_unit = None;
    let n = compute(t, c, &mut path, P, false).unwrap();
    (n, path)
}

fn pts(v: &[(i32, i32)]) -> Vec<Point> {
    v.iter().map(|&(x, y)| Point::new(x, y)).collect()
}

// ---- helpers ---------------------------------------------------

#[test]
fn octant_and_path_distance() {
    let t = tables();
    let o = |dx, dy| octant(Point::new(0, 0), Point::new(dx, dy));
    assert_eq!(o(0, 0), 12);
    assert_eq!(o(7, 0), 22); // horizontal, dx clamped to 2
    assert_eq!(o(-5, 0), 2);
    assert_eq!(o(-1, 5), 9); // vertical branch, dx < 0
    assert_eq!(o(-1, -5), 5);
    assert_eq!(o(1, 1), 18);
    assert_eq!(o(2, 5), 14); // vertical, dx := 2 & 1 = 0
    assert_eq!(o(1, 5), 19); // vertical, dx := 1
    assert_eq!(o(-3, -3), 0);
    // Distance: table below 8, else 2·max + min.
    let d = |dx, dy| path_distance(&t, Point::new(0, 0), Point::new(dx, dy));
    assert_eq!(d(1, 1), 0); // dist8_path[9] = −1 → 0
    assert_eq!(d(7, 0), 9);
    assert_eq!(d(8, 3), 19);
}

#[test]
fn ray_test_returns_cell_before_block() {
    let t = tables();
    let mut w = FakeWorld::new(40, 40);
    for y in 5..=15 {
        w.wall(15, y);
    }
    let _ = t;
    let r = ray_test(
        &w,
        Some(ROOM),
        1,
        0x1C09,
        Point::new(10, 10),
        Point::new(20, 10),
    );
    assert_eq!(r, Ray::Blocked(Point::new(13, 10)));
    let r = ray_test(
        &w,
        Some(ROOM),
        1,
        0x1C09,
        Point::new(10, 10),
        Point::new(13, 30),
    );
    assert_eq!(r, Ray::Clear);
    // Edge case 4: after a minor-axis step with err = 0 the new cell is
    // not tested. (10,10)→(13,11): Nx 4, Ny 2; the first minor step
    // leaves err = 0 at (11,11), which is never tested (point pattern).
    let mut w2 = FakeWorld::new(40, 40);
    w2.wall(11, 11);
    let r = ray_test(
        &w2,
        Some(ROOM),
        0,
        0x1,
        Point::new(10, 10),
        Point::new(13, 11),
    );
    assert_eq!(r, Ray::Clear);
    // With err > 0 after the minor step the cell is tested: (10,10)→(12,11)
    // (Nx 3, Ny 2) tests (11,11).
    let r = ray_test(
        &w2,
        Some(ROOM),
        0,
        0x1,
        Point::new(10, 10),
        Point::new(12, 11),
    );
    assert_eq!(r, Ray::Blocked(Point::new(10, 10)));
}

// ---- path vectors W1–W9 ---------------------------------------------

#[test]
fn w1_clear_ray() {
    let (t, mut c) = setup(40, 40, 10, 10);
    let (n, p) = compute_to(&t, &mut c, Point::new(20, 15));
    assert_eq!(n, 1);
    assert_eq!(p.live_points(), pts(&[(20, 15)]).as_slice());
    assert_eq!(p.final_target(), Point::new(20, 15));
    assert_ne!(p.flags & flags::ACTIVE, 0);
    assert_eq!(p.flags & flags::OUTSIDE_ROOM, 0);
}

fn w2_world(x: i32, y: i32) -> (PathTables, Ctx) {
    let (t, mut c) = setup(40, 40, x, y);
    for yy in 5..=15 {
        c.w.wall(15, yy);
    }
    (t, c)
}

#[test]
fn w2_astar_around_wall() {
    let (t, mut c) = w2_world(10, 10);
    let (n, p) = compute_to(&t, &mut c, Point::new(20, 10));
    assert_eq!(
        p.live_points(),
        pts(&[(13, 13), (13, 15), (15, 17), (17, 15), (17, 13), (20, 10)]).as_slice()
    );
    assert_eq!(n, 6);
}

#[test]
fn w3_no_astar_beyond_radius() {
    let (t, mut c) = setup(60, 60, 20, 20);
    for y in 0..=49 {
        c.w.wall(30, y);
    }
    let (n, p) = compute_to(&t, &mut c, Point::new(40, 20));
    assert_eq!(n, 1);
    assert_eq!(p.live_points(), pts(&[(28, 20)]).as_slice());
    // No target unit, flag 0x10 clear → target := last point.
    assert_eq!(p.target(), Point::new(28, 20));
}

#[test]
fn w4_blocked_target_prepared() {
    let (t, mut c) = w2_world(10, 10);
    let (n, p) = compute_to(&t, &mut c, Point::new(15, 10));
    assert_eq!(n, 1);
    assert_eq!(p.final_target(), Point::new(13, 10));
    assert_eq!(p.live_points(), pts(&[(13, 10)]).as_slice());
}

#[test]
fn w5_preparation_reaches_start() {
    let (t, mut c) = w2_world(13, 10);
    let (n, p) = compute_to(&t, &mut c, Point::new(15, 10));
    assert_eq!(n, 0);
    assert_eq!((p.cur_point, p.point_count), (0, 0));
    assert_eq!(p.flags & flags::ACTIVE, 0);
    // The probe found the start, so the target keeps
    // the requested point; the compute record's target, written before
    // the start test, is the start, and the final target takes it.
    assert_eq!(p.target(), Point::new(15, 10));
    assert_eq!(p.final_target(), Point::new(13, 10));
    // Through the request: count 0 → neutral start.
    let o = request(
        &t,
        &mut c,
        P,
        None,
        2,
        WalkTarget::Point(Point::new(15, 10)),
        false,
    )
    .unwrap();
    assert_eq!(o, Outcome::Neutral);
    assert_eq!(c.u.units[&P].mode, 1);
}

#[test]
fn w6_push_stops_at_collision() {
    let (t, mut c) = setup(40, 40, 14, 25);
    for x in 10..=20 {
        c.w.wall(x, 20);
    }
    let (n, p) = compute_to(&t, &mut c, Point::new(15, 20));
    assert_eq!(n, 1);
    assert_eq!(p.final_target(), Point::new(15, 22));
    assert_eq!(p.live_points(), pts(&[(15, 22)]).as_slice());
}

#[test]
fn w9_greedy_walk_with_duplicate_first_corner() {
    let (t, mut c) = setup(80, 80, 10, 10);
    for y in 9..=11 {
        c.w.wall(15, y);
    }
    let (n, p) = compute_to(&t, &mut c, Point::new(34, 22));
    let expect = pts(&[
        (14, 12),
        (14, 12),
        (15, 13),
        (17, 13),
        (18, 14),
        (19, 14),
        (20, 15),
        (21, 15),
        (22, 16),
        (23, 16),
        (24, 17),
        (25, 17),
        (26, 18),
        (27, 18),
        (28, 19),
        (29, 19),
        (30, 20),
        (31, 20),
        (32, 21),
        (33, 21),
    ]);
    assert_eq!(p.live_points(), expect.as_slice());
    assert_eq!(n, 20);
}

#[test]
fn far_target_clears_path() {
    let (t, mut c) = setup(300, 40, 10, 10);
    let (n, p) = compute_to(&t, &mut c, Point::new(10 + 101, 10));
    assert_eq!(n, 0);
    assert_eq!((p.cur_point, p.point_count), (0, 0));
    let (n, _) = compute_to(&t, &mut c, Point::new(10 + 100, 10));
    assert_eq!(n, 1);
}

#[test]
fn compute_owner_mismatch_is_fatal() {
    let (t, mut c) = setup(40, 40, 10, 10);
    let mut p = c.w.paths[&P].clone();
    p.owner = Some(UnitId(9));
    p.put_target(Point::new(20, 10));
    assert!(compute(&t, &mut c, &mut p, P, false).is_err());
}

#[test]
fn target_unit_position_refreshes_target() {
    let (t, mut c) = setup(40, 40, 10, 10);
    let mut m = fake::FakeUnit::player();
    m.ty = UnitType::Monster;
    m.guid = 77;
    m.pos = Point::new(20, 12);
    m.size = 0;
    c.u.units.insert(UnitId(2), m);
    let o = request(
        &t,
        &mut c,
        P,
        None,
        2,
        WalkTarget::Unit {
            ty: UnitType::Monster,
            guid: 77,
        },
        false,
    )
    .unwrap();
    assert_eq!(o, Outcome::Moving(1));
    let p = &c.w.paths[&P];
    assert_eq!(p.target(), Point::new(20, 12));
    assert_eq!(p.live_points(), pts(&[(20, 12)]).as_slice());
    assert!(p.target_unit.is_some());
    // A target unit at (0, y): result 0, nothing reset.
    c.u.unit(UnitId(2)).pos = Point::new(0, 12);
    let mut p2 = c.w.paths[&P].clone();
    let before = p2.clone();
    assert_eq!(compute(&t, &mut c, &mut p2, P, false).unwrap(), 0);
    assert_eq!(p2.point_count, before.point_count);
    // Unknown GUID: a log line, nothing else.
    let o = request(
        &t,
        &mut c,
        P,
        None,
        2,
        WalkTarget::Unit {
            ty: UnitType::Monster,
            guid: 5,
        },
        false,
    )
    .unwrap();
    assert_eq!(o, Outcome::NoTargetUnit);
}

// ---- direction vectors D1–D4 ----------------------------------------

#[test]
fn d_direction_vectors() {
    let t = tables();
    let fp = |x: i32| (x as u32) << 16;
    let dv =
        |x: i32, y: i32| direction_vector(&t, (fp(1000), fp(1000)), (fp(1000 + x), fp(1000 + y)));
    assert_eq!(dv(10, 0), ((4096, 0), 56));
    assert_eq!(dv(0, -10), ((0, -4096), 40));
    assert_eq!(dv(5, 5), ((2896, 2896), 0));
    assert_eq!(dv(3, 1), ((3888, 1286), 59));
}

// A far point wraps 127·small to a negative index, which reads from
// the rows before the table (`tan_before`). First case: the act 3
// playthrough's FrogDemon Submerge at (0, 0), seen from about tile
// (10177, 11875): index −1, row (0, 0, 0). Second: index −87, the other
// octant.
#[test]
fn d_direction_vector_far_point_reads_before_the_table() {
    let t = tables();
    assert_eq!(t.tan_before.len(), 127);
    assert_eq!(t.tan_before[126], [0, 0, 0]);
    assert_eq!(
        direction_vector(&t, (666_992_640, 778_272_768), (0x8000, 0x8000)),
        ((0, 0), 39)
    );
    assert_eq!(
        direction_vector(&t, (100_000_000, 100_000_000), (120_000_000, 119_999_999)),
        ((1, -1), 7)
    );
}

// ---- velocity V1–V4 -------------------------------------------------

// Stat 67 is the unit total; a player's base is 100 from
// creation, so V1–V3 state stat 67 = 100 / 150 /
// 150 and V3 gives p = 17 + 150 = 167 → 2565; V4 is the floor of 25
// (PQ1, answered).
#[test]
fn v_velocities() {
    let t = tables();
    let mut c = units_only();
    // V1.
    assert_eq!(c.u.units[&P].stats[&67], 100);
    assert_eq!(mode_velocity(&t, &c, P, 2), Some(0x600));
    assert_eq!(mode_velocity(&t, &c, P, 19), Some(0x1000));
    assert_eq!(run_velocity_bonus(6, 9), Some(50));
    assert_eq!(run_velocity_bonus(0, 9), None);
    // V2.
    c.u.unit(P).stats.insert(67, 150);
    assert_eq!(mode_velocity(&t, &c, P, 3), Some(0x900));
    // V3: f = 150·20/170 = 17.
    c.u.unit(P).item_stats.insert(96, 20);
    assert_eq!(mode_velocity(&t, &c, P, 3), Some(2565));
    // V4: floor 25 %.
    c.u.unit(P).item_stats.clear();
    c.u.unit(P).stats.insert(67, 10);
    assert_eq!(mode_velocity(&t, &c, P, 2), Some(384));
    // Neutral has no modifier: no rule (seam).
    assert_eq!(mode_velocity(&t, &c, P, 1), None);
}

// ---- movement M1, M2 ------------------------------------------------

fn run_ticks(t: &PathTables, c: &mut Ctx, max: usize) -> Vec<(u32, u32, Step)> {
    let mut out = Vec::new();
    for _ in 0..max {
        let s = Walk { t, c: &mut *c }.player_event0(P).unwrap();
        let p = &c.w.paths[&P];
        out.push((p.precise_x, p.precise_y, s));
        if s == Step::Stopped {
            break;
        }
    }
    out
}

#[test]
fn m1_walk_per_tick() {
    let (t, mut c) = setup(200, 200, 100, 100);
    let (r, o) = handle_message(&t, &mut c, P, 0x01, 105, 100).unwrap();
    assert_eq!((r, o), (0, Some(Outcome::Moving(1))));
    assert_eq!(c.u.units[&P].mode, 2);
    assert_eq!(c.w.paths[&P].velocity, 0x600);
    let ticks = run_ticks(&t, &mut c, 20);
    assert_eq!(ticks.len(), 14);
    for (k, &(x, y, s)) in ticks.iter().take(13).enumerate() {
        assert_eq!(x, 0x64E000 + k as u32 * 0x6000, "tick {}", k + 1);
        assert_eq!(y, 0x648000);
        assert_eq!(s, Step::Moving);
    }
    assert_eq!(ticks[13], (0x698000, 0x648000, Step::Stopped));
    let p = &c.w.paths[&P];
    assert_eq!(
        (p.cur_point, p.point_count, p.flags & flags::ACTIVE),
        (0, 0, 0)
    );
    assert_eq!(c.u.units[&P].mode, 1);
    // The footprint moved with the unit: the old centre is clear, the
    // new one carries the player marker.
    assert_eq!(c.w.value(Point::new(100, 100)) & 0x1000, 0);
    assert_eq!(c.w.value(Point::new(105, 100)) & 0x1080, 0x1080);
}

#[test]
fn m2_run_per_tick() {
    let (t, mut c) = setup(200, 200, 100, 100);
    let (_, o) = handle_message(&t, &mut c, P, 0x03, 103, 101).unwrap();
    assert_eq!(o, Some(Outcome::Moving(1)));
    assert_eq!(c.u.units[&P].mode, 3);
    assert!(c.u.log.contains(&"run list 50".to_string()));
    assert_eq!(c.w.paths[&P].velocity, 0x900);
    let ticks = run_ticks(&t, &mut c, 20);
    let expect = [
        (0x6508B0, 0x64AD36),
        (0x659160, 0x64DA6C),
        (0x661A10, 0x6507A2),
        (0x66A2C0, 0x6534D8),
        (0x672B1F, 0x656301),
        (0x678000, 0x658000),
    ];
    assert_eq!(ticks.len(), 6);
    for (k, &(x, y, _)) in ticks.iter().enumerate() {
        assert_eq!((x, y), expect[k], "tick {}", k + 1);
    }
    assert_eq!(ticks[5].2, Step::Stopped);
    // Five run-drain ticks of 40 (S1) before the stop tick's drain too.
    assert_eq!(c.u.units[&P].stats[&10], (100 << 8) - 6 * 40);
}

#[test]
fn blocked_cell_stops_at_last_free_centre() {
    let (t, mut c) = setup(200, 200, 100, 100);
    request(
        &t,
        &mut c,
        P,
        None,
        2,
        WalkTarget::Point(Point::new(110, 100)),
        false,
    )
    .unwrap();
    // A wall appears after the path was computed: the plus at 103 hits it.
    c.w.wall(104, 100);
    let ticks = run_ticks(&t, &mut c, 40);
    let last = *ticks.last().unwrap();
    assert_eq!(last.2, Step::Stopped);
    assert_eq!((last.0, last.1), (0x668000, 0x648000)); // centre of 102
    assert_eq!(c.w.paths[&P].point_count, 0);
    assert_eq!(c.u.units[&P].mode, 1);
    // Footprint stays on 102.
    assert_eq!(c.w.value(Point::new(102, 100)) & 0x1000, 0x1000);
}

// ---- stamina S1, S2 -------------------------------------------------

#[test]
fn s_run_drain() {
    let (t, mut c) = setup(40, 40, 10, 10);
    let p = c.w.paths[&P].clone();
    c.u.walk_velocity = (6, 9, 20);
    c.u.unit(P).stats.insert(10, 1000);
    assert!(!Walk { t: &t, c: &mut c }.run_drain(P, &p));
    assert_eq!(c.u.units[&P].stats[&10], 960);
    c.u.unit(P).torso_speed = Some(10);
    c.u.unit(P).item_stats.insert(154, 25);
    assert!(!Walk { t: &t, c: &mut c }.run_drain(P, &p));
    assert_eq!(c.u.units[&P].stats[&10], 900);
    // Exhausted: stamina := 0.
    c.u.unit(P).stats.insert(10, 30);
    assert!(Walk { t: &t, c: &mut c }.run_drain(P, &p));
    assert_eq!(c.u.units[&P].stats[&10], 0);
    // Town: no drain.
    c.w.town = true;
    c.u.unit(P).stats.insert(10, 30);
    assert!(!Walk { t: &t, c: &mut c }.run_drain(P, &p));
    assert_eq!(c.u.units[&P].stats[&10], 30);
}

#[test]
fn run_without_stamina_is_walk_and_town_walk() {
    let (t, mut c) = setup(200, 200, 100, 100);
    c.u.unit(P).stats.insert(10, 0);
    request(
        &t,
        &mut c,
        P,
        None,
        3,
        WalkTarget::Point(Point::new(105, 100)),
        false,
    )
    .unwrap();
    assert_eq!(c.u.units[&P].mode, 2);
    c.w.town = true;
    request(
        &t,
        &mut c,
        P,
        None,
        2,
        WalkTarget::Point(Point::new(108, 100)),
        false,
    )
    .unwrap();
    assert_eq!(c.u.units[&P].mode, 6);
}

// ---- mode and interrupt checks ---------------------------

#[test]
fn mode_check_rules() {
    let mut c = units_only();
    c.g.frame = 100;
    c.u.unit(P).cursor = true;
    assert!(!mode_check(&c, P, 2));
    assert!(mode_check(&c, P, 17));
    assert!(mode_check(&c, P, 0));
    c.u.unit(P).cursor = false;
    for m in [0, 1, 5, 17] {
        c.u.unit(P).mode = 0;
        assert!(mode_check(&c, P, m));
    }
    for cur in [0, 4, 9, 17] {
        c.u.unit(P).mode = cur;
        assert!(!mode_check(&c, P, 2));
    }
    // Attack: frame ≤ E + 5.
    c.u.unit(P).mode = 7;
    c.u.type1_expire = 94;
    assert!(!mode_check(&c, P, 2));
    assert!(mode_check(&c, P, 4));
    c.u.type1_expire = 95;
    assert!(mode_check(&c, P, 2));
    c.u.unit(P).mode = 12;
    c.u.type1_expire = 0;
    assert!(!mode_check(&c, P, 4));
    // S1 Amazon never, S3 Druid never.
    c.u.unit(P).mode = 13;
    assert!(!mode_check(&c, P, 2));
    c.u.unit(P).class = 1;
    assert!(mode_check(&c, P, 2));
    c.u.unit(P).mode = 15;
    c.u.unit(P).class = 5;
    assert!(!mode_check(&c, P, 2));
    // SQ: SeqInput > 0.
    c.u.unit(P).mode = 18;
    assert!(!mode_check(&c, P, 2));
    c.u.unit(P).used_skill = Some(UsedSkill {
        seq_input: 1,
        ..UsedSkill::default()
    });
    assert!(mode_check(&c, P, 2));
    for cur in [1, 2, 3, 5, 6, 14, 16, 19, 25] {
        c.u.unit(P).mode = cur;
        assert!(mode_check(&c, P, 2));
    }
}

#[test]
fn interrupt_check_rules() {
    let (t, mut c) = setup(40, 40, 10, 10);
    c.g.frame = 100;
    c.u.unit(P).mode = 2;
    // Rule 1.
    c.u.unit(P).states = vec![54];
    assert!(!interrupt_check(&t, &mut c, P, 2, None).unwrap());
    c.u.unit(P).states.clear();
    // Rule 2: no used skill.
    assert!(interrupt_check(&t, &mut c, P, 2, None).unwrap());
    // Rule 3: srvdofunc 67 with the same skill.
    c.u.unit(P).used_skill = Some(UsedSkill {
        id: 9,
        srvdofunc: 67,
        ..UsedSkill::default()
    });
    assert!(interrupt_check(&t, &mut c, P, 2, Some(9)).unwrap());
    // Rule 4: interrupt clear, not neutral, m = 2 → refuse.
    assert!(!interrupt_check(&t, &mut c, P, 2, None).unwrap());
    // Neutral → re-enter neutral (mode 1 start) and allow.
    c.u.unit(P).mode = 1;
    assert!(interrupt_check(&t, &mut c, P, 2, None).unwrap());
    assert!(c.u.log.contains(&"start 1".to_string()));
    // Rule 5: interrupt set, concentration with stat 164 = 100: one draw,
    // r < 100 → rule 6; mode 2 → refuse.
    c.u.unit(P).mode = 2;
    c.u.unit(P).used_skill = Some(UsedSkill {
        id: 9,
        interrupt: true,
        ..UsedSkill::default()
    });
    c.u.unit(P).states = vec![42];
    c.u.unit(P).state_stats.insert((42, 164), 100);
    let before = c.u.units[&P].seed;
    assert!(!interrupt_check(&t, &mut c, P, 2, None).unwrap());
    let mut s = before;
    s.roll(100);
    assert_eq!(c.u.units[&P].seed, s);
    // v = 0: no rule 6, allow.
    c.u.unit(P).state_stats.insert((42, 164), 0);
    assert!(interrupt_check(&t, &mut c, P, 2, None).unwrap());
    // State 15 → rule 6 → refuse in mode 2; no draw.
    c.u.unit(P).states = vec![15];
    let before = c.u.units[&P].seed;
    assert!(!interrupt_check(&t, &mut c, P, 2, None).unwrap());
    assert_eq!(c.u.units[&P].seed, before);
    // No state: allow.
    c.u.unit(P).states.clear();
    assert!(interrupt_check(&t, &mut c, P, 2, None).unwrap());
}

#[test]
fn refused_request_changes_nothing() {
    let (t, mut c) = setup(40, 40, 10, 10);
    c.u.unit(P).mode = 4; // GH
    let before = c.w.paths[&P].clone();
    let o = request(
        &t,
        &mut c,
        P,
        None,
        2,
        WalkTarget::Point(Point::new(20, 10)),
        false,
    )
    .unwrap();
    assert_eq!(o, Outcome::ModeRefused);
    assert_eq!(c.w.paths[&P], before);
    c.u.unit(P).mode = 2;
    c.u.unit(P).states = vec![54];
    let o = request(
        &t,
        &mut c,
        P,
        None,
        2,
        WalkTarget::Point(Point::new(20, 10)),
        false,
    )
    .unwrap();
    assert_eq!(o, Outcome::InterruptRefused);
}

#[test]
fn request_while_moving_keeps_fraction() {
    let (t, mut c) = setup(200, 200, 100, 100);
    request(
        &t,
        &mut c,
        P,
        None,
        2,
        WalkTarget::Point(Point::new(110, 100)),
        false,
    )
    .unwrap();
    run_ticks(&t, &mut c, 3);
    let x = c.w.paths[&P].precise_x;
    assert_eq!(x & 0xFFFF, (0x648000u32 + 3 * 0x6000) & 0xFFFF);
    request(
        &t,
        &mut c,
        P,
        None,
        2,
        WalkTarget::Point(Point::new(101, 106)),
        false,
    )
    .unwrap();
    let p = &c.w.paths[&P];
    assert_eq!(p.precise_x, x);
    assert_eq!(p.live_points(), pts(&[(101, 106)]).as_slice());
}

// ---- room change --------------------------------------

#[test]
fn room_change_sends_merge_of_client_arrays() {
    let (t, mut c) = setup(40, 20, 18, 10);
    c.w.room0 = TileRect::new(0, 0, 20, 20);
    let r1 = RoomId(1);
    c.w.rooms.insert(r1, TileRect::new(20, 0, 20, 20));
    c.w.clients.insert(ROOM, vec![ClientId(1), ClientId(2)]);
    c.w.clients.insert(r1, vec![ClientId(2), ClientId(3)]);
    c.u.client_players.insert(ClientId(1), P);
    c.u.client_players.insert(ClientId(3), UnitId(9));
    request(
        &t,
        &mut c,
        P,
        None,
        2,
        WalkTarget::Point(Point::new(24, 10)),
        false,
    )
    .unwrap();
    assert_ne!(c.w.paths[&P].flags & flags::OUTSIDE_ROOM, 0);
    run_ticks(&t, &mut c, 30);
    let p = &c.w.paths[&P];
    assert_eq!(p.room, Some(r1));
    assert_eq!(p.prev_room, Some(ROOM));
    assert_eq!(p.flags & flags::ROOM_CHANGED, 0);
    assert!(c.w.log.contains(&"leave 1 0".to_string()));
    assert!(c.w.log.contains(&"insert 1 1".to_string()));
    // Client 1 (the unit's own) gets no removal; 2 is in both; 3 gets the add.
    let msgs: Vec<&String> =
        c.u.log
            .iter()
            .filter(|l| l.starts_with("add") || l.starts_with("remove"))
            .collect();
    assert_eq!(msgs, vec!["add 1 to 3"]);
}

#[test]
fn room_change_without_previous_room_only_adds() {
    // A previous room that is no longer a room of the unit's
    // act counts as none, so no client gets a removal; every client of
    // the new room but the unit's own gets the add messages.
    let t = tables();
    let mut c = units_only();
    let (r0, r1) = (RoomId(0), RoomId(1));
    c.w.clients.insert(r0, vec![ClientId(2)]);
    c.w.clients.insert(r1, vec![ClientId(1), ClientId(3)]);
    c.u.client_players.insert(ClientId(1), P);
    let mut p = DynamicPath {
        owner: Some(P),
        room: Some(r1),
        prev_room: Some(r0),
        flags: flags::ROOM_CHANGED,
        ..DynamicPath::default()
    };
    let msgs = |c: &Ctx| -> Vec<String> {
        c.u.log
            .iter()
            .filter(|l| l.starts_with("add") || l.starts_with("remove"))
            .cloned()
            .collect()
    };
    let mut q = p.clone();
    Walk { t: &t, c: &mut c }.room_change_messages(P, &mut q);
    assert_eq!(msgs(&c), ["remove 1 to 2", "add 1 to 3"]);
    c.u.log.clear();
    c.w.other_act.push(r0);
    Walk { t: &t, c: &mut c }.room_change_messages(P, &mut p);
    assert_eq!(msgs(&c), ["add 1 to 3"]);
    assert_eq!(p.flags & flags::ROOM_CHANGED, 0);
}

// ---- re-path --------------------------------------------------

#[test]
fn repath_without_budget_stops() {
    // Only a monster tests the re-path budget (path +0x94).
    let (t, mut c) = setup(40, 40, 10, 10);
    c.u.unit(P).ty = UnitType::Monster;
    let mut p = c.w.paths[&P].clone();
    p.set_path_type(&t, false, 2).unwrap();
    p.repath_budget = 0;
    let r = Walk { t: &t, c: &mut c }.repath(P, &mut p, false).unwrap();
    assert_eq!(r, 0);
    assert!(c.w.log.is_empty());
    // With budget: queue, unit flag, budget −= index (clamped at 0), the
    // distance budget (+0x90) untouched, compute.
    p.repath_budget = 1;
    p.put_target(Point::new(20, 10));
    p.put_final_target(Point::new(20, 10));
    p.dist_budget = 5;
    p.cur_point = 2;
    let r = Walk { t: &t, c: &mut c }.repath(P, &mut p, false).unwrap();
    assert_eq!(r, 1);
    assert_eq!(p.repath_budget, 0);
    assert_eq!(p.dist_budget, 5);
    assert!(c.w.log.contains(&"queue 1".to_string()));
    // A player skips the budget test: budget 0 still re-paths.
    let (t, mut c) = setup(40, 40, 10, 10);
    let mut p = c.w.paths[&P].clone();
    p.put_target(Point::new(20, 10));
    assert_eq!(p.repath_budget, 0);
    let r = Walk { t: &t, c: &mut c }.repath(P, &mut p, false).unwrap();
    assert_eq!(r, 1);
    assert!(c.w.log.contains(&"queue 1".to_string()));
}

// ---- facing -------------------------------------------

#[test]
fn facing_turn_step_and_face_away() {
    use super::geom::set_facing;
    use super::velocity::aim;
    let t = tables();
    let mut p = DynamicPath {
        owner: Some(P),
        ..DynamicPath::default()
    };
    p.direction = 10;
    set_facing(&t, &mut p, UnitType::Player, 20);
    assert_eq!((p.direction, p.new_direction, p.turn_step), (10, 20, 4)); // dirdiff[10]
    p.turn_step = 0;
    set_facing(&t, &mut p, UnitType::Player, 20 + 64);
    assert_eq!(p.turn_step, 0); // same new direction: unchanged
    set_facing(&t, &mut p, UnitType::Item, 33);
    assert_eq!(p.direction, 33);
    p.flags = 0x40;
    set_facing(&t, &mut p, UnitType::Missile, 5);
    assert_eq!(p.direction, 33);
    p.flags = 0;
    set_facing(&t, &mut p, UnitType::Missile, 5);
    assert_eq!(p.direction, 5);
    // Flag 0x200: the computed direction − 32. (0,0)→(10,0) is 56.
    let mut p = DynamicPath {
        owner: Some(P),
        ..DynamicPath::default()
    };
    p.precise_x = centre(100);
    p.precise_y = centre(100);
    p.velocity = 0x600;
    p.point_count = 1;
    p.points[0] = PathPoint { x: 110, y: 100 };
    aim(&t, &mut p, UnitType::Player);
    assert_eq!(p.new_direction, 56);
    assert_eq!((p.vel_vec_x, p.vel_vec_y), (0x6000, 0));
    p.flags = flags::FACE_AWAY;
    aim(&t, &mut p, UnitType::Player);
    assert_eq!(p.new_direction, 24);
}

// ---- target unit -------------------------

#[test]
fn target_unit_check_and_stop_distance() {
    let (t, mut c) = setup(200, 200, 100, 100);
    let mut m = fake::FakeUnit::player();
    m.ty = UnitType::Monster;
    m.guid = 77;
    m.pos = Point::new(110, 100);
    m.size = 2;
    c.u.units.insert(UnitId(2), m);
    let o = request(
        &t,
        &mut c,
        P,
        None,
        2,
        WalkTarget::Unit {
            ty: UnitType::Monster,
            guid: 77,
        },
        false,
    )
    .unwrap();
    assert_eq!(o, Outcome::Moving(1));
    assert!(c.u.log.contains(&"clear queued".to_string()));
    // Stop distance 5: the unit stops once the unit distance is ≤ 5.
    c.w.paths.get_mut(&P).unwrap().stop_distance = 5;
    let ticks = run_ticks(&t, &mut c, 40);
    assert_eq!(ticks.last().unwrap().2, Step::Stopped);
    let cell = c.w.paths[&P].cell();
    let d = super::geom::unit_distance(&t, cell, 2, Point::new(110, 100), 2);
    assert!(d <= 5, "stopped at {cell:?}, distance {d}");
    assert!(cell.x < 109);
    // Target check: a GUID that now names another unit drops the target.
    let mut p = c.w.paths[&P].clone();
    p.target_unit = Some(super::seams::TargetUnit {
        unit: UnitId(3),
        ty: UnitType::Monster,
        guid: 77,
    });
    Walk { t: &t, c: &mut c }.target_check(&mut p);
    assert_eq!(p.target_unit, None);
    // An item in mode 1 or 2 (equipped, belt) is dropped too.
    let mut it = fake::FakeUnit::player();
    it.ty = UnitType::Item;
    it.guid = 5;
    it.mode = 1;
    c.u.units.insert(UnitId(4), it);
    p.target_unit = Some(super::seams::TargetUnit {
        unit: UnitId(4),
        ty: UnitType::Item,
        guid: 5,
    });
    Walk { t: &t, c: &mut c }.target_check(&mut p);
    assert_eq!(p.target_unit, None);
    c.u.unit(UnitId(4)).mode = 3;
    p.target_unit = Some(super::seams::TargetUnit {
        unit: UnitId(4),
        ty: UnitType::Item,
        guid: 5,
    });
    Walk { t: &t, c: &mut c }.target_check(&mut p);
    assert!(p.target_unit.is_some());
}

// ---- cell walk limit, edge case 5 ----------------------------------

// Edge case 5 needs a step over 10 sub-tiles; arrival rule 2.1 computes
// (m · direction) in 32 bits, which wraps above 8 sub-tiles per tick on an
// axis, so the cell walk is driven directly with a 12-cell Δ here.
#[test]
fn cell_walk_stops_testing_after_ten_cells() {
    let (t, mut c) = setup(200, 200, 100, 100);
    let mut p = c.w.paths[&P].clone();
    // A wall at 113 touches the plus at 112: cells 101..110 are tested,
    // 111 and 112 are not.
    c.w.wall(113, 100);
    let r = Walk { t: &t, c: &mut c }
        .cell_walk(P, &mut p, (12 << 16, 0))
        .unwrap();
    assert_eq!(r, Ok((centre(112), centre(100))));
    assert_eq!(p.saved_count, 10);
    assert_eq!(p.saved_steps[0], PathPoint { x: 101, y: 100 });
    assert_eq!(p.saved_steps[9], PathPoint { x: 110, y: 100 });
    assert_ne!(p.flags & flags::MOVED, 0);
    // The footprint stopped at the last tested cell.
    assert_eq!(c.w.value(Point::new(110, 100)) & 0x1000, 0x1000);
    // A wall inside the first ten cells blocks: Q = centre of the last
    // free cell.
    let (t, mut c) = setup(200, 200, 100, 100);
    let mut p = c.w.paths[&P].clone();
    c.w.wall(106, 100);
    let r = Walk { t: &t, c: &mut c }
        .cell_walk(P, &mut p, (12 << 16, 0))
        .unwrap();
    assert_eq!(r, Err((centre(104), centre(100))));
    assert_eq!(p.collided_mask, 0x1);
}

// ---- next-position check, start (0,0) ----------------

#[test]
fn zero_velocity_probe_and_origin_start() {
    let (t, mut c) = setup(40, 40, 10, 10);
    // Velocity 0: P := position, blocked; greedy walk from the start.
    c.w.paths.get_mut(&P).unwrap().velocity = 0;
    let (n, p) = compute_to(&t, &mut c, Point::new(13, 10));
    assert_eq!(
        (n, p.live_points().as_slice()),
        (1, pts(&[(13, 10)]).as_slice())
    );
    // Greedy diagonal then straight: (10,10)→(14,12) gives the corner
    // where the direction changes and the tail end.
    // Greedy from the start to (14,12): octants 22, 24, 23, 18 turn at
    // every step, so each corner is a point and the target itself is not.
    let (n, p) = compute_to(&t, &mut c, Point::new(14, 12));
    assert_eq!(
        (n, p.live_points().as_slice()),
        (3, pts(&[(11, 10), (12, 11), (13, 11)]).as_slice())
    );
    // The unit's own footprint is back after the compute (steps 6, 9).
    assert_eq!(c.w.value(Point::new(10, 10)) & 0x1080, 0x1080);
    // A unit at (0, 0): no path.
    let (t, mut c) = setup(40, 40, 0, 0);
    let (n, p) = compute_to(&t, &mut c, Point::new(5, 5));
    assert_eq!((n, p.point_count), (0, 0));
}

// A straight path onto an occupied
// neighbouring cell. The first probe steps back onto the start, the
// compute returns 0 and the target keeps the requested cell (as
// measured: Warriv's walk in radius onto a zombie's cell).
#[test]
fn preparation_onto_start_keeps_the_target() {
    let (t, mut c) = setup(40, 40, 14, 10);
    c.w.wall(15, 9);
    let (n, p) = compute_to(&t, &mut c, Point::new(15, 9));
    assert_eq!(n, 0);
    assert_eq!(p.target(), Point::new(15, 9));
    assert_eq!(p.flags & flags::ACTIVE, 0);
}

/// A doorway three cells wide in a wall column at x = 65 (open at
/// y 22..=24): the player's plus fits only on row 23. A straight walk
/// from the centre of (60, 22) to (69, 23) clips the corner of (65, 22),
/// whose plus touches the wall at (65, 21), for a hair before it crosses
/// into row 23. Whether a tick's step ends inside that corner depends on
/// the tick rate; the walk must not: it slides past the corner and
/// reaches the target at every rate.
#[test]
fn a_walk_through_a_doorway_corner_does_not_depend_on_the_tick_rate() {
    let walk = |hz: u32| {
        let (t, mut c) = setup(200, 200, 60, 22);
        c.g.tick_rate = crate::tick::TickRate::new(hz).unwrap();
        for y in 0..200 {
            if !(22..=24).contains(&y) {
                c.w.wall(65, y);
            }
        }
        request(
            &t,
            &mut c,
            P,
            None,
            2,
            WalkTarget::Point(Point::new(69, 23)),
            false,
        )
        .unwrap();
        for _ in 0..600 {
            c.g.frame += 1;
            let s = Walk { t: &t, c: &mut c }.player_event0(P).unwrap();
            if s == Step::Stopped {
                break;
            }
        }
        let p = &c.w.paths[&P];
        (p.precise_x >> 16, p.precise_y >> 16)
    };
    for hz in [25, 60, 144] {
        assert_eq!(walk(hz), (69, 23), "{hz} Hz");
    }
}

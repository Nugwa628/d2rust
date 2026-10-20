//! Gap tests: walk rules no earlier test claimed (path compute text,
//! circling text, A* r1, velocity r3, unit distance text and r1, footprint move r1, edge case 11). Rule
//! text and table values only; synthetic grids of [`super::fake`].

use std::collections::BTreeMap;

use super::fake::{Ctx, FakeUnit, FakeWorld, ROOM};
use super::{setup, tables, P};
use crate::drlg::{CollisionGrid, TileRect};
use crate::path::collision::CollisionRooms;
use crate::path::coords::to_fp16_center as centre;
use crate::path::footprint::{self, FootShape, Footprint, RemoveRule};
use crate::path::record::{flags, DynamicPath, PathPoint};
use crate::path::tables::PathTables;
use crate::path::walk::find::{compute, toward, Finder};
use crate::path::walk::geom::unit_distance;
use crate::path::walk::seams::{PathInfo, PathWorld, Point, TargetUnit, WalkUnits};
use crate::path::walk::step::{Walk, STEP_BASE};
use crate::path::walk::velocity::set_velocity;
use crate::rng::Seed;
use crate::units::{ClientId, RoomId, UnitId, UnitType};

/// A context over [`FakeWorld`] with typed unit positions (targets of
/// Step 4) and one seed.
pub(super) struct Recorder {
    w: FakeWorld,
    units: BTreeMap<UnitId, (UnitType, Point)>,
    seed: Seed,
}

impl Recorder {
    fn new(w: FakeWorld) -> Recorder {
        Recorder {
            w,
            units: BTreeMap::from([(P, (UnitType::Player, Point::new(0, 0)))]),
            seed: Seed::default(),
        }
    }
}

fn footprint_of(p: &DynamicPath) -> Footprint {
    Footprint {
        room: p.room,
        x: p.x(),
        y: p.y(),
        shape: FootShape::Pattern(p.pattern),
        mask: p.foot_mask,
    }
}

impl CollisionRooms for Recorder {
    fn subtile_rect(&self, room: RoomId) -> Option<TileRect> {
        self.w.subtile_rect(room)
    }
    fn adjacent_count(&self, room: RoomId) -> usize {
        self.w.adjacent_count(room)
    }
    fn adjacent(&self, room: RoomId, i: usize) -> Option<RoomId> {
        self.w.adjacent(room, i)
    }
    fn grid(&self, room: RoomId) -> Option<&CollisionGrid> {
        self.w.grid(room)
    }
    fn grid_mut(&mut self, room: RoomId) -> Option<&mut CollisionGrid> {
        self.w.grid_mut(room)
    }
}

impl PathWorld for Recorder {
    fn load_path(&self, unit: UnitId) -> Option<DynamicPath> {
        self.w.paths.get(&unit).cloned()
    }
    fn store_path(&mut self, unit: UnitId, path: &DynamicPath) {
        self.w.paths.insert(unit, path.clone());
    }
    fn room_in_town(&self, _room: RoomId) -> bool {
        false
    }
    fn remove_footprint(&mut self, unit: UnitId, force: bool) -> bool {
        let Some(fp) = self.w.paths.get(&unit).map(footprint_of) else {
            return false;
        };
        footprint::remove_footprint(&mut self.w, &fp, RemoveRule::Other, force)
    }
    fn add_footprint(&mut self, unit: UnitId) {
        if let Some(fp) = self.w.paths.get(&unit).map(footprint_of) {
            footprint::add_footprint(&mut self.w, &fp);
        }
    }
    fn room_list_remove(&mut self, _unit: UnitId, _room: RoomId) {}
    fn room_list_insert(&mut self, _unit: UnitId, _room: RoomId) {}
    fn queue_for_update(&mut self, _unit: UnitId) {}
    fn room_clients(&self, _room: RoomId) -> Vec<ClientId> {
        Vec::new()
    }
}

impl WalkUnits for Recorder {
    fn tick_rate(&self) -> crate::tick::TickRate {
        crate::tick::TickRate::BASE
    }
    fn unit_type(&self, unit: UnitId) -> UnitType {
        self.units[&unit].0
    }
    fn frame(&self) -> i32 {
        0
    }
    fn mode(&self, _unit: UnitId) -> u32 {
        1
    }
    fn position(&self, unit: UnitId) -> Point {
        self.units[&unit].1
    }
    fn seed(&mut self, _unit: UnitId) -> &mut Seed {
        &mut self.seed
    }
}

#[test]
fn ida_star_receives_the_slack_of_step_4_doubled() {
    // Test vector through the compute: type 0 from (10, 10) to
    // (13, 11) on a free grid, slack r = 1 (doubled 2): (12, 10), (13,
    // 11).
    let t = tables();
    let mut w = FakeWorld::new(40, 40);
    w.add_player(&t, P, 10, 10);
    let mut path = w.paths[&P].clone();
    path.set_path_type(&t, true, 0).unwrap();
    path.put_target(Point::new(13, 11));
    let mut c = Recorder::new(w);
    assert_eq!(compute(&t, &mut c, &mut path, P, false).unwrap(), 2);
    assert_eq!(
        (path.point(0), path.point(1)),
        (Point::new(12, 10), Point::new(13, 11))
    );
    // An item target: slack r = 2, doubled 4: the search
    // stops at (12, 10) (h 3 < 4), one recorded point, so 0.
    let item = UnitId(5);
    c.units.insert(item, (UnitType::Item, Point::new(13, 11)));
    path.target_unit = Some(TargetUnit {
        unit: item,
        ty: UnitType::Item,
        guid: 5,
    });
    assert_eq!(compute(&t, &mut c, &mut path, P, false).unwrap(), 0);
}

#[test]
fn toward_with_a_direction_offset_runs_the_circling_function() {
    let t = tables();
    let mut base = DynamicPath {
        precise_x: centre(10),
        precise_y: centre(10),
        velocity: 0x800,
        room: Some(ROOM),
        pattern: t.pattern_of_size[2],
        move_mask: 0x1C09,
        owner: Some(P),
        ..DynamicPath::default()
    };
    base.update_client();
    let info = PathInfo {
        start: Point::new(10, 10),
        target: Point::new(20, 10),
        start_room: Some(ROOM),
        target_room: Some(ROOM),
        slack: 1,
        max_distance: 14,
        idastar_score: 0,
        path_type: 5,
        size: 2,
        pattern: base.pattern,
        move_mask: 0x1C09,
    };
    // Type 5 (k = +2), free grid, 14 iterations (synthetic, `testdir`
    // rows 22, 21, 20, 15, 10 turned by 2): down from the start (which
    // is appended), diagonal from (10, 16), right from (15, 21), tail at
    // (18, 21). Index and count were reset first.
    let mut path = base.clone();
    path.set_path_type(&t, false, 5).unwrap();
    assert_eq!(path.dir_offset, 2);
    path.cur_point = 3;
    path.point_count = 5;
    let mut c = Recorder::new(FakeWorld::new(40, 40));
    let mut f = Finder {
        t: &t,
        c: &mut c,
        owner_ty: UnitType::Monster,
    };
    assert_eq!(toward(&mut f, &mut path, &info).unwrap(), 4);
    assert_eq!((path.cur_point, path.point_count), (0, 4));
    assert_eq!(
        path.live_points(),
        [
            Point::new(10, 10),
            Point::new(10, 16),
            Point::new(15, 21),
            Point::new(18, 21)
        ]
    );
    // Type 12 (back-up turn): k is −4 again after it, and its walks
    // append after the first (which starts with the start).
    let mut path = base.clone();
    path.set_path_type(&t, false, 12).unwrap();
    let mut f = Finder {
        t: &t,
        c: &mut c,
        owner_ty: UnitType::Monster,
    };
    let info12 = PathInfo {
        path_type: 12,
        ..info
    };
    let n = crate::path::walk::other::back_up_turn(&mut f, &mut path, &info12).unwrap();
    assert!(n > 2, "{n}");
    assert_eq!(path.dir_offset, -4);
    assert_eq!(path.point(0), Point::new(10, 10));
    // Offset 0 (type 2): no circling; index and count still 0.
    let mut path = base.clone();
    path.set_path_type(&t, false, 2).unwrap();
    path.cur_point = 3;
    path.point_count = 5;
    let mut f = Finder {
        t: &t,
        c: &mut c,
        owner_ty: UnitType::Monster,
    };
    let info = PathInfo {
        path_type: 2,
        ..info
    };
    assert_eq!(toward(&mut f, &mut path, &info).unwrap(), 1);
    assert_eq!((path.cur_point, path.point_count), (0, 0));
    assert_eq!(path.point(0), Point::new(20, 10));
}

const M: UnitId = UnitId(2);

/// A player at (10, 20) with a type-1 (A*) path; walls at the given
/// cells; a monster `M` at (20, 20).
fn astar_setup(walls: &[(i32, i32)]) -> (PathTables, Ctx, DynamicPath) {
    let (t, mut c) = setup(40, 40, 10, 20);
    for &(x, y) in walls {
        c.w.wall(x, y);
    }
    let mut monster = FakeUnit::player();
    monster.ty = UnitType::Monster;
    monster.guid = 7;
    monster.pos = Point::new(20, 20);
    c.u.units.insert(M, monster);
    let mut path = c.w.paths[&P].clone();
    path.set_path_type(&t, true, 1).unwrap();
    (t, c, path)
}

/// The eight probe cells of A* rule 1 around (20, 20).
const PROBES: [(i32, i32); 8] = [
    (18, 18),
    (18, 22),
    (22, 18),
    (22, 22),
    (18, 20),
    (20, 18),
    (22, 20),
    (20, 22),
];

#[test]
fn astar_target_room_check_needs_a_free_probe_cell() {
    let target_unit = Some(TargetUnit {
        unit: M,
        ty: UnitType::Monster,
        guid: 7,
    });
    // Every probe cell blocked, with a target unit: A* returns 0.
    let (t, mut c, mut path) = astar_setup(&PROBES);
    path.target_unit = target_unit;
    assert_eq!(compute(&t, &mut c, &mut path, P, false).unwrap(), 0);
    assert_eq!(path.point_count, 0);
    // The same grid with a point target: the check does not run.
    let (t, mut c, mut path) = astar_setup(&PROBES);
    path.put_target(Point::new(20, 20));
    assert!(compute(&t, &mut c, &mut path, P, false).unwrap() > 0);
    // One probe cell free (the last tested, (0, +2)): A* runs.
    let (t, mut c, mut path) = astar_setup(&PROBES[..7]);
    path.target_unit = target_unit;
    assert!(compute(&t, &mut c, &mut path, P, false).unwrap() > 0);
}

#[test]
fn velocity_setter_marks_a_change_and_sets_the_max() {
    let mut p = DynamicPath {
        velocity: 0x600,
        max_velocity: 0x600,
        ..DynamicPath::default()
    };
    set_velocity(&mut p, 0x900);
    assert_eq!((p.field_38, p.velocity, p.max_velocity), (15, 0x900, 0x900));
    // The same value again: +0x38 is not set.
    p.field_38 = 0;
    set_velocity(&mut p, 0x900);
    assert_eq!((p.field_38, p.velocity), (0, 0x900));
}

#[test]
fn unit_distance_table_and_formula() {
    let t = tables();
    let d = |dx: i32, dy: i32, sa: i32, sb: i32| {
        unit_distance(&t, Point::new(50 + dx, 50 + dy), sa, Point::new(50, 50), sb)
    };
    // Both Δ < 8 and both sizes < 4: `dist8_unit`[Δx + 8Δy]
    // (rows 0 = −1, 3 = 0, 4 = 2, 19 = 2).
    assert_eq!(d(4, 0, 2, 2), 2);
    assert_eq!(d(3, 2, 2, 2), 2);
    assert_eq!(d(0, 0, 2, 2), 0); // negative → 0
    assert_eq!(d(2, 0, 2, 1), 0); // negative: no +1 for a size < 2
    assert_eq!(d(0, 0, 1, 1), 0);
    assert_eq!(d(4, 0, 3, 2), 1); // size 3: minus 1
    assert_eq!(d(4, 0, 2, 3), 1);
    assert_eq!(d(3, 0, 3, 2), 0); // not below 0
    assert_eq!(d(4, 0, 1, 2), 3); // size < 2: + 1
    assert_eq!(d(3, 0, 3, 1), 1); // 0 − 1 → 0, then + 1
                                  // Otherwise: Δ − (s1/2 + s2/2) per axis (not below 0), 2·max + min.
    assert_eq!(d(10, 3, 2, 4), 14); // (7, 0)
    assert_eq!(d(8, 5, 2, 2), 2 * 6 + 3); // Δx = 8 leaves the table: (6, 3)
    assert_eq!(d(4, 0, 4, 2), 2); // size 4 leaves the table: (1, 0)
    assert_eq!(d(1, 1, 5, 5), 0);
}

/// A unit at (10, 10) moving toward (14, 10): one point, velocity
/// vector along +x from `velocity`.
fn moving(c: &Ctx, velocity: i32) -> DynamicPath {
    let mut p = c.w.paths[&P].clone();
    p.points[0] = PathPoint::from_point(Point::new(14, 10));
    p.put_final_target(Point::new(14, 10));
    p.point_count = 1;
    p.cur_point = 0;
    p.velocity = velocity;
    p.dir_vec_x = 4096;
    p.dir_vec_y = 0;
    p.flags |= flags::ACTIVE;
    p
}

fn movement(t: &PathTables, c: &mut Ctx, p: &mut DynamicPath) -> bool {
    Walk { t, c }.movement(P, p, STEP_BASE).unwrap()
}

#[test]
fn one_step_clears_collided_and_only_a_monster_with_flag_0x10_repaths() {
    // Collided mask := 0 (not a missile: arrival rule 1 does not clear it).
    let (t, mut c) = setup(40, 40, 10, 10);
    let mut p = moving(&c, 0x400); // 0x4000 per tick: stays in the cell
    p.collided_mask = 0x55;
    assert!(movement(&t, &mut c, &mut p));
    assert_eq!(p.collided_mask, 0);

    // A wall at (12, 10) blocks the first cell crossed (pattern 1).
    // 0x800 → 0x8000 per tick: from the centre into cell 11.
    let blocked = |ty: UnitType, keep: bool| {
        let (t, mut c) = setup(40, 40, 10, 10);
        c.w.wall(12, 10);
        c.u.unit(P).ty = ty;
        let mut p = moving(&c, 0x800);
        p.repath_budget = 5;
        if ty == UnitType::Monster {
            p.set_path_type(&t, false, 2).unwrap();
        }
        if keep {
            p.flags |= flags::KEEP_TARGET;
        }
        c.w.paths.insert(P, p.clone());
        let r = movement(&t, &mut c, &mut p);
        (r, p, c.w.log)
    };
    // Monster with flag 0x10: re-path; its new points drive the movement.
    let (r, p, _) = blocked(UnitType::Monster, true);
    assert!(r);
    assert!(p.point_count > 0 && p.cur_point < p.point_count);
    assert_eq!(p.precise_x, centre(10));
    // Monster without the flag, player with it: the movement ends at the
    // last free cell's centre, no re-path.
    for (ty, keep) in [(UnitType::Monster, false), (UnitType::Player, true)] {
        let (r, p, log) = blocked(ty, keep);
        assert!(!r);
        assert_eq!((p.point_count, p.cur_point), (0, 0));
        assert_eq!(p.precise_x, centre(10));
        assert!(!log.iter().any(|l| l.starts_with("queue")));
    }
}

#[test]
fn arrival_passes_for_circling_types_past_the_last_point() {
    // No target unit, index ≥ count, position ≠ final target: types 5, 6
    // pass (no re-path); type 2 re-paths (rule 2: the queue for update).
    for (ty, repaths) in [(5u32, false), (6, false), (2, true)] {
        let (t, mut c) = setup(40, 40, 10, 10);
        c.u.unit(P).ty = UnitType::Monster;
        let mut p = moving(&c, 0x800);
        p.repath_budget = 5;
        p.set_path_type(&t, false, ty).unwrap();
        p.cur_point = 1;
        c.w.paths.insert(P, p.clone());
        movement(&t, &mut c, &mut p);
        assert_eq!(
            c.w.log.contains(&"queue 1".to_string()),
            repaths,
            "type {ty}"
        );
    }
}

#[test]
fn room_recache_can_leave_a_non_missile_without_a_room() {
    let (t, mut c) = setup(40, 40, 18, 10);
    c.w.room0 = TileRect::new(0, 0, 20, 40); // no room holds x ≥ 20
    let mut p = c.w.paths[&P].clone();
    p.flags |= flags::OUTSIDE_ROOM;
    let q = (centre(25), centre(10));
    Walk { t: &t, c: &mut c }.set_position(P, &mut p, q, None);
    assert_eq!((p.precise_x, p.precise_y), q);
    assert_eq!(p.room, None);
    assert_eq!(p.prev_room, Some(ROOM));
    assert_ne!(p.flags & flags::ROOM_CHANGED, 0);
    assert_eq!(c.w.log, vec!["leave 1 0".to_string()]);
    // A missile in the same place keeps its position and room, count 0.
    c.u.unit(P).ty = UnitType::Missile;
    let mut m = c.w.paths[&P].clone();
    m.flags |= flags::OUTSIDE_ROOM;
    m.point_count = 3;
    let before = (m.precise_x, m.room);
    Walk { t: &t, c: &mut c }.set_position(P, &mut m, q, None);
    assert_eq!(
        (m.precise_x, m.room, m.point_count),
        (before.0, before.1, 0)
    );
}

/// A monster [`Finder`] over a free 40 × 40 [`Recorder`] grid with
/// `walls`, and a path info from `start` to `target` (one-cell pattern 0,
/// move mask 0x1C09).
pub(super) fn other_setup(walls: &[(i32, i32)]) -> Recorder {
    let mut w = FakeWorld::new(40, 40);
    for &(x, y) in walls {
        w.wall(x, y);
    }
    Recorder::new(w)
}

pub(super) fn other_info(ty: u32, start: (i32, i32), target: (i32, i32), max: i32) -> PathInfo {
    PathInfo {
        start: Point::new(start.0, start.1),
        target: Point::new(target.0, target.1),
        start_room: Some(ROOM),
        target_room: Some(ROOM),
        slack: 0,
        max_distance: max,
        idastar_score: 70,
        path_type: ty,
        size: 1,
        pattern: 0,
        move_mask: 0x1C09,
    }
}

pub(super) fn owned_path(max: u8) -> DynamicPath {
    DynamicPath {
        owner: Some(P),
        max_distance: max,
        room: Some(ROOM),
        ..DynamicPath::default()
    }
}

#[test]
fn ida_star_test_vector() {
    // Test vector: (10, 10) → (13, 11), slack 0, score 70.
    let t = tables();
    let mut c = other_setup(&[]);
    let mut f = Finder {
        t: &t,
        c: &mut c,
        owner_ty: UnitType::Monster,
    };
    let mut path = owned_path(14);
    let info = other_info(0, (10, 10), (13, 11), 14);
    let n = crate::path::walk::other::ida_star(&mut f, &mut path, &info).unwrap();
    assert_eq!(n, 2);
    assert_eq!(
        (path.point(0), path.point(1)),
        (Point::new(12, 10), Point::new(13, 11))
    );
    // A straight run records only the found node: 0.
    let info = other_info(0, (10, 10), (13, 10), 14);
    assert_eq!(
        crate::path::walk::other::ida_star(&mut f, &mut path, &info).unwrap(),
        0
    );
    // Type 16 draws on the owner's seed; type 0 does not.
    let before = f.c.seed;
    let info = other_info(0, (10, 10), (13, 11), 14);
    crate::path::walk::other::ida_star(&mut f, &mut path, &info).unwrap();
    assert_eq!(f.c.seed, before);
    let info = other_info(16, (10, 10), (13, 11), 14);
    crate::path::walk::other::ida_star(&mut f, &mut path, &info).unwrap();
    assert_ne!(f.c.seed, before);
}

#[test]
fn wall_follow_test_vector() {
    // Test vector: one-cell pattern, L 14, only (12, 10) blocked,
    // (10, 10) → (15, 10): (11, 10), (12, 11), (13, 10), (15, 10).
    let t = tables();
    let mut c = other_setup(&[(12, 10)]);
    let mut f = Finder {
        t: &t,
        c: &mut c,
        owner_ty: UnitType::Monster,
    };
    let mut path = owned_path(14);
    let info = other_info(15, (10, 10), (15, 10), 14);
    let n = crate::path::walk::other::wall_follow(&mut f, &mut path, &info).unwrap();
    assert_eq!(n, 4);
    assert_eq!(
        path.points[..4]
            .iter()
            .map(|p| p.point())
            .collect::<Vec<_>>(),
        [
            Point::new(11, 10),
            Point::new(12, 11),
            Point::new(13, 10),
            Point::new(15, 10)
        ]
    );
    // N ≤ 2 → 0; a line longer than L − 1 → no cells → 0.
    let info = other_info(15, (10, 10), (12, 10), 14);
    assert_eq!(
        crate::path::walk::other::wall_follow(&mut f, &mut path, &info).unwrap(),
        0
    );
    let info = other_info(15, (10, 10), (25, 10), 14);
    assert_eq!(
        crate::path::walk::other::wall_follow(&mut f, &mut path, &info).unwrap(),
        0
    );
    // A free line compresses to its last cell.
    let mut c = other_setup(&[]);
    let mut f = Finder {
        t: &t,
        c: &mut c,
        owner_ty: UnitType::Monster,
    };
    let info = other_info(15, (10, 10), (15, 10), 14);
    assert_eq!(
        crate::path::walk::other::wall_follow(&mut f, &mut path, &info).unwrap(),
        1
    );
    assert_eq!(path.point(0), Point::new(15, 10));
    // Only players and monsters own a wall-follow path.
    let mut f = Finder {
        t: &t,
        c: &mut c,
        owner_ty: UnitType::Object,
    };
    assert!(crate::path::walk::other::wall_follow(&mut f, &mut path, &info).is_err());
}

#[test]
fn wall_follow_no_rejoin_reads_an_empty_followers_done_flag() {
    // S (10, 10) → (15, 6), L 14; line (11, 10), (12, 9),
    // (13, 8), (14, 7), (15, 6), D 1. (11, 10) blocked at i = 0, P = S,
    // d0 = 2. A steps to (11, 11); B's four tries (11, 9), (10, 9),
    // (9, 10), (10, 11) are blocked: B done with no points. Its "last
    // point" is its done flag, (1, 0): dB = 232 > dA = 41 and dS = 41 ≥
    // dA → A's point is taken: one point (11, 11). (Read as B's position
    // P, dB = 41 = dA would pick the empty B: 0 points.)
    let t = tables();
    let mut c = other_setup(&[(11, 10), (11, 9), (10, 9), (9, 10), (10, 11)]);
    let mut f = Finder {
        t: &t,
        c: &mut c,
        owner_ty: UnitType::Monster,
    };
    let mut path = owned_path(14);
    let info = other_info(15, (10, 10), (15, 6), 14);
    let n = crate::path::walk::other::wall_follow(&mut f, &mut path, &info).unwrap();
    assert_eq!(n, 1);
    assert_eq!(path.point(0), Point::new(11, 11));
}

#[test]
fn leap_and_server_knockback() {
    let t = tables();
    let mut c = other_setup(&[(14, 10)]);
    let mut f = Finder {
        t: &t,
        c: &mut c,
        owner_ty: UnitType::Monster,
    };
    // Knockback without a target unit: the ray's last free cell, which
    // becomes the point target; the index is not reset.
    let mut path = owned_path(14);
    path.cur_point = 2;
    let info = other_info(8, (10, 10), (16, 10), 14);
    assert_eq!(
        crate::path::walk::other::knockback_server(&mut f, &mut path, &info),
        1
    );
    assert_eq!(path.point(0), Point::new(13, 10));
    assert_eq!((path.target(), path.cur_point), (Point::new(13, 10), 2));
    // A target unit: k = (budget >> 1) + 1 cells away from it.
    let m = UnitId(7);
    f.c.units.insert(m, (UnitType::Monster, Point::new(12, 12)));
    path.target_unit = Some(TargetUnit {
        unit: m,
        ty: UnitType::Monster,
        guid: 7,
    });
    path.dist_budget = 6;
    let info = other_info(8, (10, 10), (0, 0), 14);
    assert_eq!(
        crate::path::walk::other::knockback_server(&mut f, &mut path, &info),
        1
    );
    assert_eq!(path.point(0), Point::new(6, 6));
    assert!(path.target_unit.is_none());
    // Blocked at the first cell: the start, result 0.
    let mut c = other_setup(&[(11, 10)]);
    let mut f = Finder {
        t: &t,
        c: &mut c,
        owner_ty: UnitType::Monster,
    };
    let info = other_info(8, (10, 10), (16, 10), 14);
    assert_eq!(
        crate::path::walk::other::knockback_server(&mut f, &mut path, &info),
        0
    );
    // Leap with no velocity: the start (P = start → 0).
    let info = other_info(9, (10, 10), (16, 10), 14);
    assert_eq!(crate::path::walk::other::leap(&mut f, &mut path, &info), 0);
}

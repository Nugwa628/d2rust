//! Property tests of walk / run (`d2_sim::path::walk`) on fakes of the
//! seam traits: one room of
//! random walls whose collision view follows `path-placement.md` §4–§6
//! for patterns 0–5.
//!
//! - Path compute (§3) with types 1, 2, 7 (§5–§7): every point is free,
//!   every segment the unit walks is an 8-direction run of free cells
//!   (or the first ray of §5.1 rule 4, checked against a reference ray),
//!   the point count stays within 77 and the greedy walk within the max
//!   distance, the grid is unchanged afterwards (§3 steps 6, 9), and two
//!   runs give identical records.
//! - A* (§7) reaches the target iff a breadth-first search on the same
//!   grid does, whenever the reachable component fits the 200 nodes.
//! - Per-tick movement (§9) after a request: no accepted footprint move
//!   enters a blocked cell, each crosses one cell, the unit ends at a
//!   cell centre on the path's last point when nothing refused it, and
//!   velocity / direction values stay in their integer ranges.
//! - Requests (§1) on arbitrary 0x01–0x04 payloads and player states
//!   return 0 and a checked movement outcome, never panic.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use d2_sim::drlg::{CollisionGrid, TileRect};
use d2_sim::path::collision::{find_room, CollisionRooms};
use d2_sim::path::coords::to_fp16_center;
use d2_sim::path::record::{flags, DynamicPath, PathPoint};
use d2_sim::path::walk::find::{astar, compute, Finder};
use d2_sim::path::walk::request::{handle_message, Outcome};
use d2_sim::path::walk::seams::{PathInfo, PathWorld, Point, UsedSkill, WalkUnits};
use d2_sim::path::walk::{Step, Walk};
use d2_sim::path::PathTables;
use d2_sim::rng::Seed;
use d2_sim::units::{ClientId, RoomId, UnitId, UnitType};
use proptest::prelude::*;

fn config(default: u32) -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default);
    ProptestConfig {
        cases,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

const P: UnitId = UnitId(1);
const T: UnitId = UnitId(2);
const ROOM: RoomId = RoomId(0);
/// The room's lower-left sub-tile (positions (0, any) mean "none").
const OX: i32 = 40;
const OY: i32 = 60;
const WALL: u16 = 0x1;

// ---- the world fake ---------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
/// The walk context: one room's grid, the path store and the units
/// (`PathWorld` + `WalkUnits` on one object, as the walk code takes it).
struct World {
    w: i32,
    h: i32,
    g: CollisionGrid,
    town: bool,
    paths: BTreeMap<UnitId, DynamicPath>,
    units: Units,
}

fn pattern_cells(pattern: u32) -> Option<&'static [(i32, i32)]> {
    const POINT: [(i32, i32); 1] = [(0, 0)];
    const PLUS: [(i32, i32); 5] = [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)];
    const BOX: [(i32, i32); 9] = [
        (-1, -1),
        (0, -1),
        (1, -1),
        (-1, 0),
        (0, 0),
        (1, 0),
        (-1, 1),
        (0, 1),
        (1, 1),
    ];
    match pattern {
        0 => Some(&POINT),
        1 | 3 | 5 => Some(&PLUS),
        2 | 4 => Some(&BOX),
        _ => None,
    }
}

impl World {
    fn new(w: i32, h: i32, walls: &[bool]) -> World {
        let mut g = CollisionGrid::new(TileRect::new(OX, OY, w, h));
        for (i, m) in g.masks.iter_mut().enumerate() {
            *m = if walls[i % walls.len()] { WALL } else { 0 };
        }
        World {
            w,
            h,
            g,
            town: false,
            paths: BTreeMap::new(),
            units: player_units(),
        }
    }

    fn inside(&self, p: Point) -> bool {
        p.x >= OX && p.y >= OY && p.x < OX + self.w && p.y < OY + self.h
    }

    fn idx(&self, p: Point) -> usize {
        ((p.y - OY) * self.w + (p.x - OX)) as usize
    }

    /// `path-placement.md` §4 rule 5 pattern query in one room: OR of
    /// masked values, 0x27 unmasked for a cell outside; other patterns
    /// 0xFFFF.
    fn query(&self, room: Option<RoomId>, at: Point, pattern: u32, mask: u16) -> u16 {
        if room != Some(ROOM) || !self.inside(at) {
            return 0x27;
        }
        let Some(cs) = pattern_cells(pattern) else {
            return 0xFFFF;
        };
        cs.iter().fold(0, |a, &(dx, dy)| {
            let c = Point::new(at.x + dx, at.y + dy);
            a | if self.inside(c) {
                self.g.masks[self.idx(c)] & mask
            } else {
                0x27
            }
        })
    }

    /// §5.1 pattern stamp / clear with the §3 markers.
    fn stamp(&mut self, at: Point, pattern: u32, mask: u16, set: bool) {
        let Some(cs) = pattern_cells(pattern) else {
            return;
        };
        let marker: Option<(u16, &[(i32, i32)])> = match pattern {
            1 => Some((0x1000, &[(0, 0)])),
            2 => Some((0x1000, pattern_cells(1).unwrap())),
            3 => Some((0x2000, &[(0, 0)])),
            4 => Some((0x2000, pattern_cells(1).unwrap())),
            _ => None,
        };
        let mut apply = |cells: &[(i32, i32)], m: u16| {
            for &(dx, dy) in cells {
                let c = Point::new(at.x + dx, at.y + dy);
                if self.inside(c) {
                    let i = self.idx(c);
                    if set {
                        self.g.masks[i] |= m;
                    } else {
                        self.g.masks[i] &= !m;
                    }
                }
            }
        };
        apply(cs, mask);
        if mask != 0 {
            if let Some((m, c)) = marker {
                apply(c, m);
            }
        }
    }

    /// The walls-only view: free iff the pattern at `p` meets no wall and
    /// lies in the room (the unit's own footprint never blocks it: its
    /// bits are outside the move masks once removed, §3 step 6).
    fn free(&self, p: Point, pattern: u32) -> bool {
        self.query(Some(ROOM), p, pattern, WALL) == 0
    }

    fn add_unit(&mut self, unit: UnitId, x: i32, y: i32, player: bool, t: &PathTables) {
        // `path-placement.md` §2.4.
        let mut p = DynamicPath {
            owner: Some(unit),
            unit_size: 2,
            pattern: 1,
            precise_x: to_fp16_center(x),
            precise_y: to_fp16_center(y),
            velocity: 0x800,
            room: Some(ROOM),
            saved_count: 1,
            ..DynamicPath::default()
        };
        p.saved_steps[0] = PathPoint::from_point(Point::new(x, y));
        if player {
            p.foot_mask = 0x80;
            p.move_mask = 0x1C09;
            p.set_path_type(t, true, 7).unwrap();
            p.max_distance = 73;
            p.ida_score = 70;
        } else {
            p.foot_mask = 0x100;
            p.move_mask = 0x3C01;
            p.set_path_type(t, false, 2).unwrap();
            p.max_distance = 14;
        }
        self.stamp(Point::new(x, y), p.pattern, p.foot_mask, true);
        self.paths.insert(unit, p);
    }
}

impl CollisionRooms for World {
    fn subtile_rect(&self, room: RoomId) -> Option<TileRect> {
        (room == ROOM).then_some(self.g.rect)
    }
    fn adjacent_count(&self, _room: RoomId) -> usize {
        0
    }
    fn adjacent(&self, _room: RoomId, _i: usize) -> Option<RoomId> {
        None
    }
    fn grid(&self, room: RoomId) -> Option<&CollisionGrid> {
        (room == ROOM).then_some(&self.g)
    }
    fn grid_mut(&mut self, room: RoomId) -> Option<&mut CollisionGrid> {
        (room == ROOM).then_some(&mut self.g)
    }
}

impl PathWorld for World {
    fn load_path(&self, unit: UnitId) -> Option<DynamicPath> {
        self.paths.get(&unit).cloned()
    }
    fn store_path(&mut self, unit: UnitId, path: &DynamicPath) {
        self.paths.insert(unit, path.clone());
    }
    fn room_in_town(&self, _room: RoomId) -> bool {
        self.town
    }
    fn remove_footprint(&mut self, unit: UnitId, _force: bool) -> bool {
        let Some(p) = self.paths.get(&unit).cloned() else {
            return false;
        };
        self.stamp(p.cell(), p.pattern, p.foot_mask, false);
        true
    }
    fn add_footprint(&mut self, unit: UnitId) {
        if let Some(p) = self.paths.get(&unit).cloned() {
            self.stamp(p.cell(), p.pattern, p.foot_mask, true);
        }
    }
    fn room_list_remove(&mut self, _unit: UnitId, _room: RoomId) {}
    fn room_list_insert(&mut self, _unit: UnitId, _room: RoomId) {}
    fn queue_for_update(&mut self, _unit: UnitId) {}
    fn room_clients(&self, _room: RoomId) -> Vec<ClientId> {
        Vec::new()
    }
}

// ---- the unit fake ----------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
struct Unit {
    ty: UnitType,
    class: u32,
    guid: u32,
    mode: u32,
    pos: Point,
    size: i32,
    stats: BTreeMap<u16, i32>,
    item_stats: BTreeMap<u16, i32>,
    states: BTreeSet<u16>,
    used_skill: Option<UsedSkill>,
    seed: Seed,
    cursor: bool,
    run_bonus: i32,
}

impl Unit {
    fn new(ty: UnitType, guid: u32) -> Unit {
        Unit {
            ty,
            class: 0,
            guid,
            mode: 1,
            pos: Point::default(),
            size: 2,
            stats: BTreeMap::from([(10, 100 << 8), (67, 100)]),
            item_stats: BTreeMap::new(),
            states: BTreeSet::new(),
            used_skill: None,
            seed: Seed::default(),
            cursor: false,
            run_bonus: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Units {
    units: BTreeMap<UnitId, Unit>,
    type1_expire: i32,
    frame: i32,
    events0: u32,
}

impl Units {
    fn get(&self, u: UnitId) -> &Unit {
        &self.units[&u]
    }
    fn get_mut(&mut self, u: UnitId) -> &mut Unit {
        self.units.get_mut(&u).unwrap()
    }
}

impl WalkUnits for World {
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        d2_sim::tick::TickRate::BASE
    }
    fn unit_type(&self, unit: UnitId) -> UnitType {
        self.units.get(unit).ty
    }
    fn class(&self, unit: UnitId) -> u32 {
        self.units.get(unit).class
    }
    fn guid(&self, unit: UnitId) -> u32 {
        self.units.get(unit).guid
    }
    fn mode(&self, unit: UnitId) -> u32 {
        self.units.get(unit).mode
    }
    fn find_unit(&self, ty: UnitType, guid: u32) -> Option<UnitId> {
        self.units
            .units
            .iter()
            .find(|(_, u)| u.ty == ty && u.guid == guid)
            .map(|(&id, _)| id)
    }
    fn position(&self, unit: UnitId) -> Point {
        self.units.get(unit).pos
    }
    fn unit_size(&self, unit: UnitId) -> i32 {
        self.units.get(unit).size
    }
    fn frame(&self) -> i32 {
        self.units.frame
    }
    fn first_type1_expire(&self, _unit: UnitId) -> i32 {
        self.units.type1_expire
    }
    fn has_cursor_item(&self, unit: UnitId) -> bool {
        self.units.get(unit).cursor
    }
    fn has_state(&self, unit: UnitId, state: u16) -> bool {
        self.units.get(unit).states.contains(&state)
    }
    fn state_stat(&self, _unit: UnitId, _state: u16, _stat: u16) -> i32 {
        50
    }
    fn stat(&self, unit: UnitId, stat: u16) -> i32 {
        let u = self.units.get(unit);
        let base = u.stats.get(&stat).copied().unwrap_or(0);
        if stat == 67 {
            base + u.run_bonus
        } else {
            base
        }
    }
    fn item_stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.units
            .get(unit)
            .item_stats
            .get(&stat)
            .copied()
            .unwrap_or(0)
    }
    fn add_base_stat(&mut self, unit: UnitId, stat: u16, delta: i32) {
        *self.units.get_mut(unit).stats.entry(stat).or_insert(0) += delta;
    }
    fn set_base_stat(&mut self, unit: UnitId, stat: u16, value: i32) {
        self.units.get_mut(unit).stats.insert(stat, value);
    }
    fn seed(&mut self, unit: UnitId) -> &mut Seed {
        &mut self.units.get_mut(unit).seed
    }
    fn used_skill(&self, unit: UnitId) -> Option<UsedSkill> {
        self.units.get(unit).used_skill
    }
    fn set_used_skill(&mut self, unit: UnitId, skill: Option<u16>) {
        let u = self.units.get_mut(unit);
        u.used_skill = skill.map(|id| UsedSkill {
            id,
            ..UsedSkill::default()
        });
    }
    fn set_mode(&mut self, unit: UnitId, mode: u32) {
        let u = self.units.get_mut(unit);
        u.mode = mode;
        // The run stat list is freed at the next mode set (§8.2).
        u.run_bonus = 0;
    }
    fn schedule_event0(&mut self, _unit: UnitId) {
        self.units.events0 += 1;
    }
    fn attach_run_stats(&mut self, unit: UnitId, value: i32) {
        self.units.get_mut(unit).run_bonus = value;
    }
    fn charstats_velocity(&self, _unit: UnitId) -> (i32, i32, i32) {
        // Fixed WalkVelocity, RunVelocity and RunDrain inputs for the fake.
        (6, 9, 20)
    }
}

// ---- reference geometry ----------------------------------------------

/// §5.1 rule 4 ray test, as written: `Ok` clear, `Err(e)` the last cell
/// before the blocking one.
fn ref_ray(free: &dyn Fn(Point) -> bool, s: Point, e: Point) -> Result<(), Point> {
    let (dx, dy) = (e.x - s.x, e.y - s.y);
    let (nx, ny) = (dx.abs() + 1, dy.abs() + 1);
    let sx = if dx < 0 { -1 } else { 1 };
    let sy = if dy < 0 { -1 } else { 1 };
    let (mut x, mut y) = (s.x, s.y);
    if nx > ny {
        if e.x == s.x {
            return Ok(());
        }
        let mut err = ny;
        loop {
            let rem = Point::new(x, y);
            x += sx;
            if !free(Point::new(x, y)) {
                return Err(rem);
            }
            err += ny;
            if err >= nx {
                y += sy;
                err -= nx;
                if err > 0 && !free(Point::new(x, y)) {
                    return Err(rem);
                }
            }
            if x == e.x {
                return Ok(());
            }
        }
    } else if nx == ny {
        loop {
            let rem = Point::new(x, y);
            if x == e.x {
                return Ok(());
            }
            x += sx;
            y += sy;
            if !free(Point::new(x, y)) {
                return Err(rem);
            }
        }
    } else {
        if e.y == s.y {
            return Ok(());
        }
        let mut err = nx;
        loop {
            let rem = Point::new(x, y);
            y += sy;
            if !free(Point::new(x, y)) {
                return Err(rem);
            }
            err += nx;
            if err >= ny {
                x += sx;
                err -= ny;
                if err > 0 && !free(Point::new(x, y)) {
                    return Err(rem);
                }
            }
            if y == e.y {
                return Ok(());
            }
        }
    }
}

/// An 8-direction run from `a` to `b`: its cells after `a`, or `None`.
fn run_cells(a: Point, b: Point) -> Option<Vec<Point>> {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    if dx != 0 && dy != 0 && dx.abs() != dy.abs() {
        return None;
    }
    let n = dx.abs().max(dy.abs());
    Some(
        (1..=n)
            .map(|k| Point::new(a.x + dx.signum() * k, a.y + dy.signum() * k))
            .collect(),
    )
}

const NEIGHBOURS: [(i32, i32); 8] = [
    (-1, -1),
    (-1, 1),
    (1, -1),
    (1, 1),
    (-1, 0),
    (0, -1),
    (1, 0),
    (0, 1),
];

/// Breadth-first search over free cells (8 neighbours; the start is not
/// tested, as §7 rule 5 tests neighbours only): (reaches target, size of
/// the component including the start).
fn bfs(free: &dyn Fn(Point) -> bool, start: Point, target: Point, bound: i32) -> (bool, usize) {
    let mut seen = BTreeSet::from([start]);
    let mut q = VecDeque::from([start]);
    while let Some(c) = q.pop_front() {
        for (dx, dy) in NEIGHBOURS {
            let n = Point::new(c.x + dx, c.y + dy);
            if n.x.abs() > bound || n.y.abs() > bound {
                continue;
            }
            if !seen.contains(&n) && free(n) {
                seen.insert(n);
                q.push_back(n);
            }
        }
    }
    (seen.contains(&target), seen.len())
}

// ---- strategies -------------------------------------------------------

fn walls(density: u32) -> impl Strategy<Value = Vec<bool>> {
    proptest::collection::vec((0u32..100).prop_map(move |r| r < density), 1..600)
}

/// A room of `w`×`h` and the start cell, with the start's plus cleared so
/// the unit stands free.
fn arena(max: i32) -> impl Strategy<Value = (i32, i32, Vec<bool>, Point, Point)> {
    (
        4i32..max,
        4i32..max,
        prop_oneof![2 => 0u32..12, 1 => 0u32..55],
    )
        .prop_flat_map(|(w, h, d)| {
            (
                Just(w),
                Just(h),
                walls(d),
                1..w - 1,
                1..h - 1,
                -2..w + 2,
                -2..h + 2,
            )
                .prop_map(|(w, h, walls, x, y, tx, ty)| {
                    (
                        w,
                        h,
                        walls,
                        Point::new(OX + x, OY + y),
                        Point::new(OX + tx, OY + ty),
                    )
                })
        })
}

fn clear_plus(world: &mut World, p: Point) {
    for (dx, dy) in [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)] {
        let c = Point::new(p.x + dx, p.y + dy);
        if world.inside(c) {
            let i = world.idx(c);
            world.g.masks[i] = 0;
        }
    }
}

fn player_units() -> Units {
    let mut p = Unit::new(UnitType::Player, 1);
    p.mode = 1;
    Units {
        units: BTreeMap::from([(P, p)]),
        type1_expire: 0,
        frame: 0,
        events0: 0,
    }
}

// ---- path compute -----------------------------------------------------

/// Checks the points of a computed path (shared by the compute
/// property): free points, aligned free runs, the first ray, counts.
fn check_path(
    world: &World,
    path: &DynamicPath,
    start: Point,
    ty: u32,
    n: i32,
) -> Result<(), TestCaseError> {
    let pts = path.live_points();
    prop_assert_eq!(n, path.point_count as i32);
    prop_assert!((0..=77).contains(&n), "count {}", n);
    prop_assert_eq!(path.cur_point, 0);
    let free = |p: Point| world.free(p, path.pattern);
    if n == 0 {
        return Ok(());
    }
    let target = path.final_target();
    // §5.1 rule 4 on the (prepared) target: a clear ray is the whole path
    // for toward and straight (§5.2 step 1, §6 step 2).
    let ray = ref_ray(&free, start, target);
    if ty != 1 && ray.is_ok() {
        prop_assert_eq!(&pts[..], &[target][..]);
        return Ok(());
    }
    // Edge case 4: after a minor step with err = 0 the ray does not test
    // the new cell, so the returned P (§5.2 steps 2–3, and its duplicate,
    // edge case 3) may collide. Every other point was tested free.
    let untested = if ty != 1 { ray.err() } else { None };
    for (k, p) in pts.iter().enumerate() {
        if k <= 1 && Some(*p) == untested {
            continue;
        }
        prop_assert!(
            free(*p),
            "point {:?} collides; path {:?} start {:?} target {:?} grid {}",
            p,
            pts,
            start,
            target,
            dump(world)
        );
    }
    // §5.2 steps 3–4: the greedy walk starts at P (or at the start when
    // the ray stopped there); each of its steps is a cell of a run.
    let greedy_from_start = ty == 2 && untested == Some(start);
    let mut from = start;
    let mut greedy = 0;
    for (k, &p) in pts.iter().enumerate() {
        match run_cells(from, p) {
            Some(cells) => {
                for c in &cells {
                    if k == 0 && Some(*c) == untested {
                        continue;
                    }
                    prop_assert!(free(*c), "cell {:?} of {:?} → {:?} collides", c, from, p);
                }
                if k > 0 || greedy_from_start {
                    greedy += cells.len() as i32;
                }
            }
            None => {
                // Only the toward ray (§5.2 steps 1–3) leaves a run.
                prop_assert!(
                    ty != 1 && k == 0,
                    "segment {:?} → {:?} is not a run",
                    from,
                    p
                );
                prop_assert_eq!(ray, Err(p));
            }
        }
        from = p;
    }
    if ty == 2 {
        // §5.2 step 4: at most `max distance` greedy steps.
        prop_assert!(
            greedy <= path.max_distance as i32,
            "greedy {} steps: {:?}",
            greedy,
            pts
        );
    }
    Ok(())
}

proptest! {
    #![proptest_config(config(384))]

    /// §3 with types 1, 2 (a monster), 7 (a player).
    #[test]
    fn compute_returns_free_bounded_deterministic_paths(
        (w, h, walls, start, target) in arena(30),
        ty in prop_oneof![Just(1u32), Just(2u32), Just(7u32)],
        velocity in prop_oneof![Just(0i32), Just(0x600), Just(0x800), Just(0x900)],
    ) {
        let t = PathTables::spec().unwrap();
        let mut world = World::new(w, h, &walls);
        clear_plus(&mut world, start);
        let player = ty != 2;
        world.add_unit(P, start.x, start.y, player, &t);
        if !player {
            world.units.get_mut(P).ty = UnitType::Monster;
        }
        let mut path = world.paths[&P].clone();
        path.set_path_type(&t, player, ty).unwrap();
        path.velocity = velocity;
        path.put_target(target);
        let before = world.g.clone();

        let (mut w2, mut p2) = (world.clone(), path.clone());
        let n = compute(&t, &mut world, &mut path, P, false).unwrap();
        prop_assert_eq!(&world.g, &before, "§3 steps 6 and 9 restore the footprint");
        if velocity != 0 || ty == 1 {
            check_path(&world, &path, start, ty, n)?;
        }
        // Determinism: a second run on copies gives the same record.
        let n2 = compute(&t, &mut w2, &mut p2, P, false).unwrap();
        prop_assert_eq!(n, n2);
        prop_assert_eq!(&path, &p2);
        prop_assert_eq!(&world, &w2);
    }

    /// §7 against breadth-first search on the same grid.
    #[test]
    fn astar_reaches_iff_bfs_does(
        (w, h, walls, start, target) in arena(22),
        pattern in prop_oneof![Just(0u32), Just(1u32), Just(2u32)],
    ) {
        let t = PathTables::spec().unwrap();
        let mut world = World::new(w, h, &walls);
        clear_plus(&mut world, start);
        let mut path = DynamicPath { owner: Some(P), ..DynamicPath::default() };
        let info = PathInfo {
            start,
            target,
            start_room: Some(ROOM),
            target_room: find_room(&world, Some(ROOM), target.x, target.y),
            slack: 1,
            max_distance: 73,
            idastar_score: 70,
            path_type: 1,
            size: 2,
            pattern,
            move_mask: 0x1C09,
        };
        let reference = world.clone();
        let f = Finder { t: &t, c: &mut world, owner_ty: UnitType::Player };
        let n = astar(&f, &mut path, &info).unwrap();
        let world = reference;
        prop_assert!((0..=77).contains(&n));
        let pts: Vec<Point> = path.points[..n as usize].iter().map(|p| p.point()).collect();
        let free = |p: Point| world.free(p, pattern);
        // §7 rule 6: corners of 8-direction runs of free cells from the start.
        let mut from = start;
        for &p in &pts {
            let cells = run_cells(from, p);
            prop_assert!(cells.is_some(), "{:?} → {:?}", from, p);
            for c in cells.unwrap() {
                prop_assert!(free(c));
            }
            from = p;
        }
        let reached = n > 0 && pts[n as usize - 1] == target;
        if start == target {
            // h(start) = 0: the start is best, nothing is output.
            prop_assert_eq!(n, 0);
            return Ok(());
        }
        let (bfs_reaches, component) = bfs(&free, start, target, 10_000);
        if reached {
            prop_assert!(bfs_reaches);
        }
        if component <= 200 {
            // Storage never fills: the search ends when the target is
            // popped or the open list is empty.
            prop_assert_eq!(reached, bfs_reaches, "component {}", component);
        }
    }
}

/// A serpentine corridor of free cells (pattern 0): wall rows at odd y
/// with one gap, alternating ends, so A* must store most of the
/// corridor before it reaches the far end.
fn serpentine(w: i32, h: i32) -> World {
    let mut world = World::new(w, h, &[false]);
    for y in (1..h).step_by(2) {
        let gap = if (y / 2) % 2 == 0 { w - 1 } else { 0 };
        for x in 0..w {
            if x != gap {
                let i = (y * w + x) as usize;
                world.g.masks[i] = WALL;
            }
        }
    }
    world
}

proptest! {
    #![proptest_config(config(128))]

    /// §7 rule 2 storage bound: on a corridor of at most 200 cells the
    /// far end is always reached (whatever cell of it is the start).
    #[test]
    fn astar_serpentine_reaches_far_end(
        (w, h) in (3i32..30).prop_flat_map(|w| (Just(w), 3i32..(400 / w).clamp(4, 30))),
        flip in any::<bool>(),
    ) {
        let mut world = serpentine(w, h);
        let cells: Vec<Point> = (0..h)
            .flat_map(|y| (0..w).map(move |x| Point::new(OX + x, OY + y)))
            .filter(|&p| world.free(p, 0))
            .collect();
        prop_assume!(cells.len() <= 200 && cells.len() >= 2);
        // The far end: the free cell with the largest BFS distance from
        // the lower-left corner.
        let mut start = Point::new(OX, OY);
        let mut dist = BTreeMap::from([(start, 0)]);
        let mut q = VecDeque::from([start]);
        while let Some(c) = q.pop_front() {
            for (dx, dy) in NEIGHBOURS {
                let n = Point::new(c.x + dx, c.y + dy);
                if world.free(n, 0) && !dist.contains_key(&n) {
                    dist.insert(n, dist[&c] + 1);
                    q.push_back(n);
                }
            }
        }
        let mut target = *dist.iter().max_by_key(|(p, d)| (**d, p.y, p.x)).unwrap().0;
        if flip {
            std::mem::swap(&mut start, &mut target);
        }
        prop_assume!(world.free(start, 0) && world.free(target, 0));
        let t = PathTables::spec().unwrap();
        let mut path = DynamicPath { owner: Some(P), ..DynamicPath::default() };
        let info = PathInfo {
            start,
            target,
            start_room: Some(ROOM),
            target_room: Some(ROOM),
            slack: 1,
            max_distance: 73,
            idastar_score: 70,
            path_type: 1,
            size: 2,
            pattern: 0,
            move_mask: 0x1C09,
        };
        let f = Finder { t: &t, c: &mut world, owner_ty: UnitType::Player };
        let n = astar(&f, &mut path, &info).unwrap();
        let (reach, comp) = bfs(&|p| world.free(p, 0), start, target, 10_000);
        prop_assert!(reach && comp <= 200);
        // Corners: two per row change, within the 77 outputs while h ≤ 30.
        prop_assert!(n > 0, "no path on a {}×{} corridor of {} cells", w, h, comp);
        prop_assert_eq!(path.point(n as usize - 1), target);
        // Direct A* returns the count; only compute() publishes point_count.
        // Inspect its returned points without assuming compute() bookkeeping.
        prop_assert!((1..=77).contains(&n), "count={n}");
        let mut from = start;
        for point in &path.points[..n as usize] {
            let to = point.point();
            let cells = run_cells(from, to);
            prop_assert!(cells.is_some(), "segment {from:?} -> {to:?} is not a run");
            for cell in cells.unwrap() {
                prop_assert!(world.free(cell, info.pattern), "blocked cell {cell:?} on {from:?} -> {to:?}");
            }
            from = to;
        }
    }
}

proptest! {
    #![proptest_config(config(128))]

    /// §5.2 step 4 runs exactly `max distance` iterations in the open:
    /// with velocity 0 the next-position check returns P = start (§5.1
    /// rule 5), the target is out of reach and every step toward it is
    /// free, so the greedy part is 14 steps for a monster (13 when the
    /// last iteration turned and its end is not appended, step 5).
    #[test]
    fn greedy_walk_stops_at_max_distance(
        dx in prop_oneof![-60i32..=-20, 20i32..=60],
        dy in -16i32..16,
    ) {
        let t = PathTables::spec().unwrap();
        let mut world = World::new(130, 40, &[false]);
        let start = Point::new(OX + 65, OY + 20);
        world.add_unit(P, start.x, start.y, false, &t);
        world.units.get_mut(P).ty = UnitType::Monster;
        let mut path = world.paths[&P].clone();
        path.velocity = 0;
        path.put_target(Point::new(start.x + dx, start.y + dy));
        let n = compute(&t, &mut world, &mut path, P, false).unwrap();
        prop_assert!(n > 0);
        let mut from = start;
        let mut steps = 0;
        for p in path.live_points() {
            let cells = run_cells(from, p);
            prop_assert!(cells.is_some());
            steps += cells.unwrap().len() as i32;
            from = p;
        }
        prop_assert!(steps == 14 || steps == 13, "{} greedy steps: {:?}", steps, path.live_points());
    }
}

// ---- per-tick movement ------------------------------------------------

proptest! {
    #![proptest_config(config(256))]

    /// §1 → §9: request then event 0 every tick until the unit stops.
    #[test]
    fn movement_stays_free_and_stops(
        (w, h, walls, start, target) in arena(30),
        run in any::<bool>(),
        stamina in prop_oneof![Just(0i32), Just(100 << 8)],
        stat67 in 0i32..200,
        stat96 in prop_oneof![Just(0i32), 1i32..200],
        town in any::<bool>(),
    ) {
        let t = PathTables::spec().unwrap();
        let mut world = World::new(w, h, &walls);
        world.town = town;
        clear_plus(&mut world, start);
        world.add_unit(P, start.x, start.y, true, &t);
        {
            let p = world.units.get_mut(P);
            p.stats.insert(10, stamina);
            p.stats.insert(67, stat67);
            p.item_stats.insert(96, stat96);
        }
        let id = if run { 0x03 } else { 0x01 };
        let (r, o) = handle_message(&t, &mut world, P, id, target.x as u32, target.y as u32).unwrap();
        prop_assert_eq!(r, 0);
        let Some(Outcome::Moving(n)) = o else {
            // Neutral (no path): mode 1 or 5 (§1.5 step 5).
            prop_assert_eq!(o, Some(Outcome::Neutral));
            prop_assert_eq!(world.units.get(P).mode, if town { 5 } else { 1 });
            return Ok(());
        };
        prop_assert!(n > 0);
        // §1.5 step 2 mode, §8.1 / §8.2 velocity.
        let mode = if run && stamina > 0 { 3 } else if town { 6 } else { 2 };
        prop_assert_eq!(world.units.get(P).mode, mode);
        let f = if stat96 != 0 { 150 * stat96 / (150 + stat96) } else { 0 };
        let p = (f + stat67 + if mode == 3 { 100 * 9 / 6 - 100 } else { 0 }).max(25);
        let v = 6 * 256 * p / 100;
        let path0 = world.paths[&P].clone();
        prop_assert_eq!(path0.velocity, v);
        let last = path0.point(path0.point_count as usize - 1);

        let mut stopped = false;
        let mut refused = false;
        let mut prev = start;
        for _tick in 0..2000 {
            let s = Walk { t: &t, c: &mut world }.player_event0(P).unwrap();
            let path = &world.paths[&P];
            // Integer ranges (§8.3, §8.4, §8.5).
            prop_assert!(path.dir_vec_x.abs() <= 4096 && path.dir_vec_y.abs() <= 4096, "{:?}", (path.dir_vec_x, path.dir_vec_y));
            prop_assert!(path.direction < 64 && path.new_direction < 64);
            prop_assert!(path.velocity == v || path.velocity == 0, "velocity {}", path.velocity);
            let bound = 16 * path.velocity.abs();
            prop_assert!(path.vel_vec_x.abs() <= bound && path.vel_vec_y.abs() <= bound, "{:?}", (path.vel_vec_x, path.vel_vec_y));
            prop_assert!(world.free(path.cell(), path.pattern), "unit at blocked {:?}", path.cell());
            // §9.6 rules 5–6 through the saved steps (type 7 has flag
            // 0x20000): each accepted footprint move crosses one cell into
            // a free cell, and the unit ends in the last one; flag 0x8 iff
            // a cell was crossed.
            let cell = path.cell();
            let crossed = path.flags & flags::MOVED != 0;
            prop_assert_eq!(crossed, cell != prev, "flag 0x8 vs {:?} → {:?}", prev, cell);
            if crossed {
                let steps: Vec<Point> = path.saved_steps[..path.saved_count as usize].iter().map(|p| p.point()).collect();
                prop_assert!(!steps.is_empty());
                let mut c = prev;
                for &n in &steps {
                    prop_assert!((n.x - c.x).abs() <= 1 && (n.y - c.y).abs() <= 1 && n != c, "{:?} → {:?}", c, n);
                    prop_assert!(world.free(n, 1), "entered blocked {:?}", n);
                    c = n;
                }
                prop_assert_eq!(c, cell);
            }
            // A refused move (collided mask ≠ 0) ends the movement in the
            // same tick (§9.6 rule 4, edge case 7).
            if path.collided_mask != 0 {
                refused = true;
                prop_assert_eq!(s, Step::Stopped, "refused but still moving");
            }
            prev = cell;
            if s == Step::Stopped {
                stopped = true;
                break;
            }
        }
        prop_assert!(stopped, "still moving after 2000 ticks");
        let path = &world.paths[&P];
        // §9.7 / §9.6 rule 4: at rest on a cell centre.
        prop_assert_eq!(path.precise_x & 0xFFFF, 0x8000);
        prop_assert_eq!(path.precise_y & 0xFFFF, 0x8000);
        prop_assert_eq!((path.point_count, path.cur_point), (0, 0));
        let exhausted = mode == 3 && world.units.get(P).stats[&10] == 0;
        if !refused && !exhausted {
            // §9.5 / edge case 6: it stops on the last point.
            prop_assert_eq!(path.cell(), last);
        }
        // The footprint is where the unit is.
        let mut expect = World::new(w, h, &walls);
        clear_plus(&mut expect, start);
        expect.stamp(path.cell(), 1, 0x80, true);
        prop_assert_eq!(&world.g, &expect.g);
    }
}

// ---- requests on arbitrary payloads ----------------------------------

/// §1.3 mode check from the table.
fn ref_mode_check(u: &Unit, frame: i32, e: i32, m: u32) -> bool {
    if u.cursor && m != 0 {
        return m == 17;
    }
    if matches!(m, 0 | 1 | 5 | 17) {
        return true;
    }
    let early = frame <= e + 5;
    match u.mode {
        0 | 4 | 9 | 17 => false,
        7 | 8 | 10 | 11 => early || m == 4 || m == 9,
        12 => early,
        13 => u.class != 0,
        15 => u.class != 5,
        18 => u.used_skill.is_some_and(|s| s.seq_input > 0) || early,
        _ => true,
    }
}

fn used_skill() -> impl Strategy<Value = Option<UsedSkill>> {
    proptest::option::of(
        (
            0u16..400,
            -1i32..3,
            prop_oneof![Just(0i32), Just(67), Just(76), 0i32..200],
            any::<bool>(),
            any::<u32>(),
        )
            .prop_map(
                |(id, seq_input, srvdofunc, interrupt, skill_flags)| UsedSkill {
                    id,
                    seq_input,
                    srvdofunc,
                    interrupt,
                    skill_flags,
                },
            ),
    )
}

proptest! {
    #![proptest_config(config(512))]

    #[test]
    fn requests_never_panic(
        (w, h, walls, start, near) in arena(24),
        aim in any::<bool>(),
        id in prop_oneof![4 => 1u8..=4, 1 => any::<u8>()],
        xy in (any::<u16>(), any::<u16>()),
        ab in (prop_oneof![3 => 0u32..6, 1 => any::<u32>()], prop_oneof![3 => 0u32..4, 1 => any::<u32>()]),
        mode in prop_oneof![2 => Just(1u32), 1 => Just(2u32), 1 => Just(5u32), 3 => 0u32..22],
        class in 0u32..7,
        cursor in proptest::bool::weighted(0.15),
        states in proptest::collection::btree_set(prop_oneof![Just(13u16), Just(15), Just(42), Just(54)], 0..3),
        skill in used_skill(),
        frame in 0i32..20,
        expire in 0i32..20,
        target in (0u32..6, 0u32..4, -40i32..30, -2i32..30, 0u32..20, 0i32..4),
        town in any::<bool>(),
        ticks in 0usize..40,
    ) {
        let t = PathTables::spec().unwrap();
        let mut world = World::new(w, h, &walls);
        world.town = town;
        clear_plus(&mut world, start);
        world.add_unit(P, start.x, start.y, true, &t);
        {
            let p = world.units.get_mut(P);
            p.mode = mode;
            p.class = class;
            p.cursor = cursor;
            p.states = states.clone();
            p.used_skill = skill;
        }
        // A target unit of any type (guid 0..3 so it is sometimes found).
        let mut tu = Unit::new(UnitType::ALL[target.0 as usize], target.1);
        tu.pos = Point::new(OX + target.2, OY + target.3);
        tu.mode = target.4;
        tu.size = target.5;
        world.units.units.insert(T, tu);
        world.units.type1_expire = expire;
        world.units.frame = frame;
        let (a, b) = match (matches!(id, 0x02 | 0x04), aim) {
            (true, true) => (target.0, target.1),
            (true, false) => ab,
            (false, true) => (near.x as u32, near.y as u32),
            (false, false) => (xy.0 as u32, xy.1 as u32),
        };
        let seed0 = world.units.get(P).seed;
        let before = world.units.get(P).clone();
        let (r, o) = handle_message(&t, &mut world, P, id, a, b).unwrap();
        prop_assert_eq!(r, 0);
        if !(1..=4).contains(&id) {
            prop_assert_eq!(o, None);
            return Ok(());
        }
        let o = o.unwrap();
        let m = if id >= 3 { 3 } else { 2 };
        let found = !matches!(id, 0x02 | 0x04)
            || (a < 6 && world.find_unit(UnitType::ALL[a as usize], b).is_some());
        match o {
            Outcome::NoTargetUnit => prop_assert!(!found),
            Outcome::ModeRefused => prop_assert!(!ref_mode_check(&before, frame, expire, m)),
            Outcome::InterruptRefused => prop_assert!(ref_mode_check(&before, frame, expire, m)),
            Outcome::Moving(n) => {
                prop_assert!(n > 0 && n <= 77);
                prop_assert!(matches!(world.units.get(P).mode, 2 | 3 | 6));
            }
            Outcome::Neutral => prop_assert!(matches!(world.units.get(P).mode, 1 | 5)),
            Outcome::KnockbackIgnored | Outcome::OtherMode => prop_assert!(false, "{:?} for mode {}", o, m),
        }
        if found && !matches!(o, Outcome::ModeRefused) {
            prop_assert!(ref_mode_check(&before, frame, expire, m));
        }
        // Randomness 2: the only draw needs state 42 and an interrupt skill.
        if world.units.get(P).seed != seed0 {
            prop_assert!(before.states.contains(&42) && before.used_skill.is_some_and(|s| s.interrupt));
        }
        // Then a few ticks of event 0 on whatever was started.
        for _ in 0..ticks {
            if !matches!(world.units.get(P).mode, 2 | 3 | 6) {
                break;
            }
            let s = Walk { t: &t, c: &mut world }.player_event0(P).unwrap();
            if s == Step::Stopped {
                prop_assert!(matches!(world.units.get(P).mode, 1 | 5));
                break;
            }
        }
    }
}

fn dump(w: &World) -> String {
    let mut s = String::from("\n");
    for y in (0..w.h).rev() {
        s += &format!("{:3} ", OY + y);
        for x in 0..w.w {
            s.push(if w.g.masks[(y * w.w + x) as usize] & 1 != 0 {
                '#'
            } else {
                '.'
            });
        }
        s.push('\n');
    }
    s
}

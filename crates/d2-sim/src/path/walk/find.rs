//! Path finding: from a start cell and a target to a list of corner
//! points. No randomness except IDA* type 16
//! (the owner's unit seed).

use super::geom::{add, octant, path_distance, ray_test, step, Probe, Ray, NO_DIR};
use super::other;
use super::seams::{count, index, room_contains, PathInfo, PathWorld, Point, WalkError, WalkUnits};
use super::velocity::aim;
use crate::path::collision::find_room;
use crate::path::record::{flags, path_types, DynamicPath, PathPoint, PATH_POINTS};
use crate::path::tables::PathTables;
use crate::units::{UnitId, UnitType};

/// Path range per axis.
pub const PATH_RANGE: i32 = 100;
/// A* radius: dx² + dy² ≤ 324.
pub const ASTAR_RADIUS_SQ: i32 = 324;
/// A* node storage (the start included).
pub const ASTAR_NODES: usize = 200;
/// A* children per node.
pub const ASTAR_CHILDREN: usize = 8;
/// A* propagation stack.
pub const ASTAR_STACK: usize = 200;

/// Path type reset. The type sets are the
/// core's [`DynamicPath::set_path_type`].
pub fn reset_type(
    t: &PathTables,
    path: &mut DynamicPath,
    owner_ty: UnitType,
) -> Result<(), WalkError> {
    if path.flags & flags::SAVE_VELOCITY != 0 {
        path.velocity = path.saved_velocity;
    }
    if owner_ty == UnitType::Player {
        path.set_path_type(t, true, path_types::STRAIGHT)?;
    } else if path.flags & flags::SAVE_PREV_TYPE != 0 {
        path.set_path_type(t, false, path.prev_path_type)?;
    }
    Ok(())
}

/// Context of a path function: tables, the walk context (collision
/// source and unit-side seams).
pub struct Finder<'a, C: ?Sized> {
    pub t: &'a PathTables,
    pub c: &'a mut C,
    pub owner_ty: UnitType,
}

/// Target refresh point: the target unit's position
/// plus the target lead. The lead byte (+0x68) has no reachable writer,
/// so the lead adds 0: the position.
pub fn refresh_point<U: WalkUnits + ?Sized>(u: &U, _path: &DynamicPath, unit: UnitId) -> Point {
    u.position(unit)
}

/// Path compute. Returns the
/// point count.
pub fn compute<C: PathWorld + WalkUnits + ?Sized>(
    t: &PathTables,
    c: &mut C,
    path: &mut DynamicPath,
    unit: UnitId,
    town_access: bool,
) -> Result<i32, WalkError> {
    // Step 1.
    if path.owner != Some(unit) {
        return Err(WalkError::Fatal("path owner differs"));
    }
    let owner_ty = c.unit_type(unit);
    if path.flags & flags::MISSILE != 0 {
        // Missile path.
        return super::missile::missile_path(t, c, path, unit);
    }
    // Step 2.
    path.collided_mask = 0;
    if let Some(tu) = path.target_unit {
        let p = c.position(tu.unit);
        if p.x == 0 || p.y == 0 {
            return Ok(0);
        }
    }
    let r = (|| -> Result<Option<bool>, WalkError> {
        // Step 3.
        let start = path.cell();
        if start == Point::default() {
            return Ok(None);
        }
        // Step 4.
        let mut slack = 1;
        if let Some(tu) = path.target_unit {
            let tp = c.position(tu.unit);
            path.put_target(tp);
            if tp == Point::default() {
                return Ok(None);
            }
            match c.unit_type(tu.unit) {
                UnitType::Player | UnitType::Monster => {
                    // Target lead (path +0x68 ≠ 0): +0x68 has
                    // no reachable writer, so the lead adds 0;
                    // r = 1 either way.
                    slack = 1;
                }
                UnitType::Object => {
                    if let Some(orient) = c.door_orientation(tu.unit) {
                        let mut tg = path.target();
                        if orient {
                            tg.y += if start.y < tg.y { -2 } else { 2 };
                        } else {
                            tg.x += if start.x < tg.x { -2 } else { 2 };
                        }
                        path.put_target(tg);
                    }
                    slack = 2;
                }
                UnitType::Item => slack = 2,
                _ => slack = 1,
            }
        }
        // Step 5.
        let target = path.target();
        if target == Point::default() || start == target {
            return Ok(None);
        }
        if (target.x - start.x).abs() > PATH_RANGE || (target.y - start.y).abs() > PATH_RANGE {
            return Ok(None);
        }
        let start_room = path.room;
        let target_room = find_room(&*c, start_room, target.x, target.y);
        let (Some(_), Some(troom)) = (start_room, target_room) else {
            return Ok(None);
        };
        if !town_access
            && owner_ty == UnitType::Monster
            && !c.monster_can_be_in_town(unit)
            && c.room_in_town(troom)
        {
            return Ok(None);
        }
        // Step 6.
        c.remove_footprint(unit, false);
        let mut target_removed = None;
        if let Some(tu) = path.target_unit {
            if c.unit_size(tu.unit) != 0 && path.flags & flags::REMOVE_TARGET_FOOTPRINT != 0 {
                target_removed = Some((tu.unit, c.remove_footprint(tu.unit, false)));
            }
        }
        // Step 7.
        let mut info = info_of(path, start, target, start_room, target_room, slack);
        let mut run = true;
        if path.flags & flags::PREPARE_TARGET != 0
            && c.collides(start_room, target, path.pattern, path.move_mask)
        {
            run = prepare(t, &*c, owner_ty, path, &mut info)?;
        }
        // Step 8.
        path.cur_point = 0;
        path.point_count = 0;
        // Final target +0x18 := the compute record's target.
        path.put_final_target(info.target);
        let n = if run {
            run_function(t, c, owner_ty, path, &info)?
        } else {
            0
        };
        path.point_count = n as u32;
        // Step 9.
        if let Some((tunit, true)) = target_removed {
            c.add_footprint(tunit);
        }
        c.add_footprint(unit);
        if n == 0 {
            return Ok(Some(false));
        }
        // Step 10.
        aim(t, path, owner_ty);
        if index(path) < count(path) {
            room_exit_flag(&*c, path);
            path.put_prev_target(path.target());
            path.field_38 = 0;
            if path.flags & flags::KEEP_TARGET == 0 && path.target_unit.is_none() {
                path.put_target(path.point((count(path) - 1) as usize));
            }
            path.flags |= flags::ACTIVE;
            return Ok(Some(true));
        }
        Ok(None)
    })();
    match r? {
        Some(true) => Ok(count(path)),
        // Step 11 then 12.
        None => {
            path.cur_point = 0;
            path.point_count = 0;
            path.flags &= !flags::ACTIVE;
            Ok(0)
        }
        // Step 12.
        Some(false) => {
            path.flags &= !flags::ACTIVE;
            Ok(0)
        }
    }
}

fn info_of(
    path: &DynamicPath,
    start: Point,
    target: Point,
    start_room: Option<crate::units::RoomId>,
    target_room: Option<crate::units::RoomId>,
    slack: i32,
) -> PathInfo {
    PathInfo {
        start,
        target,
        start_room,
        target_room,
        slack,
        max_distance: path.max_distance as i32,
        idastar_score: path.ida_score as i32,
        path_type: path.path_type,
        size: path.unit_size,
        pattern: path.pattern,
        move_mask: path.move_mask,
    }
}

/// Room-exit flag.
fn room_exit_flag<W: PathWorld + ?Sized>(w: &W, path: &mut DynamicPath) {
    path.flags &= !flags::OUTSIDE_ROOM;
    let Some(room) = path.room else { return };
    for p in path.live_points() {
        if !room_contains(w, room, p) {
            path.flags |= flags::OUTSIDE_ROOM;
            return;
        }
    }
}

/// Runs the type's function (path type table).
fn run_function<C: PathWorld + WalkUnits + ?Sized>(
    t: &PathTables,
    c: &mut C,
    owner_ty: UnitType,
    path: &mut DynamicPath,
    info: &PathInfo,
) -> Result<i32, WalkError> {
    let mut f = Finder { t, c, owner_ty };
    match info.path_type {
        path_types::ASTAR => Ok(astar(&f, path, info)?),
        // 2, 5, 6, 13 share; 5 and 6 have a direction offset.
        path_types::TOWARD | 5 | 6 | path_types::TOWARD_FINISH => toward(&mut f, path, info),
        path_types::STRAIGHT => straight(&mut f, path, info),
        // IDA* (0, 16), type 3 (index 0, then Toward), knockbacks,
        // leap, back-up turn, wall follow.
        0 | 16 => other::ida_star(&mut f, path, info),
        3 => {
            path.cur_point = 0;
            toward(&mut f, path, info)
        }
        8 => Ok(other::knockback_server(&mut f, path, info)),
        9 => Ok(other::leap(&mut f, path, info)),
        11 => Ok(other::knockback_client(&mut f, path, info)),
        12 => other::back_up_turn(&mut f, path, info),
        path_types::WALL_FOLLOW => other::wall_follow(&mut f, path, info),
        // 4, 10, 14 carry flag 0x40000 and are computed as missile paths; reaching
        // the function table without it is fatal.
        _ => Err(WalkError::Fatal("path type without a function")),
    }
}

/// Target preparation. Returns false for "result 0"
/// (no path function runs).
pub fn prepare<W: PathWorld + ?Sized>(
    t: &PathTables,
    w: &W,
    owner_ty: UnitType,
    path: &mut DynamicPath,
    info: &mut PathInfo,
) -> Result<bool, WalkError> {
    if path.flags & flags::MISSILE != 0 {
        return Err(WalkError::Fatal("target preparation on a missile path"));
    }
    let start = info.start;
    let room = info.start_room;
    let free = |p: Point| !w.collides(room, p, path.pattern, path.move_mask);
    let (mut p0, mut p1, mut p2) = (info.target, info.target, info.target);
    // altdir entries are 0..7 or 255 (none).
    let row = t.altdir[octant(info.target, start)];
    let (mut d0, d1, d2) = (row[0] as u8, row[1] as u8, row[2] as u8);
    let found = loop {
        if p0 == start {
            return Ok(false);
        }
        p0 = add(p0, step(t, d0));
        if free(p0) {
            break p0;
        }
        p1 = add(p1, step(t, d1));
        if free(p1) {
            break p1;
        }
        if d2 != NO_DIR {
            p2 = add(p2, step(t, d2));
            if free(p2) {
                break p2;
            }
        }
        d0 = t.altdir[octant(p0, start)][0] as u8;
    };
    // The found probe is written
    // to the compute record's target first, then compared with the start
    // (equal → result 0 with that target = the start); rules 2–4 never
    // write the path's +0x10 / +0x12. Recorded: `interact-operate-stash`
    // frame 4, the run to the stash keeps path target (4866, 4229), the
    // stash's own cell.
    info.target = found;
    if found == start {
        return Ok(false);
    }
    if owner_ty == UnitType::Player {
        push(t, w, path, info);
    }
    Ok(true)
}

/// Orthogonal push.
fn push<W: PathWorld + ?Sized>(t: &PathTables, w: &W, path: &mut DynamicPath, info: &mut PathInfo) {
    let dx = info.target.x - info.start.x;
    let dy = info.target.y - info.start.y;
    if dx.abs() < 5 && dy.abs() < 5 && (dx, dy) != (0, 0) {
        let ox = t.snap9[(40 + dx + 9 * dy) as usize];
        let oy = t.snap9[(40 + dy + 9 * dx) as usize];
        let mut c = dx.abs().max(dy.abs());
        while c < 5 {
            let cand = Point::new(info.target.x + ox, info.target.y + oy);
            if w.collides(info.start_room, cand, path.pattern, path.move_mask) {
                break;
            }
            info.target = cand;
            c += 1;
        }
    }
}

pub(super) fn put(path: &mut DynamicPath, n: &mut usize, p: Point) -> Result<(), WalkError> {
    if *n >= PATH_POINTS {
        return Err(WalkError::Fatal("path points overflow"));
    }
    path.points[*n] = PathPoint::from_point(p);
    *n += 1;
    Ok(())
}

/// Next-position check: `None` = clear, else
/// the returned point P.
fn next_position<C: PathWorld + WalkUnits + ?Sized>(
    f: &Finder<'_, C>,
    path: &mut DynamicPath,
    info: &PathInfo,
) -> Option<Point> {
    if path.velocity == 0 {
        return Some(info.start);
    }
    aim(f.t, path, f.owner_ty);
    if path.velocity == 0 {
        return Some(info.start);
    }
    match ray_test(
        &*f.c,
        info.start_room,
        info.pattern,
        info.move_mask,
        info.start,
        info.target,
    ) {
        Ray::Clear => None,
        Ray::Blocked(p) => Some(p),
    }
}

/// Toward (type 2).
pub fn toward<C: PathWorld + WalkUnits + ?Sized>(
    f: &mut Finder<'_, C>,
    path: &mut DynamicPath,
    info: &PathInfo,
) -> Result<i32, WalkError> {
    path.cur_point = 0;
    path.point_count = 0;
    if path.dir_offset != 0 {
        // Monster circling.
        return other::circling(f, path, info);
    }
    let t = f.t;
    let start = info.start;
    let target = info.target;
    // Step 1.
    path.points[0] = PathPoint::from_point(target);
    let Some(p) = next_position(f, path, info) else {
        path.points[0] = PathPoint::from_point(target);
        return Ok(1);
    };
    // Step 2.
    if path_distance(t, p, target) <= info.slack {
        path.points[0] = PathPoint::from_point(p);
        return Ok(1);
    }
    // Step 3.
    let mut n = 0usize;
    let mut cur = start;
    if p != start {
        put(path, &mut n, p)?;
        cur = p;
    }
    // Step 4.
    let mut prev = NO_DIR;
    let mut steps = 0;
    let mut turned = false;
    let mut tail = false;
    for _ in 0..info.max_distance {
        if cur == target {
            break;
        }
        turned = false;
        let row = t.testdir[octant(cur, target)];
        let mut d = None;
        for (k, &entry) in row.iter().enumerate() {
            // testdir entries are 0..7 or 255 (none).
            let c = entry as u8;
            if c == NO_DIR {
                if k == 2 {
                    return Err(WalkError::Fatal("testdir t2 = 255"));
                }
                continue;
            }
            if !f.c.collides(
                info.start_room,
                add(cur, step(t, c)),
                info.pattern,
                info.move_mask,
            ) {
                d = Some(c);
                break;
            }
        }
        let Some(d) = d else {
            tail = true;
            break;
        };
        if prev != NO_DIR && (d.wrapping_sub(4) & 7) == prev {
            tail = true;
            break;
        }
        if d != prev {
            if cur != start {
                put(path, &mut n, cur)?;
            }
            turned = true;
        }
        cur = add(cur, step(t, d));
        steps += 1;
        prev = d;
    }
    // Step 5.
    if (tail || !turned) && steps != 0 {
        put(path, &mut n, cur)?;
    }
    // Step 6.
    Ok(n as i32)
}

/// Straight (type 7).
pub fn straight<C: PathWorld + WalkUnits + ?Sized>(
    f: &mut Finder<'_, C>,
    path: &mut DynamicPath,
    info: &PathInfo,
) -> Result<i32, WalkError> {
    path.cur_point = 0;
    path.point_count = 0;
    let n = toward(f, path, info)?;
    if n > 0 {
        let last = path.point((n - 1) as usize);
        if path_distance(f.t, last, info.target) <= info.slack && last != info.start {
            return Ok(n);
        }
    }
    let dx = info.target.x - info.start.x;
    let dy = info.target.y - info.start.y;
    if dx * dx + dy * dy <= ASTAR_RADIUS_SQ {
        let a = astar(f, path, info)?;
        if a != 0 {
            return Ok(a);
        }
        if n != 0 {
            return toward(f, path, info);
        }
    }
    Ok(n)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NodeState {
    Open,
    Closed,
}

#[derive(Clone, Copy)]
struct Node {
    p: Point,
    g: i32,
    h: i32,
    f: i32,
    parent: Option<usize>,
    children: [Option<usize>; ASTAR_CHILDREN],
    state: NodeState,
}

fn heuristic(p: Point, q: Point) -> i32 {
    let ax = (p.x - q.x).abs();
    let ay = (p.y - q.y).abs();
    2 * ax.max(ay) + ax.min(ay)
}

fn step_cost(a: Point, b: Point) -> i32 {
    if a.x == b.x || a.y == b.y {
        2
    } else {
        3
    }
}

fn add_child(nodes: &mut [Node], parent: usize, child: usize) {
    if let Some(slot) = nodes[parent].children.iter_mut().find(|c| c.is_none()) {
        *slot = Some(child);
    }
}

/// Neighbour order of the expansion.
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

/// Box probe offsets of the target room check.
const TARGET_PROBES: [(i32, i32); 8] = [
    (-2, -2),
    (-2, 2),
    (2, -2),
    (2, 2),
    (-2, 0),
    (0, -2),
    (2, 0),
    (0, 2),
];

/// A* (type 1).
pub fn astar<C: PathWorld + WalkUnits + ?Sized>(
    f: &Finder<'_, C>,
    path: &mut DynamicPath,
    info: &PathInfo,
) -> Result<i32, WalkError> {
    let w = &*f.c;
    let target = info.target;
    let room = info.start_room;
    let collides = |p: Point| w.collides(room, p, info.pattern, info.move_mask);
    // Rule 1.
    if path.target_unit.is_some()
        && TARGET_PROBES
            .iter()
            .all(|&(dx, dy)| collides(Point::new(target.x + dx, target.y + dy)))
    {
        return Ok(0);
    }
    // Rules 2–4.
    let mut nodes: Vec<Node> = Vec::with_capacity(ASTAR_NODES);
    let h0 = heuristic(info.start, target);
    nodes.push(Node {
        p: info.start,
        g: 0,
        h: h0,
        f: h0,
        parent: None,
        children: [None; ASTAR_CHILDREN],
        state: NodeState::Open,
    });
    let mut open: Vec<usize> = vec![0];
    let mut best: Option<usize> = None;
    // Rule 5.
    'search: while !open.is_empty() {
        let c = open.remove(0);
        nodes[c].state = NodeState::Closed;
        best = match best {
            None => Some(c),
            Some(b) => {
                if nodes[c].h < nodes[b].h
                    || (nodes[c].h == nodes[b].h && nodes[c].g > nodes[b].g + 5)
                {
                    Some(c)
                } else {
                    Some(b)
                }
            }
        };
        if nodes[c].h == 0 {
            break;
        }
        let cp = nodes[c].p;
        for &(dx, dy) in NEIGHBOURS.iter() {
            let q = Point::new(cp.x + dx, cp.y + dy);
            if collides(q) {
                continue;
            }
            let g2 = nodes[c].g + step_cost(cp, q);
            match nodes.iter().position(|n| n.p == q) {
                Some(qi) if nodes[qi].state == NodeState::Open => {
                    add_child(&mut nodes, c, qi);
                    if g2 < nodes[qi].g {
                        nodes[qi].parent = Some(c);
                        nodes[qi].g = g2;
                        nodes[qi].f = g2 + nodes[qi].h;
                    }
                }
                Some(qi) => {
                    add_child(&mut nodes, c, qi);
                    if g2 < nodes[qi].g {
                        nodes[qi].parent = Some(c);
                        nodes[qi].g = g2;
                        nodes[qi].f = g2 + nodes[qi].h;
                        propagate(&mut nodes, qi)?;
                    }
                }
                None => {
                    if nodes.len() >= ASTAR_NODES {
                        break 'search;
                    }
                    let h = heuristic(q, target);
                    let qi = nodes.len();
                    nodes.push(Node {
                        p: q,
                        g: g2,
                        h,
                        f: g2 + h,
                        parent: Some(c),
                        children: [None; ASTAR_CHILDREN],
                        state: NodeState::Open,
                    });
                    add_child(&mut nodes, c, qi);
                    let at = open
                        .iter()
                        .position(|&o| nodes[o].f >= g2 + h)
                        .unwrap_or(open.len());
                    open.insert(at, qi);
                }
            }
        }
    }
    // Rule 6.
    let Some(best) = best else { return Ok(0) };
    let mut out: Vec<Point> = Vec::new();
    let mut prev_step: Option<(i32, i32)> = None;
    let mut cur = best;
    while let Some(parent) = nodes[cur].parent {
        let s = (
            nodes[cur].p.x - nodes[parent].p.x,
            nodes[cur].p.y - nodes[parent].p.y,
        );
        if prev_step != Some(s) {
            out.push(nodes[cur].p);
            prev_step = Some(s);
            if out.len() >= 78 {
                break;
            }
        }
        cur = parent;
    }
    let n = out.len();
    if n == 0 || n >= 78 {
        return Ok(0);
    }
    for (k, p) in out.iter().rev().enumerate() {
        path.points[k] = PathPoint::from_point(*p);
    }
    Ok(n as i32)
}

/// Propagation of an improved closed node (A* rule 5, stack ≤ 200;
/// edge case 2: more is fatal).
fn propagate(nodes: &mut [Node], from: usize) -> Result<(), WalkError> {
    let mut stack = vec![from];
    while let Some(n) = stack.pop() {
        for k in 0..ASTAR_CHILDREN {
            let Some(c) = nodes[n].children[k] else { break };
            let g2 = nodes[n].g + step_cost(nodes[n].p, nodes[c].p);
            if g2 < nodes[c].g {
                nodes[c].parent = Some(n);
                nodes[c].g = g2;
                nodes[c].f = g2 + nodes[c].h;
                if stack.len() >= ASTAR_STACK {
                    return Err(WalkError::Fatal("A* propagation stack overflow"));
                }
                stack.push(c);
            }
        }
    }
    Ok(())
}

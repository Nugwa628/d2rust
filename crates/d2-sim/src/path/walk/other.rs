//! The path functions of monster AI and skills. Same calling form
//! as Toward: a [`PathInfo`] and the path's points; each returns
//! the count. IDA* type 16 draws on the path owner's unit seed (IDA*
//! rule 2); every other function is deterministic without draws.

use super::find::{put, Finder};
use super::geom::{add, octant, ray_test, step, Probe, Ray, NO_DIR};
use super::seams::{PathInfo, PathWorld, Point, WalkError, WalkUnits};
use super::velocity::aim;
use crate::path::record::{DynamicPath, PathPoint};
use crate::path::tables::PathTables;
use crate::units::UnitType;

// ---- circling walk ----------------------------------------------

/// Circling walk (types 5, 6 and 12): circling step 4
/// from the start, appending after the current count, every `testdir`
/// entry turned by the direction offset k = path +0x98. Writes and
/// returns the count.
pub fn circling<C: PathWorld + WalkUnits + ?Sized>(
    f: &mut Finder<'_, C>,
    path: &mut DynamicPath,
    info: &PathInfo,
) -> Result<i32, WalkError> {
    let t = f.t;
    let k = path.dir_offset;
    let target = info.target;
    let mut n = path.point_count as usize;
    let mut cur = info.start;
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
        // Each entry gets (t + k) & 7 before the first-free pick; none
        // can be 255 after it.
        let d = row.iter().map(|&e| ((e + k) & 7) as u8).find(|&c| {
            !f.c.collides(
                info.start_room,
                add(cur, step(t, c)),
                info.pattern,
                info.move_mask,
            )
        });
        let Some(d) = d else {
            tail = true;
            break;
        };
        if prev != NO_DIR && (d.wrapping_sub(4) & 7) == prev {
            tail = true;
            break;
        }
        if d != prev {
            // No "unless cur = start": the first step appends the start.
            put(path, &mut n, cur)?;
            turned = true;
        }
        cur = add(cur, step(t, d));
        steps += 1;
        prev = d;
    }
    if (tail || !turned) && steps != 0 {
        put(path, &mut n, cur)?;
    }
    path.point_count = n as u32;
    Ok(n as i32)
}

// ---- back-up turn -------------------------------------------------

/// Type 12, back-up turn.
pub fn back_up_turn<C: PathWorld + WalkUnits + ?Sized>(
    f: &mut Finder<'_, C>,
    path: &mut DynamicPath,
    info: &PathInfo,
) -> Result<i32, WalkError> {
    path.cur_point = 0;
    path.dir_offset = -4;
    let n = circling(f, path, info)?;
    if n == 0 {
        return Ok(0);
    }
    path.point_count = n as u32;
    path.dir_offset = -2;
    let mut r = circling(f, path, info)?;
    if r == n {
        path.dir_offset = 2;
        r = circling(f, path, info)?;
    }
    path.dir_offset = -4;
    Ok(r)
}

// ---- leap and knockbacks -------------------------------------

/// Next-position check toward `p` (point 0
/// := `p` first): the point the ray reaches, `p` when clear.
fn next_point<C: PathWorld + WalkUnits + ?Sized>(
    f: &Finder<'_, C>,
    path: &mut DynamicPath,
    info: &PathInfo,
    p: Point,
) -> Point {
    path.points[0] = PathPoint::from_point(p);
    if path.velocity == 0 {
        return info.start;
    }
    aim(f.t, path, f.owner_ty);
    if path.velocity == 0 {
        return info.start;
    }
    match ray_test(
        &*f.c,
        info.start_room,
        info.pattern,
        info.move_mask,
        info.start,
        p,
    ) {
        Ray::Clear => p,
        Ray::Blocked(q) => q,
    }
}

/// Type 9, leap.
pub fn leap<C: PathWorld + WalkUnits + ?Sized>(
    f: &mut Finder<'_, C>,
    path: &mut DynamicPath,
    info: &PathInfo,
) -> i32 {
    path.cur_point = 0;
    let p = next_point(f, path, info, info.target);
    if p == info.start {
        return 0;
    }
    path.points[0] = PathPoint::from_point(p);
    path.point_count = 1;
    1
}

/// 16-bit wrapping add of a coordinate.
fn add16(a: i32, b: i32) -> i32 {
    i32::from(a.wrapping_add(b) as u16)
}

/// Type 8, knockback server. The index is not reset.
pub fn knockback_server<C: PathWorld + WalkUnits + ?Sized>(
    f: &mut Finder<'_, C>,
    path: &mut DynamicPath,
    info: &PathInfo,
) -> i32 {
    let start = info.start;
    let mut p = info.target;
    if let Some(tu) = path.target_unit {
        let k = i32::from(path.dist_budget >> 1) + 1;
        let at = f.c.position(tu.unit);
        let (mut dx, mut dy) = (start.x - at.x, start.y - at.y);
        let m = dx.abs().max(dy.abs());
        if m != 0 {
            dx = k * dx / m;
            dy = k * dy / m;
        }
        p = Point::new(add16(start.x, dx), add16(start.y, dy));
    }
    path.points[0] = PathPoint::from_point(p);
    if let Ray::Blocked(q) = ray_test(
        &*f.c,
        info.start_room,
        info.pattern,
        info.move_mask,
        start,
        p,
    ) {
        p = q;
    }
    // The point target; the target unit goes.
    path.target_unit = None;
    path.put_target(p);
    if p == start {
        return 0;
    }
    path.points[0] = PathPoint::from_point(p);
    path.point_count = 1;
    1
}

/// Type 11, knockback client: result 1, the count
/// not written.
pub fn knockback_client<C: PathWorld + WalkUnits + ?Sized>(
    f: &mut Finder<'_, C>,
    path: &mut DynamicPath,
    info: &PathInfo,
) -> i32 {
    path.cur_point = 0;
    let p = next_point(f, path, info, info.target);
    path.points[0] = PathPoint::from_point(p);
    1
}

// ---- IDA* ---------------------------------------------------------

/// Node pool (`0x384`).
pub const IDA_NODES: usize = 900;
/// Turns of one depth-first pass.
pub const IDA_TURNS: u32 = 10_000;
/// Largest search box area (fatal above).
pub const IDA_MAX_AREA: i32 = 50_000;
/// Threshold step between passes.
pub const IDA_STEP: i32 = 5;
/// Order table (8 rows × 8 increments) followed in memory
/// by the random offsets R (32 entries): an order pointer
/// past row 7 reads R (IDA* rule 6, a backtrack into a root on its last
/// entry).
pub const IDA_ORDER: [i32; 96] = {
    const ROWS: [[i32; 8]; 8] = [
        [0, 1, 6, 3, 4, 5, 2, 7],
        [0, 1, 6, 3, 1, 3, 6, 1],
        [0, 1, 1, 1, 1, 1, 2, 7],
        [1, 1, 1, 1, 1, 3, 6, 1],
        [6, 4, 3, 6, 1, 3, 2, 7],
        [7, 7, 7, 7, 7, 5, 2, 7],
        [0, 7, 7, 7, 7, 5, 2, 7],
        [0, 7, 2, 5, 7, 5, 2, 7],
    ];
    let mut t = [0i32; 96];
    let mut i = 0;
    while i < 64 {
        t[i] = ROWS[i / 8][i % 8];
        i += 1;
    }
    while i < 96 {
        t[i] = IDA_RANDOM[i - 64];
        i += 1;
    }
    t
};
/// Random direction offsets R: (−2, −1, 0, 1, 2) six times,
/// then −1, 1.
pub const IDA_RANDOM: [i32; 32] = {
    let mut t = [0i32; 32];
    let mut i = 0;
    while i < 30 {
        t[i] = (i % 5) as i32 - 2;
        i += 1;
    }
    t[30] = -1;
    t[31] = 1;
    t
};

/// A search node (0x1C bytes; its f and h fields are written,
/// never read back by the search).
#[derive(Clone, Copy, Default)]
struct IdaNode {
    g: u16,
    tries: i16,
    pos: Point,
    /// Index into [`IDA_ORDER`].
    order: usize,
    dir: u8,
    parent: Option<usize>,
    child: Option<usize>,
}

/// h(p) = min + 2·max of the axis distances to the target.
fn ida_h(p: Point, target: Point) -> i32 {
    let ax = (target.x - p.x).abs();
    let ay = (target.y - p.y).abs();
    ax.min(ay) + 2 * ax.max(ay)
}

/// pref(p): the first `testdir` entry of o(p → target).
fn pref(t: &PathTables, p: Point, target: Point) -> u8 {
    (t.testdir[octant(p, target)][0] & 7) as u8
}

struct Ida<'a, 'f, C: ?Sized> {
    f: &'a mut Finder<'f, C>,
    info: &'a PathInfo,
    owner: Option<crate::units::UnitId>,
    random: bool,
    slack2: i32,
    bx: i32,
    by: i32,
    w: i32,
    h: i32,
    grid: Vec<u32>,
    nodes: Vec<IdaNode>,
}

impl<C: PathWorld + WalkUnits + ?Sized> Ida<'_, '_, C> {
    /// R[s] of one step of the owner's unit seed (random mode), else 0.
    fn jitter(&mut self) -> i32 {
        if !self.random {
            return 0;
        }
        let Some(u) = self.owner else { return 0 };
        let lo = self.f.c.seed(u).step();
        IDA_RANDOM[(lo & 0x1F) as usize]
    }

    /// The next order entry and the direction it gives.
    fn advance(&mut self, n: usize) {
        self.nodes[n].order += 1;
        let e = IDA_ORDER.get(self.nodes[n].order).copied().unwrap_or(0);
        let j = self.jitter();
        let d = i32::from(self.nodes[n].dir) + e + j;
        self.nodes[n].dir = (d & 7) as u8;
    }

    /// Next try: `None` = not found (back at the
    /// root), else the node to continue with.
    fn next_try(&mut self, mut n: usize) -> Option<usize> {
        if self.nodes[n].tries < 4 {
            self.advance(n);
        }
        self.nodes[n].tries += 1;
        while self.nodes[n].tries == 5 {
            n = self.nodes[n].parent?;
            self.advance(n);
            self.nodes[n].tries += 1;
        }
        Some(n)
    }

    fn cell(&self, p: Point) -> usize {
        ((p.y - self.by + 3) * (self.w + 6) + (p.x - self.bx + 3)) as usize
    }

    fn in_box(&self, p: Point) -> bool {
        (self.bx..=self.bx + self.w).contains(&p.x) && (self.by..=self.by + self.h).contains(&p.y)
    }

    fn reset_root(&mut self) {
        let start = self.info.start;
        self.nodes.clear();
        self.nodes.push(IdaNode {
            g: 0,
            tries: -3,
            pos: start,
            order: 0,
            dir: pref(self.f.t, start, self.info.target),
            parent: None,
            child: None,
        });
    }

    /// Depth-first pass with threshold `limit`.
    fn pass(&mut self, limit: i32) -> Option<usize> {
        let target = self.info.target;
        let mut n = 0usize;
        let mut turns = 0u32;
        loop {
            let node = self.nodes[n];
            if node.pos == target {
                return Some(n);
            }
            turns += 1;
            if turns > IDA_TURNS {
                return None;
            }
            if !self.in_box(node.pos) {
                return Some(n);
            }
            let c = add(node.pos, step(self.f.t, node.dir));
            let ci = self.cell(c);
            if self.grid[ci] == 0
                && self.f.c.collides(
                    self.info.start_room,
                    c,
                    self.info.pattern,
                    self.info.move_mask,
                )
            {
                self.grid[ci] = 1;
                n = self.next_try(n)?;
                continue;
            }
            let cost = if c.x == node.pos.x || c.y == node.pos.y {
                2
            } else {
                3
            };
            let gc = node.g.wrapping_add(cost);
            if self.grid[ci] != 0 && self.grid[ci] < u32::from(gc) {
                n = self.next_try(n)?;
                continue;
            }
            self.grid[ci] = u32::from(gc);
            let hc = ida_h(c, target) as u16;
            let fc = hc.wrapping_add(gc);
            if i32::from(fc) > limit {
                n = self.next_try(n)?;
                continue;
            }
            // The one child slot is reused for every try; a reused node
            // keeps its own child slot (only a new pool node is zeroed).
            let child = match node.child {
                Some(ch) => ch,
                None => {
                    if self.nodes.len() >= IDA_NODES {
                        return None;
                    }
                    self.nodes.push(IdaNode::default());
                    let ch = self.nodes.len() - 1;
                    self.nodes[n].child = Some(ch);
                    ch
                }
            };
            let row = ((i32::from(pref(self.f.t, c, target)) - i32::from(node.dir)) & 7) as usize;
            let order = row * 8;
            let j = self.jitter();
            let dir = ((i32::from(node.dir) + IDA_ORDER[order] + j) & 7) as u8;
            self.nodes[child] = IdaNode {
                g: gc,
                tries: 0,
                pos: c,
                order,
                dir,
                parent: Some(n),
                child: self.nodes[child].child,
            };
            if i32::from(hc) < self.slack2 {
                return Some(child);
            }
            n = child;
        }
    }

    /// Output.
    fn output(&self, path: &mut DynamicPath, found: usize) -> i32 {
        let mut out: Vec<Point> = Vec::new();
        let mut last_step: Option<(i32, i32)> = None;
        let mut cur = found;
        while let Some(parent) = self.nodes[cur].parent {
            let p = self.nodes[cur].pos;
            let q = self.nodes[parent].pos;
            let s = (p.x - q.x, p.y - q.y);
            if last_step != Some(s) {
                if out.len() >= 78 {
                    break;
                }
                out.push(p);
                last_step = Some(s);
            }
            cur = parent;
        }
        let n = out.len();
        if !(2..=77).contains(&n) {
            return 0;
        }
        for (k, p) in out.iter().rev().enumerate() {
            path.points[k] = PathPoint::from_point(*p);
        }
        n as i32
    }
}

/// Types 0 and 16, IDA*.
pub fn ida_star<C: PathWorld + WalkUnits + ?Sized>(
    f: &mut Finder<'_, C>,
    path: &mut DynamicPath,
    info: &PathInfo,
) -> Result<i32, WalkError> {
    // Rule 1.
    let slack2 = i32::from((info.slack as u8).wrapping_mul(2));
    let Some(s) = info.start_room.and_then(|r| f.c.subtile_rect(r)) else {
        return Err(WalkError::Fatal("IDA* start room without a box"));
    };
    let (mut bx, mut by, mut w, mut h) = (s.x, s.y, s.w, s.h);
    let other = info
        .target_room
        .filter(|&r| Some(r) != info.start_room)
        .and_then(|r| f.c.subtile_rect(r));
    if let Some(o) = other {
        if o.x < s.x {
            (bx, w) = (o.x, s.x + s.w - o.x);
        } else {
            (bx, w) = (s.x, o.x + o.w - s.x);
        }
        if o.y < s.y {
            (by, h) = (o.y, s.y + s.h - o.y);
        } else {
            (by, h) = (s.y, o.y + o.h - s.y);
        }
    }
    if other.is_none() || w == s.w {
        bx -= 10;
        w += 20;
    }
    if other.is_none() || h == s.h {
        by -= 10;
        h += 20;
    }
    // Rule 2.
    if w * h > IDA_MAX_AREA {
        return Err(WalkError::Fatal("IDA* box over 50,000 cells"));
    }
    let mut ida = Ida {
        owner: path.owner,
        random: info.path_type == 16,
        slack2,
        bx,
        by,
        w,
        h,
        grid: vec![0; ((w + 6) * (h + 6)) as usize],
        nodes: Vec::with_capacity(IDA_NODES),
        info,
        f,
    };
    // Rule 4.
    let h0 = ida_h(info.start, info.target);
    let mut limit = if ida.random { h0 + h0 / 2 } else { h0 };
    let max = limit.max(info.idastar_score);
    loop {
        ida.reset_root();
        if let Some(found) = ida.pass(limit) {
            return Ok(ida.output(path, found));
        }
        if ida.nodes.len() >= IDA_NODES {
            return Ok(0);
        }
        limit += IDA_STEP;
        if limit >= max {
            return Ok(0);
        }
    }
}

// ---- wall follow --------------------------------------------------

/// Cell buffer of the wall follow.
pub const WALL_CELLS: usize = 88;
/// Points per follower.
pub const FOLLOWER_POINTS: usize = 201;
/// Steps of the wall-follow directions: 0 (0, −1), 1 (1,
/// −1), 2 (1, 0), 3 (1, 1), 4 (0, 1), 5 (−1, 1), 6 (−1, 0), 7 (−1, −1).
pub const WALL_STEPS: [(i32, i32); 8] = [
    (0, -1),
    (1, -1),
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
];
/// A's turn after a free step.
const A_FREE: [usize; 8] = [6, 6, 0, 0, 2, 2, 4, 4];
/// A's turn after a blocked try, also B's free turn.
const A_BLOCKED: [usize; 8] = [2, 2, 4, 4, 6, 6, 0, 0];
/// B's turn after a blocked try.
const B_BLOCKED: [usize; 8] = [6, 0, 0, 2, 2, 4, 4, 6];

fn wall_step(p: Point, d: usize) -> Point {
    let (x, y) = WALL_STEPS[d & 7];
    Point::new(p.x + x, p.y + y)
}

/// dir(P → q) of two neighbouring cells.
fn wall_dir(p: Point, q: Point) -> usize {
    let d = (q.x - p.x, q.y - p.y);
    WALL_STEPS.iter().position(|&s| s == d).unwrap_or(0)
}

#[derive(Clone)]
struct Follower {
    pos: Point,
    dir: usize,
    points: Vec<Point>,
    mark: usize,
    done: bool,
    free: &'static [usize; 8],
    blocked: &'static [usize; 8],
}

/// What a turn ended in.
enum Turn {
    Go,
    Rejoined,
    Failed,
}

struct Wall<'a, 'f, C: ?Sized> {
    f: &'a mut Finder<'f, C>,
    info: &'a PathInfo,
    start: Point,
    l: i32,
    d: usize,
    buf: Vec<Point>,
    i: usize,
    n: usize,
}

impl<C: PathWorld + WalkUnits + ?Sized> Wall<'_, '_, C> {
    fn blocked(&self, p: Point) -> bool {
        self.f.c.collides(
            self.info.start_room,
            p,
            self.info.pattern,
            self.info.move_mask,
        )
    }

    /// Rule 2: the line from S (excluded) to the target (included).
    fn line(&mut self) {
        let (s, e) = (self.start, self.info.target);
        let (dx, dy) = (e.x - s.x, e.y - s.y);
        if (dx, dy) == (0, 0) {
            return;
        }
        let x_major = dx.abs() >= dy.abs();
        let (major, minor) = if x_major { (dx, dy) } else { (dy, dx) };
        let len = major.abs();
        if len > self.l - 1 {
            return;
        }
        self.d = match (x_major, major < 0) {
            (false, true) => 0,
            (false, false) => 2,
            (true, false) => 1,
            (true, true) => 3,
        };
        let (smaj, smin) = (major.signum(), minor.signum());
        let mut err = 0;
        let mut p = s;
        for _ in 0..len {
            let (mut a, mut b) = if x_major { (p.x, p.y) } else { (p.y, p.x) };
            a += smaj;
            err += minor.abs();
            if err >= len {
                err -= len;
                b += smin;
            }
            p = if x_major {
                Point::new(a, b)
            } else {
                Point::new(b, a)
            };
            self.buf.push(p);
        }
    }

    /// Step (rule 4.1): the candidate, or `None` (done).
    fn try_step(&self, fl: &mut Follower) -> Option<Point> {
        for _ in 0..4 {
            let c = wall_step(fl.pos, fl.dir);
            if !self.blocked(c) {
                return Some(c);
            }
            fl.dir = fl.blocked[fl.dir & 7];
        }
        None
    }

    /// One turn of `fl` with the other follower `o` (rule 4).
    fn turn(&mut self, p: Point, fl: &mut Follower, o: &Follower) -> Result<Turn, WalkError> {
        let Some(c) = self.try_step(fl) else {
            fl.done = true;
            return Ok(Turn::Go);
        };
        let l_i = self.l - self.i as i32;
        // Rule 4.2.
        if !fl.points.is_empty() {
            let k = match self.d {
                0 => p.y - c.y,
                1 => c.x - p.x,
                2 => c.y - p.y,
                _ => p.x - c.x,
            };
            if k > 0 {
                let j = self.i + k as usize - 1;
                if j < self.n && self.buf[j] == c {
                    if fl.points.len() as i32 >= l_i {
                        return Err(WalkError::Fatal("wall-follow rejoin past the budget"));
                    }
                    fl.points.push(c);
                    let m = fl.points.len();
                    self.buf.splice(self.i..=j, fl.points.iter().copied());
                    if self.buf.len() > WALL_CELLS {
                        return Err(WalkError::Fatal("wall-follow cell buffer overflow"));
                    }
                    self.n = self.n + m - (j - self.i + 1);
                    self.i += m;
                    return Ok(Turn::Rejoined);
                }
            }
        }
        // Rule 4.3.
        if o.points.len() > 1 {
            if fl.mark != 0 {
                if o.points.get(fl.mark - 2) == Some(&c) {
                    return Ok(Turn::Failed);
                }
                fl.mark = 0;
            }
            if c == o.pos {
                fl.mark = o.points.len();
            }
        }
        // Rule 4.4.
        if fl.points.len() >= FOLLOWER_POINTS {
            return Err(WalkError::Fatal("wall-follow follower past 201 points"));
        }
        fl.points.push(c);
        fl.pos = c;
        fl.dir = fl.free[fl.dir & 7];
        if fl.points.len() as i32 >= l_i - self.n as i32 - 1 {
            fl.done = true;
        }
        Ok(Turn::Go)
    }

    /// Repair (rules 4–5) at `buf[i]` from P. `false` =
    /// failed.
    fn repair(&mut self, p: Point) -> Result<bool, WalkError> {
        let d0 = wall_dir(p, self.buf[self.i]);
        let mut a = Follower {
            pos: p,
            dir: d0 + 1,
            points: Vec::new(),
            mark: 0,
            done: false,
            free: &A_FREE,
            blocked: &A_BLOCKED,
        };
        let mut b = Follower {
            dir: d0 + 7,
            free: &A_BLOCKED,
            blocked: &B_BLOCKED,
            ..a.clone()
        };
        let mut a_turn = true;
        while !a.done && !b.done {
            let (fl, o) = if a_turn { (&mut a, &b) } else { (&mut b, &a) };
            let o = o.clone();
            match self.turn(p, fl, &o)? {
                Turn::Rejoined => return Ok(true),
                Turn::Failed => {
                    self.cut();
                    return Ok(false);
                }
                Turn::Go => {}
            }
            if fl.mark == 0 || fl.done {
                a_turn = !a_turn;
            }
        }
        // Rule 5.
        if self.l - self.i as i32 > 80 {
            self.cut();
            return Ok(false);
        }
        let t = self.info.target;
        let d2 = |q: Point| (q.x - t.x) * (q.x - t.x) + (q.y - t.y) * (q.y - t.y);
        // A follower without points: the dword read as its "last point"
        // is its own done flag, so (1, 0) when done, else (0, 0).
        let last = |fl: &Follower| {
            fl.points
                .last()
                .copied()
                .unwrap_or(Point::new(i32::from(fl.done), 0))
        };
        let (da, db, ds) = (d2(last(&a)), d2(last(&b)), d2(self.start));
        let chosen = if db > da {
            (ds >= da).then_some(a)
        } else {
            (ds >= db).then_some(b)
        };
        let Some(c) = chosen else {
            return Ok(false);
        };
        self.buf.truncate(self.i);
        self.buf.extend(c.points.iter().copied());
        if self.buf.len() > WALL_CELLS {
            return Err(WalkError::Fatal("wall-follow cell buffer overflow"));
        }
        self.i += c.points.len();
        self.n = self.i;
        Ok(true)
    }

    /// A failed repair: the buffer cut at i.
    fn cut(&mut self) {
        self.buf.truncate(self.i);
        self.n = self.i;
    }

    /// Compression of the first `n` cells.
    fn compress(&self, path: &mut DynamicPath, n: usize) -> i32 {
        let cells = &self.buf[..n.min(self.buf.len())];
        let Some(&last) = cells.last() else { return 0 };
        let mut out = Vec::new();
        if cells.len() > 1 {
            let mut p = (cells[0].x - self.start.x, cells[0].y - self.start.y);
            let mut run = 0;
            for k in 0..cells.len() - 1 {
                let mut d = (cells[k + 1].x - cells[k].x, cells[k + 1].y - cells[k].y);
                if d == p {
                    run += 1;
                } else if run <= 0 && d.0 != p.0 && d.1 != p.1 {
                    run = 1;
                    d.0 = -2;
                } else {
                    out.push(cells[k]);
                    run = 0;
                }
                p = d;
            }
        }
        out.push(last);
        for (k, q) in out.iter().enumerate() {
            path.points[k] = PathPoint::from_point(*q);
        }
        out.len() as i32
    }
}

/// Type 15, wall follow.
pub fn wall_follow<C: PathWorld + WalkUnits + ?Sized>(
    f: &mut Finder<'_, C>,
    path: &mut DynamicPath,
    info: &PathInfo,
) -> Result<i32, WalkError> {
    if !matches!(f.owner_ty, UnitType::Player | UnitType::Monster) {
        return Err(WalkError::Fatal(
            "wall follow of a non-player, non-monster path",
        ));
    }
    // Rule 1.
    let mut l = i32::from(path.max_distance);
    if path.target_unit.is_some() && l < 40 {
        l = 40;
    }
    let mut w = Wall {
        f,
        info,
        start: info.start,
        l,
        d: 0,
        buf: Vec::with_capacity(WALL_CELLS),
        i: 0,
        n: 0,
    };
    // Rule 2.
    w.line();
    w.n = w.buf.len();
    if w.n > WALL_CELLS {
        return Err(WalkError::Fatal("wall-follow line past the cell buffer"));
    }
    if w.n <= 2 {
        return Ok(0);
    }
    // Rule 3.
    let mut p = w.start;
    while w.i < w.n {
        if w.blocked(w.buf[w.i]) && !w.repair(p)? {
            return Ok(w.compress(path, w.i));
        }
        // After a repair: the cell at the new i, untested.
        p = w.buf.get(w.i).copied().unwrap_or(p);
        w.i += 1;
    }
    Ok(w.compress(path, w.n))
}

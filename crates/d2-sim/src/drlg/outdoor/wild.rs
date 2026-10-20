//! Act I outdoor levels: cliff marking, river and
//! caves, transitions, special presets, dirt paths (grid path search and
//! jitter).

use super::grid::{cell, file_of, spawn_valid, Gen, Op};
use super::tilesub::BorderCtx;
use super::{OutdoorError, PathEnds, PathPoint};

/// River upper/lower files for P = 4..15.
pub const RIVER_FILES: [(i32, i32); 12] = [
    (2, 2),
    (0, 3),
    (1, 1),
    (3, 0),
    (0, 2),
    (0, 1),
    (1, 0),
    (2, 0),
    (2, 3),
    (1, 3),
    (3, 1),
    (3, 2),
];

/// Path search order rows.
pub const ORDER: [[i32; 4]; 4] = [[0, 1, 2, 3], [0, 1, 1, 1], [3, 2, 1, 2], [0, 3, 2, 1]];
/// Steps by facing.
pub const STEP_X: [i32; 4] = [1, 0, -1, 0];
pub const STEP_Y: [i32; 4] = [0, 1, 0, -1];
/// Dir table, index 5dx + dy + 12.
pub const DIR_TABLE: [i32; 25] = [
    5, 4, 4, 4, 3, 6, 5, 4, 3, 2, 6, 6, 6, 2, 2, 6, 7, 0, 1, 2, 7, 0, 0, 0, 1,
];
/// Node budget of one search round.
pub const PATH_NODE_LIMIT: usize = 900;

/// Dir(p, q): the direction from p to q, with the parity
/// quirk on a non-negative minor axis.
pub fn dir(p: (i32, i32), q: (i32, i32)) -> i32 {
    let (mut dx, mut dy) = (q.0 - p.0, q.1 - p.1);
    if dx.abs() >= 2 * dy.abs() {
        dy = if dy < 0 { -1 } else { dy & 1 };
    } else if dy.abs() >= 2 * dx.abs() {
        dx = if dx < 0 { -1 } else { dx & 1 };
    }
    dx = dx.clamp(-2, 2);
    dy = dy.clamp(-2, 2);
    DIR_TABLE[(5 * dx + dy + 12) as usize]
}

fn h(p: (i32, i32), b: (i32, i32)) -> i32 {
    let (dx, dy) = ((p.0 - b.0).abs(), (p.1 - b.1).abs());
    dx.min(dy) + 2 * dx.max(dy)
}

struct Node {
    cell: (i32, i32),
    g: i32,
    tries: i32,
    row: usize,
    pos: usize,
    facing: i32,
    parent: Option<usize>,
    child: Option<usize>,
}

enum Round {
    Found(Vec<(i32, i32)>),
    Failed,
    OutOfNodes,
}

/// Grid path search: cells from B
/// back to A, or `None`. `blocked(c)` = grid-2 bit 0x200 at c; the grid
/// is (0, 0, gw, gh).
pub fn grid_path(
    a: (i32, i32),
    b: (i32, i32),
    gw: i32,
    gh: i32,
    blocked: impl Fn((i32, i32)) -> bool,
) -> Option<Vec<(i32, i32)>> {
    if (a.0 - b.0).abs() + (a.1 - b.1).abs() < 2 {
        return Some(vec![a, b]);
    }
    let base = h(a, b) + h(a, b) / 2;
    let mut budget = base;
    loop {
        match search_round(a, b, gw, gh, budget, &blocked) {
            Round::Found(p) => return Some(p),
            Round::OutOfNodes => return None,
            Round::Failed => {
                budget += 5;
                if budget >= base + 35 {
                    return None;
                }
            }
        }
    }
}

fn search_round(
    a: (i32, i32),
    b: (i32, i32),
    gw: i32,
    gh: i32,
    budget: i32,
    blocked: &impl Fn((i32, i32)) -> bool,
) -> Round {
    let mut nodes = vec![Node {
        cell: a,
        g: 0,
        tries: -1,
        row: 0,
        pos: 0,
        facing: (dir(a, b) / 2) & 3,
        parent: None,
        child: None,
    }];
    // TODO: whether the root counts toward the 900
    // nodes is not stated; it does here.
    let mut cur = 0usize;
    while nodes[cur].cell != b {
        let n = &nodes[cur];
        let c = (
            n.cell.0 + STEP_X[n.facing as usize],
            n.cell.1 + STEP_Y[n.facing as usize],
        );
        let on_chain = {
            let mut k = Some(cur);
            let mut hit = false;
            while let Some(i) = k {
                if nodes[i].cell == c {
                    hit = true;
                    break;
                }
                k = nodes[i].parent;
            }
            hit
        };
        let ok =
            c == b || (c.0 >= 0 && c.1 >= 0 && c.0 < gw && c.1 < gh && !blocked(c) && !on_chain);
        if ok {
            let g2 = n.g + 2;
            if g2 + h(c, b) <= budget {
                let d = dir(c, b) / 2;
                let row = ((n.facing - d) & 3) as usize;
                let child = Node {
                    cell: c,
                    g: g2,
                    tries: 0,
                    row,
                    pos: 0,
                    facing: (d + ORDER[row][0]) & 3,
                    parent: Some(cur),
                    child: None,
                };
                let id = match nodes[cur].child {
                    Some(id) => {
                        nodes[id] = child;
                        id
                    }
                    None => {
                        if nodes.len() >= PATH_NODE_LIMIT {
                            return Round::OutOfNodes;
                        }
                        nodes.push(child);
                        let id = nodes.len() - 1;
                        nodes[cur].child = Some(id);
                        id
                    }
                };
                cur = id;
                continue;
            }
        }
        // Advance, climbing while tries reach 3.
        // TODO: "reaching the root this way fails the
        // round" is read as the root's own tries reaching 3 (a climb to the
        // root advances it like any node).
        loop {
            let n = &mut nodes[cur];
            if n.tries < 4 {
                n.pos += 1;
                if n.pos < 4 {
                    n.facing = (n.facing + ORDER[n.row][n.pos]) & 3;
                }
            }
            n.tries += 1;
            if n.tries < 3 {
                break;
            }
            match n.parent {
                Some(p) => cur = p,
                None => return Round::Failed,
            }
        }
    }
    let mut out = Vec::new();
    let mut k = Some(cur);
    while let Some(i) = k {
        out.push(nodes[i].cell);
        k = nodes[i].parent;
    }
    Round::Found(out)
}

impl Gen<'_> {
    /// Act I build.
    pub fn act1(&mut self) -> Result<(), OutdoorError> {
        let id = self.id;
        if !matches!(id, 2 | 3 | 17) {
            self.cliff_marking();
        }
        self.link_flags()?;
        self.borders()?;
        if (2..=7).contains(&id) {
            self.border_sub(BorderCtx::wild(0, 4))?;
            self.river_caves()?;
            self.border_sub(BorderCtx::wild(1, 4))?;
            self.border_sub(BorderCtx::wild(2, 4))?;
            self.transitions()?;
            self.border_sub(BorderCtx::wild(3, 4))?;
            self.dirt_paths()?;
        }
        if id == 39 {
            for t in 0..4 {
                self.border_sub(BorderCtx::wild(t, 4))?;
            }
        }
        if (3..=6).contains(&id) {
            self.waypoint()?;
        }
        if (2..=7).contains(&id) {
            self.shrines(5);
        }
        self.special_presets()
    }

    /// Cliff marking.
    pub fn cliff_marking(&mut self) {
        let n = self.info.vertices.len();
        if n == 0 {
            return;
        }
        let vs = |g: &Self, i: usize| g.info.vertices[i % n];
        let link = |g: &Self, i: usize| vs(g, i).is_link();
        let stop = |g: &Self, w: usize| {
            let (a, b) = (vs(g, w), vs(g, w + 1));
            a.y < b.y || a.x > b.x || a.is_link() || b.is_link()
        };
        let turn = |g: &Self, w: usize| {
            let (a, b, c) = (vs(g, w), vs(g, w + 1), vs(g, w + 2));
            !a.is_link() && !b.is_link() && ((a.x < b.x && b.y < c.y) || (a.y > b.y && b.x < c.x))
        };
        let head = 0usize;
        let mut v = head;
        let mut p = n - 1;
        let mut head_passed = false;
        loop {
            let (cv, cp, cn) = (vs(self, v), vs(self, p), vs(self, v + 1));
            let starts = !link(self, v)
                && !link(self, p)
                && ((cv.x < cn.x && cp.y > cv.y) || (cv.y > cn.y && cp.x > cv.x));
            let mut end = v;
            if starts {
                let mut w = v;
                let mut u = None;
                loop {
                    if w == head {
                        head_passed = true;
                    }
                    if stop(self, w) {
                        end = w;
                        break;
                    }
                    if turn(self, w) {
                        u = Some(w);
                    }
                    w = (w + 1) % n;
                    if w == v {
                        end = w;
                        break;
                    }
                }
                if let Some(u) = u {
                    let mut k = v;
                    loop {
                        self.info.vertices[k].direction = 1;
                        if k == u {
                            break;
                        }
                        k = (k + 1) % n;
                    }
                    self.info.flags |= 0x20;
                }
            }
            p = end;
            v = (p + 1) % n;
            if head_passed || v == head {
                break;
            }
        }
    }

    /// "No row has a direction bit at column x or x + 1".
    fn no_direction_in(&self, x: i32) -> bool {
        (0..self.gh()).all(|y| (self.g(2, x, y) | self.g(2, x + 1, y)) & cell::DIRECTION == 0)
    }

    /// River, cliff caves, side cave.
    pub fn river_caves(&mut self) -> Result<(), OutdoorError> {
        let (gw, gh) = (self.gw(), self.gh());
        if self.info.flags & 0xC != 0 && self.no_direction_in(gw - 2) {
            self.river(gw - 2)?;
        }
        if self.info.flags & 0x20 != 0 && self.info.flags & 0x40 == 0 {
            // Site.
            let bit = self.seed().mask(2);
            let mut found = None;
            'scan: for outer in 0..gh {
                for inner in 0..gw {
                    // Edge case 8: the second order uses (outer, inner) as
                    // (x, y). Cells outside the grid read as 0.
                    let (x, y) = if bit == 0 {
                        (inner, outer)
                    } else {
                        (outer, inner)
                    };
                    match self.g(0, x, y) {
                        16 => found = Some((x, y, 25)),
                        17 => found = Some((x, y, 24)),
                        _ => continue,
                    }
                    break 'scan;
                }
            }
            if let Some((x, y, p)) = found {
                self.stamp(x, y, p, -1, false)?;
                self.info.flags |= 0x40;
            }
        }
        if self.info.flags & 0x1C != 0 && self.info.flags & 0x40 == 0 {
            let mut y = gh - 4;
            let mut x = if self.info.flags & 0x10 != 0 {
                gw - 4
            } else {
                gw - 5
            };
            // Site.
            let r = self.seed().mask(4);
            if r & 1 != 0 {
                x = 3;
            }
            if r >= 2 {
                y = 3;
            }
            let p = if self.id == 2 { 52 } else { 51 };
            self.stamp(x, y, p, -1, false)?;
            self.info.flags |= 0x40;
        }
        Ok(())
    }

    /// Transitions and caves.
    pub fn transitions(&mut self) -> Result<(), OutdoorError> {
        let (gw, gh) = (self.gw(), self.gh());
        let f = self.info.flags;
        if f & 0x10 != 0 {
            let x = gw / 2 - 1;
            if self.no_direction_in(x) {
                self.river(x)?;
            }
        }
        if f & 0x80 != 0 {
            self.stamp(0, 0, 3, 1, false)?;
        }
        if f & 0x100 != 0 {
            self.stamp(gw - 7, 0, 3, 2, false)?;
        }
        if f & 0x200 != 0 {
            self.stamp(0, 1, 2, 1, false)?;
        }
        if f & 0x400 != 0 {
            self.stamp(0, gh - 6, 2, 1, false)?;
        }
        if self.info.flags & 0x40 == 0 {
            if self.id == 2 {
                let town = self
                    .drlg
                    .find_level(1)
                    .map(|l| self.drlg.level(l).rect)
                    .ok_or(OutdoorError::LevelMissing(1))?;
                self.far_away(town, 52, -1, 1, 15)?;
            } else {
                self.spawn_preset(51, -1, 1, 15)?;
            }
            // Not found: a warning only.
            self.info.flags |= 0x40;
        }
        Ok(())
    }

    /// River and bridge at column x.
    pub fn river(&mut self, x: i32) -> Result<(), OutdoorError> {
        let gh = self.gh();
        let file = |g: &Self, cx: i32, cy: i32, upper: bool| {
            let p = g.g(0, cx, cy);
            let f = file_of(g.g(2, cx, cy));
            match p {
                0 if g.g(2, cx, cy) & cell::BLANK != 0 => 0,
                0 => 3,
                7 if f == 3 => 3,
                4..=15 => {
                    let (u, l) = RIVER_FILES[(p - 4) as usize];
                    if upper {
                        u
                    } else {
                        l
                    }
                }
                // TODO: U/L for P outside 0, 4..15 are not
                // given; file 0 used.
                _ => 0,
            }
        };
        for y in 0..gh {
            let f = file(self, x, y, true);
            self.stamp(x, y, 26, f, false)?;
            let f = file(self, x + 1, y, false);
            self.stamp(x + 1, y, 27, f, false)?;
        }
        let flags = self.info.flags;
        if flags & 0x14 != 0 {
            let r_n = gh - 2;
            let r = self.seed().roll(r_n) as i32;
            for i in 0..r_n.max(0) {
                let y = (r + i) % r_n + 1;
                let ok = spawn_valid(self.g(2, x - 1, y))
                    && (flags & 0x4 != 0 || spawn_valid(self.g(2, x + 2, y)))
                    && file_of(self.g(2, x, y)) == 3
                    && file_of(self.g(2, x + 1, y)) == 3;
                if ok {
                    self.stamp(x, y, 28, 1, false)?;
                    self.stamp(x + 1, y, 28, if flags & 0x4 != 0 { 3 } else { 2 }, false)?;
                    break;
                }
            }
        }
        Ok(())
    }

    /// Cottage(P, extra).
    pub fn cottage(&mut self, p: u32, extra: bool) -> Result<(), OutdoorError> {
        // Site.
        if self.seed().mask(4) != 0 {
            self.r(p)?;
            // Site.
            if extra && self.seed().mask(2) != 0 {
                self.r(49)?;
            }
        } else {
            self.r(p)?;
            self.r(p)?;
        }
        Ok(())
    }

    /// Special presets.
    fn special_presets(&mut self) -> Result<(), OutdoorError> {
        match self.id {
            2 => {
                self.r(46)?;
                self.cottage(47, false)?;
                self.s(29)?;
                self.s(30)?;
            }
            3 => {
                self.cottage(48, true)?;
                self.s(44)?;
                self.s(29)?;
                self.s(30)?;
            }
            4 => {
                self.r(160)?;
                self.r(45)?;
                self.s(162)?;
                self.cottage(47, true)?;
                self.cottage(42, false)?;
                self.s(31)?;
            }
            5 => {
                self.s(161)?;
                self.s(41)?;
                self.s(40)?;
                self.cottage(48, true)?;
                self.cottage(43, false)?;
                self.s(29)?;
                self.s(30)?;
            }
            6 => {
                self.s(163)?;
                self.s(38)?;
                self.s(39)?;
                self.cottage(47, true)?;
                self.cottage(42, false)?;
                self.s(29)?;
                self.s(30)?;
            }
            7 => {
                self.cottage(48, true)?;
                self.cottage(43, false)?;
                self.s(31)?;
            }
            17 => self.stamp(1, 1, 108, -1, false)?,
            39 => {
                for p in [50, 46, 31, 38, 39, 29, 30] {
                    self.s(p)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Path starts.
    pub fn path_starts(&self) -> Vec<PathPoint> {
        let mut out = Vec::new();
        for e in &self.info.orth {
            let r = e.rect;
            match e.level_id {
                1 => {
                    let (x, y) = match e.direction {
                        0 => (r.x + 59, r.y + 19),
                        1 => (r.x + 29, r.y + 35),
                        2 => (r.x + 4, r.y + 22),
                        3 => (r.x + 29, r.y + 3),
                        // TODO: direction −1 not described.
                        _ => continue,
                    };
                    out.push(PathPoint {
                        x,
                        y,
                        direction: e.direction,
                    });
                }
                26 => out.push(PathPoint {
                    x: r.x + 27,
                    y: r.y + 13,
                    direction: 1,
                }),
                _ => {}
            }
        }
        let (gw, gh) = (self.gw(), self.gh());
        for x in 0..gw {
            for y in 0..gh {
                let p = self.g(0, x, y);
                let f = file_of(self.g(2, x, y));
                let d = match p {
                    4 if f == 3 => 3,
                    5 if f == 3 => 0,
                    6 if f == 3 => 1,
                    7 if f == 3 => 2,
                    24 => 1,
                    25 => 0,
                    28 if f == 1 && x == gw - 2 => 2,
                    51 | 52 => i32::from(f != 0),
                    _ => continue,
                };
                out.push(PathPoint {
                    x: self.rect.x + 8 * x + 3,
                    y: self.rect.y + 8 * y + 3,
                    direction: d,
                });
            }
        }
        out
    }

    /// Adjusted point.
    pub fn adjust(&self, p: PathPoint) -> PathPoint {
        let (ox, oy) = (self.rect.x, self.rect.y);
        let (mut qx, mut qy) = (p.x - ox, p.y - oy);
        match p.direction {
            0 => qx = 8 * (qx / 8) + 11,
            1 => qy = 8 * (qy / 8) + 11,
            2 => qx = 8 * (qx / 8) - 5,
            3 => qy = 8 * (qy / 8) - 5,
            _ => {}
        }
        PathPoint {
            x: qx + ox,
            y: qy + oy,
            direction: p.direction,
        }
    }

    /// Bridge cell at x = gw/2 − 1: first y in 1..gw−2 (gw, not gh)
    /// with grid 0 = 28 and file 1.
    fn bridge_cell(&self) -> Option<(i32, i32)> {
        let x = self.gw() / 2 - 1;
        (1..self.gw() - 1)
            .find(|&y| self.g(0, x, y) == 28 && file_of(self.g(2, x, y)) == 1)
            .map(|y| (x, y))
    }

    /// Dirt paths.
    pub fn dirt_paths(&mut self) -> Result<(), OutdoorError> {
        let starts = self.path_starts();
        // TODO: the layout keeps 6 path slots; more starts
        // are kept here.
        let count = starts.len() as i32;
        let mut ends: Vec<PathEnds> = starts
            .iter()
            .map(|&s| PathEnds {
                start: s,
                start_adjusted: self.adjust(s),
                ..PathEnds::default()
            })
            .collect();
        // Step 3: join points.
        let bridge = if self.info.flags & 0x10 != 0 {
            self.bridge_cell()
        } else {
            None
        };
        if let Some((x, y)) = bridge {
            let bx = self.rect.x + 8 * x + 3;
            let by = self.rect.y + 8 * y + 3;
            for e in &mut ends {
                e.join = if e.start.x <= bx {
                    PathPoint {
                        x: bx,
                        y: by,
                        direction: 2,
                    }
                } else {
                    PathPoint {
                        x: bx + 8,
                        y: by,
                        direction: 0,
                    }
                };
            }
        } else if count > 0 {
            let (gw, gh) = (self.gw(), self.gh());
            let (cx, cy) = if count == 1 {
                (gw / 2, gh / 2)
            } else {
                let sx: i32 = starts.iter().map(|s| s.x - self.rect.x).sum();
                let sy: i32 = starts.iter().map(|s| s.y - self.rect.y).sum();
                (sx / (8 * count), sy / (8 * count))
            };
            const D: [(i32, i32); 4] = [(-1, 0), (0, 1), (0, -1), (1, 0)];
            let mut last = (cx, cy);
            'find: for r in 0..8 {
                for (dx, dy) in D {
                    let c = (cx + r * dx, cy + r * dy);
                    last = c;
                    if self.in_grid(c.0, c.1) && spawn_valid(self.g(2, c.0, c.1)) {
                        break 'find;
                    }
                }
            }
            let j = PathPoint {
                x: self.rect.x + 8 * last.0 + 3,
                y: self.rect.y + 8 * last.1 + 3,
                direction: 4,
            };
            for e in &mut ends {
                e.join = j;
            }
        }
        for e in &mut ends {
            e.join_adjusted = self.adjust(e.join);
        }
        // Step 4.
        let (gw, gh) = (self.gw(), self.gh());
        let mut paths = Vec::new();
        for e in &ends {
            let a = (
                (e.start_adjusted.x - self.rect.x) / 8,
                (e.start_adjusted.y - self.rect.y) / 8,
            );
            let b = (
                (e.join_adjusted.x - self.rect.x) / 8,
                (e.join_adjusted.y - self.rect.y) / 8,
            );
            let grid2 = &self.info.grids[2];
            let path = grid_path(a, b, gw, gh, |c| grid2.get(c.0, c.1) & cell::PRESET != 0);
            if let Some(p) = &path {
                for &(x, y) in p {
                    self.op(2, x, y, Op::Or, cell::PATH);
                }
            }
            // Edge case 7: the jitter draws even without a path.
            paths.push(self.jitter(e, path.unwrap_or_default()));
        }
        self.info.path_ends = ends;
        self.info.paths = paths;
        Ok(())
    }

    /// Jitter: the path's tile vertices.
    pub fn jitter(&mut self, e: &PathEnds, grid: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
        const XJ: [i32; 4] = [1, 0, -1, 0];
        const YJ: [i32; 4] = [0, 1, 0, -1];
        // Site, always.
        let mut k = self.seed().mask(4) as usize;
        if grid.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        if e.join.direction != 4 {
            out.push((e.join.x, e.join.y));
        }
        let n = grid.len();
        for (i, &(x, y)) in grid.iter().enumerate() {
            if i == 0 {
                out.push((e.join_adjusted.x, e.join_adjusted.y));
            } else if i + 1 < n {
                let ox = (self.seed().mask(2) as i32 + 2) * XJ[k];
                let oy = (self.seed().mask(2) as i32 + 2) * YJ[k];
                k = (k + 1) % 4;
                out.push((8 * x + self.rect.x + ox + 3, 8 * y + self.rect.y + oy + 3));
            } else {
                out.push((e.start_adjusted.x, e.start_adjusted.y));
            }
        }
        out.push((e.start.x, e.start.y));
        out
    }
}

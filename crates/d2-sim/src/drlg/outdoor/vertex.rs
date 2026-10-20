//! The vertex polygon of an outdoor level, the link flags along its edges, and the
//! border pieces stamped around it.
//!
//! `build_polygon` and `to_cells` turn a level rect and its neighbour entries into a
//! closed polygon in 8-tile cells. `link_flags` marks the cells of link edges, and
//! `borders` stamps straight border pieces and corners along every edge. These run
//! before the act-specific generators fill in the interior.

use super::super::TileRect;
use super::grid::{cell, Gen, Op};
use super::{dispatch_act, Orth, OutdoorError, Vertex, VERTEX_LINK, VERTEX_PRESET_LINK};

/// Builds the polygon of a level rect and its neighbour entries.
///
/// Coordinates are relative to the level origin, in tiles. Fails on an unknown
/// neighbour direction.
pub fn build_polygon(rect: TileRect, orth: &[Orth]) -> Result<Vec<Vertex>, OutdoorError> {
    let r = TileRect::new(rect.x, rect.y, rect.w - 1, rect.h - 1);
    let v = |x, y| Vertex {
        x,
        y,
        direction: 0,
        flags: 0,
    };
    // Arena with next links; corners are nodes 0..3.
    let mut nodes = vec![
        (v(r.x, r.y + r.h), 1usize),
        (v(r.x, r.y), 2),
        (v(r.x + r.w, r.y), 3),
        (v(r.x + r.w, r.y + r.h), 0),
    ];
    let insert_after = |nodes: &mut Vec<(Vertex, usize)>, at: usize, vx: Vertex| {
        let next = nodes[at].1;
        nodes.push((vx, next));
        let id = nodes.len() - 1;
        nodes[at].1 = id;
        id
    };
    for e in orth {
        let b = TileRect::new(e.rect.x, e.rect.y, e.rect.w - 1, e.rect.h - 1);
        let (c, on_x, s, p, q, a, bb) = match e.direction {
            0 => (
                0usize,
                false,
                -1,
                b.y + b.h,
                b.y,
                nodes[0].0.y,
                nodes[1].0.y,
            ),
            1 => (1, true, 1, b.x, b.x + b.w, nodes[1].0.x, nodes[2].0.x),
            2 => (2, false, 1, b.y, b.y + b.h, nodes[2].0.y, nodes[3].0.y),
            3 => (3, true, -1, b.x + b.w, b.x, nodes[3].0.x, nodes[0].0.x),
            d => return Err(OutdoorError::UnknownDirection(d)),
        };
        let lf = VERTEX_LINK | if e.preset { VERTEX_PRESET_LINK } else { 0 };
        let corner = nodes[c].0;
        let at = |t: i32| {
            if on_x {
                v(t, corner.y)
            } else {
                v(corner.x, t)
            }
        };
        if s * p > s * a {
            if s * p <= s * bb {
                let mut u = at(p);
                u.flags |= lf;
                let u = insert_after(&mut nodes, c, u);
                if s * q < s * bb {
                    insert_after(&mut nodes, u, at(q));
                }
            }
        } else if s * q >= s * a {
            nodes[c].0.flags |= lf;
            if s * q < s * bb {
                insert_after(&mut nodes, c, at(q));
            }
        }
    }
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        let mut vx = nodes[i].0;
        vx.x -= rect.x;
        vx.y -= rect.y;
        out.push(vx);
        i = nodes[i].1;
        if i == 0 {
            break;
        }
    }
    Ok(out)
}

/// Converts polygon vertices to cell coordinates (divide by 8) and merges consecutive
/// equal vertices.
///
/// The earlier vertex keeps its place, ORs in the later one's flags and takes its
/// direction.
pub fn to_cells(vs: &mut Vec<Vertex>) {
    for v in vs.iter_mut() {
        v.x /= 8;
        v.y /= 8;
    }
    // Only neighbours in list order are merged; the closing pair (last, head) of the
    // circular list is not.
    let mut i = 0;
    while i + 1 < vs.len() {
        if vs[i].x == vs[i + 1].x && vs[i].y == vs[i + 1].y {
            let later = vs.remove(i + 1);
            vs[i].flags |= later.flags;
            vs[i].direction = later.direction;
        } else {
            i += 1;
        }
    }
}

/// Border lookup table N: maps a direction-pair index in 0..91 to a piece slot, or −1.
pub fn n_table(i: i32) -> i32 {
    const N: [(i32, i32); 37] = [
        (1, 1),
        (3, 0),
        (5, 2),
        (7, 3),
        (9, 0),
        (10, 1),
        (11, 9),
        (12, 9),
        (15, 1),
        (16, 8),
        (19, 12),
        (24, 12),
        (25, 4),
        (28, 5),
        (29, 2),
        (30, 2),
        (31, 10),
        (36, 10),
        (37, 1),
        (38, 9),
        (39, 9),
        (61, 11),
        (62, 11),
        (63, 3),
        (64, 12),
        (69, 12),
        (70, 4),
        (71, 4),
        (72, 7),
        (75, 2),
        (76, 10),
        (81, 10),
        (84, 6),
        (85, 3),
        (88, 11),
        (89, 11),
        (90, 3),
    ];
    N.iter().find(|&&(k, _)| k == i).map_or(-1, |&(_, v)| v)
}

/// Border table P: rows 0..12 × [cliff, wild, desert, mesa] of preset ids.
///
/// Row 0 holds values unrelated to borders; it is reached only by the edge case where
/// `border_piece` gets slot −1.
pub const P: [[u32; 4]; 13] = [
    [8192, 16384, 32768, 0],
    [0, 4, 364, 799],
    [16, 5, 365, 800],
    [17, 6, 366, 801],
    [0, 7, 367, 802],
    [18, 8, 368, 803],
    [19, 9, 369, 804],
    [22, 10, 370, 805],
    [0, 11, 371, 806],
    [0, 12, 372, 807],
    [23, 13, 373, 808],
    [0, 14, 374, 809],
    [0, 15, 375, 810],
];

/// Border table Q, rows 0..11 × [barricade, snow]: returns the preset id for row `k`
/// and column `col`, or 0 when `k` is outside 0..12.
pub fn q_table(k: i32, col: i32) -> u32 {
    if !(0..12).contains(&k) {
        // Rows outside 0..11 (corner slot 0, border slot −1) have no entry in Q and give
        // no piece. The edge shapes in the game data do not reach them.
        return 0;
    }
    if col == 0 {
        881 + k as u32
    } else {
        957 + k as u32
    }
}

/// Preset id of the straight border piece for edge direction `(dx, dy)` and style `s`.
///
/// Returns 0 when there is no piece.
pub fn border_piece(dx: i32, dy: i32, s: i32) -> u32 {
    let k = n_table(dx + 3 * dy + 4);
    if s < 0 {
        // Style −1 has no column; borders are only requested for the level types that
        // have a style, so this gives no piece.
        return 0;
    }
    if s < 4 {
        P.get((k + 1) as usize).map_or(0, |r| r[s as usize])
    } else {
        q_table(k, s - 4)
    }
}

/// Preset id of the corner piece between an incoming edge `(a, b)` and an outgoing edge
/// `(c, e)` for style `s`.
///
/// Returns 0 when there is no piece.
pub fn corner_piece(a: i32, b: i32, c: i32, e: i32, s: i32) -> u32 {
    let grow = |v: i32| v + 2 * v.signum();
    let k = n_table(b + grow(a) + 9 * (e + grow(c)) + 50);
    if k == -1 || s < 0 {
        return 0;
    }
    if s < 4 {
        P[k as usize][s as usize]
    } else {
        q_table(k - 1, s - 4)
    }
}

/// Border style of a level type and vertex direction, or −1 for an unstyled level type.
pub fn style(level_type: u32, id: u32, d: u8) -> i32 {
    match level_type {
        2 => i32::from(d == 0),
        16 => 2,
        27 => 3,
        31 => 4 + i32::from(id == 117),
        _ => -1,
    }
}

impl Gen<'_> {
    /// Builds the level's polygon from its neighbour entries, then converts it to cells.
    pub fn polygon(&mut self) -> Result<(), OutdoorError> {
        let mut vs = build_polygon(self.rect, &self.info.orth)?;
        to_cells(&mut vs);
        self.info.vertices = vs;
        Ok(())
    }

    fn next_vertex(&self, i: usize) -> Vertex {
        let n = self.info.vertices.len();
        self.info.vertices[(i + 1) % n]
    }

    /// Returns the vis flag for a link vertex: bit `j + 4` when the neighbour level
    /// across the edge is slot `j` of the level's vis array.
    ///
    /// Returns 0 for non-edge vertices, uncovered edges, initial entries, and neighbours
    /// not in the vis array.
    pub fn link_vis(&self, v: Vertex) -> Result<u32, OutdoorError> {
        let (gw, gh) = (self.gw(), self.gh());
        let s = if v.x == 0 {
            if v.y == 0 {
                1
            } else {
                0
            }
        } else if v.y == 0 {
            if v.x == gw - 1 {
                2
            } else {
                1
            }
        } else if v.x == gw - 1 {
            if v.y == gh - 1 {
                3
            } else {
                2
            }
        } else if v.y == gh - 1 {
            3
        } else {
            return Ok(0);
        };
        const D: [(i32, i32); 4] = [(-4, 4), (4, -4), (12, 4), (4, 12)];
        let px = self.rect.x + 8 * v.x + D[s as usize].0;
        let py = self.rect.y + 8 * v.y + D[s as usize].1;
        let Some(e) = self
            .info
            .orth
            .iter()
            .find(|e| e.direction == s && e.rect.contains(px, py))
        else {
            return Ok(0);
        };
        if e.init {
            return Ok(0);
        }
        let vis = self.drlg.vis_array(self.data, self.id)?;
        // A neighbour that is in no vis slot gives flag 0.
        Ok(vis
            .iter()
            .position(|&x| x == e.level_id)
            .map_or(0, |j| 1u32 << (j + 4)))
    }

    /// Calls `f` for each cell from `a` to `b`, both ends included, stepping diagonally
    /// until one axis lines up.
    fn walk_cells(&mut self, a: (i32, i32), b: (i32, i32), mut f: impl FnMut(&mut Self, i32, i32)) {
        let (sx, sy) = ((b.0 - a.0).signum(), (b.1 - a.1).signum());
        let (mut x, mut y) = a;
        loop {
            f(self, x, y);
            if (x, y) == b {
                break;
            }
            if x != b.0 {
                x += sx;
            }
            if y != b.1 {
                y += sy;
            }
        }
    }

    /// Marks the cells along every link edge with the vis flag and border bits.
    pub fn link_flags(&mut self) -> Result<(), OutdoorError> {
        for i in 0..self.info.vertices.len() {
            let v = self.info.vertices[i];
            if !v.is_link() {
                continue;
            }
            let n = self.next_vertex(i);
            let vis = self.link_vis(v)?;
            let bits = cell::BORDER | if v.direction != 0 { cell::DIRECTION } else { 0 };
            self.walk_cells((v.x, v.y), (n.x, n.y), |g, x, y| {
                g.op(1, x, y, Op::Or, vis);
                g.op(2, x, y, Op::Or, bits);
            });
        }
        Ok(())
    }

    /// Stamps border pieces along every polygon edge and a corner piece at each vertex,
    /// then blanks the corners outside the border.
    pub fn borders(&mut self) -> Result<(), OutdoorError> {
        let lt = self.drlg.level(self.level).level_type;
        let act = dispatch_act(self.id);
        let count = self.info.vertices.len();
        for i in 0..count {
            let v = self.info.vertices[i];
            let n = self.next_vertex(i);
            let nn = self.next_vertex((i + 1) % count);
            let (dx, dy) = ((n.x - v.x).signum(), (n.y - v.y).signum());
            let (ndx, ndy) = ((nn.x - n.x).signum(), (nn.y - n.y).signum());
            // Pick the style and straight piece for this edge.
            let s = style(lt, self.id, v.direction);
            let straight = border_piece(dx, dy, s);
            let bits = cell::BORDER | if v.direction != 0 { cell::DIRECTION } else { 0 };
            // Stamp the straight piece on every cell of the edge after the start. A zero
            // piece (cliff style on W/S edges) is not stamped, and the stamp leaves the
            // border flag clear. Preset links get no straight pieces.
            if !v.is_preset_link() {
                let (mut x, mut y) = (v.x, v.y);
                while (x, y) != (n.x, n.y) {
                    x += dx;
                    y += dy;
                    if straight != 0 {
                        self.stamp(x, y, straight, -1, false)?;
                    }
                    self.op(2, x, y, Op::Or, bits);
                }
            }
            // Link edges get a marker at the midpoint of the edge.
            if v.is_link() && !v.is_preset_link() {
                let len = (n.x - v.x).abs() + (n.y - v.y).abs();
                let mx = v.x.min(n.x) + dx.abs() * len / 2;
                let my = v.y.min(n.y) + dy.abs() * len / 2;
                match act {
                    0 | 3 | 4 => {
                        let f = if act != 3 && self.id == 17 {
                            0x40400
                        } else {
                            0x30400
                        };
                        self.op(2, mx, my, Op::AndNot, cell::FILE_MASK);
                        self.op(2, mx, my, Op::Or, f);
                    }
                    1 => {
                        const PAIRS: [(u32, u32); 5] =
                            [(373, 372), (372, 375), (0, 0), (373, 374), (374, 375)];
                        let (a, b) = PAIRS[(dx + 2 * dy + 2) as usize];
                        // The pair (0, 0), for no direction, stamps nothing.
                        if a != 0 {
                            self.stamp(mx, my, a, -1, false)?;
                        }
                        if b != 0 {
                            self.stamp(mx + dx, my + dy, b, -1, false)?;
                        }
                    }
                    _ => {}
                }
            }
            // Corner piece at the end vertex `n`.
            let d = if v.direction != 0 {
                v.direction
            } else {
                n.direction
            };
            let s2 = style(lt, self.id, d);
            let dbl = |x: i32, keep: bool| if keep { x } else { 2 * x };
            let mut piece = corner_piece(
                dbl(dx, v.is_preset_link()),
                dbl(dy, v.is_preset_link()),
                dbl(ndx, n.is_preset_link()),
                dbl(ndy, n.is_preset_link()),
                s2,
            );
            if piece == 19 {
                if v.direction == 1 && n.direction != 1 {
                    piece = 20;
                } else if v.direction != 1 {
                    piece = 21;
                }
            }
            if piece != 0 {
                self.stamp(n.x, n.y, piece, -1, false)?;
                let bits = cell::BORDER | if d != 0 { cell::DIRECTION } else { 0 };
                self.op(2, n.x, n.y, Op::Or, bits);
            }
        }
        // Blank the cells in the corners outside the border.
        self.blank_corners();
        Ok(())
    }

    /// Flags as blank every cell reachable from each grid corner without crossing a
    /// border cell.
    pub fn blank_corners(&mut self) {
        let (gw, gh) = (self.gw(), self.gh());
        let corners = [
            (0, 0, 1, 1),
            (gw - 1, 0, -1, 1),
            (0, gh - 1, 1, -1),
            (gw - 1, gh - 1, -1, -1),
        ];
        let border = |g: &Self, x, y| g.g(2, x, y) & cell::BORDER != 0;
        for (cx, cy, sx, sy) in corners {
            // Row pass: scan row by row from the corner, stopping at a border cell.
            let mut y = cy;
            while self.in_grid(cx, y) && !border(self, cx, y) {
                let mut x = cx;
                while self.in_grid(x, y) && !border(self, x, y) {
                    self.op(2, x, y, Op::Or, cell::BLANK);
                    x += sx;
                }
                y += sy;
            }
            // Column pass: the row pass transposed. It stops at the first column whose
            // start cell has the border bit.
            let mut x = cx;
            while self.in_grid(x, cy) && !border(self, x, cy) {
                let mut y = cy;
                while self.in_grid(x, y) && !border(self, x, y) {
                    self.op(2, x, y, Op::Or, cell::BLANK);
                    y += sy;
                }
                x += sx;
            }
        }
    }
}

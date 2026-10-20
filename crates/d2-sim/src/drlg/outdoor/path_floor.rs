//! Act I path floor per room: the level's jittered path vertices drawn two cells
//! thick on a (w + 3) × (h + 3) grid over the room (origin: room corner − 1), then
//! a floor style chosen by each cell's 8-neighbour mask (`drlg/outdoor-path-floor.tsv`).
//! Draws no random numbers. `rooms.rs` calls it while building a room's grids.

use super::super::tiles::CellGrid;
use super::super::TileRect;

/// Floor style indexed by the 8-neighbour mask of a path cell (`drlg/outdoor-path-floor.tsv`).
pub const PATH_FLOOR_STYLE: [u8; 256] = [
    0, 0, 16, 16, 0, 0, 16, 16, 14, 14, 6, 19, 14, 14, 6, 19, 15, 15, 5, 5, 15, 15, 21, 21, 8, 8,
    10, 38, 8, 8, 40, 20, 0, 0, 16, 16, 0, 0, 16, 16, 14, 14, 6, 19, 14, 14, 6, 19, 15, 15, 5, 5,
    15, 15, 21, 21, 8, 8, 10, 38, 8, 8, 40, 20, 13, 13, 7, 7, 13, 13, 13, 7, 4, 4, 11, 37, 4, 4,
    11, 43, 3, 3, 12, 12, 3, 3, 39, 39, 9, 9, 2, 43, 9, 9, 44, 26, 13, 13, 7, 7, 13, 13, 13, 7, 23,
    23, 41, 17, 23, 23, 41, 17, 3, 3, 12, 12, 3, 3, 39, 39, 42, 42, 46, 42, 42, 42, 33, 31, 0, 0,
    16, 16, 0, 0, 16, 16, 14, 14, 6, 19, 14, 14, 6, 19, 15, 15, 5, 5, 15, 15, 21, 21, 8, 8, 10, 38,
    8, 8, 35, 20, 0, 0, 16, 16, 0, 0, 16, 16, 14, 14, 6, 19, 14, 14, 6, 19, 15, 15, 5, 5, 15, 15,
    21, 21, 8, 8, 10, 38, 8, 8, 40, 20, 13, 13, 7, 7, 13, 13, 13, 7, 4, 4, 11, 37, 4, 4, 11, 37,
    18, 18, 35, 35, 18, 18, 22, 22, 36, 36, 45, 34, 36, 36, 28, 29, 13, 13, 7, 7, 13, 13, 13, 7,
    23, 23, 41, 17, 23, 23, 41, 17, 18, 18, 35, 35, 18, 18, 22, 22, 24, 24, 25, 32, 24, 24, 30, 1,
];

/// The path grid of one room: (w + 3) × (h + 3) cells, origin (room x − 1, room y − 1).
struct PathGrid {
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
    cells: Vec<bool>,
}

impl PathGrid {
    fn new(rect: TileRect) -> Self {
        let (w, h) = (rect.w + 3, rect.h + 3);
        PathGrid {
            x0: rect.x - 1,
            y0: rect.y - 1,
            w,
            h,
            cells: vec![false; (w * h) as usize],
        }
    }

    fn set(&mut self, x: i32, y: i32) {
        if (0..self.w).contains(&x) && (0..self.h).contains(&y) {
            self.cells[(y * self.w + x) as usize] = true;
        }
    }

    fn get(&self, x: i32, y: i32) -> bool {
        (0..self.w).contains(&x)
            && (0..self.h).contains(&y)
            && self.cells[(y * self.w + x) as usize]
    }

    /// Draws one segment two cells thick, in path-grid cells.
    ///
    /// Every point of the major axis from `a` to `b` is set, both ends included; the
    /// minor coordinate moves one cell when the accumulated error strictly exceeds the
    /// major length; the second cell is +1 on the minor axis.
    fn segment(&mut self, a: (i32, i32), b: (i32, i32)) {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let (sx, sy) = (dx.signum(), dy.signum());
        let (ax, ay) = (dx.abs(), dy.abs());
        // A tie |dx| = |dy| is drawn x-major: the y-major branch is taken only when
        // |dx| < |dy| (`a_diagonal_segment_is_drawn_x_major` pins this).
        if ay > ax {
            for n in 0..=ay {
                let x = a.0 + sx * minor(n, ax, ay);
                let y = a.1 + sy * n;
                self.set(x, y);
                self.set(x + 1, y);
            }
        } else {
            for n in 0..=ax {
                let x = a.0 + sx * n;
                let y = a.1 + sy * minor(n, ay, ax);
                self.set(x, y);
                self.set(x, y + 1);
            }
        }
    }
}

/// Minor-axis offset after `n` major steps: the error n·minor exceeds
/// the major length `k` times, k = (n·minor − 1) / major (0 at n = 0).
fn minor(n: i32, minor: i32, major: i32) -> i32 {
    if n == 0 || major == 0 {
        0
    } else {
        (n * minor - 1) / major
    }
}

/// Writes the path floor of the room at `rect` (tiles) into `floor` ((w + 1) × (h + 1) cells).
///
/// Every segment between consecutive vertices of each path (tile coordinates) is drawn
/// first. Then, for path-grid columns X = 1..=w + 1 and rows Y = h + 1 down to 1, a set
/// cell gets its 8-neighbour mask and, when the style s is not 0, floor cell
/// (X − 1, Y − 1) becomes `(s << 8) | 0x82`.
pub fn path_floor(paths: &[Vec<(i32, i32)>], rect: TileRect, floor: &mut CellGrid) {
    let mut g = PathGrid::new(rect);
    for p in paths {
        for s in p.windows(2) {
            g.segment(
                (s[0].0 - g.x0, s[0].1 - g.y0),
                (s[1].0 - g.x0, s[1].1 - g.y0),
            );
        }
    }
    for x in 1..=rect.w + 1 {
        for y in (1..=rect.h + 1).rev() {
            if !g.get(x, y) {
                continue;
            }
            let n = [
                (x + 1, y - 1),
                (x + 1, y),
                (x + 1, y + 1),
                (x, y - 1),
                (x, y + 1),
                (x - 1, y - 1),
                (x - 1, y),
                (x - 1, y + 1),
            ];
            let mask = n
                .iter()
                .fold(0usize, |m, &(cx, cy)| (m << 1) | g.get(cx, cy) as usize);
            let s = PATH_FLOOR_STYLE[mask] as u32;
            if s != 0 {
                floor.set((x - 1) as usize, (y - 1) as usize, (s << 8) | 0x82);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(a: (i32, i32), b: (i32, i32)) -> Vec<(i32, i32)> {
        let mut g = PathGrid::new(TileRect::new(1, 1, 8, 8));
        g.segment(a, b);
        let mut out = Vec::new();
        for y in 0..g.h {
            for x in 0..g.w {
                if g.get(x, y) {
                    out.push((x, y));
                }
            }
        }
        out.sort();
        out
    }

    /// Segments from a Blood Moor room (1032, 848), in path-grid cells: y-major with the
    /// error rule, the vertical and horizontal cases, both ends included, the second
    /// cell +1 on the minor axis.
    #[test]
    fn segments_follow_the_recorded_cells() {
        // (1, 12) -> (4, 2): x steps at y 8 and 5, not at the end (y 2).
        let got = cells((1, 12), (4, 2));
        assert_eq!(
            got,
            vec![
                (1, 9),
                (1, 10),
                (2, 6),
                (2, 7),
                (2, 8),
                (2, 9),
                (2, 10),
                (3, 2),
                (3, 3),
                (3, 4),
                (3, 5),
                (3, 6),
                (3, 7),
                (3, 8),
                (4, 2),
                (4, 3),
                (4, 4),
                (4, 5),
            ]
        );
        // (1, 20) -> (4, 10): only y = 10 is in the grid, at x 3.
        assert_eq!(cells((1, 20), (4, 10)), vec![(3, 10), (4, 10)]);
        // (4, 10) -> (4, 4): x 4 and 5.
        let v = cells((4, 10), (4, 4));
        assert_eq!(v.len(), 14);
        assert!(v
            .iter()
            .all(|&(x, y)| (4..=5).contains(&x) && (4..=10).contains(&y)));
        // (4, 4) -> (-4, 4): rows 4 and 5, x 0..=4.
        let v = cells((4, 4), (-4, 4));
        assert_eq!(v.len(), 10);
        assert!(v
            .iter()
            .all(|&(x, y)| (0..=4).contains(&x) && (4..=5).contains(&y)));
    }

    /// A tie |dx| = |dy| is x-major: the second cell is (x, y + 1) and x advances every
    /// step, y by the error rule ((n·3 − 1) / 3: 0, 0, 1, 2).
    #[test]
    fn a_diagonal_segment_is_drawn_x_major() {
        assert_eq!(
            cells((1, 1), (4, 4)),
            vec![
                (1, 1),
                (1, 2),
                (2, 1),
                (2, 2),
                (3, 2),
                (3, 3),
                (4, 3),
                (4, 4)
            ]
        );
    }

    /// The floor pass: a cell with set neighbours gets `(style << 8) | 0x82` at
    /// (X − 1, Y − 1); isolated cells (mask 0) and style-0 masks keep the floor.
    #[test]
    fn floor_cells_take_the_style_of_their_neighbour_mask() {
        let rect = TileRect::new(100, 200, 8, 8);
        let mut floor = CellGrid::new(9, 9);
        floor.cells.fill(0x40002);
        // A vertical path through tile x 103 (path-grid X 4), y 200..=208.
        path_floor(&[vec![(103, 199), (103, 210)]], rect, &mut floor);
        // Column X 4 (floor x 3): neighbours east (X 5) above, at, below
        // and north / south: mask b7 b6 b5 b4 b3 = 0xF8 -> style.
        let s = PATH_FLOOR_STYLE[0xF8] as u32;
        assert_ne!(s, 0);
        assert_eq!(floor.get(3, 4), (s << 8) | 0x82);
        // Column X 5 (floor x 4): west neighbours and north / south: 0x1F.
        let s = PATH_FLOOR_STYLE[0x1F] as u32;
        assert_eq!(
            floor.get(4, 4),
            if s != 0 { (s << 8) | 0x82 } else { 0x40002 }
        );
        assert_eq!(floor.get(0, 0), 0x40002);
        assert_eq!(floor.get(8, 8), 0x40002);
    }

    #[test]
    fn the_style_table_has_240_non_zero_entries() {
        assert_eq!(PATH_FLOOR_STYLE.iter().filter(|&&s| s != 0).count(), 240);
        assert!(PATH_FLOOR_STYLE.iter().all(|&s| s <= 46));
    }

    /// `PATH_FLOOR_STYLE` equals `drlg/outdoor-path-floor.tsv` row for row.
    #[test]
    fn the_style_table_is_the_spec_tsv() {
        let tsv = include_str!("../../../../../tables/drlg/outdoor-path-floor.tsv");
        let mut lines = tsv.lines();
        assert_eq!(lines.next(), Some("mask\tstyle"));
        let rows: Vec<(usize, u8)> = lines
            .map(|l| {
                let (m, s) = l.split_once('\t').expect("two columns");
                (m.parse().expect("mask"), s.parse().expect("style"))
            })
            .collect();
        assert_eq!(rows.len(), 256);
        for (i, (m, s)) in rows.into_iter().enumerate() {
            assert_eq!((m, s), (i, PATH_FLOOR_STYLE[i]), "mask {i}");
        }
    }
}

//! Free-point searches: the nearest free point and its
//! wrappers, the walk-back field `ExpField.D2` and the coarse
//! free-box search. No function here draws; the ring
//! and scan orders decide which free cell wins and are followed exactly.

use super::coords::Point;
use super::place_seams::{CollisionView, PlaceError};
use crate::drlg::TileRect;

/// Max distance of the wrappers.
pub const FREE_MAX_DISTANCE: i32 = 50;
/// Passes of the coarse search.
pub const COARSE_PASSES: i32 = 49;

/// `field_dx` (path tables): x offset of a field
/// direction 0..8.
pub const FIELD_DX: [i32; 9] = [0, 1, 1, 1, 0, -1, -1, -1, 0];
/// `field_dy`.
pub const FIELD_DY: [i32; 9] = [-1, -1, 0, 1, 1, 1, 0, -1, 0];
/// The field byte of the centre.
pub const FIELD_CENTRE: u8 = 8;
/// Fixed centre offset and row stride of the field readers (field rules
/// 1, 3).
const FIELD_HALF: i32 = 128;
const FIELD_STRIDE: i32 = 256;
/// Walk length guard: the shipped field reaches the centre from every cell
/// (field rule 2), so no walk is longer than the cell count.
const FIELD_MAX_STEPS: usize = 256 * 256;

/// The walk-back direction field `data\global\ExpField.D2`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpField {
    /// u16 at +0 (0x010A in the shipped file, unused).
    pub version: u16,
    pub height: u32,
    pub width: u32,
    cells: Vec<u8>,
}

/// Errors reading `ExpField.D2`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FieldError {
    #[error("ExpField.D2 shorter than its 10-byte header")]
    Header,
    #[error("ExpField.D2 is {got} bytes, header says {want}")]
    Size { got: usize, want: u64 },
}

impl ExpField {
    /// Archive path of the field.
    pub const PATH: &'static str = "data\\global\\ExpField.D2";

    /// Parses the file: u16 at +0, u32 height at +2, u32
    /// width at +6, then height × width bytes row-major. Strict: the size
    /// must be exactly header + cells.
    pub fn from_bytes(bytes: &[u8]) -> Result<ExpField, FieldError> {
        if bytes.len() < 10 {
            return Err(FieldError::Header);
        }
        let version = u16::from_le_bytes([bytes[0], bytes[1]]);
        let height = u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]);
        let width = u32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]);
        let want = 10 + u64::from(height) * u64::from(width);
        if bytes.len() as u64 != want {
            return Err(FieldError::Size {
                got: bytes.len(),
                want,
            });
        }
        Ok(ExpField {
            version,
            height,
            width,
            cells: bytes[10..].to_vec(),
        })
    }

    /// Builds a field from its parts (height × width bytes, row-major).
    pub fn from_cells(height: u32, width: u32, cells: Vec<u8>) -> Option<ExpField> {
        (cells.len() as u64 == u64::from(height) * u64::from(width)).then_some(ExpField {
            version: 0,
            height,
            width,
            cells,
        })
    }

    /// The byte the readers see at field cell (fx, fy): fixed row stride
    /// 256. `None` outside the field.
    pub fn byte(&self, fx: i32, fy: i32) -> Option<u8> {
        if !(0..FIELD_STRIDE).contains(&fx) || fy < 0 {
            return None;
        }
        let i = usize::try_from(fy * FIELD_STRIDE + fx).ok()?;
        self.cells.get(i).copied()
    }

    /// Byte for a world cell (x, y) of a walk toward `origin`: the cell
    /// (x − ox + 128, y − oy + 128).
    fn byte_at(&self, x: i32, y: i32, origin: Point) -> Result<u8, PlaceError> {
        self.byte(x - origin.x + FIELD_HALF, y - origin.y + FIELD_HALF)
            .ok_or(PlaceError::FieldOutOfRange { x, y })
    }
}

/// Walk-back test.
/// Every point test uses `room` (edge case 4).
pub fn walk_back<C: CollisionView>(
    cv: &C,
    field: &ExpField,
    room: Option<C::Room>,
    x: i32,
    y: i32,
    origin: Point,
    fmask: u32,
) -> Result<bool, PlaceError> {
    let Some(room) = room else {
        return Ok(true);
    };
    if cv.point_query(room, x, y, fmask) != 0 {
        return Ok(false);
    }
    let (mut x, mut y) = (x, y);
    for _ in 0..FIELD_MAX_STEPS {
        let d = field.byte_at(x, y, origin)?;
        let d = usize::from(d);
        if d > 8 {
            return Err(PlaceError::FieldDirection(d as u8));
        }
        x += FIELD_DX[d];
        y += FIELD_DY[d];
        if field.byte_at(x, y, origin)? == FIELD_CENTRE {
            return Ok(true);
        }
        if cv.point_query(room, x, y, fmask) != 0 {
            return Ok(false);
        }
    }
    Err(PlaceError::FieldNoEnd)
}

/// The walk-back part of a search (the field arguments).
#[derive(Clone, Copy, Debug)]
pub struct FieldTest<'a> {
    pub field: &'a ExpField,
    /// The field origin: the point the candidate must walk back to.
    pub origin: Point,
    /// The field mask (0x801 for floor drops).
    pub mask: u32,
}

/// Arguments of the nearest-free search besides the room and point.
#[derive(Clone, Copy, Debug)]
pub struct FreeSearch<'a> {
    pub size: i32,
    pub mask: u32,
    pub field: Option<FieldTest<'a>>,
    pub fallback: bool,
    /// D.
    pub max_distance: i32,
    /// k.
    pub step: i32,
}

/// Nearest free point. `point` is updated in place;
/// the result is a room or none.
pub fn nearest_free_point<C: CollisionView>(
    cv: &C,
    room: Option<C::Room>,
    point: &mut Point,
    args: &FreeSearch<'_>,
) -> Result<Option<C::Room>, PlaceError> {
    let (x0, y0) = (point.x, point.y);
    let mut hint = room;

    // Rule 1: lookup from the hint (a found room becomes the hint), the
    // size query, then the walk-back test.
    let free = |hint: &mut Option<C::Room>, x: i32, y: i32| -> Result<bool, PlaceError> {
        let Some(r) = cv.cell_room(*hint, x, y) else {
            return Ok(false);
        };
        *hint = Some(r);
        if cv.size_query(r, x, y, args.size, args.mask) != 0 {
            return Ok(false);
        }
        match args.field {
            // The candidate cell's room (the room rule 1's
            // lookup just found, also the new hint; nearest-free search →
            // room lookup).
            Some(f) => walk_back(cv, f.field, Some(r), x, y, f.origin, f.mask),
            None => Ok(true),
        }
    };

    // Rule 2.
    if free(&mut hint, x0, y0)? {
        return Ok(cv.cell_room(hint, x0, y0));
    }

    // Rule 3.
    let k = args.step;
    let mut kept: Option<(Point, i32)> = None;
    if args.max_distance > 1 {
        let mut r: i32 = 1;
        loop {
            let off = (r - 1) * k;
            let (l, rr) = (x0 - off, x0 + off);
            let (t, b) = (y0 - 1 - off, y0 + 1 + off);
            let s = 2 + 2 * off;
            let consider = |hint: &mut Option<C::Room>,
                            kept: &mut Option<(Point, i32)>,
                            x: i32,
                            y: i32|
             -> Result<(), PlaceError> {
                if free(hint, x, y)? {
                    let d = (x - x0).abs() + (y - y0).abs();
                    if kept.is_none_or(|(_, kd)| d < kd) {
                        *kept = Some((Point::new(x, y), d));
                    }
                }
                Ok(())
            };
            // 3.1 the two side columns, top to bottom, left before right.
            let mut y = t;
            while y <= b {
                for x in [l - 1, l - 1 + s] {
                    consider(&mut hint, &mut kept, x, y)?;
                }
                y += k;
            }
            // 3.2 the top and bottom rows, left to right, top first.
            let mut x = l;
            while x <= rr {
                for y in [t, t + s] {
                    consider(&mut hint, &mut kept, x, y)?;
                }
                x += k;
            }
            // 3.4
            if kept.is_some() || r * k + 1 >= args.max_distance {
                break;
            }
            r += 1;
        }
    }

    // Rule 4.
    match kept {
        Some((p, _)) => {
            *point = p;
            Ok(cv.cell_room(hint, p.x, p.y))
        }
        None if args.fallback => Ok(cv.cell_room(hint, x0, y0)),
        None => Ok(None),
    }
}

/// No field, max distance
/// 50, step 1.
pub fn free_point<C: CollisionView>(
    cv: &C,
    room: Option<C::Room>,
    point: &mut Point,
    size: i32,
    mask: u32,
    fallback: bool,
) -> Result<Option<C::Room>, PlaceError> {
    let args = FreeSearch {
        size,
        mask,
        field: None,
        fallback,
        max_distance: FREE_MAX_DISTANCE,
        step: 1,
    };
    nearest_free_point(cv, room, point, &args)
}

/// No field, max distance 50,
/// no fallback.
pub fn free_point_step<C: CollisionView>(
    cv: &C,
    room: Option<C::Room>,
    point: &mut Point,
    size: i32,
    mask: u32,
    step: i32,
) -> Result<Option<C::Room>, PlaceError> {
    let args = FreeSearch {
        size,
        mask,
        field: None,
        fallback: false,
        max_distance: FREE_MAX_DISTANCE,
        step,
    };
    nearest_free_point(cv, room, point, &args)
}

/// With
/// the walk-back field, max distance 50, step 1.
#[allow(clippy::too_many_arguments)]
pub fn free_point_field<C: CollisionView>(
    cv: &C,
    field: &ExpField,
    room: Option<C::Room>,
    point: &mut Point,
    origin: Point,
    size: i32,
    mask: u32,
    fmask: u32,
    fallback: bool,
) -> Result<Option<C::Room>, PlaceError> {
    let args = FreeSearch {
        size,
        mask,
        field: Some(FieldTest {
            field,
            origin,
            mask: fmask,
        }),
        fallback,
        max_distance: FREE_MAX_DISTANCE,
        step: 1,
    };
    nearest_free_point(cv, room, point, &args)
}

/// `y` inside the rect's rows: `y0 ≤ y < y0 + h` (half-open, as the
/// containment test).
fn has_row(r: &TileRect, y: i32) -> bool {
    r.y <= y && y < r.y + r.h
}

/// `x` inside the rect's columns (half-open).
fn has_column(r: &TileRect, x: i32) -> bool {
    r.x <= x && x < r.x + r.w
}

/// Coarse free-box search.
/// `point` holds the found cell, or the last written coordinates
/// when none is found (edge case 5); the result is the out room.
pub fn coarse_free_box<C: CollisionView>(
    cv: &C,
    room: C::Room,
    point: &mut Point,
    n: i32,
    mask: u32,
) -> Option<C::Room> {
    let (x0, y0) = (point.x, point.y);
    let mut rect = cv.room_rect(room);
    for j in 1..=COARSE_PASSES {
        let mut dy = -j;
        while dy < j {
            // Rule 2.
            let y = y0 + dy;
            point.y = y;
            let row_room = if has_row(&rect, y) {
                Some(room)
            } else {
                cv.cell_room(Some(room), point.x, y)
            };
            if let Some(row_room) = row_room {
                let mut dx = -j;
                while dx < j {
                    // Rule 3.
                    let x = x0 + dx;
                    point.x = x;
                    rect = cv.room_rect(row_room);
                    let cell_room = if has_column(&rect, x) {
                        Some(row_room)
                    } else {
                        cv.cell_room(Some(row_room), x, y)
                    };
                    if let Some(c) = cell_room {
                        let side = n + 2;
                        let v = if side < 2 {
                            // N + 2 ≤ 1 reads the cell's grid
                            // value masked with `mask`, from the room the
                            // cell lookup finds from the cell room (no
                            // room or grid → 0x27 unmasked).
                            cv.point_query(c, x, y, mask)
                        } else {
                            cv.box_query(c, x, y, side as u32, side as u32, mask)
                        };
                        if v == 0 {
                            return Some(c);
                        }
                    }
                    dx += 2;
                }
            }
            dy += 2;
        }
    }
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::place_seams::MISSING_ROOM_VALUE;
    use super::*;

    /// One or more rooms with collision grids; rooms are rects, adjacency
    /// = every other room in index order.
    #[derive(Clone, Debug)]
    pub(crate) struct Grid {
        pub rooms: Vec<(TileRect, Vec<u32>)>,
        /// Room-of-unit and units for the placement tests.
        pub units: Vec<FakeUnit>,
        pub log: Vec<String>,
        pub teleports: Vec<(usize, usize, i32, i32)>,
    }

    #[derive(Clone, Debug)]
    pub(crate) struct FakeUnit {
        pub room: Option<usize>,
        pub pos: Point,
        pub size: i32,
        pub has_path: bool,
    }

    impl Grid {
        /// The test vectors' room: [0, 20) × [0, 20), all masks 0.
        pub(crate) fn vec20() -> Grid {
            Grid::with_rooms(&[TileRect {
                x: 0,
                y: 0,
                w: 20,
                h: 20,
            }])
        }
        pub(crate) fn with_rooms(rects: &[TileRect]) -> Grid {
            Grid {
                rooms: rects
                    .iter()
                    .map(|r| (*r, vec![0; (r.w * r.h) as usize]))
                    .collect(),
                units: Vec::new(),
                log: Vec::new(),
                teleports: Vec::new(),
            }
        }
        fn find(&self, x: i32, y: i32) -> Option<usize> {
            self.rooms.iter().position(|(r, _)| r.contains(x, y))
        }
        pub(crate) fn set(&mut self, x: i32, y: i32, bits: u32) {
            let i = self.find(x, y).expect("cell in a room");
            let (r, g) = &mut self.rooms[i];
            g[((y - r.y) * r.w + (x - r.x)) as usize] |= bits;
        }
        fn raw(&self, x: i32, y: i32) -> u32 {
            match self.find(x, y) {
                Some(i) => {
                    let (r, g) = &self.rooms[i];
                    g[((y - r.y) * r.w + (x - r.x)) as usize]
                }
                None => MISSING_ROOM_VALUE,
            }
        }
        fn masked(&self, x: i32, y: i32, mask: u32) -> u32 {
            match self.find(x, y) {
                Some(_) => self.raw(x, y) & mask,
                None => MISSING_ROOM_VALUE,
            }
        }
    }

    impl CollisionView for Grid {
        type Room = usize;
        type Unit = usize;
        fn cell_room(&self, hint: Option<usize>, x: i32, y: i32) -> Option<usize> {
            let h = hint?;
            let inside = |i: usize| {
                let r = self.rooms[i].0;
                r.contains(x, y)
            };
            if inside(h) {
                return Some(h);
            }
            (0..self.rooms.len()).find(|&i| i != h && inside(i))
        }
        fn room_rect(&self, room: usize) -> TileRect {
            self.rooms[room].0
        }
        fn cell_value(&self, _room: usize, x: i32, y: i32) -> u32 {
            self.raw(x, y)
        }
        fn point_query(&self, _room: usize, x: i32, y: i32, mask: u32) -> u32 {
            self.masked(x, y, mask)
        }
        fn size_query(&self, _room: usize, x: i32, y: i32, size: i32, mask: u32) -> u32 {
            let cells: &[(i32, i32)] = match size {
                0 | 1 => &[(0, 0)],
                2 => &[(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)],
                3 => &[
                    (-1, -1),
                    (0, -1),
                    (1, -1),
                    (-1, 0),
                    (0, 0),
                    (1, 0),
                    (-1, 1),
                    (0, 1),
                    (1, 1),
                ],
                _ => return 0xFFFF,
            };
            cells
                .iter()
                .fold(0, |a, (dx, dy)| a | self.masked(x + dx, y + dy, mask))
        }
        fn box_query(&self, _room: usize, x: i32, y: i32, sx: u32, sy: u32, mask: u32) -> u32 {
            let (l, b) = (x - (sx / 2) as i32, y - (sy / 2) as i32);
            let mut v = 0;
            for yy in b..b + sy as i32 {
                for xx in l..l + sx as i32 {
                    v |= self.masked(xx, yy, mask);
                }
            }
            v
        }
        fn has_path(&self, unit: usize) -> bool {
            self.units[unit].has_path
        }
        fn unit_room(&self, unit: usize) -> Option<usize> {
            self.units[unit].room
        }
        fn unit_size(&self, unit: usize) -> i32 {
            self.units[unit].size
        }
        fn teleport(&mut self, unit: usize, room: usize, x: i32, y: i32) {
            self.teleports.push((unit, room, x, y));
            self.log.push(format!("teleport {unit} r{room} ({x},{y})"));
            let u = &mut self.units[unit];
            u.room = Some(room);
            u.pos = Point::new(x, y);
        }
        fn client_room_switch(&mut self, player: usize, room: usize) {
            self.log.push(format!("switch {player} r{room}"));
        }
        fn add_player_to_world(&mut self, unit: usize, room: usize, x: i32, y: i32) {
            self.log.push(format!("add {unit} r{room} ({x},{y})"));
            let u = &mut self.units[unit];
            u.room = Some(room);
            u.pos = Point::new(x, y);
        }
    }

    /// A synthetic walk-back field: each cell steps one cell toward the
    /// centre on each axis (sign of the offset). It reproduces the
    /// measured vectors F1–F3; the shipped file is checked by
    /// `expfield_live`.
    pub(crate) fn sign_field() -> ExpField {
        let mut cells = vec![0u8; 256 * 256];
        for fy in 0..256i32 {
            for fx in 0..256i32 {
                let (sx, sy) = ((128 - fx).signum(), (128 - fy).signum());
                let d = (0..9)
                    .find(|&d| FIELD_DX[d] == sx && FIELD_DY[d] == sy)
                    .unwrap();
                cells[(fy * 256 + fx) as usize] = d as u8;
            }
        }
        ExpField::from_cells(256, 256, cells).unwrap()
    }

    fn walls(g: &mut Grid, xs: std::ops::Range<i32>, ys: std::ops::Range<i32>) {
        for y in ys {
            for x in xs.clone() {
                g.set(x, y, 0x1);
            }
        }
    }

    fn p1() -> Grid {
        let mut g = Grid::vec20();
        walls(&mut g, 5..15, 5..15);
        g
    }

    fn p2() -> Grid {
        let mut g = Grid::vec20();
        for (x, y) in [
            (10, 10),
            (9, 10),
            (11, 10),
            (10, 9),
            (10, 11),
            (9, 9),
            (11, 11),
        ] {
            g.set(x, y, 0x1);
        }
        g
    }

    fn search(g: &Grid, x: i32, y: i32, size: i32, step: i32) -> (Option<usize>, Point) {
        let mut p = Point::new(x, y);
        let r = free_point_step(g, Some(0), &mut p, size, 0x1C09, step).unwrap();
        (r, p)
    }

    #[test]
    fn p1_ring_five_corner() {
        assert_eq!(search(&p1(), 10, 10, 2, 1), (Some(0), Point::new(15, 15)));
        // P1b: (15, 10) d 5 first; (10, 15) has the same d, found later.
        assert_eq!(search(&p1(), 10, 10, 1, 1), (Some(0), Point::new(15, 10)));
    }

    #[test]
    fn p2_plus_ring() {
        assert_eq!(search(&p2(), 10, 10, 2, 1), (Some(0), Point::new(12, 9)));
        assert_eq!(search(&p2(), 10, 10, 1, 1), (Some(0), Point::new(11, 9)));
    }

    #[test]
    fn p3_step_two() {
        assert_eq!(search(&p1(), 10, 10, 2, 2), (Some(0), Point::new(15, 15)));
    }

    #[test]
    fn p4_start_free() {
        let g = Grid::vec20();
        assert_eq!(search(&g, 10, 10, 2, 1), (Some(0), Point::new(10, 10)));
        // Max distance 1: the ring loop is not entered at all.
        let mut p = Point::new(10, 10);
        let mut g = Grid::vec20();
        g.set(10, 10, 1);
        let args = FreeSearch {
            size: 1,
            mask: 0x1C09,
            field: None,
            fallback: false,
            max_distance: 1,
            step: 1,
        };
        assert_eq!(nearest_free_point(&g, Some(0), &mut p, &args), Ok(None));
        assert_eq!(p, Point::new(10, 10));
    }

    #[test]
    fn p5_outside_room() {
        let g = Grid::vec20();
        assert_eq!(search(&g, -1, 5, 1, 1), (Some(0), Point::new(0, 5)));
        assert_eq!(search(&g, -1, 5, 2, 1), (Some(0), Point::new(1, 5)));
    }

    #[test]
    fn fallback_returns_unchanged_point_room() {
        // Everything walled: nothing free within 49 rings.
        let mut g = Grid::with_rooms(&[TileRect {
            x: 0,
            y: 0,
            w: 120,
            h: 120,
        }]);
        walls(&mut g, 0..120, 0..120);
        let mut p = Point::new(60, 60);
        assert_eq!(
            free_point(&g, Some(0), &mut p, 1, 0x1C09, true),
            Ok(Some(0))
        );
        assert_eq!(p, Point::new(60, 60));
        assert_eq!(free_point(&g, Some(0), &mut p, 1, 0x1C09, false), Ok(None));
        // Ring 49 is the last: a free cell at Chebyshev radius 49 is found,
        // one at radius 50 is not.
        let mut g2 = g.clone();
        g2.rooms[0].1[(60 * 120 + 109) as usize] = 0;
        let mut p = Point::new(60, 60);
        assert_eq!(
            free_point(&g2, Some(0), &mut p, 1, 0x1C09, false),
            Ok(Some(0))
        );
        assert_eq!(p, Point::new(109, 60));
        let mut g3 = g.clone();
        g3.rooms[0].1[(60 * 120 + 110) as usize] = 0;
        let mut p = Point::new(60, 60);
        assert_eq!(free_point(&g3, Some(0), &mut p, 1, 0x1C09, false), Ok(None));
    }

    #[test]
    fn ring_wins_over_closer_cell_of_next_ring() {
        // P1 with size 2: ring 5's corner (d = 10) beats a ring-6 side cell
        // (16, 10) whose d is 6.
        let (_, p) = search(&p1(), 10, 10, 2, 1);
        assert_eq!(p, Point::new(15, 15));
        assert!((p.x - 10).abs() + (p.y - 10).abs() > 6);
    }

    #[test]
    fn hint_follows_found_rooms() {
        // Two rooms side by side; the start lies in room 1, searched from
        // room 0's hint: the lookup finds room 1 through room 0's
        // neighbours and returns it.
        let g = Grid::with_rooms(&[
            TileRect {
                x: 0,
                y: 0,
                w: 10,
                h: 10,
            },
            TileRect {
                x: 10,
                y: 0,
                w: 10,
                h: 10,
            },
        ]);
        let mut p = Point::new(12, 3);
        assert_eq!(
            free_point(&g, Some(0), &mut p, 1, 0x1C09, false),
            Ok(Some(1))
        );
        // A null room finds nothing.
        let mut p = Point::new(12, 3);
        assert_eq!(free_point(&g, None, &mut p, 1, 0x1C09, true), Ok(None));
    }

    #[test]
    fn field_tables_and_f1() {
        let f = sign_field();
        let row = |dy: i32| -> Vec<u8> {
            (-1..=1)
                .map(|dx| f.byte(128 + dx, 128 + dy).unwrap())
                .collect()
        };
        assert_eq!(row(-1), [3, 4, 5]);
        assert_eq!(row(0), [2, 8, 6]);
        assert_eq!(row(1), [1, 0, 7]);
    }

    /// The `field_dx` / `field_dy` rows of `sim/path-tables.tsv`.
    fn tsv_field(tsv: &str, table: &str) -> Vec<i32> {
        tsv.lines()
            .skip(1)
            .map(|l| l.split('\t').collect::<Vec<_>>())
            .filter(|c| c[0] == table)
            .map(|c| c[2].parse().unwrap())
            .collect()
    }

    fn check_field_tables(tsv: &str) -> Result<(), String> {
        for (name, ours) in [("field_dx", &FIELD_DX), ("field_dy", &FIELD_DY)] {
            if tsv_field(tsv, name) != ours.to_vec() {
                return Err(name.to_string());
            }
        }
        Ok(())
    }

    #[test]
    fn field_tables_match_tsv() {
        let tsv = include_str!("../../../../tables/sim/path-tables.tsv");
        assert_eq!(check_field_tables(tsv), Ok(()));
        // M08: a changed row is reported.
        let bad = tsv.replace("field_dy\t3\t1\t", "field_dy\t3\t0\t");
        assert_ne!(bad, tsv);
        assert_eq!(check_field_tables(&bad), Err("field_dy".into()));
    }

    struct Tracing<'a> {
        g: &'a Grid,
        seen: std::cell::RefCell<Vec<(i32, i32)>>,
    }

    impl CollisionView for Tracing<'_> {
        type Room = usize;
        type Unit = usize;
        fn cell_room(&self, h: Option<usize>, x: i32, y: i32) -> Option<usize> {
            self.g.cell_room(h, x, y)
        }
        fn room_rect(&self, r: usize) -> TileRect {
            self.g.room_rect(r)
        }
        fn cell_value(&self, r: usize, x: i32, y: i32) -> u32 {
            self.g.cell_value(r, x, y)
        }
        fn point_query(&self, r: usize, x: i32, y: i32, m: u32) -> u32 {
            self.seen.borrow_mut().push((x, y));
            self.g.point_query(r, x, y, m)
        }
        fn size_query(&self, r: usize, x: i32, y: i32, s: i32, m: u32) -> u32 {
            self.g.size_query(r, x, y, s, m)
        }
        fn box_query(&self, r: usize, x: i32, y: i32, sx: u32, sy: u32, m: u32) -> u32 {
            self.seen.borrow_mut().push((x, y));
            self.g.box_query(r, x, y, sx, sy, m)
        }
        fn has_path(&self, _: usize) -> bool {
            false
        }
        fn unit_room(&self, _: usize) -> Option<usize> {
            None
        }
        fn unit_size(&self, _: usize) -> i32 {
            0
        }
        fn teleport(&mut self, _: usize, _: usize, _: i32, _: i32) {}
        fn add_player_to_world(&mut self, _: usize, _: usize, _: i32, _: i32) {}
    }

    #[test]
    fn f2_f3_walk_back_paths() {
        let g = Grid::vec20();
        let f = sign_field();
        let t = Tracing {
            g: &g,
            seen: Default::default(),
        };
        // Offsets only matter relative to the origin; use (10, 10) as the
        // origin inside the 20×20 room: F2 (103, 98) → (13, 8).
        let o = Point::new(10, 10);
        assert_eq!(walk_back(&t, &f, Some(0), 13, 8, o, 0x801), Ok(true));
        // Tested: the start, then each step except the centre.
        assert_eq!(*t.seen.borrow(), [(13, 8), (12, 9), (11, 10)]);
        t.seen.borrow_mut().clear();
        assert_eq!(walk_back(&t, &f, Some(0), 6, 14, o, 0x801), Ok(true));
        assert_eq!(*t.seen.borrow(), [(6, 14), (7, 13), (8, 12), (9, 11)]);
        // A null room passes without a test.
        assert_eq!(walk_back(&g, &f, None, 0, 0, o, 0x801), Ok(true));
    }

    #[test]
    fn walk_back_outside_rooms_is_blocked() {
        // Room [0, 20)²; the walk from (1, 1) toward origin (-5, -5)
        // leaves every room: the point test reads 0x27 there.
        let g = Grid::vec20();
        let f = sign_field();
        let o = Point::new(-5, -5);
        assert_eq!(walk_back(&g, &f, Some(0), 1, 1, o, 0x801), Ok(false));
        // A start already on a wall fails without walking.
        let mut g = Grid::vec20();
        g.set(13, 8, 0x800);
        assert_eq!(
            walk_back(&g, &f, Some(0), 13, 8, Point::new(10, 10), 0x801),
            Ok(false)
        );
    }

    #[test]
    fn expfield_parse() {
        let mut b = vec![0x0A, 0x01, 2, 0, 0, 0, 3, 0, 0, 0];
        b.extend([0, 1, 2, 3, 4, 5]);
        let f = ExpField::from_bytes(&b).unwrap();
        assert_eq!((f.version, f.height, f.width), (0x010A, 2, 3));
        // Readers use a fixed 256 stride: row 1 starts at byte 256.
        assert_eq!(f.byte(2, 0), Some(2));
        assert_eq!(f.byte(0, 1), None);
        b.push(0);
        assert_eq!(
            ExpField::from_bytes(&b),
            Err(FieldError::Size { got: 17, want: 16 })
        );
        assert_eq!(ExpField::from_bytes(&b[..9]), Err(FieldError::Header));
    }

    /// F1–F3 on the shipped field file (`D2_GAME_DIR`).
    #[test]
    #[ignore = "needs the game MPQs in D2_GAME_DIR"]
    fn expfield_live() {
        let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
        let set = d2_formats::mpq::ArchiveSet::open_dir(dir).expect("archives open");
        let bytes = set.read(ExpField::PATH).expect("ExpField.D2");
        assert_eq!(bytes.len(), 65_546);
        let f = ExpField::from_bytes(&bytes).unwrap();
        assert_eq!((f.version, f.height, f.width), (0x010A, 256, 256));
        // F1.
        let row = |dy: i32| -> Vec<u8> {
            (-1..=1)
                .map(|dx| f.byte(128 + dx, 128 + dy).unwrap())
                .collect()
        };
        assert_eq!(
            [row(-1), row(0), row(1)],
            [vec![3, 4, 5], vec![2, 8, 6], vec![1, 0, 7]]
        );
        // F2, F3 on an empty 200×200 room.
        let g = Grid::with_rooms(&[TileRect {
            x: 0,
            y: 0,
            w: 200,
            h: 200,
        }]);
        let t = Tracing {
            g: &g,
            seen: Default::default(),
        };
        let o = Point::new(100, 100);
        assert_eq!(walk_back(&t, &f, Some(0), 103, 98, o, 0x801), Ok(true));
        assert_eq!(*t.seen.borrow(), [(103, 98), (102, 99), (101, 100)]);
        t.seen.borrow_mut().clear();
        assert_eq!(walk_back(&t, &f, Some(0), 96, 104, o, 0x801), Ok(true));
        assert_eq!(
            *t.seen.borrow(),
            [(96, 104), (97, 103), (98, 102), (99, 101)]
        );
    }

    #[test]
    fn coarse_scan_order_and_parity() {
        let g = Grid::vec20();
        let t = Tracing {
            g: &g,
            seen: Default::default(),
        };
        // Mask 0x1: everything walled → no cell; record the visit order.
        let mut gw = Grid::vec20();
        walls(&mut gw, 0..20, 0..20);
        let tw = Tracing {
            g: &gw,
            seen: Default::default(),
        };
        let mut p = Point::new(10, 10);
        assert_eq!(coarse_free_box(&tw, 0, &mut p, 0, 0x1), None);
        let seen = tw.seen.borrow();
        // Pass 1: (9, 9). Pass 2: {−2, 0}² rows first. Pass 3: odd offsets.
        assert_eq!(seen[0], (9, 9));
        assert_eq!(seen[1..5], [(8, 8), (10, 8), (8, 10), (10, 10)]);
        assert_eq!(seen[5..8], [(7, 7), (9, 7), (11, 7)]);
        // Edge case 5: pass 3 rescans pass 1's cell.
        assert_eq!(seen[9], (9, 9));
        // Edge case 5: the out point holds the last written coordinates:
        // y of pass 49's last row (Y0 + 47, no room there) and x of the
        // last row that had a room (X0 + 47).
        assert_eq!(p, Point::new(10 + 47, 10 + 47));
        drop(seen);
        // Empty grid, n = 1: 3×3 box at (9, 9) is free → stop there.
        let mut p = Point::new(10, 10);
        assert_eq!(coarse_free_box(&t, 0, &mut p, 1, 0x1), Some(0));
        assert_eq!(p, Point::new(9, 9));
    }

    #[test]
    fn coarse_box_size_and_rooms() {
        // A wall at (9, 9) blocks pass 1 for n = 1 (3×3 box); pass 2 tries
        // (8, 8), whose box covers (9, 9) too; then (10, 8): box 9..11 ×
        // 7..9 holds (9, 9): blocked; (8, 10): box 7..9 × 9..11: blocked;
        // (10, 10): blocked; pass 3 (7, 7): box 6..8 free.
        let mut g = Grid::vec20();
        g.set(9, 9, 0x1);
        let mut p = Point::new(10, 10);
        assert_eq!(coarse_free_box(&g, 0, &mut p, 1, 0x1), Some(0));
        assert_eq!(p, Point::new(7, 7));
        // Cells of a neighbour room are reached by the lookup and that room
        // is the out room.
        let g = Grid::with_rooms(&[
            TileRect {
                x: 0,
                y: 0,
                w: 10,
                h: 10,
            },
            TileRect {
                x: 0,
                y: 10,
                w: 10,
                h: 10,
            },
        ]);
        let mut gb = g.clone();
        walls(&mut gb, 0..10, 0..10);
        let mut p = Point::new(5, 9);
        // Pass 1: (4, 8) in room 0 (walled). Pass 2: rows y = 7 (room 0),
        // y = 9 (room 0); pass 3: y = 6, 8 (room 0), y = 10 → room 1:
        // (2, 10) free with n = −2 (side 0: the raw cell value).
        assert_eq!(coarse_free_box(&gb, 0, &mut p, -2, 0x1), Some(1));
        assert_eq!(p, Point::new(2, 10));
    }
}

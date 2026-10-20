//! Pure geometry helpers. Integer arithmetic only; the game's code is
//! integer here (the x87 use of the target lead is a seam, open question 4).

use crate::path::collision::{pattern_collides, CollisionRooms};
use crate::path::coords::Point;
use crate::path::record::DynamicPath;
use crate::path::tables::PathTables;
use crate::units::{RoomId, UnitType};

/// No direction (255) in `altdir`.
pub const NO_DIR: u8 = 255;

fn clamp(v: i32, lo: i32, hi: i32) -> i32 {
    v.clamp(lo, hi)
}

/// Octant o(p → q): a row 0..24 of `testdir`
/// and `altdir`.
pub fn octant(p: Point, q: Point) -> usize {
    let mut dx = q.x - p.x;
    let mut dy = q.y - p.y;
    let ax = dx.abs();
    let ay = dy.abs();
    if ax < 2 * ay && 2 * ax <= ay {
        if dx < 0 {
            return (7 + clamp(dy, -2, 2)) as usize;
        }
        dx &= 1;
    } else if ax >= 2 * ay {
        dy = if dy < 0 { -1 } else { dy & 1 };
    }
    let dx = clamp(dx, -2, 2);
    if dy < -1 {
        (5 * dx + 10) as usize
    } else {
        (5 * dx + 12 + dy.min(2)) as usize
    }
}

/// Step of a direction (`dir8_toward`).
pub fn step(t: &PathTables, d: u8) -> Point {
    let [x, y] = t.dir8_toward[(d & 7) as usize];
    Point::new(x, y)
}

pub fn add(a: Point, b: Point) -> Point {
    Point::new(a.x + b.x, a.y + b.y)
}

/// Path distance.
pub fn path_distance(t: &PathTables, a: Point, b: Point) -> i32 {
    let ax = (a.x - b.x).abs();
    let ay = (a.y - b.y).abs();
    if ax < 8 && ay < 8 {
        let v = t.dist8_path[(ax + 8 * ay) as usize];
        if v < 0 {
            0
        } else {
            v + 1
        }
    } else {
        2 * ax.max(ay) + ax.min(ay)
    }
}

/// Unit distance between positions with sizes: a
/// negative `dist8_unit` entry returns 0 at once, with no size
/// adjustment.
pub fn unit_distance(t: &PathTables, a: Point, size_a: i32, b: Point, size_b: i32) -> i32 {
    let dx = (a.x - b.x).abs();
    let dy = (a.y - b.y).abs();
    if dx < 8 && dy < 8 && size_a < 4 && size_b < 4 {
        let mut d = t.dist8_unit[(dx + 8 * dy) as usize];
        // A negative entry returns 0 at once, with no size adjustment
        // (measured: the stash, SizeX 1, stops the run 2
        // sub-tiles away).
        if d < 0 {
            return 0;
        }
        if size_a == 3 || size_b == 3 {
            d = (d - 1).max(0);
        }
        if size_a < 2 || size_b < 2 {
            d + 1
        } else {
            d
        }
    } else {
        let ex = (dx - (size_a / 2 + size_b / 2)).max(0);
        let ey = (dy - (size_a / 2 + size_b / 2)).max(0);
        2 * ex.max(ey) + ex.min(ey)
    }
}

/// The collision source of a ray test or a probe: the path's room,
/// pattern and move mask.
pub trait Probe {
    /// Pattern query from `room`.
    fn collides(&self, room: Option<RoomId>, p: Point, pattern: u32, mask: u16) -> bool;
}

impl<W: CollisionRooms + ?Sized> Probe for W {
    fn collides(&self, room: Option<RoomId>, p: Point, pattern: u32, mask: u16) -> bool {
        pattern_collides(self, room, p.x, p.y, pattern, mask)
    }
}

/// Result of a ray test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ray {
    Clear,
    /// Blocked; the last cell before the blocking one.
    Blocked(Point),
}

/// Ray test from `s` to `e`.
pub fn ray_test<P: Probe + ?Sized>(
    w: &P,
    room: Option<RoomId>,
    pattern: u32,
    mask: u16,
    s: Point,
    e: Point,
) -> Ray {
    let nx = (e.x - s.x).abs() + 1;
    let ny = (e.y - s.y).abs() + 1;
    let sx = if e.x - s.x < 0 { -1 } else { 1 };
    let sy = if e.y - s.y < 0 { -1 } else { 1 };
    let test = |x: i32, y: i32| w.collides(room, Point::new(x, y), pattern, mask);
    let (mut x, mut y) = (s.x, s.y);
    if nx > ny {
        if e.x == s.x {
            return Ray::Clear;
        }
        let mut err = ny;
        loop {
            let remembered = Point::new(x, y);
            x += sx;
            if test(x, y) {
                return Ray::Blocked(remembered);
            }
            err += ny;
            if err >= nx {
                y += sy;
                err -= nx;
                if err > 0 && test(x, y) {
                    return Ray::Blocked(remembered);
                }
            }
            if x == e.x {
                return Ray::Clear;
            }
        }
    } else if nx == ny {
        loop {
            let remembered = Point::new(x, y);
            if x == e.x {
                return Ray::Clear;
            }
            x += sx;
            y += sy;
            if test(x, y) {
                return Ray::Blocked(remembered);
            }
        }
    } else {
        if e.y == s.y {
            return Ray::Clear;
        }
        let mut err = nx;
        loop {
            let remembered = Point::new(x, y);
            y += sy;
            if test(x, y) {
                return Ray::Blocked(remembered);
            }
            err += nx;
            if err >= ny {
                x += sx;
                err -= ny;
                if err > 0 && test(x, y) {
                    return Ray::Blocked(remembered);
                }
            }
            if y == e.y {
                return Ray::Clear;
            }
        }
    }
}

/// Direction vector from precise (sx, sy) to (tx,
/// ty): (vector, direction 0..63, before the 0x200 flip). A far point
/// wraps 127·small to a negative index (−127..−1), which reads from
/// the rows before the table (`tan_before`).
pub fn direction_vector(t: &PathTables, s: (u32, u32), e: (u32, u32)) -> ((i32, i32), u8) {
    let lx = e.0.abs_diff(s.0) as i32;
    let ly = e.1.abs_diff(s.1) as i32;
    let (mut vx, mut vy, mut a);
    if lx <= ly {
        let i = if ly == 0 {
            0
        } else {
            127i32.wrapping_mul(lx) / ly
        };
        let [x, y, angle] = tan_row(t, i);
        vx = x;
        vy = y;
        a = angle;
    } else {
        let i = 127i32.wrapping_mul(ly) / lx;
        let [x, y, angle] = tan_row(t, i);
        vx = y;
        vy = x;
        a = (-1 - angle) & 15;
    }
    if e.1 < s.1 {
        vy = -vy;
        a = (-1 - a) & 31;
    }
    let dir = if e.0 < s.0 {
        vx = -vx;
        (a + 8) & 63
    } else {
        (((-1 - a) & 63) + 8) & 63
    };
    ((vx, vy), dir as u8)
}

/// Row i of the `tan` table, unchecked: i < 0
/// (|i| ≤ 127) is the 12-byte row i before it.
fn tan_row(t: &PathTables, i: i32) -> [i32; 3] {
    match usize::try_from(i) {
        Ok(i) => t.tan[i],
        Err(_) => t.tan_before[(i + 127) as usize],
    }
}

/// Facing.
pub fn set_facing(t: &PathTables, path: &mut DynamicPath, ty: UnitType, d: i32) {
    let d = (d & 63) as u8;
    match ty {
        UnitType::Object | UnitType::Item => path.direction = d,
        UnitType::Missile if path.flags & 0x40 == 0 => path.direction = d,
        UnitType::Missile => {}
        _ => {
            if d != path.new_direction {
                path.new_direction = d;
                path.turn_step =
                    t.dirdiff[((d as i32 - path.direction as i32) & 63) as usize] as u8;
            }
        }
    }
}

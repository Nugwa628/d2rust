//! Property tests of the free-point searches, unit placement and warps
//! (`d2_sim::path::{search, place, warp}`), through the public API and
//! fakes of the seam traits (`path::place_seams`):
//!
//! - Nearest free point against a brute-force reference
//!   written from §7.2's ring order (any step, max distance, size, mask,
//!   fallback, walk-back field, rooms with any adjacency); and, for step 1
//!   and max distance 50 on rooms that do not overlap, an independent
//!   key form: a free point is returned iff one exists within Chebyshev
//!   radius 49, on the smallest such ring, with the smallest Manhattan
//!   distance there, the first of equals in §7.2 rule 3's order;
//! - Coarse free-box search likewise (reference from the
//!   rules; on one room the key form: pass j visits offsets of j's parity
//!   in [−j, j), so the winner is the free cell of smallest (pass, dy,
//!   dx) among those with dx ≡ dy (mod 2));
//! - §9 floor drop and §10 unit placement never put the result on a
//!   blocked sub-tile of the fake view (except §7's fallback point, edge
//!   case 3), make no RNG draw, call the host in the expected order,
//!   and are deterministic;
//! - §11 spawn point / game entry / level warp: the only draws are those
//!   of the DRLG's `spawn_room` seam (`drlg/levels.md` §10), called once,
//!   in its order; nothing else draws;
//! - §12 warp tile preset and warp arrival on random warp tables never
//!   panic and give the outcome the rules give.
//!
//! The fake view follows §4: a cell lookup is the hint room if it holds
//! the cell, else the first room of the hint's adjacency list that does;
//! a missing room or grid reads 0x27 unmasked; size and box queries look
//! their cells up from the centre's (box: the low corner's) room.
//! The reference follows the current choices for unresolved behavior:
//! walk-back uses the candidate's room; coarse search uses half-open rects.

use d2_sim::drlg::TileRect;
use d2_sim::path::place::{
    floor_drop, game_entry, level_spawn_point, level_warp_place, place_unit,
};
use d2_sim::path::place_seams::{
    CollisionView, LevelView, LvlWarp, PlaceError, PlaceHost, PlaceMessage, RoomReveal,
    WarpDestination, WarpTileView,
};
use d2_sim::path::search::{
    coarse_free_box, free_point, nearest_free_point, ExpField, FieldTest, FreeSearch,
};
use d2_sim::path::warp::{warp_player, warp_slot, warp_tile_preset, WarpOutcome};
use d2_sim::path::Point;
use d2_sim::rng::Seed;
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

// ---- deterministic generator for the fixtures ----------------------------

/// splitmix64: builds worlds and fields from one proptest seed.
struct Mix(u64);

impl Mix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + self.below((hi - lo) as u64) as i32
    }
}

// ---- fake collision view (§4) --------------------------------------------

const MISSING: u32 = 0x27;
/// Collision bits the grids draw from (§3: WALL, the masks' other bits,
/// and bits no mask of §7–§12 holds).
const BITS: [u32; 10] = [
    0x1, 0x2, 0x4, 0x8, 0x80, 0x200, 0x400, 0x800, 0x1000, 0x2000,
];

#[derive(Clone, Debug, PartialEq)]
struct Room {
    rect: TileRect,
    grid: Option<Vec<u32>>,
    /// Adjacency array (`drlg/rooms.md` §6 order).
    adj: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
struct FakeUnit {
    room: Option<usize>,
    pos: Point,
    size: i32,
    has_path: bool,
}

#[derive(Clone, Debug, PartialEq, Default)]
struct World {
    rooms: Vec<Room>,
    units: Vec<FakeUnit>,
    teleports: Vec<(usize, usize, i32, i32)>,
    added: Vec<(usize, usize, i32, i32)>,
    /// Game entry's room switches (player, room).
    switched: Vec<(usize, usize)>,
}

impl World {
    fn holds(&self, r: usize, x: i32, y: i32) -> bool {
        let rc = self.rooms[r].rect;
        rc.contains(x, y)
    }

    /// §4 rule 2 from a looked-up room: unmasked value, 0x27 for no room
    /// or no grid.
    fn raw(&self, room: Option<usize>, x: i32, y: i32) -> Option<u32> {
        let r = room?;
        let rm = &self.rooms[r];
        let g = rm.grid.as_ref()?;
        let rc = rm.rect;
        Some(g[((y - rc.y) * rc.w + (x - rc.x)) as usize])
    }

    fn masked_from(&self, room: Option<usize>, x: i32, y: i32, mask: u32) -> u32 {
        match self.raw(self.cell_room(room, x, y), x, y) {
            Some(v) => v & mask,
            None => MISSING,
        }
    }

    /// For 3 of 4 seeds: walls the cell (cx, cy) and sets a random bit on
    /// about half the cells within Chebyshev radius 4 of it (in every
    /// room holding them), so the ring and scan orders decide the result
    /// instead of a free start.
    fn crowd(mut self, seed: u64, cx: i32, cy: i32) -> World {
        if seed & 3 == 0 {
            return self;
        }
        let mut m = Mix(seed ^ 0x5EED_C0DE);
        for y in cy - 4..=cy + 4 {
            for x in cx - 4..=cx + 4 {
                let bits = if (x, y) == (cx, cy) {
                    0x1
                } else if m.below(2) == 0 {
                    BITS[m.below(BITS.len() as u64) as usize]
                } else {
                    continue;
                };
                for rm in &mut self.rooms {
                    let rc = rm.rect;
                    if let (true, Some(g)) = (rc.contains(x, y), rm.grid.as_mut()) {
                        g[((y - rc.y) * rc.w + (x - rc.x)) as usize] |= bits;
                    }
                }
            }
        }
        self
    }

    /// The room holding the cell, when rooms do not overlap.
    fn room_at(&self, x: i32, y: i32) -> Option<usize> {
        (0..self.rooms.len()).find(|&r| self.holds(r, x, y))
    }

    fn gen_grid(m: &mut Mix, rect: TileRect, density: u64) -> Option<Vec<u32>> {
        if m.below(10) == 0 {
            return None;
        }
        Some(
            (0..rect.w * rect.h)
                .map(|_| {
                    if m.below(100) < density {
                        BITS[m.below(BITS.len() as u64) as usize]
                    } else {
                        0
                    }
                })
                .collect(),
        )
    }

    /// 1–3 rooms anywhere in [−10, 55)², overlapping (when `overlap`) or
    /// not, random adjacency arrays (subset, any order). Real rooms do
    /// not overlap; overlaps only exercise the hint rules of §4 rule 1 /
    /// §7.2 rule 1 against the references (with them, the room §7.2
    /// rule 4 returns may hold a different grid than the one tested).
    fn any(seed: u64, density: u64, overlap: bool) -> World {
        let mut m = Mix(seed);
        let mut rooms: Vec<Room> = Vec::new();
        for _ in 0..1 + m.below(3) {
            let rect = TileRect {
                x: m.range(-10, 30),
                y: m.range(-10, 30),
                w: m.range(1, 26),
                h: m.range(1, 26),
            };
            let meets = |o: &Room| {
                let q = o.rect;
                rect.x < q.x + q.w
                    && q.x < rect.x + rect.w
                    && rect.y < q.y + q.h
                    && q.y < rect.y + rect.h
            };
            if !overlap && rooms.iter().any(meets) {
                continue;
            }
            let grid = World::gen_grid(&mut m, rect, density);
            rooms.push(Room {
                rect,
                grid,
                adj: Vec::new(),
            });
        }
        let n = rooms.len();
        for (i, room) in rooms.iter_mut().enumerate() {
            let mut others: Vec<usize> = (0..n).filter(|&j| j != i && m.below(3) != 0).collect();
            for a in (1..others.len()).rev() {
                let b = m.below(a as u64 + 1) as usize;
                others.swap(a, b);
            }
            room.adj = others;
        }
        World {
            rooms,
            ..World::default()
        }
    }

    /// 1–4 rooms in the 2 × 2 blocks of 20 × 20 at (0, 0), (20, 0),
    /// (0, 20), (20, 20), none overlapping, each adjacent to all others:
    /// whether a cell is free does not depend on the hint.
    fn tiled(seed: u64, density: u64) -> World {
        let mut m = Mix(seed);
        let n = 1 + m.below(4) as usize;
        let mut rooms = Vec::new();
        for i in 0..n {
            let (bx, by) = ((i as i32 % 2) * 20, (i as i32 / 2) * 20);
            let (w, h) = (m.range(1, 21), m.range(1, 21));
            let rect = TileRect {
                x: bx + m.range(0, 21 - w + 1).min(20 - w),
                y: by + m.range(0, 21 - h + 1).min(20 - h),
                w,
                h,
            };
            let grid = World::gen_grid(&mut m, rect, density);
            rooms.push(Room {
                rect,
                grid,
                adj: (0..n).filter(|&j| j != i).collect(),
            });
        }
        World {
            rooms,
            ..World::default()
        }
    }
}

impl CollisionView for World {
    type Room = usize;
    type Unit = usize;

    fn cell_room(&self, hint: Option<usize>, x: i32, y: i32) -> Option<usize> {
        let h = hint?;
        if self.holds(h, x, y) {
            return Some(h);
        }
        self.rooms[h]
            .adj
            .iter()
            .copied()
            .find(|&r| self.holds(r, x, y))
    }
    fn room_rect(&self, room: usize) -> TileRect {
        self.rooms[room].rect
    }
    fn cell_value(&self, room: usize, x: i32, y: i32) -> u32 {
        self.raw(self.cell_room(Some(room), x, y), x, y)
            .unwrap_or(MISSING)
    }
    fn point_query(&self, room: usize, x: i32, y: i32, mask: u32) -> u32 {
        self.masked_from(Some(room), x, y, mask)
    }
    fn size_query(&self, room: usize, x: i32, y: i32, size: i32, mask: u32) -> u32 {
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
        let Some(c) = self.cell_room(Some(room), x, y) else {
            return MISSING;
        };
        cells.iter().fold(0, |a, (dx, dy)| {
            a | self.masked_from(Some(c), x + dx, y + dy, mask)
        })
    }
    fn box_query(&self, room: usize, x: i32, y: i32, sx: u32, sy: u32, mask: u32) -> u32 {
        let (l, b) = (x - (sx / 2) as i32, y - (sy / 2) as i32);
        let Some(base) = self.cell_room(Some(room), l, b) else {
            return MISSING;
        };
        let mut v = 0;
        for yy in b..b + sy as i32 {
            for xx in l..l + sx as i32 {
                v |= self.masked_from(Some(base), xx, yy, mask);
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
        let u = &mut self.units[unit];
        u.room = Some(room);
        u.pos = Point::new(x, y);
    }
    fn add_player_to_world(&mut self, unit: usize, room: usize, x: i32, y: i32) {
        // Game entry runs the room switch before the placement (§11).
        assert_eq!(self.switched.last().map(|&(u, _)| u), Some(unit));
        self.added.push((unit, room, x, y));
        let u = &mut self.units[unit];
        u.room = Some(room);
        u.pos = Point::new(x, y);
    }
    fn client_room_switch(&mut self, player: usize, room: usize) {
        self.switched.push((player, room));
    }
}

// ---- walk-back fields (§7.3) ---------------------------------------------

/// Direction offsets of §7.3 rule 2, written from the spec's list.
const DIRS: [(i32, i32); 9] = [
    (0, -1),
    (1, -1),
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, 0),
];

/// A random 256 × 256 field with §7.3 rule 2's measured shape: 8 only at
/// the centre (128, 128), every other byte a direction that brings the
/// cell one Chebyshev step closer to the centre (so every walk ends at
/// the centre inside the grid).
fn gen_field(seed: u64) -> Vec<u8> {
    let mut m = Mix(seed);
    let mut cells = vec![0u8; 256 * 256];
    for fy in 0..256i32 {
        for fx in 0..256i32 {
            let (a, b) = (fx - 128, fy - 128);
            let cheb = a.abs().max(b.abs());
            if cheb == 0 {
                cells[(fy * 256 + fx) as usize] = 8;
                continue;
            }
            let ok: Vec<u8> = (0..8u8)
                .filter(|&d| {
                    let (dx, dy) = DIRS[d as usize];
                    (a + dx).abs().max((b + dy).abs()) < cheb
                })
                .collect();
            cells[(fy * 256 + fx) as usize] = ok[m.below(ok.len() as u64) as usize];
        }
    }
    cells
}

struct RefField<'a> {
    cells: &'a [u8],
    origin: Point,
    fmask: u32,
}

impl RefField<'_> {
    fn at(&self, x: i32, y: i32) -> u8 {
        let (fx, fy) = (x - self.origin.x + 128, y - self.origin.y + 128);
        assert!(
            (0..256).contains(&fx) && (0..256).contains(&fy),
            "fixture keeps walks in the field"
        );
        self.cells[(fy * 256 + fx) as usize]
    }
}

/// §7.3 rule 3, every point test from `room`.
fn ref_walk(w: &World, f: &RefField, room: usize, x: i32, y: i32) -> bool {
    if w.point_query(room, x, y, f.fmask) != 0 {
        return false;
    }
    let (mut x, mut y) = (x, y);
    loop {
        let (dx, dy) = DIRS[f.at(x, y) as usize];
        x += dx;
        y += dy;
        if f.at(x, y) == 8 {
            return true;
        }
        if w.point_query(room, x, y, f.fmask) != 0 {
            return false;
        }
    }
}

// ---- §7 reference --------------------------------------------------------

struct RefArgs<'a> {
    size: i32,
    mask: u32,
    field: Option<RefField<'a>>,
    fallback: bool,
    d: i32,
    k: i32,
}

/// §7.2 rule 1 with the hint update.
fn ref_free(w: &World, hint: &mut Option<usize>, x: i32, y: i32, a: &RefArgs) -> bool {
    let Some(r) = w.cell_room(*hint, x, y) else {
        return false;
    };
    *hint = Some(r);
    if w.size_query(r, x, y, a.size, a.mask) != 0 {
        return false;
    }
    match &a.field {
        None => true,
        Some(f) => ref_walk(w, f, r, x, y),
    }
}

/// The candidates of ring r in §7.2 rule 3's order.
fn ring(x0: i32, y0: i32, r: i32, k: i32) -> Vec<(i32, i32)> {
    let off = (r - 1) * k;
    let (l, rr, t, b, s) = (x0 - off, x0 + off, y0 - 1 - off, y0 + 1 + off, 2 + 2 * off);
    let mut v = Vec::new();
    let mut y = t;
    while y <= b {
        v.push((l - 1, y));
        v.push((l - 1 + s, y));
        y += k;
    }
    let mut x = l;
    while x <= rr {
        v.push((x, t));
        v.push((x, t + s));
        x += k;
    }
    v
}

/// §7.2: (result room, point after the call).
fn ref_nearest(w: &World, room: Option<usize>, p: Point, a: &RefArgs) -> (Option<usize>, Point) {
    let mut hint = room;
    if ref_free(w, &mut hint, p.x, p.y, a) {
        return (w.cell_room(hint, p.x, p.y), p);
    }
    let mut kept: Option<(Point, i32)> = None;
    if a.d > 1 {
        let mut r = 1;
        loop {
            for (x, y) in ring(p.x, p.y, r, a.k) {
                if ref_free(w, &mut hint, x, y, a) {
                    let d = (x - p.x).abs() + (y - p.y).abs();
                    if kept.is_none_or(|(_, kd)| d < kd) {
                        kept = Some((Point::new(x, y), d));
                    }
                }
            }
            if kept.is_some() || r * a.k + 1 >= a.d {
                break;
            }
            r += 1;
        }
    }
    match kept {
        Some((q, _)) => (w.cell_room(hint, q.x, q.y), q),
        None if a.fallback => (w.cell_room(hint, p.x, p.y), p),
        None => (None, p),
    }
}

/// Free on a world of non-overlapping, all-adjacent rooms: the cell's
/// room exists and its size query is 0 (no hint dependence).
fn free_static(w: &World, x: i32, y: i32, size: i32, mask: u32) -> bool {
    w.room_at(x, y)
        .is_some_and(|r| w.size_query(r, x, y, size, mask) == 0)
}

/// Ring key of [`ref_k1`]: (d, (columns 0 / rows 1, y or x, far side)).
type RingKey = (i32, (u8, i32, bool));

/// Step 1, max distance 50, independent key form (module docs).
fn ref_k1(w: &World, x0: i32, y0: i32, size: i32, mask: u32) -> Option<Point> {
    for r in 1..=49 {
        let mut best: Option<(RingKey, Point)> = None;
        for y in y0 - r..=y0 + r {
            for x in x0 - r..=x0 + r {
                let (ax, ay) = ((x - x0).abs(), (y - y0).abs());
                if ax.max(ay) != r || !free_static(w, x, y, size, mask) {
                    continue;
                }
                // Columns (|dx| = r) come first, by y, left before right;
                // then rows by x, top before bottom.
                let order = if ax == r {
                    (0, y, x > x0)
                } else {
                    (1, x, y > y0)
                };
                let key = (ax + ay, order);
                if best.as_ref().is_none_or(|(bk, _)| key < *bk) {
                    best = Some((key, Point::new(x, y)));
                }
            }
        }
        if let Some((_, p)) = best {
            return Some(p);
        }
    }
    None
}

// ---- §8 reference --------------------------------------------------------

/// §8 "inside the rect's rows / columns", half-open (the code's reading,
/// `impl-path-place.md` §4 question 3).
fn has_row(r: &TileRect, y: i32) -> bool {
    r.y <= y && y < r.y + r.h
}

fn has_column(r: &TileRect, x: i32) -> bool {
    r.x <= x && x < r.x + r.w
}

fn ref_coarse(w: &World, room: usize, p: Point, n: i32, mask: u32) -> (Option<usize>, Point) {
    let (x0, y0) = (p.x, p.y);
    let mut pt = p;
    let mut rect = w.room_rect(room);
    for j in 1..=49 {
        let mut dy = -j;
        while dy < j {
            let y = y0 + dy;
            pt.y = y;
            let row = if has_row(&rect, y) {
                Some(room)
            } else {
                w.cell_room(Some(room), pt.x, y)
            };
            if let Some(row) = row {
                let mut dx = -j;
                while dx < j {
                    let x = x0 + dx;
                    pt.x = x;
                    rect = w.room_rect(row);
                    let cell = if has_column(&rect, x) {
                        Some(row)
                    } else {
                        w.cell_room(Some(row), x, y)
                    };
                    if let Some(c) = cell {
                        // §8 rule 3: n + 2 ≤ 1 reads the grid value
                        // masked (0x27 unmasked without a room or grid).
                        let v = if n + 2 < 2 {
                            w.point_query(c, x, y, mask)
                        } else {
                            w.box_query(c, x, y, (n + 2) as u32, (n + 2) as u32, mask)
                        };
                        if v == 0 {
                            return (Some(c), pt);
                        }
                    }
                    dx += 2;
                }
            }
            dy += 2;
        }
    }
    (None, pt)
}

fn coarse_free(w: &World, room: usize, x: i32, y: i32, n: i32, mask: u32) -> bool {
    let v = if n + 2 < 2 {
        w.point_query(room, x, y, mask)
    } else {
        w.box_query(room, x, y, (n + 2) as u32, (n + 2) as u32, mask)
    };
    v == 0
}

// ---- fake hosts ----------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Ev {
    Send(usize, PlaceMessage<usize>),
    Queue(usize),
    Flags(usize, u32),
    RoomChange(usize),
    Point(usize, i32, i32),
    Timer(usize, u8, u32),
    Pets(usize),
    Walk(usize, i32, i32),
}

#[derive(Clone, Debug, PartialEq, Default)]
struct Host {
    players: Vec<usize>,
    log: Vec<Ev>,
    life: u8,
}

impl PlaceHost<usize> for Host {
    fn is_player(&self, unit: usize) -> bool {
        self.players.contains(&unit)
    }
    fn queue_update(&mut self, unit: usize) {
        self.log.push(Ev::Queue(unit));
    }
    fn or_flags2(&mut self, unit: usize, bits: u32) {
        self.log.push(Ev::Flags(unit, bits));
    }
    fn room_change_messages(&mut self, unit: usize) {
        self.log.push(Ev::RoomChange(unit));
    }
    fn send(&mut self, player: usize, msg: PlaceMessage<usize>) {
        self.log.push(Ev::Send(player, msg));
    }
    fn set_player_point(&mut self, player: usize, x: i32, y: i32) {
        self.log.push(Ev::Point(player, x, y));
    }
    fn schedule_event(&mut self, unit: usize, event: u8, delay: u32) {
        self.log.push(Ev::Timer(unit, event, delay));
    }
    fn pets_follow(&mut self, player: usize) {
        self.log.push(Ev::Pets(player));
    }
    fn request_walk(&mut self, player: usize, x: i32, y: i32) {
        self.log.push(Ev::Walk(player, x, y));
    }
    fn life_percent(&self, _unit: usize) -> u8 {
        self.life
    }
}

/// A seed that records every draw.
#[derive(Clone, Debug, PartialEq)]
struct CountingRng {
    seed: Seed,
    draws: Vec<u32>,
}

impl CountingRng {
    fn new(lo: u32) -> CountingRng {
        CountingRng {
            seed: Seed::init_low(lo),
            draws: Vec::new(),
        }
    }
    fn roll(&mut self, n: i32) -> u32 {
        let v = self.seed.roll(n);
        self.draws.push(v);
        v
    }
}

/// DRLG fake: `spawn_room` draws `spawn_draws` times on the level seed
/// (the only draw on these paths, `drlg/levels.md` §10).
#[derive(Clone, Debug, PartialEq)]
struct Levels {
    rng: CountingRng,
    spawn: Option<(usize, i32, i32)>,
    spawn_draws: u32,
    spawn_calls: Vec<(u8, u32, u32)>,
    start_level: u32,
    warp: Option<WarpDestination<usize>>,
    gate: u32,
}

impl Levels {
    fn new(lo: u32) -> Levels {
        Levels {
            rng: CountingRng::new(lo),
            spawn: None,
            spawn_draws: 0,
            spawn_calls: Vec::new(),
            start_level: 1,
            warp: None,
            gate: 0,
        }
    }
}

fn reveal(room: usize) -> RoomReveal {
    RoomReveal {
        tile_x: 30 + room as i32,
        tile_y: 60 - room as i32,
        level: 2 + room as u32,
    }
}

impl LevelView<usize> for Levels {
    type Act = u8;
    fn spawn_room(&mut self, act: u8, level: u32, tile_index: u32) -> Option<(usize, i32, i32)> {
        self.spawn_calls.push((act, level, tile_index));
        for _ in 0..self.spawn_draws {
            self.rng.roll(100);
        }
        self.spawn
    }
    fn act_start_level(&self, _act: u8) -> u32 {
        self.start_level
    }
    fn room_reveal(&self, room: usize) -> RoomReveal {
        reveal(room)
    }
    fn warp_destination(&self, _tile_room: usize, _class: u32) -> Option<WarpDestination<usize>> {
        self.warp
    }
    fn quest_gate(&self, _source: u32, _level: u32) -> u32 {
        self.gate
    }
}

fn first_draws(lo: u32, n: usize) -> Vec<u32> {
    let mut s = Seed::init_low(lo);
    (0..n).map(|_| s.roll(100)).collect()
}

fn reveal_msg(room: usize) -> PlaceMessage<usize> {
    let r = reveal(room);
    PlaceMessage::MapReveal {
        x: r.tile_x as u16,
        y: r.tile_y as u16,
        level: r.level as u8,
    }
}

/// Host calls of §10 rules 5–6 for a placed unit.
fn place_events(player: bool, u: usize, room: usize, p: Point, alt: bool) -> Vec<Ev> {
    let bits = if alt { 0x800 } else { 0x10000 };
    if !player {
        return vec![Ev::Queue(u), Ev::Flags(u, bits), Ev::RoomChange(u)];
    }
    vec![
        Ev::Send(u, reveal_msg(room)),
        Ev::Queue(u),
        Ev::Flags(u, bits),
        Ev::RoomChange(u),
        Ev::Point(u, p.x, p.y),
        Ev::Timer(u, 14, 50),
        Ev::Pets(u),
    ]
}

const MASKS: [u32; 5] = [0x1C09, 0x3E01, 0x801, 0x1, 0];

// ---- §7 ------------------------------------------------------------------

proptest! {
    #![proptest_config(config(256))]

    /// §7.2 against the ring-order reference, any arguments.
    #[test]
    fn nearest_matches_ring_reference(
        seed in any::<u64>(),
        density in 0u64..90,
        px in -15i32..60, py in -15i32..60,
        size in 0i32..5, mask_i in 0usize..5,
        fallback in any::<bool>(),
        d in 0i32..60, k in 1i32..5,
        room_none in 0u8..10,
    ) {
        let w = World::any(seed, density, true).crowd(seed, px, py);
        let room = (room_none != 0).then(|| (seed % w.rooms.len() as u64) as usize);
        let a = RefArgs { size, mask: MASKS[mask_i], field: None, fallback, d, k };
        let mut p = Point::new(px, py);
        let args = FreeSearch { size, mask: MASKS[mask_i], field: None, fallback, max_distance: d, step: k };
        let got = nearest_free_point(&w, room, &mut p, &args).unwrap();
        prop_assert_eq!((got, p), ref_nearest(&w, room, Point::new(px, py), &a));
    }

    /// §7.2 + §7.3 with random walk-back fields and origins.
    #[test]
    fn nearest_with_field_matches_reference(
        seed in any::<u64>(),
        fseed in any::<u64>(),
        density in 0u64..60,
        px in -10i32..50, py in -10i32..50,
        ox in -10i32..50, oy in -10i32..50,
        size in 0i32..4, fallback in any::<bool>(),
        d in 0i32..60,
    ) {
        let w = World::any(seed, density, true).crowd(seed, px, py);
        let cells = gen_field(fseed);
        let field = ExpField::from_cells(256, 256, cells.clone()).unwrap();
        let origin = Point::new(ox, oy);
        let room = Some((seed % w.rooms.len() as u64) as usize);
        let args = FreeSearch {
            size, mask: 0x3E01,
            field: Some(FieldTest { field: &field, origin, mask: 0x801 }),
            fallback, max_distance: d, step: 1,
        };
        let a = RefArgs {
            size, mask: 0x3E01,
            field: Some(RefField { cells: &cells, origin, fmask: 0x801 }),
            fallback, d, k: 1,
        };
        let mut p = Point::new(px, py);
        let got = nearest_free_point(&w, room, &mut p, &args).unwrap();
        prop_assert_eq!((got, p), ref_nearest(&w, room, Point::new(px, py), &a));
    }

    /// `free_point` (step 1, max distance 50): a free point is returned
    /// iff one exists within radius 49 (or the start is free), and it is
    /// the first nearest one of the smallest ring.
    #[test]
    fn free_point_iff_within_radius(
        seed in any::<u64>(),
        density in 0u64..101,
        px in -60i32..100, py in -60i32..100,
        size in 0i32..4, mask_i in 0usize..5,
        fallback in any::<bool>(),
    ) {
        let w = World::tiled(seed, density).crowd(seed, px, py);
        let room = Some((seed % w.rooms.len() as u64) as usize);
        let mut p = Point::new(px, py);
        let mask = MASKS[mask_i];
        let got = free_point(&w, room, &mut p, size, mask, fallback).unwrap();
        if free_static(&w, px, py, size, mask) {
            prop_assert_eq!(p, Point::new(px, py));
            prop_assert_eq!(got, w.room_at(px, py));
            return Ok(());
        }
        match ref_k1(&w, px, py, size, mask) {
            Some(q) => {
                prop_assert_eq!(p, q);
                prop_assert_eq!(got, w.room_at(q.x, q.y));
                prop_assert!(free_static(&w, p.x, p.y, size, mask));
            }
            None => {
                prop_assert_eq!(p, Point::new(px, py));
                let want = if fallback { w.room_at(px, py) } else { None };
                prop_assert_eq!(got, want);
            }
        }
    }
}

// ---- §8 ------------------------------------------------------------------

proptest! {
    #![proptest_config(config(256))]

    /// §8 against the rule reference, any rooms.
    #[test]
    fn coarse_matches_reference(
        seed in any::<u64>(),
        density in 0u64..101,
        px in -15i32..60, py in -15i32..60,
        n in -3i32..4, mask_i in 0usize..5,
    ) {
        let w = World::any(seed, density, true).crowd(seed, px, py);
        let room = (seed % w.rooms.len() as u64) as usize;
        let mut p = Point::new(px, py);
        let got = coarse_free_box(&w, room, &mut p, n, MASKS[mask_i]);
        prop_assert_eq!((got, p), ref_coarse(&w, room, Point::new(px, py), n, MASKS[mask_i]));
    }

    /// One room: found iff a free cell with dx ≡ dy (mod 2) and both in
    /// [−49, 47] exists; the winner has the smallest (pass, dy, dx).
    #[test]
    fn coarse_iff_and_first_in_scan(
        seed in any::<u64>(),
        density in 0u64..101,
        px in -60i32..100, py in -60i32..100,
        n in -3i32..4, mask_i in 0usize..5,
    ) {
        let mut w = World::tiled(seed, density).crowd(seed, px, py);
        w.rooms.truncate(1);
        w.rooms[0].adj.clear();
        let mask = MASKS[mask_i];
        let mut best: Option<((i32, i32, i32), Point)> = None;
        for dy in -49..=47 {
            for dx in -49..=47 {
                if (dx - dy) % 2 != 0 {
                    continue;
                }
                let (x, y) = (px + dx, py + dy);
                if !w.holds(0, x, y) || !coarse_free(&w, 0, x, y, n, mask) {
                    continue;
                }
                let mut j = (dx + 1).max(-dx).max(dy + 1).max(-dy);
                if (j - dx) % 2 != 0 {
                    j += 1;
                }
                let key = (j, dy, dx);
                if best.as_ref().is_none_or(|(bk, _)| key < *bk) {
                    best = Some((key, Point::new(x, y)));
                }
            }
        }
        let mut p = Point::new(px, py);
        let got = coarse_free_box(&w, 0, &mut p, n, mask);
        match best {
            Some((_, q)) => {
                prop_assert_eq!(got, Some(0));
                prop_assert_eq!(p, q);
            }
            None => {
                prop_assert_eq!(got, None);
                // The last row written is pass 49's last, dy = 47.
                prop_assert_eq!(p.y, py + 47);
            }
        }
    }
}

// ---- §9 ------------------------------------------------------------------

proptest! {
    #![proptest_config(config(128))]

    /// §9: the reference result; a point found by the search is free of
    /// 0x3E01 for the size and walks back to `from` through 0x801; with
    /// nothing found the result is the fallback of the unchanged start.
    #[test]
    fn floor_drop_never_blocked(
        seed in any::<u64>(),
        fseed in any::<u64>(),
        density in 0u64..80,
        fx in -10i32..50, fy in -10i32..50,
        size in 0i32..4, fallback in any::<bool>(),
    ) {
        let w = World::any(seed, density, false).crowd(seed, fx + 2, fy + 3);
        let cells = gen_field(fseed);
        let field = ExpField::from_cells(256, 256, cells.clone()).unwrap();
        let room = Some((seed % w.rooms.len() as u64) as usize);
        let from = Point::new(fx, fy);
        let before = w.clone();
        let got = floor_drop(&w, &field, room, from, size, fallback).unwrap();
        prop_assert_eq!(&w, &before, "a drop changes nothing");
        // Rule 1.
        let shifted = Point::new(fx + 2, fy + 3);
        let start = if w.cell_room(room, shifted.x, shifted.y).is_some() { shifted } else { from };
        let a = RefArgs {
            size, mask: 0x3E01,
            field: Some(RefField { cells: &cells, origin: from, fmask: 0x801 }),
            fallback, d: 50, k: 1,
        };
        let want = ref_nearest(&w, room, start, &a);
        prop_assert_eq!(got, want);
        // Never on a blocked cell, unless it is the fallback start.
        let nofb = RefArgs { fallback: false, ..a };
        let found = ref_nearest(&w, room, start, &nofb).0.is_some();
        if let (Some(r), p) = got {
            if found {
                prop_assert_eq!(w.size_query(r, p.x, p.y, size, 0x3E01), 0);
                let f = RefField { cells: &cells, origin: from, fmask: 0x801 };
                prop_assert!(ref_walk(&w, &f, r, p.x, p.y));
            } else {
                prop_assert!(fallback);
                prop_assert_eq!(p, start);
            }
        }
        // Deterministic.
        prop_assert_eq!(floor_drop(&w, &field, room, from, size, fallback).unwrap(), got);
    }
}

// ---- §10 -----------------------------------------------------------------

proptest! {
    #![proptest_config(config(256))]

    /// §10: never placed on a cell the unit's size query with 0x1C09
    /// finds blocked (unless `exact`), host calls in rule order, no draw,
    /// deterministic.
    #[test]
    fn place_unit_never_blocked(
        seed in any::<u64>(),
        density in 0u64..101,
        x in -15i32..60, y in -15i32..60,
        size in 0i32..5, has_path in prop::bool::weighted(0.9),
        player in any::<bool>(), exact in any::<bool>(), alt in any::<bool>(),
        room_sel in 0u8..4, unit_room_sel in 0u8..4,
    ) {
        let mut w = World::any(seed, density, false).crowd(seed, x, y);
        let nr = w.rooms.len();
        let pick = |s: u8| (s != 0).then(|| (s as usize + seed as usize) % nr);
        w.units.push(FakeUnit {
            room: pick(unit_room_sel),
            pos: Point::new(0, 0),
            size,
            has_path,
        });
        let u = 0;
        let room = pick(room_sel);
        let mut host = Host { players: if player { vec![u] } else { vec![] }, ..Host::default() };
        let levels = Levels::new(seed as u32);
        let (w0, h0, l0) = (w.clone(), host.clone(), levels.clone());

        let got = place_unit(&mut w, &mut host, &levels, u, room, x, y, exact, alt);

        // Determinism: the same call on the same state gives the same state.
        let (mut w2, mut h2) = (w0.clone(), h0.clone());
        prop_assert_eq!(place_unit(&mut w2, &mut h2, &l0, u, room, x, y, exact, alt), got);
        prop_assert_eq!(&w2, &w);
        prop_assert_eq!(&h2, &host);
        // Placement consumes no RNG draw and leaves the DRLG unchanged.
        prop_assert_eq!(&levels, &l0);

        if !has_path {
            prop_assert_eq!(got, Err(PlaceError::NoPath));
            prop_assert_eq!(&w, &w0);
            prop_assert!(host.log.is_empty());
            return Ok(());
        }
        // Rule 2 (edge case 7: the null-hint lookup finds nothing).
        let resolved = room.or_else(|| w0.cell_room(w0.units[u].room, x, y));
        let Some(r0) = resolved else {
            prop_assert_eq!(got, Ok(false));
            prop_assert!(w.teleports.is_empty() && host.log.is_empty());
            return Ok(());
        };
        // Rule 3.
        let (room_at, p) = if exact {
            (Some(r0), Point::new(x, y))
        } else {
            let a = RefArgs { size, mask: 0x1C09, field: None, fallback: false, d: 50, k: 1 };
            ref_nearest(&w0, Some(r0), Point::new(x, y), &a)
        };
        match room_at {
            None => {
                prop_assert_eq!(got, Ok(false));
                prop_assert!(w.teleports.is_empty() && host.log.is_empty());
            }
            Some(r) => {
                prop_assert_eq!(got, Ok(true));
                prop_assert_eq!(&w.teleports, &vec![(u, r, p.x, p.y)]);
                if !exact {
                    prop_assert_eq!(w.size_query(r, p.x, p.y, size, 0x1C09), 0);
                    prop_assert_eq!(w.cell_room(Some(r), p.x, p.y), Some(r));
                }
                prop_assert_eq!(&host.log, &place_events(player, u, r, p, alt));
            }
        }
    }
}

// ---- §11 -----------------------------------------------------------------

proptest! {
    #![proptest_config(config(256))]

    /// §11: one `spawn_room` call, whose draws are the only ones (count
    /// and order of the level seed); then the free point of rule 3 or the
    /// fatal assert; entry and level warp place where the spawn point is.
    #[test]
    fn spawn_entry_warp_draws_and_points(
        seed in any::<u64>(),
        density in 0u64..101,
        tx in -3i32..12, ty in -3i32..12,
        has_spawn in prop::bool::weighted(0.9),
        draws in 0u32..6,
        size in 0i32..4,
        level in 0u32..140, tile_index in 0u32..4, act in 0u8..5,
        which in 0u8..3,
    ) {
        let mut w = World::any(seed, density, false).crowd(seed, tx * 5 + 3, ty * 5 + 3);
        let nr = w.rooms.len();
        let sroom = (seed % nr as u64) as usize;
        w.units.push(FakeUnit { room: None, pos: Point::new(0, 0), size, has_path: true });
        let u = 0;
        let mut levels = Levels::new((seed >> 32) as u32);
        levels.spawn = has_spawn.then_some((sroom, tx, ty));
        levels.spawn_draws = draws;
        levels.start_level = level;
        let mut host = Host { players: vec![u], ..Host::default() };
        let (w0, h0, l0) = (w.clone(), host.clone(), levels.clone());

        let run = |w: &mut World, h: &mut Host, l: &mut Levels| -> Result<String, PlaceError> {
            match which {
                0 => level_spawn_point(&*w, l, Some(act), level, tile_index, size).map(|r| format!("{r:?}")),
                1 => game_entry(w, h, l, u, act).map(|r| format!("{r:?}")),
                _ => level_warp_place(w, h, l, u, act, level, tile_index).map(|r| format!("{r:?}")),
            }
        };
        let got = run(&mut w, &mut host, &mut levels);
        let (mut w2, mut h2, mut l2) = (w0.clone(), h0.clone(), l0.clone());
        prop_assert_eq!(run(&mut w2, &mut h2, &mut l2), got.clone());
        prop_assert_eq!((&w2, &h2, &l2), (&w, &host, &levels));

        // Draws: exactly one spawn_room call's, in the seed's order.
        let tile = if which == 1 { 0 } else { tile_index };
        prop_assert_eq!(&levels.spawn_calls, &vec![(act, level, tile)]);
        prop_assert_eq!(&levels.rng.draws, &first_draws((seed >> 32) as u32, draws as usize));

        let a = RefArgs { size, mask: 0x1C09, field: None, fallback: false, d: 50, k: 1 };
        let start = Point::new(tx * 5 + 3, ty * 5 + 3);
        let spawn = has_spawn.then(|| ref_nearest(&w0, Some(sroom), start, &a));
        match spawn {
            None => {
                // §11: game entry without a spawn room is a fatal assert;
                // the level warp does nothing.
                match which {
                    0 => prop_assert_eq!(got.unwrap(), "None"),
                    1 => prop_assert_eq!(got, Err(PlaceError::NoSpawnRoom)),
                    _ => prop_assert_eq!(got.unwrap(), "false"),
                }
                prop_assert!(w.teleports.is_empty() && w.added.is_empty() && w.switched.is_empty() && host.log.is_empty());
            }
            Some((None, _)) => {
                prop_assert_eq!(got, Err(PlaceError::SpawnNotFree));
            }
            Some((Some(r), p)) => {
                prop_assert_eq!(w0.size_query(r, p.x, p.y, size, 0x1C09), 0);
                match which {
                    0 => prop_assert_eq!(got.unwrap(), format!("{:?}", Some((r, p)))),
                    1 => {
                        prop_assert_eq!(got.unwrap(), "true");
                        prop_assert_eq!(&w.added, &vec![(u, r, p.x, p.y)]);
                        prop_assert_eq!(&w.switched, &vec![(u, r)]);
                        prop_assert!(w.teleports.is_empty());
                        prop_assert_eq!(&host.log, &vec![
                            Ev::Send(u, reveal_msg(r)),
                            Ev::Send(u, PlaceMessage::ReassignPlayer { unit: u, x: p.x as u16, y: p.y as u16, flag: 1 }),
                            Ev::Send(u, PlaceMessage::GameEntryDone),
                        ]);
                    }
                    _ => {
                        // The second search (unit size = size) finds the
                        // same point (§11 last paragraph).
                        prop_assert_eq!(got.unwrap(), "true");
                        prop_assert_eq!(&w.teleports, &vec![(u, r, p.x, p.y)]);
                        prop_assert_eq!(&host.log, &place_events(true, u, r, p, false));
                    }
                }
            }
        }
    }
}

// ---- §12 -----------------------------------------------------------------

#[derive(Clone, Debug)]
struct Drlg {
    rect: TileRect,
    recs: Vec<(u32, u8, LvlWarp)>,
    added: Vec<(u8, u32, u32, i32, i32)>,
}

impl WarpTileView for Drlg {
    type DrlgRoom = u8;
    fn tile_rect(&self, _room: u8) -> TileRect {
        self.rect
    }
    fn lvlwarp(&self, _room: u8, slot: u32, letter: u8) -> Option<LvlWarp> {
        self.recs
            .iter()
            .find(|(s, l, _)| *s == slot && *l == letter)
            .map(|r| r.2)
    }
    fn add_preset_unit(&mut self, _room: u8, ty: u8, class: u32, mode: u32, x: i32, y: i32) {
        self.added.push((ty, class, mode, x, y));
    }
}

/// Table i32 values, biased to the overflow edges.
fn table_i32() -> impl Strategy<Value = i32> {
    prop_oneof![
        any::<i32>(),
        i32::MAX - 400..=i32::MAX,
        i32::MIN..=i32::MIN + 400,
        i32::MAX - 8..=i32::MAX,
        i32::MIN..=i32::MIN + 8,
        -100i32..100,
    ]
}

fn lvlwarp() -> impl Strategy<Value = (u32, u8, LvlWarp)> {
    (
        0u32..64,
        prop::sample::select(vec![b'l', b'r']),
        any::<u32>(),
        table_i32(),
        table_i32(),
    )
        .prop_map(|(slot, letter, id, offset_x, offset_y)| {
            (
                slot,
                letter,
                LvlWarp {
                    id,
                    offset_x,
                    offset_y,
                },
            )
        })
}

proptest! {
    #![proptest_config(config(512))]

    /// §12.1 on random lvlwarp tables: never panics; the error iff no
    /// record; nothing on the far edge; else the one preset of rule 3.
    #[test]
    fn warp_tile_preset_random_tables(
        recs in prop::collection::vec(lvlwarp(), 0..8),
        rx in -5000i32..5000, ry in -5000i32..5000,
        rw in 0i32..64, rh in 0i32..64,
        dx in -2i32..66, dy in -2i32..66,
        t in 0u32..16, v in any::<u32>(),
    ) {
        let rect = TileRect { x: rx, y: ry, w: rw, h: rh };
        let mut d = Drlg { rect, recs: recs.clone(), added: Vec::new() };
        let got = warp_tile_preset(&mut d, 0, t, rx + dx, ry + dy, v);
        let letter = if t == 10 { b'l' } else { b'r' };
        let slot = warp_slot(v);
        prop_assert_eq!(slot, (v >> 20) & 0x3F);
        match recs.iter().find(|(s, l, _)| *s == slot && *l == letter) {
            None => {
                prop_assert_eq!(got, Err(PlaceError::NoLvlWarp { slot }));
                prop_assert!(d.added.is_empty());
            }
            Some(&(_, _, rec)) if dx == rw || dy == rh => {
                prop_assert_eq!(got, Ok(false));
                prop_assert!(d.added.is_empty());
                let _ = rec;
            }
            Some(&(_, _, rec)) => {
                prop_assert_eq!(got, Ok(true));
                prop_assert_eq!(&d.added, &vec![(
                    5, rec.id, 0,
                    (5 * dx).wrapping_add(rec.offset_x),
                    (5 * dy).wrapping_add(rec.offset_y),
                )]);
            }
        }
    }

    /// §12.2 on random warp destinations and lvlwarp walk-outs: never
    /// panics, no draw, the outcome of the rules.
    #[test]
    fn warp_player_random_tables(
        seed in any::<u64>(),
        density in 0u64..101,
        has_dest in prop::bool::weighted(0.9),
        px in -20i32..70, py in -20i32..70,
        ewx in table_i32(), ewy in table_i32(),
        src in any::<u32>(),
        level in prop::sample::select(vec![1u32, 2, 73, 100, 118, 128, 132, 0, u32::MAX]),
        gate in prop::sample::select(vec![0u32, 1, u32::MAX]),
        size in 0i32..6, has_path in prop::bool::weighted(0.9),
        life in any::<u8>(),
    ) {
        let mut w = World::any(seed, density, false).crowd(seed, px, py);
        let nr = w.rooms.len();
        let droom = (seed % nr as u64) as usize;
        w.units.push(FakeUnit { room: Some(0), pos: Point::new(0, 0), size, has_path });
        let u = 0;
        let mut levels = Levels::new(seed as u32);
        levels.warp = has_dest.then_some(WarpDestination {
            room: droom,
            point: Point::new(px, py),
            exit_walk_x: ewx,
            exit_walk_y: ewy,
            source_level: src,
            level,
        });
        levels.gate = gate;
        let mut host = Host { players: vec![u], life, ..Host::default() };
        let (w0, l0) = (w.clone(), levels.clone());

        let got = warp_player(&mut w, &mut host, &levels, u, 0, 7);
        prop_assert_eq!(&levels, &l0, "no draw");

        let a = RefArgs { size, mask: 0x1C09, field: None, fallback: true, d: 50, k: 1 };
        let gated = [73, 100, 118, 128, 132].contains(&level) && gate != 0;
        let want = if !has_dest {
            Ok(WarpOutcome::NoDestination)
        } else {
            match ref_nearest(&w0, Some(droom), Point::new(px, py), &a) {
                (None, _) => Ok(WarpOutcome::NoFreePoint),
                _ if gated => Ok(WarpOutcome::QuestGate),
                _ if !has_path => Err(PlaceError::NoPath),
                (Some(_), p) => {
                    let b = RefArgs { fallback: false, ..a };
                    match ref_nearest(&w0, Some(droom), p, &b) {
                        (None, _) => Ok(WarpOutcome::NotPlaced),
                        (Some(r), q) => {
                            prop_assert_eq!(&w.teleports, &vec![(u, r, q.x, q.y)]);
                            let (tx, ty) = (p.x.wrapping_add(ewx), p.y.wrapping_add(ewy));
                            let mut ev = place_events(true, u, r, q, false);
                            ev.push(Ev::Walk(u, tx, ty));
                            ev.push(Ev::Send(u, PlaceMessage::PlayerStop {
                                unit: u, a: 1, x: tx as u16, y: ty as u16, b: 0, life_pct: life,
                            }));
                            prop_assert_eq!(&host.log, &ev);
                            Ok(WarpOutcome::Arrived)
                        }
                    }
                }
            }
        };
        prop_assert_eq!(got, want);
        if !matches!(got, Ok(WarpOutcome::Arrived)) {
            prop_assert!(w.teleports.is_empty());
        }
    }
}

//! The spawn point in a coordinate rectangle, the
//! placement search and creation call with its
//! wrappers, and the ring walk.

use super::seams::{Alloc, PopHost};
use super::{spawn, CoordRect, Ctx};
use crate::rng::Seed;
use crate::units::{RoomId, UnitId};

/// Creation flags (u16 at record +0x24).
pub mod flags {
    /// Probe only, no unit.
    pub const PROBE: u16 = 0x01;
    /// Skip per-class extras.
    pub const NO_EXTRAS: u16 = 0x02;
    /// Tentacle offset set 0 instead of 1.
    pub const TENTACLE_SET0: u16 = 0x04;
    /// Not counted in region spawned; monster flag 2 instead.
    pub const NO_COUNT: u16 = 0x08;
    /// Allocate with the caller's GUID.
    pub const GUID: u16 = 0x20;
    /// No party minions.
    pub const NO_PARTY: u16 = 0x40;
    /// Ignore collision at placement.
    pub const NO_COLLISION: u16 = 0x80;
}

/// Point tries of the spawn-point search.
pub const POINT_TRIES: u32 = 20;
/// Default collision mask.
pub const MASK_DEFAULT: u16 = 0x3C01;

/// The 0x28-byte spawn record (D2MOO `D2UnkMonCreateStrc`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpawnReq {
    pub room: Option<RoomId>,
    pub cl: Option<CoordRect>,
    pub class: i32,
    pub mode: u8,
    pub guid: Option<u32>,
    pub x: i32,
    pub y: i32,
    pub r: i32,
    pub flags: u16,
}

/// What returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placed {
    /// Null.
    Failed,
    /// Probe (flags & 1): a point was accepted, nothing created.
    Probe,
    Unit(UnitId),
}

impl Placed {
    pub fn unit(self) -> Option<UnitId> {
        match self {
            Placed::Unit(u) => Some(u),
            _ => None,
        }
    }
}

/// Whether `class` with `monstats2` `spawnCol` `spawn_col` places only on a
/// water tile of its room (the frog demons): `spawnCol` 1, except the
/// tentacles (258–263) and class 153, which place as land monsters.
pub fn water_only(class: i32, spawn_col: u8) -> bool {
    spawn_col == 1 && !(258..=263).contains(&class) && class != 153
}

/// The collision mask of a `spawnCol` value (a jump table in the base
/// game).
pub fn spawn_mask(spawn_col: u8) -> u16 {
    match spawn_col {
        1 => 0x1C0,
        2 => 0x3F11,
        3 => 0,
        _ => MASK_DEFAULT,
    }
}

/// The ring walk. Four room-seed draws per ring, then up to 8d
/// tests (one when d = 0). `r < 0` → one ring d = 0; `r = 0` → failure
/// with no draws; `r ≥ 1` → rings 3, 6, …, 3r.
pub fn ring_search(
    seed: &mut Seed,
    x0: i32,
    y0: i32,
    r: i32,
    mut test: impl FnMut(i32, i32) -> bool,
) -> Option<(i32, i32)> {
    let rings: Vec<i32> = match r {
        0 => return None,
        r if r < 0 => vec![0],
        r => (1..=r).map(|i| 3 * i).collect(),
    };
    for d in rings {
        let p = seed.step() & 1;
        let q = seed.roll(d) as i32;
        let sx = seed.step() & 1;
        let sy = seed.step() & 1;
        let (mut ox, mut oy, mut dir) = if p == 1 {
            (d, q, (0, 1))
        } else {
            (q, d, (1, 0))
        };
        if sx == 1 {
            ox = -ox;
        }
        if sy == 1 {
            oy = -oy;
        }
        let (mut x, mut y) = (x0 + ox, y0 + oy);
        let (l, rr, t, b) = (x0 - d, x0 + d, y0 - d, y0 + d);
        let n = if d == 0 { 1 } else { 8 * d };
        for _ in 0..n {
            // Corner turns: a later match overrides an earlier one.
            if (x, y) == (l, t) {
                dir = (1, 0);
            }
            if (x, y) == (rr, t) {
                dir = (0, 1);
            }
            if (x, y) == (rr, b) {
                dir = (-1, 0);
            }
            if (x, y) == (l, b) {
                dir = (0, -1);
            }
            if l == rr && t == b {
                dir = (0, 0);
            }
            x += dir.0;
            y += dir.1;
            if test(x, y) {
                return Some((x, y));
            }
        }
    }
    None
}

/// Special footprints
/// keyed on `BaseId`.
fn footprint_ok<H: PopHost + ?Sized>(
    host: &H,
    base: Option<i32>,
    room: RoomId,
    x: i32,
    y: i32,
) -> bool {
    let (px, py, mask) = match base {
        Some(206) => (x, y + 3, MASK_DEFAULT),
        Some(228) => (x, y + 2, MASK_DEFAULT),
        Some(298) => return true,
        Some(334) => (x - 2, y - 2, 0x1C0),
        Some(528) => (x + 2, y + 4, MASK_DEFAULT),
        _ => return true,
    };
    match host.room_at(room, px, py) {
        Some(r) => !host.collides(r, px, py, 2, mask),
        None => false,
    }
}

/// A water point for frog demons.
fn water_point<H: PopHost + ?Sized>(host: &mut H, room: RoomId) -> Option<(i32, i32)> {
    let list = host.tile_records(room);
    let n = list.len() as i32;
    if n == 0 {
        return None;
    }
    let mut s = host.room_seed(room).roll(n) as i32;
    if s == 0 {
        s = 1;
    }
    if n == 1 {
        // Original bug: with n = 1 the
        // loop tests exactly one record, index 1, one past the end of the
        // list (the next 0x30 bytes in memory), then stops; index 0 is
        // never tested. Those bytes are not tile data here, so no point.
        return None;
    }
    const NEAR: [(i32, i32); 4] = [(0, -3), (3, 0), (0, 3), (-3, 0)];
    let mut i = s;
    while i != s - 1 {
        let rec = list[i as usize];
        if rec.water {
            let x = rec.x * 5 + 3;
            let y = rec.y * 5 + 3;
            if !host.mask_at(room, x, y, 0x100)
                && NEAR
                    .iter()
                    .any(|&(dx, dy)| !host.collides(room, x + dx, y + dy, 2, 0x1C09))
            {
                return Some((x, y));
            }
        }
        i = (i + 1) % n;
    }
    None
}

pub fn place<H: PopHost + ?Sized>(cx: &mut Ctx<'_, H>, req: SpawnReq) -> Placed {
    let Some((room, x, y)) = search(cx, &req) else {
        return Placed::Failed;
    };
    if req.flags & flags::PROBE != 0 {
        return Placed::Probe;
    }
    create(cx, req, room, x, y).map_or(Placed::Failed, Placed::Unit)
}

/// The placement steps of the monster creation, without the creation: the accepted point and
/// its room (the room-seed draws of the search are made), or `None`.
pub fn search<H: PopHost + ?Sized>(
    cx: &mut Ctx<'_, H>,
    req: &SpawnReq,
) -> Option<(RoomId, i32, i32)> {
    let t = cx.tables;
    let (Some(mon), Some(m2)) = (t.mon(req.class), t.mon2(req.class)) else {
        return None;
    };
    let mask = spawn_mask(m2.spawn_col);
    let mut room = req.room?;
    let (bounds, index) = match req.cl {
        Some(cl) => (cl.subtiles(), Some(cl.index)),
        None => {
            let b = cx.host.room_box(room);
            ([b.x, b.y, b.x + b.width, b.y + b.height], None)
        }
    };
    let point = if water_only(req.class, m2.spawn_col) {
        let p = water_point(cx.host, room);
        if let Some((x, y)) = p {
            room = cx.host.room_at(room, x, y)?;
        }
        p
    } else {
        let base = Some(i32::from(mon.base_id));
        let size = i32::from(m2.size_x);
        let host = &mut *cx.host;
        let mut seed = *host.room_seed(room);
        let found = ring_search(&mut seed, req.x, req.y, req.r, |x, y| {
            let [l, tp, r, b] = bounds;
            if !(l <= x && x < r && tp <= y && y < b) {
                return false;
            }
            if index.is_some_and(|i| host.coord_index_at(room, x, y) != i) {
                return false;
            }
            if !footprint_ok(&*host, base, room, x, y) {
                return false;
            }
            req.flags & flags::NO_COLLISION != 0 || !host.collides(room, x, y, size, mask)
        });
        *host.room_seed(room) = seed;
        found
    };
    point
        .filter(|&(x, y)| x != 0 && y != 0)
        .map(|(x, y)| (room, x, y))
}

/// The creation call at an accepted point.
fn create<H: PopHost + ?Sized>(
    cx: &mut Ctx<'_, H>,
    req: SpawnReq,
    room: RoomId,
    x: i32,
    y: i32,
) -> Option<UnitId> {
    let t = cx.tables;
    let mon = t.mon(req.class)?;
    let unit = cx.host.allocate_monster(
        Alloc {
            class: req.class,
            x,
            y,
            room,
            mode: req.mode,
            guid: req.guid.filter(|_| req.flags & flags::GUID != 0),
        },
        cx.state,
    )?;
    let nc = req.flags & flags::NO_COUNT != 0 || mon.never_count;
    let level = cx.host.room_level(room);
    if cx.state.regions.count_spawn(level, nc) {
        cx.host.set_monster_flag(unit, 2);
    }
    cx.host.set_coord_record(unit, req.cl, room, x, y);
    let (align, flag) = match mon.align {
        1 => (2, true),
        2 => (1, false),
        _ => (0, false),
    };
    cx.host.set_alignment(unit, align);
    if flag {
        cx.host.set_unit_flags(unit, 0x20000);
    }
    if req.flags & flags::NO_EXTRAS == 0 {
        cx.host.class_extras(unit, cx.state);
    }
    cx.host.init_monster(unit, cx.state);
    if req.flags & flags::NO_PARTY == 0 {
        spawn::party(cx, unit, req.class, req.flags);
    }
    Some(unit)
}

/// (room, x, y; no cl).
#[allow(clippy::too_many_arguments)]
pub fn place_at<H: PopHost + ?Sized>(
    cx: &mut Ctx<'_, H>,
    room: RoomId,
    cl: Option<CoordRect>,
    x: i32,
    y: i32,
    class: i32,
    mode: u8,
    r: i32,
    flags: u16,
) -> Placed {
    place(
        cx,
        SpawnReq {
            room: Some(room),
            cl,
            class,
            mode,
            guid: None,
            x,
            y,
            r,
            flags,
        },
    )
}

/// (cl, near a unit) / (near a unit, no cl):
/// Around the unit's position in the room it stands in.
pub fn place_near<H: PopHost + ?Sized>(
    cx: &mut Ctx<'_, H>,
    cl: Option<CoordRect>,
    unit: UnitId,
    class: i32,
    mode: u8,
    r: i32,
    flags: u16,
) -> Placed {
    let room = cx.host.unit_room(unit);
    let (x, y) = cx.host.unit_position(unit);
    place(
        cx,
        SpawnReq {
            room,
            cl,
            class,
            mode,
            guid: None,
            x,
            y,
            r,
            flags,
        },
    )
}

/// A spawn point in `cl` (or the room box), checked by a
/// probe placement.
pub fn spawn_point<H: PopHost + ?Sized>(
    cx: &mut Ctx<'_, H>,
    room: RoomId,
    cl: Option<CoordRect>,
    class: i32,
    warp_check: bool,
) -> Option<(i32, i32)> {
    let (left, top, w, h) = match cl {
        Some(c) => {
            let [x0, y0, x1, y1] = c.subtiles();
            (x0 + 1, y0 + 1, x1 - (x0 + 1), y1 - (y0 + 1))
        }
        None => {
            let b = cx.host.room_box(room);
            (b.x + 1, b.y + 1, b.width - 1, b.height - 1)
        }
    };
    let warp_dist = if warp_check {
        let lvl = cx.host.room_level(room);
        i64::from(cx.tables.level(lvl).map_or(0, |l| l.warp_dist))
    } else {
        0
    };
    for _ in 0..POINT_TRIES {
        let seed = cx.host.room_seed(room);
        let x = seed.roll(w) as i32 + left;
        let y = seed.roll(h) as i32 + top;
        if warp_check && near_warp(cx, room, x, y, warp_dist) {
            continue;
        }
        if let Some(c) = cl {
            if cx.host.coord_index_at(room, x, y) != c.index {
                continue;
            }
        }
        if place_at(cx, room, cl, x, y, class, 1, -1, flags::PROBE) == Placed::Probe {
            return Some((x, y));
        }
    }
    None
}

/// Inside `WarpDist` (squared) of a warp point
/// or of the level's kind-11 spawn location.
fn near_warp<H: PopHost + ?Sized>(
    cx: &mut Ctx<'_, H>,
    room: RoomId,
    x: i32,
    y: i32,
    dist: i64,
) -> bool {
    let close = |px: i32, py: i32| {
        let (dx, dy) = (i64::from(x - px), i64::from(y - py));
        dx * dx + dy * dy < dist
    };
    if cx
        .host
        .warp_points(room)
        .iter()
        .any(|&(px, py)| close(px, py))
    {
        return true;
    }
    match cx.host.spawn_location(room, 11) {
        Some((tx, ty)) if tx > 0 && ty > 0 => close(tx * 5, ty * 5),
        _ => false,
    }
}

// Spec: specs/world/object-population.md
//! Object population of a room (`0x00552610`): the pre-check and its
//! region counters (§2, §3), the theme gate (§4), the eight `ObjGrp`
//! slots (§5), the placement helpers (§6) and the populate functions
//! `PopulateFn` 1–9 (§7). Rooms, seeds and collision come through
//! [`PopulateWorld`]; objects are allocated with [`super::allocate`]
//! (mode 0, the object's init run inside it, `world/objects-2.md` §22
//! rule 4).

use d2_data::tables::Objgroup;

use crate::rng::Seed;
use crate::units::{RoomId, UnitId};

use super::{
    allocate, oflags, selectable, shrine_pick, ObjectControl, ObjectError, ObjectTables,
    ObjectWorld, Region, TOWNS,
};

/// Collision mask of the placement fits (§6: 0x3F11).
pub const MASK_PLACE: u32 = 0x3F11;
/// Collision mask wall | object | door (§6: 0xC01).
pub const MASK_WALL_OBJECT_DOOR: u32 = 0xC01;
/// The `Gore` limit byte `0x00731BBC` (§5 rule 6).
pub const GORE_LIMIT: u8 = 2;
/// The fallback health shrine class `0x00552AC0` (§7.2).
pub const HEALTH_FALLBACK_CLASS: u16 = 84;
/// Flies (`0x00551150`, §7.6).
pub const FLIES_CLASS: u16 = 103;
/// The probability argument every slot passes (§5).
const P: u32 = 100;

/// Walk direction signs `0x00731B7C` (X) and `0x00731B9C` (Y) (§6).
const WALK_X: [i32; 8] = [-1, 0, 1, -1, 1, -1, 0, 1];
const WALK_Y: [i32; 8] = [-1, -1, -1, 0, 0, 1, 1, 1];
/// The urn / crate class list of fn 1 (A = 4) and fn 5 (§7.1, §7.5).
const URNS: [u16; 5] = [4, 9, 52, 94, 95];

/// The rogue-on-stick point offsets (§7.7, `0x00731EB4` + 8q). The counts
/// the function reads at `0x00731EB8` + 8q are all 0 in 1.14d, so no
/// point is placed (edge case 3).
pub const ROGUE_PATTERNS: [&[(i32, i32)]; 4] = [
    &[(-4, 0), (0, 0), (4, 0), (8, 0), (0, 4), (0, -4)],
    &[(-8, 0), (-4, 0), (0, 0), (4, 0), (8, 0)],
    &[(0, -8), (0, -4), (0, 0), (0, 4), (0, 8)],
    &[
        (-7, -5),
        (-5, -3),
        (-3, -1),
        (0, 0),
        (3, -1),
        (5, -3),
        (7, -5),
    ],
];
/// The point counts as read (`0x00731EB8` + 8q).
pub const ROGUE_COUNTS: [usize; 4] = [0; 4];

/// The room facts the pre-check and the helpers read, gathered by the
/// caller before the population (they do not change during it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomInfo {
    pub room: RoomId,
    /// `0x0061A1B0`: the level of the room's DRLG room (L0, §1).
    pub level: u32,
    /// `0x0061A1F0`: the populated level (0 for a flag-0x800000 room).
    pub populated: u32,
    /// `0x0061A210`: DRLG room flags & 0x30000.
    pub waypoint: bool,
    /// `0x0061ABB0`: room type 1 and outdoor flags bit 0x80.
    pub dirt_path: bool,
    /// `0x00619730`: sub-tile rect {x0, y0, w, h}.
    pub rect: (i32, i32, i32, i32),
}

/// What population needs beyond [`ObjectWorld`]. Provider: the DRLG
/// (seeds, room counts), the path code (collision).
pub trait PopulateWorld: ObjectWorld {
    /// The active room seed (room +0x6C); `None`: no active room.
    fn room_seed(&mut self, room: RoomId) -> Option<&mut Seed>;
    /// `0x0061ABF0(act, level)` → `0x00642BE0`: rooms of `level` in the
    /// act without flag 0x800000 (allocates the level when absent).
    fn populated_room_count(&mut self, act: u8, level: u32) -> i32;
    /// `0x0064D800(room, x, y, sx, sy, mask)`: both sizes ≤ 1 → the point
    /// query, else the centred box query (`sim/path-placement.md` §4).
    fn box_query(&self, room: RoomId, x: i32, y: i32, sx: u32, sy: u32, mask: u32) -> u32;
    /// Unit +0x04 := `class` (§7.2 rule 4.3).
    fn set_unit_class(&mut self, unit: UnitId, class: u16);
}

/// What one populate call did: the objects it allocated, in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Populated {
    pub objects: Vec<UnitId>,
}

/// A step on the room seed (§5, Randomness R); a room without an active
/// seed is an error.
fn room_step<W: PopulateWorld>(w: &mut W, room: RoomId) -> Result<u32, ObjectError> {
    w.room_seed(room)
        .map(|s| s.step())
        .ok_or(ObjectError::NoActiveRoom(room))
}

fn room_roll<W: PopulateWorld>(w: &mut W, room: RoomId, n: i32) -> Result<i32, ObjectError> {
    w.room_seed(room)
        .map(|s| s.roll(n) as i32)
        .ok_or(ObjectError::NoActiveRoom(room))
}

fn region(ctl: &mut ObjectControl, level: u32) -> Result<&mut Region, ObjectError> {
    ctl.regions
        .get_mut(level as usize)
        .and_then(Option::as_mut)
        .ok_or(ObjectError::NoRow {
            table: "object regions",
            row: level,
        })
}

// ------------------------------------------------------------------ §2

/// Want health (`0x00547330`).
pub fn want_health(r: &Region) -> bool {
    r.w08 > 0 && r.counted.wrapping_mul(128) / r.w08 > 96 && r.health == 0
}
/// Shrine cap (`0x00547360`).
pub fn shrine_cap(r: &Region) -> bool {
    r.shrines == 10 || r.shrines > r.w08 / 8
}
/// Well cap (`0x00547300`).
pub fn well_cap(r: &Region) -> bool {
    r.wells == 4 || r.wells > r.w08 / 8
}
/// Shrine spacing (`0x00547430`): passes when no recorded point is within
/// 50 on either axis (edge case 4).
pub fn shrine_spaced(r: &Region, x: i32, y: i32) -> bool {
    spaced(
        &r.shrine_points[..r.shrines.clamp(0, 10) as usize],
        x,
        y,
        50,
    )
}
/// Well spacing (`0x005473A0`).
pub fn well_spaced(r: &Region, x: i32, y: i32) -> bool {
    spaced(&r.well_points[..r.wells.clamp(0, 4) as usize], x, y, 100)
}
fn spaced(pts: &[(i32, i32)], x: i32, y: i32, d: i32) -> bool {
    !pts.iter().any(|&(sx, sy)| {
        x.wrapping_sub(sx).wrapping_abs() < d || y.wrapping_sub(sy).wrapping_abs() < d
    })
}
/// Record a shrine point (`0x00547490`).
pub fn record_shrine(r: &mut Region, at: (i32, i32)) {
    if (0..10).contains(&r.shrines) {
        r.shrine_points[r.shrines as usize] = at;
        r.shrines += 1;
    }
}
/// Record a well point (`0x00547400`).
pub fn record_well(r: &mut Region, at: (i32, i32)) {
    if (0..4).contains(&r.wells) {
        r.well_points[r.wells as usize] = at;
        r.wells += 1;
    }
}

// ------------------------------------------------------------------ §1

/// `0x00552610(game, room)` (§1): the room's random objects.
pub fn populate_room<W: PopulateWorld>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    info: &RoomInfo,
) -> Result<Populated, ObjectError> {
    let lv = info.level;
    if lv == 0 || lv as usize >= t.levels.len() {
        return Err(ObjectError::NoRow {
            table: "levels",
            row: lv,
        });
    }
    let mut out = Populated::default();
    if !precheck(ctl, t, w, info)? {
        return Ok(out);
    }
    let rec = &t.levels[lv as usize];
    let grp = [
        rec.objgrp0,
        rec.objgrp1,
        rec.objgrp2,
        rec.objgrp3,
        rec.objgrp4,
        rec.objgrp5,
        rec.objgrp6,
        rec.objgrp7,
    ];
    let prb = [
        rec.objprb0,
        rec.objprb1,
        rec.objprb2,
        rec.objprb3,
        rec.objprb4,
        rec.objprb5,
        rec.objprb6,
        rec.objprb7,
    ];
    let pl = info.populated;
    for i in 0..8 {
        let g = grp[i];
        let mut r = room_step(w, info.room)? % 100;
        if want_health(region(ctl, pl)?) && t.object(u16::from(g))?.subclass != 0 {
            r = 100;
        }
        if g == 0 || r > u32::from(prb[i]) {
            continue;
        }
        let Some(group) = t.objgroup.get(usize::from(g)) else {
            return Ok(out);
        };
        let r2 = room_step(w, info.room)? % 100;
        let mut acc = 0u32;
        for (id, density, prob) in members(group) {
            if id == 0 {
                break;
            }
            acc += u32::from(prob);
            let class = u16::try_from(id).map_err(|_| ObjectError::NoRow {
                table: "objects",
                row: id,
            })?;
            let row = t.object(class)?;
            if r2 < acc && row.gore <= GORE_LIMIT {
                let f = row.populatefn;
                if f >= 10 {
                    return Err(ObjectError::PopulateFn(f));
                }
                if f == 0 {
                    break;
                }
                if density >= 128 {
                    return Err(ObjectError::Density(density));
                }
                run(ctl, t, w, info, f, density, class, &mut out)?;
                break;
            }
        }
    }
    Ok(out)
}

/// The eight members {`ID`, `DENSITY`, `PROB`} of a group record.
fn members(g: &Objgroup) -> [(u32, u8, u8); 8] {
    [
        (g.id0, g.density0, g.prob0),
        (g.id1, g.density1, g.prob1),
        (g.id2, g.density2, g.prob2),
        (g.id3, g.density3, g.prob3),
        (g.id4, g.density4, g.prob4),
        (g.id5, g.density5, g.prob5),
        (g.id6, g.density6, g.prob6),
        (g.id7, g.density7, g.prob7),
    ]
}

/// The populate table `0x00731D00` (§5).
#[allow(clippy::too_many_arguments)]
fn run<W: PopulateWorld>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    info: &RoomInfo,
    f: u8,
    d: u8,
    class: u16,
    out: &mut Populated,
) -> Result<(), ObjectError> {
    let mut cx = Cx {
        ctl,
        t,
        w,
        info,
        out,
    };
    match f {
        1 => cx.fn1(i32::from(d), class),
        2 => cx.fn2(class).map(drop),
        3 => cx.fn3(i32::from(d), class).map(drop),
        4 => cx.fn4(i32::from(d)),
        5 => cx.fn5(i32::from(d), class),
        6 => cx.fn6(i32::from(d), class),
        7 => cx.fn7(class),
        8 => cx.fn8(i32::from(d), class).map(drop),
        9 => cx.fn9(i32::from(d), class).map(drop),
        _ => Ok(()),
    }
}

// ------------------------------------------------------------------ §3, §4

/// `0x00552560` (§3): 0 (`false`) at the first failing rule.
fn precheck<W: PopulateWorld>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    info: &RoomInfo,
) -> Result<bool, ObjectError> {
    if info.waypoint {
        return Ok(false);
    }
    let l = info.populated;
    if l == 0 || info.dirt_path || l as usize >= t.levels.len() || TOWNS.contains(&l) {
        return Ok(false);
    }
    let act = region(ctl, l)?.act;
    if region(ctl, l)?.w08 == 0x7FFF_FFFF {
        let total = w.populated_room_count(act, l);
        region(ctl, l)?.w08 = total;
    }
    region(ctl, l)?.counted += 1;
    let themes = t.levels[info.level as usize].themes;
    if themes != 0 && theme_gate(ctl, l, themes)? {
        return Ok(false);
    }
    Ok(true)
}

/// `0x00552400` (§4): draws, and runs a theme only when its number is
/// below the set-bit count (never with the 1.14d data, edge case 2).
fn theme_gate(ctl: &mut ObjectControl, l: u32, themes: u32) -> Result<bool, ObjectError> {
    let r = *region(ctl, l)?;
    // +0x0C theme count has no writer (always 0, open question 4).
    let theme_count = 0i32;
    if r.w08 < theme_count.wrapping_mul(10) {
        return Ok(false);
    }
    let roll = ctl.seed.step() % 100;
    let mut p = 0;
    if r.counted > r.w08 / 2 {
        p += 5;
    }
    if r.counted > r.w08 - r.w08 / 4 {
        p += 5;
    }
    if roll >= p + 12 {
        return Ok(false);
    }
    let list: Vec<u32> = (0..7)
        .filter(|k| themes & (1 << k) != 0)
        .map(|k| k + 1)
        .collect();
    let n = list.len() as u32;
    let j = ctl.seed.roll(n as i32);
    if j >= n {
        return Ok(false);
    }
    let theme = list[j as usize];
    if theme >= n {
        return Ok(false);
    }
    match theme {
        // Table `0x00731E68`: 0 and 3 inactive.
        0 | 3 => Ok(false),
        // `0x00551FF0`: returns 1, places nothing.
        1 => Ok(true),
        // `0x00552000`, `0x00552140`, `0x00552200`: bodies not specified
        // (open question 5); unreachable with the 1.14d data.
        t => Err(ObjectError::Theme(t)),
    }
}

// ------------------------------------------------------------------ §6, §7

struct Cx<'a, W> {
    ctl: &'a mut ObjectControl,
    t: &'a ObjectTables,
    w: &'a mut W,
    info: &'a RoomInfo,
    out: &'a mut Populated,
}

/// Low 16 bits as compared against the signed bounds (§6).
fn lo16(v: i32) -> i32 {
    v & 0xFFFF
}

/// Which filter a spot helper runs after Fit C.
#[derive(Clone, Copy)]
enum Filter {
    Shrine,
    Well,
}

impl<W: PopulateWorld> Cx<'_, W> {
    fn room(&self) -> RoomId {
        self.info.room
    }
    fn rect(&self) -> (i32, i32, i32, i32) {
        self.info.rect
    }
    fn c_step(&mut self) -> u32 {
        self.ctl.seed.step()
    }
    fn c_roll(&mut self, n: i32) -> i32 {
        self.ctl.seed.roll(n) as i32
    }
    fn sizes(&self, class: u16) -> Result<(i32, i32), ObjectError> {
        let o = self.t.object(class)?;
        Ok((o.sizex as i32, o.sizey as i32))
    }
    /// "count" := ((w · h) >> 7) · d >> 8.
    fn count(&self, d: i32) -> i32 {
        let (_, _, w, h) = self.rect();
        (w.wrapping_mul(h) >> 7).wrapping_mul(d) >> 8
    }
    fn free(&self, x: i32, y: i32, sx: i32, sy: i32, mask: u32) -> bool {
        self.w
            .box_query(self.room(), x, y, sx as u32, sy as u32, mask)
            == 0
    }

    /// Fit A (`0x00550220`).
    fn fit_a(&self, x: i32, y: i32, sx: i32, sy: i32) -> bool {
        let (x0, y0, w, h) = self.rect();
        let (x, y) = (lo16(x), lo16(y));
        w >= sx + 2
            && h >= sy + 2
            && x > x0 + 1
            && y > y0 + 1
            && x < x0 - sx - 2 + w
            && y < y0 - sy - 2 + h
            && self.free(x, y, sx + 7, sy + 7, MASK_WALL_OBJECT_DOOR)
            && self.free(x, y, sx, sy, MASK_PLACE)
    }
    /// Fit B (`0x005502F0`).
    fn fit_b(&self, x: i32, y: i32, sx: i32, sy: i32) -> bool {
        let (x0, y0, w, h) = self.rect();
        let (x, y) = (lo16(x), lo16(y));
        w >= 2
            && h >= 2
            && x > x0 + 2
            && y > y0 + 2
            && x < x0 - sx + w
            && y < y0 - sy + h
            && self.free(x, y, sx + 2, sy + 2, MASK_PLACE)
    }
    /// Fit C (inside the spot helpers), then the filter.
    fn fit_c(&mut self, x: i32, y: i32, sx: i32, sy: i32, f: Option<Filter>) -> bool {
        let (x0, y0, w, h) = self.rect();
        let (x, y) = (lo16(x), lo16(y));
        let ok = x != 0
            && y != 0
            && x > x0
            && y > y0
            && x < x0 + w - 1
            && y < y0 + h - 1
            && self.free(x, y, sx + 6, sy + 6, MASK_PLACE);
        if !ok {
            return false;
        }
        let l = self.info.populated;
        match f {
            None => true,
            Some(Filter::Shrine) => region(self.ctl, l).is_ok_and(|r| shrine_spaced(r, x, y)),
            Some(Filter::Well) => region(self.ctl, l).is_ok_and(|r| well_spaced(r, x, y)),
        }
    }

    /// Allocation `0x00555230(type 2, class, x, y, …, mode 0)`.
    fn alloc(&mut self, class: u16, x: i32, y: i32) -> Result<Option<UnitId>, ObjectError> {
        let room = self.room();
        let u = allocate(self.ctl, self.t, self.w, room, class, x, y, 0)?;
        if let Some(u) = u {
            self.out.objects.push(u);
        }
        Ok(u)
    }

    /// "sel": flag 0x2 := `Selectable[mode]` of `class` ≠ 0.
    fn sel(&mut self, u: UnitId, class: u16) -> Result<(), ObjectError> {
        let s = selectable(self.t.object(class)?, self.w.mode(u));
        let f = self.w.flags(u);
        self.w.set_flags(
            u,
            if s != 0 {
                f | oflags::SELECTABLE
            } else {
                f & !oflags::SELECTABLE
            },
        );
        Ok(())
    }

    /// Random spot `0x00550380` (control seed).
    fn random_spot(&mut self, class: u16, sx: i32, sy: i32) -> Result<Option<UnitId>, ObjectError> {
        let (x0, y0, w, h) = self.rect();
        if w < 2 || h < 2 {
            return Ok(None);
        }
        for _ in 0..5 {
            let x = x0 + self.c_roll(w - sx - 1);
            let y = y0 + self.c_roll(h - sy - 1);
            if self.fit_c(x, y, sx, sy, None) {
                return self.alloc(class, x, y);
            }
        }
        Ok(None)
    }

    /// Oriented spot `0x00550A30` (room seed).
    fn oriented_spot(
        &mut self,
        class: u16,
        sx: i32,
        sy: i32,
        orientation: u8,
        f: Filter,
    ) -> Result<Option<UnitId>, ObjectError> {
        let (x0, y0, w, h) = self.rect();
        if w < sx + 2 || h < sy + 2 {
            return Ok(None);
        }
        let room = self.room();
        for _ in 0..5 {
            let (x, y) = match orientation {
                1 => {
                    let x = x0 + w / 4 + room_roll(self.w, room, w / 2)?;
                    room_step(self.w, room)?;
                    (x, y0 + 1)
                }
                2 => {
                    room_step(self.w, room)?;
                    let y = y0 + h / 4 + room_roll(self.w, room, h / 2)?;
                    (x0 + 1, y)
                }
                _ => {
                    let x = x0 + room_roll(self.w, room, w - sx - 1)?;
                    (x, y0 + room_roll(self.w, room, h - sy - 1)?)
                }
            };
            if self.fit_c(x, y, sx, sy, Some(f)) {
                return self.alloc(class, x, y);
            }
        }
        Ok(None)
    }

    /// Spread spot `0x00550540` (room seed).
    fn spread_spot(
        &mut self,
        class: u16,
        sx: i32,
        sy: i32,
        f: Filter,
    ) -> Result<Option<UnitId>, ObjectError> {
        let (x0, y0, w, h) = self.rect();
        if w < 2 || h < 2 || w <= sx || h <= sy {
            return Ok(None);
        }
        let room = self.room();
        for _ in 0..5 {
            let x = x0 + room_roll(self.w, room, w - sx - 1)?;
            let y = y0 + room_roll(self.w, room, h - sy - 1)?;
            if self.fit_c(x, y, sx, sy, Some(f)) {
                return self.alloc(class, x, y);
            }
        }
        Ok(None)
    }

    /// Fn 1 (`0x00550C20`, §7.1).
    fn fn1(&mut self, d: i32, a: u16) -> Result<(), ObjectError> {
        let (sx, sy) = self.sizes(a)?;
        if self.c_step() % 100 > P {
            return Ok(());
        }
        let (list, mut tries, b, s, fit_a): (&[u16], i32, i32, i32, bool) = match a {
            3 => (&[3, 28], 18, 5, 5, true),
            1 | 79 => (&[79, 53, 1], 12, 5, 5, true),
            4 => (&URNS, 12, 1, 0, false),
            89 => (&[89, 284], 12, 5, 5, true),
            208 | 209 => (&[208, 209], 12, 1, 0, false),
            _ => return Ok(()),
        };
        let len = list.len() as i32;
        let mut j = self.count(d);
        if j < 1 {
            return Ok(());
        }
        while tries >= 1 {
            let mut class = list[self.c_roll(len) as usize];
            let (x0, y0, w, h) = self.rect();
            let mut x = x0 + self.c_roll(w - sx - 1);
            let mut y = y0 + self.c_roll(h - sy - 1);
            if !self.fit_a(x, y, sx, sy) {
                tries -= 1;
                continue;
            }
            self.alloc(class, x, y)?;
            let mut n = 1;
            let mut found = true;
            loop {
                if n >> 1 >= 1 && self.c_roll(n >> 1) != 0 {
                    break;
                }
                if !found {
                    break;
                }
                found = false;
                for _ in 0..3 * j.max(4) {
                    let dir = (self.c_step() & 7) as usize;
                    x += 2 * (b + self.c_roll(s)) * WALK_X[dir];
                    y += 2 * (b + self.c_roll(s)) * WALK_Y[dir];
                    let ok = if fit_a {
                        self.fit_a(x, y, sx, sy)
                    } else {
                        self.fit_b(x, y, sx, sy)
                    };
                    if ok {
                        found = true;
                        break;
                    }
                }
                if found {
                    class = list[self.c_roll(len) as usize];
                    self.alloc(class, x, y)?;
                    n += 1;
                }
            }
            j -= 1;
            tries -= 1;
            if j < 1 {
                return Ok(());
            }
        }
        Ok(())
    }

    /// Fn 2 (`0x00552B50`, §7.2).
    fn fn2(&mut self, a: u16) -> Result<Option<UnitId>, ObjectError> {
        let l = self.info.populated;
        let room = self.room();
        let r = room_step(self.w, room)? % 100;
        let forced = want_health(region(self.ctl, l)?);
        let tries = if forced {
            30
        } else {
            if r > P {
                return Ok(None);
            }
            3
        };
        if shrine_cap(region(self.ctl, l)?) {
            return Ok(None);
        }
        let (sx, sy) = self.sizes(a)?;
        let (orientation, parm1) = {
            let o = self.t.object(a)?;
            (o.orientation, o.parm1)
        };
        for _ in 0..tries {
            let Some(u) = self.oriented_spot(a, sx, sy, orientation, Filter::Shrine)? else {
                continue;
            };
            self.sel(u, a)?;
            if self.ctl.get(u)?.interact == 2 {
                region(self.ctl, l)?.health += 1;
            } else if forced {
                let class = if parm1 > 0 && parm1 < 573 {
                    parm1 as u16
                } else {
                    HEALTH_FALLBACK_CLASS
                };
                self.w.set_unit_class(u, class);
                shrine_pick(self.ctl, self.t, 2, 1)?;
                let d = self.ctl.get_mut(u)?;
                d.interact = 2;
                d.shrine = Some(2);
                region(self.ctl, l)?.health += 1;
            }
            let at = self.w.position(u);
            record_shrine(region(self.ctl, l)?, at);
            return Ok(Some(u));
        }
        Ok(None)
    }

    /// Fn 3 (`0x00551470`, §7.3).
    fn fn3(&mut self, d: i32, a: u16) -> Result<Option<UnitId>, ObjectError> {
        if d > 128 {
            return Err(ObjectError::Density(d as u8));
        }
        if self.c_step() % 100 > P {
            return Ok(None);
        }
        let (sx, sy) = self.sizes(a)?;
        let mut last = None;
        for _ in 0..self.count(d) {
            last = self.random_spot(a, sx, sy)?;
            if let Some(u) = last {
                self.sel(u, a)?;
            }
        }
        Ok(last)
    }

    /// Fn 4 (`0x00551850`, §7.4).
    fn fn4(&mut self, d: i32) -> Result<(), ObjectError> {
        let (sx, sy) = self.sizes(7)?;
        let (xs, ys) = {
            let o = self.t.object(7)?;
            (i32::from(o.xspace), i32::from(o.yspace))
        };
        if d > 128 {
            return Err(ObjectError::Density(d as u8));
        }
        if self.c_step() % 100 > P {
            return Ok(());
        }
        let mut j = self.count(d);
        let mut k = 2 * j;
        let mut c = 0;
        while j > 0 && k > 0 {
            let mut class = if self.c_step().is_multiple_of(3) {
                11
            } else {
                7
            };
            let (x0, y0, w, h) = self.rect();
            let mut x = x0 + self.c_roll(w);
            let mut y = y0 + self.c_roll(h);
            if !self.fit_b(x, y, sx, sy) {
                k -= 1;
                continue;
            }
            self.alloc(class, x, y)?;
            c += 1;
            if c >= 8 {
                return Ok(());
            }
            let mut n = 1;
            let mut found = true;
            loop {
                if n >> 1 >= 1 && self.c_roll(n >> 1) != 0 {
                    break;
                }
                if !found {
                    break;
                }
                found = false;
                for _ in 0..15 {
                    let dir = (self.c_step() & 7) as usize;
                    x += xs * WALK_X[dir];
                    y += ys * WALK_Y[dir];
                    if self.fit_b(x, y, sx, sy) {
                        found = true;
                        break;
                    }
                }
                if found {
                    class = if self.c_step() & 3 == 0 { 11 } else { 7 };
                    self.alloc(class, x, y)?;
                    n += 1;
                    c += 1;
                    if c >= 8 {
                        return Ok(());
                    }
                }
            }
            j -= 1;
            k -= 1;
        }
        Ok(())
    }

    /// Fn 5 (`0x00551C00`, §7.5).
    fn fn5(&mut self, d: i32, a: u16) -> Result<(), ObjectError> {
        let (sx, sy) = self.sizes(a)?;
        let (xs, ys) = {
            let o = self.t.object(a)?;
            (i32::from(o.xspace), i32::from(o.yspace))
        };
        if d > 128 {
            return Err(ObjectError::Density(d as u8));
        }
        if self.c_step() % 100 > P {
            return Ok(());
        }
        let mut j = self.count(d);
        if j < 1 {
            return Ok(());
        }
        let (mut k, mut l, mut c) = (2 * j, 4 * j, 0);
        while k >= 1 {
            let mut class = if a == 46 {
                a
            } else {
                URNS[self.c_roll(5) as usize]
            };
            let (x0, y0, w, h) = self.rect();
            let mut x = x0 + self.c_roll(w);
            let mut y = y0 + self.c_roll(h);
            if !self.fit_b(x, y, sx, sy) {
                k -= 1;
                if j < 1 {
                    return Ok(());
                }
                continue;
            }
            if class != 46 {
                class = URNS[self.c_roll(5) as usize];
            }
            self.alloc(class, x, y)?;
            let mut n = 1;
            let mut found = true;
            loop {
                if n >> 1 >= 1 && self.c_roll(n >> 1) != 0 {
                    break;
                }
                if !found {
                    break;
                }
                found = false;
                for _ in 0..l.max(4) {
                    let dir = (self.c_step() & 7) as usize;
                    x += xs * WALK_X[dir];
                    y += ys * WALK_Y[dir];
                    if self.fit_b(x, y, sx, sy) {
                        found = true;
                        break;
                    }
                }
                if found {
                    if self.alloc(class, x, y)?.is_some() {
                        c += 1;
                        if c >= 8 {
                            return Ok(());
                        }
                    }
                    n += 1;
                }
            }
            j -= 1;
            l -= 4;
            k -= 1;
            if j < 1 {
                return Ok(());
            }
        }
        Ok(())
    }

    /// Flies on `u` (`0x00551150`, §7.6): at its position in its room.
    fn flies(&mut self, u: UnitId) -> Result<(), ObjectError> {
        let (x, y) = self.w.position(u);
        let room = self.w.room(u).unwrap_or(self.room());
        if let Some(f) = allocate(self.ctl, self.t, self.w, room, FLIES_CLASS, x, y, 0)? {
            self.out.objects.push(f);
        }
        Ok(())
    }

    /// Fn 6 (`0x00551690`, §7.6).
    fn fn6(&mut self, d: i32, a: u16) -> Result<(), ObjectError> {
        if let Some(u) = self.fn3(d, a)? {
            if self.c_step() % 100 > 70 {
                self.flies(u)?;
            }
        }
        Ok(())
    }

    /// Fn 7 (`0x00551200`, §7.7): the 1.14d counts are 0, so only the
    /// draws happen.
    fn fn7(&mut self, a: u16) -> Result<(), ObjectError> {
        if self.c_step() % 100 > P {
            return Ok(());
        }
        let q = self.c_roll(4) as usize;
        let (sx, sy) = self.sizes(a)?;
        let (x0, y0, w, h) = self.rect();
        let mut base = None;
        for _ in 0..8 {
            let x = x0 + self.c_roll(w - sx - 1);
            let y = y0 + self.c_roll(h - sy - 1);
            if self.fit_a(x, y, 5, 5) {
                base = Some((x, y));
                break;
            }
        }
        let Some((bx, by)) = base else {
            return Ok(());
        };
        for &(dx, dy) in &ROGUE_PATTERNS[q][..ROGUE_COUNTS[q]] {
            let (x, y) = (bx + dx, by + dy);
            if self.fit_a(x, y, sx, sy) {
                let class = 57 + (self.c_step() & 1) as u16;
                if let Some(u) = self.alloc(class, x, y)? {
                    self.flies(u)?;
                }
            }
        }
        Ok(())
    }

    /// Fn 8 (`0x005516C0`, §7.8).
    fn fn8(&mut self, d: i32, a: u16) -> Result<Option<UnitId>, ObjectError> {
        let l = self.info.populated;
        if well_cap(region(self.ctl, l)?) {
            return Ok(None);
        }
        if d > 128 {
            return Err(ObjectError::Density(d as u8));
        }
        let room = self.room();
        if room_step(self.w, room)? % 100 > P {
            return Ok(None);
        }
        let (sx, sy) = self.sizes(a)?;
        let Some(u) = self.spread_spot(a, 2 * sx + 1, 2 * sy + 1, Filter::Well)? else {
            return Ok(None);
        };
        self.sel(u, a)?;
        let at = self.w.position(u);
        record_well(region(self.ctl, l)?, at);
        Ok(Some(u))
    }

    /// Fn 9 (`0x00551580`, §7.9).
    fn fn9(&mut self, d: i32, a: u16) -> Result<Option<UnitId>, ObjectError> {
        if d > 128 {
            return Err(ObjectError::Density(d as u8));
        }
        let room = self.room();
        if room_step(self.w, room)? % 100 > P {
            return Ok(None);
        }
        let (sx, sy) = self.sizes(a)?;
        for _ in 0..self.count(d) {
            if let Some(u) = self.random_spot(a, sx, sy)? {
                self.sel(u, a)?;
                return Ok(Some(u));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests;

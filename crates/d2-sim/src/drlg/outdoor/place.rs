//! Act-wide outdoor placement: where each outdoor level of an act sits relative to the
//! others.
//!
//! Holds the per-act link tables, the link driver and its linkers, the placement
//! cases, the overlap checks, the Act I border flags, adjacency warps and neighbour
//! entries, plus Act III's jungle placer and Kurast chain. `Outdoor::create_act_levels`
//! is the entry point; the per-level generators run afterwards.

use crate::rng::Seed;

use super::super::data::DrlgData;
use super::super::level::Drlg;
use super::super::room::near_gaps;
use super::super::seams::LevelTypes;
use super::super::{LevelIdx, TileRect};
use super::jungle::place_jungles;
use super::{Orth, Outdoor, OutdoorData, OutdoorError, DRLG_OUTDOOR};

/// Maximum rows in a link table; sizes the driver's per-row arrays.
pub const TABLE_ROWS: usize = 15;

/// The ways a link-table row can position its level relative to a parent row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Linker {
    /// Fixed position from the level definition's offset.
    Def,
    /// One of four sides of the parent, tried in rotation from a random start.
    R4,
    /// One of eight positions around the parent (shifted variant), tried in rotation.
    R8,
    /// Four sides of the parent with a size that depends on the side (96x56 or 56x96),
    /// with a random choice between placement shapes.
    Bm,
    /// Like `Bm` but keeps the level's own size.
    Re,
    /// Fixed side of the parent: below it, no variant.
    Fix,
    /// Two alternatives: left of the parent (shape A) or above it (shape B).
    Rw,
    /// One of eight positions around the parent (unshifted variant), tried in rotation.
    Vs,
    /// East of the parent, placed one of two ways; records a transition flag.
    Os,
    /// 64x160 or 160x64 level placed left of the parent, by a random orientation.
    B1,
    /// 64x160 or 160x64 level at its definition's fixed offset.
    Bd,
    /// 64x160 or 160x64 level offset from the parent through `B2_OFFSETS`.
    B2,
}

/// A link-table row: (linker, level, link row or −1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkRow {
    /// How the level is positioned.
    pub linker: Linker,
    /// Level id placed by this row; 0 ends the table early.
    pub level: u32,
    /// Index of the parent row this level hangs off, or −1 for none.
    pub link: i32,
}

const fn row(linker: Linker, level: u32, link: i32) -> LinkRow {
    LinkRow {
        linker,
        level,
        link,
    }
}

use Linker::*;

/// Act I link table `A1W` (levels 4, 3, 2, 1, 17).
pub const A1W: [LinkRow; 5] = [
    row(Def, 4, -1),
    row(R4, 3, 0),
    row(Bm, 2, 1),
    row(Re, 1, 2),
    row(R4, 17, 1),
];
/// Act I link table `A1M` (levels 39, 26, 7, 6, 5).
pub const A1M: [LinkRow; 5] = [
    row(Def, 39, -1),
    row(Def, 26, -1),
    row(Fix, 7, 1),
    row(R4, 6, 2),
    row(R4, 5, 3),
];
/// Act II main link table (levels 40 to 45).
pub const A2: [LinkRow; 6] = [
    row(Def, 40, -1),
    row(Rw, 41, 0),
    row(R8, 42, 1),
    row(R8, 43, 2),
    row(R8, 44, 3),
    row(Vs, 45, 4),
];
/// Act II link table `A2C` (level 46 only).
pub const A2C: [LinkRow; 1] = [row(Def, 46, -1)];
/// Act V link table `A5U` (levels 134 and 136).
pub const A5U: [LinkRow; 2] = [row(Def, 134, -1), row(Def, 136, -1)];
/// Act IV main link table (levels 103 to 106).
pub const A4: [LinkRow; 4] = [
    row(Def, 103, -1),
    row(Os, 104, 0),
    row(R4, 105, 1),
    row(R4, 106, 2),
];
/// Act IV link table `A4C` (level 108 only).
pub const A4C: [LinkRow; 1] = [row(Def, 108, -1)];
/// Act V main link table (levels 109 to 112).
pub const A5: [LinkRow; 4] = [
    row(Def, 109, -1),
    row(Def, 110, 0),
    row(B1, 111, 1),
    row(B2, 112, 2),
];
/// Act V link table `A5T` (level 117 only).
pub const A5T: [LinkRow; 1] = [row(Bd, 117, -1)];

/// The check the driver runs on a row after placing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    /// No check; every placement is accepted.
    None,
    /// Act I `A1W` check: no overlap, plus the rogue table and level 17 direction rules.
    A1W,
    /// Act I `A1M` check: no overlap, plus a clear strip above row 0.
    A1M,
    /// Rows j < i, j other than the row's link, must not overlap row i.
    Simple,
}

/// Indices that pass the `A1W` rogue check for level 1; every other index fails.
pub const ROGUE_OK: [usize; 16] = [0, 1, 9, 19, 21, 24, 30, 31, 33, 39, 41, 53, 54, 56, 62, 63];

/// Act I flag rows: (level filter, excluded level 1, excluded level 2, r, rNext, flags).
///
/// A filter of 0 matches any level; `r` and `rNext` are the row's and the next row's
/// placement value.
pub const ACT1_FLAG_ROWS: [(u32, u32, u32, i32, i32, u32); 15] = [
    (0, 2, 3, 1, 0, 0x4),
    (0, 2, 3, 2, 3, 0x4),
    (0, 3, 17, 2, 1, 0x8),
    (0, 3, 17, 3, 0, 0x8),
    (0, 3, 17, 1, 1, 0x10),
    (0, 3, 17, 3, 3, 0x10),
    (2, 0, 0, 0, 0, 0x8),
    (2, 0, 0, 2, 2, 0x8),
    (2, 0, 0, 3, 0, 0x8),
    (2, 0, 0, 3, 2, 0x8),
    (2, 0, 0, 0, 1, 0x400),
    (2, 0, 0, 1, 1, 0x400),
    (2, 0, 0, 2, 1, 0x200),
    (2, 0, 0, 2, 2, 0x80),
    (2, 0, 0, 3, 2, 0x100),
];

/// Offsets of a `B2` row from its parent, indexed by its own R0 + 2 × the parent's R0.
pub const B2_OFFSETS: [(i32, i32); 4] = [(0, -160), (-96, -64), (-64, -96), (-160, 0)];

/// The link driver's state: the placed rect of every row and its linker bookkeeping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Driver {
    /// Placed rect of each row.
    pub c: [TileRect; TABLE_ROWS],
    /// R0..R3 per row: the linker's current choice, its starting choice, and extra
    /// state. -1 means unset.
    pub r: [[i32; TABLE_ROWS]; 4],
    /// The driver's copy of the DRLG seed.
    pub seed: Seed,
    /// Transition flag set by the `Os` linker (0 if it never ran).
    pub transition: u32,
}

impl Driver {
    /// Creates a driver with empty rects, unset linker state and the given seed.
    pub fn new(seed: Seed) -> Self {
        Self {
            c: [TileRect::default(); TABLE_ROWS],
            r: [[-1; TABLE_ROWS]; 4],
            seed,
            transition: 0,
        }
    }
}

/// Placement shape A: positions `c` against parent `p` for side `case` (0 to 3) and
/// variant `v`.
pub fn place_a(p: TileRect, c: &mut TileRect, case: i32, v: i32) {
    match case {
        0 => {
            (c.x, c.y) = (p.x, p.y + p.h);
            if v == 1 {
                c.x -= 16;
            }
        }
        1 => {
            (c.x, c.y) = (p.x - c.w, p.y);
            match v {
                1 => c.y -= 16,
                2 => c.y += 8,
                _ => {}
            }
        }
        2 => {
            (c.x, c.y) = (p.x + p.w - c.w, p.y - c.h);
            if v == 1 {
                c.x += 16;
            }
        }
        3 => {
            (c.x, c.y) = (p.x + p.w, p.y + p.h - c.h);
            match v {
                1 => c.y += 16,
                2 => c.y -= 8,
                3 => c.y += 8,
                _ => {}
            }
        }
        _ => {}
    }
}

/// Placement shape B: the mirror of shape A, positioning `c` against `p` for side
/// `case` (0 to 3) and variant `v`.
pub fn place_b(p: TileRect, c: &mut TileRect, case: i32, v: i32) {
    match case {
        0 => {
            (c.x, c.y) = (p.x + p.w - c.w, p.y + p.h);
            if v == 1 {
                c.x += 16;
            }
        }
        1 => {
            (c.x, c.y) = (p.x - c.w, p.y + p.h - c.h);
            match v {
                1 => c.y += 16,
                2 => c.y -= 8,
                _ => {}
            }
        }
        2 => {
            (c.x, c.y) = (p.x, p.y - c.h);
            if v == 1 {
                c.x -= 16;
            }
        }
        3 => {
            (c.x, c.y) = (p.x + p.w, p.y);
            match v {
                1 => c.y -= 16,
                2 => c.y += 8,
                3 => c.y -= 8,
                _ => {}
            }
        }
        _ => {}
    }
}

/// Placement shape C (Act II): positions `c` against `p` for one of eight cases. The
/// half-size shifts apply only with variant 1.
pub fn place_c(p: TileRect, c: &mut TileRect, case: i32, v: i32) {
    let hx = if v == 1 { c.w / 2 + 8 } else { 0 };
    let hy = if v == 1 { c.h / 2 + 8 } else { 0 };
    match case {
        0 => (c.x, c.y) = (p.x - hx, p.y + p.h),
        1 => (c.x, c.y) = (p.x + hx, p.y + p.h),
        2 => (c.x, c.y) = (p.x - c.w, p.y - hy),
        3 => (c.x, c.y) = (p.x - c.w, p.y + hy),
        4 => (c.x, c.y) = (p.x - hx, p.y - c.h),
        5 => (c.x, c.y) = (p.x + hx, p.y - c.h),
        6 => (c.x, c.y) = (p.x + p.w, p.y - hy),
        7 => (c.x, c.y) = (p.x + p.w, p.y + hy),
        _ => {}
    }
}

/// True when the two rects overlap. Touching edges are allowed: the rects overlap
/// unless one gap is at least 0.
pub fn overlaps(a: &TileRect, b: &TileRect) -> bool {
    let (gx, gy) = near_gaps(a, b);
    !(gx >= 0 || gy >= 0)
}

/// True when the two rects share an edge: one gap is 0 and the other at most −1.
pub fn shares_edge(a: &TileRect, b: &TileRect) -> bool {
    let (gx, gy) = near_gaps(a, b);
    (gx == 0 && gy <= -1) || (gy == 0 && gx <= -1)
}

/// Direction from rect `a` to the rect `b` that touches it: 0 west, 1 north, 2 east,
/// 3 south, −1 when they do not touch along an edge.
pub fn direction(a: &TileRect, b: &TileRect) -> i32 {
    if b.x < a.x && a.x == b.x + b.w {
        0
    } else if b.x >= a.x && b.x == a.x + a.w {
        2
    } else if b.y < a.y && a.y == b.y + b.h {
        1
    } else if b.y >= a.y && b.y == a.y + a.h {
        3
    } else {
        -1
    }
}

/// Runs the linker of row `i` once, placing the row. Returns false when the linker's
/// alternatives are used up.
fn run_linker(
    d: &mut Driver,
    rows: &[LinkRow],
    i: usize,
    data: &DrlgData,
) -> Result<bool, OutdoorError> {
    let row = rows[i];
    let first = d.r[1][i] == -1;
    let parent = if row.link >= 0 {
        d.c[row.link as usize]
    } else {
        TileRect::default()
    };
    let mut c = d.c[i];
    match row.linker {
        Def => {
            d.r[0][i] = -1;
            let (x, y) = data.level(row.level)?.offset;
            (c.x, c.y) = (x, y);
        }
        R4 | R8 | Vs => {
            let n = if row.linker == R4 { 4 } else { 8 };
            if first {
                let v = d.seed.mask(n as u32) as i32;
                d.r[0][i] = v;
                d.r[1][i] = v;
            } else {
                let next = (d.r[0][i] + 1) % n;
                if next == d.r[1][i] {
                    return Ok(false);
                }
                d.r[0][i] = next;
            }
            match row.linker {
                R4 => place_a(parent, &mut c, d.r[0][i], 1),
                R8 => place_c(parent, &mut c, d.r[0][i], 1),
                _ => place_c(parent, &mut c, d.r[0][i], 0),
            }
        }
        Bm | Re => {
            let t;
            if first {
                let r0 = d.seed.mask(4) as i32;
                d.r[0][i] = r0;
                d.r[1][i] = r0;
                d.r[3][i] = d.seed.mask(2) as i32;
                t = d.r[3][i];
            } else {
                t = (d.r[2][i] + 1) % 2;
                let r = (d.r[2][i] + d.r[0][i]) % 4;
                if r == d.r[1][i] && t == d.r[3][i] {
                    return Ok(false);
                }
                d.r[0][i] = r;
            }
            d.r[2][i] = t;
            let case = d.r[0][i];
            let v = if row.linker == Bm { 1 } else { 2 };
            if row.linker == Bm {
                (c.w, c.h) = if case % 2 == 1 { (96, 56) } else { (56, 96) };
            }
            if t == 1 {
                place_a(parent, &mut c, case, v);
            } else {
                place_b(parent, &mut c, case, v);
            }
        }
        Fix => {
            d.r[0][i] = 0;
            d.r[1][i] = 0;
            place_a(parent, &mut c, 0, 0);
        }
        Rw => {
            if first {
                let v = d.seed.mask(2) as i32 + 1;
                d.r[0][i] = v;
                d.r[1][i] = v;
            } else {
                let r = if d.r[0][i] == 1 { 2 } else { 1 };
                if r == d.r[1][i] {
                    return Ok(false);
                }
                d.r[0][i] = r;
            }
            if d.r[0][i] == 1 {
                place_a(parent, &mut c, 1, 0);
            } else {
                place_b(parent, &mut c, d.r[0][i], 0);
            }
        }
        Os => {
            d.r[0][i] = 3;
            d.r[1][i] = 3;
            if d.seed.mask(2) == 0 {
                place_b(parent, &mut c, 3, 3);
                d.transition = 0x40_0000;
            } else {
                place_a(parent, &mut c, 3, 3);
                d.transition = 0x80_0000;
            }
        }
        B1 | Bd => {
            let v = d.seed.mask(2) as i32;
            d.r[0][i] = v;
            d.r[1][i] = v;
            (c.w, c.h) = if v == 0 { (64, 160) } else { (160, 64) };
            if row.linker == B1 {
                c.x = parent.x - c.w;
                c.y = parent.y + parent.h - c.h - 16;
            } else {
                let (x, y) = data.level(row.level)?.offset;
                (c.x, c.y) = (x, y);
            }
        }
        B2 => {
            if first {
                let v = d.seed.mask(2) as i32;
                d.r[0][i] = v;
                d.r[1][i] = v;
            } else {
                let r = i32::from(d.r[0][i] == 0);
                if r == d.r[1][i] {
                    return Ok(false);
                }
                d.r[0][i] = r;
            }
            let r0 = d.r[0][i];
            (c.w, c.h) = if r0 == 0 { (64, 160) } else { (160, 64) };
            let link_r0 = if row.link >= 0 {
                d.r[0][row.link as usize]
            } else {
                0
            };
            // An index outside 0..3 (the parent's R0 not 0 or 1) falls back to a zero
            // offset. The link of a `B2` row is a `B1` row, so this is not reached.
            let (ox, oy) = B2_OFFSETS
                .get((r0 + 2 * link_r0) as usize)
                .copied()
                .unwrap_or((0, 0));
            (c.x, c.y) = (parent.x + ox, parent.y + oy);
        }
    }
    d.c[i] = c;
    Ok(true)
}

/// Runs the check `check` on row `i` against the rows placed so far.
fn check(d: &Driver, rows: &[LinkRow], i: usize, check: Check) -> bool {
    let link = rows[i].link;
    let others_clear = |d: &Driver| (0..i).all(|j| j as i32 == link || !overlaps(&d.c[j], &d.c[i]));
    match check {
        Check::None => true,
        Check::Simple => others_clear(d),
        Check::A1W => {
            if !others_clear(d) {
                return false;
            }
            match rows[i].level {
                1 if link >= 0 => {
                    let l = link as usize;
                    let idx = d.r[0][i] + 4 * (d.r[2][i] + 2 * (d.r[0][l] + 4 * d.r[2][l]));
                    idx >= 0 && ROGUE_OK.contains(&(idx as usize))
                }
                17 => !rows
                    .iter()
                    .enumerate()
                    .any(|(j, r)| j != i && r.link == link && d.r[0][j] == d.r[0][i]),
                _ => true,
            }
        }
        Check::A1M => {
            if !others_clear(d) {
                return false;
            }
            if i > 0 {
                let mut r0 = d.c[0];
                r0.y -= 200;
                r0.h += 200;
                if overlaps(&r0, &d.c[i]) {
                    return false;
                }
            }
            true
        }
    }
}

impl Outdoor {
    /// Places the current act's outdoor levels.
    ///
    /// `types` allocates levels; outdoor init may be a no-op there.
    pub fn create_act_levels(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        _od: &OutdoorData,
        types: &mut dyn LevelTypes,
    ) -> Result<(), OutdoorError> {
        match drlg.act {
            0 => {
                self.drive(drlg, data, types, &A1W, Check::A1W, true)?;
                self.drive(drlg, data, types, &A1M, Check::A1M, true)?;
                self.neighbours(drlg, data, types, 1, 17)?;
            }
            1 => {
                self.drive(drlg, data, types, &A2, Check::Simple, false)?;
                self.drive(drlg, data, types, &A2C, Check::Simple, false)?;
                self.neighbours(drlg, data, types, 40, 46)?;
            }
            2 => {
                let l = drlg.get_or_alloc_level(data, types, 75)?;
                let def = data.level(75)?;
                let (w, h) = def.size[(drlg.difficulty as usize).min(2)];
                drlg.level_mut(l).rect = TileRect::new(def.offset.0, def.offset.1, w, h);
                let l78 = self.jungles(drlg, data, types)?;
                self.kurast_chain(drlg, data, types, l78)?;
                self.adjacency(drlg, data, 75, 83)?;
                self.neighbours(drlg, data, types, 75, 83)?;
            }
            3 => {
                let t1 = self.drive(drlg, data, types, &A4, Check::Simple, false)?;
                self.drive(drlg, data, types, &A4C, Check::Simple, false)?;
                // The driver keeps the last `Os` linker's transition flag.
                if let Some(l) = drlg.find_level(104) {
                    self.info_mut(l).flags |= t1.transition;
                }
                self.neighbours(drlg, data, types, 103, 106)?;
            }
            4 => {
                self.drive(drlg, data, types, &A5, Check::None, false)?;
                self.drive(drlg, data, types, &A5T, Check::None, false)?;
                self.adjacency(drlg, data, 111, 112)?;
                self.neighbours(drlg, data, types, 111, 112)?;
                self.adjacency(drlg, data, 110, 111)?;
                self.adjacency(drlg, data, 109, 110)?;
                self.drive(drlg, data, types, &A5U, Check::Simple, false)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Places every row of a link table with backtracking, then allocates the levels.
    ///
    /// Returns the final driver state. `flags_fn` turns on the Act I border flags.
    pub fn drive(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        types: &mut dyn LevelTypes,
        rows: &[LinkRow],
        chk: Check,
        flags_fn: bool,
    ) -> Result<Driver, OutdoorError> {
        let act5 = drlg.act == 4;
        // The driver works on its own copy of the DRLG seed.
        let mut d = Driver::new(drlg.seed);
        let diff = (drlg.difficulty as usize).min(2);
        for (i, r) in rows.iter().enumerate() {
            let (w, h) = data.level(r.level)?.size[diff];
            d.c[i].w = w;
            d.c[i].h = h;
        }
        // Place rows in order; a linker that runs out of alternatives sends the driver
        // back one row, and a failed check retries the same row.
        let mut i = 0usize;
        while i < rows.len() && rows[i].level != 0 {
            if !run_linker(&mut d, rows, i, data)? {
                for k in 0..4 {
                    d.r[k][i] = -1;
                }
                // A first row that runs out of alternatives has nothing to go back to
                // and reports `DriverUnderflow`; the shipped tables do not reach this.
                i = i.checked_sub(1).ok_or(OutdoorError::DriverUnderflow)?;
                continue;
            }
            if check(&d, rows, i, chk) {
                i += 1;
            }
        }
        // Allocate each level, record its rect, and add direction, flags and warps.
        for (i, r) in rows.iter().enumerate() {
            let l = drlg.get_or_alloc_level(data, types, r.level)?;
            drlg.level_mut(l).rect = d.c[i];
            if drlg.level(l).drlg_type == 2 {
                match r.level {
                    1 => {
                        self.preset_direction.insert(1, d.r[0][i]);
                    }
                    40 => {
                        let v = d.r[0].get(i + 1).copied().unwrap_or(-1);
                        self.preset_direction.insert(40, v);
                    }
                    _ => {}
                }
            }
            if r.level == 6 {
                drlg.get_or_alloc_level(data, types, 27)?;
                match d.r[0][i] {
                    1 => {
                        // This draw uses the DRLG seed itself, not the driver's copy.
                        let v = drlg.seed.step();
                        self.preset_direction.insert(27, 2 - (v & 1) as i32);
                    }
                    3 => {
                        // Same: a draw from the DRLG seed itself.
                        let v = drlg.seed.step();
                        self.preset_direction.insert(27, 1 - (v & 1) as i32);
                    }
                    _ => {}
                }
            }
            if flags_fn && drlg.level(l).drlg_type == DRLG_OUTDOOR {
                self.act1_flags(l, r.level, &d, i);
            }
            if !act5 && r.link >= 0 {
                let other = rows[r.link as usize].level;
                drlg.set_warp(data, r.level, other, -1, -1)?;
                drlg.set_warp(data, other, r.level, -1, -1)?;
            }
        }
        Ok(d)
    }

    /// ORs the Act I border flags that match this row's and the next row's placement
    /// values into the level's info.
    pub fn act1_flags(&mut self, l: LevelIdx, id: u32, d: &Driver, row: usize) {
        let r = d.r[0][row];
        let r_next = d.r[0].get(row + 1).copied().unwrap_or(-1);
        let mut flags = 0;
        for &(filter, e1, e2, a, b, f) in &ACT1_FLAG_ROWS {
            if (filter == 0 || filter == id) && id != e1 && id != e2 && a == r && b == r_next {
                flags |= f;
            }
        }
        self.info_mut(l).flags |= flags;
    }

    /// Adds a warp both ways between every pair of levels in `a..=b` whose rects share
    /// an edge.
    pub fn adjacency(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        a: u32,
        b: u32,
    ) -> Result<(), OutdoorError> {
        for i in a..=b {
            for j in a..=b {
                if i == j {
                    continue;
                }
                // A level in the range that is not allocated is skipped.
                let (Some(li), Some(lj)) = (drlg.find_level(i), drlg.find_level(j)) else {
                    continue;
                };
                if shares_edge(&drlg.level(li).rect, &drlg.level(lj).rect) {
                    drlg.set_warp(data, i, j, -1, -1)?;
                }
            }
        }
        Ok(())
    }

    /// Records, for each outdoor level in `a..=b`, its visible neighbours that lack a
    /// warp, as `Orth` entries with the direction towards them.
    pub fn neighbours(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        types: &mut dyn LevelTypes,
        a: u32,
        b: u32,
    ) -> Result<(), OutdoorError> {
        for id in a..=b {
            // Each id is looked up by get-or-allocate before its type is tested, so every
            // level in the range exists afterwards. In Act I this allocates, in
            // ascending order, the levels of 1..17 that no driver row allocated
            // (8 to 16).
            let l = drlg.get_or_alloc_level(data, types, id)?;
            if drlg.level(l).drlg_type != DRLG_OUTDOOR {
                continue;
            }
            let vis = drlg.vis_array(data, id)?;
            let warp = drlg.warp_array(data, id)?;
            for j in 0..8 {
                if vis[j] == 0 || warp[j] != -1 {
                    continue;
                }
                let Some(n) = drlg.find_level(vis[j]) else {
                    continue;
                };
                let rect = drlg.level(n).rect;
                let entry = Orth {
                    level_id: vis[j],
                    direction: direction(&drlg.level(l).rect, &rect),
                    init: false,
                    rect,
                    preset: drlg.level(n).drlg_type == 2,
                };
                // Entries go in at the head of the list; the polygon and link probes
                // walk the list in that order.
                self.info_mut(l).orth.insert(0, entry);
            }
        }
        Ok(())
    }

    /// Act III jungle placer, drawing from the DRLG seed itself. Returns level 78.
    fn jungles(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        types: &mut dyn LevelTypes,
    ) -> Result<LevelIdx, OutdoorError> {
        let diff = (drlg.difficulty as usize).min(2);
        let (sx, sy) = data.level(76)?.size[diff];
        let docks = drlg
            .find_level(75)
            .map(|l| drlg.level(l).rect)
            .ok_or(OutdoorError::LevelMissing(75))?;
        let j = place_jungles(&mut drlg.seed, docks, sx, sy)?;
        // Levels 76..78 take the jungles in their sorted order.
        let mut last = None;
        for (n, &k) in j.order.iter().enumerate() {
            let rec = &j.recs[k];
            let l = drlg.get_or_alloc_level(data, types, 76 + n as u32)?;
            let info = self.info_mut(l);
            info.jungle_ids = Some(rec.ids.clone());
            info.jungle_clearings = rec.clearings;
            drlg.level_mut(l).rect = rec.rect;
            last = Some(l);
        }
        // The last level placed is level 78.
        last.ok_or(OutdoorError::LevelMissing(78))
    }

    /// Act III Kurast chain: levels 79 to 83 stacked northwards, centred on the anchor
    /// (level 78, the jungle placer's return value).
    fn kurast_chain(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        types: &mut dyn LevelTypes,
        anchor: LevelIdx,
    ) -> Result<(), OutdoorError> {
        let diff = (drlg.difficulty as usize).min(2);
        let a = drlg.level(anchor).rect;
        let mut y = 0;
        for id in 79..=83 {
            let (w, h) = data.level(id)?.size[diff];
            y -= h;
            let l = drlg.get_or_alloc_level(data, types, id)?;
            drlg.level_mut(l).rect = TileRect::new(a.x + a.w / 2 - w / 2, a.y + y, w, h);
        }
        Ok(())
    }
}

/// The jungle case offsets (y1, y3) for a jungle height SY.
///
/// y1 is ⌊(⌊SY·0x55555555 / 2³²⌋ − SY) / 2⌋, plus 1 if negative; y3 is −2SY/3 truncated
/// toward zero.
pub fn jungle_offsets(sy: i32) -> (i32, i32) {
    let third = ((i64::from(sy) * 0x5555_5555) >> 32) as i32;
    let mut y1 = (third - sy).div_euclid(2);
    if y1 < 0 {
        y1 += 1;
    }
    (y1, -2 * sy / 3)
}

#[cfg(test)]
#[path = "linker_tests.rs"]
mod linker_tests;

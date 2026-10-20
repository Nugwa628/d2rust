//! Act III jungle placer: the three jungle records, the block grids, river and attach
//! points with the restart point, river connections, attach directions and the table
//! from block codes to lvlprest ids.
//!
//! The three jungle levels (76..=78) are laid out together at act creation. Handing the
//! result to those levels is `place.rs`; `act3.rs` stamps the ids at level build. Every
//! draw is on the seed passed in (the DRLG seed itself).

use crate::rng::Seed;

use super::super::TileRect;
use super::place::{jungle_offsets, overlaps};
use super::OutdoorError;

/// Block size in tiles.
pub const BLOCK: i32 = 32;

/// Clearing preset ids, indexed by the block code `D >> 4` for 0..15. Entry 0 (256) is
/// unreachable; entries 7, 11 and 13..15 are 0, and `code_to_id` rejects them.
pub const CLEARING_IDS: [u32; 16] = [
    256, 575, 576, 577, 578, 579, 580, 0, 581, 582, 583, 0, 584, 0, 0, 0,
];

/// River-with-exit preset ids, rows 1..14 (index r − 1); slot 0 W, 1 E, 2 S, 3 N;
/// 0 = none.
pub const EXIT_IDS: [[u32; 4]; 14] = [
    [0, 545, 546, 547],
    [548, 0, 549, 550],
    [0, 0, 551, 552],
    [553, 554, 0, 555],
    [0, 556, 0, 557],
    [558, 0, 0, 559],
    [0, 0, 0, 560],
    [561, 562, 563, 0],
    [0, 564, 565, 0],
    [566, 0, 567, 0],
    [0, 0, 568, 0],
    [569, 570, 0, 0],
    [0, 571, 0, 0],
    [572, 0, 0, 0],
];

/// One jungle: its rectangle, how it attaches to the others, and its block ids.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JungleRec {
    /// Rectangle in tiles.
    pub rect: TileRect,
    /// Placement case (0..=4) relative to the base jungle; record 0: 0.
    pub case: i32,
    /// Index of the base record; record 0: 0.
    pub base: usize,
    /// Branch records in placement order.
    pub branches: Vec<usize>,
    /// Block column of the top-left block in the shared grid.
    pub bx: i32,
    /// Block row of the top-left block in the shared grid.
    pub by: i32,
    /// Block ids, row-major from the top-left block.
    pub ids: Vec<u32>,
    /// Number of clearing blocks among `ids`.
    pub clearings: i32,
}

/// Fatal error codes of the jungle placer and the jungle stamping.
pub mod fatal {
    /// A jungle level has no id grid.
    pub const NO_IDS: u32 = 0x27;
    /// A jungle level has more than 3 clearings.
    pub const CLEARINGS: u32 = 0x47;
    /// A jungle already has 3 branches.
    pub const BRANCHES: u32 = 0x63E;
    /// The jungles' combined width is not a whole number of blocks.
    pub const SPAN_X: u32 = 0x64D;
    /// The jungles' combined height is not a whole number of blocks.
    pub const SPAN_Y: u32 = 0x64E;
    /// A block code maps to no river-exit id.
    pub const EXIT_ID: u32 = 0x78C;
    /// A block code maps to no clearing id.
    pub const CLEARING_ID: u32 = 0x799;
}

/// River exit bits of the block code D: N, S, E, W.
const RIVER_BITS: [i32; 4] = [8, 4, 2, 1];
/// Attach exit bits of the block code D, in direction order 0 N, 1 S, 2 E, 3 W.
const ATTACH_BITS: [i32; 4] = [0x80, 0x40, 0x20, 0x10];
/// Neighbour offsets (column, row) for N, S, E, W.
const DIRS: [(i32, i32); 4] = [(0, -1), (0, 1), (1, 0), (-1, 0)];
/// The opposite of direction i (N ↔ S, E ↔ W).
const OPPOSITE: [usize; 4] = [1, 0, 3, 2];

/// The four W×H block grids shared by the three jungles.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JungleGrid {
    /// Width in blocks.
    pub w: i32,
    /// Height in blocks.
    pub h: i32,
    /// A: owner (jungle index + 1).
    pub a: Vec<i32>,
    /// B: river order.
    pub b: Vec<i32>,
    /// C: marks (0 none, 1 link, 2 attach point).
    pub c: Vec<i32>,
    /// D: code.
    pub d: Vec<i32>,
}

impl JungleGrid {
    /// Four zeroed grids of `w` × `h` blocks.
    pub fn new(w: i32, h: i32) -> Self {
        let n = (w.max(0) * h.max(0)) as usize;
        Self {
            w,
            h,
            a: vec![0; n],
            b: vec![0; n],
            c: vec![0; n],
            d: vec![0; n],
        }
    }

    /// The restart point: sets all four grids to 0.
    pub fn clear(&mut self) {
        for g in [&mut self.a, &mut self.b, &mut self.c, &mut self.d] {
            g.iter_mut().for_each(|v| *v = 0);
        }
    }

    /// Index of (column, row), or None outside the grid. The grid has a one-block outer
    /// ring, so neighbour reads of river and attach cells stay inside; the guard only
    /// keeps out-of-grid reads from panicking.
    pub fn at(&self, col: i32, row: i32) -> Option<usize> {
        (col >= 0 && row >= 0 && col < self.w && row < self.h)
            .then(|| (row * self.w + col) as usize)
    }

    fn get(g: &[i32], i: Option<usize>) -> i32 {
        i.map_or(0, |i| g[i])
    }

    /// Owner (jungle index + 1) at a block; 0 outside the grid.
    pub fn a_at(&self, col: i32, row: i32) -> i32 {
        Self::get(&self.a, self.at(col, row))
    }

    /// River order at a block; 0 outside the grid.
    pub fn b_at(&self, col: i32, row: i32) -> i32 {
        Self::get(&self.b, self.at(col, row))
    }

    /// Mark at a block (0 none, 1 link, 2 attach point); 0 outside the grid.
    pub fn c_at(&self, col: i32, row: i32) -> i32 {
        Self::get(&self.c, self.at(col, row))
    }

    /// Code at a block; 0 outside the grid.
    pub fn d_at(&self, col: i32, row: i32) -> i32 {
        Self::get(&self.d, self.at(col, row))
    }

    fn set_b(&mut self, col: i32, row: i32, v: i32) {
        if let Some(i) = self.at(col, row) {
            self.b[i] = v;
        }
    }

    fn set_c(&mut self, col: i32, row: i32, v: i32) {
        if let Some(i) = self.at(col, row) {
            self.c[i] = v;
        }
    }

    fn or_d(&mut self, col: i32, row: i32, v: i32) {
        if let Some(i) = self.at(col, row) {
            self.d[i] |= v;
        }
    }

    /// Computes the river exit bits of every block, whole grid in row-major order, with
    /// no random draws.
    pub fn connect_rivers(&mut self) {
        for row in 0..self.h {
            for col in 0..self.w {
                let (ac, bc, cc) = (
                    self.a_at(col, row),
                    self.b_at(col, row),
                    self.c_at(col, row),
                );
                let mut f = 0;
                // A link block opens towards its lowest-order river neighbour of the same
                // owner, and towards link blocks of other owners.
                if cc == 1 {
                    let mut best: Option<(usize, i32)> = None;
                    for (k, &(dx, dy)) in DIRS.iter().enumerate() {
                        let (nc, nr) = (col + dx, row + dy);
                        let nb = self.b_at(nc, nr);
                        if nb != 0 && self.a_at(nc, nr) == ac && best.is_none_or(|(_, bb)| nb < bb)
                        {
                            best = Some((k, nb));
                        }
                    }
                    if let Some((k, _)) = best {
                        f = RIVER_BITS[k];
                        let (dx, dy) = DIRS[k];
                        self.or_d(col + dx, row + dy, RIVER_BITS[OPPOSITE[k]]);
                    }
                    for (k, &(dx, dy)) in DIRS.iter().enumerate() {
                        let (nc, nr) = (col + dx, row + dy);
                        if self.c_at(nc, nr) == 1 && self.a_at(nc, nr) != ac {
                            f |= RIVER_BITS[k];
                        }
                    }
                }
                // A river block opens towards neighbours whose order differs by one.
                // Owners are not compared.
                if bc != 0 {
                    for (k, &(dx, dy)) in DIRS.iter().enumerate() {
                        if (self.b_at(col + dx, row + dy) - bc).abs() == 1 {
                            f |= RIVER_BITS[k];
                        }
                    }
                }
                self.or_d(col, row, f);
            }
        }
    }

    /// The first direction from `d` (order (d + i) & 3) whose neighbour
    /// passes `test`.
    fn search(
        &self,
        col: i32,
        row: i32,
        d: u32,
        test: impl Fn(usize, i32, i32) -> bool,
    ) -> Option<usize> {
        (0..4u32)
            .map(|i| ((d + i) & 3) as usize)
            .find(|&k| test(k, col + DIRS[k].0, row + DIRS[k].1))
    }

    /// Chooses the exit directions of the attach points, whole grid in row-major order.
    /// Returns false when an attach point finds no river block, which restarts the
    /// placement; the draws made so far stay made.
    pub fn attach(&mut self, seed: &mut Seed) -> bool {
        for row in 0..self.h {
            for col in 0..self.w {
                // Search start direction, drawn for every block before any test.
                let mut d = seed.mask(4);
                let mut u = self.d_at(col, row);
                let empty = u == 0;
                let ac = self.a_at(col, row);
                if self.c_at(col, row) == 2 {
                    // Exit towards a river-code block of the same owner.
                    let first = self.search(col, row, d, |_, nc, nr| {
                        let nd = self.d_at(nc, nr);
                        self.a_at(nc, nr) == ac && 0 < nd && nd < 15
                    });
                    if let Some(k) = first {
                        u |= ATTACH_BITS[k];
                    }
                    // No exit at all: the placement restarts.
                    if u == 0 {
                        return false;
                    }
                    // A block that had no code of its own may get a second exit towards an
                    // attach point of another owner, and sometimes a third.
                    if empty {
                        let second = self.search(col, row, d, |_, nc, nr| {
                            self.a_at(nc, nr) != ac && self.c_at(nc, nr) == 2
                        });
                        if let Some(k) = second {
                            u |= ATTACH_BITS[k];
                        }
                        // The coin for a third exit is drawn first, then a new start direction.
                        let m = seed.mask(2);
                        let more = m == 1 && second.is_none();
                        d = seed.mask(4);
                        if more {
                            let third = self.search(col, row, d, |k, nc, nr| {
                                let nd = self.d_at(nc, nr);
                                0 < nd && nd < 15 && u & ATTACH_BITS[k] == 0
                            });
                            if let Some(k) = third {
                                u |= ATTACH_BITS[k];
                            }
                        }
                    }
                    // Mirror each exit onto the neighbour it leads to.
                    for k in 0..4 {
                        if u & ATTACH_BITS[k] != 0 {
                            let (dx, dy) = DIRS[k];
                            self.or_d(col + dx, row + dy, ATTACH_BITS[OPPOSITE[k]]);
                        }
                    }
                    self.set_c(col, row, 0);
                }
                self.or_d(col, row, u);
            }
        }
        true
    }
}

/// Maps a block code to its lvlprest id: a plain river block (codes below 16), a
/// clearing (low nibble 0), or a river with exits.
///
/// Exit bits chain through `EXIT_IDS`: each set bit looks up the previous result as the
/// row. A chain that lands on row 0 or row 15 ends as fatal `fatal::EXIT_ID`; a chain
/// that lands on an id row (545..572) is 0 in slots 0 and 2 and an error in slots 1 and 3,
/// reported as [`OutdoorError::Crash`] (the game stores a non-id there and later crashes);
/// a lookup in a larger row is fatal.
pub fn code_to_id(d: i32) -> Result<u32, OutdoorError> {
    if d == 0 {
        return Ok(0);
    }
    let l = d & 15;
    if l == 0 {
        // Codes of 16 and above with an empty low nibble: a clearing.
        let id = CLEARING_IDS[((d >> 4) & 15) as usize];
        if id == 0 {
            return Err(OutdoorError::Fatal(fatal::CLEARING_ID));
        }
        return Ok(id);
    }
    if d < 16 {
        return Ok(529 + l as u32);
    }
    let mut r = l as u32;
    for (slot, bit) in [0x10, 0x20, 0x40, 0x80].into_iter().enumerate() {
        if d & bit != 0 {
            r = match r {
                1..=14 => EXIT_IDS[r as usize - 1][slot],
                // Row 0 holds no ids.
                0 => 0,
                // Row 15 (low nibble 15 with an attach bit) holds no ids either, so the
                // chain ends as fatal `fatal::EXIT_ID`.
                15 => 0,
                545..=572 if slot % 2 == 0 => 0,
                545..=572 => return Err(OutdoorError::Crash(CRASH_EXIT_ID)),
                // A block id used as a row again: there is no such row.
                _ => return Err(OutdoorError::Fatal(fatal::EXIT_ID)),
            };
        }
    }
    if r == 0 {
        return Err(OutdoorError::Fatal(fatal::EXIT_ID));
    }
    Ok(r)
}

/// The [`OutdoorError::Crash`] text of a block id from row 545..572, slot 1 or 3 of the
/// exit table.
pub const CRASH_EXIT_ID: &str =
    "jungle block id V(r) from a T row past 14 is invalid (outdoor-act3-act5.md §2.7)";

/// The jungle placer's result: records in placement order and the level order (indices
/// into the records, level 76 first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Jungles {
    /// The three jungle records in placement order.
    pub recs: Vec<JungleRec>,
    /// Record index for levels 76, 77 and 78.
    pub order: [usize; 3],
    /// The shared block grids.
    pub grid: JungleGrid,
}

/// Places the three jungles on `seed` (the DRLG seed itself): the first sits above the
/// docks, the others attach to earlier ones. `docks` is level 75's rect; `sx`, `sy` the
/// jungle size (leveldefs 76).
pub fn place_jungles(
    seed: &mut Seed,
    docks: TileRect,
    sx: i32,
    sy: i32,
) -> Result<Jungles, OutdoorError> {
    let (sxb, syb) = (sx / BLOCK, sy / BLOCK);
    let (y1, y3) = jungle_offsets(sy);
    // The first jungle sits directly above the docks.
    let mut recs = vec![JungleRec {
        rect: TileRect::new(docks.x, docks.y - sy, sx, sy),
        ..JungleRec::default()
    }];
    let (mut min_x, mut max_x) = (docks.x, docks.x + sx);
    let (mut min_y, max_y) = (docks.y - sy, docks.y);
    // Place jungles 1 and 2: pick a base jungle and one of five relative positions, and
    // retry when the rectangle overlaps an earlier jungle.
    let mut k = 1i32;
    while k < 3 {
        // For k = 1 the roll gives 0 (only jungle 0 can be the base) but still steps the seed.
        let base = seed.roll(k) as usize;
        // The placement case.
        let case = (seed.step() % 5) as i32;
        let (ox, oy) = match case {
            0 => (0, -sy),
            1 => (-sx, y1),
            2 => (sx, y1),
            3 => (-sx, y3),
            _ => (sx, y3),
        };
        let b = recs[base].rect;
        let rect = TileRect::new(b.x + ox, b.y + oy, sx, sy);
        if recs.iter().any(|r| overlaps(&r.rect, &rect)) {
            continue;
        }
        if recs[base].branches.len() >= 3 {
            return Err(OutdoorError::Fatal(fatal::BRANCHES));
        }
        let new = recs.len();
        recs[base].branches.push(new);
        recs.push(JungleRec {
            rect,
            case,
            base,
            ..JungleRec::default()
        });
        min_x = min_x.min(rect.x);
        min_y = min_y.min(rect.y);
        max_x = max_x.max(rect.x + rect.w);
        k += 1;
    }
    // The combined span must be a whole number of blocks.
    if (max_x - min_x) % BLOCK != 0 {
        return Err(OutdoorError::Fatal(fatal::SPAN_X));
    }
    if (max_y - min_y) % BLOCK != 0 {
        return Err(OutdoorError::Fatal(fatal::SPAN_Y));
    }
    // The shared grid has a one-block ring around the combined area.
    let mut g = JungleGrid::new((max_x - min_x) / BLOCK + 2, (max_y - min_y) / BLOCK + 2);
    for r in &mut recs {
        r.bx = (r.rect.x - min_x) / BLOCK + 1;
        r.by = (r.rect.y - min_y) / BLOCK + 1;
    }
    // Rivers, connections and attach points; an attach failure restarts from a cleared
    // grid.
    loop {
        g.clear();
        for k in 0..recs.len() {
            river_and_attach(&mut g, seed, &recs, k, sxb, syb);
        }
        g.connect_rivers();
        if g.attach(seed) {
            break;
        }
    }
    // Convert codes to ids over the whole grid, then cut each jungle's ids out of it.
    let mut ids = Vec::with_capacity(g.d.len());
    for &d in &g.d {
        ids.push(code_to_id(d)?);
    }
    for r in &mut recs {
        r.ids = (0..sxb * syb)
            .map(|i| g.at(r.bx + i % sxb, r.by + i / sxb).map_or(0, |j| ids[j]))
            .collect();
        r.clearings = r.ids.iter().filter(|&&id| id > 574).count() as i32;
    }
    // Level order: bubble passes, swapping when the first y is smaller.
    let mut order = [0usize, 1, 2];
    loop {
        let mut swapped = false;
        for i in 0..2 {
            if recs[order[i]].rect.y < recs[order[i + 1]].rect.y {
                order.swap(i, i + 1);
                swapped = true;
            }
        }
        if !swapped {
            break;
        }
    }
    Ok(Jungles {
        recs,
        order,
        grid: g,
    })
}

/// Lays the river of jungle `k` into the shared grids: picks the river side, makes
/// passes until the jungle has enough attach points, then drops extras.
pub fn river_and_attach(
    g: &mut JungleGrid,
    seed: &mut Seed,
    recs: &[JungleRec],
    k: usize,
    sxb: i32,
    syb: i32,
) {
    let rec = &recs[k];
    let (bx, by) = (rec.bx, rec.by);
    // The side the river starts on: drawn for jungle 0, from the placement case otherwise.
    let mut s = if k == 0 {
        seed.mask(2) as i32
    } else {
        i32::from(rec.case % 2 == 1)
    };
    let opposite = |col: i32, s: i32| if s == 0 { col + 1 } else { col - 1 };
    let mut n;
    // Passes repeat while the jungle has fewer than two attach points.
    loop {
        // Claim the jungle's blocks and clear their river, mark and code.
        for row in by..by + syb {
            for col in bx..bx + sxb {
                if let Some(i) = g.at(col, row) {
                    g.a[i] = k as i32 + 1;
                    g.b[i] = 0;
                    g.c[i] = 0;
                    g.d[i] = 0;
                }
            }
        }
        let mut col = bx + s;
        let last = by + syb - 1;
        // A branch jungle links to its base at the bottom row.
        if k > 0 {
            g.set_c(col, last, 1);
            g.set_c(opposite(col, s), last, 2);
        }
        // Mark the link block of each branch, by its placement case.
        for &br in &rec.branches {
            let (cx, cy) = match recs[br].case {
                0 => (bx, by),
                1 => (bx, by + 3),
                2 => (bx + 1, by + 3),
                3 => (bx, by + 1),
                _ => (bx + 1, by + 1),
            };
            g.set_c(cx, cy, 1);
        }
        // The river runs up from the bottom row, numbered from 100 * (jungle + 1).
        n = i32::from(k > 0);
        let mut run = 0;
        let mut v = 100 * (k as i32 + 1);
        let mut row = last;
        while row >= by {
            g.set_b(col, row, v);
            v += 1;
            let advance = if run == 0 || row == by {
                true
            } else {
                // One in three steps switches the river to the other column.
                let t = seed.step() % 3;
                if t != 0 {
                    true
                } else {
                    run = 0;
                    if s == 0 {
                        col += 1;
                        s = 1;
                    } else {
                        col -= 1;
                        s = 0;
                    }
                    false
                }
            };
            if advance {
                run += 1;
                let opp = opposite(col, s);
                if run >= 2 && row >= 2 && g.c_at(opp, row) == 0 {
                    g.set_c(opp, row, 2);
                    n += 1;
                }
                row -= 1;
            }
        }
        if n >= 2 {
            break;
        }
    }
    // Drop random attach points while there are more than three.
    while n > 3 {
        let i = seed.roll(n) as usize;
        let hit = (by..by + syb)
            .flat_map(|row| (bx..bx + sxb).map(move |col| (col, row)))
            .filter(|&(col, row)| g.c_at(col, row) == 2)
            .nth(i);
        // n decrements only on a hit; a miss draws again. n always equals the count of
        // attach marks, so a miss is not reached.
        if let Some((col, row)) = hit {
            g.set_c(col, row, 0);
            n -= 1;
        }
    }
}

#[cfg(test)]
#[path = "act3_tests.rs"]
mod act3_tests;

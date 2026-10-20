//! Act V outdoor level build: the barricade border walk, the ravine, entrances, caves,
//! the connection to the siege strip, prisons and the special presets.
//!
//! `Gen::act5` runs these steps in order; the tables here hold the fixed preset ids.

use super::grid::{cell, Gen, Op};
use super::tilesub::BorderCtx;
use super::vertex::{border_piece, corner_piece};
use super::OutdoorError;

/// Ravine step direction for each of the 12 ravine pieces, indexed by `k`, the piece's
/// offset from the ravine base id.
pub const RAVINE_STEPS: [(i32, i32); 12] = [
    (-1, 0),
    (0, -1),
    (1, 0),
    (0, 1),
    (0, -1),
    (1, 0),
    (0, 1),
    (-1, 0),
    (-1, 0),
    (0, -1),
    (1, 0),
    (0, 1),
];

/// Cave rows: (level, file, far side, preset for a tall level, preset for a wide level).
pub const CAVES: [(u32, i32, bool, u32, u32); 3] = [
    (112, 0, false, 913, 914),
    (117, 0, true, 983, 984),
    (117, 0, false, 985, 986),
];

/// One row of the special-preset table: which preset to spawn on a level and how often.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpecialRow {
    /// Level the row applies to.
    pub level: u32,
    /// Preset used when the level is taller than wide.
    pub tall: u32,
    /// Preset used otherwise.
    pub wide: u32,
    /// File (variant) to spawn; -1 lets the placer choose.
    pub f: i32,
    /// Number of spawn attempts.
    pub count: u32,
    /// Whether a failed last attempt aborts the build.
    pub fatal: bool,
}

const fn row(level: u32, tall: u32, wide: u32, f: i32, count: u32, fatal: bool) -> SpecialRow {
    SpecialRow {
        level,
        tall,
        wide,
        f,
        count,
        fatal,
    }
}

/// The 15 special-preset rows in table order.
pub const SPECIALS: [SpecialRow; 15] = [
    row(111, 955, 956, 0, 1, true),
    row(112, 955, 956, 0, 1, true),
    row(117, 955, 956, 1, 1, true),
    row(112, 953, 953, -1, 1, true),
    row(117, 954, 954, -1, 1, true),
    row(111, 944, 947, -1, 1, false),
    row(111, 942, 945, -1, 4, false),
    row(111, 943, 946, -1, 4, false),
    row(112, 941, 941, -1, 1, false),
    row(112, 939, 939, -1, 1, false),
    row(112, 940, 940, -1, 5, false),
    row(117, 948, 948, -1, 4, false),
    row(117, 949, 949, -1, 4, false),
    row(117, 950, 950, -1, 4, false),
    row(117, 951, 951, -1, 3, false),
];

/// Fatal error code for a special-preset row whose last spawn failed.
pub const FATAL_SPECIAL: u32 = 0x219;
/// Fatal error code for a level 111 build that places fewer than 3 prisons.
pub const FATAL_PRISONS: u32 = 0x259;
/// [`OutdoorError::Crash`] message for a ravine walk that leaves the barricade pieces.
pub const CRASH_RAVINE_WALK: &str =
    "ravine walk off the barricade pieces (outdoor.md §11 step 3: non-row stamp or endless walk)";
/// [`OutdoorError::Crash`] message for a barricade straight walk that never reaches its end.
/// Not reachable on the axis-aligned level polygons.
pub const CRASH_BARRICADE_WALK: &str =
    "barricade straight walk that never meets its end (outdoor.md §11 step 2)";

impl Gen<'_> {
    /// Builds an Act V outdoor level: link flags, barricade borders, ravine, entrances,
    /// caves, the siege connection (level 111), border substitution, prisons and specials.
    /// Level 110 is only the siege strip.
    pub fn act5(&mut self) -> Result<(), OutdoorError> {
        if self.id == 110 {
            // Siege strip: 15 presets laid right to left along the top edge.
            let s = self.od.preset(865)?.size_x / 8;
            for i in 0..15 {
                let x = self.gw() - s * (i + 1);
                if x < 0 {
                    return Err(OutdoorError::SiegeStrip(865 + i as u32));
                }
                self.stamp(x, 0, 865 + i as u32, 0, false)?;
            }
            return Ok(());
        }
        self.link_flags()?;
        self.barricade_borders()?;
        self.ravine()?;
        self.entrances()?;
        self.caves()?;
        if self.id == 111 {
            self.connect_to_siege()?;
        }
        // Border substitution for the barricade pieces.
        self.border_sub(BorderCtx::barricade())?;
        if self.id == 111 {
            self.prisons()?;
        }
        self.act5_specials()
    }

    /// Walks the level polygon and stamps barricade border and corner pieces along each
    /// edge, marking link cells.
    pub fn barricade_borders(&mut self) -> Result<(), OutdoorError> {
        let s = 4 + i32::from(self.id == 117);
        let count = self.info.vertices.len();
        for i in 0..count {
            let v = self.info.vertices[i];
            let n = self.info.vertices[(i + 1) % count];
            let nn = self.info.vertices[(i + 2) % count];
            // Sign of each component of this edge and of the next one.
            let (dx, dy) = ((n.x - v.x).signum(), (n.y - v.y).signum());
            let (ndx, ndy) = ((nn.x - n.x).signum(), (nn.y - n.y).signum());
            let (vx, vy) = (v.x & !1, v.y & !1);
            let (nx, ny) = (n.x & !1, n.y & !1);
            if !v.is_preset_link() {
                let piece = border_piece(dx, dy, s);
                // Test (x, y) = (nx, ny) before every step. Edges are axis-aligned and
                // both ends have bit 0 cleared, so the walk always meets (nx, ny); a
                // guard past the grid size reports the (unreached) endless walk.
                let (mut x, mut y) = (vx, vy);
                let mut steps = 0;
                while (x, y) != (nx, ny) {
                    steps += 1;
                    if steps > self.gw() + self.gh() {
                        return Err(OutdoorError::Crash(CRASH_BARRICADE_WALK));
                    }
                    x += 2 * dx;
                    y += 2 * dy;
                    if piece != 0 {
                        self.stamp(x, y, piece, -1, false)?;
                    }
                    self.op(2, x, y, Op::Or, cell::BORDER);
                }
            }
            if v.is_link() {
                let x = (v.x.max(n.x) - 4 * dx.abs()) & !1;
                let y = (v.y.max(n.y) - 4 * dy.abs()) & !1;
                self.op(2, x, y, Op::Or, cell::LINK);
                self.op(2, x + 2 * dx.abs(), y + 2 * dy.abs(), Op::Or, cell::LINK);
            }
            let piece = corner_piece(2 * dx, 2 * dy, 2 * ndx, 2 * ndy, s);
            if piece != 0 {
                self.stamp(nx, ny, piece, -1, false)?;
                self.op(2, nx, ny, Op::Or, cell::BORDER);
            }
        }
        if self.id == 111 {
            let (gw, gh) = (self.gw(), self.gh());
            self.op(2, gw - 2, gh - 4, Op::Or, cell::LINK);
            self.op(2, gw - 2, gh - 3, Op::Or, cell::LINK);
        }
        Ok(())
    }

    /// Follows the ravine from the top-right to the bottom-left of the grid, stamping the
    /// matching ravine piece on each cell and then the start and end pieces.
    pub fn ravine(&mut self) -> Result<(), OutdoorError> {
        let (b, b2, start, end) = if self.id == 117 {
            (957, 969, 982, 981)
        } else {
            (881, 893, 906, 905)
        };
        let (gw, gh) = (self.gw(), self.gh());
        let (mut x, mut y) = (gw - 2, 0);
        let mut steps = 0;
        while (x, y) != (0, gh - 2) {
            let k = self.g(0, x, y) as i32 - b;
            steps += 1;
            if !(0..12).contains(&k) || steps > gw * gh {
                // An out-of-range k would stamp a non-ravine id, or the walk would never
                // end. Real data never gets here: every visited cell holds base + 0..11.
                return Err(OutdoorError::Crash(CRASH_RAVINE_WALK));
            }
            self.stamp(x, y, (b2 + k) as u32, -1, false)?;
            let (ddx, ddy) = RAVINE_STEPS[k as usize];
            x += 2 * ddx;
            y += 2 * ddy;
        }
        self.stamp(gw - 2, 0, start, -1, false)?;
        self.stamp(0, gh - 2, end, -1, false)
    }

    /// Stamps an entrance piece on each of the four edges at the first link cell found.
    pub fn entrances(&mut self) -> Result<(), OutdoorError> {
        let (gw, gh) = (self.gw(), self.gh());
        let link = |g: &Self, x, y| g.g(2, x, y) & cell::LINK != 0;
        if let Some(x) = (0..gw).find(|&x| link(self, x, 0)) {
            self.stamp(x, 0, 909, -1, false)?;
        }
        if let Some(x) = (0..gw).find(|&x| link(self, x, gh - 2)) {
            self.stamp(x, gh - 2, 908, -1, false)?;
        }
        if let Some(y) = (0..gh).find(|&y| link(self, 0, y)) {
            self.stamp(0, y, 910, -1, false)?;
        }
        if let Some(y) = (0..gh).find(|&y| link(self, gw - 2, y)) {
            self.stamp(gw - 2, y, 907, -1, false)?;
        }
        Ok(())
    }

    /// Stamps the cave entrance rows that apply to this level, using the wide or tall
    /// preset by the level's shape.
    pub fn caves(&mut self) -> Result<(), OutdoorError> {
        let (gw, gh) = (self.gw(), self.gh());
        let wide = self.rect.w > self.rect.h;
        for &(level, f, side, tall_p, wide_p) in &CAVES {
            if level != self.id {
                continue;
            }
            if wide {
                self.stamp(if side { gw - 2 } else { 0 }, 2, wide_p, f, false)?;
            } else {
                self.stamp(2, if side { gh - 2 } else { 0 }, tall_p, f, false)?;
            }
        }
        Ok(())
    }

    /// Level 111 only: stamps the connection to the siege strip at the bottom-right corner.
    pub fn connect_to_siege(&mut self) -> Result<(), OutdoorError> {
        let (sw, sh) = self.preset_cells(880)?;
        let x = self.gw() - sw;
        let y = self.gh() - sh;
        self.stamp(x, y, 880, -1, false)?;
        self.stamp(x, y - 2, 896, -1, false)
    }

    /// Preset id at a cell if it holds a preset, else 0.
    fn prison_value(&self, x: i32, y: i32) -> u32 {
        if self.g(2, x, y) & cell::PRESET != 0 {
            self.g(0, x, y)
        } else {
            0
        }
    }

    /// Replaces a prison preset (915..=922) at the cell with its variant 16 ids higher;
    /// true when placed.
    fn prison_at(&mut self, x: i32, y: i32) -> Result<bool, OutdoorError> {
        let v = self.prison_value(x, y);
        if (915..=922).contains(&v) {
            self.stamp(x, y, v + 16, -1, false)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Level 111 only: converts three prison presets, first by random probes (up to 90),
    /// then by scanning the grid from a random offset. Fails when fewer than 3 are found.
    pub fn prisons(&mut self) -> Result<(), OutdoorError> {
        let (gw, gh) = (self.gw(), self.gh());
        let mut placed = 0;
        let mut tries = 0;
        while tries < 90 && placed < 3 {
            tries += 1;
            // Roll x, then y.
            let x = 2 * self.seed().roll(gw / 2) as i32;
            let y = 2 * self.seed().roll(gh / 2) as i32;
            if self.prison_at(x, y)? {
                placed += 1;
            }
        }
        // The scan offset is always drawn, even when the probes already placed three.
        let sx = self.seed().roll(gw / 2) as i32;
        let sy = self.seed().roll(gh / 2) as i32;
        'scan: for i in 0..gh {
            for j in 0..gw {
                if placed >= 3 {
                    break 'scan;
                }
                let x = (j + 2 * sx) % gw;
                let y = (i + 2 * sy) % gh;
                if self.prison_at(x, y)? {
                    placed += 1;
                }
            }
        }
        if placed < 3 {
            return Err(OutdoorError::Fatal(FATAL_PRISONS));
        }
        Ok(())
    }

    /// Spawns the special presets listed for this level in `SPECIALS`.
    pub fn act5_specials(&mut self) -> Result<(), OutdoorError> {
        let tall = self.rect.w < self.rect.h;
        let id = self.id;
        for r in SPECIALS.iter().filter(|r| r.level == id) {
            let p = if tall { r.tall } else { r.wide };
            let mut last = true;
            for _ in 0..r.count {
                last = self.spawn_preset(p, r.f, 0, 15)?;
            }
            if !last && r.fatal {
                return Err(OutdoorError::Fatal(FATAL_SPECIAL));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "act5_tests.rs"]
mod act5_tests;

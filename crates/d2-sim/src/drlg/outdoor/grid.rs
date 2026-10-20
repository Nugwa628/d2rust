//! Cell grids for outdoor levels, the generation context `Gen`, preset stamping with its
//! per-preset file build list, fit tests, the shuffled cell list and the placers
//! (spawn, random, far-away, waypoint, shrines).
//!
//! Every level builder in the sibling files (`wild`, `acts`, `act3`, `act5`, `kurast`)
//! works through `Gen`. A level is an 8×8-tile cell grid; grid 0 holds the preset id
//! stamped at a cell, grid 1 per-cell room flags, grid 2 the cell bits in `cell`.

use crate::rng::Seed;

use super::super::data::DrlgData;
use super::super::level::Drlg;
use super::super::{LevelIdx, TileRect};
use super::{BuildNode, OutdoorData, OutdoorError, OutdoorLevel, SubFiles};

/// Bits of a grid-2 cell.
pub mod cell {
    /// The cell lies on a level border.
    pub const BORDER: u32 = 0x1;
    /// The cell carries a direction mark.
    pub const DIRECTION: u32 = 0x2;
    /// The cell is on a path.
    pub const PATH: u32 = 0x80;
    /// The cell is blank.
    pub const BLANK: u32 = 0x100;
    /// A preset was stamped on the cell.
    pub const PRESET: u32 = 0x200;
    /// The cell is part of a link to another level.
    pub const LINK: u32 = 0x400;
    /// The level waypoint sits on the cell.
    pub const WAYPOINT: u32 = 0x800;
    /// A shrine sits on the cell.
    pub const SHRINE: u32 = 0x1000;
    /// The file (variant) picked for the stamped preset, in bits 16..19.
    pub const FILE_MASK: u32 = 0xF0000;
    /// Bits that make a cell not "spawn valid": border, path, blank, waypoint and shrine.
    pub const NOT_SPAWN: u32 = 0x1B81;
}

/// How `Grid::op` combines a value with a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// Cell | value.
    Or,
    /// Cell & value.
    And,
    /// Cell ^ value.
    Xor,
    /// Cell = value.
    Set,
    /// Cell = value only when the cell is 0.
    SetIfZero,
    /// Cell & !value.
    AndNot,
}

/// One cell grid (gw × gh u32 cells, row-major).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Grid {
    /// Width in cells.
    pub w: i32,
    /// Height in cells.
    pub h: i32,
    /// The cells, `w * h` of them.
    pub cells: Vec<u32>,
}

impl Grid {
    /// A zeroed grid of `w` × `h` cells.
    pub fn new(w: i32, h: i32) -> Self {
        let n = (w.max(0) * h.max(0)) as usize;
        Self {
            w,
            h,
            cells: vec![0; n],
        }
    }

    /// True when `(x, y)` is inside the grid.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.w && y < self.h
    }

    /// The cell, or 0 outside the grid.
    pub fn get(&self, x: i32, y: i32) -> u32 {
        if self.contains(x, y) {
            self.cells[(y * self.w + x) as usize]
        } else {
            0
        }
    }

    /// Applies an op to an in-grid cell; outside the grid nothing.
    pub fn op(&mut self, x: i32, y: i32, op: Op, v: u32) {
        if !self.contains(x, y) {
            return;
        }
        let c = &mut self.cells[(y * self.w + x) as usize];
        *c = match op {
            Op::Or => *c | v,
            Op::And => *c & v,
            Op::Xor => *c ^ v,
            Op::Set => v,
            Op::SetIfZero if *c == 0 => v,
            Op::SetIfZero => *c,
            Op::AndNot => *c & !v,
        };
    }
}

/// True when a grid-2 cell is free for spawning: none of the `cell::NOT_SPAWN` bits set.
pub fn spawn_valid(c: u32) -> bool {
    c & cell::NOT_SPAWN == 0
}

/// The picked file of a grid-2 cell (bits 16..19).
pub fn file_of(c: u32) -> i32 {
    ((c >> 16) & 0xF) as i32
}

/// Level generation context: the level, its outdoor info and the data.
pub struct Gen<'a> {
    /// The DRLG the level belongs to.
    pub drlg: &'a mut Drlg,
    /// Level and preset data tables.
    pub data: &'a DrlgData,
    /// Outdoor table view (preset sizes and file counts, lvlsub rows).
    pub od: &'a OutdoorData,
    /// Parsed substitution files.
    pub subs: &'a dyn SubFiles,
    /// Index of the level being built.
    pub level: LevelIdx,
    /// Level id of the level being built.
    pub id: u32,
    /// Level rect (tiles).
    pub rect: TileRect,
    /// The outdoor state being filled in.
    pub info: &'a mut OutdoorLevel,
}

impl Gen<'_> {
    /// The level's random seed; every placement roll draws from it.
    pub fn seed(&mut self) -> &mut Seed {
        &mut self.drlg.level_mut(self.level).seed
    }

    /// Grid width in cells.
    pub fn gw(&self) -> i32 {
        self.info.gw()
    }

    /// Grid height in cells.
    pub fn gh(&self) -> i32 {
        self.info.gh()
    }

    /// Reads a cell of grid `grid` (0 outside the grid).
    pub fn g(&self, grid: usize, x: i32, y: i32) -> u32 {
        self.info.grids[grid].get(x, y)
    }

    /// Applies `op` with value `v` to a cell of grid `grid`.
    pub fn op(&mut self, grid: usize, x: i32, y: i32, op: Op, v: u32) {
        self.info.grids[grid].op(x, y, op, v);
    }

    /// True when `(x, y)` is inside the level grid.
    pub fn in_grid(&self, x: i32, y: i32) -> bool {
        self.info.grids[2].contains(x, y)
    }

    /// Size of a preset in cells (`SizeX/8`, `SizeY/8`).
    pub fn preset_cells(&self, p: u32) -> Result<(i32, i32), OutdoorError> {
        let d = self.od.preset(p)?;
        Ok((d.size_x / 8, d.size_y / 8))
    }

    /// The next file (variant) of preset `p`, cycling through its files in order.
    ///
    /// The first request for a preset adds it to the build list with a random starting
    /// file; later requests step on from there.
    pub fn next_file(&mut self, p: u32) -> Result<i32, OutdoorError> {
        let pos = match self.info.build_list.iter().position(|n| n.preset == p) {
            Some(i) => i,
            None => {
                let n = self.od.preset(p)?.files;
                // Random starting file for a preset seen for the first time.
                let r = self.seed().roll(n) as i32;
                self.info.build_list.insert(
                    0,
                    BuildNode {
                        preset: p,
                        files: n,
                        current: r,
                    },
                );
                0
            }
        };
        let node = &mut self.info.build_list[pos];
        if node.files == 0 {
            return Err(OutdoorError::NoFiles(p));
        }
        // Signed remainder.
        node.current = (node.current + 1) % node.files;
        Ok(node.current)
    }

    /// Stamps preset `p` onto the grids with its top-left at `(x, y)`. `f` = −1 takes the
    /// next file from the build list. With `border` set, presets 4..=15 and 364..=375 also
    /// mark their cells as border.
    pub fn stamp(
        &mut self,
        x: i32,
        y: i32,
        p: u32,
        f: i32,
        border: bool,
    ) -> Result<(), OutdoorError> {
        let f = if f == -1 { self.next_file(p)? } else { f };
        let (sw, sh) = self.preset_cells(p)?;
        let border_bit = border && (matches!(p, 4..=15) || matches!(p, 364..=375));
        for j in 0..sh {
            for i in 0..sw {
                let (cx, cy) = (x + i, y + j);
                self.op(2, cx, cy, Op::AndNot, cell::FILE_MASK);
                self.op(2, cx, cy, Op::Or, cell::PRESET | ((f as u32) << 16));
                if border_bit {
                    self.op(2, cx, cy, Op::Or, cell::BORDER);
                }
                self.op(0, cx, cy, Op::Set, 0);
            }
        }
        self.op(0, x, y, Op::Set, p);
        Ok(())
    }

    /// True when preset `p` placed at `(x, y)` lies inside the grid on spawn-valid cells.
    /// `p` = 0 means a 1×1 area. A margin `m` widens the tested area on the sides chosen
    /// by bits of `flags` (1 top, 2 right, 4 bottom, 8 left).
    pub fn fits(&self, x: i32, y: i32, p: u32, m: i32, flags: u32) -> Result<bool, OutdoorError> {
        let (mut w, mut h) = if p == 0 {
            (1, 1)
        } else {
            self.preset_cells(p)?
        };
        let (mut x, mut y) = (x, y);
        if m != 0 {
            if flags & 1 != 0 {
                y -= m;
                h += m;
            }
            if flags & 2 != 0 {
                w += m;
            }
            if flags & 4 != 0 {
                h += m;
            }
            if flags & 8 != 0 {
                x -= m;
                w += m;
            }
        }
        for j in 0..h {
            for i in 0..w {
                let (cx, cy) = (x + i, y + j);
                if !self.in_grid(cx, cy) || !spawn_valid(self.g(2, cx, cy)) {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// All cells inside the one-cell border, in a random order drawn from the level seed.
    pub fn shuffle(&mut self) -> Vec<(i32, i32)> {
        let (w, h) = (self.gw() - 2, self.gh() - 2);
        shuffle_cells(self.seed(), w, h)
            .into_iter()
            .map(|(x, y)| (x + 1, y + 1))
            .collect()
    }

    /// Stamps preset `p` at the first cell of a shuffled scan where it fits. Returns false
    /// when it fits nowhere.
    pub fn spawn_preset(
        &mut self,
        p: u32,
        f: i32,
        m: i32,
        flags: u32,
    ) -> Result<bool, OutdoorError> {
        for (x, y) in self.shuffle() {
            if self.fits(x, y, p, m, flags)? {
                self.stamp(x, y, p, f, false)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Short form of `spawn_preset(p, -1, 0, 15)`.
    pub fn s(&mut self, p: u32) -> Result<bool, OutdoorError> {
        self.spawn_preset(p, -1, 0, 15)
    }

    /// Stamps preset `p` next to a random path cell. Falls back to `spawn_preset` when no
    /// path cell has a fitting neighbour.
    pub fn random_ds1(&mut self, p: u32, f: i32) -> Result<(), OutdoorError> {
        const DX: [i32; 8] = [-1, 0, 0, 1, -1, 1, 1, -1];
        const DY: [i32; 8] = [0, -1, 1, 0, -1, 1, -1, 1];
        for (x, y) in self.shuffle() {
            if self.g(2, x, y) & cell::PATH == 0 {
                continue;
            }
            for k in 0..8 {
                let (nx, ny) = (x + DX[k], y + DY[k]);
                if self.fits(nx, ny, p, 0, 15)? {
                    return self.stamp(nx, ny, p, f, false);
                }
            }
        }
        // No path cell with a fitting neighbour among all candidates; the scan above
        // continues past a path cell whose neighbours do not fit.
        self.spawn_preset(p, f, 0, 15)?;
        Ok(())
    }

    /// Short form of `random_ds1(p, -1)`.
    pub fn r(&mut self, p: u32) -> Result<(), OutdoorError> {
        self.random_ds1(p, -1)
    }

    /// Stamps preset `p` at the fitting cell farthest from the centre of `rect`. Returns
    /// false when it fits nowhere.
    pub fn far_away(
        &mut self,
        rect: TileRect,
        p: u32,
        f: i32,
        m: i32,
        flags: u32,
    ) -> Result<bool, OutdoorError> {
        let (gw, gh) = (self.gw(), self.gh());
        let rx = self.seed().roll(gw - 2) as i32;
        let ry = self.seed().roll(gh - 2) as i32;
        let (w, h) = (gw - 2, gh - 2);
        if w <= 0 || h <= 0 {
            // A grid without interior cells has nowhere to place; real level sizes never
            // get here.
            return Ok(false);
        }
        let (cx, cy) = (rect.x + rect.w / 2, rect.y + rect.h / 2);
        let mut best: Option<(i32, i32, i32)> = None;
        // Both loops are inclusive, so the scan visits one extra row and column of
        // (wrapped) candidates.
        for i in 0..=h {
            for j in 0..=w {
                let x = (j + rx) % w + 1;
                let y = (i + ry) % h + 1;
                if !self.fits(x, y, p, m, flags)? {
                    continue;
                }
                let ax = (8 * x - cx + self.rect.x + 4).abs();
                let ay = (8 * y - cy + self.rect.y + 4).abs();
                let d = if ax > ay { ay + 2 * ax } else { ax + 2 * ay } / 2;
                if best.is_none_or(|(_, _, bd)| d > bd) {
                    best = Some((x, y, d));
                }
            }
        }
        match best {
            Some((x, y, _)) => {
                self.stamp(x, y, p, f, false)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Places the level waypoint. Level 3 first tries a link cell matching its vis entry;
    /// every other case takes the first spawn-valid cell of a shuffled scan.
    pub fn waypoint(&mut self) -> Result<(), OutdoorError> {
        const WP: u32 = super::super::room_flags::WAYPOINT;
        const WP_SMALL: u32 = super::super::room_flags::WAYPOINT_SMALL;
        if self.id == 3 {
            let vis = self.drlg.vis_array(self.data, 3)?;
            let i = vis.iter().position(|&v| v == 2).unwrap_or(8) as u32;
            let mask = 1u32 << (i + 4);
            let (gw, gh) = (self.gw(), self.gh());
            for y in 0..gh {
                for x in 0..gw {
                    if self.g(1, x, y) & mask != 0 && self.g(2, x, y) & cell::LINK != 0 {
                        let x = x.clamp(1, gw - 2);
                        let y = y.clamp(1, gh - 2);
                        self.op(1, x, y, Op::Or, WP_SMALL);
                        self.op(2, x, y, Op::Or, cell::WAYPOINT);
                        return Ok(());
                    }
                }
            }
        }
        for (x, y) in self.shuffle() {
            if spawn_valid(self.g(2, x, y)) {
                self.op(1, x, y, Op::Or, WP);
                self.op(2, x, y, Op::Or, cell::WAYPOINT);
                break;
            }
        }
        Ok(())
    }

    /// Places up to `n` shrines on spawn-valid cells of a shuffled scan, cycling through
    /// four shrine kinds from a random start.
    pub fn shrines(&mut self, n: i32) {
        const BITS: [u32; 4] = [0x1000, 0x2000, 0x4000, 0x8000];
        // The starting kind is drawn even when no shrine ends up placed.
        let mut k = self.seed().mask(4) as usize;
        let mut n = n;
        for (x, y) in self.shuffle() {
            if n <= 0 {
                break;
            }
            if spawn_valid(self.g(2, x, y)) {
                self.op(1, x, y, Op::Or, BITS[k]);
                self.op(2, x, y, Op::Or, cell::SHRINE);
                k = (k + 1) % 4;
                n -= 1;
            }
        }
    }
}

/// Shuffles the `w` × `h` cells: entries (k mod w, k div w) for k < w·h, then for each k
/// two rolls of w·h and a swap of those entries. An empty area yields nothing and draws
/// no rolls.
pub fn shuffle_cells(seed: &mut Seed, w: i32, h: i32) -> Vec<(i32, i32)> {
    if w <= 0 || h <= 0 {
        // Non-positive sizes (including both negative) yield nothing.
        return Vec::new();
    }
    let a = w * h;
    let mut e: Vec<(i32, i32)> = (0..a).map(|k| (k % w, k / w)).collect();
    for _ in 0..a {
        let i = seed.roll(a) as usize;
        let j = seed.roll(a) as usize;
        e.swap(i, j);
    }
    e
}

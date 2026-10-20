//! Kurast and Travincal stamps: the border rows of levels 79..=81, fixed and random
//! presets, the Kurast Causeway level and Travincal.
//!
//! Called from the Act III build in `act3.rs`.

use super::grid::{shuffle_cells, Gen};
use super::OutdoorError;

/// Border-row preset ids of one Kurast level.
struct Border {
    /// Sides: (gw−1, i), then (0, i).
    side_e: u32,
    side_w: u32,
    /// Corners in stamp order: (0,0), (gw−1,0), (0,gh−1), (gw−1,gh−1).
    corners: [u32; 4],
}

impl Gen<'_> {
    /// Builds the Kurast levels 79..=82 with their border rows; other levels get nothing.
    pub fn kurast(&mut self) -> Result<(), OutdoorError> {
        let (gw, gh) = (self.gw(), self.gh());
        let (x, y) = (gw - 4, gh - 4);
        match self.id {
            79 => {
                self.kurast_border()?;
                self.spawn_preset(631, 0, 0, 15)?;
                self.random_presets(618, 618, 4)?;
                self.random_presets(616, 617, 0)?;
                self.random_presets(615, 615, 0)?;
            }
            80 => {
                self.kurast_border()?;
                self.stamp(3, 3, 629, 0, false)?;
                self.stamp(x, 3, 629, 1, false)?;
                self.spawn_preset(630, 0, 0, 15)?;
                self.spawn_preset(630, 1, 0, 15)?;
                self.spawn_preset(631, 0, 0, 15)?;
                self.random_presets(635, 635, 4)?;
                self.random_presets(633, 634, 0)?;
                self.random_presets(632, 632, 0)?;
            }
            81 => {
                self.kurast_border()?;
                self.stamp(3, y, 646, 0, false)?;
                self.stamp(x, y, 646, 1, false)?;
                self.spawn_preset(647, 0, 0, 15)?;
                self.spawn_preset(647, 1, 0, 15)?;
                self.spawn_preset(631, 0, 0, 15)?;
                self.random_presets(651, 651, 4)?;
                self.random_presets(649, 650, 0)?;
                self.random_presets(648, 648, 0)?;
            }
            82 => self.stamp(0, 0, 652, 0, false)?,
            _ => {}
        }
        Ok(())
    }

    /// Border rows of levels 79..=81, all with file -1. Rows go in the order top, bottom,
    /// sides, corners.
    fn kurast_border(&mut self) -> Result<(), OutdoorError> {
        let (gw, gh) = (self.gw(), self.gh());
        let j = self.drlg.jungle_link;
        let b = match self.id {
            79 => {
                let t = if j { 1 } else { gw - 2 };
                for i in 1..=gw - 2 {
                    self.stamp(i, 0, if i == t { 613 } else { 605 }, -1, false)?;
                }
                self.skip_row(gh - 1, 606, 614)?;
                Border {
                    side_e: 607,
                    side_w: 608,
                    corners: [610, 609, 612, 611],
                }
            }
            80 => {
                let (a, bb) = if j { (1, gw - 2) } else { (gw - 2, 1) };
                for i in 1..=gw - 2 {
                    self.stamp(i, 0, if i == bb { 627 } else { 619 }, -1, false)?;
                    self.stamp(i, gh - 1, if i == a { 628 } else { 620 }, -1, false)?;
                }
                Border {
                    side_e: 621,
                    side_w: 622,
                    corners: [624, 623, 626, 625],
                }
            }
            81 => {
                self.skip_row(0, 636, 644)?;
                let t = if j { gw - 2 } else { 1 };
                for i in 1..=gw - 2 {
                    self.stamp(i, gh - 1, if i == t { 645 } else { 637 }, -1, false)?;
                }
                Border {
                    side_e: 638,
                    side_w: 639,
                    corners: [641, 640, 643, 642],
                }
            }
            _ => return Ok(()),
        };
        for i in 1..=gh - 2 {
            self.stamp(gw - 1, i, b.side_e, -1, false)?;
            self.stamp(0, i, b.side_w, -1, false)?;
        }
        let cells = [(0, 0), (gw - 1, 0), (0, gh - 1), (gw - 1, gh - 1)];
        for (&(cx, cy), &p) in cells.iter().zip(b.corners.iter()) {
            self.stamp(cx, cy, p, -1, false)?;
        }
        Ok(())
    }

    /// A border row with a wide piece: stamps `p` for i from 1 while i < gw − 1, or `mid`
    /// at i = (gw − 1)/2, which also skips the next i.
    fn skip_row(&mut self, y: i32, p: u32, mid: u32) -> Result<(), OutdoorError> {
        let gw = self.gw();
        // Signed division truncates toward zero.
        let half = (gw - 1) / 2;
        let mut i = 1;
        while i < gw - 1 {
            if i == half {
                self.stamp(i, y, mid, -1, false)?;
                i += 1;
            } else {
                self.stamp(i, y, p, -1, false)?;
            }
            i += 1;
        }
        Ok(())
    }

    /// Places random presets from `lo..=hi`: shuffles the full grid, rolls one preset per
    /// tried cell, and stamps it (file -1) where it fits. `max` = 0 means no limit.
    pub fn random_presets(&mut self, lo: u32, hi: u32, max: i32) -> Result<(), OutdoorError> {
        let n = hi as i32 - lo as i32 + 1;
        let (gw, gh) = (self.gw(), self.gh());
        // An empty grid yields nothing (`shuffle_cells` returns no entries).
        let cells = shuffle_cells(self.seed(), gw, gh);
        let mut count = 0;
        for (x, y) in cells {
            let p = lo + self.seed().roll(n);
            if self.fits(x, y, p, 0, 15)? {
                self.stamp(x, y, p, -1, false)?;
                count += 1;
                if max > 0 && count >= max {
                    break;
                }
            }
        }
        Ok(())
    }

    /// Stamps the six Travincal presets on level 83; other levels get nothing.
    pub fn travincal(&mut self) -> Result<(), OutdoorError> {
        if self.id != 83 {
            return Ok(());
        }
        const STAMPS: [(i32, i32, u32); 6] = [
            (0, 0, 653),
            (2, 0, 654),
            (6, 0, 655),
            (0, 4, 656),
            (2, 4, 657),
            (6, 4, 658),
        ];
        for (x, y, p) in STAMPS {
            self.stamp(x, y, p, -1, false)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "kurast_tests.rs"]
mod kurast_tests;

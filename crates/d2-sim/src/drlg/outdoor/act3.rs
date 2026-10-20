//! Act III outdoor level build: link flags, jungle stamping for the three jungle levels
//! (76..=78), then Kurast and Travincal (`kurast.rs`).
//!
//! The jungle levels are tiled from a per-level id grid chosen at level creation; this
//! file reads that grid and stamps the head, tail and clearing presets.

use super::grid::Gen;
use super::jungle::{fatal, BLOCK};
use super::OutdoorError;

/// lvlprest 573 "Jungle Head" (64×32, Files 0).
pub const JUNGLE_HEAD: u32 = 573;
/// lvlprest 574 "Jungle Tail" (64×32, Files 0).
pub const JUNGLE_TAIL: u32 = 574;
/// Offset added to a clearing preset id for levels 76..=78 (Webby, Boggy, Pygmy).
pub const CLEARING_FAMILY: [u32; 3] = [0, 10, 20];
/// File (variant) order for the three clearings of a jungle level: six orders of three,
/// indexed `3 * r + clearing` where `r` is the order chosen for the level.
pub const CLEARING_FILES: [i32; 18] = [0, 1, 2, 1, 0, 2, 0, 2, 1, 1, 2, 0, 2, 0, 1, 2, 1, 0];

impl Gen<'_> {
    /// Builds an Act III outdoor level: link flags, jungle stamping on levels 76..=78,
    /// then Kurast and Travincal.
    pub fn act3(&mut self) -> Result<(), OutdoorError> {
        self.link_flags()?;
        if (76..=78).contains(&self.id) {
            self.jungle_stamping()?;
        }
        self.kurast()?;
        self.travincal()
    }

    /// Stamps a jungle level from its id grid: head and tail presets at the ends, clearing
    /// presets where the grid holds a clearing id, and plain jungle blocks elsewhere.
    pub fn jungle_stamping(&mut self) -> Result<(), OutdoorError> {
        let diff = (self.drlg.difficulty as usize).min(2);
        let (sx, sy) = self.data.level(76)?.size[diff];
        let (sxb, syb) = (sx / BLOCK, sy / BLOCK);
        // Pick which of the clearing file orders this level uses. The roll is drawn before
        // the id-grid check below.
        let n = 2 + 4 * i32::from(self.info.jungle_clearings == 3);
        let r = self.seed().roll(n) as usize;
        // The id grid chosen at level creation.
        let ids = self
            .info
            .jungle_ids
            .clone()
            .ok_or(OutdoorError::Fatal(fatal::NO_IDS))?;
        // Creation and build read leveldefs 76 at the same difficulty,
        // so the array always holds SXb·SYb ids; a shorter one is a D2RUST
        // error, never a valid layout.
        let want = (sxb * syb).max(0) as usize;
        if ids.len() < want {
            return Err(OutdoorError::JungleIdsShort(ids.len(), want));
        }
        let id_at = |i: i32| ids[i as usize];
        // Walk the block rows. The last row of level 76 holds the head and the first row
        // of level 78 holds the tail; every other cell is a plain block or a clearing.
        let mut idx = 0i32;
        let mut c = 0usize;
        for i in 0..syb {
            if self.id == 76 && i == syb - 1 {
                let f = i32::from(id_at(sxb * syb - 1) == 0);
                self.stamp(0, 4 * i, JUNGLE_HEAD, f, false)?;
                idx += 2;
            } else if self.id == 78 && i == 0 {
                let f = i32::from(id_at(1) == 0);
                self.stamp(0, 0, JUNGLE_TAIL, f, false)?;
                idx += 2;
            } else {
                for j in 0..sxb {
                    let mut p = id_at(idx);
                    idx += 1;
                    let mut f = -1;
                    if p > JUNGLE_TAIL {
                        if c >= 3 {
                            return Err(OutdoorError::Fatal(fatal::CLEARINGS));
                        }
                        // Clearing family per level: Webby, Boggy, Pygmy.
                        p += CLEARING_FAMILY[(self.id as usize).saturating_sub(76).min(2)];
                        f = CLEARING_FILES[3 * r + c];
                        c += 1;
                    }
                    if p != 0 {
                        self.stamp(4 * j, 4 * i, p, f, false)?;
                    }
                }
            }
        }
        Ok(())
    }
}

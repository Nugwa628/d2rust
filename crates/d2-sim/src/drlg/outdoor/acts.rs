//! Acts II and IV outdoor level builds: the desert levels and tomb row of Act II, and the
//! mesas and Chaos Sanctum of Act IV. Act III is `act3.rs`, Act V `act5.rs`.

use super::grid::Gen;
use super::tilesub::BorderCtx;
use super::OutdoorError;

/// One of the eight desert cliff rows, selected by `r` (0..=7): five stamps of
/// (preset, file, x, y) each.
pub fn desert_cliff_row(r: u32) -> [(u32, i32, i32, i32); 5] {
    match r {
        0..=2 => {
            // The three wall/path entries are listed in ascending x. The order matters:
            // it decides which first stamp of 377/378 draws its build-list roll first.
            let path_x = 2 + 2 * r as i32;
            let piece = |x: i32| if x == path_x { 378 } else { 377 };
            [
                (376, 1, 0, 4),
                (piece(2), -1, 2, 4),
                (piece(4), -1, 4, 4),
                (piece(6), -1, 6, 4),
                (376, 2, 8, 4),
            ]
        }
        3 | 4 => {
            let (a, b) = if r == 3 { (377, 381) } else { (378, 380) };
            [
                (376, 2, 8, 4),
                (a, -1, 6, 4),
                (382, -1, 4, 4),
                (b, -1, 4, 6),
                (379, 2, 4, 8),
            ]
        }
        _ => {
            // As rows 0..=2, listed in ascending y.
            let path_y = 2 + 2 * (r as i32 - 5);
            let piece = |y: i32| if y == path_y { 381 } else { 380 };
            [
                (379, 1, 4, 0),
                (piece(2), -1, 4, 2),
                (piece(4), -1, 4, 4),
                (piece(6), -1, 4, 6),
                (379, 2, 4, 8),
            ]
        }
    }
}

/// Stamps of the Act II tomb row, as (preset, file, x, y).
pub const TOMB_ROW: [(u32, i32, i32, i32); 9] = [
    (384, 0, 8, 0),
    (383, 2, 6, 0),
    (383, 1, 4, 0),
    (383, 0, 2, 0),
    (387, 0, 0, 0),
    (385, 0, 0, 2),
    (385, 1, 0, 4),
    (385, 2, 0, 6),
    (386, 0, 0, 8),
];

/// Preset for cell `i` of the 5×5 Chaos Sanctum layout; cells without a special piece
/// use the filler preset 836.
pub fn sanctum_piece(i: i32) -> u32 {
    match i {
        22 => 857,
        11 => 858,
        13 => 859,
        17 => 860,
        7 => 861,
        12 => 862,
        _ => 836,
    }
}

impl Gen<'_> {
    /// Builds an Act II outdoor level: link flags, borders, then the per-level stamps,
    /// exits, waypoint, shrines and preset variants.
    pub fn act2(&mut self) -> Result<(), OutdoorError> {
        self.link_flags()?;
        self.borders()?;
        match self.id {
            41 => {
                self.town_transition()?;
                self.pb()?;
                self.exit(388)?;
                self.shrines(5);
                self.variants(&[395, 411, 401, 402, 399, 398, 403], false)?;
            }
            42 => {
                self.desert_cliffs()?;
                self.pb()?;
                self.exit(388)?;
                self.waypoint()?;
                self.shrines(5);
                self.variants(&[395, 411, 400, 398], false)?;
                self.variants(&[404, 405, 406, 407], true)?;
            }
            43 => {
                self.desert_cliffs()?;
                self.pb()?;
                self.exit(390)?;
                self.variants(&[396, 397], false)?;
                self.waypoint()?;
                self.shrines(5);
                self.variants(&[411, 399, 398, 403], false)?;
                self.variants(&[395], true)?;
                self.variants(&[395], true)?;
            }
            44 => {
                self.desert_cliffs()?;
                self.pb()?;
                self.exit(412)?;
                self.variants(&[413, 408, 409, 410], false)?;
                self.waypoint()?;
                self.shrines(5);
                self.variants(&[395, 400, 398, 404, 405], false)?;
                self.variants(&[411], true)?;
                self.variants(&[411], true)?;
            }
            45 => self.exit(389)?,
            46 => {
                self.tomb_row()?;
                self.pb()?;
                self.shrines(5);
                self.variants(&[401, 402, 406, 407, 403], false)?;
                self.variants(&[392, 393], true)?;
            }
            134 => {
                self.stamp(4, 4, 394, -1, false)?;
                self.pb()?;
                self.variants(&[401, 402, 406, 407, 403], false)?;
                self.variants(&[392, 393], true)?;
                self.shrines(5);
            }
            _ => {}
        }
        Ok(())
    }

    /// "PB": border substitution of types 2, 1 and 3 with base preset 364.
    fn pb(&mut self) -> Result<(), OutdoorError> {
        for t in [2, 1, 3] {
            self.border_sub(BorderCtx::wild(t, 364))?;
        }
        Ok(())
    }

    /// "Exit": S(E), fatal if not placed.
    fn exit(&mut self, e: u32) -> Result<(), OutdoorError> {
        if self.s(e)? {
            Ok(())
        } else {
            Err(OutdoorError::ExitNotPlaced(e))
        }
    }

    /// Spawns every preset in `list` once, starting at a random entry and wrapping round.
    /// With `iter` set, each preset is spawned once per file instead of once.
    pub fn variants(&mut self, list: &[u32], iter: bool) -> Result<(), OutdoorError> {
        let n = list.len() as i32;
        let mut r = self.seed().roll(n) as usize;
        for _ in 0..n {
            let p = list[r];
            if iter {
                for f in 0..self.od.preset(p)?.files {
                    self.spawn_preset(p, f, 0, 15)?;
                }
            } else {
                self.spawn_preset(p, -1, 0, 15)?;
            }
            r = (r + 1) % list.len();
        }
        Ok(())
    }

    /// Stamps the town transition preset (363 or 362, by exit direction) at the edge that
    /// links to level 40. Does nothing when there is no such link.
    fn town_transition(&mut self) -> Result<(), OutdoorError> {
        let Some(e) = self.info.orth.iter().find(|e| e.level_id == 40).copied() else {
            return Ok(());
        };
        if e.direction == 3 {
            let y = self.gh() - 1;
            self.stamp(0, y, 363, -1, false)
        } else {
            let x = self.gw() - 1;
            self.stamp(x, 0, 362, -1, false)
        }
    }

    /// Stamps one randomly chosen desert cliff row.
    fn desert_cliffs(&mut self) -> Result<(), OutdoorError> {
        let r = self.seed().mask(8);
        for (p, f, x, y) in desert_cliff_row(r) {
            self.stamp(x, y, p, f, false)?;
        }
        Ok(())
    }

    /// Stamps the tomb row and the centre preset 394.
    fn tomb_row(&mut self) -> Result<(), OutdoorError> {
        for (p, f, x, y) in TOMB_ROW {
            self.stamp(x, y, p, f, false)?;
        }
        self.stamp(4, 4, 394, -1, false)
    }

    /// Builds an Act IV outdoor level: the Chaos Sanctum (108) or a mesa level (104..=106).
    pub fn act4(&mut self) -> Result<(), OutdoorError> {
        if self.id == 108 {
            self.link_flags()?;
            for i in 0..25 {
                self.stamp(3 * (i % 5), 3 * (i / 5), sanctum_piece(i), -1, false)?;
            }
            return Ok(());
        }
        if !(104..=106).contains(&self.id) {
            return Ok(());
        }
        self.link_flags()?;
        self.borders()?;
        // Edge stamps for the two link flags, then border substitution.
        if self.info.flags & 0x40_0000 != 0 {
            self.stamp(0, 1, 798, -1, false)?;
        }
        if self.info.flags & 0x80_0000 != 0 {
            self.stamp(0, 4, 798, -1, false)?;
        }
        for t in [1, 2, 3] {
            self.border_sub(BorderCtx::wild(t, 799))?;
        }
        // Fixed stamp sequence: a middle series from `m` and a tail series from `t`.
        if self.id == 106 {
            self.s(811)?;
        }
        let (m, t) = match self.id {
            104 => (812, 828),
            105 => (817, 832),
            _ => (823, 832),
        };
        let times = |p: u32, n: u32| (0..n).map(move |_| p);
        let mut seq: Vec<u32> = Vec::new();
        seq.push(m);
        seq.extend(times(m + 1, 2));
        seq.extend(times(m + 2, 2));
        seq.extend(times(m + 3, 2));
        if self.id == 105 {
            seq.push(822);
        }
        seq.extend(times(m + 4, 4));
        seq.push(t);
        seq.extend(times(t + 1, 2));
        seq.extend(times(t + 2, 2));
        seq.extend(times(t + 3, 4));
        for p in seq {
            self.s(p)?;
        }
        Ok(())
    }
}

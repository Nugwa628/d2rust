//! Tile substitution driven by the `lvlsub` table: pattern files that replace or decorate
//! parts of outdoor levels.
//!
//! Covers the table rows and their pattern files, the border substitution on the level
//! grids (run from the level seed), the sub-theme pick made when an outdoor room is
//! created, and the room substitutions applied at tile build time (run from the room
//! seed). `rooms.rs` and the act generators call into this file.

use crate::rng::Seed;

use super::super::tiles::{CellGrid, TileRef};
use super::super::DrlgError;
use super::grid::{cell, shuffle_cells, spawn_valid, Gen, Op};
use super::{OutdoorData, OutdoorError, OutdoorRoom, SubFile, SubFiles, SubGroup};

/// Skip style a border substitution uses when its context sets none.
pub const SKIP_STYLE: i32 = 62;

/// Style-map result meaning "any" / "no stamp".
pub const STYLE_ANY: i32 = -5;

/// File index of a replacement's stamp. It is 0, not −1, so the stamp makes no
/// build-list roll and the build list is left untouched.
pub const SUB_STAMP_FILE: i32 = 0;

/// One `lvlsub` row, in file order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubRow {
    /// The row's `Type`; rows of one type are contiguous.
    pub type_: i32,
    /// Name of the row's pattern file.
    pub file: Vec<u8>,
    /// Non-zero: apply the pattern at every position that passes the test, instead of
    /// scattering a few copies.
    pub check_all: i32,
    /// Border substitution mode: 0 stops the row after the first replacement, 1 stops
    /// the group, anything else keeps going.
    pub bord_type: i32,
    /// DT1 mask bits a room gains when this row is picked.
    pub dt1_mask: u32,
    /// Cell size of one pattern cell on the level grid.
    pub grid_size: i32,
    /// `Prob0..4`, `Trials0..4`, `Max0..4` by sub theme.
    pub prob: [i32; 5],
    /// Placement attempts per copy, by sub theme; −1 tries every position.
    pub trials: [i32; 5],
    /// Copies to scatter, by sub theme.
    pub max: [i32; 5],
}

impl OutdoorData {
    /// Returns the rows of type `t`: from the first row of the type while `Type` is `t`.
    pub fn sub_rows(&self, t: i32) -> Result<&[SubRow], OutdoorError> {
        let first = self
            .subs
            .iter()
            .position(|r| r.type_ == t)
            .ok_or(OutdoorError::NoSubRows(t))?;
        // The run of rows ends at the table end if no other type follows.
        let n = self.subs[first..]
            .iter()
            .take_while(|r| r.type_ == t)
            .count();
        Ok(&self.subs[first..first + n])
    }
}

/// Loads a row's pattern file; a missing file or a file with no groups is an error.
pub fn load<'s>(subs: &'s dyn SubFiles, row: &SubRow) -> Result<&'s SubFile, OutdoorError> {
    let f = subs
        .sub_file(&row.file)
        .ok_or_else(|| OutdoorError::MissingSubFile(row.file.clone()))?;
    if f.groups.is_empty() {
        return Err(OutdoorError::SubFileNoGroups(row.file.clone()));
    }
    Ok(f)
}

/// The test and style callbacks a border substitution uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Callbacks {
    /// Acts I, II, IV: link test, 1×1 fit test.
    Wild,
    /// Act V: custom test through the Act V style map.
    Barricade,
}

/// Settings of one border substitution run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BorderCtx {
    /// `lvlsub` type whose rows are applied.
    pub t: i32,
    /// Preset id that style 1 of a pattern maps to.
    pub base: u32,
    /// Style that is never stamped; −1 until the run sets `SKIP_STYLE`.
    pub skip: i32,
    /// Which callback set decides matches and preset ids.
    pub callbacks: Callbacks,
}

impl BorderCtx {
    /// Context for Acts I, II and IV, with `Wild` callbacks.
    pub fn wild(t: i32, base: u32) -> Self {
        Self {
            t,
            base,
            skip: -1,
            callbacks: Callbacks::Wild,
        }
    }

    /// Context for Act V: type 12, base 0, `Barricade` callbacks.
    pub fn barricade() -> Self {
        Self {
            t: 12,
            base: 0,
            skip: -1,
            callbacks: Callbacks::Barricade,
        }
    }
}

/// Act V style map rows: (style, lo, hi, preset id, preset id in snow).
pub const STYLE_MAP: [(u32, u32, u32, i32, i32); 10] = [
    (49, 1, 16, 915, 987),
    (49, 31, 46, 915, 987),
    (48, 1, 1, 883, 959),
    (48, 2, 3, 881, 957),
    (48, 4, 4, 884, 960),
    (48, 5, 5, 895, 971),
    (48, 6, 7, 893, 969),
    (48, 8, 8, 896, 972),
    (48, 30, 30, 0, 0),
    (48, 31, 31, -5, -5),
];

/// Maps an Act V pattern style and value to a preset id, for snow or not.
///
/// The style must be 48 or 49 and a row must match, otherwise it is an error.
pub fn style_map(style: u32, v: u32, snow: bool) -> Result<i32, OutdoorError> {
    if style != 48 && style != 49 {
        return Err(OutdoorError::StyleMap(style, v));
    }
    STYLE_MAP
        .iter()
        .find(|r| r.0 == style && (r.1..=r.2).contains(&v))
        .map(|r| {
            let p = if snow { r.4 } else { r.3 };
            if p > 0 {
                p + (v - r.1) as i32
            } else {
                p
            }
        })
        .ok_or(OutdoorError::StyleMap(style, v))
}

/// One replacement made by a border substitution.
///
/// It records the `lvlsub` type, the row index within the type's rows, the group index,
/// the snapped cell, the variant roll and the level seed's low word after it. It is a
/// log of what the generator did and does not feed back into generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubHit {
    /// `lvlsub` type.
    pub t: i32,
    /// Row index within the type's rows.
    pub row: usize,
    /// Group index within the row's file.
    pub group: usize,
    /// Snapped cell x.
    pub x: i32,
    /// Snapped cell y.
    pub y: i32,
    /// The variant rolled.
    pub variant: i32,
    /// Low word of the level seed after the roll.
    pub lo: u32,
}

/// Outcome of trying one group of a border substitution.
enum GroupEnd {
    NextGroup,
    StopRow,
}

impl Gen<'_> {
    /// Runs the border substitution for every row of the context's type, drawing from
    /// the level seed.
    pub fn border_sub(&mut self, mut ctx: BorderCtx) -> Result<(), OutdoorError> {
        let od = self.od;
        let subs = self.subs;
        for (r, row) in od.sub_rows(ctx.t)?.iter().enumerate() {
            let file = load(subs, row)?;
            if ctx.skip == -1 {
                ctx.skip = SKIP_STYLE;
            }
            let count = file.groups.len() as i32;
            // A row with `bord_type` 0 starts at a random group; others start at the first.
            let g0 = if row.bord_type == 0 {
                self.seed().roll(count) as i32
            } else {
                0
            };
            for j in 0..count {
                let gi = ((g0 + j) % count) as usize;
                if let GroupEnd::StopRow = self.border_group(&ctx, (r, row), file, gi)? {
                    break;
                }
            }
        }
        Ok(())
    }

    /// Tries one group at every cell of the shuffled candidate cells and replaces at each
    /// match, until the row's mode says to stop.
    fn border_group(
        &mut self,
        ctx: &BorderCtx,
        (r, row): (usize, &SubRow),
        file: &SubFile,
        gi: usize,
    ) -> Result<GroupEnd, OutdoorError> {
        let g = file.groups[gi];
        let off = if ctx.t == 1 && self.info.flags & 0xC != 0 {
            -1
        } else {
            1
        };
        let w = self.gw() - row.grid_size * g.w + off;
        let h = self.gh() - row.grid_size * g.h + 1;
        if w * h <= 0 {
            return Ok(GroupEnd::NextGroup);
        }
        let small = ctx.t == 1 && (2..=7).contains(&self.id) && w < 6 && h < 6;
        // Candidate cells in a shuffled order drawn from the level seed.
        let entries = shuffle_cells(self.seed(), w, h);
        for (x, y) in entries {
            if small && (x, y) == (2, 2) {
                continue;
            }
            if self.sub_test(ctx, row, file, g, x, y)? {
                // A match rolls the variant, then logs and applies the replacement.
                let v = self.seed().roll(g.variants) as i32;
                let (sx, sy) = Self::snap(row, x, y);
                let lo = self.seed().lo;
                self.info.sub_hits.push(SubHit {
                    t: ctx.t,
                    row: r,
                    group: gi,
                    x: sx,
                    y: sy,
                    variant: v,
                    lo,
                });
                self.sub_replace(ctx, row, file, g, x, y, (v + 1) * (g.w + 1))?;
                match row.bord_type {
                    0 => return Ok(GroupEnd::StopRow),
                    1 => return Ok(GroupEnd::NextGroup),
                    _ => {}
                }
            }
        }
        Ok(GroupEnd::NextGroup)
    }

    fn snap(row: &SubRow, x: i32, y: i32) -> (i32, i32) {
        let gs = row.grid_size;
        if gs == 0 {
            // A grid size of 0 cannot snap; the cell is used as is. No table row has it.
            return (x, y);
        }
        (x - x % gs, y - y % gs)
    }

    /// Tests whether a group fits at cell `(x, y)` (snapped to the row's grid).
    fn sub_test(
        &self,
        ctx: &BorderCtx,
        row: &SubRow,
        file: &SubFile,
        g: SubGroup,
        x: i32,
        y: i32,
    ) -> Result<bool, OutdoorError> {
        let (x, y) = Self::snap(row, x, y);
        let snow = self.id == 117;
        for j in 0..g.h {
            for i in 0..g.w {
                let (cx, cy) = (x + i * row.grid_size, y + j * row.grid_size);
                let f = file.floor_at(g.x + i, g.y + j);
                let w = file.wall_at(0, g.x + i, g.y + j);
                let c = self.g(0, cx, cy) as i32;
                let not_link = self.g(2, cx, cy) & cell::LINK == 0;
                let pass = match ctx.callbacks {
                    Callbacks::Barricade => {
                        let m = style_map(w >> 20 & 0x3F, w >> 8 & 0xFF, snow)?;
                        m == STYLE_ANY || (m == c && not_link)
                    }
                    Callbacks::Wild if w & 1 != 0 => {
                        let s = (w >> 8 & 0xFF) as i32 - 1;
                        (s == ctx.skip || c == ctx.base as i32 + s) && not_link
                    }
                    Callbacks::Wild if f & 2 != 0 => {
                        self.in_grid(cx, cy) && spawn_valid(self.g(2, cx, cy))
                    }
                    Callbacks::Wild => true,
                };
                if !pass {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// Replaces the cells of a group at `(x, y)` with the pattern at x offset `xoff`.
    #[allow(clippy::too_many_arguments)]
    fn sub_replace(
        &mut self,
        ctx: &BorderCtx,
        row: &SubRow,
        file: &SubFile,
        g: SubGroup,
        x: i32,
        y: i32,
        xoff: i32,
    ) -> Result<(), OutdoorError> {
        let (x, y) = Self::snap(row, x, y);
        let snow = self.id == 117;
        for j in 0..g.h {
            for i in 0..g.w {
                let (cx, cy) = (x + i * row.grid_size, y + j * row.grid_size);
                let f = file.floor_at(g.x + i + xoff, g.y + j);
                let w = file.wall_at(0, g.x + i + xoff, g.y + j);
                if w & 1 != 0 {
                    let s = (w >> 8 & 0xFF) as i32 - 1;
                    let p = match ctx.callbacks {
                        Callbacks::Barricade => style_map(w >> 20 & 0x3F, w >> 8 & 0xFF, snow)?,
                        Callbacks::Wild => ctx.base as i32 + s,
                    };
                    if p != STYLE_ANY && s != ctx.skip {
                        // A preset id of 0 or less (style map row (48, 30, 30) gives
                        // 0) is skipped; there is nothing to stamp.
                        if p > 0 {
                            // File 0: no build-list roll.
                            self.stamp(cx, cy, p as u32, SUB_STAMP_FILE, true)?;
                        }
                    }
                } else if f & 2 != 0 {
                    // Keep cell: clear the preset and flags, so the cell is empty but kept.
                    self.op(0, cx, cy, Op::Set, 0);
                    self.op(2, cx, cy, Op::Set, 0);
                } else {
                    // Blank cell: clear the preset and mark the cell blank.
                    self.op(0, cx, cy, Op::Set, 0);
                    self.op(2, cx, cy, Op::Set, cell::BLANK);
                }
            }
        }
        Ok(())
    }
}

/// Picks the sub-theme rows of type `t` for a new outdoor room: one room-seed step per
/// row.
///
/// Returns the picked mask (bit k = row k) and the DT1 mask bits to OR into the room.
pub fn pick_sub_themes(
    od: &OutdoorData,
    seed: &mut Seed,
    t: i32,
    theme: i32,
) -> Result<(u32, u32), OutdoorError> {
    if t == -1 || theme == -1 {
        return Ok((0, 0));
    }
    let h = theme_index(theme)?;
    let (mut mask, mut dt1) = (0, 0);
    for (k, row) in od.sub_rows(t)?.iter().enumerate() {
        // The row is picked when the roll (0..99) is below its probability.
        let r = (seed.step() % 100) as i32;
        if r < row.prob[h] {
            mask |= 1u32 << k;
            dt1 |= row.dt1_mask;
        }
    }
    Ok((mask, dt1))
}

fn theme_index(theme: i32) -> Result<usize, OutdoorError> {
    if (0..5).contains(&theme) {
        Ok(theme as usize)
    } else {
        Err(OutdoorError::SubTheme(theme))
    }
}

/// The room side of a room substitution: size (8 × 8 for outdoor rooms),
/// tile origin, and its grids (one wall layer).
pub struct RoomSub<'a> {
    /// Room width in cells.
    pub w: i32,
    /// Room height in cells.
    pub h: i32,
    /// Tile x of the room's origin.
    pub tile_x: i32,
    /// Tile y of the room's origin.
    pub tile_y: i32,
    /// The room whose grids are changed.
    pub room: &'a mut OutdoorRoom,
    /// Chooses the tile for a shadow cell (type 13, the cell's main and sub index),
    /// drawing from the seed passed in; `None` records the cells without a tile.
    pub pick: Option<&'a ShadowPick<'a>>,
}

/// A shadow tile picker: takes the seed and a packed cell, returns the tile.
pub type ShadowPick<'a> = dyn Fn(&mut Seed, u32) -> Result<TileRef, DrlgError> + 'a;

/// Applies the room substitution for the rows of type `t` selected by `mask` (bit k =
/// row k), for sub theme `theme`, drawing from the room seed `seed`.
pub fn room_substitution(
    od: &OutdoorData,
    subs: &dyn SubFiles,
    seed: &mut Seed,
    rs: &mut RoomSub<'_>,
    t: i32,
    theme: i32,
    mask: u32,
) -> Result<(), OutdoorError> {
    if t == -1 {
        return Ok(());
    }
    let mut mask = mask;
    for row in od.sub_rows(t)? {
        if mask == 0 {
            break;
        }
        if mask & 1 != 0 {
            let h = theme_index(theme)?;
            let file = load(subs, row)?;
            if row.check_all != 0 {
                check_all(seed, rs, row, file, h)?;
            } else {
                scattered(seed, rs, row, file, h)?;
            }
        }
        mask >>= 1;
    }
    Ok(())
}

/// Applies a `check_all` row: tries every position of each group, by the file's method.
fn check_all(
    seed: &mut Seed,
    rs: &mut RoomSub<'_>,
    row: &SubRow,
    file: &SubFile,
    h: usize,
) -> Result<(), OutdoorError> {
    for &g in &file.groups {
        let (gw, gh) = (rs.w - g.w + 1, rs.h - g.h + 1);
        if gw <= 0 || gh <= 0 {
            continue;
        }
        match file.method {
            1 => {
                for y in 1..gh {
                    for x in 1..gw {
                        if fixed_test(rs, file, g, x, y) {
                            apply(seed, rs, file, g, x, y, 0)?;
                        }
                    }
                }
            }
            2 => {
                for y in 0..gh {
                    for x in 0..gw {
                        if random_test(rs, file, g, x, y) {
                            let r = (seed.step() % 100) as i32;
                            // The comparison is the opposite of `pick_sub_themes`: the
                            // group applies when the roll is above the probability.
                            if row.prob[h] < r {
                                let v = seed.roll(g.variants) as i32;
                                apply(seed, rs, file, g, x, y, (v + 1) * (g.w + 1))?;
                            }
                        }
                    }
                }
            }
            // Other methods do nothing.
            _ => {}
        }
    }
    Ok(())
}

/// Applies a scattered row: up to `Max` copies of random groups at random positions.
fn scattered(
    seed: &mut Seed,
    rs: &mut RoomSub<'_>,
    row: &SubRow,
    file: &SubFile,
    h: usize,
) -> Result<(), OutdoorError> {
    let count = file.groups.len() as i32;
    if count == 0 {
        return Ok(());
    }
    for _ in 0..row.max[h].max(0) {
        // The group is drawn even if it does not fit the room.
        let g = file.groups[seed.roll(count) as usize];
        let (aw, ah) = (rs.w - g.w, rs.h - g.h);
        if aw <= 0 || ah <= 0 {
            continue;
        }
        let trials = row.trials[h];
        if trials == -1 {
            // Trials of -1: try every position in a shuffled order until one fits.
            for (x, y) in shuffle_cells(seed, aw, ah) {
                if fixed_test(rs, file, g, x + 1, y + 1) {
                    apply(seed, rs, file, g, x + 1, y + 1, 0)?;
                    break;
                }
            }
        } else if trials > 0 {
            for _ in 0..trials {
                // Otherwise try random positions, up to the trial count.
                let x = seed.roll(aw) as i32 + 1;
                let y = seed.roll(ah) as i32 + 1;
                if fixed_test(rs, file, g, x, y) {
                    apply(seed, rs, file, g, x, y, 0)?;
                    break;
                }
            }
        }
    }
    Ok(())
}

/// True when the group's floor and wall cells all land on free, plain room cells at
/// `(x, y)`.
pub fn fixed_test(rs: &RoomSub<'_>, file: &SubFile, g: SubGroup, x: i32, y: i32) -> bool {
    for j in 0..g.h {
        for i in 0..g.w {
            let pf = file.floor_at(g.x + i, g.y + j);
            let pw = file.wall_at(0, g.x + i, g.y + j);
            let has_wall0 = !file.walls.is_empty();
            if pf & 2 != 0 || (has_wall0 && pw & 1 != 0) {
                let rf = rs.room.floor.get(x + i, y + j);
                let rw = rs.room.wall.get(x + i, y + j);
                if rf & 2 == 0 || rf & 0x3F0_FF00 != 0 || rw & 1 != 0 {
                    return false;
                }
            }
        }
    }
    true
}

/// True when the group matches the room at `(x, y)`: same tile types, and floor and wall
/// cells whose masked bits agree where the pattern has them.
pub fn random_test(rs: &RoomSub<'_>, file: &SubFile, g: SubGroup, x: i32, y: i32) -> bool {
    const M: u32 = 0x3F0_FF00;
    for j in 0..g.h {
        for i in 0..g.w {
            let (px, py) = (g.x + i, g.y + j);
            let (rx, ry) = (x + i, y + j);
            if file.tile_type_at(0, px, py) != rs.room.tile_type.get(rx, ry) {
                return false;
            }
            if file.floor.is_some() {
                let pf = file.floor_at(px, py);
                let rf = rs.room.floor.get(rx, ry);
                if pf & 2 != 0 && (rf & 2 == 0 || rf & M != pf & M) {
                    return false;
                }
            }
            if !file.walls.is_empty() {
                let pw = file.wall_at(0, px, py);
                let rw = rs.room.wall.get(rx, ry);
                if pw & 1 != 0 && (rw & 1 == 0 || rw & M != pw & M) {
                    return false;
                }
            }
        }
    }
    true
}

fn set(g: &mut CellGrid, x: i32, y: i32, v: u32) {
    if x >= 0 && y >= 0 && (x as usize) < g.width && (y as usize) < g.height {
        g.set(x as usize, y as usize, v);
    }
}

/// Writes a group into the room at cell `(x, y)` with pattern x offset `o`.
///
/// Copies floor, wall and tile-type cells, records shadows and moves preset units.
#[allow(clippy::too_many_arguments)]
pub fn apply(
    seed: &mut Seed,
    rs: &mut RoomSub<'_>,
    file: &SubFile,
    g: SubGroup,
    x: i32,
    y: i32,
    o: i32,
) -> Result<(), OutdoorError> {
    // Count the pattern's roof cells (shadow bit 0x800_0000) into the room's roof count.
    let mut roofs = 0;
    for j in 0..g.h {
        for i in 0..g.w {
            if file.shadow_at(g.x + i + o, g.y + j) & 0x800_0000 != 0 {
                roofs += 1;
            }
        }
    }
    rs.room.roof_count += roofs;
    // Copy the pattern's cells into the room grids.
    for j in 0..g.h {
        for i in 0..g.w {
            let (px, py) = (g.x + i + o, g.y + j);
            let (rx, ry) = (x + i, y + j);
            if file.floor.is_some() {
                let f = file.floor_at(px, py);
                if f & 2 != 0 {
                    set(&mut rs.room.floor, rx, ry, f | 0x80);
                }
            }
            // The room has one wall layer and one tile-type grid (k = 0).
            if !file.walls.is_empty() {
                let w = file.wall_at(0, px, py);
                if w & 1 != 0 {
                    set(&mut rs.room.wall, rx, ry, w);
                }
                let t = file.tile_type_at(0, px, py);
                if t != 0 {
                    set(&mut rs.room.tile_type, rx, ry, t);
                }
            }
            let s = file.shadow_at(px, py);
            if s & 0x800_0000 != 0 {
                // The shadow tile is chosen here, from the room seed, between this
                // apply's other draws.
                if let Some(pick) = rs.pick {
                    let tile = pick(seed, s)?;
                    rs.room.shadow_tiles.push(tile);
                }
                rs.room
                    .shadows
                    .push((rs.tile_x + x + i, rs.tile_y + y + j, s));
            }
        }
    }
    // Copy the file's preset units that lie strictly inside the pattern box.
    let (bx0, by0) = (g.x * 5, g.y * 5);
    let (bx1, by1) = ((g.x + g.w) * 5, (g.y + g.h) * 5);
    for u in &file.units {
        if bx0 < u.x && u.x < bx1 && by0 < u.y && u.y < by1 {
            let mut nu = *u;
            nu.x = 5 * x + u.x - bx0;
            nu.y = 5 * y + u.y - by0;
            // Units are prepended, so the room's list ends up in the reverse of the
            // file's order.
            rs.room.units.insert(0, nu);
        }
    }
    Ok(())
}

//! The light values handed to the draws.
//!
//! - A unit is lit by the cell under its feet (one light word).
//! - A wall reads six light points along its face, and each 32-pixel block
//!   column of the wall is shaded between two neighbouring points.
//! - A floor tile reads an 8 × 8 grid of cells around it, so its blocks
//!   can be shaded smoothly.
//! - A roof reads the floor grid under it, or the act environment's light
//!   when it sits high above the ground.
//!
//! [`crate::rules::shading`] turns these values into the light maps the
//! draws use. Every read goes through [`LightMap::read`] (eighths of a
//! sub-tile, clamped to the map).

use std::sync::OnceLock;

use super::map::{LightCell, LightMap};
use crate::rules::blend::wall_block_ops;
use crate::rules::shading::ShadeTables;
use crate::rules::view::{BlockRect, BlockShade};

/// The wall light points, `tables/render/wall-light-points.tsv`: for the
/// normal and the faded table, wall directions 1–9, six points each.
pub const WALL_LIGHT_POINTS_TSV: &str =
    include_str!("../../../../../tables/render/wall-light-points.tsv");
/// Light points of a wall with a non-zero direction.
pub const WALL_POINTS: usize = 6;
/// Directions the point tables hold (1…9).
pub const WALL_DIRECTIONS: u32 = 9;
/// Cells per axis of the floor / roof light grid.
pub const GRID: usize = 8;
/// DT1 material flag (tile header `+0x06`) of a floor that is never lit:
/// every byte of its grid is 0xFF.
pub const MATERIAL_UNLIT: u32 = 0x100;

/// Errors of the draw light values. A case D2RUST does not define is an
/// error naming it, never a made-up value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DrawLightError {
    /// Wall direction 0 (never in the game's tiles) as the first wall of its
    /// pass: no earlier record left light words in the pass's array, so
    /// there are no values to take (fatal).
    #[error("wall direction 0 with no earlier wall in the pass: light words are undefined (render/lighting.md Edge case 7, OQ8)")]
    WallDirection0,
    /// A direction outside the 1…9 the point tables hold.
    #[error(
        "wall direction {0} is outside the point tables (1…9) of render/wall-light-points.tsv"
    )]
    WallDirection(u32),
    /// A block column whose `c + 1` is not one of the six points: a wall
    /// reads points `c` and `c + 1` of the 0…5 a wall has (a 160-pixel
    /// wall has columns 0…4); a block past them is malformed input.
    #[error("wall block x {0}: column c = x >> 5 needs points c and c + 1 of 0…5 (render/lighting.md §11 r2)")]
    BlockColumn(i32),
    /// A malformed row of `wall-light-points.tsv`.
    #[error("wall-light-points.tsv line {line}: {msg}")]
    Tsv { line: usize, msg: String },
}

/// The light word of a unit draw: the cell of the unit's sub-tile as
/// `B << 24 | G << 16 | R << 8 | I`.
pub fn unit_light(map: &LightMap, sub_tile: (i32, i32)) -> u32 {
    map.read(sub_tile.0 * 8, sub_tile.1 * 8).word()
}

/// The palette light byte `v` of a light word: its low byte, the
/// intensity.
pub fn light_byte(word: u32) -> u8 {
    (word & 0xFF) as u8
}

/// Which point table a wall reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PointTable {
    /// A wall drawn solid.
    Normal,
    /// A wall faded out to let the player be seen (fade state bit 0): its
    /// points sit on the other side of the wall.
    Faded,
}

impl PointTable {
    /// The table of a wall record with fade state `fade` (see
    /// [`crate::rules::draw_order`]): bit 0 selects the faded table.
    pub fn of_fade(fade: u32) -> PointTable {
        if fade & 1 != 0 {
            PointTable::Faded
        } else {
            PointTable::Normal
        }
    }

    fn index(self) -> usize {
        match self {
            PointTable::Normal => 0,
            PointTable::Faded => 1,
        }
    }
}

/// One row of `wall-light-points.tsv`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WallPointRow {
    pub direction: u32,
    pub table: PointTable,
    pub point: usize,
    pub dx: i32,
    pub dy: i32,
}

/// The two point tables, directions 1…9, six `(dx, dy)` sub-tile offsets
/// each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WallPoints {
    /// `[table][direction − 1][point]`.
    points: [[[(i32, i32); WALL_POINTS]; WALL_DIRECTIONS as usize]; 2],
    rows: Vec<WallPointRow>,
}

fn tsv_err(line: usize, msg: impl Into<String>) -> DrawLightError {
    DrawLightError::Tsv {
        line,
        msg: msg.into(),
    }
}

impl WallPoints {
    /// Parses the TSV strictly: the header `direction table point dx dy`,
    /// five tab-separated fields per row, direction 1…9, table `normal` or
    /// `faded`, point 0…5, `dx` / `dy` integers, and every
    /// (direction, table, point) exactly once.
    pub fn parse(text: &str) -> Result<WallPoints, DrawLightError> {
        let mut lines = text.lines().enumerate();
        match lines.next() {
            Some((_, "direction\ttable\tpoint\tdx\tdy")) => {}
            _ => return Err(tsv_err(1, "header is not `direction table point dx dy`")),
        }
        let mut seen = [[[false; WALL_POINTS]; WALL_DIRECTIONS as usize]; 2];
        let mut points = [[[(0, 0); WALL_POINTS]; WALL_DIRECTIONS as usize]; 2];
        let mut rows = Vec::new();
        for (n, line) in lines {
            let line_no = n + 1;
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() != 5 {
                return Err(tsv_err(line_no, format!("{} fields, expected 5", f.len())));
            }
            let int = |s: &str, what: &str| {
                s.parse::<i32>()
                    .map_err(|_| tsv_err(line_no, format!("{what} `{s}` is not an integer")))
            };
            let direction = int(f[0], "direction")?;
            if !(1..=WALL_DIRECTIONS as i32).contains(&direction) {
                return Err(tsv_err(
                    line_no,
                    format!("direction {direction} not in 1…9"),
                ));
            }
            let table = match f[1] {
                "normal" => PointTable::Normal,
                "faded" => PointTable::Faded,
                t => return Err(tsv_err(line_no, format!("table `{t}`"))),
            };
            let point = int(f[2], "point")?;
            if !(0..WALL_POINTS as i32).contains(&point) {
                return Err(tsv_err(line_no, format!("point {point} not in 0…5")));
            }
            let (dx, dy) = (int(f[3], "dx")?, int(f[4], "dy")?);
            let (d, p) = (direction as usize - 1, point as usize);
            let slot = &mut seen[table.index()][d][p];
            if *slot {
                return Err(tsv_err(line_no, "duplicate (direction, table, point)"));
            }
            *slot = true;
            points[table.index()][d][p] = (dx, dy);
            rows.push(WallPointRow {
                direction: direction as u32,
                table,
                point: p,
                dx,
                dy,
            });
        }
        if seen.iter().flatten().flatten().any(|s| !s) {
            return Err(tsv_err(
                text.lines().count(),
                "missing rows: need 9 directions × 2 tables × 6 points",
            ));
        }
        Ok(WallPoints { points, rows })
    }

    /// The rows in file order.
    pub fn rows(&self) -> &[WallPointRow] {
        &self.rows
    }

    /// The six `(dx, dy)` of a wall with DT1 `direction` in `table`.
    /// Direction 0 has no points: [`DrawLightError::WallDirection0`]
    /// here; [`WallPass`] gives it the previous wall's words.
    pub fn offsets(
        &self,
        direction: u32,
        table: PointTable,
    ) -> Result<[(i32, i32); WALL_POINTS], DrawLightError> {
        match direction {
            0 => Err(DrawLightError::WallDirection0),
            d if d <= WALL_DIRECTIONS => Ok(self.points[table.index()][d as usize - 1]),
            d => Err(DrawLightError::WallDirection(d)),
        }
    }
}

/// The parsed `wall-light-points.tsv` (the table ships with the source; a
/// parse failure is a broken checkout).
pub fn wall_points() -> &'static WallPoints {
    static POINTS: OnceLock<WallPoints> = OnceLock::new();
    POINTS.get_or_init(|| {
        WallPoints::parse(WALL_LIGHT_POINTS_TSV)
            .expect("tables/render/wall-light-points.tsv parses")
    })
}

/// The origin sub-tile of a wall or floor tile: the room's sub-tile origin
/// plus 5 sub-tiles per tile of the record's tile position.
pub fn tile_origin_sub_tile(room_sub_tile: (i32, i32), tile: (i32, i32)) -> (i32, i32) {
    (room_sub_tile.0 + 5 * tile.0, room_sub_tile.1 + 5 * tile.1)
}

/// The six light words of a wall or lower wall: point `p` reads the cell
/// at the tile's origin sub-tile plus `(dx_p, dy_p)`. `fade` is the
/// record's fade state (bit 0 → the faded table).
pub fn wall_light_words(
    map: &LightMap,
    origin_sub_tile: (i32, i32),
    direction: u32,
    fade: u32,
) -> Result<[u32; WALL_POINTS], DrawLightError> {
    let offsets = wall_points().offsets(direction, PointTable::of_fade(fade))?;
    let (x, y) = (8 * origin_sub_tile.0, 8 * origin_sub_tile.1);
    Ok(offsets.map(|(dx, dy)| map.read(x + 8 * dx, y + 8 * dy).word()))
}

/// The light words of one wall pass. The pass shares one set of words, so
/// a wall with direction 0 (no light points; no shipped tile has one)
/// keeps the words the previous wall of the pass wrote. Start a new one
/// per pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WallPass {
    last: Option<[u32; WALL_POINTS]>,
}

impl WallPass {
    /// [`wall_light_words`] for the next wall of the pass; direction 0
    /// reuses the previous wall's words, and is
    /// [`DrawLightError::WallDirection0`] when it is the pass's first.
    pub fn words(
        &mut self,
        map: &LightMap,
        origin_sub_tile: (i32, i32),
        direction: u32,
        fade: u32,
    ) -> Result<[u32; WALL_POINTS], DrawLightError> {
        if direction == 0 {
            return self.last.ok_or(DrawLightError::WallDirection0);
        }
        let w = wall_light_words(map, origin_sub_tile, direction, fade)?;
        self.last = Some(w);
        Ok(w)
    }
}

/// The corners `c0…c3` of the wall block at block x `block_x`: column
/// `c = block_x >> 5`, the left corners `c0 = c3` take point `c`, the
/// right corners `c1 = c2` take point `c + 1` (light bytes of the words),
/// so light varies across the wall but not up it. A column whose `c + 1`
/// is past point 5 (block x ≥ 160) or negative is
/// [`DrawLightError::BlockColumn`].
pub fn wall_block_corners(
    words: &[u32; WALL_POINTS],
    block_x: i32,
) -> Result<[u8; 4], DrawLightError> {
    let c = block_x >> 5;
    if c < 0 || c as usize + 1 >= WALL_POINTS {
        return Err(DrawLightError::BlockColumn(block_x));
    }
    let (a, b) = (
        light_byte(words[c as usize]),
        light_byte(words[c as usize + 1]),
    );
    Ok([a, b, b, a])
}

/// The shaded blocks of a wall or roof tile: per block, its corners from
/// `words` at the block's x, then the lit (opaque) or translucent ops of
/// wall alpha `alpha` ([`wall_block_ops`]). Blocks the wall alpha hides
/// are left out. The gradient position is (0, 0); the view moves it to the
/// block.
pub fn wall_block_shades(
    tables: &ShadeTables,
    words: &[u32; WALL_POINTS],
    blocks: &[BlockRect],
    alpha: u8,
    low_quality: bool,
) -> Result<Vec<BlockShade>, DrawLightError> {
    let mut out = Vec::with_capacity(blocks.len());
    for block in blocks {
        let corners = wall_block_corners(words, block.x)?;
        if let Some((shade, blend)) = wall_block_ops(tables, alpha, corners, low_quality, 0, 0) {
            out.push(BlockShade {
                block: *block,
                shade,
                blend,
            });
        }
    }
    Ok(out)
}

/// The 8 × 8 floor / roof light grid, row-major: each cell holds I, R, G,
/// B.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LightGrid {
    /// `cells[j * 8 + i]` = `[I, R, G, B]`.
    pub cells: [[u8; 4]; GRID * GRID],
}

impl LightGrid {
    /// Every cell `[I, R, G, B]` = `value`.
    pub fn filled(value: [u8; 4]) -> LightGrid {
        LightGrid {
            cells: [value; GRID * GRID],
        }
    }

    /// Cell `(i, j)` (column `i`, row `j`, 0…7).
    pub fn cell(&self, i: usize, j: usize) -> [u8; 4] {
        self.cells[j * GRID + i]
    }

    /// The intensity of every cell in row-major order: what
    /// [`crate::rules::shading::floor_block_light`] reads.
    pub fn light_values(&self) -> [u8; GRID * GRID] {
        self.cells.map(|c| c[0])
    }

    /// Reads the grid whose cell `(i, j)` is sub-tile
    /// `(sx − 1 + i, sy − 1 + j)`: one sub-tile of margin before the tile,
    /// so blocks at the tile's edge can blend with their neighbours.
    fn read(map: &LightMap, sub_tile: (i32, i32)) -> LightGrid {
        let mut grid = LightGrid::filled([0; 4]);
        for j in 0..GRID {
            for i in 0..GRID {
                let c = map.read(
                    8 * (sub_tile.0 - 1 + i as i32),
                    8 * (sub_tile.1 - 1 + j as i32),
                );
                grid.cells[j * GRID + i] = [c.i, c.r, c.g, c.b];
            }
        }
        grid
    }
}

/// The light grid of a floor tile whose origin sub-tile is `(sx, sy)`:
/// cell `(i, j)` = I, R, G, B of sub-tile `(sx − 1 + i, sy − 1 + j)`. A
/// tile with the [`MATERIAL_UNLIT`] material flag is fully bright (every
/// byte 0xFF).
pub fn floor_light_grid(map: &LightMap, origin_sub_tile: (i32, i32), material: u32) -> LightGrid {
    if material & MATERIAL_UNLIT != 0 {
        return LightGrid::filled([0xFF; 4]);
    }
    LightGrid::read(map, origin_sub_tile)
}

/// The light grid of a roof: the floor grid at the roof's sub-tile
/// (`draw_pos` in eighths of a sub-tile). A roof with a height (tile
/// header `+0x04` ≠ 0) is above the light of the ground, so every cell is
/// the act environment's I, R, G, B instead.
pub fn roof_light_grid(
    map: &LightMap,
    draw_pos: (i32, i32),
    height: u32,
    environment: LightCell,
) -> LightGrid {
    if height != 0 {
        let e = environment;
        return LightGrid::filled([e.i, e.r, e.g, e.b]);
    }
    LightGrid::read(map, (draw_pos.0 >> 3, draw_pos.1 >> 3))
}

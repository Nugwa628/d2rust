//! How one light record adds its light to the light map.
//!
//! All three shapes share one spread: over a square window of sub-tiles
//! around the light, each cell gets `I · (r − d) / r`, the record's
//! intensity falling off linearly with the octagonal distance `d` from
//! the light, and the cell's colour is mixed towards the light's colour by
//! the share of intensity it brings.
//!
//! - **Plain** lights spread freely (at the lowest quality, monsters'
//!   lights shrink and missiles' lights go out).
//! - **Shadowed** lights first cast rays over a 64 × 64 shade grid from
//!   the light outwards: cells that block light are walls, and every cell
//!   behind them gets a shade that dims or removes its share.
//! - **Cached** lights cast the same rays once, store the shade values on
//!   the record and reuse them every frame until something invalidates
//!   them.
//!
//! Integer math only: the results are part of the frame's state key.

use super::map::LightMap;
use super::records::{unit_type, LightError, LightRecord, LightWorld};

/// Side of the shade grid.
pub const GRID: i32 = 64;
/// Centre of the shade grid: the light's own sub-tile.
pub const CENTRE: i32 = 32;
/// The shade value of a wall. A shade of [`WALL`] or more means full
/// shadow.
pub const WALL: i32 = 16;

/// Octagonal distance.
pub fn oct(a: i32, b: i32) -> i32 {
    (983 * a.max(b) + 407 * a.min(b)) >> 10
}

/// The reciprocal table `T[i] = 65536 / i`, `T[0] = 0`: the falloff
/// divisor of a light's cells.
pub fn recip(i: u8) -> i32 {
    if i == 0 {
        0
    } else {
        65536 / i32::from(i)
    }
}

/// Adds light value `v` of colour `src` to cell `(gx, gy)`. Non-positive
/// values and cells off the map are skipped.
///
/// The intensity adds up and saturates at 255. The colour becomes the
/// intensity-weighted mix of the colour already there (weighted by the old
/// intensity) and `src` (weighted by `v`). Without coloured light the cell
/// loses its colour entirely.
pub fn add(map: &mut LightMap, gx: i32, gy: i32, v: i32, src: (u8, u8, u8), colored: bool) {
    if v <= 0 {
        return;
    }
    let Some(c) = map.cell_mut(gx, gy) else {
        return;
    };
    let old = i64::from(c.i);
    let new_i = (old + i64::from(v)).min(255) as u8;
    if colored {
        let t = i64::from(recip(new_i));
        let mix = |cur: u8, s: u8| -> u8 {
            (((i64::from(cur) * old + i64::from(s) * i64::from(v)) * t) >> 16).min(255) as u8
        };
        c.r = mix(c.r, src.0);
        c.g = mix(c.g, src.1);
        c.b = mix(c.b, src.2);
    } else {
        c.r = 0;
        c.g = 0;
        c.b = 0;
    }
    c.i = new_i;
}

/// The square window a light covers: `n + 1` sub-tiles on each side,
/// starting at `(x0, y0)` (eighths of a sub-tile, on a sub-tile boundary).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub x0: i32,
    pub y0: i32,
    /// `n = 2r >> 3 = 2m`, `m` the radius in whole sub-tiles.
    pub n: i32,
}

/// The window of a light at `(x, y)` with radius `r` (all in eighths of a
/// sub-tile). `None` when the radius gives no light or the window misses
/// the map entirely.
pub fn window(map: &LightMap, x: i32, y: i32, r: i32) -> Option<Window> {
    if !(1..=255).contains(&r) {
        return None;
    }
    // Snap to the sub-tile boundary (`%` truncates towards zero; light
    // positions are non-negative in practice), then step back by the
    // radius.
    let x0 = (x - x % 8) - r;
    let y0 = (y - y % 8) - r;
    let n = (2 * r) >> 3;
    let (ox, oy) = map.origin;
    if x0 >> 3 > ox + 48 || (x0 >> 3) + n + 1 < ox || y0 >> 3 > oy + 48 || (y0 >> 3) + n + 1 < oy {
        return None;
    }
    Some(Window { x0, y0, n })
}

/// Spreads the record's light over window `w` with radius `r`: for row
/// `j`, column `i`, the base value `b = I · (r − d) / r` (`d` the
/// octagonal distance from the light to the cell's corner) goes through
/// `weight(j, i, b)`, which may dim or drop it, before it is added.
fn spread(
    map: &mut LightMap,
    rec: &LightRecord,
    r: i32,
    w: Window,
    colored: bool,
    weight: impl Fn(i32, i32, i32) -> Option<i32>,
) {
    let k = (i32::from(rec.i) << 16) / r;
    let (ox, oy) = map.origin;
    for j in 0..=w.n {
        for i in 0..=w.n {
            let cx = w.x0 + 8 * i;
            let cy = w.y0 + 8 * j;
            let d = oct((rec.x - cx).abs(), (rec.y - cy).abs());
            let b = ((r - d) * k) >> 16;
            if let Some(v) = weight(j, i, b) {
                add(
                    map,
                    (cx >> 3) - ox,
                    (cy >> 3) - oy,
                    v,
                    (rec.r, rec.g, rec.b),
                    colored,
                );
            }
        }
    }
}

/// The radius a plain light spreads with. At the lowest quality (`q` = 0)
/// lights are cut back to save work: a remote player's light of two
/// sub-tiles or more shrinks to two, every monster's light becomes one
/// sub-tile, and missiles give no light at all (`None`). The local
/// player's light is never cut.
pub fn plain_radius(rec: &LightRecord, q: u8, owner_is_local_player: bool) -> Option<i32> {
    let r = rec.radius;
    if q != 0 {
        return Some(r);
    }
    match rec.owner_type {
        unit_type::PLAYER if !owner_is_local_player && r >= 16 => Some(16),
        unit_type::MONSTER => Some(8),
        unit_type::MISSILE => None,
        _ => Some(r),
    }
}

/// The plain contribution: the light spreads with no shadows.
pub fn plain(
    map: &mut LightMap,
    rec: &LightRecord,
    q: u8,
    owner_is_local_player: bool,
    colored: bool,
) {
    let Some(r) = plain_radius(rec, q, owner_is_local_player) else {
        return;
    };
    let Some(w) = window(map, rec.x, rec.y, r) else {
        return;
    };
    spread(map, rec, r, w, colored, |_, _, b| Some(b));
}

/// The 64 × 64 planes of a shadow cast, row-major: the blockers (walls,
/// [`WALL`] or 0) and the shade cast over them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadeGrid {
    pub b: Vec<i32>,
    pub s: Vec<i32>,
}

impl Default for ShadeGrid {
    fn default() -> Self {
        ShadeGrid {
            b: vec![0; (GRID * GRID) as usize],
            s: vec![0; (GRID * GRID) as usize],
        }
    }
}

impl ShadeGrid {
    fn at(row: i32, col: i32) -> usize {
        (row * GRID + col) as usize
    }

    /// `v(c)` = `B[c]` when non-zero, else `S[c]`.
    pub fn v(&self, row: i32, col: i32) -> i32 {
        let c = Self::at(row, col);
        if self.b[c] != 0 {
            self.b[c]
        } else {
            self.s[c]
        }
    }

    /// The shade cast on cell `(row, col)`.
    pub fn shade(&self, row: i32, col: i32) -> i32 {
        self.s[Self::at(row, col)]
    }

    pub fn set_blocker(&mut self, row: i32, col: i32, v: i32) {
        self.b[Self::at(row, col)] = v;
    }

    /// Casts the shade of one cell: grid `(row, col)` is sub-tile
    /// `(sx, sy)`, the light is at `(x, y)` (eighths). The shade is taken
    /// from the one or two neighbours one step closer to the light, mixed
    /// by how steep the ray from the light to the cell's centre is, so a
    /// wall's shadow widens smoothly behind it.
    fn shade_cell(&mut self, row: i32, col: i32, sx: i32, sy: i32, x: i32, y: i32) {
        let dx = x - (8 * sx + 4);
        let dy = y - (8 * sy + 4);
        let sgx = if dx < 0 { -1 } else { 1 };
        let sgy = if dy < 0 { -1 } else { 1 };
        let (ax, ay) = (dx.abs(), dy.abs());
        let s = if dx == 0 && dy == 0 {
            return;
        } else if dx == 0 {
            self.v(row + sgy, col)
        } else if dy == 0 {
            self.v(row, col + sgx)
        } else if ay <= ax {
            let f = (ay << 8) / ax;
            ((256 - f) * self.v(row, col + sgx) + f * self.v(row + sgy, col + sgx)) >> 8
        } else {
            let f = (ax << 8) / ay;
            ((256 - f) * self.v(row + sgy, col) + f * self.v(row + sgy, col + sgx)) >> 8
        };
        self.s[Self::at(row, col)] = s;
    }

    /// Casts the shade over rings 2…m around the grid centre (sub-tile
    /// `centre`), ring by ring outwards so each cell reads neighbours that
    /// are already cast; rays come from the light at `(x, y)`. Rings 0 and
    /// 1 stay unshaded: a wall right next to a light never shadows the
    /// light's own surroundings.
    pub fn shade_rings(&mut self, m: i32, centre: (i32, i32), x: i32, y: i32) {
        for t in 2..=m {
            for s in 0..=t {
                // The ring's eight octant points at offset `s` along it.
                let offs = [
                    (-s, -t),
                    (s, -t),
                    (-s, t),
                    (s, t),
                    (t, -s),
                    (t, s),
                    (-t, -s),
                    (-t, s),
                ];
                for (ox, oy) in offs {
                    self.shade_cell(CENTRE + oy, CENTRE + ox, centre.0 + ox, centre.1 + oy, x, y);
                }
            }
        }
    }
}

/// The shade grid of a shadowed record, `None` when the record's position
/// or radius gives nothing to shade.
pub fn shadow_grid(map: &LightMap, rec: &LightRecord) -> Option<ShadeGrid> {
    let r = rec.radius;
    window(map, rec.x, rec.y, r)?;
    let m = r >> 3;
    let (lx, ly) = (rec.x >> 3, rec.y >> 3);
    let (ox, oy) = map.origin;
    let mut g = ShadeGrid::default();
    for row in CENTRE - m..=CENTRE + m {
        for col in CENTRE - m..=CENTRE + m {
            let gx = lx + col - CENTRE - ox;
            let gy = ly + row - CENTRE - oy;
            let blocked = map.cell(gx, gy).is_none_or(|c| c.blocks != 0);
            g.set_blocker(row, col, if blocked { WALL } else { 0 });
        }
    }
    g.shade_rings(m, (lx, ly), rec.x, rec.y);
    Some(g)
}

/// The value a cell with base value `b` gets under shade `s`: full shade
/// ([`WALL`] or more) drops it, lesser shade dims it by `s / 16`.
pub fn shaded_value(b: i32, s: i32) -> Option<i32> {
    (s < WALL).then(|| (b * (8 - (s >> 1))) >> 3)
}

/// The shadowed contribution (kind 0 at the highest quality): the shade is
/// cast fresh from this frame's blockers, then the light spreads through
/// it.
pub fn shadowed(map: &mut LightMap, rec: &LightRecord, colored: bool) {
    let Some(g) = shadow_grid(map, rec) else {
        return;
    };
    let r = rec.radius;
    let m = r >> 3;
    let Some(w) = window(map, rec.x, rec.y, r) else {
        return;
    };
    spread(map, rec, r, w, colored, |j, i, b| {
        shaded_value(b, g.shade(CENTRE - m + j, CENTRE - m + i))
    });
}

/// Builds a cached record's shade: the blockers of the whole 64 × 64 grid
/// from `blocks` at sub-tile `(ux + col − 32, uy + row − 32)` (the world's
/// walls, not the light map's, so the grid also covers what is off the
/// map), the shade cast over rings 2…m, then the `(2m + 1)²` square of
/// shade around the centre row by row. The grid is centred on the owner's
/// sub-tile `(ux, uy)`.
pub fn build_cache(
    rec: &LightRecord,
    owner_subtile: (i32, i32),
    blocks: impl Fn(i32, i32) -> bool,
) -> Vec<i32> {
    let m = rec.radius >> 3;
    let (ux, uy) = owner_subtile;
    let mut g = ShadeGrid::default();
    for row in 0..GRID {
        for col in 0..GRID {
            if blocks(ux + col - CENTRE, uy + row - CENTRE) {
                g.set_blocker(row, col, WALL);
            }
        }
    }
    g.shade_rings(m, (ux, uy), rec.x, rec.y);
    let mut out = Vec::with_capacity(((2 * m + 1) * (2 * m + 1)) as usize);
    for row in CENTRE - m..=CENTRE + m {
        for col in CENTRE - m..=CENTRE + m {
            out.push(g.shade(row, col));
        }
    }
    out
}

/// The cached contribution (kind 2, every quality): the shade is rebuilt
/// when the record's cache is invalid, then the light spreads through the
/// cached shade.
pub fn cached(
    map: &mut LightMap,
    rec: &mut LightRecord,
    world: &impl LightWorld,
    colored: bool,
) -> Result<(), LightError> {
    let r = rec.radius;
    if !(1..=255).contains(&r) {
        return Ok(());
    }
    if !rec.cache_valid {
        let owner = rec.owner().ok_or(LightError::CacheWithoutOwner)?;
        world
            .owner_room(&owner)
            .ok_or(LightError::CacheWithoutOwner)?;
        let sub = world
            .owner_subtile(&owner)
            .ok_or(LightError::CacheWithoutOwner)?;
        rec.cache = build_cache(rec, sub, |x, y| world.owner_blocks(&owner, x, y));
        rec.cache_valid = true;
    }
    cached_contribution(map, rec, colored);
    Ok(())
}

/// Spreads a record's light through its built cache (`cache_valid` set);
/// nothing when the record gives no light on this map.
pub fn cached_contribution(map: &mut LightMap, rec: &LightRecord, colored: bool) {
    let r = rec.radius;
    if !(1..=255).contains(&r) {
        return;
    }
    let Some(w) = window(map, rec.x, rec.y, r) else {
        return;
    };
    let side = 2 * (r >> 3) + 1;
    spread(map, rec, r, w, colored, |j, i, b| {
        let s = rec.cache[(j * side + i) as usize];
        shaded_value(b, s)
    });
}

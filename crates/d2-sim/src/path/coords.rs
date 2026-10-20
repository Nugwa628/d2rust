//! Coordinates: sub-tiles (5 per tile), 16.16 "precise" positions of
//! dynamic paths, client (drawing) coordinates, square distance.

/// A sub-tile point (5 sub-tiles per tile): path points,
/// targets and placement positions. Signed 32-bit; the
/// path record stores its points as u16 ([`super::record::PathPoint`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Point {
        Point { x, y }
    }
}

/// Sub-tiles per tile.
pub const SUBTILES_PER_TILE: i32 = 5;

/// Fraction of a unit at rest on a cell centre (`PATH_ToFP16Center`).
pub const FP16_CENTER: u32 = 0x8000;

/// Tile → sub-tile.
pub fn tile_to_subtile(t: i32) -> i32 {
    t.wrapping_mul(SUBTILES_PER_TILE)
}

/// Sub-tile → the 16.16 value of its cell centre (rule 2).
pub fn to_fp16_center(sub: i32) -> u32 {
    ((sub as u32) << 16) | FP16_CENTER
}

/// The sub-tile of a 16.16 value: its high 16 bits (rule 2).
pub fn subtile_of(precise: u32) -> i32 {
    (precise >> 16) as i32
}

/// Client coordinates of a dynamic path from its precise values
/// (rule 3): a = px >> 11, b = py >> 11 (arithmetic),
/// (a − b) >> 1, (a + b) >> 2.
pub fn client_from_precise(px: u32, py: u32) -> (i32, i32) {
    let a = (px as i32) >> 11;
    let b = (py as i32) >> 11;
    (a.wrapping_sub(b) >> 1, a.wrapping_add(b) >> 2)
}

/// Client coordinates of a static path from sub-tiles
/// (rule 3): (x − y)·16, (x + y)·8.
pub fn client_from_subtile(x: i32, y: i32) -> (i32, i32) {
    (
        x.wrapping_sub(y).wrapping_mul(16),
        x.wrapping_add(y).wrapping_mul(8),
    )
}

/// Square distance: dx² + dy², no root.
pub fn dist_sq(dx: i32, dy: i32) -> i32 {
    dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy))
}

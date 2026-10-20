//! Edge floors: in levels with
//! `DrawEdges`, after the floor pass's last room, up to four strips of
//! three extra floors near the drawn floors' sub-tile extents.
//!
//! The strips and their tests are computed here. The drawn tile is the
//! act's edge record (the first
//! tile of a fixed key in the act's base library, acts I–III); the caller
//! supplies it, and a strip that needs it in an act without one (IV, V:
//! a zero record) is an error.

use super::{OrderKey, TileRecord, REC_DRAWN, REC_SKIP};
use crate::scene::order::pass;

const SPEC: &str = "render/draw-order-2.md";

/// Margins of the strip tests: `A` for the min sides, `B` for the
/// max sides; with perspective on, 33 and 23.
pub const MARGIN_A: i32 = 30;
pub const MARGIN_B: i32 = 25;
pub const PERSPECTIVE_MARGIN_A: i32 = 33;
pub const PERSPECTIVE_MARGIN_B: i32 = 23;
/// Sub-tiles per tile: the strip snap and step.
pub const SUBTILES: i32 = 5;
/// Edge floors per strip.
pub const STRIP_LEN: usize = 3;

/// Errors of the edge floors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EdgeError {
    /// A strip draws in an act whose edge record holds no tile (acts IV
    /// and V keep it zero; nothing of it is drawn).
    #[error(
        "TODO(spec: {SPEC} open question 2): a strip draws the act edge record (act +0x18), \
         which holds no tile in acts IV and V"
    )]
    NoEdgeRecord,
    /// The strip tests read the extents before any floor was drawn; what
    /// the extents hold then is not known.
    #[error("TODO(spec: {SPEC} §14): drawn extents read before any floor widened them")]
    NoExtents,
}

/// Whether the floor pass draws edge floors: `DrawEdges`
/// ≠ 0, open mode 0 and resolution mode 2.
pub fn edges_active(draw_edges: bool, open_mode: u32, resolution_mode: u32) -> bool {
    draw_edges && open_mode == 0 && resolution_mode == 2
}

/// The sub-tile extents of the drawn floors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Extents {
    pub min_x: i32,
    pub max_x: i32,
    pub min_y: i32,
    pub max_y: i32,
}

/// Tracks the extents through a floor pass: every drawn floor (a record
/// that passed the whole-tile test) and every
/// drawn edge floor widens them by its sub-tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct DrawnExtents {
    bounds: Option<Extents>,
}

impl DrawnExtents {
    pub fn new() -> Self {
        Self::default()
    }

    /// Widens the extents to cover the sub-tile `(x, y)`.
    pub fn widen(&mut self, (x, y): (i32, i32)) {
        self.bounds = Some(match self.bounds {
            None => Extents {
                min_x: x,
                max_x: x,
                min_y: y,
                max_y: y,
            },
            Some(e) => Extents {
                min_x: e.min_x.min(x),
                max_x: e.max_x.max(x),
                min_y: e.min_y.min(y),
                max_y: e.max_y.max(y),
            },
        });
    }

    /// The extents, `None` before the first widen.
    pub fn get(&self) -> Option<Extents> {
        self.bounds
    }
}

/// The four strips in test order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Side {
    /// `px − min x < A`: start (min x − 5, py), step (0, −5).
    MinX,
    /// `max x − px < B`: start (max x + 5, py), step (0, +5).
    MaxX,
    /// `py − min y < A`: start (px, min y − 5), step (−5, 0).
    MinY,
    /// `max y − py < B`: start (px, max y + 5), step (+5, 0).
    MaxY,
}

impl Side {
    pub const ALL: [Side; 4] = [Side::MinX, Side::MaxX, Side::MinY, Side::MaxY];
}

/// `(v / 5) × 5`, C (truncating) division.
pub fn snap(v: i32) -> i32 {
    (v / SUBTILES) * SUBTILES
}

/// The sub-tiles of a strip, or `None` when its test fails. `player` is
/// the player's sub-tile `(px, py)`.
pub fn strip(
    side: Side,
    e: &Extents,
    (px, py): (i32, i32),
    perspective: bool,
) -> Option<[(i32, i32); STRIP_LEN]> {
    let (a, b) = if perspective {
        (PERSPECTIVE_MARGIN_A, PERSPECTIVE_MARGIN_B)
    } else {
        (MARGIN_A, MARGIN_B)
    };
    let (test, start, step) = match side {
        Side::MinX => (px - e.min_x < a, (e.min_x - 5, py), (0, -5)),
        Side::MaxX => (e.max_x - px < b, (e.max_x + 5, py), (0, 5)),
        Side::MinY => (py - e.min_y < a, (px, e.min_y - 5), (-5, 0)),
        Side::MaxY => (e.max_y - py < b, (px, e.max_y + 5), (5, 0)),
    };
    if !test {
        return None;
    }
    let (sx, sy) = (snap(start.0), snap(start.1));
    Some(std::array::from_fn(|k| {
        let k = k as i32;
        (sx + k * step.0, sy + k * step.1)
    }))
}

/// One drawn edge floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EdgeFloor {
    pub side: Side,
    /// 0…2 within the strip.
    pub index: usize,
    /// Absolute sub-tile (a multiple of 5 on both axes).
    pub subtile: (i32, i32),
}

impl EdgeFloor {
    /// The tile of the sub-tile (exact: the sub-tile is snapped).
    pub fn tile(&self) -> (i32, i32) {
        (self.subtile.0 / SUBTILES, self.subtile.1 / SUBTILES)
    }
}

/// The `DrawKey` of the `n`-th drawn edge floor of a frame with `rooms`
/// near rooms: pass 3, after every room's floors (major 2 × rooms), minor
/// in draw order: they draw after the last room.
pub fn edge_key(rooms: usize, n: usize) -> OrderKey {
    OrderKey {
        pass: pass::FLOORS,
        major: 2 * rooms as u32,
        minor: n as u32,
    }
}

/// The edge floors of a frame, run after the floor pass's
/// last room when [`edges_active`]. The four tests run in table order,
/// each reading `extents` as widened by the strips before it.
///
/// Each strip position draws the act's edge record through the floor
/// draw (filter argument 1): flags & 0x408 = 0, then `view` (the camera's
/// whole-tile test at the sub-tile's floor position), then DT1
/// orientation 0. A drawn edge floor gets flag 0x20000 and widens
/// `extents`. The caller's draw (and the splash spawn of a water tile)
/// follows for each returned floor, in order.
pub fn edge_floors(
    extents: &mut DrawnExtents,
    player: (i32, i32),
    perspective: bool,
    mut edge: Option<&mut TileRecord>,
    mut view: impl FnMut(&EdgeFloor) -> bool,
) -> Result<Vec<EdgeFloor>, EdgeError> {
    let mut out = Vec::new();
    for side in Side::ALL {
        let e = extents.get().ok_or(EdgeError::NoExtents)?;
        let Some(cells) = strip(side, &e, player, perspective) else {
            continue;
        };
        let rec = edge.as_deref_mut().ok_or(EdgeError::NoEdgeRecord)?;
        for (index, subtile) in cells.into_iter().enumerate() {
            let floor = EdgeFloor {
                side,
                index,
                subtile,
            };
            if rec.flags & REC_SKIP == 0 && view(&floor) && rec.dt1.orientation == 0 {
                rec.flags |= REC_DRAWN;
                extents.widen(subtile);
                out.push(floor);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "edges_tests.rs"]
mod tests;

//! The draw order of one world frame: which map tiles and units draw, in
//! which pass, and in which order within the pass.
//!
//! A frame is ordered in four steps:
//!
//! 1. [`grid`]: the screen is covered by a diamond grid of draw cells, one
//!    per map tile, with a margin around the view so tall walls and units
//!    standing just outside it still draw.
//! 2. [`fill`]: the near rooms of the local player are walked in array
//!    order. Every wall, shadow and unit inside the view is filed into the
//!    list of its cell (shadow, wall, unit, roof or lower-wall list); walls
//!    are kept sorted by their layer so stacked walls draw bottom up.
//! 3. [`passes`]: the world passes read the lists back cell by cell (lower
//!    walls, floors, shadows, walls and units, roofs) and emit each item
//!    with its [`OrderKey`] (pass, major = cell or room, minor = position
//!    in the list). Walls in front of the player fade to half alpha
//!    ([`fade`]); units hidden from the player by walls do not draw
//!    ([`units`], [`sight`]).
//! 4. [`source`]: the ordered items become the world view's map tiles and
//!    unit slots, and the frame's weather passes are rolled.
//!
//! All of it is plain integer Rust over the map-tile feed ([`NearRooms`]).
//! The order writes its frame state back into the feed: the faded-out,
//! hidden and drawn record flags, the fade bytes, and the drawn / visible
//! unit flags, which the automap reveal and the next frame read.
//!
//! The level backgrounds ([`background`]), the edge floors ([`edges`]) and
//! the weather passes ([`weather`], [`sky`]) live beside the core order.

pub mod background;
pub mod edges;
pub mod sight;
pub mod sky;
pub mod source;
pub mod weather;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use crate::bridge::world::UnitKey;
use crate::scene::order::pass;

use super::camera::{tile_entry, Camera, ClientPos, OpenMode, TileList};
use super::view::BlockRect;

/// The design document of the draw order, named by the view errors the
/// order raises.
const SPEC: &str = "render/draw-order.md";

/// Pool size of a frame: at most 3,000 entries are filed into the cell
/// lists. Entries past it are counted but dropped, so a crowded screen
/// loses its last-filed walls and units rather than growing without bound.
pub const POOL: usize = 3_000;
/// Grid margins in tiles: the grid reaches `MARGIN_A` tiles beyond the
/// view width and `MARGIN_N` beyond its height, and starts `MARGIN_X`
/// tiles left of the view's top-left tile, so walls and units that hang
/// into the view from outside it are still filed.
pub const MARGIN_A: i32 = 11;
pub const MARGIN_N: i32 = 11;
pub const MARGIN_X: i32 = 3;
/// How far a room may lie below or left of the view (client pixels) and
/// still be walked: its tall walls and units can reach up into the view.
pub const ROOM_SLACK_BOTTOM: i32 = 400;
pub const ROOM_SLACK_LEFT: i32 = 200;

/// Tile record flags. `REC_NO_FADE` marks a wall that never fades,
/// `REC_HIDDEN` a record that finished a fade-to-hidden, `REC_FADED_OUT` a
/// record whose alpha reached 0, `REC_DRAWN` a record drawn at least once
/// (the automap reveals those), and `REC_LAYER_BITS` the record's layer
/// plus one (0 for the shadow-like walls of no layer).
pub const REC_NO_FADE: u32 = 0x4;
pub const REC_HIDDEN: u32 = 0x8;
pub const REC_FADED_OUT: u32 = 0x400;
pub const REC_DRAWN: u32 = 0x2_0000;
pub const REC_LAYER_BITS: u32 = 0x1_C000;
const REC_SKIP: u32 = REC_HIDDEN | REC_FADED_OUT;

/// Unit flags. `UNIT_FLAT` forces a unit into the shadow list (it lies on
/// the ground), `UNIT_MISSILE_FLAT` does the same for a missile,
/// `UNIT_DRAWN` marks a unit drawn this frame. In the extended flags,
/// `UNIT_EX_VISIBLE` records that the unit passed its sight test (its
/// shadow draws) and `UNIT_EX_SKIP` takes it out of the world draw.
pub const UNIT_FLAT: u32 = 0x10_0000;
pub const UNIT_MISSILE_FLAT: u32 = 0x1_0000;
pub const UNIT_DRAWN: u32 = 0x1000_0000;
pub const UNIT_EX_VISIBLE: u32 = 0x80;
pub const UNIT_EX_SKIP: u32 = 1 << 18;

/// Unit types.
pub const PLAYER: u8 = 0;
pub const MONSTER: u8 = 1;
pub const OBJECT: u8 = 2;
pub const MISSILE: u8 = 3;
pub const ITEM: u8 = 4;

/// Why a frame cannot be ordered: a broken input the order refuses to
/// guess around, or a rule D2RUST has not decided yet. Neither is ever
/// skipped or defaulted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OrderError {
    /// Fatal: a room's rectangle has left > right or top > bottom.
    #[error("room {room}: rectangle ({left}, {top})–({right}, {bottom}) is inverted (§3 r1)")]
    RoomRect {
        room: usize,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    },
    /// Fatal 0xF9: a type-15 record in a wall list.
    #[error("room {room} wall record {record}: type 15 in a wall list (fatal 0xF9, §6 r4)")]
    RoofInWallList { room: usize, record: usize },
    /// A room unit without a client position.
    #[error("unit ({}, {}) of room {room} has no client position", .key.unit_type, .key.guid)]
    NoPosition { room: usize, key: UnitKey },
    /// The edge floors lack an input.
    #[error(transparent)]
    Edge(#[from] edges::EdgeError),
    /// A rule of the draw order D2RUST has not decided yet decides this
    /// frame (`question` numbers it in the design document).
    #[error("TODO(spec: {SPEC} open question {question}): {message}")]
    Open { question: u8, message: String },
}

fn open(question: u8, message: impl Into<String>) -> OrderError {
    OrderError::Open {
        question,
        message: message.into(),
    }
}

// ---------------------------------------------------------------- the grid

/// The grid cell `q(v)` of a coordinate: `v / 160`, minus one for negative `v`.
pub fn q(v: i32) -> i32 {
    if v >= 0 {
        v / 160
    } else {
        v / 160 - 1
    }
}

/// `T(x, y)`: the tile of a client pixel.
pub fn tile_of(x: i32, y: i32) -> (i32, i32) {
    (q(x + 2 * y), q(2 * y - x))
}

/// A rectangle in client pixels, edges inclusive as the tests compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PixelRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// The draw-cell grid of a frame: side `n`, tile origin `(x0, y0)`,
/// and the view rectangle in client pixels the fill tests use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DrawGrid {
    pub a: i32,
    pub side: i32,
    pub origin: (i32, i32),
    pub view: PixelRect,
}

impl DrawGrid {
    /// The grid for view size `Wv × Hv` and tile origin `(cx_t, cy_t)`.
    pub fn new(wv: i32, hv: i32, tile_origin: ClientPos) -> Self {
        let a = wv / 160 + MARGIN_A;
        let side = a + hv / 80 + MARGIN_N;
        let t = tile_of(tile_origin.x, tile_origin.y);
        DrawGrid {
            a,
            side,
            origin: (t.0 - MARGIN_X, t.1 - a),
            view: PixelRect {
                left: tile_origin.x,
                top: tile_origin.y,
                right: tile_origin.x + wv,
                bottom: tile_origin.y + hv,
            },
        }
    }

    /// The grid of a frame's camera: `Wv`, `Hv` from the view rectangle
    /// (the camera's view), origin the camera's tile origin.
    pub fn of_camera(camera: &Camera) -> Self {
        let v = &camera.view;
        DrawGrid::new(v.right - v.left, v.bottom - v.top, camera.tile)
    }

    /// Number of cells `n²`.
    pub fn cells(&self) -> usize {
        (self.side * self.side) as usize
    }

    /// Cell index `r × n + c` of tile `(tx, ty)`, `None` outside the grid.
    pub fn cell(&self, tile: (i32, i32)) -> Option<usize> {
        let c = tile.0 - self.origin.0;
        let r = tile.1 - self.origin.1;
        let range = 0..self.side;
        (range.contains(&c) && range.contains(&r)).then(|| (r * self.side + c) as usize)
    }
}

// ---------------------------------------------------------------- inputs

/// The DT1 tile fields the order reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Dt1Facts {
    pub orientation: u32,
    pub main: u32,
    pub sub: u32,
    /// +0x04.
    pub roof_height: i32,
    /// +0x08.
    pub height: i32,
    /// Material flags (bit 0x2 starts water effects).
    pub material: u16,
    /// Light direction: picks the wall light points of the lighting
    /// rules.
    pub light_direction: u32,
}

/// A record's fade bytes: state, target and the ramp's ends and clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fade {
    pub state: u8,
    pub alpha: u8,
    pub from: u8,
    pub to: u8,
    pub end: u32,
}

impl Fade {
    /// No fade running, alpha 0xFF.
    pub const OPAQUE: Fade = Fade {
        state: 0,
        alpha: 0xFF,
        from: 0,
        to: 0,
        end: 0,
    };
}

/// The coordinate record a wall record points at (one logical room of a
/// preset): its box corner (x0, y0, level tiles) and index. Set only by
/// the grid build of a preset room with lvlprest
/// `Logicals` ≠ 0; every other record has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Logical {
    pub x0: i32,
    pub y0: i32,
    pub index: i32,
}

/// One 0x30-byte tile record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileRecord {
    /// Tile position in the room (+0x08, +0x0C).
    pub tile: (i32, i32),
    /// +0x14.
    pub flags: u32,
    /// +0x1C.
    pub ty: u32,
    pub dt1: Dt1Facts,
    pub fade: Fade,
    /// The coordinate record (the group-mode fade's input), `None` for 0.
    pub logical: Option<Logical>,
}

impl TileRecord {
    /// `ℓ`: record flag bits 14–16 (layer + 1).
    pub fn layer(&self) -> u32 {
        (self.flags & REC_LAYER_BITS) >> 14
    }
}

/// What the order reads of a unit besides its position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct UnitFacts {
    pub unit_type: u8,
    pub mode: u32,
    pub flags: u32,
    pub flag_ex: u32,
    /// The local player.
    pub local: bool,
    /// monstats2 `unflatDead` (bit 20).
    pub unflat_dead: bool,
    /// objects `DrawUnder`.
    pub draw_under: u8,
    /// States 7 `playerbody`, 143 `attached`, 146 `invis`.
    pub playerbody: bool,
    pub attached: bool,
    pub invis: bool,
    /// The answer of the sight test (
    /// [`sight::sight_hidden`] over the client DRLG's collision rooms),
    /// `true` = hidden. The feed fills it; `None` (the feed has no
    /// collision rooms yet) is an error for a unit the test applies to.
    pub sight_hidden: Option<bool>,
}

/// A unit of a room's unit list (room +0x74, list order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RoomUnit {
    pub key: UnitKey,
    pub facts: UnitFacts,
}

/// A room's tile rectangle (+0x5C x, +0x60 y, +0x64 w, +0x68 h).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TileRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// One room of the near-room array.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Room {
    /// Level id of the room (the automap reveal walks the player level's
    /// rooms only).
    pub level: u32,
    pub tiles: TileRect,
    /// Subtile origin (+0x4C, +0x50), for the fade test.
    pub subtile_origin: (i32, i32),
    pub walls: Vec<TileRecord>,
    pub floors: Vec<TileRecord>,
    pub shadows: Vec<TileRecord>,
    /// The room's unit list, head first. The fill runs one y-sort pass on
    /// it in place, so the order carries over from frame to frame.
    pub units: Vec<RoomUnit>,
    /// Set by the fill when it sorted `units` (the room passed the room
    /// test and units were not skipped): the owner writes the order back
    /// to the client's list, where it persists.
    pub units_sorted: bool,
}

/// The level fields the passes read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LevelFacts {
    /// Level id of the local player's room (the background pass reads it).
    pub id: u32,
    /// levels `DrawEdges` (+9).
    pub draw_edges: bool,
    /// The geometric fade branch when `true`. The game runs in group mode
    /// (`false`, the default and the only live value); the geometric
    /// branch is kept for its rule only.
    pub fade_geometric: bool,
}

/// The map-tile feed of a frame: the near-room array of the local
/// player's active room, in array order, plus the player's tile.
/// Owned by the client DRLG copy; the order
/// writes the frame's flag and fade changes back into it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NearRooms {
    pub rooms: Vec<Room>,
    /// The player's tile `(px, py)`: the path subtile / 5.
    pub player_tile: (i32, i32),
    /// The player's coordinate-record index (recomputed each frame from
    /// the player's room and path sub-tile): 0 when the point is in no
    /// room, −1 for a null record.
    pub player_logical: i32,
    /// The player's path sub-tile, the reference of the edge floor strips.
    pub player_subtile: (i32, i32),
    /// The act's edge floor record: `None` in acts IV and V, whose record
    /// holds no tile.
    pub edge: Option<TileRecord>,
    pub level: LevelFacts,
}

/// The fade clock of a frame: `now` is the `GetTickCount()` millisecond
/// count (`u32`, wrapping); `instant` is true for the display types up to
/// 3 (GDI and DirectDraw), where every ramp completes at its first walk
/// and `now` only matters for the unreachable bit-2 branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FadeClock {
    pub now: u32,
    pub instant: bool,
}

// ---------------------------------------------------------------- fill

/// Which record array of a room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TileArray {
    Wall,
    Floor,
    Shadow,
    /// The act's edge floor record ([`NearRooms::edge`]): its items carry
    /// room and record 0.
    Edge,
}

/// A grid entry: kind 0 unit, 1 tile, 2 unit shadow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Entry {
    Unit {
        room: usize,
        unit: usize,
    },
    Tile {
        room: usize,
        array: TileArray,
        record: usize,
    },
    UnitShadow {
        room: usize,
        unit: usize,
    },
}

/// A wall-array record in a layer-sorted list, with its `ℓ`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Layered {
    pub room: usize,
    pub record: usize,
    pub layer: u32,
}

/// One draw cell: flag word and five lists.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Cell {
    pub flags: u32,
    pub shadow: Vec<Entry>,
    pub wall: Vec<Layered>,
    pub unit: Vec<Entry>,
    pub roof: Vec<Layered>,
    pub lower: Vec<Layered>,
}

/// The grid flags at view +0x38.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct GridFlags {
    pub lower_walls: bool,
    pub roofs: bool,
    pub shadows: bool,
}

/// The filled grid of a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lists {
    pub grid: DrawGrid,
    pub cells: Vec<Cell>,
    /// Entries asked of the pool (+0xEA9C), dropped ones included.
    pub count: usize,
    pub flags: GridFlags,
}

/// Layer-sorted insertion: the first element with a successor whose
/// `ℓ` exceeds the new one gets it in front; else append. The last
/// element is never compared.
pub fn insert_layered<T>(list: &mut Vec<T>, item: T, layer: impl Fn(&T) -> u32) {
    let l = layer(&item);
    let at = (0..list.len().saturating_sub(1)).find(|&i| l < layer(&list[i]));
    match at {
        Some(i) => list.insert(i, item),
        None => list.push(item),
    }
}

/// A flat unit: the unit kind's flat test or unit flag 0x100000.
pub fn is_flat(u: &UnitFacts) -> bool {
    u.flags & UNIT_FLAT != 0
        || match u.unit_type {
            PLAYER => u.mode == 17,
            MONSTER => u.mode == 12 && !u.unflat_dead,
            OBJECT => u.draw_under & 2 != 0 || (u.draw_under & 1 != 0 && u.mode == 2),
            MISSILE => u.flags & UNIT_MISSILE_FLAT != 0,
            ITEM => u.mode == 3,
            _ => false,
        }
}

/// Room corner `E(x, y)`.
pub fn room_corner(x: i32, y: i32) -> (i32, i32) {
    ((x - y) * 80 - 80, (x + y) * 40 + 80)
}

/// The room's rectangle in client pixels.
pub fn room_rect(index: usize, t: &TileRect) -> Result<PixelRect, OrderError> {
    let r = PixelRect {
        left: room_corner(t.x, t.y + t.h).0,
        top: room_corner(t.x, t.y).1,
        right: room_corner(t.x + t.w, t.y).0,
        bottom: room_corner(t.x + t.w, t.y + t.h).1,
    };
    if r.left > r.right || r.top > r.bottom {
        return Err(OrderError::RoomRect {
            room: index,
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        });
    }
    Ok(r)
}

/// Whether the room is filed: its rectangle meets the view.
pub fn room_visible(room: &PixelRect, view: &PixelRect) -> bool {
    room.left <= view.right
        && room.top <= view.bottom + ROOM_SLACK_BOTTOM
        && room.right >= view.left - ROOM_SLACK_LEFT
        && room.bottom >= view.top
}

/// Whether a record at entry `(e0, e1)` with DT1 height `h`
/// and roof height `rh` (0 for the shadow array) is kept.
pub fn record_visible(e: (i32, i32), rh: i32, h: i32, view: &PixelRect) -> bool {
    e.0 <= view.right
        && e.0 + 160 >= view.left
        && e.1 - rh + h <= view.bottom
        && e.1 - rh - h >= view.top
}

struct Filler {
    cells: Vec<Cell>,
    count: usize,
    flags: GridFlags,
}

impl Filler {
    /// Takes a pool entry: `false` past the 3,000th (dropped).
    fn take(&mut self) -> bool {
        self.count += 1;
        self.count <= POOL
    }
}

fn absolute(room: &Room, rec: &TileRecord) -> (i32, i32) {
    (room.tiles.x + rec.tile.0, room.tiles.y + rec.tile.1)
}

/// The cell of a tile record: `T(e0, e1) − (0, 1)`.
fn record_cell(grid: &DrawGrid, e: (i32, i32)) -> Option<usize> {
    let t = tile_of(e.0, e.1);
    grid.cell((t.0, t.1 - 1))
}

/// Fills the grid from the near rooms in array order. `positions`
/// holds each room unit's client position. Wall records filed into a
/// wall list get their fade target update.
pub fn fill(
    grid: &DrawGrid,
    near: &mut NearRooms,
    positions: &BTreeMap<UnitKey, ClientPos>,
    clock: FadeClock,
    skip_units: bool,
) -> Result<Lists, OrderError> {
    let mut f = Filler {
        cells: vec![Cell::default(); grid.cells()],
        count: 0,
        flags: GridFlags::default(),
    };
    let view = grid.view;
    let player = near.player_tile;
    let player_logical = near.player_logical;
    let geometric = near.level.fade_geometric;
    for (ri, room) in near.rooms.iter_mut().enumerate() {
        let rect = room_rect(ri, &room.tiles)?;
        if !room_visible(&rect, &view) {
            continue;
        }
        // The wall array.
        for i in 0..room.walls.len() {
            let abs = absolute(room, &room.walls[i]);
            let rec = &mut room.walls[i];
            let e = tile_entry(abs.0, abs.1);
            if !record_visible(e, rec.dt1.roof_height, rec.dt1.height, &view) {
                continue;
            }
            let Some(ci) = record_cell(grid, e) else {
                continue;
            };
            f.cells[ci].flags = 0;
            let layered = Layered {
                room: ri,
                record: i,
                layer: rec.layer(),
            };
            let flag4 = !matches!(rec.ty, 0 | 13) && rec.flags & REC_NO_FADE == 0;
            if rec.flags & REC_LAYER_BITS == 0 {
                if f.take() {
                    f.flags.shadows = true;
                    f.cells[ci].shadow.push(Entry::Tile {
                        room: ri,
                        array: TileArray::Wall,
                        record: i,
                    });
                }
            } else if rec.ty == 15 {
                if f.take() {
                    f.flags.roofs = true;
                    insert_layered(&mut f.cells[ci].roof, layered, |l| l.layer);
                }
            } else if (16..=19).contains(&rec.ty) {
                // Cell flag 4 comes before the pool test (pool overflow).
                if flag4 {
                    f.cells[ci].flags |= 4;
                }
                if f.take() {
                    f.flags.lower_walls = true;
                    insert_layered(&mut f.cells[ci].lower, layered, |l| l.layer);
                }
            } else {
                if flag4 {
                    f.cells[ci].flags |= 4;
                }
                // A dropped wall gets no fade target (pool overflow:
                // it is dropped before the clock is read).
                if !f.take() {
                    continue;
                }
                if rec.flags & REC_NO_FADE == 0 {
                    let near = if geometric {
                        let sub = (
                            room.subtile_origin.0 + 5 * rec.tile.0,
                            room.subtile_origin.1 + 5 * rec.tile.1,
                        );
                        fade_near(sub, player, rec.ty)
                    } else {
                        fade_near_group(rec.logical, player_logical, player)
                    };
                    retarget(&mut rec.fade, near, clock.now);
                }
                insert_layered(&mut f.cells[ci].wall, layered, |l| l.layer);
            }
        }
        // The shadow array.
        for (i, rec) in room.shadows.iter().enumerate() {
            let abs = (room.tiles.x + rec.tile.0, room.tiles.y + rec.tile.1);
            let e = tile_entry(abs.0, abs.1);
            if !record_visible(e, 0, rec.dt1.height, &view) {
                continue;
            }
            let Some(ci) = record_cell(grid, e) else {
                continue;
            };
            if f.take() {
                f.flags.shadows = true;
                f.cells[ci].shadow.push(Entry::Tile {
                    room: ri,
                    array: TileArray::Shadow,
                    record: i,
                });
            }
        }
        // The units, after the in-place Y sort of the room's list (one
        // adjacent-swap pass from the head, swapping while the earlier y
        // is strictly greater, signed; the order persists to the next
        // frame's pass).
        if skip_units {
            continue;
        }
        for unit in &room.units {
            if !positions.contains_key(&unit.key) {
                return Err(OrderError::NoPosition {
                    room: ri,
                    key: unit.key,
                });
            }
        }
        for i in 1..room.units.len() {
            if positions[&room.units[i].key].y < positions[&room.units[i - 1].key].y {
                room.units.swap(i - 1, i);
            }
        }
        room.units_sorted = true;
        for (ui, unit) in room.units.iter().enumerate() {
            let at = positions.get(&unit.key).ok_or(OrderError::NoPosition {
                room: ri,
                key: unit.key,
            })?;
            let Some(ci) = grid.cell(tile_of(at.x, at.y)) else {
                continue;
            };
            if is_flat(&unit.facts) {
                if f.take() {
                    f.flags.shadows = true;
                    f.cells[ci].shadow.push(Entry::Unit { room: ri, unit: ui });
                }
                continue;
            }
            if f.take() {
                f.cells[ci].unit.push(Entry::Unit { room: ri, unit: ui });
            }
            if unit.facts.flag_ex & UNIT_EX_VISIBLE != 0 && f.take() {
                f.flags.shadows = true;
                f.cells[ci]
                    .shadow
                    .push(Entry::UnitShadow { room: ri, unit: ui });
            }
        }
    }
    Ok(Lists {
        grid: *grid,
        cells: f.cells,
        count: f.count,
        flags: f.flags,
    })
}

// ---------------------------------------------------------------- units

/// Units the entry skips.
pub fn unit_skipped(u: &UnitFacts) -> bool {
    u.flag_ex & UNIT_EX_SKIP != 0
        || (u.unit_type == PLAYER && u.mode == 17 && !u.playerbody)
        || (matches!(u.unit_type, MONSTER | OBJECT) && (u.attached || u.invis))
}

/// Units the sight test applies to.
pub fn sight_tested(u: &UnitFacts) -> bool {
    match u.unit_type {
        PLAYER => !u.local && !matches!(u.mode, 0 | 17),
        MONSTER => !matches!(u.mode, 0 | 12),
        MISSILE | ITEM => true,
        _ => false,
    }
}

/// The unit entry: whether the unit draws, with its flag writes (sight
/// hidden / seen, drawn).
pub fn unit_draws(u: &mut UnitFacts) -> Result<bool, OrderError> {
    if unit_skipped(u) {
        return Ok(false);
    }
    let hidden = if sight_tested(u) {
        u.sight_hidden.ok_or_else(|| {
            open(
                9,
                "no sight answer (draw-order-2.md §15) for a sight-tested unit",
            )
        })?
    } else {
        false
    };
    if hidden {
        u.flag_ex &= !UNIT_EX_VISIBLE;
        return Ok(false);
    }
    u.flag_ex |= UNIT_EX_VISIBLE;
    u.flags |= UNIT_DRAWN;
    Ok(true)
}

// ---------------------------------------------------------------- fade

/// "Near" in the unused geometric branch (never live; the game runs in
/// group mode): the record's absolute subtile `sub` against the player's tile.
pub fn fade_near(sub: (i32, i32), player: (i32, i32), ty: u32) -> bool {
    let (lx, ly) = sub;
    let (px, py) = player;
    (5 * px < lx && lx < 5 * px + 20 && matches!(ty, 1 | 4 | 5 | 7 | 8 | 10 | 12))
        || (5 * py < ly && ly < 5 * py + 20 && matches!(ty, 2 | 3 | 6 | 7 | 9 | 11 | 12))
}

/// "Near" in the group mode (the live mode):
/// the record's coordinate record `G` is set, its index differs from the
/// player's, and the player's tile lies before `G`'s corner on x or y —
/// the wall belongs to another logical room in front of the player.
pub fn fade_near_group(g: Option<Logical>, player_index: i32, player: (i32, i32)) -> bool {
    g.is_some_and(|g| g.index != player_index && (player.0 < g.x0 || player.1 < g.y0))
}

/// The fade target update at `now` (`t = now + 500`).
pub fn retarget(fade: &mut Fade, near: bool, now: u32) {
    let t = now.wrapping_add(500);
    let alpha = i32::from(fade.alpha);
    if near && fade.state & 1 == 0 {
        fade.from = 0xFF;
        fade.to = 0x80;
        fade.end = t.wrapping_add_signed(500 * (alpha - 0xFF) / 127);
        fade.state |= 3;
    } else if !near && fade.state & 1 != 0 {
        fade.from = 0x80;
        fade.to = 0xFF;
        fade.end = t.wrapping_add_signed(500 * (0x80 - alpha) / 127);
        fade.state = (fade.state & !1) | 2;
    }
}

/// Advances a running fade (state bit 1) at the clock and writes
/// flags 0x400 / 0x8. Clock arithmetic: "end reached" is the
/// unsigned `end ≤ now`; the ramp `(to − from) × ((now − end) + 500)` is a
/// wrapping 32-bit signed product, C-divided by 500, its low byte added
/// to `from` mod 256 (so a ramp across the `GetTickCount` wrap
/// overshoots; this is kept on purpose).
pub fn advance(rec: &mut TileRecord, clock: FadeClock) {
    let f = &mut rec.fade;
    if f.state & 2 == 0 {
        return;
    }
    let reached = f.end <= clock.now;
    if clock.instant || reached {
        f.alpha = f.to;
        f.state &= !2;
    } else {
        let span = (clock.now.wrapping_sub(f.end) as i32).wrapping_add(500);
        let diff = i32::from(f.to) - i32::from(f.from);
        let step = diff.wrapping_mul(span) / 500;
        f.alpha = f.from.wrapping_add(step as u8);
    }
    if f.alpha == 0 {
        rec.flags |= REC_FADED_OUT;
    } else {
        rec.flags &= !REC_FADED_OUT;
    }
    if f.state & 4 != 0 && reached {
        rec.flags |= REC_HIDDEN;
    }
}

// ---------------------------------------------------------------- passes

/// Whether the draw of an ordered tile sets record flag 0x20000,
/// decided after the drawer's culling. Walls and lower
/// walls: the wall drawer returned non-zero, i.e. at least one block
/// (screen position = handed (X, Y) + the block's (x, y)) passed the
/// block test. Floors and roofs: the whole-tile test passed. Shadow tiles
/// never set it (units are not tiles).
pub fn sets_drawn_flag(camera: &Camera, tile: &OrderedTile, blocks: &[BlockRect]) -> bool {
    let list = source::placement_list(tile);
    let handed = camera.tile_handed(list, tile.cell.0, tile.cell.1);
    match tile.kind {
        TileKind::ShadowTile => false,
        TileKind::Floor { .. } | TileKind::Roof { .. } => camera.floor_roof_visible(handed),
        TileKind::Wall | TileKind::LowerWall => blocks
            .iter()
            .any(|b| camera.wall_block_visible(handed.0 + b.x, handed.1 + b.y)),
    }
}

/// Writes flag 0x20000 into the records whose draw set it.
pub fn mark_drawn(near: &mut NearRooms, drawn: &[(usize, TileArray, usize)]) {
    for &(room, array, record) in drawn {
        let rec = match array {
            TileArray::Edge => match near.edge.as_mut() {
                Some(e) => e,
                None => continue,
            },
            TileArray::Wall => &mut near.rooms[room].walls[record],
            TileArray::Floor => &mut near.rooms[room].floors[record],
            TileArray::Shadow => &mut near.rooms[room].shadows[record],
        };
        rec.flags |= REC_DRAWN;
    }
}

/// The `DrawKey` fields of an item; `sub` is the unit composite's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OrderKey {
    pub pass: u32,
    pub major: u32,
    pub minor: u32,
}

/// How an ordered tile is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TileKind {
    /// Wall drawer at the wall position.
    LowerWall,
    /// Floor drawer; `layer` is `ℓ` (1 or 2).
    Floor { layer: u32 },
    /// The shadow-tile drawer at the wall position, draw mode 4.
    ShadowTile,
    /// Wall drawer at the wall position.
    Wall,
    /// Floor drawer at the roof position, roof pass `L` (1–4).
    Roof { pass: u32 },
}

/// A tile item in draw order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OrderedTile {
    pub room: usize,
    pub array: TileArray,
    pub record: usize,
    pub kind: TileKind,
    /// Absolute tile `(tx, ty)`.
    pub cell: (i32, i32),
    pub dt1: Dt1Facts,
    /// The record's alpha byte at its draw (0xFF opaque).
    pub alpha: u8,
    /// The record's fade state as this frame's filing left it (the
    /// lower-wall advance changes only bit 1); bit 0 picks the faded wall
    /// light points.
    pub fade_state: u8,
    pub key: OrderKey,
}

/// One item of the frame in draw order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ordered {
    Tile(OrderedTile),
    /// A unit drawn through the unit composite.
    Unit {
        key: UnitKey,
        at: OrderKey,
    },
    /// A unit's shadow (the shadow pass's unit draw).
    UnitShadow {
        key: UnitKey,
        at: OrderKey,
    },
}

/// A unit's slot in the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnitSlot {
    /// No draw order computed this frame (no map feed).
    Unordered,
    /// Not filed, skipped or hidden.
    NotDrawn,
    Drawn(OrderKey),
}

/// The frame's draw order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FrameOrder {
    pub items: Vec<Ordered>,
    /// Pool entries asked this frame.
    pub count: usize,
}

impl FrameOrder {
    /// The slot of a unit: drawn with its key, else not drawn.
    pub fn unit_slot(&self, key: UnitKey) -> UnitSlot {
        self.items
            .iter()
            .find_map(|o| match o {
                Ordered::Unit { key: k, at } if *k == key => Some(UnitSlot::Drawn(*at)),
                _ => None,
            })
            .unwrap_or(UnitSlot::NotDrawn)
    }
}

fn key(pass: u32, major: usize, minor: usize) -> OrderKey {
    OrderKey {
        pass,
        major: major as u32,
        minor: minor as u32,
    }
}

struct Passes<'a> {
    near: &'a mut NearRooms,
    clock: FadeClock,
    out: Vec<Ordered>,
}

impl Passes<'_> {
    fn tile(&mut self, room: usize, array: TileArray, record: usize, kind: TileKind, at: OrderKey) {
        let r = &self.near.rooms[room];
        let rec = match array {
            TileArray::Wall => &r.walls[record],
            TileArray::Floor => &r.floors[record],
            TileArray::Shadow => &r.shadows[record],
            TileArray::Edge => unreachable!("edge floors are placed by order_frame"),
        };
        self.out.push(Ordered::Tile(OrderedTile {
            room,
            array,
            record,
            kind,
            cell: absolute(r, rec),
            dt1: rec.dt1,
            alpha: rec.fade.alpha,
            fade_state: rec.fade.state,
            key: at,
        }));
    }

    fn wall_record(&mut self, l: &Layered) -> &mut TileRecord {
        &mut self.near.rooms[l.room].walls[l.record]
    }

    /// Lower walls and walls: advance the fade, then whether it draws.
    fn walk_wall(&mut self, l: &Layered) -> bool {
        let clock = self.clock;
        let rec = self.wall_record(l);
        advance(rec, clock);
        rec.flags & REC_LAYER_BITS != 0 && rec.flags & REC_SKIP == 0
    }

    fn unit(&mut self, room: usize, unit: usize, at: OrderKey) -> Result<(), OrderError> {
        let u = &mut self.near.rooms[room].units[unit];
        if unit_draws(&mut u.facts)? {
            self.out.push(Ordered::Unit { key: u.key, at });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- automap reveal

/// The automap reveal state: the countdown and
/// the position of the last reveal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct AutomapReveal {
    pub countdown: u32,
    pub last: (i32, i32),
}

/// A record the reveal adds to the automap (the automap owns what it
/// does with it).
pub type RevealedRecord = (usize, TileArray, usize);

/// The reveal of one room: the room's floor array then its wall array, in
/// order; a record is added when it lacks flag 0x8 and has flag 0x20000,
/// or always when `all` (reveal everything). The room objects follow;
/// the automap handles them.
pub fn reveal_room(ri: usize, room: &Room, all: bool, out: &mut Vec<RevealedRecord>) {
    let keep = |r: &TileRecord| all || (r.flags & REC_HIDDEN == 0 && r.flags & REC_DRAWN != 0);
    for (i, r) in room.floors.iter().enumerate() {
        if keep(r) {
            out.push((ri, TileArray::Floor, i));
        }
    }
    for (i, r) in room.walls.iter().enumerate() {
        if keep(r) {
            out.push((ri, TileArray::Wall, i));
        }
    }
}

impl AutomapReveal {
    /// One frame of the reveal (run once per drawn frame): a
    /// non-zero countdown is decremented instead; otherwise, when the
    /// player moved by `d = (2·max + min) / 2 ≥ 0x50` since the last
    /// reveal, stores the position and reveals every near room whose
    /// level has the same leveldefs `Layer` as the player's level
    /// (`level`; `layer` maps a level id to its `Layer`), not only the
    /// rooms of the player's level.
    pub fn frame(
        &mut self,
        player: (i32, i32),
        level: u32,
        near: &NearRooms,
        layer: impl Fn(u32) -> u32,
    ) -> Vec<RevealedRecord> {
        let mut out = Vec::new();
        if self.countdown != 0 {
            self.countdown -= 1;
            return out;
        }
        let dx = (player.0 - self.last.0).abs();
        let dy = (player.1 - self.last.1).abs();
        let d = (2 * dx.max(dy) + dx.min(dy)) / 2;
        if d < 0x50 {
            return out;
        }
        self.last = player;
        let l = layer(level);
        for (ri, room) in near.rooms.iter().enumerate() {
            if layer(room.level) == l {
                reveal_room(ri, room, false, &mut out);
            }
        }
        out
    }
}

/// The draw order of a frame through the grid of
/// `camera`. Open mode 3 draws no world where the two side panels cover
/// the frame ([`crate::rules::camera::FrameSize::draws_world`]: every 4:3
/// frame).
pub fn order_frame(
    camera: &Camera,
    mode: OpenMode,
    near: &mut NearRooms,
    positions: &BTreeMap<UnitKey, ClientPos>,
    clock: FadeClock,
) -> Result<FrameOrder, OrderError> {
    if !camera.size.draws_world(mode) {
        return Ok(FrameOrder::default());
    }
    let mut order = order_grid(&DrawGrid::of_camera(camera), near, positions, clock)?;
    let active = edges::edges_active(
        near.level.draw_edges,
        u32::from(mode.get()),
        camera.size.resolution_mode(),
    );
    if active {
        edge_floors(camera, near, &mut order)?;
    }
    Ok(order)
}

/// The edge floors of the frame, after the floor
/// pass's last room: the drawn extents are the sub-tiles (tile × 5) of
/// the room floors that pass the whole-tile test at their floor position,
/// and each drawn edge floor is a
/// floor item (`ℓ` 1, filter argument 1) of the act's edge record keyed
/// by [`edges::edge_key`].
fn edge_floors(
    camera: &Camera,
    near: &mut NearRooms,
    order: &mut FrameOrder,
) -> Result<(), OrderError> {
    let visible = |cell: (i32, i32)| {
        camera.floor_roof_visible(camera.tile_handed(TileList::Floor, cell.0, cell.1))
    };
    let mut extents = edges::DrawnExtents::new();
    let mut at = order.items.len();
    for (i, item) in order.items.iter().enumerate() {
        let key = match item {
            Ordered::Tile(t) => {
                if matches!(t.kind, TileKind::Floor { .. }) && visible(t.cell) {
                    extents.widen((t.cell.0 * edges::SUBTILES, t.cell.1 * edges::SUBTILES));
                }
                t.key
            }
            Ordered::Unit { at, .. } | Ordered::UnitShadow { at, .. } => *at,
        };
        if key.pass > pass::FLOORS {
            at = at.min(i);
        }
    }
    let rooms = near.rooms.len();
    let floors = edges::edge_floors(
        &mut extents,
        near.player_subtile,
        false,
        near.edge.as_mut(),
        |f| visible(f.tile()),
    )?;
    let Some(rec) = near.edge else {
        return Ok(());
    };
    let items = floors.iter().enumerate().map(|(n, f)| {
        Ordered::Tile(OrderedTile {
            room: 0,
            array: TileArray::Edge,
            record: 0,
            kind: TileKind::Floor { layer: 1 },
            cell: f.tile(),
            dt1: rec.dt1,
            alpha: rec.fade.alpha,
            fade_state: rec.fade.state,
            key: edges::edge_key(rooms, n),
        })
    });
    order.items.splice(at..at, items.collect::<Vec<_>>());
    Ok(())
}

/// [`order_frame`] for a given grid, without the edge floors (they need
/// the camera's view test).
pub fn order_grid(
    grid: &DrawGrid,
    near: &mut NearRooms,
    positions: &BTreeMap<UnitKey, ClientPos>,
    clock: FadeClock,
) -> Result<FrameOrder, OrderError> {
    // Pass 1: level backgrounds (levels 74 and 120) are drawn by the
    // world view's pass-1 layer (`world_view::background_view`).
    let lists = fill(grid, near, positions, clock, false)?;
    let mut p = Passes {
        near,
        clock,
        out: Vec::new(),
    };

    // Pass 2: lower walls.
    if lists.flags.lower_walls {
        for (ci, cell) in lists.cells.iter().enumerate() {
            for (pos, l) in cell.lower.iter().enumerate() {
                if p.walk_wall(l) {
                    let k = key(pass::LOWER_WALLS, ci, pos);
                    p.tile(l.room, TileArray::Wall, l.record, TileKind::LowerWall, k);
                }
            }
        }
    }

    // Pass 3: floors, straight from the rooms.
    for ri in 0..p.near.rooms.len() {
        for layer in 1..=2 {
            for i in 0..p.near.rooms[ri].floors.len() {
                let rec = &p.near.rooms[ri].floors[i];
                if rec.layer() == layer && rec.flags & REC_SKIP == 0 && rec.dt1.orientation == 0 {
                    let k = key(pass::FLOORS, 2 * ri + (layer as usize - 1), i);
                    p.tile(ri, TileArray::Floor, i, TileKind::Floor { layer }, k);
                }
            }
        }
    }

    // Pass 4: the environment pools come from
    // the feed's weather state (`weather::Weather::pass4`, source.rs).

    // Pass 5: the shadow pass.
    if lists.flags.shadows {
        for (ci, cell) in lists.cells.iter().enumerate() {
            for (pos, e) in cell.shadow.iter().enumerate() {
                let k = key(pass::SHADOWS, ci, pos);
                match *e {
                    Entry::Unit { room, unit } => p.unit(room, unit, k)?,
                    Entry::Tile {
                        room,
                        array,
                        record,
                    } => {
                        let r = &p.near.rooms[room];
                        let rec = match array {
                            TileArray::Shadow => &r.shadows[record],
                            _ => &r.walls[record],
                        };
                        if rec.flags & REC_SKIP == 0 {
                            p.tile(room, array, record, TileKind::ShadowTile, k);
                        }
                    }
                    Entry::UnitShadow { room, unit } => {
                        let key = p.near.rooms[room].units[unit].key;
                        p.out.push(Ordered::UnitShadow { key, at: k });
                    }
                }
            }
            // The wall list's records without layer bits (none in practice):
            // the walk still advances the fades.
            for l in &cell.wall {
                let clock = p.clock;
                advance(p.wall_record(l), clock);
            }
        }
    }

    // Pass 6: walls and units.
    for (ci, cell) in lists.cells.iter().enumerate() {
        for (pos, l) in cell.wall.iter().enumerate() {
            if p.near.rooms[l.room].walls[l.record].ty == 15 {
                return Err(OrderError::RoofInWallList {
                    room: l.room,
                    record: l.record,
                });
            }
            if p.walk_wall(l) {
                let k = key(pass::WALLS_UNITS, ci, pos);
                p.tile(l.room, TileArray::Wall, l.record, TileKind::Wall, k);
            }
        }
        let walls = cell.wall.len();
        for (pos, e) in cell.unit.iter().enumerate() {
            if let Entry::Unit { room, unit } = *e {
                p.unit(room, unit, key(pass::WALLS_UNITS, ci, walls + pos))?;
            }
        }
    }

    // Pass 7: roofs, four times by layer mask.
    if lists.flags.roofs {
        let n2 = lists.grid.cells();
        for roof_pass in 1..=4u32 {
            let mask = roof_pass << 14;
            for (ci, cell) in lists.cells.iter().enumerate() {
                for (pos, l) in cell.roof.iter().enumerate() {
                    let clock = p.clock;
                    let rec = p.wall_record(l);
                    advance(rec, clock);
                    if rec.flags & mask == mask && rec.flags & REC_SKIP == 0 {
                        let major = (roof_pass as usize - 1) * n2 + ci;
                        let k = key(pass::ROOFS, major, pos);
                        let kind = TileKind::Roof { pass: roof_pass };
                        p.tile(l.room, TileArray::Wall, l.record, kind, k);
                    }
                }
            }
        }
    }

    // Pass 8 never runs (`background::pass8_items`); pass 9 is
    // `weather::Weather::pass9` (`source.rs`); the screen fade (pass 10)
    // has no D2RUST input yet.

    Ok(FrameOrder {
        items: p.out,
        count: lists.count,
    })
}

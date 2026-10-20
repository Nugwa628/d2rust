//! Logical rooms (D2MOO `DrlgDrlgLogic.cpp`): the coordinate lists a room gets at the end
//! of its tile fill, the grid build of preset rooms with lvlprest `Logicals`, and the
//! lookups monster population reads. Building draws nothing from any seed.
//!
//! A logical room is a set of coordinate records: rectangles of tiles that share an index,
//! so that population can tell which connected region of walkable floor a point is in.

use super::level::Drlg;
use super::room::near_gaps;
use super::tiles::{CellGrid, TileRecord};
use super::{DrlgRoomId, TileRect, SUBTILES};

/// Info flag 1: one record for the whole room.
pub const INFO_ONE: u32 = 1;
/// Info flag 2: built from grids.
pub const INFO_GRID: u32 = 2;
/// Index-cell bit: the cell has been assigned to a region.
pub const VISITED: u32 = 0x1000_0000;
/// Index-cell bit: the region is a node region.
pub const NODE: u32 = 0x2000_0000;
/// Mask of the region index in an index cell.
pub const INDEX_MASK: u32 = 0x0FFF_FFFF;

/// A coordinate record: a tile rectangle with a region index.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CoordRec {
    /// x0, y0, x1, y1 in level tiles (x1 / y1 exclusive).
    pub boxr: [i32; 4],
    /// The clipped box population reads.
    pub clipped: [i32; 4],
    /// Node flag.
    pub node: bool,
    /// Region index.
    pub index: u32,
}

/// The grids the grid build reads, cut like the tile-fill grids ((W+1) × (H+1),
/// room-relative): wall layer 0's orientation grid, floor layer 0 and wall layer 0.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogicGrids {
    /// Wall layer 0's orientation grid.
    pub orientation: CellGrid,
    /// Floor layer 0.
    pub floor: CellGrid,
    /// Wall layer 0.
    pub wall: CellGrid,
}

/// Logical-room info of a DRLG room: its coordinate records and, for grid-built rooms,
/// the index and record grids.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogicInfo {
    /// [`INFO_ONE`] or [`INFO_GRID`].
    pub flags: u32,
    /// Number of records allocated as one array.
    pub array_len: u32,
    /// Index grid, (W+1) × (H+1), row-major; flag 2 only.
    pub index_grid: Vec<u32>,
    /// Record grid: per cell, the position in [`LogicInfo::list`] of its record; flag 2
    /// only.
    pub record_grid: Vec<usize>,
    /// The records in list order, head first.
    pub list: Vec<CoordRec>,
}

impl Drlg {
    /// Builds the one-record form: one record for the whole room, and the level's
    /// coordinate counter is set to 1.
    pub(super) fn build_logic_one(&mut self, id: DrlgRoomId) {
        let r = self.room(id).rect;
        let l = self.room(id).level;
        // Always 1, overwriting any earlier value of the counter.
        self.level_mut(l).coord_counter = 1;
        let b = [r.x, r.y, r.x + r.w, r.y + r.h];
        self.room_mut(id).logic = Some(LogicInfo {
            flags: INFO_ONE,
            array_len: 1,
            index_grid: Vec::new(),
            record_grid: Vec::new(),
            list: vec![CoordRec {
                boxr: b,
                clipped: b,
                node: false,
                index: 1,
            }],
        });
    }

    /// Builds the grid form: flood-fills the room into regions, cuts the regions into
    /// rectangles, then merges region indexes with the neighbouring rooms.
    pub(super) fn build_logic_grid(&mut self, id: DrlgRoomId, g: &LogicGrids) {
        let rect = self.room(id).rect;
        let l = self.room(id).level;
        let (cw, ch) = ((rect.w + 1) as usize, (rect.h + 1) as usize);
        // A zeroed index grid.
        let mut index = vec![0u32; cw * ch];
        // Marking tree wall records in the wall layer 0 grid is not done: wall records
        // always carry layer bits, and nothing later reads that grid.
        // The level's counter starts at 1 if unset.
        if self.level(l).coord_counter == 0 {
            self.level_mut(l).coord_counter = 1;
        }
        let start = self.level(l).coord_counter;
        // Cells that block the flood fill.
        let blockers = self.blocker_grid(id, cw, ch);
        // Regions: each unvisited cell starts a new region index.
        let mut counter = start;
        for y in 0..ch {
            for x in 0..cw {
                if index[y * cw + x] & VISITED != 0 {
                    continue;
                }
                counter = counter.wrapping_add(1);
                let mut m = (counter & INDEX_MASK) | VISITED;
                let v = g.floor.get(x as i32, y as i32);
                if v & 0x01E0_FF00 == 0x01E0_0000 || v & 0x8000_0000 != 0 {
                    m |= NODE;
                }
                fill(
                    &mut index,
                    &blockers,
                    &g.orientation,
                    rect,
                    (x as i32, y as i32, -1),
                    m,
                );
            }
        }
        // Advance the level's counter past the indexes used.
        let array_len = counter.wrapping_sub(start).wrapping_add(1);
        self.level_mut(l).coord_counter = counter.wrapping_add(array_len);
        // Rectangles, in scan order (prepended below).
        let mut cell_rec: Vec<Option<usize>> = vec![None; cw * ch];
        let mut made: Vec<CoordRec> = Vec::new();
        for y in 0..ch {
            for x in 0..cw {
                if cell_rec[y * cw + x].is_some() {
                    continue;
                }
                let v = index[y * cw + x];
                let free = |cx: usize, cy: usize, cr: &[Option<usize>]| {
                    index[cy * cw + cx] == v && cr[cy * cw + cx].is_none()
                };
                let mut x1 = x;
                while x1 < cw && free(x1, y, &cell_rec) {
                    x1 += 1;
                }
                let mut y1 = y + 1;
                while y1 < ch && (x..x1).all(|cx| free(cx, y1, &cell_rec)) {
                    y1 += 1;
                }
                let k = made.len();
                for cy in y..y1 {
                    for cx in x..x1 {
                        cell_rec[cy * cw + cx] = Some(k);
                    }
                }
                let boxr = [
                    x as i32 + rect.x,
                    y as i32 + rect.y,
                    x1 as i32 + rect.x,
                    y1 as i32 + rect.y,
                ];
                let (ex, ey) = (rect.x + rect.w, rect.y + rect.h);
                let clipped = if boxr[0] >= ex || boxr[1] >= ey {
                    [0; 4]
                } else {
                    [boxr[0], boxr[1], boxr[2].min(ex), boxr[3].min(ey)]
                };
                made.push(CoordRec {
                    boxr,
                    clipped,
                    node: v & NODE != 0,
                    index: v & INDEX_MASK,
                });
            }
        }
        // List: the rectangles prepended (reverse scan order), then one zeroed record
        // from the end of the allocated array; the rest of the array is never linked.
        let n = made.len();
        let mut list: Vec<CoordRec> = made.into_iter().rev().collect();
        list.push(CoordRec::default());
        let record_grid = cell_rec
            .into_iter()
            .map(|k| n - 1 - k.expect("every cell gets a record"))
            .collect();
        // Wall records carry no pointer to their coordinate record; `wall_coord` looks it
        // up on demand.
        self.room_mut(id).logic = Some(LogicInfo {
            flags: INFO_GRID,
            array_len,
            index_grid: index,
            record_grid,
            list,
        });
        // Merge region indexes with the neighbouring rooms.
        self.merge_logic(id);
    }

    /// The blocker grid, (W+1) × (H+1): cells holding a blocking wall record of this room
    /// or of a near room.
    fn blocker_grid(&self, id: DrlgRoomId, cw: usize, ch: usize) -> Vec<bool> {
        let room = self.room(id);
        let rect = room.rect;
        let mut b = vec![false; cw * ch];
        let blocks = |r: &TileRecord| {
            r.flags & LAYER_BITS == 0x4000 && r.kind != ROOF && r.flags & OBJECT_WALL == 0
        };
        if let Some(t) = &room.tiles {
            for r in t.walls.iter().filter(|r| blocks(r)) {
                // A room's own records lie in 0..=W × 0..=H, so this test never skips one.
                debug_assert!((0..cw as i32).contains(&r.x) && (0..ch as i32).contains(&r.y));
                if (0..cw as i32).contains(&r.x) && (0..ch as i32).contains(&r.y) {
                    b[r.y as usize * cw + r.x as usize] = true;
                }
            }
        }
        for &n in room.near.as_deref().unwrap_or(&[]) {
            if n == id {
                continue;
            }
            let nr = self.room(n);
            let Some(t) = &nr.tiles else { continue };
            for &(kind, i) in &t.other_links {
                let r = &t.records(kind)[i];
                let (px, py) = (nr.rect.x + r.x, nr.rect.y + r.y);
                if blocks(r) && rect.contains_closed(px, py) {
                    b[(py - rect.y) as usize * cw + (px - rect.x) as usize] = true;
                }
            }
        }
        b
    }

    /// Merges region indexes with the rooms near that touch this one, along the borders.
    fn merge_logic(&mut self, id: DrlgRoomId) {
        let rect = self.room(id).rect;
        let near: Vec<DrlgRoomId> = self.room(id).near.clone().unwrap_or_default();
        for n in near {
            if n == id || self.room(n).logic.is_none() {
                continue;
            }
            let (gx, gy) = near_gaps(&rect, &self.room(n).rect);
            if !(gx < 1 && gy < 1) {
                continue;
            }
            for x in rect.x..=rect.x + rect.w {
                self.merge_cell(id, n, x, rect.y);
                self.merge_cell(id, n, x, rect.y + rect.h);
            }
            for y in rect.y..=rect.y + rect.h {
                self.merge_cell(id, n, rect.x, y);
                self.merge_cell(id, n, rect.x + rect.w, y);
            }
        }
    }

    /// Merge step for one border cell (in tiles): renames this room's region index to the
    /// neighbour's when both are non-zero, in the same level, differ and are both node or
    /// both non-node.
    fn merge_cell(&mut self, id: DrlgRoomId, n: DrlgRoomId, x: i32, y: i32) {
        if !self.room(n).rect.contains_closed(x, y) {
            return;
        }
        let (Some(a), Some(b)) = (self.coord_at_tile(n, x, y), self.coord_at_tile(id, x, y)) else {
            return;
        };
        let same_level = self.level(self.room(n).level).id == self.level(self.room(id).level).id;
        if a.index != 0 && b.index != 0 && same_level && a.index != b.index && a.node == b.node {
            self.rename_logic(id, b.index, a.index);
        }
    }

    /// Renames region index `old` to `new` in a grid-built room and, if any record
    /// changed, in its rooms near of the same level id.
    pub(super) fn rename_logic(&mut self, id: DrlgRoomId, old: u32, new: u32) {
        let Some(info) = self.room_mut(id).logic.as_mut() else {
            return;
        };
        if info.flags != INFO_GRID {
            return;
        }
        let mut any = false;
        for r in info.list.iter_mut().filter(|r| r.index == old) {
            r.index = new;
            any = true;
        }
        if !any {
            return;
        }
        let lid = self.level(self.room(id).level).id;
        let near: Vec<DrlgRoomId> = self.room(id).near.clone().unwrap_or_default();
        for n in near {
            if n != id && self.level(self.room(n).level).id == lid {
                self.rename_logic(n, old, new);
            }
        }
    }

    /// The record at a tile point (`coord_at` without the sub-tile scaling).
    fn coord_at_tile(&self, id: DrlgRoomId, x: i32, y: i32) -> Option<CoordRec> {
        let room = self.room(id);
        let info = room.logic.as_ref()?;
        if info.flags == INFO_ONE {
            return info.list.first().copied();
        }
        let (cx, cy) = (x - room.rect.x, y - room.rect.y);
        let cw = room.rect.w + 1;
        // The record grid is one block of (W+1)(H+1) cells, so a cx outside 0..W wraps
        // into the row before or after. An index outside the block has no meaningful
        // record, so it returns None.
        if !(0..=room.rect.h).contains(&cy) {
            return None;
        }
        let k = cy * cw + cx;
        if !(0..cw * (room.rect.h + 1)).contains(&k) {
            return None;
        }
        Some(info.list[info.record_grid[k as usize]])
    }

    /// A wall record's coordinate record for the record at room-relative tile (x, y): the
    /// record-grid entry at its cell in a grid-built room; `None` in a one-record room
    /// or a room without info. Read when needed, so an index renamed by a later merge is
    /// the current one. Read by drawing code only.
    pub fn wall_coord(&self, id: DrlgRoomId, x: i32, y: i32) -> Option<CoordRec> {
        let room = self.room(id);
        if room.logic.as_ref()?.flags == INFO_ONE {
            return None;
        }
        self.coord_at_tile(id, room.rect.x + x, room.rect.y + y)
    }

    /// The record list of the room's info, head first; `None` without info.
    pub fn coord_first(&self, id: DrlgRoomId) -> Option<&[CoordRec]> {
        self.room(id).logic.as_ref().map(|l| l.list.as_slice())
    }

    /// The record at a sub-tile point: a one-record room gives its record; otherwise the
    /// record grid at (x/5 − X, y/5 − Y) (truncating division), a column outside 0..W
    /// wrapping into the next or previous row. `None` without info or outside the grid.
    pub fn coord_at(&self, id: DrlgRoomId, x: i32, y: i32) -> Option<CoordRec> {
        self.coord_at_tile(id, x / SUBTILES, y / SUBTILES)
    }
}

/// T1: rule row by wall orientation.
const T1: [i32; 20] = [-1, 0, 1, 2, 2, 0, 1, 3, 0, 1, 0, 1, 4, -1, 4, 0, 0, 0, 0, 0];
/// T2: fill rules (26 entries). Bit 1 takes the cell; bits 2, 4, 8 and 16 spread to the
/// +x, +y, −x and −y neighbours; bit 32 spreads to the (+1, +1) diagonal.
const T2: [u32; 26] = [
    23, 0, 5, 21, 17, 15, 3, 0, 9, 7, 39, 0, 0, 5, 3, 31, 31, 31, 31, 31, 31, 31, 31, 31, 31, 0,
];
/// The five rule entries that precede T2, used when T1 = −1: indexes −5..−1 (d = −1..3).
const BEFORE_T2: [u32; 5] = [0xFFFF_FFFF, 0, 0, 0xFFFF_FFFF, 0];
/// Direction offsets: +x, +y, −x, −y.
const DIRS: [(i32, i32); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];
/// Record layer bits; roof type; object-wall flag. Used to pick blocking wall records.
const LAYER_BITS: u32 = 0x1_C000;
const ROOF: u32 = 15;
const OBJECT_WALL: u32 = 0x800;

/// The fill rule for wall orientation `o` entered in direction `d`.
fn rule(o: u32, d: i32) -> u32 {
    // Orientations above 19 are outside T1; no wall layer holds one. They read as rule 0.
    let Some(&t1) = T1.get(o as usize) else {
        return 0;
    };
    let i = d + 5 * t1 + 1;
    if i < 0 {
        BEFORE_T2[(i + 5) as usize]
    } else {
        T2[i as usize]
    }
}

/// Flood fill of one region from `at`. The recursion is run on an explicit stack in call
/// order: a call's neighbour fills run before its diagonal continuation (rule bit 32),
/// which is the call's last action.
fn fill(
    index: &mut [u32],
    blockers: &[bool],
    orient: &CellGrid,
    rect: TileRect,
    at: (i32, i32, i32),
    m: u32,
) {
    let cw = (rect.w + 1) as usize;
    let mut stack = vec![at];
    while let Some((x, y, d)) = stack.pop() {
        // Stop outside the room or on an already visited cell.
        if !rect.contains_closed(rect.x + x, rect.y + y) {
            continue;
        }
        let c = y as usize * cw + x as usize;
        if index[c] & VISITED != 0 {
            continue;
        }
        // Open cell: take it and spread to all four neighbours.
        if !blockers[c] {
            index[c] |= m;
            for k in (0..4).rev() {
                let (dx, dy) = DIRS[k];
                stack.push((x + dx, y + dy, k as i32));
            }
            continue;
        }
        // Blocked cell: the wall orientation's rule decides what to take and where to spread.
        let r = rule(orient.get(x, y), d);
        if r & 1 != 0 {
            index[c] |= m;
        }
        if r & 32 != 0 {
            stack.push((x + 1, y + 1, -1));
        }
        let calls = [(2, 2), (4, 3), (8, 0), (16, 1)];
        for k in (0..4).rev() {
            let (bit, not_d) = calls[k];
            if r & bit != 0 && d != not_d {
                let (dx, dy) = DIRS[k];
                stack.push((x + dx, y + dy, k as i32));
            }
        }
    }
}

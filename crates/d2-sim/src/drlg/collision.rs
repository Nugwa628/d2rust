//! Collision grids of active rooms: one `u16` mask per sub-tile, built from the tile
//! records of the room and its adjacency list, and updated when a linked tile changes.
//!
//! The grid is the walkability and blocking map that movement and placement read; units
//! set and clear their own run-time bits in it.

use super::level::Drlg;
use super::tiles::{rec_flags, RecordKind, TileRecord, TileRef};
use super::{DrlgRoomId, TileRect, SUBTILES};

/// Collision bits (D2MOO names).
pub mod bits {
    /// Blocks walking.
    pub const WALL: u16 = 0x0001;
    /// Bit 0x0002 (D2MOO name `VISIBLE`).
    pub const VISIBLE: u16 = 0x0002;
    /// Blocks missiles.
    pub const MISSILE_BARRIER: u16 = 0x0004;
    /// Bit 0x0008 (D2MOO name `NOPLAYER`).
    pub const NOPLAYER: u16 = 0x0008;
    /// Set over a block by door or exit tile records.
    pub const PRESET: u16 = 0x0010;
    /// Bit 0x0020 (D2MOO name `BLANK`).
    pub const BLANK: u16 = 0x0020;
    /// A missile occupies the sub-tile.
    pub const MISSILE: u16 = 0x0040;
    /// A player occupies the sub-tile.
    pub const PLAYER: u16 = 0x0080;
    /// A monster occupies the sub-tile.
    pub const MONSTER: u16 = 0x0100;
    /// An item lies on the sub-tile.
    pub const ITEM: u16 = 0x0200;
    /// An object occupies the sub-tile.
    pub const OBJECT: u16 = 0x0400;
    /// A door occupies the sub-tile.
    pub const DOOR: u16 = 0x0800;
    /// Pathing may not enter.
    pub const NO_PATH: u16 = 0x1000;
    /// A pet occupies the sub-tile.
    pub const PET: u16 = 0x2000;
    /// Bit 0x4000; meaning not named.
    pub const BIT_4000: u16 = 0x4000;
    /// A corpse occupies the sub-tile.
    pub const CORPSE: u16 = 0x8000;
}

/// A room's collision grid: sub-tile rect and masks, row stride = width.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CollisionGrid {
    /// The sub-tile rect the grid covers.
    pub rect: TileRect,
    /// One mask per sub-tile, row by row.
    pub masks: Vec<u16>,
}

impl CollisionGrid {
    /// A zeroed grid over a sub-tile rect.
    pub fn new(rect: TileRect) -> Self {
        let n = (rect.w.max(0) * rect.h.max(0)) as usize;
        Self {
            rect,
            masks: vec![0; n],
        }
    }

    fn index(&self, sx: i32, sy: i32) -> Option<usize> {
        self.rect
            .contains(sx, sy)
            .then(|| ((sy - self.rect.y) * self.rect.w + (sx - self.rect.x)) as usize)
    }

    /// The mask at sub-tile (sx, sy), if inside.
    pub fn get(&self, sx: i32, sy: i32) -> Option<u16> {
        self.index(sx, sy).map(|i| self.masks[i])
    }

    /// Mutable mask at sub-tile (sx, sy), for units' run-time bits.
    pub fn get_mut(&mut self, sx: i32, sy: i32) -> Option<&mut u16> {
        self.index(sx, sy).map(|i| &mut self.masks[i])
    }

    /// ORs (`set`) or clears DT1 sub-tile bytes over the 5×5 block at (ox, oy). DT1 rows
    /// are stored bottom row first.
    fn apply_dt1(&mut self, ox: i32, oy: i32, flags: &[u8; 25], set: bool) {
        for r in 0..5 {
            for c in 0..5 {
                let b = u16::from(flags[(5 * (4 - r) + c) as usize]);
                if let Some(m) = self.get_mut(ox + c, oy + r) {
                    if set {
                        *m |= b;
                    } else {
                        *m &= !b;
                    }
                }
            }
        }
    }

    /// Turns a record's flags into collision bits over the 5×5 block at (ox, oy).
    fn apply_record_flags(&mut self, ox: i32, oy: i32, flags: u32) {
        let mut b = 0;
        if flags & rec_flags::DOOR_OR_EXIT != 0 {
            b |= bits::PRESET;
        }
        if flags & rec_flags::UNWALKABLE != 0 {
            b |= bits::WALL;
        }
        if flags & rec_flags::FILL_LOS != 0 {
            b |= bits::MISSILE_BARRIER;
        }
        if b == 0 {
            return;
        }
        for r in 0..5 {
            for c in 0..5 {
                if let Some(m) = self.get_mut(ox + c, oy + r) {
                    *m |= b;
                }
            }
        }
    }
}

impl Drlg {
    /// Builds the collision grid of room `id`'s active room from the records of every
    /// room in its adjacency list whose origin lies in this room.
    pub(super) fn build_collision(&mut self, id: DrlgRoomId) {
        let Some(active) = self.room(id).active.as_ref() else {
            return;
        };
        let mut grid = CollisionGrid::new(active.subtiles);
        for &r in &active.adjacency {
            let room = self.room(r);
            let Some(tiles) = room.tiles.as_ref() else {
                continue;
            };
            for kind in [RecordKind::Floor, RecordKind::Wall, RecordKind::Shadow] {
                for rec in tiles.records(kind) {
                    let (ox, oy) = record_origin(room.rect, rec);
                    if !grid.rect.contains(ox, oy) {
                        continue;
                    }
                    grid.apply_dt1(ox, oy, &self.tile_info(rec.tile).subtile_flags, true);
                    grid.apply_record_flags(ox, oy, rec.flags);
                }
            }
        }
        self.room_mut(id)
            .active
            .as_mut()
            .expect("checked")
            .collision = grid;
    }

    /// Updates the grid after record `i` of room `n` changes tile: in the first active
    /// room among `n` and its adjacency list that contains the record's origin, clears
    /// the old tile's bytes and ORs in the new tile's. Record-flag bits are untouched.
    pub(super) fn collision_update(
        &mut self,
        n: DrlgRoomId,
        kind: RecordKind,
        i: usize,
        old: Option<TileRef>,
        new: Option<TileRef>,
    ) {
        let Some(active) = self.room(n).active.as_ref() else {
            return;
        };
        let rec = self
            .room(n)
            .tiles
            .as_ref()
            .expect("record exists")
            .records(kind)[i];
        let (ox, oy) = record_origin(self.room(n).rect, &rec);
        let mut cands = vec![n];
        cands.extend(active.adjacency.iter().copied().filter(|&x| x != n));
        let Some(target) = cands.into_iter().find(|&c| {
            self.room(c)
                .active
                .as_ref()
                .is_some_and(|a| a.collision.rect.contains(ox, oy))
        }) else {
            return;
        };
        let old_flags = old.map(|t| self.tile_info(t).subtile_flags);
        let new_flags = new.map(|t| self.tile_info(t).subtile_flags);
        let grid = &mut self
            .room_mut(target)
            .active
            .as_mut()
            .expect("found")
            .collision;
        if let Some(f) = old_flags {
            grid.apply_dt1(ox, oy, &f, false);
        }
        if let Some(f) = new_flags {
            grid.apply_dt1(ox, oy, &f, true);
        }
    }

    /// The collision mask at sub-tile (sx, sy) in any active room of this
    /// DRLG (each sub-tile belongs to one room's grid).
    pub fn collision_at(&self, sx: i32, sy: i32) -> Option<u16> {
        self.active_index.values().find_map(|&r| {
            self.room(r)
                .active
                .as_ref()
                .and_then(|a| a.collision.get(sx, sy))
        })
    }

    /// Mutable collision mask at sub-tile (sx, sy), for units' run-time bits (set by
    /// movement and unit code).
    pub fn collision_at_mut(&mut self, sx: i32, sy: i32) -> Option<&mut u16> {
        let r = self.active_index.values().copied().find(|&r| {
            self.room(r)
                .active
                .as_ref()
                .is_some_and(|a| a.collision.rect.contains(sx, sy))
        })?;
        self.room_mut(r).active.as_mut()?.collision.get_mut(sx, sy)
    }
}

/// Sub-tile origin of a record: 5 · (room tile origin + record position).
fn record_origin(room: TileRect, rec: &TileRecord) -> (i32, i32) {
    ((room.x + rec.x) * SUBTILES, (room.y + rec.y) * SUBTILES)
}

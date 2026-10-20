//! Monster population: monster regions, room population with
//! density tries, picks and boss-or-pack, random bosses,
//! packs, spawn points and the placement search ([`placement`]), parties,
//! preset monsters (`monsters/preset-monsters.tsv`) and ambient
//! spawns.
//!
//! Monster creation itself and every room, collision
//! and unit query go through the seams in [`seams`]. All randomness uses
//! [`crate::rng::Seed`] on the seed each rule names: game seed, active
//! room seed, unit seed or the monster-region seed.

pub mod data;
pub mod placement;
pub mod preset;
pub mod region;
pub mod room;
pub mod seams;
pub mod spawn;

#[cfg(test)]
mod gap_tests;
#[cfg(test)]
mod tests;

pub use data::{LevelPop, Mon2Pop, MonPop, PopTables, SuperPop};
pub use region::{Region, RegionEntry, Regions};
pub use seams::{Alloc, MonsterInit, OwnerKey, PopHost, PopWorld};

use crate::units::RoomId;

/// A DRLG coordinate rectangle (D2MOO `D2RoomCoordListStrc`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CoordRect {
    /// Left, top, right, bottom in tiles (+0x10).
    pub rect: [i32; 4],
    /// Node flag (+0x20).
    pub node_flag: i32,
    /// Index (+0x28).
    pub index: i32,
}

impl CoordRect {
    /// The rect in subtiles (each coordinate × 5).
    pub fn subtiles(&self) -> [i32; 4] {
        self.rect.map(|v| v.wrapping_mul(5))
    }
}

/// A room's subtile box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RoomBox {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// One room tile record (0x30 bytes) as the placement search reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TileRec {
    /// The record has tile data (+0x18) whose flags have bit 2.
    pub water: bool,
    /// rec+8 + room tile x, rec+0xC + room tile y (tiles).
    pub x: i32,
    pub y: i32,
}

/// One DS1 preset unit as the preset pass reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PresetUnit {
    /// Unit type (1 = monster).
    pub unit_type: i32,
    /// Mode (+0x00).
    pub mode: u8,
    /// Class (+0x04).
    pub class: i32,
    /// x (+0x08), y (+0x18), room-relative subtiles.
    pub x: i32,
    pub y: i32,
    /// Has data at +0x10.
    pub has_data: bool,
    /// "Done" bit (+0x1C bit 0).
    pub done: bool,
}

/// Game fields population reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GameInfo {
    /// Difficulty (game +0x6D): 0 Normal, 1 Nightmare, 2 Hell.
    pub difficulty: u8,
    /// Expansion (game +0x70).
    pub expansion: bool,
}

/// Population state of a game: the region array (game +0xF0), the
/// monster seed (game +0xEC) and the superunique flags (game +0x1D30).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PopState {
    pub regions: Regions,
    /// `dwMonSeed` (game +0xEC).
    pub mon_seed: u32,
    /// Bit su = superunique su placed (byte su >> 3, bit su & 7).
    pub superunique_flags: Vec<u8>,
}

impl PopState {
    pub fn superunique_placed(&self, su: i32) -> bool {
        let su = su as u32 as usize;
        self.superunique_flags
            .get(su >> 3)
            .is_some_and(|b| b & (1 << (su & 7)) != 0)
    }

    pub fn set_superunique_placed(&mut self, su: i32) {
        let su = su as u32 as usize;
        if self.superunique_flags.len() <= su >> 3 {
            self.superunique_flags.resize((su >> 3) + 1, 0);
        }
        self.superunique_flags[su >> 3] |= 1 << (su & 7);
    }
}

/// What population code works with.
pub struct Ctx<'a, H: ?Sized> {
    pub tables: &'a PopTables,
    pub info: GameInfo,
    pub state: &'a mut PopState,
    pub host: &'a mut H,
}

/// One active room in the tick room pass. Ambient spawns every
/// tick; on the first population (room +0x34 bit 0 clear, `first`; the
/// tick owns the bit): presets, inactive unit restore, objects, then room
/// monster population.
pub fn room_step<H: PopHost + ?Sized>(cx: &mut Ctx<'_, H>, room: RoomId, first: bool) {
    room::ambient(cx, room);
    if first {
        // Runs the same sequence for one room.
        populate_once(cx, room);
    }
}

/// Minus the ambient call: presets, restore, objects,
/// population. The off-tick entry itself (ambient first,
/// then this) is `crate::tick::populate_room`; its
/// callers are the portal and A2Q6 arrival paths, which run it
/// inside the caller's step.
pub fn populate_once<H: PopHost + ?Sized>(cx: &mut Ctx<'_, H>, room: RoomId) {
    preset::place_presets(cx, room);
    cx.host.restore_inactive_units(room);
    cx.host.populate_objects(room);
    room::populate_room(cx, room);
}

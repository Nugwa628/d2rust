//! The light map: 48 × 48 cells around the local player, rebuilt every
//! drawn frame.
//!
//! A frame's map is built in a fixed order (see [`LightMap::build`]):
//! 1. the window is centred on the player's sub-tile,
//! 2. every cell is filled with the ambient light of the room it lies in,
//! 3. cells whose sub-tile blocks light are flagged (only when the player
//!    stands in a room),
//! 4. every light record, newest first, adds its light.
//!
//! The map also has a canonical byte form (8 bytes per cell) whose SHA-256
//! is the light part of a frame's state key: two builds from the same
//! inputs give the same digest.

use sha2::{Digest, Sha256};

use super::records::{LightError, LightList, LightWorld};

/// Cells per axis of the light map.
pub const MAP_SIZE: i32 = 48;
/// The window half-width: origin = player sub-tile − 24.
pub const HALF: i32 = 24;
/// Bytes of the map: 8 per cell.
pub const MAP_BYTES: usize = (MAP_SIZE * MAP_SIZE) as usize * 8;

/// One light-map cell: `+0` u32 blocks-light flag, `+4` intensity
/// `I`, `+5` R, `+6` G, `+7` B.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct LightCell {
    pub blocks: u32,
    pub i: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl LightCell {
    /// The light word a draw receives: `B << 24 | G << 16 | R << 8 | I`.
    /// The intensity sits in the low byte, which is all a palette-lit draw
    /// uses.
    pub fn word(&self) -> u32 {
        u32::from(self.b) << 24
            | u32::from(self.g) << 16
            | u32::from(self.r) << 8
            | u32::from(self.i)
    }
}

/// The light map. Cell `(gx, gy)` covers sub-tile `(ox + gx, oy + gy)`;
/// cells are stored row-major (row = y).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LightMap {
    /// `(ox, oy)` = the local player's sub-tile − 24.
    pub origin: (i32, i32),
    cells: Vec<LightCell>,
}

impl LightMap {
    /// An all-dark map centred on a player standing at sub-tile `player`.
    pub fn new(player: (i32, i32)) -> Self {
        LightMap {
            origin: (player.0 - HALF, player.1 - HALF),
            cells: vec![LightCell::default(); (MAP_SIZE * MAP_SIZE) as usize],
        }
    }

    /// The inclusive upper bound a light source must not pass to be
    /// considered: origin + 48, that is the player's sub-tile + 24.
    pub fn upper(&self) -> (i32, i32) {
        (self.origin.0 + MAP_SIZE, self.origin.1 + MAP_SIZE)
    }

    fn index(gx: i32, gy: i32) -> Option<usize> {
        ((0..MAP_SIZE).contains(&gx) && (0..MAP_SIZE).contains(&gy))
            .then(|| (gy * MAP_SIZE + gx) as usize)
    }

    /// Cell `(gx, gy)` in map coordinates, `None` outside 0…47.
    pub fn cell(&self, gx: i32, gy: i32) -> Option<&LightCell> {
        Self::index(gx, gy).map(|i| &self.cells[i])
    }

    /// Mutable cell `(gx, gy)`; there is no cell outside 0…47, so light
    /// that falls off the map is simply dropped.
    pub fn cell_mut(&mut self, gx: i32, gy: i32) -> Option<&mut LightCell> {
        Self::index(gx, gy).map(|i| &mut self.cells[i])
    }

    /// Every cell, row-major.
    pub fn cells(&self) -> &[LightCell] {
        &self.cells
    }

    /// Every cell, row-major, mutable.
    pub fn cells_mut(&mut self) -> &mut [LightCell] {
        &mut self.cells
    }

    /// The cell a draw reads at `(x, y)` in eighths of a sub-tile. Draws
    /// never miss: a point outside the window reads the nearest edge cell,
    /// so something far from the player is lit like the map's border.
    pub fn read(&self, x: i32, y: i32) -> LightCell {
        let gx = ((x >> 3) - self.origin.0).clamp(0, MAP_SIZE - 1);
        let gy = ((y >> 3) - self.origin.1).clamp(0, MAP_SIZE - 1);
        self.cells[(gy * MAP_SIZE + gx) as usize]
    }

    /// The canonical byte form of the map: per cell, row-major, the
    /// blocks-light flag as a little-endian `u32`, then `I`, `R`, `G`, `B`.
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(MAP_BYTES);
        for c in &self.cells {
            out.extend_from_slice(&c.blocks.to_le_bytes());
            out.extend_from_slice(&[c.i, c.r, c.g, c.b]);
        }
        out
    }

    /// SHA-256 of [`Self::bytes`]: the light-map part of a frame's state
    /// key.
    pub fn digest(&self) -> [u8; 32] {
        Sha256::digest(self.bytes()).into()
    }

    /// The ambient fill, the first light of a frame.
    ///
    /// With no local player or no player room (`scene` is `None`) the map
    /// is all dark. Otherwise every cell takes the player room's ambient
    /// (and a clear blocks-light flag); then each nearby room except the
    /// player's own paints its ambient over its rectangle, in list order so
    /// later rooms win. A room's rectangle is painted one column and one
    /// row wider than the room, which lets neighbouring rooms overlap by
    /// one sub-tile.
    pub fn fill_ambient(&mut self, scene: Option<&AmbientScene>) {
        let Some(scene) = scene else {
            self.cells.fill(LightCell::default());
            return;
        };
        self.cells.fill(scene.player_ambient.cell());
        let (ox, oy) = self.origin;
        for room in scene.near.iter().filter(|r| !r.is_player_room) {
            let (x, y, w, h) = room.rect;
            let (xp, yp) = (x - ox, y - oy);
            if xp > MAP_SIZE || xp + w < 0 || yp > MAP_SIZE || yp + h < 0 {
                continue;
            }
            let fill = room.ambient.cell();
            for gy in yp.max(0)..=(yp + h).min(MAP_SIZE - 1) {
                for gx in xp.max(0)..=(xp + w).min(MAP_SIZE - 1) {
                    if let Some(c) = self.cell_mut(gx, gy) {
                        *c = fill;
                    }
                }
            }
        }
    }

    /// Flags the cells whose sub-tile stops light. `blocks(x, y)` is the
    /// world's collision test for light-blocking walls at sub-tile
    /// `(x, y)`; it is asked once per cell, row by row, and a cell it
    /// answers `false` keeps its flag. The build only runs this when the
    /// player stands in a room.
    pub fn fill_blocks(&mut self, mut blocks: impl FnMut(i32, i32) -> bool) {
        let (ox, oy) = self.origin;
        for gy in 0..MAP_SIZE {
            for gx in 0..MAP_SIZE {
                if blocks(ox + gx, oy + gy) {
                    self.cells[(gy * MAP_SIZE + gx) as usize].blocks = 1;
                }
            }
        }
    }

    /// Builds one drawn frame's map: the window around `player`, the
    /// ambient fill, the blocks-light flags when `scene` exists, then every
    /// record of `lights` from newest to oldest at light quality `q` (the
    /// caller picks `q`, see [`super::quality`]). Records are updated as
    /// they contribute (they follow their owners and grow or shrink), so
    /// the list is mutable.
    pub fn build(
        player: (i32, i32),
        scene: Option<&AmbientScene>,
        blocks: impl FnMut(i32, i32) -> bool,
        q: u8,
        lights: &mut LightList,
        world: &impl LightWorld,
    ) -> Result<LightMap, LightError> {
        let mut map = LightMap::new(player);
        map.fill_ambient(scene);
        if scene.is_some() {
            map.fill_blocks(blocks);
        }
        lights.frame(&mut map, q, world)?;
        Ok(map)
    }
}

/// A room's ambient light `(I, R, G, B)` as the fill uses it: what the
/// room is lit with before any light source adds to it (chosen by the
/// caller, see [`super::environment::room_ambient`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Ambient {
    pub i: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Ambient {
    fn cell(self) -> LightCell {
        LightCell {
            blocks: 0,
            i: self.i,
            r: self.r,
            g: self.g,
            b: self.b,
        }
    }
}

/// One room of the player room's list of nearby rooms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NearRoom {
    /// The room's rectangle `(x, y, w, h)` in sub-tiles.
    pub rect: (i32, i32, i32, i32),
    pub ambient: Ambient,
    /// This entry is the player's own room; it is skipped, the whole map
    /// already starts from its ambient.
    pub is_player_room: bool,
}

/// The ambient input of a frame when the local player and its room exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmbientScene {
    /// The ambient of the player's room.
    pub player_ambient: Ambient,
    /// The player room's nearby rooms, in list order.
    pub near: Vec<NearRoom>,
}

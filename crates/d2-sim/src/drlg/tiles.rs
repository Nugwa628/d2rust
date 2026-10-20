//! Room tile grids: the per-room tile library, the rarity-weighted tile choice, grid fill
//! and cell rules, tiles shared between neighbouring rooms, animated tiles and freeing.
//!
//! A room's packed source grids (from the level-type generator) are turned into floor,
//! wall and shadow records, each pointing at a DT1 tile. Records at room borders are
//! shared with the rooms near, so a cell linked to a neighbour's record merges with it
//! instead of adding a second one. The collision grid reads these records.

use std::collections::BTreeMap;

use crate::rng::Seed;

use super::data::{DrlgData, WallClass};
use super::level::Drlg;
use super::logic::LogicGrids;
use super::room::RoomKind;
use super::seams::{Services, TileInfo, TileSource};
use super::{room_flags, DrlgError, DrlgRoomId, TileRect};

/// The three fixed library files loaded after the files named by the room's DT1 mask.
pub const FIXED_LIBRARY: [&[u8]; 3] = [
    b"DATA\\GLOBAL\\Tiles\\Act1\\Outdoors\\Blank.dt1",
    b"DATA\\GLOBAL\\Tiles\\Act1\\Barracks\\InvisWal.dt1",
    b"DATA\\GLOBAL\\Tiles\\Act1\\Barracks\\Warp.dt1",
];

/// A base library path and a tile key (orientation, main, sub).
pub type EdgeTileKey = (&'static [u8], (u32, u32, u32));

/// The act's base tile library and the key (orientation, main, sub) of the act's edge
/// floor record looked up in it, by act number. Acts IV and V load no base library: their
/// record stays zero.
pub const ACT_EDGE_TILE: [Option<EdgeTileKey>; 5] = [
    Some((b"DATA\\GLOBAL\\Tiles\\Act1\\Town\\Floor.dt1", (0, 0, 0))),
    Some((b"DATA\\GLOBAL\\Tiles\\Act2\\Town\\Ground.dt1", (0, 0, 1))),
    Some((b"DATA\\GLOBAL\\Tiles\\ACT3\\Kurast\\sets.dt1", (0, 29, 12))),
    None,
    None,
];

/// The first entry of the tile lookup of `key` in one DT1 file's tiles (lookups run in
/// reverse file order), as its index in file order.
pub fn first_entry(tiles: &[TileInfo], key: (u32, u32, u32)) -> Option<u32> {
    tiles
        .iter()
        .rposition(|t| (t.orientation, t.main, t.sub) == key)
        .map(|i| i as u32)
}

/// Library slots per room.
pub const LIBRARY_SLOTS: usize = 32;

/// Lookup cap: a tile lookup returns at most 40 entries.
pub const LOOKUP_CAP: usize = 40;

/// Default animation speed when lvlprest AnimSpeed is 0.
pub const DEFAULT_ANIM_SPEED: u32 = 80;

/// Packed cell bits and fields of a source-grid cell.
pub mod cell {
    /// The cell has a wall tile.
    pub const WALL: u32 = 0x1;
    /// The cell has a floor tile.
    pub const FLOOR: u32 = 0x2;
    /// The cell may share a record with a neighbouring room.
    pub const LINKED: u32 = 0x4;
    /// Enclosed cell; sets record flag 0x4.
    pub const ENCLOSED: u32 = 0x8;
    /// The cell belongs to a layer above the base layer.
    pub const LAYER_ABOVE: u32 = 0x80;
    /// Fills line of sight; sets the record's fill-LOS flag.
    pub const FILL_LOS: u32 = 1 << 16;
    /// Not walkable; sets the record's unwalkable flag.
    pub const UNWALKABLE: u32 = 1 << 17;
    /// Sets record flags 0x20C (reveals a hidden cell).
    pub const REVEAL_HIDDEN: u32 = 1 << 26;
    /// The cell has a shadow tile.
    pub const SHADOW: u32 = 1 << 27;
    /// Sets record flags 0x102.
    pub const LINKAGE: u32 = 1 << 28;
    /// Object wall; sets record flag 0x800.
    pub const OBJECT_WALL: u32 = 1 << 29;
    /// The cell's tiles are hidden.
    pub const HIDDEN: u32 = 1 << 31;

    /// Main index (bits 20–25).
    pub const fn main(v: u32) -> u32 {
        (v >> 20) & 0x3F
    }

    /// Sub index (bits 8–15).
    pub const fn sub(v: u32) -> u32 {
        (v >> 8) & 0xFF
    }

    /// Layer index (bits 18–19).
    pub const fn layer(v: u32) -> u32 {
        (v >> 18) & 3
    }
}

/// Tile record flags.
pub mod rec_flags {
    /// The record is on a layer above the base layer.
    pub const LAYER_ABOVE: u32 = 0x1;
    /// The record is a door or exit tile.
    pub const DOOR_OR_EXIT: u32 = 0x2;
    /// The record is hidden.
    pub const HIDDEN: u32 = 0x8;
    /// Door unit already placed (not placed again).
    pub const DOOR_UNIT: u32 = 0x20;
    /// The record is not walkable.
    pub const UNWALKABLE: u32 = 0x40;
    /// The record fills line of sight.
    pub const FILL_LOS: u32 = 0x80;
    /// Layer + 1 in bits 14–16 (`0x1C000`).
    pub const LAYER_SHIFT: u32 = 14;
}

/// A cached DT1 file and its key index.
#[derive(Clone, Debug)]
pub(super) struct Dt1File {
    /// The path it was loaded by (the cache key).
    pub(super) path: Vec<u8>,
    pub(super) tiles: Vec<TileInfo>,
    /// Key → tile indexes in reverse file order (head insertion).
    keys: BTreeMap<(u32, u32, u32), Vec<u32>>,
}

impl Dt1File {
    fn new(path: &[u8], tiles: &[TileInfo]) -> Self {
        let mut keys: BTreeMap<(u32, u32, u32), Vec<u32>> = BTreeMap::new();
        for (i, t) in tiles.iter().enumerate() {
            keys.entry((t.orientation, t.main, t.sub))
                .or_default()
                .insert(0, i as u32);
        }
        Self {
            path: path.to_vec(),
            tiles: tiles.to_vec(),
            keys,
        }
    }
}

/// A library entry: DT1 file (DRLG cache id) and tile index in file order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileRef {
    /// DRLG cache id of the DT1 file.
    pub file: u32,
    /// Tile index in file order.
    pub index: u32,
}

/// Which record array a record is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordKind {
    /// Floor records.
    Floor,
    /// Wall records.
    Wall,
    /// Shadows and roofs share this array.
    Shadow,
}

/// A tile record (render-only fields left out).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileRecord {
    /// Position relative to the room, in tiles.
    pub x: i32,
    /// Position relative to the room, in tiles.
    pub y: i32,
    /// Tile type (orientation).
    pub kind: u32,
    /// Record flags ([`rec_flags`]).
    pub flags: u32,
    /// The chosen DT1 tile.
    pub tile: TileRef,
    /// The packed cell the record came from.
    pub cell: u32,
    /// The chained type-4 half of a type-3 corner (index in the same array).
    pub half: Option<usize>,
}

/// One animation entry: the records that are the frames of an animated tile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimEntry {
    /// The record array the frames are in.
    pub kind: RecordKind,
    /// Record indexes of frames 0..n−1.
    pub frames: Vec<usize>,
    /// Phase: the shown frame is `pos >> 8`.
    pub pos: u32,
    /// Phase advance per tick.
    pub speed: u32,
}

/// A room's tile grid: its floor, wall and shadow records and the link chains shared with
/// neighbouring rooms.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoomTiles {
    /// Floor records.
    pub floors: Vec<TileRecord>,
    /// Wall records.
    pub walls: Vec<TileRecord>,
    /// Shadow and roof records.
    pub shadows: Vec<TileRecord>,
    /// The floor link chain, oldest first: new entries are prepended, so the chain head
    /// is the last entry.
    pub floor_links: Vec<usize>,
    /// The non-floor link chain, oldest first (head last); a type-3 record is followed by
    /// its type-4 half (prepended after it).
    pub other_links: Vec<(RecordKind, usize)>,
    /// The room's warp record list, oldest first: the exit-wall re-chaining makes it the
    /// tail of the non-floor chain.
    pub warp_chain: Vec<(RecordKind, usize)>,
    /// Animation entries, head first.
    pub anims: Vec<AnimEntry>,
}

impl RoomTiles {
    /// The record array of a kind.
    pub fn records(&self, kind: RecordKind) -> &[TileRecord] {
        match kind {
            RecordKind::Floor => &self.floors,
            RecordKind::Wall => &self.walls,
            RecordKind::Shadow => &self.shadows,
        }
    }

    fn records_mut(&mut self, kind: RecordKind) -> &mut Vec<TileRecord> {
        match kind {
            RecordKind::Floor => &mut self.floors,
            RecordKind::Wall => &mut self.walls,
            RecordKind::Shadow => &mut self.shadows,
        }
    }
}

/// A grid of packed cells, row-major, cell (x, y) relative to the room.
/// Reads outside the grid give 0.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CellGrid {
    /// Width in cells.
    pub width: usize,
    /// Height in cells.
    pub height: usize,
    /// The cells, row-major.
    pub cells: Vec<u32>,
}

impl CellGrid {
    /// A zeroed grid.
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            cells: vec![0; width * height],
        }
    }

    /// The cell at (x, y); 0 outside the grid.
    pub fn get(&self, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return 0;
        }
        self.cells[y as usize * self.width + x as usize]
    }

    /// Sets the cell at (x, y); panics outside the grid.
    pub fn set(&mut self, x: usize, y: usize, v: u32) {
        self.cells[y * self.width + x] = v;
    }
}

/// One pass of the tile fill: a cell grid, its orientation grid (wall layers), and
/// FillBlanks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GridPass {
    /// The packed cells.
    pub cells: CellGrid,
    /// The tile type (orientation) of each cell, for wall layers.
    pub orientation: Option<CellGrid>,
    /// Whether cells without a floor get a blank floor tile.
    pub fill_blanks: bool,
}

/// A room's source grids from the level-type generator: passes in fill order, edge kills
/// and animation settings (lvlprest).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoomGrids {
    /// The fill passes, in order.
    pub passes: Vec<GridPass>,
    /// Skip the last column of every pass.
    pub kill_edge_x: bool,
    /// Skip the last row of every pass.
    pub kill_edge_y: bool,
    /// Whether animated tiles are set up.
    pub animate: bool,
    /// lvlprest AnimSpeed (0 = 80).
    pub anim_speed: u32,
    /// Preset rooms whose lvlprest `Logicals` ≠ 0: the grids the logical-room build
    /// reads.
    pub logicals: Option<LogicGrids>,
    /// Shadow records the grid init already made (outdoor substitutions): world tile
    /// (x, y), packed cell and the tile chosen then, in creation order.
    pub shadows: Vec<(i32, i32, u32, TileRef)>,
}

fn is_door(t: u32) -> bool {
    t == 8 || t == 9
}

fn is_exit(t: u32) -> bool {
    t == 10 || t == 11
}

/// Record flag rules for a cell `v` of tile type `t` (shadows skip the layer and type
/// rules). Flags are OR'd onto `existing`; only 0x8 can be cleared.
pub fn record_flags(existing: u32, t: u32, v: u32, material: u16, shadow: bool) -> u32 {
    let mut f = existing;
    if !shadow {
        if t != 13 {
            f |= (cell::layer(v) + 1) << rec_flags::LAYER_SHIFT;
        }
        if t == 14 {
            f |= 0x4;
        }
        if (8..=11).contains(&t) {
            f |= rec_flags::DOOR_OR_EXIT;
        }
    }
    if v & cell::LAYER_ABOVE != 0 {
        f |= rec_flags::LAYER_ABOVE;
    }
    if v & cell::LINKAGE != 0 {
        f |= 0x102;
    }
    if v & cell::UNWALKABLE != 0 {
        f |= rec_flags::UNWALKABLE;
    }
    if v & cell::FILL_LOS != 0 {
        f |= rec_flags::FILL_LOS;
    }
    if v & cell::ENCLOSED != 0 {
        f |= 0x4;
    }
    if v & cell::HIDDEN != 0 {
        f |= rec_flags::HIDDEN;
    } else {
        f &= !rec_flags::HIDDEN;
    }
    if v & cell::REVEAL_HIDDEN != 0 {
        f |= 0x20C;
    }
    if v & cell::OBJECT_WALL != 0 {
        f |= 0x800;
    }
    if v & cell::LINKED != 0 {
        f |= 0x2000;
    }
    if material & 0x1 != 0 {
        f |= 0x4;
    }
    if material & 0x4 != 0 {
        f |= 0x800;
    }
    f
}

/// The weighted walk over the entries' rarities for the roll `r`. A total ≤ 0 (for
/// example all rarities 0) takes the first entry with no draw.
pub fn rarity_walk(rarities: &[i32], r: u32) -> usize {
    if rarities.iter().fold(0i32, |a, &b| a.wrapping_add(b)) <= 0 {
        return 0;
    }
    let mut n = (r as i32).wrapping_add(1);
    let mut i = 0;
    while rarities.len() > 1 && n > 0 && i < rarities.len() {
        n = n.wrapping_sub(rarities[i]);
        i += 1;
    }
    i.saturating_sub(1)
}

impl Drlg {
    /// The DT1 info of a tile reference.
    pub fn tile_info(&self, t: TileRef) -> &TileInfo {
        &self.dt1_files[t.file as usize].tiles[t.index as usize]
    }

    /// The DT1 path of a tile reference's file. A tile is exactly one (DT1 file, tile
    /// index in file order); the client keeps (path, index) per record to reach the block
    /// data.
    pub fn dt1_path(&self, t: TileRef) -> &[u8] {
        &self.dt1_files[t.file as usize].path
    }

    fn dt1_id(&mut self, tiles: &dyn TileSource, path: &[u8]) -> Result<u32, DrlgError> {
        if let Some(&id) = self.dt1_by_path.get(path) {
            return Ok(id);
        }
        let t = tiles
            .dt1(path)
            .ok_or_else(|| DrlgError::MissingDt1(String::from_utf8_lossy(path).into_owned()))?;
        let id = self.dt1_files.len() as u32;
        self.dt1_files.push(Dt1File::new(path, t));
        self.dt1_by_path.insert(path.to_vec(), id);
        Ok(id)
    }

    fn load_slot(
        &mut self,
        tiles: &dyn TileSource,
        id: DrlgRoomId,
        path: &[u8],
    ) -> Result<(), DrlgError> {
        if self.room(id).library.len() >= LIBRARY_SLOTS {
            return Err(DrlgError::LibraryFull);
        }
        let f = self.dt1_id(tiles, path)?;
        self.room_mut(id).library.push(f);
        Ok(())
    }

    /// Loads the room's DT1 files: those named by its DT1 mask, then the fixed library.
    pub(super) fn load_library(
        &mut self,
        svc: &mut Services<'_>,
        id: DrlgRoomId,
    ) -> Result<(), DrlgError> {
        self.load_library_with(svc.data, svc.tiles, id)
    }

    fn load_library_with(
        &mut self,
        data: &DrlgData,
        tiles: &dyn TileSource,
        id: DrlgRoomId,
    ) -> Result<(), DrlgError> {
        let mask = self.room(id).dt1_mask;
        let lt = self.level(self.room(id).level).level_type;
        for i in 0..32 {
            if mask >> i == 0 {
                break;
            }
            if mask & (1 << i) != 0 {
                // An empty `File` name is skipped.
                if let Some(path) = data.lvltype_file(lt, i) {
                    let path = path.to_vec();
                    self.load_slot(tiles, id, &path)?;
                }
            }
        }
        for path in FIXED_LIBRARY {
            self.load_slot(tiles, id, path)?;
        }
        self.room_mut(id).flags |= room_flags::TILE_LIB_LOADED;
        Ok(())
    }

    /// Looks up the tiles with key (t, main, sub) in the room's library: slot order, then
    /// reverse file order, at most 40 entries.
    pub fn lookup_tiles(&self, id: DrlgRoomId, t: u32, main: u32, sub: u32) -> Vec<TileRef> {
        let mut out = Vec::new();
        for &f in &self.room(id).library {
            if let Some(list) = self.dt1_files[f as usize].keys.get(&(t, main, sub)) {
                for &index in list {
                    if out.len() == LOOKUP_CAP {
                        return out;
                    }
                    out.push(TileRef { file: f, index });
                }
            }
        }
        out
    }

    /// Chooses a tile for key (t, main, sub) with room `id`'s library and seed.
    pub fn choose_tile(
        &mut self,
        id: DrlgRoomId,
        t: u32,
        main: u32,
        sub: u32,
    ) -> Result<TileRef, DrlgError> {
        let e = self.lookup_tiles(id, t, main, sub);
        if e.is_empty() {
            // No match: the first entry of the (10, 0, 0) fallback, taken as is, with no
            // rarity sum and no draw.
            return self
                .lookup_tiles(id, 10, 0, 0)
                .first()
                .copied()
                .ok_or(DrlgError::NoTile);
        }
        let mut seed = self.room(id).seed;
        let tile = self.pick_tile(e, &mut seed);
        self.room_mut(id).seed = seed;
        Ok(tile)
    }

    /// Tile choice drawing from `seed`: room `id`'s library, the seed passed in (the outdoor
    /// substitution's shadow tiles draw while the room seed is out on loan).
    pub fn choose_tile_with(
        &self,
        id: DrlgRoomId,
        t: u32,
        main: u32,
        sub: u32,
        seed: &mut Seed,
    ) -> Result<TileRef, DrlgError> {
        let e = self.lookup_tiles(id, t, main, sub);
        if e.is_empty() {
            return self
                .lookup_tiles(id, 10, 0, 0)
                .first()
                .copied()
                .ok_or(DrlgError::NoTile);
        }
        Ok(self.pick_tile(e, seed))
    }

    /// The weighted pick over a non-empty lookup: one draw when the total rarity is
    /// positive.
    fn pick_tile(&self, e: Vec<TileRef>, seed: &mut Seed) -> TileRef {
        let rar: Vec<i32> = e.iter().map(|&x| self.tile_info(x).rarity as i32).collect();
        let total = rar.iter().fold(0i32, |a, &b| a.wrapping_add(b));
        let r = if total > 0 { seed.roll(total) } else { 0 };
        e[rarity_walk(&rar, r)]
    }

    fn tiles_mut(&mut self, id: DrlgRoomId) -> &mut RoomTiles {
        self.room_mut(id)
            .tiles
            .get_or_insert_with(RoomTiles::default)
    }

    fn material(&self, t: TileRef) -> u16 {
        self.tile_info(t).material
    }

    /// Appends a floor record.
    fn add_floor(&mut self, id: DrlgRoomId, x: i32, y: i32, v: u32, tile: TileRef) -> usize {
        let flags = record_flags(0, 0, v, self.material(tile), false);
        let t = self.tiles_mut(id);
        t.floors.push(TileRecord {
            x,
            y,
            kind: 0,
            flags,
            tile,
            cell: v,
            half: None,
        });
        t.floors.len() - 1
    }

    /// Appends a shadow record.
    fn add_shadow(&mut self, id: DrlgRoomId, x: i32, y: i32, v: u32, tile: TileRef) -> usize {
        let flags = record_flags(0, 13, v, self.material(tile), true);
        let t = self.tiles_mut(id);
        t.shadows.push(TileRecord {
            x,
            y,
            kind: 13,
            flags,
            tile,
            cell: v,
            half: None,
        });
        t.shadows.len() - 1
    }

    /// Adds a wall record, its door unit, and for type 3 a chosen type-4 half.
    #[allow(clippy::too_many_arguments)]
    fn add_wall(
        &mut self,
        svc: &mut Services<'_>,
        id: DrlgRoomId,
        x: i32,
        y: i32,
        v: u32,
        t: u32,
        tile: TileRef,
    ) -> Result<usize, DrlgError> {
        let rect = self.room(id).rect;
        // For a type 8 / 9 record (new, so flag 0x20 is clear) the door unit is called
        // first, and flag 0x20 is set according to its outcome.
        let door_flag = is_door(t)
            && svc
                .types
                .door_unit(self, svc.data, id, rect.x + x, rect.y + y, v, t);
        let mut flags = record_flags(0, t, v, self.material(tile), false);
        if door_flag {
            flags |= rec_flags::DOOR_UNIT;
        }
        let tl = self.tiles_mut(id);
        tl.walls.push(TileRecord {
            x,
            y,
            kind: t,
            flags,
            tile,
            cell: v,
            half: None,
        });
        let idx = tl.walls.len() - 1;
        if t == 3 {
            let t4 = self.choose_tile(id, 4, cell::main(v), cell::sub(v))?;
            let flags = record_flags(0, 4, v, self.material(t4), false);
            let tl = self.tiles_mut(id);
            tl.walls.push(TileRecord {
                x,
                y,
                kind: 4,
                flags,
                tile: t4,
                cell: v,
                half: None,
            });
            let h = tl.walls.len() - 1;
            tl.walls[idx].half = Some(h);
        }
        Ok(idx)
    }

    /// `LitVersion` and `Tiles` of the warp entry a cell's main index names: the first
    /// entry of the room's warp list whose lvlwarp `Id` is the level's warp id for that
    /// slot, re-resolved for the cell's direction letter.
    fn warp_entry_lit(
        &self,
        svc: &Services<'_>,
        id: DrlgRoomId,
        v: u32,
        t: u32,
    ) -> Option<(u32, u32)> {
        let level_id = self.level(self.room(id).level).id;
        let w = self
            .warp_id(svc.data, level_id, cell::main(v) as usize)
            .ok()?;
        let link = self
            .room(id)
            .warp_links
            .iter()
            .find(|l| svc.data.warps.get(l.lvlwarp_row).is_some_and(|d| d.id == w))?;
        let mut row = link.lvlwarp_row;
        let letter = if t == 11 { b'r' } else { b'l' };
        let dir = svc.data.warps[row].direction;
        if dir != b'b' && dir != letter {
            row = svc.data.lvlwarp_row(w, letter).ok()?;
        }
        let d = &svc.data.warps[row];
        Some((d.lit_version, d.tiles))
    }

    /// Adds the warp tiles for an exit wall record of type `t`.
    #[allow(clippy::too_many_arguments)]
    fn wall_warp_tiles(
        &mut self,
        svc: &mut Services<'_>,
        id: DrlgRoomId,
        wx: i32,
        wy: i32,
        v: u32,
        t: u32,
        rec: Option<usize>,
    ) -> Result<(), DrlgError> {
        let sub = cell::sub(v);
        if (sub == 0 || sub == 4) && !svc.types.warp_unit(self, svc.data, id, wx, wy, v, t)? {
            // No unit added: stop here.
            return Ok(());
        }
        if let Some((lit, tiles)) = self.warp_entry_lit(svc, id, v, t) {
            // The record's chain is replaced by the warp list with the record as its head,
            // so the room's non-floor chain from the record on is the record plus that
            // list; records older than it are no longer reachable from it.
            if let Some(rec) = rec {
                let tl = self.tiles_mut(id);
                let mut chain = tl.warp_chain.clone();
                chain.push((RecordKind::Wall, rec));
                tl.other_links = chain.clone();
                tl.warp_chain = chain;
            }
            if lit != 0 {
                // One tile choice for (t, v') and a hidden wall record at the record's cell.
                let vp = v | (tiles << 8);
                let rect = self.room(id).rect;
                let tile = self.choose_tile(id, t, cell::main(vp), cell::sub(vp))?;
                let i = self.add_wall(svc, id, wx - rect.x, wy - rect.y, vp, t, tile)?;
                self.tiles_mut(id).walls[i].flags |= rec_flags::HIDDEN;
            }
        }
        Ok(())
    }

    /// Adds the hidden floor warp tiles for a hidden exit cell.
    fn floor_warp_tiles(
        &mut self,
        svc: &Services<'_>,
        id: DrlgRoomId,
        wx: i32,
        wy: i32,
        v: u32,
        t: u32,
    ) -> Result<(), DrlgError> {
        self.room_mut(id).flags |= room_flags::NO_POPULATION;
        let Some((lit, _)) = self.warp_entry_lit(svc, id, v, t) else {
            return Ok(());
        };
        if lit == 0 {
            return Ok(());
        }
        // The floor-record re-chaining for warp tiles is not modelled (it draws nothing).
        let rect = self.room(id).rect;
        let sub = cell::sub(v);
        for k in 0..4u32 {
            let (dx, dy) = (k as i32 & 1, k as i32 >> 1);
            let vk = (sub << 20) | ((k | 4) << 8);
            let tile = self.choose_tile(id, 0, cell::main(vk), cell::sub(vk))?;
            let i = self.add_floor(id, wx - 1 + dx - rect.x, wy - 1 + dy - rect.y, vk, tile);
            self.tiles_mut(id).floors[i].flags |= rec_flags::HIDDEN;
        }
        Ok(())
    }

    /// Fills a room's tiles: every pass, then animation, then the logical-room lists.
    pub(super) fn fill_room_tiles(
        &mut self,
        svc: &mut Services<'_>,
        id: DrlgRoomId,
        grids: &RoomGrids,
    ) -> Result<(), DrlgError> {
        if matches!(self.room(id).kind, RoomKind::Other(_)) {
            return Ok(());
        }
        self.room_mut(id).tiles = Some(RoomTiles::default());
        let rect = self.room(id).rect;
        // The grid init's shadow records, made room-relative, ahead of the passes.
        for &(x, y, v, tile) in &grids.shadows {
            self.add_shadow(id, x - rect.x, y - rect.y, v, tile);
        }
        let cols = rect.w + 1 - i32::from(grids.kill_edge_x);
        let rows = rect.h + 1 - i32::from(grids.kill_edge_y);
        for pass in &grids.passes {
            for y in 0..rows {
                for x in 0..cols {
                    let v = pass.cells.get(x, y);
                    let t = pass.orientation.as_ref().map_or(0, |o| o.get(x, y));
                    self.tile_cell(svc, id, x, y, v, t, pass.fill_blanks)?;
                }
            }
        }
        if grids.animate {
            self.setup_animation(id, grids.anim_speed)?;
        }
        // The coordinate lists, after the record counts are fixed. Other room types
        // returned above.
        match (self.room(id).kind, &grids.logicals) {
            (RoomKind::Preset, Some(g)) => self.build_logic_grid(id, g),
            _ => self.build_logic_one(id),
        }
        Ok(())
    }

    /// Handles one source-grid cell: hidden doors and exits, linked cells, then the floor,
    /// wall and shadow records.
    #[allow(clippy::too_many_arguments)]
    fn tile_cell(
        &mut self,
        svc: &mut Services<'_>,
        id: DrlgRoomId,
        x: i32,
        y: i32,
        v: u32,
        t: u32,
        fill_blanks: bool,
    ) -> Result<(), DrlgError> {
        let rect = self.room(id).rect;
        let level_id = self.level(self.room(id).level).id;
        let (wx, wy) = (rect.x + x, rect.y + y);
        let inside = rect.contains(wx, wy);
        let (main, sub) = (cell::main(v), cell::sub(v));
        let blank_key = main == 30 && (sub == 0 || sub == 1);
        // Exit cells with a main index of 8 or more are skipped.
        if is_exit(t) && main >= 8 {
            return Ok(());
        }
        // Blank-key floors are hidden.
        let mut v = v;
        if t == 0 && blank_key {
            v |= cell::HIDDEN;
        }
        // Hidden door and exit cells place a unit instead of tiles.
        if v & cell::HIDDEN != 0 {
            if is_door(t) && !matches!(level_id, 111 | 112 | 117) {
                svc.types.door_unit(self, svc.data, id, wx, wy, v, t);
                return Ok(());
            }
            if is_exit(t) {
                svc.types.warp_unit(self, svc.data, id, wx, wy, v, t)?;
                self.floor_warp_tiles(svc, id, wx, wy, v, t)?;
                return Ok(());
            }
        }
        // Linked cells go through the neighbour-record merge.
        if v & cell::LINKED != 0 {
            if v & cell::FLOOR != 0 {
                if blank_key {
                    v &= !cell::LAYER_ABOVE;
                }
                return self.linked_cell(svc, id, x, y, v, 0);
            } else if v & cell::WALL != 0 {
                return self.linked_cell(svc, id, x, y, v, t);
            } else if v & cell::SHADOW != 0 && v & cell::HIDDEN == 0 {
                return self.linked_cell(svc, id, x, y, v, 13);
            }
        }
        // Floor (or a blank floor when filling blanks).
        if v & cell::FLOOR != 0 {
            let tile = self.choose_tile(id, 0, main, sub)?;
            self.add_floor(id, x, y, v, tile);
        } else if fill_blanks && inside {
            let bsub = u32::from(level_id == 74);
            let tile = self.choose_tile(id, 0, 30, bsub)?;
            self.add_floor(id, x, y, (v & !cell::LAYER_ABOVE) | cell::HIDDEN, tile);
        }
        // Wall.
        if v & cell::WALL != 0 {
            let tile = self.choose_tile(id, t, main, sub)?;
            self.add_wall(svc, id, x, y, v, t, tile)?;
            if is_exit(t) && level_id != 133 {
                self.wall_warp_tiles(svc, id, wx, wy, v, t, None)?;
            }
        }
        // Shadow.
        if v & cell::SHADOW != 0 {
            let tile = self.choose_tile(id, 13, main, sub)?;
            self.add_shadow(id, x, y, v, tile);
        }
        Ok(())
    }

    // ---- linked cells -----------------------------------------------------

    /// Finds the first record of a room near that a linked cell shares.
    fn find_linked(
        &self,
        id: DrlgRoomId,
        t: u32,
        v: u32,
        wx: i32,
        wy: i32,
    ) -> Option<(DrlgRoomId, RecordKind, usize)> {
        let near = self.room(id).near.as_ref()?;
        for &n in near {
            if n == id {
                continue;
            }
            let nr = self.room(n);
            let Some(tiles) = nr.tiles.as_ref() else {
                continue;
            };
            if !nr.rect.contains_closed(wx, wy) {
                continue;
            }
            // Chains are walked head first: newest first.
            let list: Vec<(RecordKind, usize)> = if t == 0 {
                tiles
                    .floor_links
                    .iter()
                    .rev()
                    .map(|&i| (RecordKind::Floor, i))
                    .collect()
            } else {
                tiles.other_links.iter().rev().copied().collect()
            };
            for (k, i) in list {
                let r = &tiles.records(k)[i];
                // Three bits (`0x1C000`): a layer-3 cell's records carry 4 there, which only
                // another layer-3 cell matches.
                let layer = (r.flags >> rec_flags::LAYER_SHIFT) & 7;
                if nr.rect.x + r.x == wx
                    && nr.rect.y + r.y == wy
                    && r.kind != 4
                    && (k == RecordKind::Shadow || v & cell::SHADOW == 0)
                    && (layer == 0 || layer - 1 == cell::layer(v))
                {
                    return Some((n, k, i));
                }
            }
        }
        None
    }

    /// Prepends wall record `i` of room `id` to its non-floor link chain, then its type-4
    /// half when it has one.
    fn link_wall(&mut self, id: DrlgRoomId, i: usize) {
        let half = self.tiles_mut(id).walls[i].half;
        let links = &mut self.tiles_mut(id).other_links;
        links.push((RecordKind::Wall, i));
        if let Some(h) = half {
            links.push((RecordKind::Wall, h));
        }
    }

    /// Finds or adds a linked cell's record.
    #[allow(clippy::too_many_arguments)]
    fn linked_cell(
        &mut self,
        svc: &mut Services<'_>,
        id: DrlgRoomId,
        x: i32,
        y: i32,
        v: u32,
        t: u32,
    ) -> Result<(), DrlgError> {
        let rect = self.room(id).rect;
        let (wx, wy) = (rect.x + x, rect.y + y);
        match self.find_linked(id, t, v, wx, wy) {
            Some((n, k, i)) => self.linked_found(svc, id, x, y, v, t, n, k, i),
            None => {
                // No shared record yet: add one.
                if is_exit(t) && !rect.contains(wx, wy) {
                    return Ok(());
                }
                let tile = self.choose_tile(id, t, cell::main(v), cell::sub(v))?;
                match t {
                    0 => {
                        let i = self.add_floor(id, x, y, v, tile);
                        self.tiles_mut(id).floor_links.push(i);
                    }
                    13 => {
                        let i = self.add_shadow(id, x, y, v, tile);
                        self.tiles_mut(id).other_links.push((RecordKind::Shadow, i));
                    }
                    _ => {
                        let i = self.add_wall(svc, id, x, y, v, t, tile)?;
                        self.link_wall(id, i);
                        // The level-133 exclusion for exit warp tiles applies here too.
                        let level_id = self.level(self.room(id).level).id;
                        if is_exit(t) && level_id != 133 {
                            self.wall_warp_tiles(svc, id, wx, wy, v, t, Some(i))?;
                        }
                    }
                }
                Ok(())
            }
        }
    }

    /// Merged type of a linked cell of type `t` over an existing record of type `r_kind`:
    /// door edge rules, then the wall-type remap table. The layer-above rule (cell bit
    /// 0x80) is the caller's. `None` = stop.
    #[allow(clippy::too_many_arguments)]
    fn merged_type(
        &self,
        data: &DrlgData,
        id: DrlgRoomId,
        n: DrlgRoomId,
        r_kind: u32,
        wx: i32,
        wy: i32,
        t: u32,
    ) -> Result<Option<u32>, DrlgError> {
        let on_top_left = |r: TileRect| wx == r.x || wy == r.y;
        if is_door(t) && on_top_left(self.room(id).rect) {
            return Ok(Some(t));
        }
        if !is_door(t) && is_door(r_kind) && on_top_left(self.room(n).rect) {
            return Ok(None);
        }
        // The wall-type remap table.
        Ok(
            match data
                .wall_remap
                .class(t)
                .ok_or(DrlgError::WallRemapType(t))?
            {
                WallClass::Table(row) => row.get(r_kind as usize).copied(),
                WallClass::Keep => Some(t),
                WallClass::Stop => None,
            },
        )
    }

    /// Handles a linked cell that found a shared record: merges the cell's type into it,
    /// replacing or hiding tiles as the corner rules say.
    #[allow(clippy::too_many_arguments)]
    fn linked_found(
        &mut self,
        svc: &mut Services<'_>,
        id: DrlgRoomId,
        x: i32,
        y: i32,
        v: u32,
        t: u32,
        n: DrlgRoomId,
        k: RecordKind,
        i: usize,
    ) -> Result<(), DrlgError> {
        let r = self
            .room(n)
            .tiles
            .as_ref()
            .expect("found in tiles")
            .records(k)[i];
        let rect = self.room(id).rect;
        let (wx, wy) = (rect.x + x, rect.y + y);
        if r.flags & rec_flags::LAYER_ABOVE != 0 {
            if is_door(r.kind) {
                self.rerun_flags(svc, id, wx, wy, v, n, k, i);
            }
            return Ok(());
        }
        // A layer-above cell takes its own type: no table, no edge test.
        let merged = if v & cell::LAYER_ABOVE != 0 {
            t
        } else {
            match self.merged_type(svc.data, id, n, r.kind, wx, wy, t)? {
                Some(m) => m,
                None => return Ok(()),
            }
        };
        // Corner handling.
        if r.kind == 3 && merged != 3 {
            // The type-3 corner leaves the type: the record linked into the neighbour's
            // chain just before it (never its own half) is hidden. With no such record
            // the cell is an error.
            let chain = &self.room(n).tiles.as_ref().expect("tiles").other_links;
            let at = chain
                .iter()
                .position(|&e| e == (k, i))
                .expect("a found record is in its chain");
            let Some(&(pk, pi)) = at.checked_sub(1).map(|p| &chain[p]) else {
                return Err(DrlgError::LinkedCornerNoNext);
            };
            let old = {
                let rec = &mut self.tiles_mut(n).records_mut(pk)[pi];
                rec.flags |= rec_flags::HIDDEN;
                rec.tile
            };
            self.collision_update(n, pk, pi, Some(old), None);
        } else if r.kind != 3 && merged == 3 {
            // A non-corner becomes a corner: hide the old record and add a type-3 wall.
            self.tiles_mut(n).records_mut(k)[i].flags |= 0xC008;
            self.collision_update(n, k, i, Some(r.tile), None);
            let tile = self.choose_tile(id, 3, cell::main(v), cell::sub(v))?;
            let ni = self.add_wall(svc, id, x, y, v, 3, tile)?;
            self.link_wall(id, ni);
        }
        let tile_key = {
            let ti = self.tile_info(r.tile);
            (ti.main, ti.sub)
        };
        if merged != r.kind || (k == RecordKind::Floor && tile_key == (30, 0)) {
            let tile = self.choose_tile(n, merged, cell::main(v), cell::sub(v))?;
            let rec = &mut self.tiles_mut(n).records_mut(k)[i];
            let old = rec.tile;
            rec.kind = merged;
            rec.tile = tile;
            if tile != old {
                self.collision_update(n, k, i, Some(old), Some(tile));
            }
        }
        self.rerun_flags(svc, id, wx, wy, v, n, k, i);
        Ok(())
    }

    /// Re-runs the flag rules on the neighbour room `n`'s shared record, with this room
    /// `id`, position (wx, wy) and this room's cell `v`. A door record without flag 0x20
    /// first gets the door unit call for this room, which puts flag 0x20 on the record
    /// according to its outcome.
    #[allow(clippy::too_many_arguments)]
    fn rerun_flags(
        &mut self,
        svc: &mut Services<'_>,
        id: DrlgRoomId,
        wx: i32,
        wy: i32,
        v: u32,
        n: DrlgRoomId,
        k: RecordKind,
        i: usize,
    ) {
        let rec = self.room(n).tiles.as_ref().expect("tiles").records(k)[i];
        let shadow = k == RecordKind::Shadow;
        let door_flag = !shadow
            && is_door(rec.kind)
            && rec.flags & rec_flags::DOOR_UNIT == 0
            && svc.types.door_unit(self, svc.data, id, wx, wy, v, rec.kind);
        let mut f = record_flags(rec.flags, rec.kind, v, self.material(rec.tile), shadow);
        if door_flag {
            f |= rec_flags::DOOR_UNIT;
        }
        self.tiles_mut(n).records_mut(k)[i].flags = f;
    }

    // ---- animation ----------------------------------------------------------

    /// Sets up animated tiles: for walls, floors, then shadows, every record whose material
    /// has the animation bit (0x100) gets one extra hidden record per further frame.
    fn setup_animation(&mut self, id: DrlgRoomId, speed: u32) -> Result<(), DrlgError> {
        let speed = if speed == 0 {
            DEFAULT_ANIM_SPEED
        } else {
            speed
        };
        for kind in [RecordKind::Wall, RecordKind::Floor, RecordKind::Shadow] {
            let n0 = self
                .room(id)
                .tiles
                .as_ref()
                .map_or(0, |t| t.records(kind).len());
            for i in 0..n0 {
                let rec = self.room(id).tiles.as_ref().expect("tiles").records(kind)[i];
                if self.material(rec.tile) & 0x100 == 0 {
                    continue;
                }
                let f = self.lookup_tiles(id, rec.kind, cell::main(rec.cell), cell::sub(rec.cell));
                let frame = |d: &Self, k: u32| {
                    f.iter()
                        .copied()
                        .find(|&e| d.tile_info(e).rarity == k)
                        .ok_or(DrlgError::MissingFrame(k))
                };
                let f0 = frame(self, 0)?;
                self.tiles_mut(id).records_mut(kind)[i].tile = f0;
                let mut frames = vec![i];
                for k in 1..f.len() as u32 {
                    let fk = frame(self, k)?;
                    // Frame records take the base record's flags plus hidden (0x8).
                    let recs = self.tiles_mut(id).records_mut(kind);
                    recs.push(TileRecord {
                        tile: fk,
                        flags: rec.flags | rec_flags::HIDDEN,
                        half: None,
                        ..rec
                    });
                    frames.push(recs.len() - 1);
                }
                self.tiles_mut(id).anims.insert(
                    0,
                    AnimEntry {
                        kind,
                        frames,
                        pos: 0,
                        speed,
                    },
                );
                self.room_mut(id).flags |= room_flags::ANIMATED;
            }
        }
        Ok(())
    }

    /// Animation tick over the room's near rooms (each room with the animated flag and
    /// tiles): hides the shown frame and shows the next. No RNG.
    pub fn animate_tiles(&mut self, id: DrlgRoomId) {
        let near = self.room(id).near.clone().unwrap_or_default();
        for n in near {
            if self.room(n).flags & room_flags::ANIMATED == 0 {
                continue;
            }
            let Some(tiles) = self.room_mut(n).tiles.as_mut() else {
                continue;
            };
            let RoomTiles {
                floors,
                walls,
                shadows,
                anims,
                ..
            } = tiles;
            for a in anims.iter_mut() {
                let recs = match a.kind {
                    RecordKind::Floor => &mut *floors,
                    RecordKind::Wall => &mut *walls,
                    RecordKind::Shadow => &mut *shadows,
                };
                let len = a.frames.len() as u32;
                recs[a.frames[(a.pos >> 8) as usize]].flags |= rec_flags::HIDDEN;
                a.pos = (a.pos + a.speed) % (len * 256);
                recs[a.frames[(a.pos >> 8) as usize]].flags &= !rec_flags::HIDDEN;
            }
        }
    }

    /// Phase copy on the local player's room change: the position of the head entry of
    /// the first near room of `old` with a tile grid and an animation entry (0 when `old`
    /// is none or none has one) goes to every entry of every near room of `new`. The
    /// record flags are not touched: the frame shown stays until the next
    /// tick hides the frame of the copied position.
    pub fn copy_anim_phase(&mut self, old: Option<DrlgRoomId>, new: DrlgRoomId) {
        let pos = old
            .and_then(|o| {
                self.room(o)
                    .near
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .find_map(|&n| {
                        self.room(n)
                            .tiles
                            .as_ref()
                            .and_then(|t| t.anims.first())
                            .map(|a| a.pos)
                    })
            })
            .unwrap_or(0);
        let near = self.room(new).near.clone().unwrap_or_default();
        for n in near {
            if let Some(t) = self.room_mut(n).tiles.as_mut() {
                for a in &mut t.anims {
                    a.pos = pos;
                }
            }
        }
    }

    // ---- freeing -----------------------------------------------------------

    /// Frees a room's tiles. The DT1 library stays loaded. The tiles-kept flag
    /// (`TILES_KEPT`) is never set.
    pub(super) fn free_room_tiles(&mut self, svc: &mut Services<'_>, id: DrlgRoomId) {
        if let Some(a) = self.room(id).active.as_ref().map(|a| a.id) {
            // Client copy path (unset handler 3): remove the active room.
            let _ = self.remove_active_room(svc, a);
            return;
        }
        if self.room(id).flags & room_flags::HAS_ROOM != 0 {
            let r = self.room_mut(id);
            r.flags &= !room_flags::HAS_ROOM;
            r.tiles = None;
            // The logical-room info and its records go with the tiles.
            r.logic = None;
            self.freed_rooms = self.freed_rooms.wrapping_add(1);
            svc.types.free_room_tiles(self, id);
        }
    }
}
